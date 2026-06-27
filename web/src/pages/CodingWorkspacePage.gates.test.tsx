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

describe("CodingWorkspacePage gate panels", () => {
  installCodingWorkspacePageTestHooks();

  it("renders tester contract blocked gate as blocked instead of failed test", async () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "blocked",
      stage: "testing",
      pendingGates: [
        {
          gate_id: "gate_0001",
          kind: "blocked",
          title: "Testing blocked",
          description: "TestPlan parse failed",
          stage: "testing",
          role: "tester",
          reason_code: "test_plan_missing_json",
          evidence_refs: ["testing_report_0001.json"],
          raw_provider_output_ref: "provider-raw/testing/plan_tests_0001.txt",
          available_actions: [
            {
              action_id: "retry_test_plan",
              label: "重试测试计划",
              action_type: "retry_test_plan",
            },
          ],
        },
      ],
    });

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    const gate = screen.getByTestId("coding-pending-gate");
    expect(gate).toHaveTextContent("Tester 未返回测试计划 JSON");
    expect(gate).toHaveTextContent("测试被阻塞");
    expect(gate).not.toHaveTextContent("测试失败");
  });

  it("renders testing result review gate as human confirmation instead of blocked", async () => {
    const api = mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "blocked",
      stage: "testing",
      pendingGates: [
        {
          gate_id: "gate_0001",
          kind: "blocked",
          title: "确认 Tester 测试结果",
          description:
            "Tester 已完成测试报告 testing_report_0001（测试通过）。请确认是否进入 Analyst 或重新测试。",
          stage: "testing",
          role: "tester",
          reason_code: "testing_result_review_required",
          evidence_refs: ["testing_report_0001.json"],
          raw_provider_output_ref: "provider-raw/testing/execute_test_plan_0001.txt",
          available_actions: [
            {
              action_id: "accept_testing_result",
              label: "结果可用，进入 Analyst",
              action_type: "accept_testing_result",
            },
            {
              action_id: "rerun_testing",
              label: "不满意，重新测试",
              action_type: "rerun_testing",
            },
          ],
        },
      ],
    });

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    const gate = screen.getByTestId("coding-pending-gate");
    expect(gate).toHaveTextContent("确认 Tester 测试结果");
    expect(gate).toHaveTextContent("等待确认 Tester 结果");
    expect(gate).not.toHaveTextContent("测试被阻塞");

    await userEvent.click(screen.getByRole("button", { name: "结果可用，进入 Analyst" }));
    expect(api.respondGate).toHaveBeenCalledWith(
      "gate_0001",
      "accept_testing_result",
      undefined,
    );

    await userEvent.click(screen.getByRole("button", { name: "不满意，重新测试" }));
    expect(api.respondGate).toHaveBeenCalledWith("gate_0001", "rerun_testing", undefined);
  });

  it("renders skipped_required_steps blocked gate with dedicated label", async () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "blocked",
      stage: "testing",
      pendingGates: [
        {
          gate_id: "gate_0001",
          kind: "blocked",
          title: "Testing blocked",
          description: "Required testing steps are missing or blocked",
          stage: "testing",
          role: "tester",
          reason_code: "skipped_required_steps",
          evidence_refs: ["testing_report_0001.json"],
          raw_provider_output_ref: "provider-raw/testing/execute_test_plan_0001.txt",
          available_actions: [
            {
              action_id: "retry_test_plan",
              label: "重试测试计划",
              action_type: "retry_test_plan",
            },
          ],
        },
      ],
    });

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    const gate = screen.getByTestId("coding-pending-gate");
    expect(gate).toHaveTextContent("required 测试步骤被阻塞（无法执行）");
    expect(gate).not.toHaveTextContent("缺少 required 测试步骤证据");
  });

  it("sends stage gate confirm for confirm-stage pending gate actions", async () => {
    const api = mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "running",
      stage: "testing",
      pendingGates: [
        {
          gate_id: "coding_stage_gate_0001",
          kind: "stage_gate",
          title: "Testing Stage Gate",
          description: "Waiting to start Testing",
          stage: "testing",
          role: "tester",
          expires_at: "2026-05-28T00:00:05Z",
          provider_snapshot: {
            coder: "fake",
            tester: "fake",
            analyst: "fake",
            code_reviewer: "fake",
            internal_reviewer: "fake",
            review_rounds: 1,
            permission_modes: DEFAULT_PERMISSION_MODES,
          },
          available_actions: [
            {
              action_id: "confirm_stage",
              label: "立即开始",
              action_type: "confirm_stage",
            },
          ],
        },
      ],
    });

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    await userEvent.click(screen.getByRole("button", { name: "Stage Gate 立即开始" }));

    expect(api.confirmStageGate).toHaveBeenCalledWith("testing");
    expect(api.respondGate).not.toHaveBeenCalled();
  });

  it("renders stage gate countdown with provider and abort action", async () => {
    const api = mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "running",
      stage: "coding",
      pendingGates: [
        {
          gate_id: "coding_stage_gate_0001",
          kind: "stage_gate",
          title: "Coding Stage Gate",
          description: "Waiting to start Coding",
          stage: "coding",
          role: "coder",
          expires_at: new Date(Date.now() + 5_000).toISOString(),
          provider_snapshot: {
            coder: "fake",
            tester: "codex",
            analyst: "fake",
            code_reviewer: "fake",
            internal_reviewer: "fake",
            review_rounds: 1,
            permission_modes: DEFAULT_PERMISSION_MODES,
          },
          available_actions: [
            {
              action_id: "confirm_stage",
              label: "立即开始",
              action_type: "confirm_stage",
            },
            {
              action_id: "abort",
              label: "中止 Attempt",
              action_type: "abort",
            },
          ],
        },
      ],
    });

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    expect(screen.getByTestId("coding-stage-gate-entry")).toHaveTextContent("Coding Stage Gate");
    expect(screen.getByTestId("coding-stage-gate-entry")).toHaveTextContent("Coder");
    expect(screen.getByTestId("coding-stage-gate-entry")).toHaveTextContent("fake");
    expect(screen.getByTestId("coding-stage-gate-entry")).toHaveTextContent("5s");

    await userEvent.click(screen.getByRole("button", { name: "Stage Gate 立即开始" }));
    await userEvent.click(screen.getByRole("button", { name: "Stage Gate 中止" }));

    expect(api.confirmStageGate).toHaveBeenCalledWith("coding");
    expect(api.abortAttempt).toHaveBeenCalled();
  });

  it("renders role provider panel and sends role-level provider selection", async () => {
    const api = mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "running",
      stage: "coding",
      roleProviderConfigSnapshot: {
        coder: "fake",
        tester: "fake",
        analyst: "fake",
        code_reviewer: "fake",
        internal_reviewer: "fake",
        review_rounds: 1,
        permission_modes: {
          coder: "supervised",
          tester: "auto",
          analyst: "auto",
          code_reviewer: "supervised",
          internal_reviewer: "supervised",
        },
      },
    });

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    expect(screen.getByTestId("coding-provider-config-panel")).toHaveTextContent("Coder");
    expect(screen.getByTestId("coding-provider-config-panel")).toHaveTextContent("Tester");
    expect(screen.getByTestId("coding-provider-config-panel")).toHaveTextContent("Auto");

    await userEvent.click(screen.getByRole("button", { name: "将 Tester 切换为 Codex" }));
    await userEvent.click(
      screen.getByRole("button", { name: "将 Tester 授权模式切换为 Supervised" }),
    );

    expect(api.sendProviderSelect).toHaveBeenCalledWith("tester", "codex");
    expect(api.sendPermissionModeSelect).toHaveBeenCalledWith("tester", "supervised");
  });

  it("sends coding context notes from the chat input", async () => {
    const api = mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "running",
      stage: "coding",
    });

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    const input = screen.getByLabelText("补充 Coding 上下文");
    await userEvent.type(input, "请覆盖空输入边界");
    await userEvent.click(screen.getByRole("button", { name: "发送上下文" }));

    expect(api.sendContextNote).toHaveBeenCalledWith("请覆盖空输入边界");
    expect(input).toHaveValue("");
  });

  it("keeps a manually selected artifact tab while the attempt is testing", async () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "running",
      stage: "testing",
      activeTab: "tests",
      testingReport: {
        id: "testing_report_0001",
        attempt_id: "coding_attempt_0001",
        overall_status: "passed",
        provider_claim: null,
        backend_verified: true,
        started_at: "2026-05-23T00:00:00Z",
        completed_at: "2026-05-23T00:00:02Z",
        commands: [],
      },
      logs: [
        {
          id: "log_0001",
          message: "manual tab stays visible",
          timestamp: "2026-05-23T00:00:03Z",
        },
      ],
    });

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    await userEvent.click(screen.getByRole("button", { name: "运行结果" }));
    await userEvent.click(screen.getByRole("button", { name: "logs" }));

    expect(screen.getByTestId("coding-artifact-tabs")).toHaveTextContent(
      "manual tab stays visible",
    );
    expect(screen.getByTestId("coding-artifact-tabs")).not.toHaveTextContent("passed");
  });

  it("renders plan based testing report details", async () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "blocked",
      stage: "testing",
      activeTab: "tests",
      testingReport: {
        id: "testing_report_0001",
        attempt_id: "coding_attempt_0001",
        commands: [],
        overall_status: "blocked",
        provider_claim: null,
        backend_verified: true,
        started_at: "2026-06-10T00:00:00Z",
        completed_at: "2026-06-10T00:00:01Z",
        plan_id: "test_plan_0001",
        plan_summary: "API smoke and security review",
        steps: [
          {
            step_id: "api_smoke",
            status: "passed",
            evidence_refs: ["stdout.log"],
            command: ["cargo", "test", "--locked", "--lib", "api_smoke"],
            provider_analysis: "API smoke passed",
          },
        ],
        missing_required_steps: ["security"],
        context_warnings: ["missing_design_spec"],
        raw_provider_output_ref: "provider-raw/testing/execute_test_plan_0001.txt",
      },
    });

    render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

    await userEvent.click(screen.getByRole("button", { name: "运行结果" }));

    const tabs = screen.getByTestId("coding-artifact-tabs");
    expect(tabs).toHaveTextContent("API smoke and security review");
    expect(tabs).toHaveTextContent("api_smoke");
    expect(tabs).toHaveTextContent("missing required: security");
    expect(tabs).toHaveTextContent("missing_design_spec");
    expect(tabs).toHaveTextContent("provider-raw/testing/execute_test_plan_0001.txt");
  });
});
