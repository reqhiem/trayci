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
    // Watching src-tauri/target while cargo writes to it crashes Vite with EBUSY on Windows.
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: { outDir: "dist" },
});
