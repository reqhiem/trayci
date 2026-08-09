import { app, ipcMain, Notification, powerMonitor } from "electron";
import { IPC } from "../shared/ipc";
import type {
  ProviderUsageSnapshot,
  TrayciSettingsPatch,
  UsageProvider,
} from "../shared/types";
import { cliArguments, isCliMode, runCli } from "./cli";
import { log, setDebug } from "./logger";
import {
  AutostartService,
  mergeSettings,
  SettingsRepository,
} from "./persistence";
import { ClaudeProvider } from "./providers/claude";
import { CodexProvider } from "./providers/codex";
import { abortAllChildren } from "./providers/common";
import { AntigravityProvider } from "./providers/antigravity";
import { UsageService } from "./usage-service";
import { QuotaNotifier } from "./notifications";
import { Popover, TrayManager } from "./window";

const args = cliArguments();
setDebug(args.includes("--debug"));

if (isCliMode(args)) {
  void runCli(args).then(
    (code) => app.exit(code),
    () => app.exit(1),
  );
} else {
  const gotLock = app.requestSingleInstanceLock();
  if (!gotLock) {
    app.quit();
  } else {
    void startApplication();
  }
}

async function startApplication(): Promise<void> {
  await app.whenReady();
  const settings = new SettingsRepository();
  await settings.load();
  const autostart = new AutostartService();
  const providers: UsageProvider[] =
    process.env.TRAYCI_E2E === "1"
      ? [
          {
            id: "claude",
            displayName: "Claude",
            detect: async () => ({
              provider: "claude",
              status: "available",
              executablePath: "/fake/claude",
            }),
            fetchUsage: async ({ now }) => ({
              provider: "claude",
              displayName: "Claude",
              status: "ok",
              plan: "Max",
              windows: [
                {
                  id: "session",
                  label: "5h",
                  usedPercent: 28,
                  durationMinutes: 300,
                  resetsAt: now + 7_200_000,
                  resetDescription: null,
                },
                {
                  id: "weekly",
                  label: "Weekly",
                  usedPercent: 52,
                  durationMinutes: 10_080,
                  resetsAt: now + 345_600_000,
                  resetDescription: null,
                },
              ],
              updatedAt: now,
              source: "api",
              error: null,
            }),
          },
          {
            id: "codex",
            displayName: "Codex",
            detect: async () => ({
              provider: "codex",
              status: "available",
              executablePath: "/fake/codex",
            }),
            fetchUsage: async ({ now }) => ({
              provider: "codex",
              displayName: "Codex",
              status: "ok",
              plan: "Plus",
              windows: [
                {
                  id: "weekly",
                  label: "Weekly",
                  usedPercent: 19,
                  durationMinutes: 10_080,
                  resetsAt: now + 460_800_000,
                  resetDescription: null,
                },
              ],
              updatedAt: now,
              source: "rpc",
              error: null,
            }),
          },
          {
            id: "antigravity",
            displayName: "Antigravity",
            detect: async () => ({
              provider: "antigravity",
              status: "available",
              executablePath: "/fake/agy",
            }),
            fetchUsage: async ({ now }) => ({
              provider: "antigravity",
              displayName: "Antigravity",
              status: "ok",
              plan: "Google AI Pro",
              windows: [
                ["gemini-session", "Gemini 5h", 12, 300],
                ["gemini-weekly", "Gemini Weekly", 24, 10_080],
                ["claude-gpt-session", "Claude/GPT 5h", 8, 300],
                ["claude-gpt-weekly", "Claude/GPT Weekly", 31, 10_080],
              ].map(([id, label, usedPercent, durationMinutes]) => ({
                id: String(id),
                label: String(label),
                usedPercent: Number(usedPercent),
                durationMinutes: Number(durationMinutes),
                resetsAt: null,
                resetDescription: null,
              })),
              updatedAt: now,
              source: "cli",
              error: null,
            }),
          },
        ]
      : [
          new ClaudeProvider(() => settings.get()),
          new CodexProvider(() => settings.get()),
          new AntigravityProvider(() => settings.get()),
        ];
  const usage = new UsageService(providers, () => settings.get());
  const notifier = new QuotaNotifier(
    () => settings.get(),
    (title, body) => {
      if (Notification.isSupported()) new Notification({ title, body }).show();
    },
  );
  const popover = new Popover();
  await popover.load();
  const tray = new TrayManager((bounds) => popover.toggle(bounds));
  let quitting = false;

  const show = (): void => {
    if (!popover.window.isVisible()) popover.toggle(tray.tray.getBounds());
  };
  app.on("second-instance", show);

  const unsubscribe = usage.subscribe((state) => {
    notifier.update(state);
    if (!popover.window.isDestroyed())
      popover.window.webContents.send(IPC.usageChanged, state);
    const summary = Object.values(state.providers)
      .filter((snapshot): snapshot is ProviderUsageSnapshot =>
        Boolean(snapshot?.windows.length),
      )
      .map(
        (snapshot) =>
          `${snapshot.displayName} ${Math.round(Math.max(...snapshot.windows.map((window) => window.usedPercent)))}%`,
      )
      .join(" · ");
    tray.setTooltip(summary ? `Trayci · ${summary}` : "Trayci");
  });

  ipcMain.handle(IPC.usageGet, () => usage.getState());
  ipcMain.handle(IPC.usageRefresh, () => usage.refreshAll("manual"));
  ipcMain.handle(IPC.settingsGet, () => settings.get());
  ipcMain.handle(
    IPC.settingsUpdate,
    async (_event, patch: TrayciSettingsPatch) => {
      const next = mergeSettings(settings.get(), patch);
      if (next.startOnLogin !== settings.get().startOnLogin) {
        const executable = process.env.APPIMAGE ?? process.execPath;
        await autostart.setEnabled(
          next.startOnLogin,
          executable,
          app.isPackaged ? [] : [process.cwd()],
        );
      }
      const updated = await settings.update(patch);
      usage.settingsChanged();
      return updated;
    },
  );
  ipcMain.handle(IPC.appHide, () => popover.window.hide());
  ipcMain.handle(IPC.appResize, (_event, width: number, height: number) => {
    if (Number.isFinite(width) && Number.isFinite(height))
      popover.resize(width, height);
  });
  ipcMain.handle(IPC.appQuit, () => app.quit());

  powerMonitor.on("resume", () => {
    if (settings.get().refreshOnResume) void usage.refreshAll("resume");
  });
  app.on("before-quit", (event) => {
    if (quitting) return;
    event.preventDefault();
    quitting = true;
    void usage.stop().finally(() => {
      abortAllChildren();
      unsubscribe();
      tray.destroy();
      popover.window.destroy();
      app.exit(0);
    });
  });

  if (process.env.TRAYCI_E2E === "1") popover.toggle(tray.tray.getBounds());
  void usage.start().catch((error) => {
    log("error", "usage_service_start_failed", {
      kind: error instanceof Error ? error.name : "unknown",
    });
  });
}
