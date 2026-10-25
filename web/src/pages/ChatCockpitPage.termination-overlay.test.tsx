// F-C 回归（owner 2026-10-25 真机复现）：「终止整个自动化」的二次确认面在会话页
// **看不见、点不到**。
//
// 旧缺陷形态（真机 headless Chromium 实测）：点击入口后 `automation-termination-confirm`
// DOM 存在、isVisible=true，但 rect 高 17px（内容 scrollHeight=44px），按钮中心
// `elementFromPoint` 命中对话流里的 DIV 而不是面板内元素 —— 因为右列用的是
// 「按 pending choice 二选一的 grid 行模板」，而无条件渲染的确认面被自动放进
// `minmax(0,1fr)` 行压成 0px，再被后序的对话流整块覆盖。
//
// 因此本文件的断言**不能只断言 DOM 存在**，必须断言"可点"的充分条件：
//   1. 确认面不在对话流 section 的 DOM 子树里（不再参与该 grid 行的分配，
//      也不可能被后序兄弟在层叠上覆盖）；
//   2. 确认面经 createPortal 挂在 document.body 的 `fixed inset-0` 浮层里，
//      层级 token 为顶层 z（视口浮层，与页面排布完全解耦）；
//   3. 卡片自身 pointer-events-auto（DialogShell 的定位壳 pointer-events-none
//      只让蒙层可点，卡片内交互不被拦截）；
//   4. 对话流右列已不是 grid 行模板（同源缺陷不再可复现）；
//   5. 真点两下（双确认）能命中并真的发出 terminate 请求（确认按钮可用，不是死按钮）。
// jsdom 没有布局引擎（elementFromPoint/getBoundingClientRect 均无几何），故用
// 上述结构不变量 + 真点击等价替代；真机 elementFromPoint 复核见回包说明。
import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type * as ApiClient from "../api/client";
import {
  findWorkspaceSessionIssueAddress,
  postAutomationEnrollmentTerminate,
} from "../api/client";
import type * as WorkspaceWsModule from "../hooks/useWorkspaceWs";
import { readCockpitSettings } from "../state/cockpit-settings";
import { useCodingWorkspaceStore } from "../state/coding-workspace-store";
import { useWorkspaceStore } from "../state/workspace-ws-store";
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
  useCockpitShellInbox: () => (cockpitInbox.length > 0 ? cockpitInbox : []),
  useCockpitInboxPulse: () => false,
  useCockpitSessionWatch: () => watchSession,
  useCockpitSettings: () => readCockpitSettings(),
  useCockpitObservedRecords: () => cockpitObservedRecords,
  useCockpitSettingsSlotRef: () => () => undefined,
  useCockpitAttentionTargetOpener: () => () => undefined,
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

describe("ChatCockpitPage「终止整个自动化」确认面可点性（F-C 回归）", () => {
  installCockpitPageTestHooks();

  beforeEach(() => {
    useCodingWorkspaceStore.getState().reset();
    vi.mocked(findWorkspaceSessionIssueAddress).mockResolvedValue({
      projectId: "project_0001",
      issueId: "issue_0001",
    });
    vi.mocked(postAutomationEnrollmentTerminate).mockResolvedValue({
      command_id: "cmd-fe-terminate-enrollment_0001",
      state: "accepted",
      enrollment_id: "enrollment_0001",
      plan_id: "plan_0001",
      terminated_at: "2026-10-25T06:00:00Z",
      outcome: "terminated",
      results: [],
    });
  });

  it("点击入口后确认面在视口浮层内、脱离对话流行模板，且确认按钮真的可点生效", async () => {
    setManagedPlanSession();
    const user = userEvent.setup();
    renderCockpit("session_001");

    expect(screen.queryByTestId("automation-termination-confirm")).toBeNull();

    await user.click(screen.getByTestId("automation-termination-entry"));

    const surface = screen.getByTestId("automation-termination-confirm");
    const conversation = screen.getByTestId("cockpit-conversation-flow");

    // 1) 确认面不是对话流 section 的后代：不再被塞进它的行模板、也不可能被它覆盖。
    expect(conversation).not.toContainElement(surface);
    // 对话流本身仍在（确认面可点性不是靠隐藏对话流换来的）。
    expect(screen.getByTestId("cockpit-conversation-flow-list")).toBeInTheDocument();

    // 2) 浮层：createPortal 到 document.body 的 fixed inset-0 层，层级为顶层 z。
    const layer = surface.closest('[data-testid="automation-termination-confirm-layer"]');
    expect(layer).not.toBeNull();
    expect(layer!.parentElement).toBe(document.body);
    expect(layer).toHaveClass("fixed", "inset-0", "z-[80]");

    // 3) 卡片自身接收指针事件（DialogShell 的定位壳 pointer-events-none 只作用于蒙层）。
    expect(surface).toHaveClass("pointer-events-auto");

    // 4) 同源缺陷不再可复现：右列已是 flex 纵向流，没有按状态切换的 grid 行模板。
    expect(conversation).toHaveClass("flex-col");
    expect(conversation.className).not.toContain("grid-rows");

    // 5) 真点两下命中并提交（双确认）：死按钮在这步会失败。
    await user.click(within(surface).getByRole("button", { name: "终止整个自动化" }));
    await user.click(within(surface).getByRole("button", { name: "确认终止整个自动化" }));

    await vi.waitFor(() => {
      expect(postAutomationEnrollmentTerminate).toHaveBeenCalledWith(
        "project_0001",
        "issue_0001",
        expect.objectContaining({ confirmed: true }),
      );
    });

    // 提交成功即收层：确认面（含浮层）整体卸载。
    expect(screen.queryByTestId("automation-termination-confirm")).toBeNull();
    expect(
      document.querySelector('[data-testid="automation-termination-confirm-layer"]'),
    ).toBeNull();
    expect(currentMockWorkspaceWs().abort).not.toHaveBeenCalled();
  });
});
