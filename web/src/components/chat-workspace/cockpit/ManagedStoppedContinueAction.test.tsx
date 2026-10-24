// 托管 stopped 卡（enabled 精确绑定）原地「补说明并继续」入口——契约：
// P4 GET automation-enrollment/control 是 expected_binding/expected_stop_key/
// allowed_actions/context_required 的唯一来源（提交时重读，不复用旧值）；
// POST continue-plan 与对话窗口恢复面同一 REST 通道；成功触发 lifecycle
// invalidation 并清空输入；失败内联后端 code+message、保留输入、不自动重试。
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type * as ApiClient from "../../../api/client";
import {
  ApiRequestError,
  findWorkspaceSessionIssueAddress,
  getAutomationEnrollmentControl,
  postAutomationContinuePlan,
  type AutomationControlStatus,
} from "../../../api/client";
import type * as WorkspaceWsModule from "../../../hooks/useWorkspaceWs";
import { subscribeToLifecycleInvalidation } from "../../../state/lifecycle-workbench-store";
import { ManagedStoppedContinueAction } from "./ManagedStoppedContinueAction";

vi.mock("../../../api/client", async (importOriginal) => {
  const actual = await importOriginal<typeof ApiClient>();
  return {
    ...actual,
    findWorkspaceSessionIssueAddress: vi.fn(),
    getAutomationEnrollmentControl: vi.fn(),
    postAutomationContinuePlan: vi.fn(),
  };
});

vi.mock("../../../hooks/useWorkspaceWs", async (importOriginal) => ({
  ...(await importOriginal<typeof WorkspaceWsModule>()),
  newCommandId: vi.fn(() => "cmd-continue-1"),
}));

const SESSION_ID = "workspace_session_auto_261ab5db";
const ADDRESS = { projectId: "project_0001", issueId: "issue_0001" };

const BINDING: NonNullable<AutomationControlStatus["binding"]> = {
  binding_version: 3,
  enrollment_id: "enrollment_0001",
  plan_id: "issue_work_item_plan_auto_261ab5db",
  session_id: SESSION_ID,
  source: { stories: [], designs: [] },
  target: {
    kind: "logical_codebase",
    logical_codebase_id: "11111111-1111-1111-1111-111111111111",
    logical_repository_id: "00000000-0000-0000-0000-000000000001",
  },
  author_provider: "fake",
  reviewer_provider: "fake",
};

function control(overrides: Partial<AutomationControlStatus> = {}): AutomationControlStatus {
  return {
    managed: true,
    binding: BINDING,
    plan_id: BINDING.plan_id,
    session_id: SESSION_ID,
    terminated_at: null,
    stop_key: "plan_gate:workspace_session_auto_261ab5db:human_confirm",
    stage: "stopped_needs_human",
    allowed_actions: ["continue_plan"],
    context_required: true,
    ...overrides,
  };
}

describe("ManagedStoppedContinueAction", () => {
  beforeEach(() => {
    vi.mocked(findWorkspaceSessionIssueAddress).mockReset();
    vi.mocked(findWorkspaceSessionIssueAddress).mockResolvedValue(ADDRESS);
    vi.mocked(getAutomationEnrollmentControl).mockReset();
    vi.mocked(getAutomationEnrollmentControl).mockResolvedValue(control());
    vi.mocked(postAutomationContinuePlan).mockReset();
  });

  afterEach(() => cleanup());

  it("renders the entry only while control allows continue_plan", async () => {
    const { unmount } = render(<ManagedStoppedContinueAction sessionId={SESSION_ID} />);

    expect(
      await screen.findByRole("button", { name: "补说明并继续" }),
    ).toBeVisible();
    // control 从 client 通道按 issue 地址读取（会话 id → 目录解析）。
    expect(findWorkspaceSessionIssueAddress).toHaveBeenCalledWith(SESSION_ID);
    expect(getAutomationEnrollmentControl).toHaveBeenCalledWith(
      ADDRESS.projectId,
      ADDRESS.issueId,
    );

    unmount();
    vi.mocked(getAutomationEnrollmentControl).mockResolvedValue(
      control({ allowed_actions: ["terminate"] }),
    );
    render(<ManagedStoppedContinueAction sessionId={SESSION_ID} />);

    await waitFor(() => {
      expect(screen.queryByTestId("stopped-managed-continue")).toBeNull();
    });
  });

  it("expands the composer and blocks an empty note when context is required", async () => {
    const user = userEvent.setup();
    render(<ManagedStoppedContinueAction sessionId={SESSION_ID} />);

    await user.click(await screen.findByRole("button", { name: "补说明并继续" }));

    const textarea = await screen.findByTestId("stopped-managed-continue-context");
    expect(textarea).toBeVisible();
    expect(screen.getByTestId("stopped-managed-continue-submit")).toBeDisabled();
    expect(screen.getByTestId("stopped-managed-continue-context-hint")).toBeVisible();

    await user.type(textarea, "请把依赖声明补齐");
    expect(screen.getByTestId("stopped-managed-continue-submit")).toBeEnabled();
    expect(screen.queryByTestId("stopped-managed-continue-context-hint")).toBeNull();
  });

  it("posts the exact continue-plan payload from a freshly read control", async () => {
    const user = userEvent.setup();
    const freshBinding = { ...BINDING, binding_version: 4 };
    vi.mocked(getAutomationEnrollmentControl)
      // 打开卡片时
      .mockResolvedValueOnce(control())
      // 点击入口时（打开输入）
      .mockResolvedValueOnce(control())
      // 提交前重读：停点身份已漂移，必须用新值（不复用旧值/不猜）
      .mockResolvedValueOnce(
        control({
          binding: freshBinding,
          stop_key: "plan_gate:workspace_session_auto_261ab5db:human_confirm:v2",
        }),
      )
      // 回执后复读：继续已收敛，入口收起
      .mockResolvedValue(
        control({ stage: "running", stop_key: null, binding: null, allowed_actions: [] }),
      );
    vi.mocked(postAutomationContinuePlan).mockResolvedValue({
      command_id: "cmd-continue-1",
      state: "accepted",
      session_id: SESSION_ID,
    });

    render(<ManagedStoppedContinueAction sessionId={SESSION_ID} />);
    await user.click(await screen.findByRole("button", { name: "补说明并继续" }));
    await user.type(
      screen.getByTestId("stopped-managed-continue-context"),
      "请重点看依赖声明",
    );
    await user.click(screen.getByTestId("stopped-managed-continue-submit"));

    await waitFor(() => {
      expect(postAutomationContinuePlan).toHaveBeenCalledTimes(1);
    });
    expect(postAutomationContinuePlan).toHaveBeenCalledWith(
      ADDRESS.projectId,
      ADDRESS.issueId,
      {
        command_id: "cmd-continue-1",
        expected_binding: freshBinding,
        expected_stop_key: "plan_gate:workspace_session_auto_261ab5db:human_confirm:v2",
        context: "请重点看依赖声明",
      },
    );
  });

  it("clears the note, closes the composer and invalidates lifecycle on acceptance", async () => {
    const user = userEvent.setup();
    const invalidated: string[] = [];
    const unsubscribe = subscribeToLifecycleInvalidation((event) => {
      invalidated.push(event.issueId);
    });
    vi.mocked(postAutomationContinuePlan).mockResolvedValue({
      command_id: "cmd-continue-1",
      state: "accepted",
      session_id: SESSION_ID,
    });
    vi.mocked(getAutomationEnrollmentControl)
      // 打开卡片 + 点击入口（输入可用）
      .mockResolvedValueOnce(control())
      .mockResolvedValueOnce(control())
      // 提交前重读（命令可发）
      .mockResolvedValueOnce(control())
      // 回执后复读：继续已收敛，入口收起
      .mockResolvedValue(
        control({ stage: "running", stop_key: null, binding: null, allowed_actions: [] }),
      );

    render(<ManagedStoppedContinueAction sessionId={SESSION_ID} />);
    await user.click(await screen.findByRole("button", { name: "补说明并继续" }));
    const textarea = screen.getByTestId("stopped-managed-continue-context");
    await user.type(textarea, "补充说明");
    await user.click(screen.getByTestId("stopped-managed-continue-submit"));

    expect(await screen.findByTestId("stopped-managed-continue-notice")).toHaveTextContent(
      "已继续，自动化推进中",
    );
    expect(screen.queryByTestId("stopped-managed-continue-context")).toBeNull();
    // 卡片状态刷新：control 复读无 continue_plan，入口不再出按钮。
    await waitFor(() => {
      expect(screen.queryByRole("button", { name: "补说明并继续" })).toBeNull();
    });
    expect(invalidated).toEqual([ADDRESS.issueId]);
    unsubscribe();
  });

  it("surfaces the backend code and message inline, keeps the note and never auto-retries", async () => {
    const user = userEvent.setup();
    vi.mocked(postAutomationContinuePlan).mockRejectedValue(
      new ApiRequestError({
        code: "automation_continue_stop_key_mismatch",
        message: "expected_stop_key does not match the current durable stop",
        details: {},
      }),
    );

    render(<ManagedStoppedContinueAction sessionId={SESSION_ID} />);
    await user.click(await screen.findByRole("button", { name: "补说明并继续" }));
    const textarea = screen.getByTestId("stopped-managed-continue-context");
    await user.type(textarea, "过期停点上的说明");
    // 提交前重读一次 control，之后不得再自动复读/重试。
    const controlCallsBeforeSubmit =
      vi.mocked(getAutomationEnrollmentControl).mock.calls.length;
    await user.click(screen.getByTestId("stopped-managed-continue-submit"));

    const notice = await screen.findByTestId("stopped-managed-continue-notice");
    expect(notice).toHaveTextContent("automation_continue_stop_key_mismatch");
    expect(notice).toHaveTextContent("expected_stop_key does not match the current durable stop");
    expect(notice).toHaveAttribute("role", "alert");
    // 输入保留、未自动重试。
    expect(screen.getByTestId("stopped-managed-continue-context")).toHaveValue(
      "过期停点上的说明",
    );
    expect(postAutomationContinuePlan).toHaveBeenCalledTimes(1);
    // 失败不静默复读 control（避免错误面被刷新吞掉）。
    expect(
      vi.mocked(getAutomationEnrollmentControl).mock.calls.length - controlCallsBeforeSubmit,
    ).toBe(1);
  });

  it("omits context when the stop does not require a note", async () => {
    const user = userEvent.setup();
    vi.mocked(getAutomationEnrollmentControl).mockResolvedValue(
      control({ stage: "failed", stop_key: "plan_failed:node_1", context_required: false }),
    );
    vi.mocked(postAutomationContinuePlan).mockResolvedValue({
      command_id: "cmd-continue-1",
      state: "accepted",
      session_id: SESSION_ID,
    });

    render(<ManagedStoppedContinueAction sessionId={SESSION_ID} />);
    await user.click(await screen.findByRole("button", { name: "补说明并继续" }));
    expect(screen.queryByTestId("stopped-managed-continue-context-hint")).toBeNull();
    await user.click(screen.getByTestId("stopped-managed-continue-submit"));

    await waitFor(() => {
      expect(postAutomationContinuePlan).toHaveBeenCalledTimes(1);
    });
    const payload = vi.mocked(postAutomationContinuePlan).mock.calls[0]?.[2];
    expect(payload).toEqual({
      command_id: "cmd-continue-1",
      expected_binding: BINDING,
      expected_stop_key: "plan_failed:node_1",
    });
    expect(payload).not.toHaveProperty("context");
  });

  it("keeps the triage surface when the receipt needs human judgement", async () => {
    const user = userEvent.setup();
    vi.mocked(postAutomationContinuePlan).mockResolvedValue({
      command_id: "cmd-continue-1",
      state: "needs_human",
      session_id: SESSION_ID,
      reason_code: "automation_continue_gate_busy",
    });

    render(<ManagedStoppedContinueAction sessionId={SESSION_ID} />);
    await user.click(await screen.findByRole("button", { name: "补说明并继续" }));
    await user.type(screen.getByTestId("stopped-managed-continue-context"), "再试一次说明");
    await user.click(screen.getByTestId("stopped-managed-continue-submit"));

    expect(await screen.findByTestId("stopped-managed-continue-notice")).toHaveTextContent(
      "automation_continue_gate_busy",
    );
    expect(screen.getByTestId("stopped-managed-continue-submit")).toBeVisible();
  });

  it("renders nothing when the session address cannot be resolved", async () => {
    vi.mocked(findWorkspaceSessionIssueAddress).mockResolvedValue(null);

    render(<ManagedStoppedContinueAction sessionId={SESSION_ID} />);

    await waitFor(() => {
      expect(findWorkspaceSessionIssueAddress).toHaveBeenCalledWith(SESSION_ID);
    });
    expect(screen.queryByTestId("stopped-managed-continue")).toBeNull();
  });
});
