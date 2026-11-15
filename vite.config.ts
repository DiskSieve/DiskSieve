import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  build: {
    rollupOptions: {
      output: {
        // Long-lived vendor chunk: React rarely changes, keeps the app chunk small.
        manualChunks: { react: ["react", "react-dom"] },
      },
    },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // Pre-transform the startup module graph so the first window load doesn't wait on it.
    warmup: { clientFiles: ["./src/boot.ts", "./src/main.tsx"] },
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
});
