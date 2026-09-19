import { defineConfig } from "@playwright/test";

process.env.NO_PROXY = "127.0.0.1,localhost";
process.env.no_proxy = "127.0.0.1,localhost";

// T4 门禁排期注：4317 与 T1 门重测部署冲突时以 ARIA_E2E_PORT 旁路（默认不变）。
const e2eApiPort = process.env.ARIA_E2E_PORT ?? "4317";

export default defineConfig({
  testDir: "./e2e",
  workers: 1,
  use: {
    baseURL: "http://127.0.0.1:5173",
    channel: "chrome",
  },
  webServer: [
    {
      command: "node ./e2e/start-api.mjs",
      url: `http://127.0.0.1:${e2eApiPort}/api/health`,
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
