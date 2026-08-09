import { readFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import type {
  ProviderDetection,
  ProviderUsageSnapshot,
  TrayciSettings,
  UsageFetchContext,
  UsageProvider,
  UsageWindow,
} from "../../shared/types";
import {
  asUsedPercent,
  ProviderError,
  resetFromText,
  resolveExecutable,
  runPty,
  toEpochMs,
} from "./common";

type ClaudeWindow = { utilization?: number; resets_at?: string | null };
type ClaudeLimit = {
  group?: string;
  percent?: number;
  resets_at?: string | null;
  scope?: { model?: { display_name?: string | null } | null } | null;
};
type ClaudeUsage = Record<string, unknown> & {
  five_hour?: ClaudeWindow | null;
  seven_day?: ClaudeWindow | null;
  limits?: ClaudeLimit[] | null;
};

const labels: Record<string, string> = {
  five_hour: "5h",
  seven_day: "Weekly",
  seven_day_fable: "Fable",
  seven_day_sonnet: "Sonnet",
  seven_day_opus: "Opus",
};

function credentialPath(): string {
  return join(
    process.env.CLAUDE_CONFIG_DIR ?? join(homedir(), ".claude"),
    ".credentials.json",
  );
}

async function readCredential(): Promise<{
  accessToken: string;
  expiresAt: number | null;
} | null> {
  try {
    const value = JSON.parse(await readFile(credentialPath(), "utf8")) as {
      claudeAiOauth?: { accessToken?: unknown; expiresAt?: unknown };
    };
    const accessToken = value.claudeAiOauth?.accessToken;
    if (typeof accessToken !== "string" || !accessToken) return null;
    const expiresAt = value.claudeAiOauth?.expiresAt;
    return {
      accessToken,
      expiresAt: typeof expiresAt === "number" ? expiresAt : null,
    };
  } catch {
    return null;
  }
}

// Per-model quotas live in `limits`; the top-level `seven_day_<model>` keys
// that used to carry them are now always null.
function limitWindow(limit: ClaudeLimit): UsageWindow | null {
  const weekly = limit.group === "weekly";
  if (!weekly && limit.group !== "session") return null;
  const usedPercent = asUsedPercent(limit.percent);
  if (usedPercent === null) return null;
  const model = limit.scope?.model?.display_name;
  return {
    // Must match the ids parseClaudeUsage emits, or the CLI and OAuth paths
    // render the same quota twice while a stale snapshot is still retained.
    id: model
      ? `${model.toLowerCase().replace(/[^a-z0-9]+/g, "-")}-weekly`
      : weekly
        ? "weekly"
        : "session",
    label: model ?? (weekly ? "Weekly" : "5h"),
    usedPercent,
    durationMinutes: weekly ? 10_080 : 300,
    resetsAt: toEpochMs(limit.resets_at),
    resetDescription: null,
  };
}

// Driven by `labels` rather than by the payload keys: unrecognised
// `seven_day_*` keys would otherwise surface as windows named after internal
// feature codenames the moment they stop being null.
function legacyWindows(raw: ClaudeUsage): UsageWindow[] {
  const windows: UsageWindow[] = [];
  for (const [key, label] of Object.entries(labels)) {
    const value = raw[key];
    if (!value || typeof value !== "object") continue;
    const window = value as ClaudeWindow;
    const usedPercent = asUsedPercent(window.utilization);
    if (usedPercent === null) continue;
    windows.push({
      id:
        key === "five_hour"
          ? "session"
          : key === "seven_day"
            ? "weekly"
            : `${key.replace("seven_day_", "")}-weekly`,
      label,
      usedPercent,
      durationMinutes: key === "five_hour" ? 300 : 10_080,
      resetsAt: toEpochMs(window.resets_at),
      resetDescription: null,
    });
  }
  return windows;
}

export function normalizeClaudeUsage(raw: ClaudeUsage): UsageWindow[] {
  const windows = raw.limits?.length
    ? raw.limits.flatMap((limit) => limitWindow(limit) ?? [])
    : legacyWindows(raw);
  const priority = (id: string): number =>
    id === "session" ? 0 : id === "weekly" ? 1 : 2;
  return windows.sort((a, b) => priority(a.id) - priority(b.id));
}

export function parseClaudeUsage(output: string, now: number): UsageWindow[] {
  const normalized = output.replace(/\s+/g, " ");
  const sections: Array<[string, string, RegExp]> = [
    [
      "session",
      "5h",
      /(?:current\s+session|session|5h)[\s\S]{0,180}?(\d+(?:\.\d+)?)%\s*(used|consumed|left|remaining|available)([\s\S]{0,80})/i,
    ],
    [
      "weekly",
      "Weekly",
      /(?:current\s+week|weekly\s+limits?|7-day|weekly)[\s\S]{0,180}?(\d+(?:\.\d+)?)%\s*(used|consumed|left|remaining|available)([\s\S]{0,80})/i,
    ],
    [
      "fable-weekly",
      "Fable",
      /fable[\s\S]{0,180}?(\d+(?:\.\d+)?)%\s*(used|consumed|left|remaining|available)([\s\S]{0,80})/i,
    ],
  ];
  return sections.flatMap(([id, label, pattern]) => {
    const match = normalized.match(pattern);
    if (!match) return [];
    const value = Number(match[1]);
    const usedPercent = /left|remaining|available/i.test(match[2] ?? "")
      ? 100 - value
      : value;
    const resetText =
      (match[3] ?? "").match(/resets?\s+(?:in\s+)?[^|·,]{1,40}/i)?.[0] ?? "";
    return [
      {
        id,
        label,
        usedPercent: Math.min(100, Math.max(0, usedPercent)),
        durationMinutes: id === "session" ? 300 : 10_080,
        resetsAt: resetText ? resetFromText(resetText, now) : null,
        resetDescription: resetText || null,
      },
    ];
  });
}

export class ClaudeProvider implements UsageProvider {
  readonly id = "claude" as const;
  readonly displayName = "Claude";

  constructor(private readonly getSettings: () => TrayciSettings) {}

  async detect(): Promise<ProviderDetection> {
    const executablePath = await resolveExecutable(
      "claude",
      this.getSettings().providers.claude.executablePath,
    );
    if (!executablePath)
      return {
        provider: this.id,
        status: "not-installed",
        executablePath: null,
      };
    return {
      provider: this.id,
      status: (await readCredential()) ? "available" : "not-authenticated",
      executablePath,
    };
  }

  async fetchUsage(context: UsageFetchContext): Promise<ProviderUsageSnapshot> {
    const detection = await this.detect();
    if (!detection.executablePath)
      throw new ProviderError("not-installed", "Claude CLI not detected");
    const credential = await readCredential();
    if (
      credential &&
      (!credential.expiresAt || credential.expiresAt > context.now)
    ) {
      try {
        const response = await fetch(
          "https://api.anthropic.com/api/oauth/usage",
          {
            headers: {
              authorization: `Bearer ${credential.accessToken}`,
              "anthropic-beta": "oauth-2025-04-20",
              "user-agent": "trayci/0.1.0",
            },
            signal: AbortSignal.any([
              context.signal,
              AbortSignal.timeout(10_000),
            ]),
          },
        );
        if (response.status === 429) {
          const retryAfter = Number(response.headers.get("retry-after"));
          throw new ProviderError(
            "rate-limited",
            "Claude usage is rate limited",
            Number.isFinite(retryAfter)
              ? context.now + retryAfter * 1000
              : null,
          );
        }
        if (!response.ok)
          throw new ProviderError("network", "Claude usage request failed");
        const windows = normalizeClaudeUsage(
          (await response.json()) as ClaudeUsage,
        );
        if (!windows.length)
          throw new ProviderError("parse", "Claude returned no usage windows");
        return this.snapshot(windows, context.now, "oauth");
      } catch (error) {
        if (context.signal.aborted)
          throw new ProviderError("aborted", "Cancelled");
        if (error instanceof ProviderError && error.kind === "rate-limited")
          throw error;
      }
    }

    const output = await runPty({
      executable: detection.executablePath,
      input: "/usage",
      timeoutMs: 25_000,
      signal: context.signal,
      complete: (value) =>
        /(?:current\s+session|5h|weekly)[\s\S]*?\d+(?:\.\d+)?%/i.test(value),
    });
    const windows = parseClaudeUsage(output, context.now);
    if (!windows.length)
      throw new ProviderError("parse", "Could not parse Claude usage");
    return this.snapshot(windows, context.now, "cli");
  }

  private snapshot(
    windows: UsageWindow[],
    updatedAt: number,
    source: "oauth" | "cli",
  ): ProviderUsageSnapshot {
    return {
      provider: this.id,
      displayName: this.displayName,
      status: "ok",
      plan: null,
      windows,
      updatedAt,
      source,
      error: null,
    };
  }
}
