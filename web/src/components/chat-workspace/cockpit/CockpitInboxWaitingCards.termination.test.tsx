// 批 2 T4（REQ-WIGA-09/08）红测：收件箱统一终止与二次确认面。
// 契约：plan §2 波 5 T4 ①②③——收件箱 abort 入口与页面共用同一确认面；
// 未确认零请求副作用；REST 428 automation_plan_termination_required 只弹
// 确认面（不落错误）；确认后 confirmed:true 提交统一终止 REST。
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type * as ApiClient from "../../../api/client";
import {
  ApiRequestError,
  getAutomationEnrollment,
  postAutomationEnrollmentTerminate,
  postCodingGateResponse,
} from "../../../api/client";
import type { IssueAutomationEnrollment } from "../../../api/types";
import type { CockpitActionFacade } from "../../../state/cockpit-action-routing";
import { useCodingWorkspaceStore } from "../../../state/coding-workspace-store";
import type { CockpitInboxItem } from "../../../state/workspace-cockpit-projection";
import {
  AutomationTerminatedBanner,
  AutomationTerminationConfirmSurface,
  C1RecoveryCard,
} from "./CockpitInboxWaitingCards";

vi.mock("../../../api/client", async (importOriginal) => ({
  ...(await importOriginal<typeof ApiClient>()),
  getAutomationEnrollment: vi.fn(),
  postAutomationEnrollmentTerminate: vi.fn(),
  postCodingGateResponse: vi.fn(),
}));

const TERMINATE_IDENTITY_DETAILS = {
  confirm_required: true,
  plan_id: "plan_0001",
  session_id: "session_0001",
  enrollment_id: "enrollment_0001",
  policy_revision: 3,
  binding_version: 2,
  terminated_at: null,
};

function c1AbortItem(): CockpitInboxItem {
  return {
    id: "issue_0001:c1:stopped:session_0001",
    kind: "c1_recovery",
    title: "Coding 等待人工",
    summary: "coding 门等待",
    createdAt: "2026-10-16T05:00:00Z",
    sessionId: "session_0001",
    triage: false,
    gate: null,
    choice: null,
    c1Info: {
      projectId: "project_0001",
      issueId: "issue_0001",
      itemId: "c1:session_0001",
      kind: "coding_gate",
      reason: "blocked",
      completedSteps: [],
      targetLabel: "repo-a",
      planId: "plan_0001",
      sessionId: "session_0001",
      attemptId: "coding_attempt_0001",
      gateId: "coding_blocked_gate_0001",
      possibleSideEffect: null,
      actions: ["abort"],
      nextPhase: null,
      expectedVersion: 7,
      actionContext: [
        { action: "abort", commandId: "cmd-gate-abort-1", expectedVersion: 7 },
      ],
      operationId: null,
      diagnostics: null,
    },
  } as unknown as CockpitInboxItem;
}

function stubActions(): CockpitActionFacade {
  return { respondGate: vi.fn() } as unknown as CockpitActionFacade;
}

function enrollmentFixture(): IssueAutomationEnrollment {
  return {
    enrollment_id: "enrollment_0001",
    policy_revision: 5,
  } as IssueAutomationEnrollment;
}

describe("AutomationTerminationConfirmSurface（T4 统一二次确认面）", () => {
  beforeEach(() => {
    useCodingWorkspaceStore.getState().reset();
    vi.clearAllMocks();
  });

  it("renders nothing while no termination is pending", () => {
    render(<AutomationTerminationConfirmSurface />);
    expect(screen.queryByTestId("automation-termination-confirm")).toBeNull();
  });

  it("sends zero requests until double-confirmed, then submits confirmed:true (identity resolved via GET)", async () => {
    vi.mocked(getAutomationEnrollment).mockResolvedValue(enrollmentFixture());
    vi.mocked(postAutomationEnrollmentTerminate).mockResolvedValue({
      command_id: "cmd-fe-terminate-enrollment_0001",
      state: "accepted",
      enrollment_id: "enrollment_0001",
      plan_id: "plan_0001",
      terminated_at: "2026-10-16T06:00:00Z",
      outcome: "terminated",
      results: [],
    });
    useCodingWorkspaceStore.getState().openTerminationConfirm({
      projectId: "project_0001",
      issueId: "issue_0001",
      // 身份缺失：提交前经 GET enrollment 补读（未知不猜）。
      enrollmentId: null,
      policyRevision: null,
    });

    const user = userEvent.setup();
    render(<AutomationTerminationConfirmSurface />);

    expect(screen.getByTestId("automation-termination-confirm")).toBeInTheDocument();
    expect(getAutomationEnrollment).not.toHaveBeenCalled();
    expect(postAutomationEnrollmentTerminate).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "终止整个自动化" }));
    expect(postAutomationEnrollmentTerminate).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "确认终止整个自动化" }));

    await vi.waitFor(() => {
      expect(postAutomationEnrollmentTerminate).toHaveBeenCalledWith(
        "project_0001",
        "issue_0001",
        {
          command_id: "cmd-fe-terminate-enrollment_0001",
          expected_enrollment_id: "enrollment_0001",
          expected_policy_revision: 5,
          confirmed: true,
        },
      );
    });

    // 成功后：确认面关闭，终态（terminated_at/outcome）进 store。
    expect(screen.queryByTestId("automation-termination-confirm")).toBeNull();
    expect(useCodingWorkspaceStore.getState().planTermination.terminated).toEqual({
      projectId: "project_0001",
      issueId: "issue_0001",
      terminatedAt: "2026-10-16T06:00:00Z",
      outcome: "terminated",
    });
  });

  it("shows the server rejection inline without closing the confirm surface", async () => {
    vi.mocked(postAutomationEnrollmentTerminate).mockRejectedValue(
      new ApiRequestError({
        code: "automation_enrollment_conflict",
        message: "termination enrollment identity mismatch",
        details: {},
      }),
    );
    useCodingWorkspaceStore.getState().openTerminationConfirm({
      projectId: "project_0001",
      issueId: "issue_0001",
      enrollmentId: "enrollment_0001",
      policyRevision: 3,
    });

    const user = userEvent.setup();
    render(<AutomationTerminationConfirmSurface />);
    await user.click(screen.getByRole("button", { name: "终止整个自动化" }));
    await user.click(screen.getByRole("button", { name: "确认终止整个自动化" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "termination enrollment identity mismatch",
    );
    expect(screen.getByTestId("automation-termination-confirm")).toBeInTheDocument();
  });
});

describe("C1RecoveryCard abort routing（T4 收件箱统一终止）", () => {
  beforeEach(() => {
    useCodingWorkspaceStore.getState().reset();
    vi.clearAllMocks();
  });

  it("manual: fires the original gate-responses REST abort and opens no confirm surface", async () => {
    vi.mocked(postCodingGateResponse).mockResolvedValue(undefined as never);
    const user = userEvent.setup();

    render(<C1RecoveryCard item={c1AbortItem()} actions={stubActions()} />);
    await user.click(screen.getByTestId("c1-action-abort"));

    expect(postCodingGateResponse).toHaveBeenCalledWith(
      {
        projectId: "project_0001",
        issueId: "issue_0001",
        attemptId: "coding_attempt_0001",
      },
      {
        command_id: "cmd-gate-abort-1",
        gate_id: "coding_blocked_gate_0001",
        action_id: "abort",
        expected_version: 7,
      },
    );
    expect(useCodingWorkspaceStore.getState().planTermination.pending).toBeNull();
  });

  it("managed 428: opens the unified confirm surface with the identity from details, no error popup", async () => {
    vi.mocked(postCodingGateResponse).mockRejectedValue(
      new ApiRequestError({
        code: "automation_plan_termination_required",
        message: "automation plan termination requires unified confirmation",
        details: TERMINATE_IDENTITY_DETAILS,
      }),
    );
    const user = userEvent.setup();

    render(<C1RecoveryCard item={c1AbortItem()} actions={stubActions()} />);
    await user.click(screen.getByTestId("c1-action-abort"));

    expect(useCodingWorkspaceStore.getState().planTermination.pending).toEqual({
      projectId: "project_0001",
      issueId: "issue_0001",
      enrollmentId: "enrollment_0001",
      policyRevision: 3,
    });
    expect(postAutomationEnrollmentTerminate).not.toHaveBeenCalled();
  });
});

describe("AutomationTerminatedBanner（T4 终止态呈现）", () => {
  it("shows the terminated_at timestamp and the irreversibility copy", () => {
    render(<AutomationTerminatedBanner terminatedAt="2026-10-16T06:00:00Z" />);

    const banner = screen.getByTestId("automation-terminated-banner");
    expect(banner).toHaveTextContent("自动化已终止");
    expect(banner).toHaveTextContent("2026-10-16T06:00:00Z");
    expect(banner).toHaveTextContent("已产出文件保留");
  });
});
