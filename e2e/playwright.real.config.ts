import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { defineConfig } from "@playwright/test";

const e2eRoot = path.dirname(fileURLToPath(import.meta.url));

/// real 全旅程执行配置(v2.0 §6:real/fake 不串):
/// 只匹配 spec/real/**;run.manifest.json 必须由 `npm run assemble:real`
/// 先行产出(providerMode=real),缺失即失败而非静默跳过。
/// 段编排由 scripts/run-real-journey.ts 逐 spec 串行驱动;本配置
/// workers=1、retries=0(Ruling 11:绝不「直到绿」)。长段超时由各 spec
/// test.setTimeout 显式声明(Story/Design/Plan/Review 5400s 上界、
/// 单 target Coding 3600s 上界、初始化 7200s 上界;失败关闭,非时长预测)。

export const RUN_MANIFEST_PATH = path.join(e2eRoot, "run.manifest.json");

type RunManifest = { baseURL?: string | null; providerMode?: string };

function readBaseURL(): string {
  let manifest: RunManifest;
  try {
    manifest = JSON.parse(readFileSync(RUN_MANIFEST_PATH, "utf8")) as RunManifest;
  } catch (error) {
    throw new Error(
      `缺少装配台账 ${RUN_MANIFEST_PATH}:先执行 npm run assemble:real。原因: ${(error as Error).message}`,
    );
  }
  if (!manifest.baseURL) {
    throw new Error("装配台账缺少 baseURL(--dry-run 台账不可驱动浏览器旅程)");
  }
  if (manifest.providerMode !== "real") {
    throw new Error(
      `real 旅程拒绝非 real 台账(providerMode=${String(manifest.providerMode)}):重新 npm run assemble:real`,
    );
  }
  return manifest.baseURL;
}

process.env.NO_PROXY = "127.0.0.1,localhost";
process.env.no_proxy = "127.0.0.1,localhost";

export default defineConfig({
  testDir: "./spec/real",
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 900_000,
  expect: { timeout: 15_000 },
  reporter: [["list"], ["json", { outputFile: "artifacts/real-report.json" }]],
  outputDir: "artifacts/real-test-results",
  use: {
    baseURL: readBaseURL(),
    channel: "chromium",
    headless: process.env.ARIA_E2E_HEADLESS !== "0",
    trace: "on",
    screenshot: { mode: "only-on-failure", fullPage: true },
    actionTimeout: 30_000,
    navigationTimeout: 30_000,
  },
});
