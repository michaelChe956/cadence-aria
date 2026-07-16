import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  groupLifecycleCards,
  type LifecycleCard,
} from "../../state/lifecycle-workbench-store";
import { IssueLifecycleWorkbench } from "./IssueLifecycleWorkbench";
import {
  codingAttemptRecord,
  installIssueLifecycleWorkbenchTestHooks,
  lifecycleFetch,
} from "./IssueLifecycleWorkbench.test-utils";

vi.mock("../../state/lifecycle-workbench-store", async (importOriginal) => {
  const actual = await importOriginal<
    typeof import("../../state/lifecycle-workbench-store")
  >();
  return {
    ...actual,
    groupLifecycleCards: vi.fn(actual.groupLifecycleCards),
  };
});

vi.mock("../shared/MonacoViewer", () => ({
  MonacoViewer: ({ value }: { value: string }) => (
    <div data-testid="monaco-viewer">{value}</div>
  ),
}));

const originalGroupLifecycleCards = vi
  .mocked(groupLifecycleCards)
  .getMockImplementation();

function exposeSingleWorkItemCard() {
  if (!originalGroupLifecycleCards) {
    throw new Error("original groupLifecycleCards implementation missing");
  }
  vi.mocked(groupLifecycleCards).mockImplementation((lifecycles) => {
    const columns = originalGroupLifecycleCards(lifecycles);
    const lifecycle = lifecycles[0];
    const workItem = lifecycle?.work_items[0];
    if (!lifecycle || !workItem) {
      return columns;
    }
    const latestAttempt = lifecycle.coding_attempts.find(
      (attempt) => attempt.work_item_id === workItem.work_item_id,
    );
    const card: LifecycleCard = {
      kind: "work_item",
      id: workItem.work_item_id,
      issueId: workItem.issue_id,
      title: workItem.title,
      status: workItem.plan_status,
      version: workItem.artifact_versions.at(-1)?.version ?? null,
      preview: workItem.artifact_versions.at(-1)?.markdown ?? null,
      sourceIds: [...workItem.story_spec_ids, ...workItem.design_spec_ids],
      artifactVersions: workItem.artifact_versions,
      raw: {
        ...workItem,
        latest_attempt: latestAttempt ?? workItem.latest_attempt,
      },
    };
    return { ...columns, work_item: [card] };
  });
}

describe("IssueLifecycleWorkbench single work item coding", () => {
  installIssueLifecycleWorkbenchTestHooks();

  afterEach(() => {
    if (originalGroupLifecycleCards) {
      vi.mocked(groupLifecycleCards).mockImplementation(
        originalGroupLifecycleCards,
      );
    }
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("opens the complete address after creating a work item attempt", async () => {
    exposeSingleWorkItemCard();
    const fetchMock = lifecycleFetch({
      confirmedWorkItem: true,
      workItemPlans: [],
    });
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
    exposeSingleWorkItemCard();
    const attempt = {
      ...codingAttemptRecord("work_item_0001"),
      attempt_id: "coding_attempt_active_0001",
      status: "running" as const,
    };
    const fetchMock = lifecycleFetch({
      confirmedWorkItem: true,
      workItemPlans: [],
      codingAttempts: [attempt],
    });
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
