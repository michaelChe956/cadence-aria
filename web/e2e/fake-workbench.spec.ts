import { expect, test, type Page } from "@playwright/test";

test("fake provider workspace streams a story spec and confirms lifecycle state", async ({
  page,
}) => {
  const seeded = await seedStoryWorkspace(page, "Aria E2E");

  await page.goto(`/workbench/workspace/${seeded.sessionId}`);

  await expect(page.getByText("Story Spec").first()).toBeVisible();
  await expect(page.getByText("Author: fake | Reviewer: fake")).toBeVisible();

  await expect(page.getByTestId("prepare-context-panel")).toBeVisible();
  const contextInput = page.getByTestId("context-note-input");
  await expect(contextInput).toBeEnabled();
  await contextInput.fill("请生成 Story Spec 和验收标准");
  await page.getByTestId("send-context-note").click();

  await expect(page.getByText("请生成 Story Spec 和验收标准").first()).toBeVisible();
  await expect(page.getByTestId("timeline-node-context_note")).toBeVisible();
  await page.getByTestId("start-generation").click();
  await expect(page.getByRole("button", { name: "确认通过" })).toBeVisible();
  await page.getByRole("button", { name: "确认通过" }).click();

  await expect(page.getByTestId("stage-badge")).toContainText("已完成");

  await page.getByRole("button", { name: "返回" }).click();
  const projectButton = page.getByRole("button", { name: seeded.projectName, exact: true });
  await expect(projectButton).toBeEnabled();
  await projectButton.click();
  const storyColumn = page.getByRole("region", { name: "Story Spec 列" });
  await expect(storyColumn).toContainText(seeded.storyTitle);
  await expect(storyColumn).toContainText("confirmed");
});

async function seedStoryWorkspace(page: Page, projectName: string) {
  const uniqueProjectName = `${projectName} ${Date.now()}`;
  const projectResponse = await page.request.post("/api/projects", {
    data: { name: uniqueProjectName, description: "Lifecycle workspace E2E" },
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

  const issueTitle = `${projectName} Story ${Date.now()}`;
  const issueResponse = await page.request.post(`/api/projects/${project.project_id}/issues`, {
    data: {
      title: issueTitle,
      description: "验证 Issue 生命周期 Workspace",
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
        reviewer_provider: "fake",
        review_rounds: 1,
        superpowers_enabled: false,
        openspec_enabled: true,
      },
    },
  );
  expect(storyResponse).toBeOK();
  const story = await storyResponse.json();

  return {
    projectId: project.project_id as string,
    issueId: issue.issue_id as string,
    projectName: uniqueProjectName,
    sessionId: story.workspace_session.workspace_session_id as string,
    storyTitle,
  };
}
