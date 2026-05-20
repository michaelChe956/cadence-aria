import { expect, test } from "@playwright/test";
import {
  clickStartGeneration,
  installWorkspaceSocketProbe,
  openWorkspaceSession,
  seedStoryWorkspace,
  sendContextNote,
  sendWorkspaceSocketMessage,
  waitForStage,
  waitForTimelineNode,
} from "./helpers/workspace";

test.describe("A. 输入语义解耦", () => {
  test("A1. context_note 不触发 Provider", async ({ page }) => {
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E A1" });

    await openWorkspaceSession(page, seeded.sessionId);
    await sendContextNote(page, "需要支持空查询参数");

    await waitForTimelineNode(page, "context_note");
    await expect(page.getByText("需要支持空查询参数").first()).toBeVisible();
    await expect(page.getByTestId("stage-badge")).toContainText("准备中");
    await expect(page.getByTestId("timeline-node-author_run")).toHaveCount(0);
  });

  test("A2. 连续 3 条 context_note 不启动 Provider", async ({ page }) => {
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E A2" });

    await openWorkspaceSession(page, seeded.sessionId);
    for (const [index, note] of ["第一条", "第二条", "第三条"].entries()) {
      await sendContextNote(page, note);
      await expect(page.getByTestId("timeline-node-context_note")).toHaveCount(index + 1);
    }

    await expect(page.getByTestId("stage-badge")).toContainText("准备中");
    await expect(page.getByTestId("timeline-node-author_run")).toHaveCount(0);
  });

  test("A3. 开始生成锁定 Provider 并切 Running", async ({ page }) => {
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E A3" });

    await openWorkspaceSession(page, seeded.sessionId);
    await clickStartGeneration(page);

    await waitForStage(page, "运行中");
    await waitForTimelineNode(page, "start_generation");
    await waitForTimelineNode(page, "author_run");
    await expect(page.getByLabel("Provider 已锁定")).toBeVisible();
  });

  test("A4. Running 阶段发 context_note 收到 protocol_error", async ({ page }) => {
    await installWorkspaceSocketProbe(page);
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E A4" });

    await openWorkspaceSession(page, seeded.sessionId);
    await clickStartGeneration(page);
    await waitForStage(page, "运行中");
    await sendWorkspaceSocketMessage(page, {
      type: "context_note",
      content: "running 阶段不允许补充上下文",
    });

    const alert = page.getByTestId("protocol-error-alert");
    await expect(alert).toContainText("INVALID_MESSAGE_FOR_STAGE");
    await expect(alert).toContainText("context_note");
  });
});
