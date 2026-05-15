import { expect, test, type Page } from "@playwright/test";

test("fake provider workbench opens a started issue, confirms, observes, rolls back and reruns", async ({
  page,
}) => {
  await seedStartedIssue(page);

  await page.goto("/");
  await expect(page.getByRole("banner")).toContainText("Aria Web");
  await expect(page.getByRole("main", { name: "项目管理工作台" })).toBeVisible();
  await expect(page.getByRole("button", { name: "打开执行" })).toBeVisible();
  await page.getByRole("button", { name: "打开执行" }).click();

  await expect(page.getByRole("navigation", { name: "Workflow map" })).toBeVisible();
  await expect(page.getByRole("main")).toContainText("Workspace");
  await expect(page.getByRole("region", { name: "Provider stream" })).toBeVisible();

  await page.getByRole("button", { name: /推进|Advance/ }).click();
  await expect(page.getByLabel("Provider prompt")).toBeVisible();
  await page.getByLabel("Provider prompt").fill("确认后的 fake provider prompt");
  await page.getByLabel("Policy override").selectOption("manual-all");
  await page.getByRole("button", { name: "确认执行" }).click();

  const providerStream = page.getByRole("region", { name: "Provider stream" });
  await expect(providerStream.getByText("provider_output", { exact: true })).not.toHaveCount(0);
  await expect(page.getByRole("button", { name: /coding_report/ })).toBeVisible();
  await page.getByRole("button", { name: /回退/ }).click();
  await expect(page.getByRole("dialog")).toContainText(/checkpoint|Checkpoint/);
  await page.getByRole("button", { name: "执行回退" }).click();

  await expect(
    page.getByRole("navigation", { name: "Workflow map" }).getByRole("button", { name: /N16 dropped/ }).first(),
  ).toBeVisible();
  await expect(page.getByLabel("Provider prompt")).toBeVisible();
});

async function seedStartedIssue(page: Page) {
  await expect(
    page.request.post("/api/projects", {
      data: { name: "Aria E2E", description: "Project workbench E2E" },
    }),
  ).resolves.toBeOK();

  const workspacesResponse = await page.request.get("/api/workspaces");
  expect(workspacesResponse).toBeOK();
  const workspacesBody = await workspacesResponse.json();
  const workspaceId = workspacesBody.workspaces[0].workspace_id;

  const issueResponse = await page.request.post("/api/issues", {
    data: {
      title: "实现 Fibonacci square sum",
      description: "实现 Fibonacci square sum",
      change_id: "aria-fibonacci-square",
    },
  });
  expect(issueResponse).toBeOK();
  const issue = await issueResponse.json();

  const startResponse = await page.request.post(`/api/issues/${issue.issue_id}/start`, {
    data: {
      workspace_id: workspaceId,
      policy_preset: "manual-write",
      provider_mode: "fake",
      timeout_secs: 2400,
    },
  });
  expect(startResponse).toBeOK();
}
