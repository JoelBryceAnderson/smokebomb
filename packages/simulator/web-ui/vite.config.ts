import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// `npm run dev` serves the UI on :5173 with hot reload and proxies to the
// simulator server on :3000. `npm run build` emits dist/, which the simulator
// server serves directly on :3000.
export default defineConfig({
  plugins: [react()],
  // three.js alone is ~700 kB; fine for a local dev tool.
  build: { chunkSizeWarningLimit: 1000 },
  server: {
    port: 5173,
    proxy: {
      "/ws": { target: "ws://localhost:3000", ws: true },
      "/api": "http://localhost:3000",
    },
  },
});
