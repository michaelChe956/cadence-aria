import { expect, test, type Locator, type Page } from "@playwright/test";
import { readManifest, recordObserved, type ObservedWorkItem } from "../../lib/run-contract.ts";
import { CodingWorkspacePage } from "../../lib/page-objects/coding-workspace.page.ts";
import {
  FOUR_LAYERS,
  bootstrapRealJourney,
  boundarySnapshot,
  dumpStage,
  enterWorkbench,
  expectNoDrift,
  readIssueLifecycle,
  requirePreviousCleared,
  runStage,
  screenshotOf,
  throttledConsole,
} from "../../lib/journey-support.ts";

/// real 全旅程 S7:四 work item 按依赖顺序(busi→api→gateway→frontend,REQ-MTG
/// 人工显式首启)逐个:drawer「开始 Coding」→ attempt 路由 → coding 驱动
/// (分诊门语义应答/choice/终门确认完成)。每 WI 完成后 durable 断言:
/// execution_status=completed + completion_commit 落盘。
/// 单 target 预算 3600s 上界(失败关闭);总超时=4×预算+余量。

const PER_TARGET_BUDGET_MS = 3_600_000;
const STAGE_TEST_TIMEOUT = 4 * PER_TARGET_BUDGET_MS + 600_000;
const bootstrap = bootstrapRealJourney();

/** 打开指定 work item 的 drawer 并核对身份(drawer-id-chip=workItemId)。 */
async function openWorkItemDrawer(page: Page, workItemId: string): Promise<Locator> {
  const cards = page.getByTestId("lifecycle-card-work_item");
  const count = await cards.count();
  for (let index = 0; index < count; index += 1) {
    const card = cards.nth(index);
    await card.click();
    const drawer = page.getByTestId("lifecycle-card-drawer");
    await expect(drawer).toBeVisible({ timeout: 15_000 });
    const chip = (await drawer.getByTestId("drawer-id-chip").innerText().catch(() => "")).trim();
    if (chip === workItemId || chip.includes(workItemId)) {
      return drawer;
    }
    await page.keyboard.press("Escape").catch(() => {});
    await expect(drawer).toBeHidden({ timeout: 10_000 }).catch(() => {});
  }
  throw new Error(`未找到 work item 卡片(${workItemId};卡片数=${count})`);
}

test.describe("real 全旅程 S7 Coding×4", () => {
  test.setTimeout(STAGE_TEST_TIMEOUT);

  test("S7 四仓逐 target 编码到终门", async ({ page }) => {
    requirePreviousCleared(bootstrap, "s7");
    await runStage(bootstrap, "s7", async () => {
      const manifest0 = readManifest(bootstrap.runId);
      const planned = manifest0.observed.workItems ?? [];
      expect(planned.length, "台账缺少 S6 workItems").toBeGreaterThanOrEqual(4);
      const projectId = manifest0.observed.projectId!;
      const issueId = manifest0.observed.issueId!;

      const completed: ObservedWorkItem[] = [];
      for (const layer of FOUR_LAYERS) {
        const manifestNow = readManifest(bootstrap.runId);
        const target =
          (manifestNow.observed.workItems ?? []).find((item) => item.alias === layer && !item.completionCommit) ??
          planned.find((item) => item.alias === layer)!;
        test.info().annotations.push({
          type: "s7-target",
          description: `target=${layer} workItemId=${target.workItemId}`,
        });

        await enterWorkbench(bootstrap, page);
        const drawer = await openWorkItemDrawer(page, target.workItemId);
        const codingButton = drawer.getByTestId("drawer-open-coding-workspace");
        await expect(codingButton).toBeVisible({ timeout: 15_000 });
        await expect(codingButton).toHaveText(target.attemptId ? /进入 Coding Workspace/ : /开始 Coding/);
        await expect(codingButton).toBeEnabled();
        await codingButton.click();

        // 创建 attempt 后自动导航到规范 coding 路由。
        await expect(page).toHaveURL(new RegExp(`/workbench/projects/${projectId}/issues/${issueId}/coding/[^/]+$`), {
          timeout: 120_000,
        });
        const attemptId = page.url().split("/").pop()!;
        recordObserved(bootstrap.runId, {
          workItems: (readManifest(bootstrap.runId).observed.workItems ?? []).map((item) =>
            item.workItemId === target.workItemId ? { ...item, attemptId } : item,
          ),
        });

        const coding = new CodingWorkspacePage(page, { projectId, issueId, attemptId });
        await expect(coding.shell).toBeVisible({ timeout: 60_000 });
        const outcome = await coding.driveToFinalConfirm({
          stage: `s7-${layer}`,
          budgetMs: PER_TARGET_BUDGET_MS,
          evidenceDir: `${bootstrap.evidenceRoot}/s7-coding/${layer}`,
          onHeartbeat: throttledConsole(`s7-${layer}`),
        });
        expect(outcome.finalConfirmedAt, `${layer} 未到达终门确认`).not.toBeNull();
        test.info().annotations.push({
          type: `s7-${layer}-outcome`,
          description: `started=${outcome.started} gates=${outcome.gatesActed.join(",") || "-"} choices=${outcome.choicesAnswered} finalConfirmedAt=${outcome.finalConfirmedAt}`,
        });

        // durable 双面:该 WI completed + completion commit 落盘。
        const lifecycle = await readIssueLifecycle(bootstrap);
        const item = lifecycle.work_items.find((candidate) => candidate.work_item_id === target.workItemId);
        expect(item, `lifecycle 缺 WI ${target.workItemId}`).toBeTruthy();
        expect(item!.execution_status, `${layer} execution_status=${item!.execution_status}(应 completed)`).toBe("completed");
        expect(item!.completion_commit, `${layer} completion_commit 缺失`).not.toBeNull();
        completed.push({ ...target, attemptId, executionStatus: item!.execution_status, completionCommit: item!.completion_commit });
        recordObserved(bootstrap.runId, {
          workItems: (readManifest(bootstrap.runId).observed.workItems ?? []).map((candidate) =>
            candidate.workItemId === target.workItemId
              ? { ...candidate, attemptId, executionStatus: "completed", completionCommit: item!.completion_commit }
              : candidate,
          ),
        });
        await screenshotOf(bootstrap, page, `s7-${layer}-completed`);
        await dumpStage(bootstrap, `s7-${layer}`);
        expectNoDrift(bootstrap, boundarySnapshot(bootstrap, `after-s7-${layer}`), `s7-${layer}`);
      }

      expect(completed, "应完成 4 个 target").toHaveLength(4);
    });
  });
});
