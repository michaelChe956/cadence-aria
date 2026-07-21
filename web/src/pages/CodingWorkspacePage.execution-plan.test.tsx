import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import {
  confirmWorkItemExecutionPlan,
  deleteCodingAttempt,
  getCodingAttemptDiff,
  requestWorkItemExecutionPlanChange,
} from "../api/client";
import { useCodingWorkspaceWs } from "../hooks/useCodingWorkspaceWs";
import { useCodingWorkspaceStore } from "../state/coding-workspace-store";
import { CodingWorkspacePage } from "./CodingWorkspacePage";
import {
  CODING_ATTEMPT_ADDRESS,
  DEFAULT_PERMISSION_MODES,
  deferred,
  executionPlan,
  installCodingWorkspacePageTestHooks,
  mockCodingWs,
  readyCodingState,
} from "./CodingWorkspacePage.test-utils";

vi.mock("../api/client", () => ({
  confirmWorkItemExecutionPlan: vi.fn(),
  deleteCodingAttempt: vi.fn(),
  getCodingAttemptDiff: vi.fn(),
  requestWorkItemExecutionPlanChange: vi.fn(),
}));

vi.mock("../hooks/useCodingWorkspaceWs", () => ({
  useCodingWorkspaceWs: vi.fn(),
}));

vi.mock("../hooks/useUnloadGuard", () => ({
  useUnloadGuard: vi.fn(),
}));

vi.mock("../components/shared/MonacoViewer", () => ({
  MonacoViewer: ({
    value,
    language,
    height,
  }: {
    value: string;
    language?: string;
    height?: string;
  }) => (
    <div data-testid="monaco-viewer" data-language={language} data-height={height}>
      {value}
    </div>
  ),
}));

vi.mock("../components/shared/MonacoDiffViewer", () => ({
  MonacoDiffViewer: ({
    original,
    modified,
    language,
    height,
  }: {
    original: string;
    modified: string;
    language?: string;
    height?: string;
  }) => (
    <div data-testid="monaco-diff-viewer" data-language={language} data-height={height}>
      <span data-testid="monaco-diff-original">{original}</span>
      <span data-testid="monaco-diff-modified">{modified}</span>
    </div>
  ),
}));

describe("CodingWorkspacePage execution plan", () => {
  installCodingWorkspacePageTestHooks();

  const OTHER_SCOPE_SAME_ATTEMPT_ADDRESS = {
    projectId: "project_0002",
    issueId: "issue_0002",
    attemptId: CODING_ATTEMPT_ADDRESS.attemptId,
  } as const;

  it("opens role run history in a drawer instead of constraining the conversation column", async () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "waiting_for_human",
      stage: "code_review",
      roleRuns: Array.from({ length: 12 }, (_, index) => ({
        id: `coding_role_run_${String(index + 1).padStart(4, "0")}`,
        attempt_id: "coding_attempt_0001",
        stage: index % 2 === 0 ? "coding" : "code_review",
        role: index % 2 === 0 ? "coder" : "code_reviewer",
        run_no: index + 1,
        status: index % 3 === 0 ? "blocked" : "completed",
        trigger: index % 2 === 0 ? "initial" : "retry_review",
        node_id: `coding_node_${String(index + 1).padStart(4, "0")}`,
        started_at: `2026-06-13T00:00:${String(index).padStart(2, "0")}Z`,
        completed_at: null,
        supersedes_run_id: null,
        superseded_by_run_id: null,
        reason_code: "code_review_blocked",
        raw_provider_output_refs: [
          "provider-raw/code-review/very-long-role-run-output-reference-that-must-not-widen-page.txt",
        ],
        artifact_refs: [
          "artifacts/code-review/very-long-reviewer-evidence-reference-that-must-scroll-inside-panel.json",
        ],
      })),
    });

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

    expect(screen.getByTestId("coding-chat-entry-list")).toBeInTheDocument();
    expect(screen.queryByTestId("coding-role-run-history")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "角色运行历史" }));

    const panel = screen.getByTestId("coding-role-run-history");
    expect(panel).toHaveClass("min-w-0", "overflow-hidden");
    expect(screen.getByRole("dialog", { name: "角色运行历史" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "提交给 Coder 修复" })).not.toBeInTheDocument();
  });

  it("shows work item execution plan during prepare stage as non blocking by default", () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      stage: "prepare_context",
      workItemExecutionPlan: executionPlan({ status: "draft" }),
    });

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

    expect(screen.getByText("执行计划")).toBeInTheDocument();
    expect(screen.getByText("实现后端 API")).toBeInTheDocument();
    expect(screen.getByText("src/product/**")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "确认执行计划" })).not.toBeInTheDocument();
  });

  it("shows confirm and change request actions when execution plan confirmation is required", () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      stage: "prepare_context",
      // 门禁开关来自 work item / snapshot 的 require_execution_plan_confirm，
      // 而非 plan 对象自身字段。
      requireExecutionPlanConfirm: true,
      workItemExecutionPlan: executionPlan({
        status: "draft",
      }),
    });

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

    expect(screen.getByRole("button", { name: "确认执行计划" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "请求修改" })).toBeInTheDocument();
  });

  it("confirms execution plan and updates store", async () => {
    const user = userEvent.setup();
    mockCodingWs();
    vi.mocked(confirmWorkItemExecutionPlan).mockResolvedValue(
      executionPlan({ status: "confirmed" }),
    );
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      requireExecutionPlanConfirm: true,
      workItemExecutionPlan: executionPlan({ status: "draft" }),
    });

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

    await user.click(screen.getByRole("button", { name: "确认执行计划" }));

    expect(confirmWorkItemExecutionPlan).toHaveBeenCalledWith(
      CODING_ATTEMPT_ADDRESS,
    );
    expect(useCodingWorkspaceStore.getState().workItemExecutionPlan?.status).toBe("confirmed");
  });

  it("requests execution plan change and updates store", async () => {
    const user = userEvent.setup();
    mockCodingWs();
    vi.mocked(requestWorkItemExecutionPlanChange).mockResolvedValue(
      executionPlan({ status: "change_requested" }),
    );
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      requireExecutionPlanConfirm: true,
      workItemExecutionPlan: executionPlan({ status: "draft" }),
    });

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

    await user.type(screen.getByLabelText("修改说明"), "需要补充边界条件测试");
    await user.click(screen.getByRole("button", { name: "请求修改" }));

    expect(requestWorkItemExecutionPlanChange).toHaveBeenCalledWith(
      CODING_ATTEMPT_ADDRESS,
      {
        note: "需要补充边界条件测试",
      },
    );
    expect(useCodingWorkspaceStore.getState().workItemExecutionPlan?.status).toBe(
      "change_requested",
    );
  });

  it("shows page error when confirming execution plan fails", async () => {
    const user = userEvent.setup();
    mockCodingWs();
    vi.mocked(confirmWorkItemExecutionPlan).mockRejectedValue(new Error("confirm failed"));
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      requireExecutionPlanConfirm: true,
      workItemExecutionPlan: executionPlan({ status: "draft" }),
    });

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

    await user.click(screen.getByRole("button", { name: "确认执行计划" }));

    expect(screen.getByText("confirm failed")).toBeInTheDocument();
    expect(useCodingWorkspaceStore.getState().workItemExecutionPlan?.status).toBe("draft");
  });

  it("shows page error when requesting change with empty note", async () => {
    const user = userEvent.setup();
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      requireExecutionPlanConfirm: true,
      workItemExecutionPlan: executionPlan({ status: "draft" }),
    });

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

    await user.click(screen.getByRole("button", { name: "请求修改" }));

    expect(requestWorkItemExecutionPlanChange).not.toHaveBeenCalled();
    expect(screen.getByText("请填写修改说明")).toBeInTheDocument();
  });

  it("shows page error when requesting execution plan change fails", async () => {
    const user = userEvent.setup();
    mockCodingWs();
    vi.mocked(requestWorkItemExecutionPlanChange).mockRejectedValue(new Error("change failed"));
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      requireExecutionPlanConfirm: true,
      workItemExecutionPlan: executionPlan({ status: "draft" }),
    });

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

    await user.type(screen.getByLabelText("修改说明"), "说明");
    await user.click(screen.getByRole("button", { name: "请求修改" }));

    expect(screen.getByText("change failed")).toBeInTheDocument();
    expect(useCodingWorkspaceStore.getState().workItemExecutionPlan?.status).toBe("draft");
  });

  it.each(["resolve", "reject"] as const)(
    "ignores a stale execution plan confirm %s after switching the full address",
    async (outcome) => {
      const user = userEvent.setup();
      mockCodingWs();
      const pending = deferred<Awaited<ReturnType<typeof confirmWorkItemExecutionPlan>>>();
      vi.mocked(confirmWorkItemExecutionPlan).mockReturnValue(pending.promise);
      const planA = executionPlan({ status: "draft", goal: "A plan" });
      const planB = executionPlan({
        id: "work_item_execution_plan_b",
        project_id: OTHER_SCOPE_SAME_ATTEMPT_ADDRESS.projectId,
        issue_id: OTHER_SCOPE_SAME_ATTEMPT_ADDRESS.issueId,
        attempt_id: OTHER_SCOPE_SAME_ATTEMPT_ADDRESS.attemptId,
        status: "draft",
        goal: "B plan",
      });
      useCodingWorkspaceStore.setState({
        ...readyCodingState(),
        requireExecutionPlanConfirm: true,
        workItemExecutionPlan: planA,
      });
      const view = render(
        <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />,
      );

      await user.click(screen.getByRole("button", { name: "确认执行计划" }));
      useCodingWorkspaceStore.setState({
        ...readyCodingState(),
        projectId: OTHER_SCOPE_SAME_ATTEMPT_ADDRESS.projectId,
        issueId: OTHER_SCOPE_SAME_ATTEMPT_ADDRESS.issueId,
        attemptId: OTHER_SCOPE_SAME_ATTEMPT_ADDRESS.attemptId,
        requireExecutionPlanConfirm: true,
        workItemExecutionPlan: planB,
      });
      view.rerender(
        <CodingWorkspacePage address={OTHER_SCOPE_SAME_ATTEMPT_ADDRESS} onBack={vi.fn()} />,
      );

      await act(async () => {
        if (outcome === "resolve") {
          pending.resolve(executionPlan({ status: "confirmed", goal: "stale A result" }));
        } else {
          pending.reject(new Error("stale A confirm error"));
        }
        await pending.promise.catch(() => undefined);
      });

      const state = useCodingWorkspaceStore.getState();
      expect(state.projectId).toBe(OTHER_SCOPE_SAME_ATTEMPT_ADDRESS.projectId);
      expect(state.issueId).toBe(OTHER_SCOPE_SAME_ATTEMPT_ADDRESS.issueId);
      expect(state.workItemExecutionPlan).toEqual(planB);
      expect(screen.queryByText("stale A result")).not.toBeInTheDocument();
      expect(screen.queryByText("stale A confirm error")).not.toBeInTheDocument();
    },
  );

  it.each(["resolve", "reject"] as const)(
    "ignores a stale execution plan change %s after switching the full address",
    async (outcome) => {
      const user = userEvent.setup();
      mockCodingWs();
      const pending = deferred<Awaited<ReturnType<typeof requestWorkItemExecutionPlanChange>>>();
      vi.mocked(requestWorkItemExecutionPlanChange).mockReturnValue(pending.promise);
      const planA = executionPlan({ status: "draft", goal: "A plan" });
      const planB = executionPlan({
        id: "work_item_execution_plan_b",
        project_id: OTHER_SCOPE_SAME_ATTEMPT_ADDRESS.projectId,
        issue_id: OTHER_SCOPE_SAME_ATTEMPT_ADDRESS.issueId,
        attempt_id: OTHER_SCOPE_SAME_ATTEMPT_ADDRESS.attemptId,
        status: "draft",
        goal: "B plan",
      });
      useCodingWorkspaceStore.setState({
        ...readyCodingState(),
        requireExecutionPlanConfirm: true,
        workItemExecutionPlan: planA,
      });
      const view = render(
        <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />,
      );

      await user.type(screen.getByLabelText("修改说明"), "A change");
      await user.click(screen.getByRole("button", { name: "请求修改" }));
      useCodingWorkspaceStore.setState({
        ...readyCodingState(),
        projectId: OTHER_SCOPE_SAME_ATTEMPT_ADDRESS.projectId,
        issueId: OTHER_SCOPE_SAME_ATTEMPT_ADDRESS.issueId,
        attemptId: OTHER_SCOPE_SAME_ATTEMPT_ADDRESS.attemptId,
        requireExecutionPlanConfirm: true,
        workItemExecutionPlan: planB,
      });
      view.rerender(
        <CodingWorkspacePage address={OTHER_SCOPE_SAME_ATTEMPT_ADDRESS} onBack={vi.fn()} />,
      );

      await act(async () => {
        if (outcome === "resolve") {
          pending.resolve(
            executionPlan({ status: "change_requested", goal: "stale A result" }),
          );
        } else {
          pending.reject(new Error("stale A change error"));
        }
        await pending.promise.catch(() => undefined);
      });

      const state = useCodingWorkspaceStore.getState();
      expect(state.projectId).toBe(OTHER_SCOPE_SAME_ATTEMPT_ADDRESS.projectId);
      expect(state.issueId).toBe(OTHER_SCOPE_SAME_ATTEMPT_ADDRESS.issueId);
      expect(state.workItemExecutionPlan).toEqual(planB);
      expect(screen.queryByText("stale A result")).not.toBeInTheDocument();
      expect(screen.queryByText("stale A change error")).not.toBeInTheDocument();
    },
  );

  it("keeps the current address confirm busy when an older confirm settles", async () => {
    const user = userEvent.setup();
    mockCodingWs();
    const pendingA = deferred<Awaited<ReturnType<typeof confirmWorkItemExecutionPlan>>>();
    const pendingB = deferred<Awaited<ReturnType<typeof confirmWorkItemExecutionPlan>>>();
    vi.mocked(confirmWorkItemExecutionPlan)
      .mockReturnValueOnce(pendingA.promise)
      .mockReturnValueOnce(pendingB.promise);
    const planB = executionPlan({
      id: "work_item_execution_plan_b",
      project_id: OTHER_SCOPE_SAME_ATTEMPT_ADDRESS.projectId,
      issue_id: OTHER_SCOPE_SAME_ATTEMPT_ADDRESS.issueId,
      attempt_id: OTHER_SCOPE_SAME_ATTEMPT_ADDRESS.attemptId,
      status: "draft",
      goal: "B plan",
    });
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      requireExecutionPlanConfirm: true,
      workItemExecutionPlan: executionPlan({ status: "draft", goal: "A plan" }),
    });
    const view = render(
      <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />,
    );
    await user.click(screen.getByRole("button", { name: "确认执行计划" }));

    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      projectId: OTHER_SCOPE_SAME_ATTEMPT_ADDRESS.projectId,
      issueId: OTHER_SCOPE_SAME_ATTEMPT_ADDRESS.issueId,
      attemptId: OTHER_SCOPE_SAME_ATTEMPT_ADDRESS.attemptId,
      requireExecutionPlanConfirm: true,
      workItemExecutionPlan: planB,
    });
    view.rerender(
      <CodingWorkspacePage address={OTHER_SCOPE_SAME_ATTEMPT_ADDRESS} onBack={vi.fn()} />,
    );
    const confirmButton = screen.getByRole("button", { name: "确认执行计划" });
    await waitFor(() => expect(confirmButton).toBeEnabled());
    await user.click(confirmButton);
    expect(confirmButton).toBeDisabled();

    await act(async () => {
      pendingA.resolve(executionPlan({ status: "confirmed", goal: "stale A result" }));
      await pendingA.promise;
    });
    expect(confirmButton).toBeDisabled();
    expect(useCodingWorkspaceStore.getState().workItemExecutionPlan).toEqual(planB);

    const confirmedPlanB = { ...planB, status: "confirmed" as const };
    await act(async () => {
      pendingB.resolve(confirmedPlanB);
      await pendingB.promise;
    });
    expect(useCodingWorkspaceStore.getState().workItemExecutionPlan).toEqual(confirmedPlanB);
  });
});
