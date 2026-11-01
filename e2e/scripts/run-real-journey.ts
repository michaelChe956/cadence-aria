import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { readManifest, readManifestPointer, type RunManifest } from "../lib/run-contract.ts";

/// real 全旅程段编排(v2.0 Q5:每阶段独立 spec+外层串行,进程边界不共享内存):
/// - 逐 spec 串行 `playwright test <spec> --config playwright.real.config.ts`;
/// - 段前置由 spec 内 guard(自身已过 skip=重放安全;前置未过 skip=not_executed);
/// - 任一 spec 非零退出 → 停止后续(teardown 回填 not_executed),编排退出非零;
/// - 阶段化重放:env/台账保留,重跑本脚本自动跳过已 pass 段,从首个未过段续跑;
/// - 停滞看门狗(只报不杀):aria 日志与台账均 20min 无变化时输出 STAGNATION 心跳。

const e2eRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

const STAGE_SPECS: { spec: string; stages: string[] }[] = [
  { spec: "onboarding-s0-s3.spec.ts", stages: ["s0", "s1", "s2", "s3"] },
  { spec: "s4-story.spec.ts", stages: ["s4"] },
  { spec: "s5-design.spec.ts", stages: ["s5"] },
  { spec: "s6-plan.spec.ts", stages: ["s6"] },
  { spec: "s7-coding.spec.ts", stages: ["s7"] },
  { spec: "s8-review.spec.ts", stages: ["s8"] },
  { spec: "s9-verification.spec.ts", stages: ["s9"] },
];

const STAGNATION_LIMIT_MS = 20 * 60_000;
const WATCH_INTERVAL_MS = 60_000;

function stagePassed(manifest: RunManifest, stage: string): boolean {
  const status = manifest.stages[stage]?.status;
  return status === "pass" || status === "pass_degraded";
}

function ariaLogPath(manifest: RunManifest): string | null {
  const candidate = path.join(manifest.evidenceRoot, "logs", "aria-web.log");
  return existsSync(candidate) ? candidate : null;
}

function snapshotSignals(manifest: RunManifest): { logSize: number; manifestMtime: number } {
  const log = ariaLogPath(manifest);
  const logSize = log ? statSync(log).size : -1;
  const runJson = path.join(path.dirname(manifest.evidenceRoot), "run.json");
  return { logSize, manifestMtime: existsSync(runJson) ? statSync(runJson).mtimeMs : -1 };
}

function main(): void {
  const pointer = readManifestPointer();
  const logFile = ariaLogPath(readManifest(pointer.runId));
  let lastChange = Date.now();
  let lastSignals = snapshotSignals(readManifest(pointer.runId));

  const watchdog = setInterval(() => {
    const current = snapshotSignals(readManifest(pointer.runId));
    if (current.logSize !== lastSignals.logSize || current.manifestMtime !== lastSignals.manifestMtime) {
      lastSignals = current;
      lastChange = Date.now();
    }
    const quietMs = Date.now() - lastChange;
    const manifest = readManifest(pointer.runId);
    const stageSummary = (manifest.stageOrder ?? Object.keys(manifest.stages))
      .map((stage) => `${stage}=${manifest.stages[stage]?.status ?? "-"}`)
      .join(" ");
    console.log(`[journey] 心跳 +${Math.round(quietMs / 1000)}s ${stageSummary}`);
    if (quietMs >= STAGNATION_LIMIT_MS) {
      console.log(`[journey][STAGNATION] ${Math.round(quietMs / 60_000)}min 无台账/aria 日志变化;保留现场,人工裁决(不自动杀进程)`);
    }
  }, WATCH_INTERVAL_MS);
  watchdog.unref?.();

  let failed = false;
  for (const entry of STAGE_SPECS) {
    const manifest = readManifest(pointer.runId);
    if (entry.stages.every((stage) => stagePassed(manifest, stage))) {
      console.log(`[journey] 跳过已通过段 ${entry.stages.join(",")}(${entry.spec})`);
      continue;
    }
    console.log(`[journey] 执行 ${entry.spec}(段 ${entry.stages.join(",")})`);
    const result = spawnSync(
      "npx",
      ["playwright", "test", path.join("spec/real", entry.spec), "--config", "playwright.real.config.ts"],
      { cwd: e2eRoot, stdio: "inherit", env: process.env },
    );
    if (result.status !== 0) {
      console.log(`[journey] ${entry.spec} 失败(exit=${result.status});停止后续段,teardown 将回填 not_executed`);
      failed = true;
      break;
    }
  }
  clearInterval(watchdog);

  const final = readManifest(pointer.runId);
  const order = final.stageOrder ?? Object.keys(final.stages);
  console.log("[journey] 段结果:");
  for (const stage of order) {
    console.log(`  ${stage}=${final.stages[stage]?.status ?? "not_executed"}`);
  }
  if (logFile) console.log(`[journey] aria 日志:${logFile}`);
  console.log(`[journey] 证据:${readManifest(pointer.runId).evidenceRoot}`);
  process.exit(failed ? 1 : 0);
}

main();
