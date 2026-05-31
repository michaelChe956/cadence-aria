import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";
import { devServerProxy } from "./dev-server-proxy";

export default defineConfig({
  plugins: [react()],
  server: {
    proxy: devServerProxy,
  },
  test: {
    environment: "jsdom",
    exclude: ["e2e/**", "node_modules/**", "dist/**"],
    setupFiles: ["./src/test/setup.ts"],
  },
});
