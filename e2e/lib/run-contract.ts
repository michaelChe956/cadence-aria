import { existsSync, readFileSync, renameSync, writeFileSync, mkdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

/// 运行台账契约:runId 生成、目录布局、原子落盘、指针文件。
///
/// 台账是「观测台账」而非产品状态来源;每段开始前 spec 重新读页面/只读 API
/// 核对前置(v2.0 §Q5)。段结果原子落盘(write tmp + rename)。

export const E2E_ROOT = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
export const ARTIFACTS_ROOT = path.join(E2E_ROOT, "artifacts");
export const FIXTURES_ROOT = path.join(E2E_ROOT, "fixtures");
/** config/teardown 读取的指针文件(assemble 写,gitignored)。 */
export const MANIFEST_POINTER_PATH = path.join(E2E_ROOT, "run.manifest.json");

export type StageStatus =
  | "pass"
  | "pass_degraded"
  | "fail"
  | "not_executed"
  | "not_observed";

export type StageResult = {
  stage: string;
  status: StageStatus;
  startedAt: string;
  endedAt: string;
  detail?: string;
  evidenceRefs?: string[];
};

export type RunManifest = {
  runId: string;
  createdAt: string;
  dryRun: boolean;
  worktreeHead: string;
  providerMode: string;
  ariaBinary: { path: string; sha256: string };
  distFingerprint: { indexHtmlSha256: string; assetCount: number };
  baseURL: string | null;
  workspaceRoot: string;
  aggregateRoot: string;
  originsRoot: string;
  evidenceRoot: string;
  ariaPid: number | null;
  ariaStartTimeTicks: string | null;
  /** spec 观测到的身份(只作台账,不作为产品状态来源)。 */
  observed: {
    projectId?: string;
    projectName?: string;
    logicalCodebaseId?: string;
    logicalCodebaseName?: string;
    initOperationId?: string;
    initStatus?: string;
    issueId?: string;
    issueTitle?: string;
    primaryAlias?: string;
  };
  stages: Record<string, StageResult>;
};

export function newRunId(prefix = "s0s3"): string {
  const now = new Date();
  const pad = (value: number) => String(value).padStart(2, "0");
  const stamp =
    `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}` +
    `-${pad(now.getHours())}${pad(now.getMinutes())}${pad(now.getSeconds())}`;
  const rand = Math.random().toString(36).slice(2, 6);
  return `${prefix}-${stamp}-${rand}`;
}

export function runDir(runId: string): string {
  return path.join(ARTIFACTS_ROOT, runId);
}

export function manifestPath(runId: string): string {
  return path.join(runDir(runId), "run.json");
}

/** 原子写 JSON(tmp + rename)。 */
export function atomicWriteJson(target: string, value: unknown): void {
  mkdirSync(path.dirname(target), { recursive: true });
  const temp = `${target}.tmp-${process.pid}`;
  writeFileSync(temp, `${JSON.stringify(value, null, 2)}\n`, "utf8");
  renameSync(temp, target);
}

export function writeManifest(manifest: RunManifest): void {
  atomicWriteJson(manifestPath(manifest.runId), manifest);
  // 指针文件:playwright.config 与 teardown 以最新 run 驱动。
  atomicWriteJson(MANIFEST_POINTER_PATH, {
    runId: manifest.runId,
    baseURL: manifest.baseURL,
    runDir: runDir(manifest.runId),
    ariaPid: manifest.ariaPid,
    dryRun: manifest.dryRun,
  });
}

export function readManifestPointer(): { runId: string; baseURL: string | null; runDir: string; ariaPid: number | null; dryRun: boolean } {
  if (!existsSync(MANIFEST_POINTER_PATH)) {
    throw new Error(`缺少装配指针 ${MANIFEST_POINTER_PATH}:先执行 npm run assemble`);
  }
  return JSON.parse(readFileSync(MANIFEST_POINTER_PATH, "utf8"));
}

export function readManifest(runId: string): RunManifest {
  const target = manifestPath(runId);
  if (!existsSync(target)) {
    throw new Error(`缺少运行台账 ${target}`);
  }
  return JSON.parse(readFileSync(target, "utf8")) as RunManifest;
}

/** 记录段结果(读-改-写,原子)。 */
export function recordStage(runId: string, result: StageResult): void {
  const manifest = readManifest(runId);
  manifest.stages = { ...manifest.stages, [result.stage]: result };
  writeManifest(manifest);
}

/** 观测身份合并(原子)。 */
export function recordObserved(runId: string, observed: Partial<RunManifest["observed"]>): void {
  const manifest = readManifest(runId);
  manifest.observed = { ...manifest.observed, ...observed };
  writeManifest(manifest);
}

/** 段前置状态:上一段未 pass/pass_degraded → 后续 not_executed。 */
export function previousStageCleared(manifest: RunManifest, stage: string): boolean {
  const order = ["s0", "s1", "s2", "s3"];
  const index = order.indexOf(stage);
  if (index <= 0) return true;
  const previous = manifest.stages[order[index - 1]];
  return previous?.status === "pass" || previous?.status === "pass_degraded";
}
