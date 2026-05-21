import { expect, test } from "@playwright/test";
import {
  clickStartGeneration,
  enablePermissionFixture,
  openWorkspaceSession,
  seedStoryWorkspace,
  waitForStage,
  waitForTimelineNode,
} from "./helpers/workspace";

test.describe("E. 断开策略", () => {
  test("E1. Running 时刷新 beforeunload 拦截并记录断开中止", async ({ page }) => {
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E E1" });
    await enablePermissionFixture(page, seeded.sessionId);
    await openWorkspaceSession(page, seeded.sessionId);
    await clickStartGeneration(page);
    await waitForStage(page, "运行中");
    await expect(page.getByTestId("permission-request-entry")).toBeVisible({
      timeout: 30_000,
    });

    let beforeUnloadShown = false;
    page.once("dialog", async (dialog) => {
      beforeUnloadShown = dialog.type() === "beforeunload";
      await dialog.accept();
    });
    await page.reload();

    expect(beforeUnloadShown).toBe(true);
    await expect(page.getByText(/上次运行因断开被中止/i)).toBeVisible({ timeout: 10_000 });
    await waitForTimelineNode(page, "aborted_by_disconnect");
  });

  test("E2. 断开中止 banner 可关闭", async ({ page }) => {
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E E2" });
    await enablePermissionFixture(page, seeded.sessionId);
    await openWorkspaceSession(page, seeded.sessionId);
    await clickStartGeneration(page);
    await waitForStage(page, "运行中");
    await expect(page.getByTestId("permission-request-entry")).toBeVisible({
      timeout: 30_000,
    });

    page.once("dialog", (dialog) => dialog.accept());
    await page.reload();
    await expect(page.getByText(/上次运行因断开被中止/i)).toBeVisible({ timeout: 10_000 });
    await page.getByRole("button", { name: "我知道了" }).click();

    await expect(page.getByText(/上次运行因断开被中止/i)).not.toBeVisible();
  });

  test("E3. PrepareContext 刷新不拦截", async ({ page }) => {
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E E3" });
    await openWorkspaceSession(page, seeded.sessionId);

    let dialogShown = false;
    page.once("dialog", async (dialog) => {
      dialogShown = true;
      await dialog.accept();
    });
    await page.reload();

    expect(dialogShown).toBe(false);
    await waitForStage(page, "准备中");
  });

  test("E4. HumanConfirm 刷新不拦截", async ({ page }) => {
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E E4" });
    await openWorkspaceSession(page, seeded.sessionId);
    await clickStartGeneration(page);
    await waitForStage(page, "等待确认", 60_000);

    let dialogShown = false;
    page.once("dialog", async (dialog) => {
      dialogShown = true;
      await dialog.accept();
    });
    await page.reload();

    expect(dialogShown).toBe(false);
    await waitForStage(page, "等待确认", 60_000);
  });

  test("E5. 主动中止不产生 aborted_by_disconnect", async ({ page }) => {
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E E5" });
    await enablePermissionFixture(page, seeded.sessionId);
    await openWorkspaceSession(page, seeded.sessionId);
    await clickStartGeneration(page);
    await waitForStage(page, "运行中");
    await expect(page.getByTestId("permission-request-entry")).toBeVisible({
      timeout: 30_000,
    });

    await page.getByRole("button", { name: "中止" }).click();

    await expect(page.getByTestId("timeline-node-author_run").first()).toContainText(
      /failed|中止/i,
    );
    await expect(page.getByTestId("timeline-node-aborted_by_disconnect")).toHaveCount(0);
  });
});
