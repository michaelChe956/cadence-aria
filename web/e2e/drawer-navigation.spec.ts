import { expect, test, type Page } from "@playwright/test";
import {
  openDrawerForStory,
  seedConfirmedStoryWorkspace,
  seedStoryWorkspace,
} from "./helpers/workspace";

test.describe("C. 看板侧滑详情", () => {
  test("C1. 卡片点击打开 Drawer", async ({ page }) => {
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E C1" });

    await page.goto("/workbench");
    await page.getByRole("button", { name: seeded.projectName, exact: true }).click();
    await expect(page.getByRole("region", { name: "Story Spec 内容" })).toContainText(
      seeded.storyTitle,
    );
    await page.getByText(seeded.storyTitle).click();

    await expect(page.getByTestId("lifecycle-card-drawer")).toBeVisible();
    await expect(page).toHaveURL(/focus=/);
  });

  test("C2. 关闭 Drawer URL 清除", async ({ page }) => {
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E C2" });

    await openDrawerForStory(page, seeded);
    await page.getByLabel("关闭").click();

    await expect(page).not.toHaveURL(/focus=/);
    await expect(page.getByTestId("lifecycle-card-drawer")).toHaveCount(0);
  });

  test("C3. Story confirmed 后在 Drawer 生成 Design Spec", async ({ page }) => {
    const seeded = await seedConfirmedStoryWorkspace(page);

    await openDrawerForStory(page, seeded);
    await page.getByTestId("drawer-generate-next").click();

    const drawer = page.getByTestId("lifecycle-card-drawer");
    await expect(drawer).toContainText("Design Spec");
    await expect(page).toHaveURL(/\/workbench\?focus=design_spec/);
    await expect(page).not.toHaveURL(/\/workbench\/workspace\//);
    await expect(page.getByTestId("drawer-open-workspace")).toBeVisible();
  });

  test("C4. Drawer 内打开 Workspace", async ({ page }) => {
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E C4" });

    await openDrawerForStory(page, seeded);
    await page.getByTestId("drawer-open-workspace").click();

    await page.waitForURL(/\/workbench\/workspace\//);
    await expect(page.getByTestId("stage-badge")).toContainText("准备中");
  });

  test("C5. URL 直接访问 focus 自动打开 Drawer", async ({ page }) => {
    await deleteAllProjects(page);
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E C5" });

    await page.goto(`/workbench?focus=${seeded.storyId}`);

    await expect(page.getByTestId("lifecycle-card-drawer")).toBeVisible();
    await expect(page.getByTestId("lifecycle-card-drawer")).toContainText(seeded.storyTitle);
  });

  test("C6. Drawer 启动 Workspace 不出现空白态", async ({ page }) => {
    const seeded = await seedStoryWorkspace(page, { projectName: "Aria E2E C6" });

    await openDrawerForStory(page, seeded);
    await page.getByTestId("drawer-open-workspace").click();

    await page.waitForURL(/\/workbench\/workspace\//);
    await expect(page.getByText("Story Spec").first()).toBeVisible();
    await expect(page.getByTestId("stage-badge")).toContainText("准备中");
  });
});

async function deleteAllProjects(page: Page) {
  const response = await page.request.get("/api/projects");
  expect(response).toBeOK();
  const body = (await response.json()) as { projects: Array<{ project_id: string }> };
  for (const project of body.projects) {
    const deleteResponse = await page.request.delete(`/api/projects/${project.project_id}`);
    expect(deleteResponse).toBeOK();
  }
}
