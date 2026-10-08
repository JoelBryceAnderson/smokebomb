import { defineConfig } from "vite";

// `npm run dev` serves on :5173 and forwards the WebSocket to the Rust
// server (`gbc-cube serve`, :3100).
export default defineConfig({
  // three.js is most of the bundle; one chunk is fine for a local tool.
  build: { chunkSizeWarningLimit: 800 },
  server: {
    proxy: { "/ws": { target: "ws://localhost:3100", ws: true } },
  },
});
