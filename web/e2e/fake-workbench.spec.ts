import { expect, test } from "@playwright/test";
import { seedStoryWorkspace, waitForStage } from "./helpers/workspace";

test("fake provider workspace streams a story spec and confirms lifecycle state", async ({
  page,
}) => {
  const seeded = await seedStoryWorkspace(page, "Aria E2E");

  await page.goto(`/workbench/workspace/${seeded.sessionId}`);

  await expect(page.getByText("Story Spec").first()).toBeVisible();
  await expect(page.getByText("Author: Fake")).toBeVisible();
  await expect(page.getByText("Reviewer: Fake")).toBeVisible();

  await expect(page.getByTestId("prepare-context-panel")).toBeVisible();
  const contextInput = page.getByTestId("context-note-input");
  await expect(contextInput).toBeEnabled();
  await contextInput.fill("请生成 Story Spec 和验收标准");
  await page.getByTestId("send-context-note").click();

  await expect(page.getByText("请生成 Story Spec 和验收标准").first()).toBeVisible();
  await expect(page.getByTestId("timeline-node-context_note")).toBeVisible();
  await page.getByTestId("start-generation").click();
  await waitForStage(page, "等待确认");
  const humanConfirmPanel = page.getByTestId("human-confirm-panel");
  await expect(humanConfirmPanel.getByRole("button", { name: "确认" })).toBeVisible();
  await humanConfirmPanel.getByRole("button", { name: "确认" }).click();

  await expect(page.getByTestId("stage-badge")).toContainText("已完成");

  await page.getByRole("button", { name: "返回" }).click();
  const projectButton = page.getByRole("button", { name: seeded.projectName, exact: true });
  await expect(projectButton).toBeEnabled();
  await projectButton.click();
  const storyColumn = page.getByRole("region", { name: "Story Spec 列" });
  await expect(storyColumn).toContainText(seeded.storyTitle);
  await expect(storyColumn).toContainText("confirmed");
});
