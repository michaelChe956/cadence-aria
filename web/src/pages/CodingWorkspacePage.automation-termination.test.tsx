// 批 2 T4（REQ-WIGA-09/08）红测：Coding Workspace 统一终止入口与隐藏。
// 契约：plan §2 波 5 T4 ①②③④⑤——
// ①任意入口（页面「终止整个自动化」/门 Abort/旧 abort 按钮）→同一二次确认
//   →confirmed:true 提交 terminate；
// ②未确认零请求副作用（确认面前零 terminate 请求）；
// ③终止后无局部 abort/restart（terminated_at 呈现）；
// ④门动作为原集合但 auto Abort 执行语义改整 plan；
// ⑤手动单 attempt abort 不变。
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type * as ApiClient from "../api/client";
import { postAutomationEnrollmentTerminate } from "../api/client";
import { useCodingWorkspaceStore } from "../state/coding-workspace-store";
import {
  CODING_ATTEMPT_ADDRESS,
  installCodingWorkspacePageTestHooks,
  mockCodingWs,
} from "./CodingWorkspacePage.test-utils";
import { useCodingWorkspaceWs } from "../hooks/useCodingWorkspaceWs";
import { CodingWorkspacePage } from "./CodingWorkspacePage";

vi.mock("../api/client", async (importOriginal) => ({
  ...(await importOriginal<typeof ApiClient>()),
  getAutomationEnrollment: vi.fn(),
  postAutomationEnrollmentTerminate: vi.fn(),
}));

vi.mock("../hooks/useCodingWorkspaceWs", () => ({
  useCodingWorkspaceWs: vi.fn(),
}));

vi.mock("../hooks/useUnloadGuard", () => ({
  useUnloadGuard: vi.fn(),
}));

vi.mock("../components/shared/MonacoViewer", () => ({
  MonacoViewer: () => <div data-testid="monaco-viewer" />,
}));

vi.mock("../components/shared/MonacoDiffViewer", () => ({
  MonacoDiffViewer: () => <div data-testid="monaco-diff-viewer" />,
}));

const MANAGED_AUTOMATION = {
  owner: "server" as const,
  enrollment_id: "enrollment_0001",
  policy_revision: 3,
  enabled: true,
  plan_id: "plan_0001",
  terminated_at: null,
};

function renderCodingPage() {
  return render(
    <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />,
  );
}

describe("CodingWorkspacePage automation termination（T4）", () => {
  installCodingWorkspacePageTestHooks();

  it("managed 时页面头部提供「终止整个自动化」，隐藏全部单 attempt 中止按钮", () => {
    const api = mockCodingWs();
    useCodingWorkspaceStore.setState({
      projectId: "project_0001",
      issueId: "issue_0001",
      attemptId: "coding_attempt_0001",
      status: "running",
      stage: "coding",
      automation: MANAGED_AUTOMATION,
    });

    renderCodingPage();

    expect(
      screen.getByTestId("automation-termination-entry"),
    ).toBeInTheDocument();
    // 局部 abort（header/底部 ActionButtons）全部隐藏。
    expect(screen.queryByRole("button", { name: "中止" })).toBeNull();
    expect(api.abortAttempt).not.toHaveBeenCalled();
  });

  it("入口→统一二次确认→confirmed:true 提交 terminate；未确认零请求", async () => {
    mockCodingWs();
    const terminate = vi.mocked(postAutomationEnrollmentTerminate);
    terminate.mockResolvedValue({
      command_id: "cmd-fe-terminate-enrollment_0001",
      state: "accepted",
      enrollment_id: "enrollment_0001",
      plan_id: "plan_0001",
      terminated_at: "2026-10-16T06:00:00Z",
      outcome: "terminated",
      results: [],
    });
    useCodingWorkspaceStore.setState({
      projectId: "project_0001",
      issueId: "issue_0001",
      attemptId: "coding_attempt_0001",
      status: "running",
      stage: "coding",
      automation: MANAGED_AUTOMATION,
    });

    const user = userEvent.setup();
    renderCodingPage();

    // 第一步：开统一确认面（此时零 terminate 请求）。
    await user.click(screen.getByTestId("automation-termination-entry"));
    expect(screen.getByTestId("automation-termination-confirm")).toBeInTheDocument();
    expect(terminate).not.toHaveBeenCalled();

    // 第二步：二次确认后才发 confirmed:true 的统一终止。
    const confirmSurface = within(screen.getByTestId("automation-termination-confirm"));
    await user.click(confirmSurface.getByRole("button", { name: "终止整个自动化" }));
    await user.click(confirmSurface.getByRole("button", { name: "确认终止整个自动化" }));

    await vi.waitFor(() => {
      expect(terminate).toHaveBeenCalledWith(
        "project_0001",
        "issue_0001",
        {
          command_id: "cmd-fe-terminate-enrollment_0001",
          expected_enrollment_id: "enrollment_0001",
          expected_policy_revision: 3,
          confirmed: true,
        },
      );
    });

    // 终止态呈现（terminated_at）＋确认面关闭＋入口撤下。
    expect(
      screen.getByTestId("automation-terminated-banner"),
    ).toHaveTextContent("2026-10-16T06:00:00Z");
    expect(screen.queryByTestId("automation-termination-confirm")).toBeNull();
    expect(screen.queryByTestId("automation-termination-entry")).toBeNull();
  });

  it("终止后（terminated_at 投影）无终止入口/局部 abort/重新开始", () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      projectId: "project_0001",
      issueId: "issue_0001",
      attemptId: "coding_attempt_0001",
      status: "aborted",
      stage: "coding",
      automation: { ...MANAGED_AUTOMATION, enabled: false, terminated_at: "2026-10-16T06:00:00Z" },
    });

    renderCodingPage();

    expect(screen.getByTestId("automation-terminated-banner")).toBeInTheDocument();
    expect(screen.queryByTestId("automation-termination-entry")).toBeNull();
    expect(screen.queryByRole("button", { name: "中止" })).toBeNull();
    // F-44 的「重新开始」在终止后隐藏。
    expect(screen.queryByRole("button", { name: "重新开始" })).toBeNull();
  });

  it("managed 时门 Abort 动作改走统一终止；非 Abort 动作原样经 respondGate", async () => {
    const api = mockCodingWs();
    useCodingWorkspaceStore.setState({
      projectId: "project_0001",
      issueId: "issue_0001",
      attemptId: "coding_attempt_0001",
      status: "blocked",
      stage: "coding",
      automation: MANAGED_AUTOMATION,
      pendingGates: [
        {
          gate_id: "coding_blocked_gate_0001",
          kind: "blocked",
          title: "运行环境阻塞",
          description: "数据库连接被拒绝",
          stage: "coding",
          role: "coder",
          reason_code: "coding_operational_blocker",
          evidence_refs: [],
          available_actions: [
            { action_id: "retry_coding", label: "让 AI 再试一次", action_type: "retry_coding" },
            { action_id: "abort", label: "终止", action_type: "abort" },
          ],
        } as never,
      ],
    });

    const user = userEvent.setup();
    renderCodingPage();

    // Abort 门动作 → 统一确认面（不开门响应）。
    await user.click(screen.getByRole("button", { name: "终止" }));
    expect(screen.getByTestId("automation-termination-confirm")).toBeInTheDocument();
    expect(api.respondGate).not.toHaveBeenCalled();

    // 非 Abort 门动作原样。
    await user.click(screen.getByRole("button", { name: "让 AI 再试一次" }));
    expect(api.respondGate).toHaveBeenCalledWith(
      "coding_blocked_gate_0001",
      "retry_coding",
      undefined,
    );
  });

  it("手动模式（无 automation 投影）保持原单 attempt 中止语义", async () => {
    const api = mockCodingWs();
    useCodingWorkspaceStore.setState({
      projectId: "project_0001",
      issueId: "issue_0001",
      attemptId: "coding_attempt_0001",
      status: "running",
      stage: "coding",
      automation: null,
    });

    const user = userEvent.setup();
    renderCodingPage();

    expect(screen.queryByTestId("automation-termination-entry")).toBeNull();
    expect(screen.queryByTestId("automation-terminated-banner")).toBeNull();

    const abortButton = screen.getByRole("button", { name: "中止" });
    await user.click(abortButton);
    expect(api.abortAttempt).toHaveBeenCalledTimes(1);
  });

  // 2026-11-06 陈旧回执修复：同 issue「终止 → 删 work item → 新建」后
  // enrollment 换绑新 plan/session——前端内存残留的旧回执不得盖在新会话上
  // 呈现「自动化已终止」。当前 plan 身份取 automation.plan_id（WS 快照同源
  // 投影）；回执缺 planId 或当前身份不可得均 fail-closed 不展示。
  function seedTerminationReceipt(planId: string | null) {
    useCodingWorkspaceStore.setState({
      planTermination: {
        pending: null,
        submitting: false,
        error: null,
        terminated: {
          projectId: "project_0001",
          issueId: "issue_0001",
          planId,
          terminatedAt: "2026-10-16T06:00:00Z",
          outcome: "terminated",
        },
      },
    });
  }

  it("同 issue 旧回执（planId=旧 plan）+ 当前是新 plan ⇒ 横幅不渲染，终止入口恢复", () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      projectId: "project_0001",
      issueId: "issue_0001",
      attemptId: "coding_attempt_0001",
      status: "running",
      stage: "coding",
      automation: { ...MANAGED_AUTOMATION, plan_id: "plan_0002" },
    });
    seedTerminationReceipt("plan_0001");

    renderCodingPage();

    expect(screen.queryByTestId("automation-terminated-banner")).toBeNull();
    // 新 plan 未终止 ⇒ 统一终止入口恢复（不再被陈旧回执压住）。
    expect(screen.getByTestId("automation-termination-entry")).toBeInTheDocument();
  });

  it("回执缺 planId（旧响应/回放）⇒ 横幅不渲染（fail-closed）", () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      projectId: "project_0001",
      issueId: "issue_0001",
      attemptId: "coding_attempt_0001",
      status: "running",
      stage: "coding",
      automation: MANAGED_AUTOMATION,
    });
    seedTerminationReceipt(null);

    renderCodingPage();

    expect(screen.queryByTestId("automation-terminated-banner")).toBeNull();
    expect(screen.getByTestId("automation-termination-entry")).toBeInTheDocument();
  });

  it("当前 plan 身份不可得（automation.plan_id 缺省）⇒ 横幅不渲染（fail-closed）", () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      projectId: "project_0001",
      issueId: "issue_0001",
      attemptId: "coding_attempt_0001",
      status: "running",
      stage: "coding",
      automation: { ...MANAGED_AUTOMATION, plan_id: null },
    });
    seedTerminationReceipt("plan_0001");

    renderCodingPage();

    expect(screen.queryByTestId("automation-terminated-banner")).toBeNull();
  });

  // r2（Important 1 / REQ-WIGA-14「旧代 outcome 不得叠加到新代」）：横幅
  // outcome 与 terminatedAt 同走身份门——服务端主源（automation.
  // terminated_at）渲染横幅时，旧代回执（planId 不匹配）的 cleanup_required
  // 文案不得叠加到新代横幅上。
  it("主源终止态下旧代回执的 cleanup_required 文案不得叠加（outcome 同走身份门）", () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      projectId: "project_0001",
      issueId: "issue_0001",
      attemptId: "coding_attempt_0001",
      status: "aborted",
      stage: "coding",
      // 终止后形态：主源 terminated_at 渲染横幅；plan_id=本（新）plan。
      automation: {
        ...MANAGED_AUTOMATION,
        enabled: false,
        terminated_at: "2026-10-16T07:00:00Z",
      },
    });
    // 旧代回执：planId≠当前 plan，且带 cleanup_required。
    useCodingWorkspaceStore.setState({
      planTermination: {
        pending: null,
        submitting: false,
        error: null,
        terminated: {
          projectId: "project_0001",
          issueId: "issue_0001",
          planId: "plan_0000",
          terminatedAt: "2026-10-16T06:00:00Z",
          outcome: "cleanup_required",
        },
      },
    });

    renderCodingPage();

    const banner = screen.getByTestId("automation-terminated-banner");
    // 主源时间戳照常（终止语义本体不弱化）。
    expect(banner).toHaveTextContent("2026-10-16T07:00:00Z");
    expect(banner).toHaveTextContent("本计划全部仓库已停止，不可继续；已产出文件保留。");
    // 旧代回执的时间戳与 cleanup_required 文案均不叠加。
    expect(banner).not.toHaveTextContent("2026-10-16T06:00:00Z");
    expect(banner).not.toHaveTextContent("清理未完成");
    expect(banner).not.toHaveTextContent("需管理员介入");
  });

  it("主源终止态 + 本 plan 回执 cleanup_required ⇒ 管理员介入文案照常（不弱化）", () => {
    mockCodingWs();
    useCodingWorkspaceStore.setState({
      projectId: "project_0001",
      issueId: "issue_0001",
      attemptId: "coding_attempt_0001",
      status: "aborted",
      stage: "coding",
      automation: {
        ...MANAGED_AUTOMATION,
        enabled: false,
        terminated_at: "2026-10-16T07:00:00Z",
      },
    });
    useCodingWorkspaceStore.setState({
      planTermination: {
        pending: null,
        submitting: false,
        error: null,
        terminated: {
          projectId: "project_0001",
          issueId: "issue_0001",
          planId: "plan_0001",
          terminatedAt: "2026-10-16T06:00:00Z",
          outcome: "cleanup_required",
        },
      },
    });

    renderCodingPage();

    const banner = screen.getByTestId("automation-terminated-banner");
    expect(banner).toHaveTextContent("清理未完成");
    expect(banner).toHaveTextContent("需管理员介入");
  });
});
