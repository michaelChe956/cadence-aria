import { expect, test } from "@playwright/test";

test("issue lifecycle workspace is the default product flow", async ({ page }) => {
  await page.goto("/");

  await expect(page.getByRole("main", { name: "Issue 生命周期工作台" })).toBeVisible();
  await expect(page.getByRole("region", { name: "Issue 卡片列表" })).toBeVisible();
  await expect(page.getByRole("region", { name: "Issue 生命周期详情" })).toBeVisible();
});
