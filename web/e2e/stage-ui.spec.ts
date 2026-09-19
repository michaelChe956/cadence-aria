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
    await page.getByRole("button", { name: "Artifact" }).click();
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

  // T4/REQ-RET-02 L1 退役留档（v1.1 修订：「stage-ui.spec.ts D4/D5 重钉为 typed 动作面
  // 或退役留档」）：
  // - D4（HumanConfirm 输入框发送修改意见）锚 legacy request-change 发送面——随
  //   前端 legacy 决策发送删除退役（human_confirm 阶段输入只读）。
  // - D5（HumanConfirm 确认/终止钮）锚 story legacy 流 terminate=human_confirm 帧——
  //   前端已切 abandon_human_gate（SC 门命令族，legacy 流白名单不放行→双轨期
  //   protocol error，REQ-RET-03 已登记迁移限制），story 流终止路径随退役不可达。
  // typed 动作面（approve=confirm 帧/abandon=abandon_human_gate 帧/feedback=
  // human_gate_feedback 帧）e2e 需 SC 会话夹具（WorkItemPlan+provider run 至人工
  // 门），现无该夹具且建夹具超出本 WP 范围——typed 面由 vitest 断言族覆盖：
  // useWorkspaceWs.actions.test.tsx（abandon/confirm wire 形状）、
  // cockpit-action-routing.test.ts（facade 路由+幂等 command_id）、
  // plan-repair-actions.test.tsx（typed 通道审计）。
});
