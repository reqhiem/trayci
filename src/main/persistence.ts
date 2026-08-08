import { constants } from "node:fs";
import {
  access,
  mkdir,
  readFile,
  rename,
  unlink,
  writeFile,
} from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, isAbsolute, join } from "node:path";
import type {
  ProviderUsageSnapshot,
  TrayciSettings,
  TrayciSettingsPatch,
} from "../shared/types";
import { DEFAULT_SETTINGS } from "../shared/types";

export function configDirectory(): string {
  return join(
    process.env.XDG_CONFIG_HOME ?? join(homedir(), ".config"),
    "trayci",
  );
}

async function atomicJson(path: string, value: unknown): Promise<void> {
  await mkdir(dirname(path), { recursive: true, mode: 0o700 });
  const temporary = `${path}.${process.pid}.tmp`;
  await writeFile(temporary, `${JSON.stringify(value, null, 2)}\n`, {
    mode: 0o600,
  });
  await rename(temporary, path);
}

function object(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function validSnapshot(value: unknown): value is ProviderUsageSnapshot {
  if (!object(value) || !Array.isArray(value.windows)) return false;
  return (
    typeof value.provider === "string" &&
    typeof value.displayName === "string" &&
    typeof value.status === "string" &&
    typeof value.updatedAt === "number" &&
    value.windows.every(
      (window) =>
        object(window) &&
        typeof window.id === "string" &&
        typeof window.label === "string" &&
        typeof window.usedPercent === "number",
    )
  );
}

export class CacheRepository {
  private readonly path = join(configDirectory(), "cache.json");

  async load(): Promise<Partial<Record<string, ProviderUsageSnapshot>>> {
    try {
      const value = JSON.parse(await readFile(this.path, "utf8")) as unknown;
      if (
        !object(value) ||
        value.schemaVersion !== 1 ||
        !object(value.providers)
      )
        return {};
      return Object.fromEntries(
        Object.entries(value.providers)
          .filter((entry): entry is [string, ProviderUsageSnapshot] =>
            validSnapshot(entry[1]),
          )
          .map(([id, snapshot]) => [
            id,
            { ...snapshot, status: "stale", source: "cache", error: null },
          ]),
      );
    } catch {
      return {};
    }
  }

  async save(
    providers: Partial<Record<string, ProviderUsageSnapshot>>,
  ): Promise<void> {
    const safe = Object.fromEntries(
      Object.entries(providers)
        .filter((entry): entry is [string, ProviderUsageSnapshot] =>
          Boolean(entry[1]),
        )
        .map(([id, snapshot]) => [
          id,
          {
            provider: snapshot.provider,
            displayName: snapshot.displayName,
            status: snapshot.status,
            plan: snapshot.plan,
            windows: snapshot.windows,
            updatedAt: snapshot.updatedAt,
            source: snapshot.source,
            error: snapshot.error,
          },
        ]),
    );
    await atomicJson(this.path, { schemaVersion: 1, providers: safe });
  }
}

const settingsKeys = new Set([
  "startOnLogin",
  "refreshOnResume",
  "pollIntervalMinutes",
  "displayMode",
  "percentageDisplay",
  "providers",
]);

function validatePatch(patch: TrayciSettingsPatch): void {
  if (
    !object(patch) ||
    Object.keys(patch).some((key) => !settingsKeys.has(key))
  ) {
    throw new TypeError("Invalid settings patch");
  }
  if (
    patch.startOnLogin !== undefined &&
    typeof patch.startOnLogin !== "boolean"
  ) {
    throw new TypeError("startOnLogin must be a boolean");
  }
  if (
    patch.refreshOnResume !== undefined &&
    typeof patch.refreshOnResume !== "boolean"
  ) {
    throw new TypeError("refreshOnResume must be a boolean");
  }
  if (
    patch.pollIntervalMinutes !== undefined &&
    (!Number.isFinite(patch.pollIntervalMinutes) ||
      patch.pollIntervalMinutes < 5 ||
      patch.pollIntervalMinutes > 1440)
  ) {
    throw new TypeError("pollIntervalMinutes must be between 5 and 1440");
  }
  if (
    patch.displayMode !== undefined &&
    !["detailed", "compact"].includes(patch.displayMode)
  ) {
    throw new TypeError("Invalid displayMode");
  }
  if (
    patch.percentageDisplay !== undefined &&
    !["used", "remaining"].includes(patch.percentageDisplay)
  ) {
    throw new TypeError("Invalid percentageDisplay");
  }
  if (patch.providers !== undefined) {
    if (
      !object(patch.providers) ||
      Object.keys(patch.providers).some(
        (key) => key !== "claude" && key !== "codex",
      )
    ) {
      throw new TypeError("Invalid provider settings");
    }
    for (const provider of Object.values(patch.providers)) {
      if (
        !object(provider) ||
        Object.keys(provider).some(
          (key) => key !== "enabled" && key !== "executablePath",
        )
      ) {
        throw new TypeError("Invalid provider settings");
      }
      if (
        provider.enabled !== undefined &&
        typeof provider.enabled !== "boolean"
      ) {
        throw new TypeError("Provider enabled must be a boolean");
      }
      if (
        provider.executablePath !== undefined &&
        provider.executablePath !== null &&
        (typeof provider.executablePath !== "string" ||
          !isAbsolute(provider.executablePath))
      ) {
        throw new TypeError(
          "Provider executablePath must be an absolute path or null",
        );
      }
    }
  }
}

export function mergeSettings(
  current: TrayciSettings,
  patch: TrayciSettingsPatch,
): TrayciSettings {
  validatePatch(patch);
  return {
    ...current,
    ...patch,
    schemaVersion: 1,
    providers: {
      claude: { ...current.providers.claude, ...patch.providers?.claude },
      codex: { ...current.providers.codex, ...patch.providers?.codex },
    },
  };
}

export class SettingsRepository {
  private readonly path = join(configDirectory(), "config.json");
  private settings: TrayciSettings = structuredClone(DEFAULT_SETTINGS);

  async load(): Promise<TrayciSettings> {
    try {
      const raw = JSON.parse(
        await readFile(this.path, "utf8"),
      ) as TrayciSettingsPatch & {
        schemaVersion?: unknown;
      };
      if (raw.schemaVersion !== 1) return this.settings;
      const patch = { ...raw };
      delete patch.schemaVersion;
      this.settings = mergeSettings(this.settings, patch);
    } catch {
      this.settings = structuredClone(DEFAULT_SETTINGS);
    }
    return this.get();
  }

  get(): TrayciSettings {
    return structuredClone(this.settings);
  }

  async update(patch: TrayciSettingsPatch): Promise<TrayciSettings> {
    this.settings = mergeSettings(this.settings, patch);
    await atomicJson(this.path, this.settings);
    return this.get();
  }
}

function desktopExec(executable: string): string {
  return `"${executable.replaceAll("\\", "\\\\").replaceAll('"', '\\"').replaceAll("%", "%%")}"`;
}

export class AutostartService {
  private readonly path = join(
    process.env.XDG_CONFIG_HOME ?? join(homedir(), ".config"),
    "autostart",
    "trayci.desktop",
  );

  async isEnabled(): Promise<boolean> {
    try {
      await access(this.path, constants.F_OK);
      return true;
    } catch {
      return false;
    }
  }

  async setEnabled(
    enabled: boolean,
    executable: string,
    args: string[] = [],
  ): Promise<void> {
    if (!enabled) {
      await unlink(this.path).catch((error: NodeJS.ErrnoException) => {
        if (error.code !== "ENOENT") throw error;
      });
      return;
    }
    await mkdir(dirname(this.path), { recursive: true, mode: 0o700 });
    await writeFile(
      this.path,
      `[Desktop Entry]\nType=Application\nName=Trayci\nComment=AI coding usage in your system tray\nExec=${[executable, ...args].map(desktopExec).join(" ")}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n`,
      { mode: 0o600 },
    );
  }
}
