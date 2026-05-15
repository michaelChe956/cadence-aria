import { expect, test, type Page } from "@playwright/test";

test("project and execution workbenches stay compact across desktop and mobile", async ({
  page,
}) => {
  const issueTitle = await seedStartedIssue(page);

  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await expect(page.getByRole("main", { name: "项目管理工作台" })).toBeVisible();
  await expect(page.getByRole("region", { name: "Issue 列表面板" })).toBeVisible();
  await expect(page.getByRole("region", { name: "仓库面板" })).toBeVisible();
  await expect(page.getByText("playful coding workbench")).toHaveCount(0);
  await expect(page.getByText("AI Coding Workbench")).toHaveCount(0);
  await expectNoHorizontalOverflow(page);

  await page.getByRole("button", { name: new RegExp(issueTitle) }).click();
  await page.getByRole("button", { name: "打开执行" }).click();
  await expect(page.getByRole("main", { name: "Aria workbench" })).toBeVisible();
  await expect(page.getByRole("region", { name: "Interaction window" })).toBeVisible();
  await expect(page.getByRole("navigation", { name: "Workflow map" })).toBeVisible();
  await expect(page.getByRole("region", { name: "Provider stream" })).toBeVisible();
  await expect(page.getByText("AI Coding Workbench")).toHaveCount(0);
  await expectNoHorizontalOverflow(page);

  await page.getByRole("button", { name: /推进|Advance/ }).click();
  await expect(page.getByLabel("Provider prompt")).toBeVisible();
  await page.getByLabel("Provider prompt").fill("确认后的 fake provider prompt");
  await page.getByLabel("Policy override").selectOption("manual-all");
  await page.getByRole("button", { name: "确认执行" }).click();
  await expect(page.getByRole("button", { name: /coding_report/ })).toBeVisible();
  await expectNoHorizontalOverflow(page);

  await page.setViewportSize({ width: 375, height: 844 });
  await expect(page.getByRole("banner")).toBeVisible();
  await expect(page.getByRole("main", { name: "Aria workbench" })).toBeVisible();
  await expectNoHorizontalOverflow(page);
});

async function expectNoHorizontalOverflow(page: Page) {
  const widths = await page.evaluate(() => ({
    clientWidth: document.documentElement.clientWidth,
    scrollWidth: document.documentElement.scrollWidth,
  }));
  expect(widths.scrollWidth).toBeLessThanOrEqual(widths.clientWidth);
}

async function seedStartedIssue(page: Page) {
  await expect(
    page.request.post("/api/projects", {
      data: { name: "Aria Visual", description: "High-density workbench visual check" },
    }),
  ).resolves.toBeOK();

  const workspacesResponse = await page.request.get("/api/workspaces");
  expect(workspacesResponse).toBeOK();
  const workspacesBody = await workspacesResponse.json();
  const workspaceId = workspacesBody.workspaces[0].workspace_id;

  const issueTitle = `视觉检查 Issue ${Date.now()}`;
  const issueResponse = await page.request.post("/api/issues", {
    data: {
      title: issueTitle,
      description: "检查项目工作台与执行工作台",
      change_id: "visual-check",
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
  return issueTitle;
}
