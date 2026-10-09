import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { defineConfig } from "@playwright/test";

const e2eRoot = path.dirname(fileURLToPath(import.meta.url));

// 冒烟执行开关:run.json 由 `npm run assemble` 先行产出(先装配后测试),
// 缺失即失败而非静默跳过(v2.0 §6 执行开关纪律)。
export const RUN_MANIFEST_PATH = path.join(e2eRoot, "run.manifest.json");

type RunManifest = { baseURL?: string | null };

function readBaseURL(): string {
  let manifest: RunManifest;
  try {
    manifest = JSON.parse(readFileSync(RUN_MANIFEST_PATH, "utf8")) as RunManifest;
  } catch (error) {
    throw new Error(
      `缺少装配台账 ${RUN_MANIFEST_PATH}:先执行 npm run assemble(或 npm run smoke)。原因: ${(error as Error).message}`,
    );
  }
  if (!manifest.baseURL) {
    throw new Error("装配台账缺少 baseURL(--dry-run 台账不可驱动浏览器冒烟)");
  }
  return manifest.baseURL;
}

process.env.NO_PROXY = "127.0.0.1,localhost";
process.env.no_proxy = "127.0.0.1,localhost";

export default defineConfig({
  testDir: "./spec",
  fullyParallel: false,
  workers: 1,
  // Ruling 11:retries 固定 0,绝不「直到绿」。
  retries: 0,
  timeout: 240_000,
  expect: { timeout: 15_000 },
  reporter: [["list"], ["json", { outputFile: "artifacts/report.json" }]],
  outputDir: "artifacts/test-results",
  use: {
    baseURL: readBaseURL(),
    // 复用 ~/.cache/ms-playwright 既有 chromium-1226(纪律 3,不重复下载);
    // channel=chromium 走完整构建的新无头模式(本机无独立 headless-shell)。
    channel: "chromium",
    headless: process.env.ARIA_E2E_HEADLESS !== "0",
    trace: "on",
    screenshot: { mode: "only-on-failure", fullPage: true },
    actionTimeout: 30_000,
    navigationTimeout: 30_000,
  },
});
