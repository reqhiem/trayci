import { app, ipcMain, powerMonitor } from "electron";
import { IPC } from "../shared/ipc";
import type { ProviderUsageSnapshot, TrayciSettingsPatch, UsageProvider } from "../shared/types";
import { cliArguments, isCliMode, runCli } from "./cli";
import { log, setDebug } from "./logger";
import { AutostartService, mergeSettings, SettingsRepository } from "./persistence";
import { ClaudeProvider } from "./providers/claude";
import { CodexProvider } from "./providers/codex";
import { abortAllChildren } from "./providers/common";
import { UsageService } from "./usage-service";
import { Popover, TrayManager } from "./window";

const args = cliArguments();
setDebug(args.includes("--debug"));

if (isCliMode(args)) {
  void runCli(args).then((code) => app.exit(code), () => app.exit(1));
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
  const providers: UsageProvider[] = process.env.TRAYCI_E2E === "1"
    ? [{
        id: "claude",
        displayName: "Claude",
        detect: async () => ({ provider: "claude", status: "available", executablePath: "/fake/claude" }),
        fetchUsage: async ({ now }) => ({
          provider: "claude",
          displayName: "Claude",
          status: "ok",
          plan: "Max",
          windows: [
            { id: "session", label: "5h", usedPercent: 28, durationMinutes: 300, resetsAt: now + 7_200_000, resetDescription: null },
            { id: "weekly", label: "Weekly", usedPercent: 52, durationMinutes: 10_080, resetsAt: now + 345_600_000, resetDescription: null }
          ],
          updatedAt: now,
          source: "api",
          error: null
        })
      }]
    : [new ClaudeProvider(() => settings.get()), new CodexProvider(() => settings.get())];
  const usage = new UsageService(providers, () => settings.get());
  const popover = new Popover();
  await popover.load();
  const tray = new TrayManager((bounds) => popover.toggle(bounds));
  let quitting = false;

  const show = (): void => {
    if (!popover.window.isVisible()) popover.toggle(tray.tray.getBounds());
  };
  app.on("second-instance", show);

  const unsubscribe = usage.subscribe((state) => {
    if (!popover.window.isDestroyed()) popover.window.webContents.send(IPC.usageChanged, state);
    const summary = Object.values(state.providers)
      .filter((snapshot): snapshot is ProviderUsageSnapshot => Boolean(snapshot?.windows.length))
      .map((snapshot) => `${snapshot.displayName} ${Math.round(Math.max(...snapshot.windows.map((window) => window.usedPercent)))}%`)
      .join(" · ");
    tray.setTooltip(summary ? `Trayci · ${summary}` : "Trayci");
  });

  ipcMain.handle(IPC.usageGet, () => usage.getState());
  ipcMain.handle(IPC.usageRefresh, () => usage.refreshAll("manual"));
  ipcMain.handle(IPC.settingsGet, () => settings.get());
  ipcMain.handle(IPC.settingsUpdate, async (_event, patch: TrayciSettingsPatch) => {
    const next = mergeSettings(settings.get(), patch);
    if (next.startOnLogin !== settings.get().startOnLogin) {
      const executable = process.env.APPIMAGE ?? process.execPath;
      await autostart.setEnabled(next.startOnLogin, executable, app.isPackaged ? [] : [process.cwd()]);
    }
    const updated = await settings.update(patch);
    usage.settingsChanged();
    return updated;
  });
  ipcMain.handle(IPC.appHide, () => popover.window.hide());
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
      kind: error instanceof Error ? error.name : "unknown"
    });
  });
}
