import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ProviderHealthResponse } from "../api/types";
import {
  confirmWorkItemExecutionPlan,
  deleteCodingAttempt,
  getCodingAttemptDiff,
  requestWorkItemExecutionPlanChange,
} from "../api/client";
import { useCodingWorkspaceWs } from "../hooks/useCodingWorkspaceWs";
import { useCodingWorkspaceStore } from "../state/coding-workspace-store";
import { useProviderAvailabilityStore } from "../state/provider-availability-store";
import { CodingWorkspacePage } from "./CodingWorkspacePage";
import {
  CODING_ATTEMPT_ADDRESS,
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

function setCodingPageProviderHealth() {
  const snapshot: ProviderHealthResponse = {
    schema_version: 1,
    generation: 1,
    checked_at: "2026-07-14T00:00:00Z",
    state_status: "ready",
    state_error: null,
    real_workflow_blocked: false,
    test_provider_enabled: true,
    providers: [
      {
        provider: "claude_code",
        display_name: "Claude Code",
        available: false,
        version: null,
        reason_code: "command_missing",
        reason: "Claude Code 未安装",
        checked_at: "2026-07-14T00:00:00Z",
        install_hint: "请先安装 Claude Code",
      },
      {
        provider: "codex",
        display_name: "Codex",
        available: true,
        version: "1.0.0",
        reason_code: null,
        reason: null,
        checked_at: "2026-07-14T00:00:00Z",
        install_hint: "",
      },
    ],
  };
  useProviderAvailabilityStore.setState({ snapshot, loadStatus: "loaded" });
}

afterEach(() => {
  useProviderAvailabilityStore.getState().reset();
});

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

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

    const gate = screen.getByTestId("coding-pending-gate");
    expect(gate).toHaveTextContent("Tester 未返回测试计划 JSON");
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
            "Tester 已完成测试报告 testing_report_0001（测试通过）。请确认是否进入 Code Reviewer 或重新测试。",
          stage: "testing",
          role: "tester",
          reason_code: "testing_result_review_required",
          evidence_refs: ["testing_report_0001.json"],
          raw_provider_output_ref: "provider-raw/testing/execute_test_plan_0001.txt",
          available_actions: [
            {
              action_id: "accept_testing_result",
              label: "结果可用，进入 Code Reviewer",
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

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

    const gate = screen.getByTestId("coding-pending-gate");
    expect(gate).toHaveTextContent("确认 Tester 测试结果");
    expect(gate).not.toHaveTextContent("测试被阻塞");

    await userEvent.click(screen.getByRole("button", { name: "结果可用，进入 Code Reviewer" }));
    expect(api.respondGate).toHaveBeenCalledWith(
      "gate_0001",
      "accept_testing_result",
      undefined,
    );

    await userEvent.click(screen.getByRole("button", { name: "不满意，重新测试" }));
    expect(api.respondGate).toHaveBeenCalledWith("gate_0001", "rerun_testing", undefined);
  });

  it("recovers an interrupted code review through the generic blocked gate", async () => {
    const api = mockCodingWs();
    vi.mocked(api.respondGate).mockImplementation((gateId) => {
      useCodingWorkspaceStore.getState().markGateSubmitting(gateId);
    });
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "blocked",
      stage: "code_review",
      pendingGates: [
        {
          gate_id: "coding_blocked_gate_0001",
          kind: "blocked",
          title: "代码审查中断",
          description: "上次代码审查已中断，可保留当前修改并重试 Reviewer。",
          stage: "code_review",
          role: "code_reviewer",
          available_actions: [
            {
              action_id: "retry_review",
              label: "重试代码审查",
              action_type: "retry_review",
            },
          ],
          reason_code: "failed_code_review_recoverable",
          evidence_refs: [],
        },
      ],
    });

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

    expect(screen.getByTestId("coding-pending-gate")).toHaveTextContent("代码审查中断");
    expect(screen.queryByRole("button", { name: "发送上下文" })).not.toBeInTheDocument();
    expect(screen.queryByLabelText("补充 Coding 上下文")).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "重试代码审查" }));

    expect(api.respondGate).toHaveBeenCalledWith(
      "coding_blocked_gate_0001",
      "retry_review",
      undefined,
    );
    expect(api.sendContextNote).not.toHaveBeenCalled();

    const submittingButton = screen.getByRole("button", { name: "处理中" });
    expect(submittingButton).toBeDisabled();
    await userEvent.click(submittingButton);
    expect(api.respondGate).toHaveBeenCalledTimes(1);

    act(() => {
      const store = useCodingWorkspaceStore.getState();
      store.setProtocolError({
        code: "coding_gate_response_failed",
        message: "Gate response failed",
      });
      store.setGateError("coding_blocked_gate_0001", "coding_gate_response_failed");
    });

    const retryButton = screen.getByRole("button", { name: "重试代码审查" });
    expect(retryButton).toBeEnabled();
    expect(screen.getByTestId("coding-pending-gate")).toHaveTextContent(
      "coding_gate_response_failed",
    );
  });

  it("restarts an interrupted coder without requiring extra context", async () => {
    const api = mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "blocked",
      stage: "coding",
      pendingGates: [
        {
          gate_id: "coding_blocked_gate_0001",
          kind: "blocked",
          title: "Coder 执行中断",
          description: "Codex resume stalled before provider progress",
          stage: "coding",
          role: "coder",
          reason_code: "coder_provider_interrupted",
          available_actions: [
            {
              action_id: "retry_coding",
              label: "重新启动 Coder",
              action_type: "retry_coding",
            },
          ],
        },
      ],
    });

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

    expect(screen.queryByLabelText("补充 Coding 上下文")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "重新启动 Coder" }));

    expect(api.respondGate).toHaveBeenCalledWith(
      "coding_blocked_gate_0001",
      "retry_coding",
      undefined,
    );
    expect(api.sendContextNote).not.toHaveBeenCalled();
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

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

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
            tester_plan: "fake",
            tester_execute: "fake",
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

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

    await userEvent.click(screen.getByRole("button", { name: "Stage Gate 立即开始" }));

    expect(api.confirmStageGate).toHaveBeenCalledWith("testing");
    expect(api.respondGate).not.toHaveBeenCalled();
  });

  it("renders stage gate countdown with provider and abort action", async () => {
    const api = mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "created",
      stage: "prepare_context",
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
            tester_plan: "codex",
            tester_execute: "codex",
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

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

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
    setCodingPageProviderHealth();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "created",
      stage: "prepare_context",
      maxAutoRework: 2,
      roleProviderConfigSnapshot: {
        coder: "fake",
        tester_plan: "fake",
        tester_execute: "fake",
        code_reviewer: "fake",
        internal_reviewer: "fake",
        review_rounds: 1,
        permission_modes: {
          coder: "supervised",
          tester: "auto",
          code_reviewer: "supervised",
          internal_reviewer: "supervised",
        },
      },
    });

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

    expect(screen.queryByTestId("coding-provider-config-panel")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Provider 设置" }));

    expect(screen.getByTestId("coding-provider-config-panel")).toHaveTextContent("Coder");
    expect(screen.getByTestId("coding-provider-config-panel")).not.toHaveTextContent("Tester Plan");
    expect(screen.getByTestId("coding-provider-config-panel")).not.toHaveTextContent("Tester Execute");
    expect(screen.getByTestId("coding-provider-config-panel")).not.toHaveTextContent("Analyst");
    expect(screen.getByTestId("coding-provider-config-panel")).toHaveTextContent("Code Reviewer");
    expect(screen.getByTestId("coding-provider-config-panel")).not.toHaveTextContent("Internal Reviewer");
    expect(screen.getByTestId("coding-provider-config-panel")).toHaveTextContent("自动修复次数");
    expect(screen.getByTestId("coding-provider-config-panel")).toHaveTextContent("Auto");
    expect(
      screen.getByRole("button", { name: "将 Coder 切换为 Claude Code" }),
    ).toBeDisabled();
    expect(screen.getAllByText("Claude Code 未安装").length).toBeGreaterThan(0);

    await userEvent.click(screen.getByRole("button", { name: "将 Code Reviewer 切换为 Codex" }));
    await userEvent.click(
      screen.getByRole("button", { name: "将 Code Reviewer 授权模式切换为 Auto" }),
    );
    fireEvent.change(screen.getByLabelText("CodeReview 自动修复次数"), {
      target: { value: "4" },
    });

    expect(api.sendProviderSelect).toHaveBeenCalledWith("code_reviewer", "codex");
    expect(api.sendPermissionModeSelect).toHaveBeenCalledWith("code_reviewer", "auto");
    expect(api.sendMaxAutoReworkSelect).toHaveBeenCalledWith(4);
  });

  it("shows GroupFinalReview provider only for work item group attempts", async () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_group_0001",
      attemptScope: "work_item_group",
      status: "created",
      stage: "prepare_context",
      maxAutoRework: 2,
      roleProviderConfigSnapshot: {
        coder: "fake",
        tester_plan: "fake",
        tester_execute: "fake",
        code_reviewer: "fake",
        internal_reviewer: "claude_code",
        review_rounds: 1,
        permission_modes: {
          coder: "supervised",
          tester: "auto",
          code_reviewer: "supervised",
          internal_reviewer: "supervised",
        },
      },
    });

    render(
      <CodingWorkspacePage
        address={{
          ...CODING_ATTEMPT_ADDRESS,
          attemptId: "coding_attempt_group_0001",
        }}
        onBack={vi.fn()}
      />,
    );

    await userEvent.click(screen.getByRole("button", { name: "Provider 设置" }));

    const panel = screen.getByTestId("coding-provider-config-panel");
    expect(panel).toHaveTextContent("Coder");
    expect(panel).toHaveTextContent("Code Reviewer");
    expect(panel).toHaveTextContent("GroupFinalReview");
    expect(panel).not.toHaveTextContent("Internal Reviewer");
    expect(panel).not.toHaveTextContent("Tester Plan");
    expect(panel).not.toHaveTextContent("Tester Execute");
    expect(panel).not.toHaveTextContent("Analyst");
  });

  it("sends coding context notes from the chat input", async () => {
    const api = mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "running",
      stage: "coding",
    });

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

    const input = screen.getByLabelText("补充 Coding 上下文");
    await userEvent.type(input, "请覆盖空输入边界");
    await userEvent.click(screen.getByRole("button", { name: "发送上下文" }));

    expect(api.sendContextNote).toHaveBeenCalledWith("请覆盖空输入边界");
    expect(input).toHaveValue("");
  });

  it("requires manual coder fix context before submitting gate actions to coder", async () => {
    const api = mockCodingWs();
    useCodingWorkspaceStore.setState({
      attemptId: "coding_attempt_0001",
      status: "waiting_for_human",
      stage: "code_review",
      pendingGates: [
        {
          gate_id: "gate_rework_limit",
          kind: "blocked",
          title: "Code Review 修复超上限",
          description: "code review 连续要求修改 2 次，已达上限，请人工介入。",
          stage: "code_review",
          role: "code_reviewer",
          reason_code: "reviewer_rework_limit_reached",
          evidence_refs: ["code_review_report_0002"],
          raw_provider_output_ref: "provider-raw/code_review/code_review_0002.txt",
          available_actions: [
            {
              action_id: "send_to_coder",
              label: "提交给 Coder 修复",
              action_type: "send_to_coder",
            },
            {
              action_id: "abort",
              label: "终止",
              action_type: "abort",
            },
          ],
        },
      ],
    });

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

    const gate = screen.getByTestId("coding-pending-gate");
    expect(gate).toHaveTextContent("Code Review 修复超上限");
    expect(gate).not.toHaveTextContent("质量豁免");
    expect(screen.queryByRole("button", { name: "发送上下文" })).not.toBeInTheDocument();
    expect(screen.getByText("请使用上方门禁操作提交人工修复意见")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "提交给 Coder 修复" }));
    expect(api.respondGate).not.toHaveBeenCalled();
    expect(gate).toHaveTextContent("需要填写人工修复意见");

    await userEvent.type(screen.getByLabelText("人工修复意见"), "优先处理第 2 条 finding");
    await userEvent.click(screen.getByRole("button", { name: "提交给 Coder 修复" }));

    expect(api.respondGate).toHaveBeenCalledWith(
      "gate_rework_limit",
      "send_to_coder",
      "优先处理第 2 条 finding",
    );
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

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

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

    render(
      <CodingWorkspacePage
        address={CODING_ATTEMPT_ADDRESS}
        onBack={vi.fn()}
      />
    );

    await userEvent.click(screen.getByRole("button", { name: "运行结果" }));

    const tabs = screen.getByTestId("coding-artifact-tabs");
    expect(tabs).toHaveTextContent("API smoke and security review");
    expect(tabs).toHaveTextContent("api_smoke");
    expect(tabs).toHaveTextContent("missing required: security");
    expect(tabs).toHaveTextContent("missing_design_spec");
    expect(tabs).toHaveTextContent("provider-raw/testing/execute_test_plan_0001.txt");
  });

});
