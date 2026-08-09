import type { TrayciApi } from "../../shared/types";

// Electron exposed the API as the `window.trayci` preload global; on Tauri
// the same shape is provided by the imported bridge module (./bridge), so no
// global augmentation remains. Kept to document the renderer API type.
export type { TrayciApi };
