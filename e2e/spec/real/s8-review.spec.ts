import { expect, test } from "@playwright/test";
import { recordObserved } from "../../lib/run-contract.ts";
import {
  bootstrapRealJourney,
  boundarySnapshot,
  dumpStage,
  enterWorkbench,
  expectNoDrift,
  readIssueLifecycle,
  requirePreviousCleared,
  runStage,
  screenshotOf,
} from "../../lib/journey-support.ts";

/// real 全旅程 S8:评审/交付终态核对。
/// Code Review 属 Coding 链(S7 内已逐 attempt 走完终门);本段核对:
/// - 四 attempt 全部 completed(真实 commit);
/// - delivery_summary.overall=all_pushed(每 entry push_status=pushed+SHA+分支);
/// - issue.status=completed(AllPushed 自动落盘);
/// - 页面交付面板(delivery-status-panel)与 durable 同色。

const STAGE_TEST_TIMEOUT = 600_000;

const bootstrap = bootstrapRealJourney();

test.describe("real 全旅程 S8 评审/交付终态", () => {
  test.setTimeout(STAGE_TEST_TIMEOUT);

  test("S8 交付汇总与 issue completed", async ({ page }) => {
    requirePreviousCleared(bootstrap, "s8");
    await runStage(bootstrap, "s8", async () => {
      // durable 面:全部 attempt completed + all_pushed + issue completed。
      const lifecycle = await readIssueLifecycle(bootstrap);
      const completedAttempts = lifecycle.coding_attempts.filter((attempt) => attempt.status === "completed");
      expect(
        completedAttempts.length,
        `coding_attempts 应 4 个 completed,实际 ${completedAttempts.length}/${lifecycle.coding_attempts.length}:${JSON.stringify(lifecycle.coding_attempts.map((attempt) => `${attempt.attempt_id}:${attempt.status}`))}`,
      ).toBe(4);
      for (const item of lifecycle.work_items) {
        expect(item.completion_commit, `WI ${item.work_item_id} 缺 completion_commit`).not.toBeNull();
        expect(item.execution_status).toBe("completed");
      }
      const summary = lifecycle.delivery_summary;
      expect(summary, "delivery_summary 缺失").toBeTruthy();
      expect(summary!.overall, `delivery overall=${summary!.overall}(应 all_pushed)`).toBe("all_pushed");
      expect(summary!.entries.length).toBeGreaterThanOrEqual(4);
      for (const entry of summary!.entries) {
        expect(entry.push_status, `${entry.repository_name} push_status=${entry.push_status}`).toBe("pushed");
        expect(entry.commit_sha, `${entry.repository_name} commit_sha 缺失`).not.toBeNull();
        expect(entry.branch_name, `${entry.repository_name} branch_name 缺失`).not.toBeNull();
      }
      expect(lifecycle.issue.status, `issue.status=${lifecycle.issue.status}(应 completed)`).toBe("completed");
      recordObserved(bootstrap.runId, { issueStatus: lifecycle.issue.status ?? "unknown", deliveryOverall: summary!.overall });

      // 页面双面:issue drawer 交付面板与 durable 一致。
      await enterWorkbench(bootstrap, page);
      const issueCard = page.getByTestId("lifecycle-card-issue");
      await expect(issueCard).toHaveCount(1, { timeout: 30_000 });
      await issueCard.nth(0).click();
      const drawer = page.getByTestId("lifecycle-card-drawer");
      await expect(drawer).toBeVisible({ timeout: 15_000 });
      const panel = drawer.getByTestId("delivery-status-panel");
      await expect(panel).toBeVisible({ timeout: 30_000 });
      await expect(panel).toContainText("已全部交付", { timeout: 30_000 });

      await screenshotOf(bootstrap, page, "s8-delivery-all-pushed");
      await dumpStage(bootstrap, "s8");
      expectNoDrift(bootstrap, boundarySnapshot(bootstrap, "after-s8"), "s8");
    });
  });
});
