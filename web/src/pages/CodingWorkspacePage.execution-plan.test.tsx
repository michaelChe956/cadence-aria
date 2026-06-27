import { render, screen, waitFor } from "@testing-library/react";
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
  DEFAULT_PERMISSION_MODES,
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

  it("constrains role run history overflow inside the conversation column", () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "waiting_for_human",
      stage: "rework",
      roleRuns: Array.from({ length: 12 }, (_, index) => ({
        id: `coding_role_run_${String(index + 1).padStart(4, "0")}`,
        attempt_id: "coding_attempt_0001",
        stage: index % 2 === 0 ? "testing" : "rework",
        role: index % 2 === 0 ? "tester" : "analyst",
        run_no: index + 1,
        status: index % 3 === 0 ? "blocked" : "completed",
        trigger: "initial",
        node_id: `coding_node_${String(index + 1).padStart(4, "0")}`,
        started_at: `2026-06-13T00:00:${String(index).padStart(2, "0")}Z`,
        completed_at: null,
        supersedes_run_id: null,
        superseded_by_run_id: null,
        reason_code: "max_auto_rework_exceeded",
        raw_provider_output_refs: [
          "provider-raw/rework/very-long-role-run-output-reference-that-must-not-widen-page.txt",
        ],
        artifact_refs: [
          "artifacts/rework/very-long-analyst-evidence-reference-that-must-scroll-inside-panel.json",
        ],
      })),
    });

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    const panel = screen.getByTestId("coding-role-run-history");
    expect(panel).toHaveClass("min-w-0", "overflow-hidden");
    expect(panel.parentElement).toHaveClass("min-w-0", "overflow-hidden");
    expect(panel.parentElement?.parentElement).toHaveClass("min-w-0", "overflow-hidden");
    expect(panel.parentElement?.parentElement?.parentElement).toHaveClass(
      "min-w-0",
      "overflow-hidden",
    );
    expect(screen.getByRole("button", { name: "继续返修" })).toBeInTheDocument();
  });

  it("shows work item execution plan during prepare stage as non blocking by default", () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      stage: "prepare_context",
      workItemExecutionPlan: executionPlan({ status: "draft" }),
    });

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

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

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

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

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    await user.click(screen.getByRole("button", { name: "确认执行计划" }));

    expect(confirmWorkItemExecutionPlan).toHaveBeenCalledWith("coding_attempt_0001");
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

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    await user.type(screen.getByLabelText("修改说明"), "需要补充边界条件测试");
    await user.click(screen.getByRole("button", { name: "请求修改" }));

    expect(requestWorkItemExecutionPlanChange).toHaveBeenCalledWith("coding_attempt_0001", {
      note: "需要补充边界条件测试",
    });
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

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

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

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

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

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    await user.type(screen.getByLabelText("修改说明"), "说明");
    await user.click(screen.getByRole("button", { name: "请求修改" }));

    expect(screen.getByText("change failed")).toBeInTheDocument();
    expect(useCodingWorkspaceStore.getState().workItemExecutionPlan?.status).toBe("draft");
  });
});
