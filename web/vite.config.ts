import { defineConfig } from "vite";

// In dev the private listener serves the WebSocket; Vite proxies to it.
const PRIVATE = process.env.MV_PRIVATE_ADDR ?? "127.0.0.1:7801";

export default defineConfig({
  // MathJax's TeX font ships as SVG path data (~2 MB); fine for a local tool.
  build: { chunkSizeWarningLimit: 4000 },
  server: {
    host: "127.0.0.1",
    proxy: {
      "/ws": { target: `ws://${PRIVATE}`, ws: true },
      "/api": { target: `http://${PRIVATE}` },
    },
  },
});
