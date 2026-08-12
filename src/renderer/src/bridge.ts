import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  TrayciApi,
  TrayciSettings,
  TrayciSettingsPatch,
  UsageState,
} from "../../shared/types";

export const trayci: TrayciApi = {
  usage: {
    getState: () => invoke<UsageState>("get_usage_state"),
    refreshAll: () => invoke<UsageState>("refresh_all"),
    subscribe(callback) {
      let unlisten: (() => void) | undefined;
      let cancelled = false;
      void listen<UsageState>("usage:state-changed", (event) =>
        callback(event.payload),
      ).then((release) => {
        if (cancelled) release();
        else unlisten = release;
      });
      return () => {
        cancelled = true;
        unlisten?.();
      };
    },
  },
  settings: {
    get: () => invoke<TrayciSettings>("get_settings"),
    update: (patch: TrayciSettingsPatch) =>
      invoke<TrayciSettings>("update_settings", { patch }),
  },
  app: {
    beginDrag: () => invoke<void>("begin_popover_drag"),
    hidePopover: () => invoke<void>("hide_popover"),
    resizePopover: (width, height) =>
      invoke<void>("resize_popover", { width, height }),
    quit: () => invoke<void>("quit_app"),
  },
};
