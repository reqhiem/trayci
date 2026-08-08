import { readFile } from "node:fs/promises";
import { join } from "node:path";
import type {
  ProviderId,
  ProviderUsageSnapshot,
  UsageProvider,
} from "../shared/types";
import { SettingsRepository } from "./persistence";
import { ClaudeProvider } from "./providers/claude";
import { CodexProvider } from "./providers/codex";
import { abortAllChildren, activeChildCount } from "./providers/common";

export function cliArguments(argv = process.argv): string[] {
  return argv
    .slice(1)
    .filter(
      (argument) => argument !== "." && !argument.endsWith("out/main/index.js"),
    );
}

export function isCliMode(args: string[]): boolean {
  return args.some(
    (argument) =>
      argument === "usage" || argument === "doctor" || argument === "--version",
  );
}

async function version(): Promise<string> {
  const packageJson = JSON.parse(
    await readFile(
      join(import.meta.dirname, "../../package.json"),
      "utf8",
    ).catch(() => '{"version":"0.1.0"}'),
  ) as { version?: string };
  return packageJson.version ?? "0.1.0";
}

function providers(settings: SettingsRepository): UsageProvider[] {
  return [
    new ClaudeProvider(() => settings.get()),
    new CodexProvider(() => settings.get()),
  ];
}

function formatReset(resetsAt: number | null, now = Date.now()): string {
  if (!resetsAt) return "";
  const minutes = Math.max(0, Math.floor((resetsAt - now) / 60_000));
  if (!minutes) return "resets now";
  if (minutes < 60) return `resets in ${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `resets in ${hours}h ${minutes % 60}m`;
  return `resets in ${Math.floor(hours / 24)}d ${hours % 24}h`;
}

function human(snapshot: ProviderUsageSnapshot): string {
  if (snapshot.status !== "ok")
    return `${snapshot.displayName}\n  ${snapshot.error ?? "Usage unavailable"}`;
  const windows = snapshot.windows.map((window) =>
    `  ${window.label.padEnd(10)} ${Math.round(window.usedPercent)}%   ${formatReset(window.resetsAt)}`.trimEnd(),
  );
  return [snapshot.displayName, ...windows].join("\n");
}

export async function runCli(args: string[]): Promise<number> {
  if (args.includes("--version")) {
    console.log(await version());
    return 0;
  }

  const settings = new SettingsRepository();
  await settings.load();
  const available = providers(settings);

  if (args.includes("doctor")) {
    const detections = await Promise.all(
      available.map((provider) => provider.detect()),
    );
    for (const detection of detections) {
      console.log(
        `${detection.provider}: ${detection.status}${detection.executablePath ? ` (${detection.executablePath})` : ""}`,
      );
    }
    console.log(`active provider probes: ${activeChildCount()}`);
    return detections.every((result) => result.status === "available") ? 0 : 1;
  }

  const providerArg = args.find(
    (argument): argument is "claude" | "codex" =>
      argument === "claude" || argument === "codex",
  );
  const selected = providerArg
    ? available.filter((provider) => provider.id === providerArg)
    : available;
  const controller = new AbortController();
  const stop = (): void => controller.abort();
  process.once("SIGINT", stop);
  process.once("SIGTERM", stop);
  try {
    const snapshots = await Promise.all(
      selected.map(async (provider) => {
        try {
          return await provider.fetchUsage({
            signal: controller.signal,
            reason: "manual",
            now: Date.now(),
          });
        } catch (error) {
          return {
            provider: provider.id,
            displayName: provider.displayName,
            status: "error" as const,
            plan: null,
            windows: [],
            updatedAt: Date.now(),
            source: null,
            error: error instanceof Error ? error.message : "Usage unavailable",
          };
        }
      }),
    );
    if (args.includes("--json")) {
      console.log(
        JSON.stringify(
          {
            providers: Object.fromEntries(
              snapshots.map((snapshot) => [snapshot.provider, snapshot]),
            ),
          },
          null,
          2,
        ),
      );
    } else {
      console.log(snapshots.map(human).join("\n\n"));
    }
    return snapshots.every((snapshot) => snapshot.status === "ok") ? 0 : 1;
  } finally {
    process.removeListener("SIGINT", stop);
    process.removeListener("SIGTERM", stop);
    abortAllChildren();
  }
}

export const isProviderId = (value: string): value is ProviderId =>
  value === "claude" || value === "codex";
