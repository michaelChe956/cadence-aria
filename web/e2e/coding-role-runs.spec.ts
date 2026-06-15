import { expect, test } from "@playwright/test";
import {
  enableCodingReviewFixture,
  openCodingAttempt,
  seedCodingRoleRunFixture,
} from "./helpers/coding";

test("coding role run history renders seeded runs and chat badges", async ({ page }) => {
  const seeded = await seedCodingRoleRunFixture(page, "rework");

  await openCodingAttempt(page, seeded.attemptId);

  const history = page.getByTestId("coding-role-run-history");
  await expect(history).toContainText("Tester #1");
  await expect(history).toContainText("Analyst #1");
  await expect(history).toContainText("阻塞");
  await expect(history).toContainText("provider-raw/rework/analyst_evidence");
  await expect(history).toContainText("events");
  await expect(history).toContainText("Tester task update");
  await expect(history).toContainText("No tasks found");
  await page.reload();
  const refreshedHistory = page.getByTestId("coding-role-run-history");
  await expect(refreshedHistory).toContainText("Tester task update");
  await expect(refreshedHistory).toContainText("No tasks found");
  await expect(page.getByTestId("chat-entry-list")).toContainText("Run #1");
  await expect(page.getByTestId("coding-pending-gate")).toContainText("重试 Analyst");
});

test("retry analyst from browser gate creates a new visible run", async ({ page }) => {
  const seeded = await seedCodingRoleRunFixture(page, "rework");
  await enableCodingReviewFixture(page, seeded.attemptId, {
    verdict: "proceed",
    next_stage: "code_review",
    reason: "retry analyst accepted from browser",
    evidence_refs: ["provider-raw/rework/analyst_evidence_0001.txt"],
    raw_provider_output_refs: [],
  });

  await openCodingAttempt(page, seeded.attemptId);
  await page.getByRole("button", { name: "重试 Analyst" }).click();

  const history = page.getByTestId("coding-role-run-history");
  await expect(history).toContainText("Analyst #2", { timeout: 30_000 });
  await expect(history).toContainText("retry_analyst");
  const previousAnalystRun = history.getByRole("button").filter({ hasText: "Analyst #1" });
  await expect(previousAnalystRun).toContainText("Analyst task update");
  await expect(page.getByTestId("chat-entry-list")).toContainText("retry analyst accepted from browser", {
    timeout: 30_000,
  });
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
