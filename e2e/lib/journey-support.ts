import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { apiGet } from "./api-reader.ts";
import { collectStageDump } from "./backend-dump.ts";
import { driftSinceBaseline, snapshotRepos, type BoundarySnapshot } from "./boundary-watch.ts";
import { WorkbenchPage } from "./page-objects/workbench.page.ts";
import {
  readManifest,
  readManifestPointer,
  recordStage,
  type RunManifest,
  type StageResult,
} from "./run-contract.ts";

/// real 全旅程共用支撑:台账引导、段骨架、lifecycle 只读读面、
/// 工作台进入、边界漂移断言(冒烟 spec 同款纪律的旅程化封装)。

export const FOUR_LAYERS = ["busi", "api", "gateway", "frontend"] as const;
export const LC_NAME = "notify-center";

export type JourneyBootstrap = {
  runId: string;
  manifest: RunManifest;
  baseURL: string;
  evidenceRoot: string;
};

/** real spec 入口引导:台账必须由 assemble-real 产出(providerMode=real)。 */
export function bootstrapRealJourney(): JourneyBootstrap {
  const pointer = readManifestPointer();
  const manifest = readManifest(pointer.runId);
  if (!manifest.baseURL) throw new Error("台账缺少 baseURL(real 旅程必须由 assemble-real 驱动)");
  if (manifest.providerMode !== "real") {
    throw new Error(`real 旅程拒绝非 real 台账(providerMode=${manifest.providerMode})`);
  }
  return { runId: pointer.runId, manifest, baseURL: manifest.baseURL, evidenceRoot: manifest.evidenceRoot };
}

/** 段执行骨架:失败如实落台账再抛出;通过/降级由调用方决定状态字。 */
export async function runStage(
  bootstrap: JourneyBootstrap,
  stage: string,
  body: () => Promise<StageResult["status"] | void>,
): Promise<void> {
  const startedAt = new Date().toISOString();
  try {
    const outcome = (await body()) ?? "pass";
    recordStage(bootstrap.runId, {
      stage,
      status: outcome,
      startedAt,
      endedAt: new Date().toISOString(),
    });
  } catch (error) {
    recordStage(bootstrap.runId, {
      stage,
      status: "fail",
      startedAt,
      endedAt: new Date().toISOString(),
      detail: `${(error as Error).message}`,
    });
    throw error;
  }
}

export async function dumpStage(bootstrap: JourneyBootstrap, stage: string): Promise<void> {
  await collectStageDump(bootstrap.baseURL, bootstrap.runId, stage);
}

/** 段门:自身已过 → skip(阶段化重放安全);前置未过 → skip(not_executed)。 */
export function requirePreviousCleared(bootstrap: JourneyBootstrap, stage: string): void {
  const manifest = readManifest(bootstrap.runId);
  const own = manifest.stages[stage]?.status;
  if (own === "pass" || own === "pass_degraded") {
    test.skip(true, `段 ${stage} 已 ${own}(台账):重放跳过`);
  }
  const order = manifest.stageOrder ?? ["s0", "s1", "s2", "s3"];
  const index = order.indexOf(stage);
  if (index <= 0) return;
  const previous = manifest.stages[order[index - 1]!];
  if (!(previous?.status === "pass" || previous?.status === "pass_degraded")) {
    test.skip(true, `前置段 ${order[index - 1]} 未通过(${previous?.status ?? "缺失"}):not_executed`);
  }
}

export type LifecycleRead = {
  issue: { issue_id: string; repo_id?: string | null; status?: string | null };
  story_specs: { story_spec_id: string; title: string; confirmation_status: string; current_version: number | null; review_status?: string | null }[];
  design_specs: { design_spec_id: string; title: string; confirmation_status: string; current_version: number | null; involved_repository_ids?: string[] | null }[];
  work_items: {
    work_item_id: string;
    repository_id: string;
    title: string;
    plan_status: string;
    execution_status: string;
    latest_attempt: { attempt_id: string; status: string; stage?: string | null } | null;
    completion_commit: string | null;
    depends_on: string[];
  }[];
  work_item_repository_groups: { target_repository_id: string | null; alias: string; status: string; items: string[] }[];
  coding_attempts: { attempt_id: string; status: string; stage?: string | null }[];
  workspace_sessions: { workspace_session_id: string; entity_id: string; workspace_type: string; status: string }[];
  delivery_summary?: {
    overall: string;
    entries: {
      repository_name: string;
      work_item_id: string;
      attempt_status: string | null;
      branch_name: string | null;
      commit_sha: string | null;
      push_status: string | null;
      push_error: string | null;
    }[];
  };
};

/** 产品既有只读投影:GET /api/issues/{id}/lifecycle?project_id=...。 */
export async function readIssueLifecycle(bootstrap: JourneyBootstrap): Promise<LifecycleRead> {
  const manifest = readManifest(bootstrap.runId);
  const { projectId, issueId } = manifest.observed;
  if (!projectId || !issueId) throw new Error("台账缺少 projectId/issueId(S3 前置未完成?)");
  const result = await apiGet(
    bootstrap.baseURL,
    `/api/issues/${issueId}/lifecycle?project_id=${projectId}`,
    30_000,
  );
  if (result.status !== 200) {
    throw new Error(`lifecycle GET ${result.status}:${JSON.stringify(result.body).slice(0, 400)}`);
  }
  return result.body as LifecycleRead;
}

export async function screenshotOf(bootstrap: JourneyBootstrap, page: Page, name: string): Promise<string> {
  const target = path.join(bootstrap.evidenceRoot, "screenshots", `${name}.png`);
  await page.screenshot({ path: target, fullPage: true });
  return target;
}


export function boundarySnapshot(bootstrap: JourneyBootstrap, label: string): BoundarySnapshot {
  return snapshotRepos({
    aggregateRoot: bootstrap.manifest.aggregateRoot,
    layers: [...FOUR_LAYERS],
    evidenceRoot: bootstrap.evidenceRoot,
    label,
  });
}

function baselineSnapshot(bootstrap: JourneyBootstrap): BoundarySnapshot {
  const dir = path.join(bootstrap.evidenceRoot, "boundary");
  const first = readdirSync(dir).filter((name) => name.includes("baseline-assemble")).sort().at(0);
  if (!first) throw new Error(`缺少装配基线快照(${dir})`);
  return JSON.parse(readFileSync(path.join(dir, first), "utf8")) as BoundarySnapshot;
}

export function expectNoDrift(bootstrap: JourneyBootstrap, current: BoundarySnapshot, stage: string): void {
  const findings = driftSinceBaseline(baselineSnapshot(bootstrap), current);
  test.info().annotations.push({ type: "boundary", description: `${stage} 漂移判定:${findings.length} 项` });
  expect(findings, `阶段 ${stage} 出现越界漂移:\n${JSON.stringify(findings, null, 2)}`).toHaveLength(0);
}

/** 每段入口:进工作台、关首次提示、选中本项目(项目名取台账观测;
 * S1 建档前 projectName 尚不存在——跳过选择,由 S1 创建并记录)。 */
export async function enterWorkbench(bootstrap: JourneyBootstrap, page: Page): Promise<WorkbenchPage> {
  const manifest = readManifest(bootstrap.runId);
  const projectName = manifest.observed.projectName;
  const workbench = new WorkbenchPage(page);
  await page.goto("/");
  await expect(page).toHaveURL(/\/workbench$/);
  await workbench.expectLoaded();
  await workbench.dismissFirstRunOverlays();
  if (!projectName) return workbench;
  const entry = page
    .getByRole("navigation", { name: "Project 切换" })
    .getByRole("button", { name: projectName, exact: true });
  if (await entry.isVisible()) {
    if ((await entry.getAttribute("aria-pressed")) !== "true") {
      await entry.click();
    }
    await expect(entry).toHaveAttribute("aria-pressed", "true");
  }
  return workbench;
}

/** 心跳节流(60s 一条 console,进 playwright 输出与编排日志;不打爆注记)。 */
export function throttledConsole(label: string): (waitedMs: number, signature: string) => void {
  let lastPrintAt = 0;
  return (waitedMs, signature) => {
    if (Date.now() - lastPrintAt >= 60_000) {
      lastPrintAt = Date.now();
      console.log(`[${label}] +${Math.round(waitedMs / 1000)}s ${signature}`);
    }
  };
}
