import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    // Cargo writes into src-tauri/target while compiling; watching it makes
    // Vite crash with EBUSY on Windows. Tauri watches src-tauri itself.
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: { outDir: "dist" },
});
