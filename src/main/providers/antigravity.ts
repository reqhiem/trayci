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
  clamp,
  ProviderError,
  resetFromText,
  resolveExecutable,
  runPty,
  toEpochMs,
} from "./common";
import { extractGeminiOAuthClient } from "./google-oauth";

const API_TIMEOUT_MS = 10_000;
const CODE_ASSIST_API = "https://cloudcode-pa.googleapis.com/v1internal";
const TOKEN_URL = "https://oauth2.googleapis.com/token";

type GoogleCredentials = {
  access_token: string;
  refresh_token: string;
  expiry_date: number;
};

type QuotaBucket = {
  remainingFraction: number;
  resetTime: string;
  modelId: string;
};

const knownLabels: Record<string, string> = {
  "gemini-3.1-pro": "3.1 Pro",
  "gemini-3.1-flash": "3.1 Flash",
  "gemini-3.1-flash-lite": "3.1 Flash Lite",
  "gemini-3.0-pro": "3.0 Pro",
  "gemini-3.0-flash": "3.0 Flash",
  "gemini-2.5-pro": "Pro",
  "gemini-2.5-flash": "Flash",
  "gemini-2.5-flash-lite": "Flash Lite",
  "gemini-2.0-pro": "2.0 Pro",
  "gemini-2.0-flash": "2.0 Flash",
  "gemini-1.5-pro": "1.5 Pro",
  "gemini-1.5-flash": "1.5 Flash",
};

function credentialsPath(): string {
  return join(
    process.env.GEMINI_CLI_HOME ?? join(homedir(), ".gemini"),
    "oauth_creds.json",
  );
}

async function readCredentials(): Promise<GoogleCredentials | null> {
  try {
    const value = JSON.parse(await readFile(credentialsPath(), "utf8")) as {
      access_token?: unknown;
      refresh_token?: unknown;
      expiry_date?: unknown;
    };
    if (
      typeof value.access_token !== "string" ||
      typeof value.refresh_token !== "string" ||
      typeof value.expiry_date !== "number"
    )
      return null;
    return value as GoogleCredentials;
  } catch {
    return null;
  }
}

function humanizeModel(modelId: string): string {
  return (
    knownLabels[modelId] ??
    modelId
      .replace(/^gemini-/i, "")
      .split("-")
      .filter(Boolean)
      .map((part) => part[0]!.toUpperCase() + part.slice(1))
      .join(" ")
  );
}

function quotaBuckets(value: unknown): QuotaBucket[] {
  const candidates = Array.isArray(value)
    ? value
    : value &&
        typeof value === "object" &&
        "buckets" in value &&
        Array.isArray(value.buckets)
      ? value.buckets
      : [];
  return candidates.filter(
    (candidate): candidate is QuotaBucket =>
      Boolean(candidate) &&
      typeof candidate === "object" &&
      "remainingFraction" in candidate &&
      typeof candidate.remainingFraction === "number" &&
      Number.isFinite(candidate.remainingFraction) &&
      "resetTime" in candidate &&
      typeof candidate.resetTime === "string" &&
      "modelId" in candidate &&
      typeof candidate.modelId === "string",
  );
}

export function normalizeAntigravityQuota(value: unknown): UsageWindow[] {
  const windows: Array<UsageWindow & { knownLabel: boolean }> = [];
  const seen = new Map<string, number>();
  for (const bucket of quotaBuckets(value)) {
    const usedPercent = Math.round(clamp((1 - bucket.remainingFraction) * 100));
    const resetsAt = toEpochMs(bucket.resetTime);
    const window = {
      id: `model-${bucket.modelId}`,
      label: humanizeModel(bucket.modelId),
      usedPercent,
      durationMinutes: 60,
      resetsAt,
      resetDescription: null,
      knownLabel: bucket.modelId in knownLabels,
    };
    const key = `${usedPercent}-${resetsAt}`;
    const previousIndex = seen.get(key);
    if (previousIndex === undefined) {
      seen.set(key, windows.length);
      windows.push(window);
      continue;
    }
    const previous = windows[previousIndex]!;
    if (
      (window.knownLabel && !previous.knownLabel) ||
      (window.knownLabel === previous.knownLabel &&
        window.label.length < previous.label.length)
    )
      windows[previousIndex] = window;
  }
  return windows.map((window) => ({
    id: window.id,
    label: window.label,
    usedPercent: window.usedPercent,
    durationMinutes: window.durationMinutes,
    resetsAt: window.resetsAt,
    resetDescription: window.resetDescription,
  }));
}

function parseQuotaGroup(
  output: string,
  heading: RegExp,
  nextHeading: RegExp | null,
  id: string,
  label: string,
  now: number,
): UsageWindow[] {
  const start = output.search(heading);
  if (start < 0) return [];
  const remainder = output.slice(start);
  const next = nextHeading ? remainder.slice(1).search(nextHeading) : -1;
  const section = (next < 0 ? remainder : remainder.slice(0, next + 1)).replace(
    /\s+/g,
    " ",
  );
  const patterns: Array<[string, string, number, RegExp]> = [
    [
      "session",
      "5h",
      300,
      /Five Hour Limit Remaining[^%]{0,200}?(\d+(?:\.\d+)?)%(.{0,160})/i,
    ],
    [
      "weekly",
      "Weekly",
      10_080,
      /Weekly Limit Remaining[^%]{0,200}?(\d+(?:\.\d+)?)%(.{0,160}?)(?=Five Hour Limit Remaining|$)/i,
    ],
  ];
  return patterns.flatMap(
    ([windowId, windowLabel, durationMinutes, pattern]) => {
      const match = section.match(pattern);
      if (!match) return [];
      const resetDescription =
        match[2]?.match(/resets?\s+(?:in\s+)?[^.]{1,80}/i)?.[0] ?? null;
      return [
        {
          id: `${id}-${windowId}`,
          label: `${label} ${windowLabel}`,
          usedPercent: clamp(100 - Number(match[1])),
          durationMinutes,
          resetsAt: resetDescription
            ? resetFromText(resetDescription, now)
            : null,
          resetDescription,
        },
      ];
    },
  );
}

export function parseAntigravityUsage(
  output: string,
  now: number,
): { windows: UsageWindow[]; plan: string | null } {
  return {
    windows: [
      ...parseQuotaGroup(
        output,
        /GEMINI MODELS/i,
        /CLAUDE AND GPT MODELS/i,
        "gemini",
        "Gemini",
        now,
      ),
      ...parseQuotaGroup(
        output,
        /CLAUDE AND GPT MODELS/i,
        null,
        "claude-gpt",
        "Claude/GPT",
        now,
      ),
    ],
    plan: output.match(/\((Google AI [^)]+)\)/i)?.[1] ?? null,
  };
}

async function apiRequest(
  path: string,
  accessToken: string,
  body: unknown,
  signal: AbortSignal,
): Promise<Response> {
  return fetch(`${CODE_ASSIST_API}:${path}`, {
    method: "POST",
    headers: {
      authorization: `Bearer ${accessToken}`,
      "content-type": "application/json",
      "user-agent": "trayci/0.1.0",
    },
    body: JSON.stringify(body),
    signal: AbortSignal.any([signal, AbortSignal.timeout(API_TIMEOUT_MS)]),
  });
}

async function refreshAccessToken(
  refreshToken: string,
  configuredExecutable: string | null,
  signal: AbortSignal,
): Promise<string> {
  const client = await extractGeminiOAuthClient(configuredExecutable);
  if (!client)
    throw new ProviderError(
      "not-authenticated",
      "Gemini CLI OAuth metadata not found",
    );
  const response = await fetch(TOKEN_URL, {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({
      client_id: client.clientId,
      client_secret: client.clientSecret,
      refresh_token: refreshToken,
      grant_type: "refresh_token",
    }),
    signal: AbortSignal.any([signal, AbortSignal.timeout(API_TIMEOUT_MS)]),
  });
  if (!response.ok)
    throw new ProviderError(
      "not-authenticated",
      "Google OAuth token refresh failed",
    );
  const value = (await response.json()) as { access_token?: unknown };
  if (typeof value.access_token !== "string")
    throw new ProviderError("parse", "Google OAuth token response changed");
  return value.access_token;
}

async function fetchQuota(
  accessToken: string,
  signal: AbortSignal,
): Promise<Response> {
  const projectResponse = await apiRequest(
    "loadCodeAssist",
    accessToken,
    { metadata: { ideType: "GEMINI_CLI", pluginType: "GEMINI" } },
    signal,
  );
  if (!projectResponse.ok) return projectResponse;
  const projectPayload = (await projectResponse.json()) as {
    cloudaicompanionProject?: unknown;
  };
  if (typeof projectPayload.cloudaicompanionProject !== "string")
    throw new ProviderError("parse", "Google Code Assist project not found");
  return apiRequest(
    "retrieveUserQuota",
    accessToken,
    { project: projectPayload.cloudaicompanionProject },
    signal,
  );
}

export class AntigravityProvider implements UsageProvider {
  readonly id = "antigravity" as const;
  readonly displayName = "Antigravity";

  constructor(
    private readonly getSettings: () => TrayciSettings,
    private readonly resolveCli: typeof resolveExecutable = resolveExecutable,
  ) {}

  async detect(): Promise<ProviderDetection> {
    const executablePath = await this.resolveCli(
      "agy",
      this.getSettings().providers.antigravity.executablePath,
    );
    return {
      provider: this.id,
      status:
        executablePath || (await readCredentials())
          ? "available"
          : "not-installed",
      executablePath,
    };
  }

  async fetchUsage(context: UsageFetchContext): Promise<ProviderUsageSnapshot> {
    const detection = await this.detect();
    let cliError: unknown;
    if (detection.executablePath) {
      try {
        const output = await runPty({
          executable: detection.executablePath,
          input: "/usage",
          writeDelayMs: 2_500,
          completionDelayMs: 1_200,
          timeoutMs: 20_000,
          signal: context.signal,
          complete: (value) =>
            (value.match(/Limit Remaining[\s\S]{0,200}?\d+(?:\.\d+)?%/gi)
              ?.length ?? 0) >= 4,
        });
        const parsed = parseAntigravityUsage(output, context.now);
        if (parsed.windows.length === 4)
          return {
            provider: this.id,
            displayName: this.displayName,
            status: "ok",
            plan: parsed.plan,
            windows: parsed.windows,
            updatedAt: context.now,
            source: "cli",
            error: null,
          };
        cliError = new ProviderError(
          "parse",
          "Could not parse Antigravity usage",
        );
      } catch (error) {
        if (context.signal.aborted)
          throw new ProviderError("aborted", "Cancelled");
        cliError = error;
      }
    }

    const credentials = await readCredentials();
    if (!credentials) {
      if (cliError) throw cliError;
      throw new ProviderError("not-installed", "Antigravity CLI not detected");
    }
    let accessToken =
      credentials.expiry_date > context.now
        ? credentials.access_token
        : await refreshAccessToken(
            credentials.refresh_token,
            null,
            context.signal,
          );
    let response = await fetchQuota(accessToken, context.signal);
    if (response.status === 401 && accessToken === credentials.access_token) {
      accessToken = await refreshAccessToken(
        credentials.refresh_token,
        null,
        context.signal,
      );
      response = await fetchQuota(accessToken, context.signal);
    }
    if (response.status === 429) {
      const seconds = Number(response.headers.get("retry-after"));
      throw new ProviderError(
        "rate-limited",
        "Antigravity usage is rate limited",
        Number.isFinite(seconds) ? context.now + seconds * 1000 : null,
      );
    }
    if (!response.ok)
      throw new ProviderError(
        response.status === 401 || response.status === 403
          ? "not-authenticated"
          : "network",
        `Antigravity quota request failed (${response.status})`,
      );
    const windows = normalizeAntigravityQuota(await response.json());
    if (!windows.length)
      throw new ProviderError("parse", "Antigravity returned no quota windows");
    return {
      provider: this.id,
      displayName: this.displayName,
      status: "ok",
      plan: null,
      windows,
      updatedAt: context.now,
      source: "oauth",
      error: null,
    };
  }
}
