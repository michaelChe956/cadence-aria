import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CodingAttempt, IssueLifecycleResponse } from "../../api/types";
import { IssueLifecycleWorkbench } from "./IssueLifecycleWorkbench";
import {
  codingAttemptRecord,
  installIssueLifecycleWorkbenchTestHooks,
  jsonResponse,
  lifecycleFetch,
  workItemRecord,
  workItemRepositoryGroupRecord,
} from "./IssueLifecycleWorkbench.test-utils";


vi.mock("../shared/MonacoViewer", () => ({
  MonacoViewer: ({ value }: { value: string }) => (
    <div data-testid="monaco-viewer">{value}</div>
  ),
}));

function singleWorkItemFetch(latestAttempt: CodingAttempt | null = null) {
  const workItem = workItemRecord({
    plan_status: "confirmed",
    latest_attempt: latestAttempt,
  });
  const fetchLifecycle = lifecycleFetch({
    confirmedWorkItem: true,
    workItemPlans: [],
    codingAttempts: latestAttempt ? [latestAttempt] : [],
    workItemRepositoryGroups: [
      workItemRepositoryGroupRecord({ items: [workItem] }),
    ],
  });
  return vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const response = await fetchLifecycle(input, init);
    if (String(input) !== "/api/issues/issue_0001/lifecycle?project_id=project_0001") {
      return response;
    }
    const lifecycle = (await response.json()) as IssueLifecycleResponse;
    // 与 API 投影一致：单项最新 attempt 属于 work_items 实体，而非展示列 mock。
    return jsonResponse({ ...lifecycle, work_items: [workItem] });
  });
}

describe("IssueLifecycleWorkbench single work item coding", () => {
  installIssueLifecycleWorkbenchTestHooks();

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("opens the complete address after creating a work item attempt", async () => {
    const fetchMock = singleWorkItemFetch();
    vi.stubGlobal("fetch", fetchMock);
    const onOpenCodingWorkspace = vi.fn();
    const user = userEvent.setup();

    render(
      <IssueLifecycleWorkbench onOpenCodingWorkspace={onOpenCodingWorkspace} />,
    );

    await user.click(
      await screen.findByRole("button", { name: "实现提示组件" }),
    );
    await user.click(screen.getByTestId("drawer-open-coding-workspace"));

    await waitFor(() =>
      expect(onOpenCodingWorkspace).toHaveBeenCalledWith({
        projectId: "project_0001",
        issueId: "issue_0001",
        attemptId: "coding_attempt_0001",
      }),
    );
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
      expect.objectContaining({ method: "POST" }),
    );
  });

  it("opens the complete address when reusing a work item attempt", async () => {
    const attempt = {
      ...codingAttemptRecord("work_item_0001"),
      attempt_id: "coding_attempt_active_0001",
      status: "running" as const,
    };
    const fetchMock = singleWorkItemFetch(attempt);
    vi.stubGlobal("fetch", fetchMock);
    const onOpenCodingWorkspace = vi.fn();
    const user = userEvent.setup();

    render(
      <IssueLifecycleWorkbench onOpenCodingWorkspace={onOpenCodingWorkspace} />,
    );

    await user.click(
      await screen.findByRole("button", { name: "实现提示组件" }),
    );
    await user.click(screen.getByTestId("drawer-open-coding-workspace"));

    await waitFor(() =>
      expect(onOpenCodingWorkspace).toHaveBeenCalledWith({
        projectId: "project_0001",
        issueId: "issue_0001",
        attemptId: "coding_attempt_active_0001",
      }),
    );
    expect(fetchMock).not.toHaveBeenCalledWith(
      "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
      expect.objectContaining({ method: "POST" }),
    );
  });
});
