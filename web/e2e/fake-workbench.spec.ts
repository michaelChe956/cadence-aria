import { expect, test } from "@playwright/test";

test("fake provider workbench creates, confirms, observes, rolls back and reruns", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByRole("banner")).toContainText("Aria Web");
  await expect(page.getByRole("navigation", { name: "Node flow" })).toBeVisible();
  await expect(page.getByRole("main")).toContainText("Node Workspace");

  await page.getByLabel("任务请求").fill("实现 Fibonacci square sum");
  await page.getByLabel("change id").fill("aria-fibonacci-square");
  await page.getByLabel("provider mode").selectOption("fake");
  await page.getByRole("button", { name: "新建任务" }).click();

  await page.getByRole("button", { name: /推进|Advance/ }).click();
  await expect(page.getByLabel("Provider prompt")).toBeVisible();
  await page.getByLabel("Provider prompt").fill("确认后的 fake provider prompt");
  await page.getByLabel("Policy override").selectOption("manual-all");
  await page.getByRole("button", { name: "确认执行" }).click();

  await expect(page.getByRole("main").getByText("provider_output")).toBeVisible();
  await expect(page.getByRole("button", { name: /coding_report/ })).toBeVisible();
  await page.getByRole("button", { name: /回退/ }).click();
  await expect(page.getByRole("dialog")).toContainText(/checkpoint|Checkpoint/);
  await page.getByRole("button", { name: "执行回退" }).click();

  await expect(page.getByRole("button", { name: /N16 dropped/ })).toBeVisible();
  await expect(page.getByLabel("Provider prompt")).toBeVisible();
});
