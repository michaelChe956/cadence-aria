// 批 2 T4（REQ-WIGA-09/08）红测：总览（plan 对话驾驶舱）的统一终止入口。
// 契约：plan §2 波 5 T4 ①⑤——托管 plan 会话显示「终止整个自动化」，点击
// 走与 Coding Workspace/收件箱同一二次确认面；ChatInputBar 的旧中止入口在
// 托管下路由统一终止；手动会话原 workspaceWs.abort 语义不变。
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type * as ApiClient from "../api/client";
import { findWorkspaceSessionIssueAddress, getAutomationEnrollmentControl } from "../api/client";
import type * as WorkspaceWsModule from "../hooks/useWorkspaceWs";
import { readCockpitSettings } from "../state/cockpit-settings";
import { useCodingWorkspaceStore } from "../state/coding-workspace-store";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import { ChatCockpitPage } from "./ChatCockpitPage";
import {
  cockpitInbox,
  cockpitObservedRecords,
  installCockpitPageTestHooks,
  renderCockpit,
  watchSession,
} from "./ChatCockpitPage.test-utils";
import { currentMockWorkspaceWs } from "./ChatWorkspacePage.test-utils";
vi.mock("../api/client", async (importOriginal) => ({
  ...(await importOriginal<typeof ApiClient>()),
  findWorkspaceSessionIssueAddress: vi.fn(),
  getAutomationEnrollment: vi.fn(),
  getAutomationEnrollmentControl: vi.fn(),
  postAutomationEnrollmentTerminate: vi.fn(),
  takeoverWorkspaceSession: vi.fn(),
}));

vi.mock("../state/bulk-confirm-store", () => ({
  useBulkConfirmStore: Object.assign(
    (selector: (state: { runs: readonly unknown[] }) => unknown) =>
      selector({ runs: [] }),
    { getState: () => ({ start: vi.fn() }) },
  ),
}));

vi.mock("../components/cockpit/CockpitShell", async (importOriginal) => ({
  ...(await importOriginal()),
}));

vi.mock("../hooks/useWorkspaceWs", async (importOriginal) => ({
  ...(await importOriginal<typeof WorkspaceWsModule>()),
  useWorkspaceWs: vi.fn(),
}));

vi.mock("../hooks/useUnloadGuard", () => ({
  useUnloadGuard: vi.fn(),
}));

vi.mock("../api/workspace-content", () => ({
  fetchWorkspaceArtifactVersion: vi.fn(),
  fetchWorkspaceEventOutput: vi.fn(),
  fetchWorkspaceNodeDetail: vi.fn(),
  fetchWorkspacePrompt: vi.fn(),
}));

vi.mock("../components/shared/MonacoViewer", () => ({
  MonacoViewer: ({ value }: { value: string }) => (
    <div data-testid="monaco-viewer">{value}</div>
  ),
}));

vi.mock("../components/cockpit/CockpitShell", () => ({
  useCockpitCodingAttemptForSession: () => () => null,
  useCockpitShellInbox: () =>
    cockpitInbox.length > 0
      ? cockpitInbox
      : [],
  useCockpitInboxPulse: () => false,
  useCockpitSessionWatch: () => watchSession,
  useCockpitSettings: () => readCockpitSettings(),
  useCockpitObservedRecords: () => cockpitObservedRecords,
  useCockpitSettingsSlotRef: () => () => undefined,
  useCockpitAttentionTargetOpener: () => () => undefined,
  // UI-3：共享待处理抽屉聚焦（null → 消费方回落旧导航，保持各测试原意图）。
  useCockpitInboxFocusOpener: () => null,
}));

const MANAGED_AUTOMATION = {
  owner: "server" as const,
  enrollment_id: "enrollment_0001",
  policy_revision: 3,
  enabled: true,
};

function setManagedPlanSession() {
  useWorkspaceStore.setState({
    workspaceType: "work_item_plan",
    stage: "running",
    sessionStatus: "running",
    automation: MANAGED_AUTOMATION,
  });
}

describe("ChatCockpitPage automation termination（T4 总览入口）", () => {
  installCockpitPageTestHooks();

  beforeEach(() => {
    useCodingWorkspaceStore.getState().reset();
    vi.mocked(findWorkspaceSessionIssueAddress).mockResolvedValue({
      projectId: "project_0001",
      issueId: "issue_0001",
    });
    // control 默认失败（等价此前未 mock 时真实 fetch 在 jsdom 失败 →
    // planControl=null，fail-closed）；需要投影的用例各自 mockResolvedValue。
    vi.mocked(getAutomationEnrollmentControl).mockRejectedValue(
      new Error("control projection not stubbed for this test"),
    );
  });

  it("managed plan 会话显示统一终止入口；点击经目录解析打开同一确认面", async () => {
    setManagedPlanSession();
    const user = userEvent.setup();
    renderCockpit("session_001");

    const entry = screen.getByTestId("automation-termination-entry");
    expect(entry).toBeInTheDocument();

    await user.click(entry);

    expect(findWorkspaceSessionIssueAddress).toHaveBeenCalledWith("session_001");
    expect(useCodingWorkspaceStore.getState().planTermination.pending).toEqual({
      projectId: "project_0001",
      issueId: "issue_0001",
      enrollmentId: "enrollment_0001",
      policyRevision: 3,
    });
    expect(screen.getByTestId("automation-termination-confirm")).toBeInTheDocument();
  });

  it("托管下 ChatInputBar 旧中止入口路由统一终止（不发改写类 WS abort）", async () => {
    setManagedPlanSession();
    const user = userEvent.setup();
    renderCockpit("session_001");

    await user.click(screen.getByRole("button", { name: "中止" }));

    expect(useCodingWorkspaceStore.getState().planTermination.pending).not.toBeNull();
    expect(currentMockWorkspaceWs().abort).not.toHaveBeenCalled();
  });

  it("手动会话（非托管）不显示终止入口，旧中止仍走 workspaceWs.abort", async () => {
    useWorkspaceStore.setState({
      workspaceType: "work_item_plan",
      stage: "running",
      sessionStatus: "running",
      automation: {
        owner: "client" as const,
        enrollment_id: null,
        policy_revision: null,
        enabled: false,
      },
    });
    const user = userEvent.setup();
    renderCockpit("session_001");

    expect(screen.queryByTestId("automation-termination-entry")).toBeNull();

    await user.click(screen.getByRole("button", { name: "中止" }));
    expect(useCodingWorkspaceStore.getState().planTermination.pending).toBeNull();
    expect(currentMockWorkspaceWs().abort).toHaveBeenCalledTimes(1);
  });

  // 2026-11-06 陈旧回执修复：同 issue「终止 → 删 work item → 新建」后
  // enrollment 换绑新 plan——前端内存残留的旧回执（planId=旧 plan）不得再
  // 盖在正常推进的新会话上呈现「自动化已终止」。当前 plan 身份取 control
  // 投影 plan_id（与终止响应同源）；缺身份/不一致均 fail-closed 不展示。
  function controlFixture(planId: string | null) {
    return {
      managed: true,
      plan_id: planId,
      session_id: "session_001",
      terminated_at: null,
      stop_key: null,
      stage: "running",
      allowed_actions: [],
      context_required: false,
    };
  }

  function seedStaleTerminationReceipt(planId: string | null) {
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

  it("同 issue 旧回执（planId=旧 plan）+ 当前是新 plan ⇒ 横幅不渲染，终止入口恢复", async () => {
    setManagedPlanSession();
    vi.mocked(getAutomationEnrollmentControl).mockResolvedValue(controlFixture("plan_0002"));
    seedStaleTerminationReceipt("plan_0001");
    renderCockpit("session_001");

    // 先等 control 投影落地（否则「不渲染」可能是尚未走到守卫的假阴性）。
    await vi.waitFor(() => {
      expect(getAutomationEnrollmentControl).toHaveBeenCalledWith("project_0001", "issue_0001");
    });

    expect(screen.queryByTestId("automation-terminated-banner")).toBeNull();
    // 新 plan 未终止 ⇒ 统一终止入口恢复可见（不再被陈旧回执压住）。
    expect(screen.getByTestId("automation-termination-entry")).toBeInTheDocument();
  });

  it("当前 plan 身份不可得（control 无 plan_id）⇒ 横幅不渲染（fail-closed）", async () => {
    setManagedPlanSession();
    vi.mocked(getAutomationEnrollmentControl).mockResolvedValue(controlFixture(null));
    seedStaleTerminationReceipt("plan_0001");
    renderCockpit("session_001");

    await vi.waitFor(() => {
      expect(getAutomationEnrollmentControl).toHaveBeenCalled();
    });

    expect(screen.queryByTestId("automation-terminated-banner")).toBeNull();
  });

  it("回执 planId 与当前 plan 一致 ⇒ 横幅照常渲染（原终止语义不弱化）", async () => {
    setManagedPlanSession();
    vi.mocked(getAutomationEnrollmentControl).mockResolvedValue(controlFixture("plan_0001"));
    seedStaleTerminationReceipt("plan_0001");
    renderCockpit("session_001");

    await vi.waitFor(() => {
      expect(getAutomationEnrollmentControl).toHaveBeenCalled();
    });

    expect(await screen.findByTestId("automation-terminated-banner")).toHaveTextContent(
      "2026-10-16T06:00:00Z",
    );
    expect(screen.queryByTestId("automation-termination-entry")).toBeNull();
  });
});
