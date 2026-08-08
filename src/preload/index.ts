import { contextBridge, ipcRenderer } from "electron";
import { IPC } from "../shared/ipc";
import type { TrayciApi, UsageState } from "../shared/types";

const api: TrayciApi = {
  usage: {
    getState: () => ipcRenderer.invoke(IPC.usageGet),
    refreshAll: () => ipcRenderer.invoke(IPC.usageRefresh),
    subscribe(callback) {
      const listener = (
        _event: Electron.IpcRendererEvent,
        state: UsageState,
      ): void => callback(state);
      ipcRenderer.on(IPC.usageChanged, listener);
      return () => ipcRenderer.removeListener(IPC.usageChanged, listener);
    },
  },
  settings: {
    get: () => ipcRenderer.invoke(IPC.settingsGet),
    update: (patch) => ipcRenderer.invoke(IPC.settingsUpdate, patch),
  },
  app: {
    hidePopover: () => ipcRenderer.invoke(IPC.appHide),
    resizePopover: (width, height) =>
      ipcRenderer.invoke(IPC.appResize, width, height),
    quit: () => ipcRenderer.invoke(IPC.appQuit),
  },
};

contextBridge.exposeInMainWorld("trayci", api);
