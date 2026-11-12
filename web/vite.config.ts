import { defineConfig } from "vite";

// The web client is served by any static host; in development Vite proxies
// API and WebSocket traffic to the Thrum server so no CORS setup is needed.
export default defineConfig({
  server: {
    port: 5173,
    proxy: {
      "/v1": {
        target: process.env.THRUM_SERVER ?? "http://127.0.0.1:8080",
        changeOrigin: false,
        ws: true,
      },
    },
  },
  build: {
    // The MLS core is large; keep chunks above the warning threshold quiet.
    chunkSizeWarningLimit: 4000,
  },
});
