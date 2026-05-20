import { expect, test } from "@playwright/test";
import {
  clickStartGeneration,
  enableReviewFixture,
  openWorkspaceSession,
  seedStoryWorkspace,
  waitForStage,
  waitForTimelineNode,
} from "./helpers/workspace";

test.describe("D. 阶段化 UI + 节点 tab", () => {
  test("D1. 节点详情 5 tab 切换", async ({ page }) => {
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E D1" });

    await openWorkspaceSession(page, seeded.sessionId);
    await clickStartGeneration(page);
    await waitForStage(page, "运行中");
    await waitForTimelineNode(page, "author_run");
    await page.getByTestId("timeline-node-author_run").click();

    for (const tabId of ["tab-overview", "tab-streaming", "tab-execution", "tab-permission", "tab-artifact"]) {
      await page.getByTestId(tabId).click();
      await expect(page.getByTestId(tabId)).toHaveAttribute("aria-pressed", "true");
    }
  });

  test("D2. Header Provider snapshot 锁定状态", async ({ page }) => {
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E D2" });

    await openWorkspaceSession(page, seeded.sessionId);
    await clickStartGeneration(page);
    await waitForStage(page, "运行中");

    await expect(page.getByLabel("Provider 已锁定")).toBeVisible();
    await expect(page.getByLabel("Provider 已锁定")).toHaveAttribute("data-locked-at", /.+/);
  });

  test("D3. ReviewDecision 直接返修路径进入 revision", async ({ page }) => {
    const seeded = await seedStoryWorkspace(page, {
      projectName: "Aria E2E D3",
      reviewerProvider: "codex",
      reviewRounds: 2,
    });

    await enableReviewFixture(page, seeded.sessionId);
    await openWorkspaceSession(page, seeded.sessionId);
    await page.getByRole("button", { name: "高级配置" }).click();
    await page.getByLabel("审核轮次").fill("2");
    await clickStartGeneration(page);
    await waitForStage(page, "审核结论待处理", 60_000);
    await expect(page.getByTestId("review-decision-panel")).toBeVisible();
    await page.getByLabel("直接返修").check();
    await page.getByRole("button", { name: "确定路径" }).click();

    await waitForTimelineNode(page, "revision");
  });

  test("D4. HumanConfirm 显示 reviewer 摘要 + diff", async ({ page }) => {
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E D4" });

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

    await expect(page.getByTestId("human-confirm-panel")).toContainText("审核摘要");
    await expect(page.getByTestId("human-confirm-panel")).toContainText("与上一版本对比");
  });

  test("D5. HumanConfirm 要求修改走结构化反馈", async ({ page }) => {
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E D5" });

    await openWorkspaceSession(page, seeded.sessionId);
    await clickStartGeneration(page);
    await waitForStage(page, "等待确认", 60_000);
    const humanConfirmPanel = page.getByTestId("human-confirm-panel");
    await humanConfirmPanel.getByRole("button", { name: "要求修改" }).click();
    await humanConfirmPanel.getByLabel("内容缺失").check();
    await humanConfirmPanel.getByLabel("具体描述").fill("缺少错误处理");
    await humanConfirmPanel.getByRole("button", { name: "提交" }).click();

    await waitForTimelineNode(page, "revision");
    await waitForStage(page, "等待确认", 60_000);
  });
});
