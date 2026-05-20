import { expect, type Page } from "@playwright/test";

export interface SeededWorkspace {
  projectId: string;
  repositoryId: string;
  issueId: string;
  storyId: string;
  sessionId: string;
  projectName: string;
  storyTitle: string;
}

interface SeedOptions {
  projectName?: string;
  authorProvider?: "fake" | "claude_code" | "codex";
  reviewerProvider?: "fake" | "claude_code" | "codex";
  reviewRounds?: number;
}

export async function seedStoryWorkspace(
  page: Page,
  options: SeedOptions | string = {},
): Promise<SeededWorkspace> {
  const normalized = typeof options === "string" ? { projectName: options } : options;
  const projectName = normalized.projectName ?? "Aria E2E";
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
        name: `${uniqueProjectName} Repo`,
        path: workspacePath,
        default_policy_preset: "manual-write",
        default_provider_mode: "fake",
      },
    },
  );
  expect(repositoryResponse).toBeOK();
  const repository = await repositoryResponse.json();

  const issueTitle = `${uniqueProjectName} Issue`;
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
        author_provider: normalized.authorProvider ?? "fake",
        reviewer_provider: normalized.reviewerProvider ?? "fake",
        review_rounds: normalized.reviewRounds ?? 1,
        superpowers_enabled: false,
        openspec_enabled: true,
      },
    },
  );
  expect(storyResponse).toBeOK();
  const story = await storyResponse.json();

  return {
    projectId: project.project_id as string,
    repositoryId: repository.repository_id as string,
    issueId: issue.issue_id as string,
    storyId: story.story_specs[0].story_spec_id as string,
    sessionId: story.workspace_session.workspace_session_id as string,
    projectName: uniqueProjectName,
    storyTitle,
  };
}

export async function openWorkspaceSession(page: Page, sessionId: string): Promise<string> {
  await page.goto(`/workbench/workspace/${sessionId}`);
  await page.waitForURL(/\/workbench\/workspace\//);
  await expect(page.getByTestId("stage-badge")).toBeVisible();
  return sessionId;
}

export async function openDrawerForStory(page: Page, seeded: SeededWorkspace) {
  await page.goto(`/workbench?focus=${seeded.storyId}`);
  await expect(page.getByTestId("lifecycle-card-drawer")).toBeVisible();
  await expect(page.getByText(seeded.storyTitle)).toBeVisible();
}

export async function seedConfirmedStoryWorkspace(page: Page): Promise<SeededWorkspace> {
  const seeded = await seedStoryWorkspace(page);
  const confirmResponse = await page.request.post(
    `/api/workspace-sessions/${seeded.sessionId}/confirm`,
    {
      data: { confirmed_by: "e2e" },
    },
  );
  expect(confirmResponse).toBeOK();
  return seeded;
}

export async function waitForStage(page: Page, stageText: string, timeout = 30_000) {
  await expect(page.getByTestId("stage-badge")).toContainText(stageText, { timeout });
}

export async function sendContextNote(page: Page, content: string) {
  await page.getByTestId("context-note-input").fill(content);
  await page.getByTestId("send-context-note").click();
}

export async function clickStartGeneration(page: Page) {
  await page.getByTestId("start-generation").click();
}

export async function waitForTimelineNode(page: Page, nodeType: string, timeout = 30_000) {
  await expect(page.getByTestId(`timeline-node-${nodeType}`).first()).toBeVisible({ timeout });
}

export async function sendRawWorkspaceMessage(page: Page, sessionId: string, payload: unknown) {
  await page.evaluate(
    ({ sessionId, payload }) =>
      new Promise<void>((resolve, reject) => {
        const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
        const ws = new WebSocket(
          `${protocol}//${window.location.host}/api/workspace-sessions/${sessionId}/ws`,
        );
        ws.onopen = () => {
          ws.send(JSON.stringify(payload));
          ws.close();
          resolve();
        };
        ws.onerror = () => reject(new Error("raw workspace websocket failed"));
      }),
    { sessionId, payload },
  );
}

export async function dropWorkspaceSocketFromServer(page: Page, sessionId: string) {
  const response = await page.request.post(`/api/test/workspace-sessions/${sessionId}/ws/drop`);
  expect(response).toBeOK();
}

export async function enablePermissionFixture(page: Page, sessionId: string) {
  const response = await page.request.post(
    `/api/test/workspace-sessions/${sessionId}/permission-fixture`,
    {
      data: { mode: "single-request" },
    },
  );
  expect(response).toBeOK();
}

export async function setPermissionTimeout(page: Page, timeoutMs: number) {
  const response = await page.request.post("/api/test/permission-timeout", {
    data: { timeout_ms: timeoutMs },
  });
  expect(response).toBeOK();
}

export async function setWsTimeout(page: Page, payload: { server_idle_timeout_ms?: number }) {
  const response = await page.request.post("/api/test/ws-timeout", { data: payload });
  expect(response).toBeOK();
}
