import type { TrayciApi } from "../../shared/types";

declare global {
  interface Window {
    trayci: TrayciApi;
  }
}

export {};
