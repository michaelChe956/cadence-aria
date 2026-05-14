import { defineConfig } from "@playwright/test";

process.env.NO_PROXY = "127.0.0.1,localhost";
process.env.no_proxy = "127.0.0.1,localhost";

export default defineConfig({
  testDir: "./e2e",
  use: {
    baseURL: "http://127.0.0.1:5173",
    channel: "chrome",
  },
  webServer: [
    {
      command: "node ./e2e/start-api.mjs",
      url: "http://127.0.0.1:4317/api/health",
      reuseExistingServer: false,
      timeout: 120_000,
    },
    {
      command: "pnpm dev --port 5173",
      url: "http://127.0.0.1:5173",
      reuseExistingServer: true,
      timeout: 120_000,
    },
  ],
});
