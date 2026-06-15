import { expect, type Page } from "@playwright/test";

export async function seedCodingRoleRunFixture(
  page: Page,
  blockedStage: "rework" | "internal_pr_review" = "rework",
): Promise<{ attemptId: string; projectId: string; issueId: string }> {
  const response = await page.request.post("/api/test/coding-attempts/role-run-fixture", {
    data: { blocked_stage: blockedStage },
  });
  expect(response).toBeOK();
  const body = await response.json();
  if (body.status !== "ok") {
    // eslint-disable-next-line no-console
    console.error("seed fixture failed", body);
  }
  expect(body.status).toBe("ok");
  return {
    attemptId: body.attempt_id as string,
    projectId: body.project_id as string,
    issueId: body.issue_id as string,
  };
}

export async function enableCodingReviewFixture(page: Page, attemptId: string, rawJson: unknown) {
  const response = await page.request.post(
    `/api/test/coding-attempts/${encodeURIComponent(attemptId)}/review-fixture`,
    {
      data: {
        verdict: "approve",
        summary: "fixture",
        comments: "fixture",
        raw_json: rawJson,
      },
    },
  );
  expect(response).toBeOK();
}

export async function openCodingAttempt(page: Page, attemptId: string) {
  await page.goto(`/workbench/coding/${encodeURIComponent(attemptId)}`);
  await expect(page.getByText(`Coding Attempt #${attemptId}`)).toBeVisible();
  await expect(page.getByTestId("coding-role-run-history")).toBeVisible();
}
