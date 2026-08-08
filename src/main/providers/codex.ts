import { spawn } from "node:child_process";
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
  boundedCwd,
  clamp,
  ProviderError,
  resetFromText,
  resolveExecutable,
  runPty,
  titleCase,
  toEpochMs,
  trackChild,
} from "./common";

type RateWindow = {
  usedPercent?: number;
  windowDurationMins?: number | null;
  resetsAt?: number | null;
};
type RateSnapshot = {
  limitId?: string | null;
  limitName?: string | null;
  planType?: string | null;
  primary?: RateWindow | null;
  secondary?: RateWindow | null;
};
type RateResponse = {
  rateLimits?: RateSnapshot;
  rateLimitsByLimitId?: Record<string, RateSnapshot> | null;
};

function codexHome(): string {
  return process.env.CODEX_HOME ?? join(homedir(), ".codex");
}

async function hasAuth(): Promise<boolean> {
  try {
    const value = JSON.parse(
      await readFile(join(codexHome(), "auth.json"), "utf8"),
    ) as {
      auth_mode?: unknown;
      tokens?: { access_token?: unknown };
    };
    return (
      typeof value.tokens?.access_token === "string" ||
      typeof value.auth_mode === "string"
    );
  } catch {
    return Boolean(process.env.OPENAI_API_KEY);
  }
}

function windowLabel(duration: number | null, fallback: string): string {
  if (duration !== null && Math.abs(duration - 300) <= 2) return "5h";
  if (duration !== null && Math.abs(duration - 10_080) <= 2) return "Weekly";
  return fallback;
}

export function normalizeCodexRateLimits(raw: RateResponse): {
  windows: UsageWindow[];
  plan: string | null;
} {
  const mapped = raw.rateLimitsByLimitId
    ? Object.entries(raw.rateLimitsByLimitId)
    : [];
  const snapshots: Array<[string, RateSnapshot]> = mapped.length
    ? mapped
    : raw.rateLimits
      ? [[raw.rateLimits.limitId ?? "codex", raw.rateLimits]]
      : [];
  const windows: UsageWindow[] = [];
  let plan: string | null = null;
  for (const [limitId, snapshot] of snapshots) {
    plan ??= snapshot.planType ?? null;
    for (const [slot, value] of [
      ["primary", snapshot.primary],
      ["secondary", snapshot.secondary],
    ] as const) {
      if (!value || typeof value.usedPercent !== "number") continue;
      const rawDuration =
        typeof value.windowDurationMins === "number"
          ? value.windowDurationMins
          : null;
      const duration =
        rawDuration !== null && Math.abs(rawDuration - 300) <= 2
          ? 300
          : rawDuration !== null && Math.abs(rawDuration - 10_080) <= 2
            ? 10_080
            : rawDuration;
      const prefix = snapshots.length > 1 ? `${limitId}-` : "";
      windows.push({
        id: `${prefix}${duration === 300 ? "session" : duration === 10_080 ? "weekly" : slot}`,
        label: windowLabel(duration, snapshot.limitName ?? titleCase(limitId)),
        usedPercent: clamp(value.usedPercent),
        durationMinutes: duration,
        resetsAt: toEpochMs(value.resetsAt),
        resetDescription: null,
      });
    }
  }
  return { windows, plan };
}

export function parseCodexUsage(output: string, now: number): UsageWindow[] {
  const normalized = output.replace(/\s+/g, " ");
  const patterns: Array<[string, string, number, RegExp]> = [
    [
      "session",
      "5h",
      300,
      /(?:5h|session)(?:\s+limit)?[^%]{0,100}?(\d+(?:\.\d+)?)%([^|·]{0,80})/i,
    ],
    [
      "weekly",
      "Weekly",
      10_080,
      /(?:weekly|week|7-day)(?:\s+limit)?[^%]{0,100}?(\d+(?:\.\d+)?)%([^|·]{0,80})/i,
    ],
  ];
  return patterns.flatMap(([id, label, durationMinutes, pattern]) => {
    const match = normalized.match(pattern);
    if (!match) return [];
    const tail = match[2] ?? "";
    const raw = Number(match[1]);
    const usedPercent = /left|remaining|available/i.test(tail)
      ? 100 - raw
      : raw;
    const reset = tail.match(/resets?\s+(?:in\s+)?[^|·,]{1,40}/i)?.[0] ?? "";
    return [
      {
        id,
        label,
        usedPercent: clamp(usedPercent),
        durationMinutes,
        resetsAt: reset ? resetFromText(reset, now) : null,
        resetDescription: reset || null,
      },
    ];
  });
}

async function fetchViaRpc(
  executable: string,
  signal: AbortSignal,
): Promise<RateResponse> {
  if (signal.aborted) throw new ProviderError("aborted", "Cancelled");
  const cwd = await boundedCwd();
  return new Promise((resolve, reject) => {
    const child = spawn(executable, ["app-server", "--stdio"], {
      cwd,
      env: process.env,
      stdio: ["pipe", "pipe", "ignore"],
    });
    const untrack = trackChild(child);
    let buffer = "";
    let settled = false;
    let phase: "initialize" | "limits" = "initialize";
    let timeout = setTimeout(
      () =>
        finish(
          new ProviderError(
            "timeout",
            "Codex app-server initialization timed out",
          ),
        ),
      30_000,
    );

    const finish = (error?: ProviderError, result?: RateResponse): void => {
      if (settled) return;
      settled = true;
      clearTimeout(timeout);
      signal.removeEventListener("abort", abort);
      child.stdout.removeAllListeners();
      child.removeAllListeners();
      child.stdin.end();
      untrack();
      child.kill("SIGTERM");
      const kill = setTimeout(() => child.kill("SIGKILL"), 500);
      kill.unref();
      if (error) reject(error);
      else resolve(result ?? {});
    };
    const abort = (): void => finish(new ProviderError("aborted", "Cancelled"));
    const send = (message: unknown): void => {
      child.stdin.write(`${JSON.stringify(message)}\n`);
    };

    signal.addEventListener("abort", abort, { once: true });
    child.on("error", () =>
      finish(
        new ProviderError(
          "rpc-unavailable",
          "Codex app-server failed to start",
        ),
      ),
    );
    child.on("exit", () =>
      finish(
        new ProviderError("rpc-unavailable", "Codex app-server exited early"),
      ),
    );
    child.stdout.on("data", (chunk: Buffer) => {
      buffer += chunk.toString("utf8");
      for (
        let newline = buffer.indexOf("\n");
        newline >= 0;
        newline = buffer.indexOf("\n")
      ) {
        const line = buffer.slice(0, newline);
        buffer = buffer.slice(newline + 1);
        let message: { id?: number; result?: unknown; error?: unknown };
        try {
          message = JSON.parse(line) as typeof message;
        } catch {
          continue;
        }
        if (phase === "initialize" && message.id === 1) {
          if (message.error)
            return finish(
              new ProviderError(
                "rpc-unavailable",
                "Codex RPC initialization failed",
              ),
            );
          phase = "limits";
          clearTimeout(timeout);
          timeout = setTimeout(
            () =>
              finish(
                new ProviderError(
                  "timeout",
                  "Codex rate-limit request timed out",
                ),
              ),
            10_000,
          );
          send({ jsonrpc: "2.0", method: "initialized" });
          send({
            jsonrpc: "2.0",
            id: 2,
            method: "account/rateLimits/read",
            params: null,
          });
        } else if (phase === "limits" && message.id === 2) {
          if (message.error)
            return finish(
              new ProviderError(
                "rpc-unavailable",
                "Codex rate limits are unavailable",
              ),
            );
          finish(undefined, message.result as RateResponse);
        }
      }
    });
    send({
      jsonrpc: "2.0",
      id: 1,
      method: "initialize",
      params: { clientInfo: { name: "trayci", version: "0.1.0" } },
    });
  });
}

export class CodexProvider implements UsageProvider {
  readonly id = "codex" as const;
  readonly displayName = "Codex";

  constructor(private readonly getSettings: () => TrayciSettings) {}

  async detect(): Promise<ProviderDetection> {
    const executablePath = await resolveExecutable(
      "codex",
      this.getSettings().providers.codex.executablePath,
    );
    if (!executablePath)
      return {
        provider: this.id,
        status: "not-installed",
        executablePath: null,
      };
    return {
      provider: this.id,
      status: (await hasAuth()) ? "available" : "not-authenticated",
      executablePath,
    };
  }

  async fetchUsage(context: UsageFetchContext): Promise<ProviderUsageSnapshot> {
    const detection = await this.detect();
    if (!detection.executablePath)
      throw new ProviderError("not-installed", "Codex CLI not detected");
    if (detection.status === "not-authenticated") {
      throw new ProviderError("not-authenticated", "Codex is not signed in");
    }
    try {
      const { windows, plan } = normalizeCodexRateLimits(
        await fetchViaRpc(detection.executablePath, context.signal),
      );
      if (!windows.length)
        throw new ProviderError("parse", "Codex returned no usage windows");
      return this.snapshot(windows, plan, context.now, "rpc");
    } catch (error) {
      if (context.signal.aborted)
        throw new ProviderError("aborted", "Cancelled");
      if (error instanceof ProviderError && error.kind === "not-authenticated")
        throw error;
    }

    const output = await runPty({
      executable: detection.executablePath,
      input: "/status",
      timeoutMs: 15_000,
      signal: context.signal,
      complete: (value) =>
        /(?:5h|weekly|7-day)[\s\S]*?\d+(?:\.\d+)?%/i.test(value),
    });
    const windows = parseCodexUsage(output, context.now);
    if (!windows.length)
      throw new ProviderError("parse", "Could not parse Codex usage");
    return this.snapshot(windows, null, context.now, "cli");
  }

  private snapshot(
    windows: UsageWindow[],
    plan: string | null,
    updatedAt: number,
    source: "rpc" | "cli",
  ): ProviderUsageSnapshot {
    return {
      provider: this.id,
      displayName: this.displayName,
      status: "ok",
      plan,
      windows,
      updatedAt,
      source,
      error: null,
    };
  }
}
