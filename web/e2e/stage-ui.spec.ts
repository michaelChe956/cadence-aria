import { expect, test } from "@playwright/test";
import {
  clickStartGeneration,
  enableReviewFixture,
  openWorkspaceSession,
  seedStoryWorkspace,
  waitForStage,
  waitForTimelineNode,
} from "./helpers/workspace";

test.describe("D. 阶段化 UI + chat 交互", () => {
  test("D1. 发送上下文后可见 chat 输入和时间线节点", async ({ page }) => {
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E D1" });

    await openWorkspaceSession(page, seeded.sessionId);
    await expect(page.getByTestId("chat-input-bar")).toBeVisible();
    await page.getByTestId("context-note-input").fill("补充登录需求");
    await page.getByTestId("send-context-note").click();
    await expect(page.getByTestId("chat-entry-list")).toContainText("补充登录需求");

    await clickStartGeneration(page);
    await waitForStage(page, "运行中");
    await waitForTimelineNode(page, "author_run");
    await expect(page.getByTestId("chat-entry-list")).toContainText("开始生成");
    await expect(page.getByTestId("artifact-pane")).toBeVisible();
  });

  test("D2. Header Provider snapshot 锁定状态", async ({ page }) => {
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E D2" });

    await openWorkspaceSession(page, seeded.sessionId);
    await clickStartGeneration(page);
    await waitForStage(page, "运行中");

    await expect(page.getByLabel("Provider 已锁定")).toBeVisible();
    await expect(page.getByLabel("Provider 已锁定")).toHaveAttribute("data-locked-at", /.+/);
  });

  test("D3. ReviewDecision 路径按钮进入 revision", async ({ page }) => {
    const seeded = await seedStoryWorkspace(page, {
      projectName: "Aria E2E D3",
      reviewerProvider: "codex",
      reviewRounds: 2,
    });

    await enableReviewFixture(page, seeded.sessionId);
    await openWorkspaceSession(page, seeded.sessionId);
    await page.getByRole("button", { name: "Provider 配置" }).click();
    await page.getByRole("button", { name: "高级配置" }).click();
    await page.getByLabel("审核轮次").fill("2");
    await page.getByRole("button", { name: "关闭 Provider 配置" }).click();
    await clickStartGeneration(page);
    await waitForStage(page, "审核结论待处理", 60_000);
    await expect(page.getByRole("button", { name: "补充上下文后修订" })).toBeVisible();
    await page.getByRole("button", { name: "补充上下文后修订" }).click();

    await waitForTimelineNode(page, "revision");
  });

  test("D4. HumanConfirm 允许通过输入框发送修改意见", async ({ page }) => {
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E D4" });

    await openWorkspaceSession(page, seeded.sessionId);
    await clickStartGeneration(page);
    await waitForStage(page, "等待确认", 60_000);
    const gatePrompt = page.getByTestId("gate-prompt-entry");
    await expect(gatePrompt).toBeVisible();
    await page.getByTestId("context-note-input").fill("补充异常路径和边界场景");
    await page.getByTestId("send-human-decision").click();
    await waitForTimelineNode(page, "revision");
    await waitForStage(page, "等待确认", 60_000);

    await expect(page.getByTestId("chat-entry-list")).toContainText("补充异常路径和边界场景");
  });

  test("D5. HumanConfirm 的确认和终止按钮可用", async ({ page }) => {
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E D5" });

    await openWorkspaceSession(page, seeded.sessionId);
    await clickStartGeneration(page);
    await waitForStage(page, "等待确认", 60_000);
    const gatePrompt = page.getByTestId("gate-prompt-entry");
    await expect(gatePrompt).toBeVisible();
    await expect(gatePrompt.getByRole("button", { name: "确认" })).toBeVisible();
    await expect(gatePrompt.getByRole("button", { name: "终止" })).toBeVisible();
    await gatePrompt.getByRole("button", { name: "终止" }).click();

    await waitForStage(page, "已完成", 60_000);
    await expect(page.getByTestId("context-note-input")).toBeDisabled();
    await expect(page.getByTestId("context-note-input")).toHaveAttribute("placeholder", "流程已完成");
  });
});
