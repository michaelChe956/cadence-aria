// 批 2 T4（REQ-WIGA-09/08）红测：总览（plan 对话驾驶舱）的统一终止入口。
// 契约：plan §2 波 5 T4 ①⑤——托管 plan 会话显示「终止整个自动化」，点击
// 走与 Coding Workspace/收件箱同一二次确认面；ChatInputBar 的旧中止入口在
// 托管下路由统一终止；手动会话原 workspaceWs.abort 语义不变。
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type * as ApiClient from "../api/client";
import { findWorkspaceSessionIssueAddress } from "../api/client";
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
});
