import { expect, test } from "@playwright/test";
import {
  enableCodingReviewFixture,
  openCodingAttempt,
  seedCodingRoleRunFixture,
} from "./helpers/coding";

test("coding role run history renders seeded runs and chat badges", async ({ page }) => {
  const seeded = await seedCodingRoleRunFixture(page, "code_review");

  await openCodingAttempt(page, seeded.attemptId);

  const history = page.getByTestId("coding-role-run-history");
  await expect(history).toContainText("Tester #1");
  await expect(history).toContainText("Code Reviewer #1");
  await expect(history).toContainText("阻塞");
  await expect(history).toContainText("provider-raw/code_review");
  await expect(history).toContainText("events");
  await expect(history).toContainText("Tester task update");
  await expect(history).toContainText("No tasks found");
  await page.reload();
  const refreshedHistory = page.getByTestId("coding-role-run-history");
  await expect(refreshedHistory).toContainText("Tester task update");
  await expect(refreshedHistory).toContainText("No tasks found");
  await expect(page.getByTestId("chat-entry-list")).toContainText("Run #1");
  await expect(page.getByTestId("coding-pending-gate")).toContainText("提交给 Coder 修复");
});

test("retry internal reviewer from browser gate stays on internal review run", async ({ page }) => {
  const seeded = await seedCodingRoleRunFixture(page, "internal_pr_review");
  await enableCodingReviewFixture(page, seeded.attemptId, {
    verdict: "approve",
    summary: "internal reviewer retry accepted",
    findings: [],
    impact_scope: ["src/lib.rs"],
    pr_description: "PR ready",
    commit_message_suggestion: "feat: work",
  });

  await openCodingAttempt(page, seeded.attemptId);
  await page.getByRole("button", { name: "重试审查" }).click();

  const history = page.getByTestId("coding-role-run-history");
  await expect(history).toContainText("Internal Reviewer #2", { timeout: 30_000 });
  await expect(history).toContainText("retry_internal_review");
  const previousInternalReviewerRun = history
    .getByRole("button")
    .filter({ hasText: "Internal Reviewer #1" });
  await expect(previousInternalReviewerRun).toContainText("Internal reviewer task update");
  await expect(history).not.toContainText("Code Reviewer #2");
  await expect(page.getByTestId("chat-entry-list")).toContainText("internal reviewer retry accepted", {
    timeout: 30_000,
  });
});
