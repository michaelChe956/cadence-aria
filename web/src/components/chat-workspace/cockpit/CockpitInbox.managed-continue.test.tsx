// 托管 stopped 卡（CockpitInbox 行）落地「补说明并继续」入口：
// - 托管卡：不出接管、出继续入口（control 允许时），点击→输入→提交走
//   continue-plan REST（expected_binding/expected_stop_key 来自 control）；
// - 非托管卡：接管分支原样保留（回归），不出现继续入口；
// - control 不含 continue_plan：不出继续按钮，托管指引仍在。
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type * as ApiClient from "../../../api/client";
import {
  findWorkspaceSessionIssueAddress,
  getAutomationEnrollmentControl,
  postAutomationContinuePlan,
  type AutomationControlStatus,
} from "../../../api/client";
import type * as WorkspaceWsModule from "../../../hooks/useWorkspaceWs";
import type { CockpitInboxItem } from "../../../state/workspace-cockpit-projection";
import { CockpitInbox } from "./CockpitInbox";

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
  newCommandId: vi.fn(() => "cmd-card-continue-1"),
}));

const BINDING: NonNullable<AutomationControlStatus["binding"]> = {
  binding_version: 2,
  enrollment_id: "enrollment_0001",
  plan_id: "issue_work_item_plan_auto_1",
  session_id: "session_001",
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
    session_id: "session_001",
    terminated_at: null,
    stop_key: "plan_gate:session_001:human_confirm",
    stage: "stopped_needs_human",
    allowed_actions: ["continue_plan"],
    context_required: true,
    ...overrides,
  };
}

function stoppedItem(sessionId: string, managed: boolean): CockpitInboxItem {
  return {
    id: `${sessionId}:stopped:${sessionId}`,
    kind: "stopped",
    severity: 2,
    title: "会话停在停点",
    summary: managed
      ? "停点原因：stopped_needs_human · 自动化托管中——在对话窗口补说明继续"
      : "停点原因：stopped_needs_human · 等待人工接管后继续",
    triage: false,
    source: "session_status",
    createdAt: null,
    gate: null,
    inlineError: null,
    choice: null,
    ...(managed ? { automationManaged: true } : {}),
  };
}

describe("CockpitInbox managed stopped continue entry", () => {
  beforeEach(() => {
    vi.mocked(findWorkspaceSessionIssueAddress).mockReset();
    vi.mocked(findWorkspaceSessionIssueAddress).mockResolvedValue({
      projectId: "project_0001",
      issueId: "issue_0001",
    });
    vi.mocked(getAutomationEnrollmentControl).mockReset();
    vi.mocked(getAutomationEnrollmentControl).mockResolvedValue(control());
    vi.mocked(postAutomationContinuePlan).mockReset();
  });

  afterEach(() => cleanup());

  it("offers the continue entry on managed cards while unmanaged keeps takeover", async () => {
    render(
      <CockpitInbox
        items={[stoppedItem("session_001", true), stoppedItem("session_002", false)]}
        onTakeover={vi.fn(async () => undefined)}
      />,
    );

    const rows = screen.getAllByTestId("cockpit-inbox-item-stopped");
    expect(rows).toHaveLength(2);
    // 托管卡：无接管，托管指引 + 继续入口。
    expect(within(rows[0]!).queryByRole("button", { name: "接管" })).toBeNull();
    expect(within(rows[0]!).getByTestId("stopped-managed-hint")).toBeVisible();
    expect(
      await within(rows[0]!).findByRole("button", { name: "补说明并继续" }),
    ).toBeVisible();
    // 非托管卡：接管原样、无继续入口（回归）。
    expect(within(rows[1]!).getByRole("button", { name: "接管" })).toBeVisible();
    expect(within(rows[1]!).queryByTestId("stopped-managed-continue")).toBeNull();
  });

  it("submits the card note through continue-plan with control facts", async () => {
    const user = userEvent.setup();
    vi.mocked(postAutomationContinuePlan).mockResolvedValue({
      command_id: "cmd-card-continue-1",
      state: "accepted",
      session_id: "session_001",
    });

    render(<CockpitInbox items={[stoppedItem("session_001", true)]} />);

    const row = screen.getByTestId("cockpit-inbox-item-stopped");
    await user.click(await within(row).findByRole("button", { name: "补说明并继续" }));
    await user.type(
      await within(row).findByTestId("stopped-managed-continue-context"),
      "把评审意见补进 plan",
    );
    await user.click(within(row).getByTestId("stopped-managed-continue-submit"));

    await waitFor(() => {
      expect(postAutomationContinuePlan).toHaveBeenCalledTimes(1);
    });
    expect(postAutomationContinuePlan).toHaveBeenCalledWith(
      "project_0001",
      "issue_0001",
      {
        command_id: "cmd-card-continue-1",
        expected_binding: BINDING,
        expected_stop_key: "plan_gate:session_001:human_confirm",
        context: "把评审意见补进 plan",
      },
    );
  });

  it("hides the continue button when control no longer allows continue_plan", async () => {
    vi.mocked(getAutomationEnrollmentControl).mockResolvedValue(
      control({ allowed_actions: ["terminate"], terminated_at: null }),
    );

    render(<CockpitInbox items={[stoppedItem("session_001", true)]} />);

    const row = screen.getByTestId("cockpit-inbox-item-stopped");
    // 托管指引照旧可见，动作面 fail-closed 不出。
    expect(within(row).getByTestId("stopped-managed-hint")).toBeVisible();
    await waitFor(() => {
      expect(getAutomationEnrollmentControl).toHaveBeenCalled();
    });
    expect(within(row).queryByTestId("stopped-managed-continue")).toBeNull();
  });
});
