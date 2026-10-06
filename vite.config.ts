import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  envPrefix: ["VITE_", "TAURI_"],
  server: { port: 1420, strictPort: true },
  build: {
    // The CSP allows fonts only from 'self'; inlined data: URIs would be blocked.
    assetsInlineLimit: 0,
    target: "es2022",
  },
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    globals: false,
    css: false,
  },
});
