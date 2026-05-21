import { expect, test } from "@playwright/test";
import {
  clickStartGeneration,
  enablePermissionFixture,
  installWorkspaceSocketProbe,
  openWorkspaceSession,
  seedStoryWorkspace,
  sendWorkspaceSocketMessage,
  setPermissionTimeout,
  waitForStage,
} from "./helpers/workspace";

test.describe("G. Permission 链路", () => {
  test("G1. 正常 approve 继续 run", async ({ page }) => {
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E G1" });
    await enablePermissionFixture(page, seeded.sessionId);
    await openWorkspaceSession(page, seeded.sessionId);
    await clickStartGeneration(page);
    await waitForStage(page, "运行中");
    await expect(page.getByTestId("permission-request-entry")).toBeVisible();

    await page.getByRole("button", { name: "允许" }).click();

    await waitForStage(page, "等待确认", 60_000);
  });

  test("G2. 正常 deny 中止 run", async ({ page }) => {
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E G2" });
    await enablePermissionFixture(page, seeded.sessionId);
    await openWorkspaceSession(page, seeded.sessionId);
    await clickStartGeneration(page);
    await waitForStage(page, "运行中");
    await expect(page.getByTestId("permission-request-entry")).toBeVisible();

    await page.getByRole("button", { name: "拒绝" }).click();

    await waitForStage(page, "准备中", 10_000);
    await expect(page.getByTestId("timeline-node-author_run").first()).toContainText(
      /failed|permission denied/i,
    );
  });

  test("G3. unmatched id 展示 protocol_error", async ({ page }) => {
    await installWorkspaceSocketProbe(page);
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E G3" });
    await enablePermissionFixture(page, seeded.sessionId);
    await openWorkspaceSession(page, seeded.sessionId);
    await clickStartGeneration(page);
    await waitForStage(page, "运行中");
    await expect(page.getByTestId("permission-request-entry")).toBeVisible();

    await sendWorkspaceSocketMessage(page, {
      type: "permission_response",
      id: "permission_not_pending",
      approved: true,
    });

    await expect(page.getByRole("alert")).toContainText("PERMISSION_ID_UNMATCHED");
  });

  test("G4. permission 超时清理", async ({ page }) => {
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E G4" });
    try {
      await setPermissionTimeout(page, 500);
      await enablePermissionFixture(page, seeded.sessionId);
      await openWorkspaceSession(page, seeded.sessionId);
      await clickStartGeneration(page);
      await waitForStage(page, "运行中");
      await expect(page.getByTestId("permission-request-entry")).toBeVisible();

      await expect(page.getByRole("alert")).toContainText("PERMISSION_TIMEOUT", {
        timeout: 10_000,
      });
      await waitForStage(page, "准备中", 10_000);
    } finally {
      await setPermissionTimeout(page, 900_000);
    }
  });

  test("G5. 全链路 trace log", async ({ page }) => {
    const logs: string[] = [];
    page.on("console", (msg) => logs.push(msg.text()));
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E G5" });
    await enablePermissionFixture(page, seeded.sessionId);
    await openWorkspaceSession(page, seeded.sessionId);
    await clickStartGeneration(page);
    await waitForStage(page, "运行中");
    await expect(page.getByTestId("permission-request-entry")).toBeVisible();

    await page.getByRole("button", { name: "允许" }).click();

    expect(logs.some((line) => line.includes("[permission] sending response"))).toBe(true);
  });
});
