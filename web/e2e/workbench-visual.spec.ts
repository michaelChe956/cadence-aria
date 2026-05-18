import { expect, test, type Page } from "@playwright/test";

test("lifecycle and workspace surfaces stay compact across desktop and mobile", async ({
  page,
}) => {
  const seeded = await seedStoryWorkspace(page, "Aria Visual");

  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await expect(page.getByRole("main", { name: "Issue 生命周期工作台" })).toBeVisible();
  await expect(page.getByRole("navigation", { name: "Project 切换" })).toBeVisible();
  const projectButton = page.getByRole("button", { name: seeded.projectName, exact: true });
  await expect(projectButton).toBeEnabled();
  await projectButton.click();
  await expect(page.getByRole("region", { name: "Issue 列" })).toContainText(
    seeded.issueTitle,
  );
  await expect(page.getByRole("region", { name: "Story Spec 列" })).toContainText(
    seeded.storyTitle,
  );
  await expect(page.getByText("playful coding workbench")).toHaveCount(0);
  await expect(page.getByText("AI Coding Workbench")).toHaveCount(0);
  await expectNoHorizontalOverflow(page);

  await page.getByRole("button", { name: `打开 Workspace ${seeded.storyTitle}` }).click();
  await expect(page.getByText("Story Spec").first()).toBeVisible();
  await expect(page.getByText("Author: fake | Reviewer: codex")).toBeVisible();
  await expect(page.getByText("AI Coding Workbench")).toHaveCount(0);
  await expectNoHorizontalOverflow(page);

  const prompt = page.getByPlaceholder("输入消息...");
  await expect(prompt).toBeEnabled();
  await prompt.fill("视觉检查 fake provider prompt");
  await prompt.press("Enter");
  await expect(page.getByRole("button", { name: "确认通过" })).toBeVisible();
  await expectNoHorizontalOverflow(page);

  await page.setViewportSize({ width: 375, height: 844 });
  await expect(page.getByRole("banner")).toBeVisible();
  await expect(page.getByText("Story Spec").first()).toBeVisible();
  await expectNoHorizontalOverflow(page);
});

async function expectNoHorizontalOverflow(page: Page) {
  const widths = await page.evaluate(() => ({
    clientWidth: document.documentElement.clientWidth,
    scrollWidth: document.documentElement.scrollWidth,
  }));
  expect(widths.scrollWidth).toBeLessThanOrEqual(widths.clientWidth);
}

async function seedStoryWorkspace(page: Page, projectName: string) {
  const uniqueProjectName = `${projectName} ${Date.now()}`;
  const projectResponse = await page.request.post("/api/projects", {
    data: { name: uniqueProjectName, description: "High-density lifecycle visual check" },
  });
  expect(projectResponse).toBeOK();
  const project = await projectResponse.json();

  const workspacesResponse = await page.request.get("/api/workspaces");
  expect(workspacesResponse).toBeOK();
  const workspacesBody = await workspacesResponse.json();
  const workspacePath = workspacesBody.workspaces[0].path;

  const repositoryResponse = await page.request.post(
    `/api/projects/${project.project_id}/repositories`,
    {
      data: {
        name: `${projectName} Repo`,
        path: workspacePath,
        default_policy_preset: "manual-write",
        default_provider_mode: "fake",
      },
    },
  );
  expect(repositoryResponse).toBeOK();
  const repository = await repositoryResponse.json();

  const issueTitle = `${projectName} Issue ${Date.now()}`;
  const issueResponse = await page.request.post(`/api/projects/${project.project_id}/issues`, {
    data: {
      title: issueTitle,
      description: "检查生命周期工作台与 Workspace",
      repository_id: repository.repository_id,
    },
  });
  expect(issueResponse).toBeOK();
  const issue = await issueResponse.json();

  const storyTitle = `${issueTitle} Story Spec`;
  const storyResponse = await page.request.post(
    `/api/projects/${project.project_id}/issues/${issue.issue_id}/story-specs:generate`,
    {
      data: {
        title: storyTitle,
        author_provider: "fake",
        reviewer_provider: "codex",
        review_rounds: 1,
        superpowers_enabled: false,
        openspec_enabled: true,
      },
    },
  );
  expect(storyResponse).toBeOK();

  return { issueTitle, storyTitle, projectName: uniqueProjectName };
}
