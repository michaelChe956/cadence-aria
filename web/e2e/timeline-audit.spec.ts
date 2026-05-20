import { expect, test } from "@playwright/test";
import {
  clickStartGeneration,
  dropWorkspaceSocketFromServer,
  enablePermissionFixture,
  openWorkspaceSession,
  seedStoryWorkspace,
  sendContextNote,
  waitForStage,
  waitForTimelineNode,
} from "./helpers/workspace";

test.describe("B. Timeline 审计 + 会话恢复", () => {
  test("B1. 流式断开并刷新后 snapshot 含 streaming 累积和 aborted_by_disconnect", async ({
    page,
  }) => {
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E B1" });
    await enablePermissionFixture(page, seeded.sessionId);

    await openWorkspaceSession(page, seeded.sessionId);
    await clickStartGeneration(page);
    await waitForStage(page, "运行中");
    await waitForTimelineNode(page, "author_run");
    await page.getByTestId("timeline-node-author_run").click();
    await page.getByTestId("tab-streaming").click();
    await expect(page.getByTestId("streaming-content")).not.toContainText("无流式输出");
    const streamingBefore = await page.getByTestId("streaming-content").textContent();

    await dropWorkspaceSocketFromServer(page, seeded.sessionId);
    await waitForStage(page, "准备中");
    await waitForTimelineNode(page, "aborted_by_disconnect");
    await page.reload();

    await waitForStage(page, "准备中");
    await waitForTimelineNode(page, "aborted_by_disconnect");
    await page.getByTestId("timeline-node-author_run").click();
    await page.getByTestId("tab-streaming").click();
    const streamingAfter = await page.getByTestId("streaming-content").textContent();
    expect((streamingAfter ?? "").length).toBeGreaterThanOrEqual(
      (streamingBefore ?? "").length,
    );
  });

  test("B2. permission_request 未应答断开并刷新后 snapshot 含 pending", async ({ page }) => {
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E B2" });
    await enablePermissionFixture(page, seeded.sessionId);

    await openWorkspaceSession(page, seeded.sessionId);
    await clickStartGeneration(page);
    await waitForStage(page, "运行中");
    await expect(page.getByText("E2E permission fixture request")).toBeVisible();

    await dropWorkspaceSocketFromServer(page, seeded.sessionId);
    await waitForStage(page, "准备中");
    await waitForTimelineNode(page, "aborted_by_disconnect");
    await page.reload();

    await waitForStage(page, "准备中");
    await waitForTimelineNode(page, "aborted_by_disconnect");
    await page.getByTestId("timeline-node-author_run").click();
    await page.getByTestId("tab-permission").click();
    await expect(page.getByTestId("node-detail-panel")).toContainText("e2e_permission_1");
    await expect(page.getByTestId("node-detail-panel")).toContainText("待应答");
  });

  test("B3. reviewer verdict 完成后刷新 snapshot 完整", async ({ page }) => {
    const seeded = await seedStoryWorkspace(page, {
      projectName: "Aria E2E B3",
      reviewerProvider: "codex",
    });

    await openWorkspaceSession(page, seeded.sessionId);
    await clickStartGeneration(page);
    await waitForStage(page, "等待确认", 60_000);

    await page.reload();

    await waitForStage(page, "等待确认", 60_000);
    await waitForTimelineNode(page, "reviewer_run");
    await page.getByTestId("timeline-node-reviewer_run").click();
    await expect(page.getByTestId("node-detail-panel")).toContainText("审核结论");
  });

  test("B4. 多版本 revision 后刷新 snapshot 完整", async ({ page }) => {
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E B4" });

    await openWorkspaceSession(page, seeded.sessionId);
    await clickStartGeneration(page);
    await waitForStage(page, "等待确认", 60_000);

    const humanConfirmPanel = page.getByTestId("human-confirm-panel");
    await humanConfirmPanel.getByRole("button", { name: "要求修改" }).click();
    await humanConfirmPanel.getByLabel("内容缺失").check();
    await humanConfirmPanel.getByLabel("具体描述").fill("补充异常路径和边界场景");
    await humanConfirmPanel.getByRole("button", { name: "提交" }).click();
    await waitForTimelineNode(page, "revision");
    await waitForStage(page, "等待确认", 60_000);

    await page.reload();

    await waitForStage(page, "等待确认", 60_000);
    await expect(page.getByTestId("timeline-node-author_run")).toHaveCount(1);
    await expect(page.getByTestId("timeline-node-revision")).toHaveCount(1);
    await expect(page.getByTestId("human-confirm-panel")).toContainText("v1 → v2");
  });

  test("B5. 100+ 节点写入后刷新仍可恢复完整 Timeline", async ({ page }) => {
    test.setTimeout(90_000);
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E B5" });

    await openWorkspaceSession(page, seeded.sessionId);
    for (let index = 0; index < 100; index += 1) {
      await sendContextNote(page, `note-${index}`);
    }
    await expect(page.getByTestId("timeline-node-context_note")).toHaveCount(100);

    const startedAt = Date.now();
    await page.reload();

    await waitForStage(page, "准备中");
    await expect(page.getByTestId("timeline-node-context_note")).toHaveCount(100);
    expect(Date.now() - startedAt).toBeLessThan(5_000);
  });
});
