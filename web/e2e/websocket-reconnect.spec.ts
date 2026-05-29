import { expect, test } from "@playwright/test";
import {
  closeActiveWorkspaceSocketFromClient,
  dropWorkspaceSocketFromServer,
  installWorkspaceSocketProbe,
  openWorkspaceSession,
  rejectNextWorkspaceSockets,
  seedStoryWorkspace,
  setWsTimeout,
  waitForStage,
  waitForWorkspaceSocketCount,
} from "./helpers/workspace";

test.describe("F. 自动重连", () => {
  test("F1. 服务端主动 drop socket 后自动重连并应用 snapshot", async ({ page }) => {
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E F1" });
    await openWorkspaceSession(page, seeded.sessionId);

    await dropWorkspaceSocketFromServer(page, seeded.sessionId);

    await expect(page.getByText(/重连中/i)).not.toBeVisible();
    await waitForStage(page, "准备中", 10_000);
  });

  test("F2. 多次失败显示重连进度 banner", async ({ page }) => {
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E F2" });
    await openWorkspaceSession(page, seeded.sessionId);

    await rejectNextWorkspaceSockets(page, seeded.sessionId, 2);
    await dropWorkspaceSocketFromServer(page, seeded.sessionId);

    await expect(page.getByText(/尝试 2 次/i)).toBeVisible({ timeout: 15_000 });
  });

  test("F3. hidden 暂停恢复", async ({ page }) => {
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E F3" });
    await openWorkspaceSession(page, seeded.sessionId);

    await page.evaluate(() => {
      Object.defineProperty(document, "hidden", { configurable: true, value: true });
      document.dispatchEvent(new Event("visibilitychange"));
    });
    await dropWorkspaceSocketFromServer(page, seeded.sessionId);
    await page.waitForTimeout(1500);
    await expect(page.getByText(/重连中/i)).not.toBeVisible();

    await page.evaluate(() => {
      Object.defineProperty(document, "hidden", { configurable: true, value: false });
      document.dispatchEvent(new Event("visibilitychange"));
    });
    await waitForStage(page, "准备中", 10_000);
  });

  test("F4. 客户端无消息超时后主动 close 并重连", async ({ page }) => {
    await installWorkspaceSocketProbe(page);
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E F4" });
    await openWorkspaceSession(page, seeded.sessionId);
    await expect(page.getByTestId("workspace-status-bar")).toContainText("连接 connected");
    await expect(page.getByText("Workspace 生成任务已准备").first()).toBeVisible();

    await waitForWorkspaceSocketCount(page, seeded.sessionId, 1);
    await closeActiveWorkspaceSocketFromClient(page, seeded.sessionId, 4000);
    await waitForWorkspaceSocketCount(page, seeded.sessionId, 2);
    await waitForStage(page, "准备中", 20_000);
  });

  test("F5. 服务端 idle timeout 触发 close", async ({ page }) => {
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E F5" });
    let closed = false;
    page.on("websocket", (ws) => {
      if (ws.url().includes(`/api/workspace-sessions/${seeded.sessionId}/ws`)) {
        ws.on("close", () => {
          closed = true;
        });
      }
    });

    try {
      await setWsTimeout(page, { server_idle_timeout_ms: 1000 });
      await openWorkspaceSession(page, seeded.sessionId);

      await expect.poll(() => closed, { timeout: 7_000 }).toBe(true);
    } finally {
      await setWsTimeout(page, { server_idle_timeout_ms: 90_000 });
    }
  });
});
