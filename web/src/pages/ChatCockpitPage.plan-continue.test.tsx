// 批 2 P5（REQ-WIGA-10、REQ-WIGA-08）：plan 阶段失败/停点的对话窗口继续 UI
// ——消费 P4 的 GET automation-enrollment/control（只读 stop_key/stage/
// allowed_actions/context_required）与 POST continue-plan；Failed/停点的
// completed 投影仍可输入与继续（红：现 completed 不渲染输入条）；继续成功
// 收敛到自动推进；不出现 coder 卡/暂停/转手动入口；control/continue 一律
// 走 client.ts 通道。
import { useProviderAvailabilityStore } from "../state/provider-availability-store";
import type * as WorkspaceWsModule from "../hooks/useWorkspaceWs";
import type * as ApiClient from "../api/client";
import type { AutomationControlStatus } from "../api/client";
import {
  ApiRequestError,
  findWorkspaceSessionIssueAddress,
  getAutomationEnrollmentControl,
  postAutomationContinuePlan,
} from "../api/client";
import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { selectCockpitInbox } from "../state/workspace-cockpit-projection";
import { useWorkspaceWs } from "../hooks/useWorkspaceWs";
import { useUnloadGuard } from "../hooks/useUnloadGuard";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import { readCockpitSettings } from "../state/cockpit-settings";
import { ChatCockpitPage } from "./ChatCockpitPage";
import {
  bulkConfirmStart,
  cockpitInbox,
  cockpitObservedRecords,
  installCockpitPageTestHooks,
  watchSession,
} from "./ChatCockpitPage.test-utils";
import { currentMockWorkspaceWs, mockWorkspaceWs } from "./ChatWorkspacePage.test-utils";

vi.mock("../state/bulk-confirm-store", () => ({
  useBulkConfirmStore: Object.assign(
    (selector: (state: { runs: readonly unknown[] }) => unknown) => selector({ runs: [] }),
    { getState: () => ({ start: bulkConfirmStart }) },
  ),
}));

vi.mock("../hooks/useWorkspaceWs", async (importOriginal) => ({
  ...(await importOriginal<typeof WorkspaceWsModule>()),
  useWorkspaceWs: vi.fn(),
  newCommandId: vi.fn(() => "op-continue-1"),
}));
vi.mock("../hooks/useUnloadGuard", () => ({ useUnloadGuard: vi.fn() }));

vi.mock("../api/client", async (importOriginal) => ({
  ...(await importOriginal<typeof ApiClient>()),
  takeoverWorkspaceSession: vi.fn(),
  findWorkspaceSessionIssueAddress: vi.fn(),
  getAutomationEnrollmentControl: vi.fn(),
  postAutomationContinuePlan: vi.fn(),
}));
vi.mock("../api/workspace-content", () => ({
  fetchWorkspaceArtifactVersion: vi.fn(),
  fetchWorkspaceEventOutput: vi.fn(),
  fetchWorkspaceNodeDetail: vi.fn(),
  fetchWorkspacePrompt: vi.fn(),
}));
vi.mock("../components/shared/MonacoViewer", () => ({
  MonacoViewer: ({ value, height }: { value: string; height?: number }) => (
    <div data-testid="monaco-viewer" data-height={height}>
      {value}
    </div>
  ),
}));

vi.mock("../components/cockpit/CockpitShell", () => ({
  useCockpitCodingAttemptForSession: () => () => null,
  useCockpitShellInbox: () =>
    cockpitInbox.length > 0
      ? cockpitInbox
      : selectCockpitInbox(useWorkspaceStore.getState()).map((item) => ({
          ...item,
          id: `${useWorkspaceStore.getState().sessionId}:${item.id}`,
        })),
  useCockpitInboxPulse: () => false,
  useCockpitSessionWatch: () => watchSession,
  useCockpitSettings: () => readCockpitSettings(),
  useCockpitObservedRecords: () => cockpitObservedRecords,
  useCockpitSettingsSlotRef: () => () => undefined,
  useCockpitAttentionTargetOpener: () => () => undefined,
}));

const BINDING = {
  binding_version: 1,
  enrollment_id: "enrollment_0001",
  plan_id: "plan_0001",
  session_id: "session_001",
  source: { stories: [], designs: [] },
  target: {
    kind: "logical_codebase" as const,
    logical_codebase_id: "11111111-1111-1111-1111-111111111111",
    logical_repository_id: "00000000-0000-0000-0000-000000000001",
  },
  author_provider: "fake" as const,
  reviewer_provider: "fake" as const,
};

function controlStatus(overrides: Partial<AutomationControlStatus>): AutomationControlStatus {
  return {
    managed: true,
    binding: BINDING,
    plan_id: "plan_0001",
    session_id: "session_001",
    terminated_at: null,
    stop_key: "plan_failed:node_1",
    stage: "failed",
    allowed_actions: ["continue_plan"],
    context_required: false,
    ...overrides,
  };
}

/** 托管 plan 会话的失败现场：stage=completed（Failed/停点的 completed 投影）。 */
function failedManagedPlanSession() {
  useWorkspaceStore.setState({
    sessionId: "session_001",
    workspaceType: "work_item_plan",
    stage: "completed",
    sessionStatus: "failed",
    flowKind: "single_candidate",
    chatEntries: [],
    automation: {
      owner: "server",
      enrollment_id: "enrollment_0001",
      policy_revision: 1,
      enabled: true,
    },
  });
}

function renderCockpitPlanContinue() {
  mockWorkspaceWs();
  useWorkspaceStore.getState().setSessionIdForTest("session_001");
  return render(
    <ChatCockpitPage
      sessionId="session_001"
      onBack={vi.fn()}
      onOpenSession={vi.fn()}
      workspaceWs={currentMockWorkspaceWs()}
    />,
  );
}

describe("ChatCockpitPage plan continue (批2 P5)", () => {
  installCockpitPageTestHooks();

  beforeEach(() => {
    vi.mocked(findWorkspaceSessionIssueAddress).mockReset();
    vi.mocked(findWorkspaceSessionIssueAddress).mockResolvedValue({
      projectId: "project_0001",
      issueId: "issue_0001",
    });
    vi.mocked(getAutomationEnrollmentControl).mockReset();
    // 缺省：非托管投影（不触发恢复面）；用例按需覆写。
    vi.mocked(getAutomationEnrollmentControl).mockResolvedValue(
      controlStatus({
        managed: false,
        binding: null,
        plan_id: null,
        session_id: null,
        stop_key: null,
        stage: "unmanaged",
        allowed_actions: [],
      }),
    );
    vi.mocked(postAutomationContinuePlan).mockReset();
    failedManagedPlanSession();
  });

  it("renders the recovery input on a completed projection for a managed failed plan session", async () => {
    vi.mocked(getAutomationEnrollmentControl).mockResolvedValue(controlStatus({}));

    renderCockpitPlanContinue();

    // 红：现 completed 舞台不渲染 ChatInputBar（ChatCockpitPage 输入条矩阵
    // 只认 prepare_context/author_confirm/running/cross_review/revision）。
    expect(await screen.findByTestId("plan-continue-card")).toBeVisible();
    expect(screen.getByTestId("chat-input-bar")).toBeVisible();
    expect(screen.getByRole("textbox")).toBeEnabled();
    // control 读取走 client.ts 通道，按 issue 地址出站。
    expect(getAutomationEnrollmentControl).toHaveBeenCalledWith(
      "project_0001",
      "issue_0001",
    );
  });

  it("continues through the client REST channel and converges back to automatic progression", async () => {
    const user = userEvent.setup();
    vi.mocked(getAutomationEnrollmentControl)
      .mockResolvedValueOnce(controlStatus({}))
      .mockResolvedValueOnce(
        controlStatus({
          stage: "running",
          stop_key: null,
          binding: null,
          allowed_actions: [],
        }),
      );
    vi.mocked(postAutomationContinuePlan).mockResolvedValue({
      command_id: "op-continue-1",
      state: "accepted",
      session_id: "session_001",
    });

    renderCockpitPlanContinue();
    await screen.findByTestId("plan-continue-submit");

    await user.type(screen.getByRole("textbox"), "请重点看依赖声明");
    await user.click(screen.getByTestId("plan-continue-submit"));

    expect(postAutomationContinuePlan).toHaveBeenCalledTimes(1);
    expect(postAutomationContinuePlan).toHaveBeenCalledWith(
      "project_0001",
      "issue_0001",
      {
        command_id: "op-continue-1",
        expected_binding: BINDING,
        expected_stop_key: "plan_failed:node_1",
        context: "请重点看依赖声明",
      },
    );
    // 继续成功收敛到自动推进：control 复读无 continue_plan 动作，恢复面收起。
    await waitFor(() => {
      expect(screen.queryByTestId("plan-continue-submit")).toBeNull();
    });
    expect(getAutomationEnrollmentControl).toHaveBeenCalledTimes(2);
  });

  it("presents the three stop states with matching copy and context requirements", async () => {
    // 评审超限（StoppedNeedsHuman）：说明必填（空说明拦截）。
    vi.mocked(getAutomationEnrollmentControl).mockResolvedValue(
      controlStatus({
        stage: "stopped_needs_human",
        stop_key: "plan_gate:session_001",
        context_required: true,
      }),
    );

    renderCockpitPlanContinue();
    expect(await screen.findByTestId("plan-continue-card")).toHaveTextContent("人工说明");
    expect(screen.getByTestId("plan-continue-submit")).toBeDisabled();
    expect(screen.getByTestId("plan-continue-context-hint")).toBeVisible();

    // NeedsHuman（派发前停等）：说明可选——推进一拍（activeNodeId 变化）触发
    // control 复读，消费新投影。
    vi.mocked(getAutomationEnrollmentControl).mockResolvedValue(
      controlStatus({
        stage: "generation_needs_human",
        stop_key: "plan_generation_failed:plan_0001",
      }),
    );
    act(() => {
      useWorkspaceStore.setState({ activeNodeId: "node_next" });
    });
    await waitFor(() => {
      expect(screen.getByTestId("plan-continue-card")).toHaveTextContent("人工分诊");
    });
    expect(screen.getByTestId("plan-continue-submit")).toBeEnabled();
    expect(screen.queryByTestId("plan-continue-context-hint")).toBeNull();
  });

  it("surfaces a needs_human receipt inline and keeps the recovery surface for triage", async () => {
    const user = userEvent.setup();
    vi.mocked(getAutomationEnrollmentControl).mockResolvedValue(
      controlStatus({
        stage: "stopped_needs_human",
        stop_key: "plan_gate:session_001",
        context_required: true,
      }),
    );
    vi.mocked(postAutomationContinuePlan).mockResolvedValue({
      command_id: "op-continue-1",
      state: "needs_human",
      session_id: "session_001",
      reason_code: "automation_continue_gate_busy",
    });

    renderCockpitPlanContinue();
    await screen.findByTestId("plan-continue-submit");
    await user.type(screen.getByRole("textbox"), "修好的说明");
    await user.click(screen.getByTestId("plan-continue-submit"));

    expect(await screen.findByTestId("plan-continue-notice")).toHaveTextContent(
      "automation_continue_gate_busy",
    );
    // 分诊未收敛：恢复面仍在，可再次处理。
    expect(screen.getByTestId("plan-continue-submit")).toBeVisible();
  });

  it("preserves the note and reports the code when the continue request is rejected", async () => {
    const user = userEvent.setup();
    vi.mocked(getAutomationEnrollmentControl).mockResolvedValue(
      controlStatus({
        stage: "stopped_needs_human",
        stop_key: "plan_gate:session_001",
        context_required: true,
      }),
    );
    vi.mocked(postAutomationContinuePlan).mockRejectedValue(
      new ApiRequestError({
        code: "automation_continue_stop_key_mismatch",
        message: "expected_stop_key does not match the current durable stop",
        details: {},
      }),
    );

    renderCockpitPlanContinue();
    await screen.findByTestId("plan-continue-submit");
    await user.type(screen.getByRole("textbox"), "过期停点上的说明");
    await user.click(screen.getByTestId("plan-continue-submit"));

    expect(await screen.findByTestId("plan-continue-notice")).toHaveTextContent(
      "automation_continue_stop_key_mismatch",
    );
    expect(screen.getByRole("textbox")).toHaveValue("过期停点上的说明");
  });

  it("does not render takeover for a managed stopped card while unmanaged keeps it", async () => {
    cockpitInbox.push({
      id: "session_001:stopped:session_001",
      kind: "stopped",
      severity: 2,
      title: "会话停在停点",
      summary: "停点原因：stopped_needs_human · 自动化托管中——在对话窗口补说明继续",
      triage: false,
      source: "session_status",
      createdAt: null,
      gate: null,
      inlineError: null,
      choice: null,
      automationManaged: true,
    });

    renderCockpitPlanContinue();

    const inbox = screen.getByTestId("cockpit-inbox");
    const rows = within(inbox).getAllByTestId("cockpit-inbox-item-stopped");
    expect(rows).toHaveLength(1);
    expect(within(rows[0]!).queryByRole("button", { name: "接管" })).toBeNull();
    expect(within(rows[0]!).getByTestId("stopped-managed-hint")).toBeVisible();
  });

  // k3 波4 F2（fix loop r1）：GenerationNeedsHuman 只可能由 resolve_plan_stop
  // 兜底臂产出（durable status Open/Running/…＋intent NeedsHuman；经
  // workspace_stage_for_status 投影为 prepare_context/running）——形态门必须
  // 纳入 open/running，否则第三张恢复卡永不渲染，与「control 为权威」矛盾。
  it("renders the recovery input for a generation-needs-human stop on open/running sessions", async () => {
    for (const sessionStatus of ["open", "running"] as const) {
      cleanup();
      useWorkspaceStore.setState({
        sessionId: "session_001",
        workspaceType: "work_item_plan",
        stage: sessionStatus === "open" ? "prepare_context" : "running",
        sessionStatus,
        flowKind: "single_candidate",
        chatEntries: [],
        automation: {
          owner: "server",
          enrollment_id: "enrollment_0001",
          policy_revision: 1,
          enabled: true,
        },
      });
      vi.mocked(getAutomationEnrollmentControl).mockResolvedValue(
        controlStatus({
          stage: "generation_needs_human",
          stop_key: "plan_generation_failed:plan_0001",
        }),
      );

      renderCockpitPlanContinue();

      expect(await screen.findByTestId("plan-continue-card")).toBeVisible();
      expect(screen.getByTestId("plan-continue-submit")).toBeEnabled();
    }
  });

  it("does not render the recovery input on open/running sessions without control qualification", async () => {
    useWorkspaceStore.setState({
      sessionId: "session_001",
      workspaceType: "work_item_plan",
      stage: "running",
      sessionStatus: "running",
      flowKind: "single_candidate",
      chatEntries: [],
      automation: {
        owner: "server",
        enrollment_id: "enrollment_0001",
        policy_revision: 1,
        enabled: true,
      },
    });
    // 缺省 mock：unmanaged、无 allowed_actions——不出恢复面。

    renderCockpitPlanContinue();
    await waitFor(() => {
      expect(getAutomationEnrollmentControl).toHaveBeenCalled();
    });
    expect(screen.queryByTestId("plan-continue-card")).toBeNull();
    expect(screen.queryByTestId("plan-continue-submit")).toBeNull();
  });

  it("keeps the inbox takeover path untouched for non-managed sessions", () => {
    cockpitInbox.push({
      id: "session_002:stopped:session_002",
      kind: "stopped",
      severity: 2,
      title: "会话停在停点",
      summary: "停点原因：stopped_needs_human · 等待人工接管后继续",
      triage: false,
      source: "session_status",
      createdAt: null,
      gate: null,
      inlineError: null,
      choice: null,
    });

    renderCockpitPlanContinue();

    const inbox = screen.getByTestId("cockpit-inbox");
    expect(within(inbox).getByRole("button", { name: "接管" })).toBeVisible();
  });
});
