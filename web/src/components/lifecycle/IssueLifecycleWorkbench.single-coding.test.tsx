import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CodingAttempt, IssueLifecycleResponse, LifecycleWorkItem } from "../../api/types";
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

function singleWorkItemFetch(
  latestAttempt: CodingAttempt | null = null,
  overrides: Partial<LifecycleWorkItem> = {},
) {
  const workItem = workItemRecord({
    plan_status: "confirmed",
    latest_attempt: latestAttempt,
    ...overrides,
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

  it.each([undefined, null])("v1 Work Item 的计划归属为 %s 时保留三个动作入口", async (sourcePlanId) => {
    vi.stubGlobal("fetch", singleWorkItemFetch(null, {
      ...(sourcePlanId === undefined ? {} : { source_work_item_plan_id: sourcePlanId }),
    }));
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await user.click(await screen.findByRole("button", { name: "实现提示组件" }));

    expect(screen.getByRole("button", { name: "打开 Workspace" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "开始 Coding" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "删除 Work Item" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "删除 Work Item 实现提示组件" })).toBeInTheDocument();
  });

  it.each([false, true])("v2 Work Item 仅展示详情，不因已有 Attempt=%s 恢复三个动作入口", async (hasAttempt) => {
    const fetchMock = singleWorkItemFetch(
      hasAttempt ? codingAttemptRecord("work_item_0001") : null,
      { source_work_item_plan_id: "issue_plan_0001" },
    );
    vi.stubGlobal("fetch", fetchMock);
    const user = userEvent.setup();

    render(<IssueLifecycleWorkbench />);
    await user.click(await screen.findByRole("button", { name: "实现提示组件" }));

    const drawer = screen.getByTestId("lifecycle-card-drawer");
    expect(drawer).toHaveTextContent("实现提示组件");
    expect(drawer).toHaveTextContent("work_item_0001");
    expect(screen.queryByRole("button", { name: "删除 Work Item 实现提示组件" })).toBeNull();
    expect(screen.queryByTestId("drawer-open-workspace")).toBeNull();
    expect(screen.queryByTestId("drawer-open-coding-workspace")).toBeNull();
    expect(screen.queryByTestId("drawer-delete-work-item")).toBeNull();
    await user.click(screen.getByRole("button", { name: "查看 Markdown 内容" }));
    expect(screen.getByTestId("monaco-viewer")).toHaveTextContent("实现会话过期提示组件");
    expect(fetchMock).not.toHaveBeenCalledWith(
      expect.stringMatching(/\/coding-attempts$/u),
      expect.objectContaining({ method: "POST" }),
    );
  });

  it.each([undefined, null])("v1 计划归属为 %s 时仍调用单项端点创建 Coding Attempt", async (sourcePlanId) => {
    const fetchMock = singleWorkItemFetch(null, {
      ...(sourcePlanId === undefined ? {} : { source_work_item_plan_id: sourcePlanId }),
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

  it.each([undefined, null])("v1 计划归属为 %s 时复用已有单项 Coding Attempt", async (sourcePlanId) => {
    const attempt = {
      ...codingAttemptRecord("work_item_0001"),
      attempt_id: "coding_attempt_active_0001",
      status: "running" as const,
    };
    const fetchMock = singleWorkItemFetch(attempt, {
      ...(sourcePlanId === undefined ? {} : { source_work_item_plan_id: sourcePlanId }),
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
