import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import {
  linkedWorkspaceAmendmentSnapshotFixture,
  repairAwaitingConfirmationFixture,
} from "../components/coding-workspace/plan-repair-test-fixtures";
import { useCodingWorkspaceStore } from "../state/coding-workspace-store";
import { useLinkedWorkspaceAmendmentStore } from "../state/linked-workspace-amendment-store";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import { CodingWorkspacePage } from "./CodingWorkspacePage";
import {
  CODING_ATTEMPT_ADDRESS,
  executionPlan,
  installCodingWorkspacePageTestHooks,
  mockCodingWs,
  mockPlanRepairWs,
  readyCodingState,
} from "./CodingWorkspacePage.test-utils";

vi.mock("../api/client", () => ({
  deleteCodingAttempt: vi.fn(),
  getCodingAttemptDiff: vi.fn(),
}));

vi.mock("../hooks/useCodingWorkspaceWs", () => ({
  useCodingWorkspaceWs: vi.fn(),
}));

vi.mock("../hooks/useWorkspaceWs", () => ({
  useWorkspaceWs: vi.fn(),
}));

vi.mock("../hooks/useUnloadGuard", () => ({
  useUnloadGuard: vi.fn(),
}));

describe("CodingWorkspacePage plan repair", () => {
  installCodingWorkspacePageTestHooks();

  it("renders plan repair inline and preserves the coding workspace route", () => {
    const repair = repairAwaitingConfirmationFixture();
    mockCodingWs();
    mockPlanRepairWs();
    window.history.replaceState(
      {},
      "",
      "/workbench/projects/project_0001/issues/issue_0001/coding/coding_attempt_0001",
    );
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      status: "awaiting_plan_amendment",
      stage: "code_review",
      activePlanRepair: repair,
      timelineNodes: repair.timelineNodes,
    });

    render(
      <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />,
    );

    expect(screen.getByRole("heading", { name: "Plan Repair" })).toBeInTheDocument();
    expect(screen.getByText("WI-01 初始化领域模型")).toBeInTheDocument();
    expect(screen.getByText("新增 failure_message")).toBeInTheDocument();
    expect(window.location.pathname).toContain("/coding/coding_attempt_0001");
    expect(window.location.pathname).not.toContain("workspace_session_repair_0001");
  });

  it.each([
    ["confirm", "确认修订并恢复执行"],
    ["regenerate", "要求重新生成"],
    ["cancel", "取消修订"],
  ] as const)("sends the %s repair decision through the child transport", async (action, label) => {
    const repair = repairAwaitingConfirmationFixture();
    mockCodingWs();
    const repairApi = mockPlanRepairWs();
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      status: "awaiting_plan_amendment",
      stage: "code_review",
      activePlanRepair: repair,
      timelineNodes: repair.timelineNodes,
    });

    render(
      <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />,
    );

    await userEvent.click(screen.getByRole("button", { name: label }));

    if (action === "confirm") {
      expect(repairApi.confirmPlanAmendment).toHaveBeenCalledWith(repair.amendment?.id);
    } else if (action === "cancel") {
      expect(repairApi.cancelPlanAmendment).toHaveBeenCalledWith(
        repair.amendment?.id,
        "用户取消修订",
      );
    } else {
      expect(repairApi.sendHumanConfirm).toHaveBeenCalledWith(
        "request-change",
        {
          description:
            action === "regenerate"
              ? "要求重新生成 Plan Repair 修订"
              : "调整 Plan Repair 修订范围",
        },
      );
    }
  });

  it("starts Story or Design amendment inside adjust scope without leaving Coding Workspace", async () => {
    const repair = repairAwaitingConfirmationFixture();
    mockCodingWs();
    const startLinkedWorkspaceAmendment = vi.fn(() => true);
    const repairApi = mockPlanRepairWs({ startLinkedWorkspaceAmendment });
    window.history.replaceState(
      {},
      "",
      "/workbench/projects/project_0001/issues/issue_0001/coding/coding_attempt_0001",
    );
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      status: "awaiting_plan_amendment",
      stage: "code_review",
      workItemExecutionPlan: executionPlan(),
      activePlanRepair: repair,
      timelineNodes: repair.timelineNodes,
    });

    render(
      <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "调整修订范围" }));
    await userEvent.selectOptions(screen.getByLabelText("修订类型"), "design");
    await userEvent.click(screen.getByRole("button", { name: "发起关联修订" }));

    expect(startLinkedWorkspaceAmendment).toHaveBeenCalledWith({
      entity_id: "design_spec_0001",
      workspace_type: "design",
      relation: "design_amendment",
    });
    expect(repairApi.sendHumanConfirm).not.toHaveBeenCalled();
    expect(window.location.pathname).toContain("/coding/coding_attempt_0001");
  });

  it("shows a scoped linked rejection and allows a successful retry", async () => {
    const repair = repairAwaitingConfirmationFixture();
    mockCodingWs();
    const startLinkedWorkspaceAmendment = vi.fn(() => true);
    mockPlanRepairWs({ startLinkedWorkspaceAmendment });
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      status: "awaiting_plan_amendment",
      stage: "code_review",
      workItemExecutionPlan: executionPlan(),
      activePlanRepair: repair,
      timelineNodes: repair.timelineNodes,
    });
    const linkedStore = useLinkedWorkspaceAmendmentStore.getState();
    linkedStore.reset(repair.childSessionId);
    linkedStore.begin({
      entity_id: "story_spec_0001",
      workspace_type: "story",
      relation: "story_amendment",
    });
    linkedStore.fail("目标 Story 已失效");

    render(
      <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "调整修订范围" }));

    expect(screen.getByRole("alert")).toHaveTextContent("目标 Story 已失效");
    expect(screen.getByRole("button", { name: "发起关联修订" })).toBeEnabled();
    await userEvent.click(screen.getByRole("button", { name: "发起关联修订" }));
    expect(startLinkedWorkspaceAmendment).toHaveBeenCalledWith({
      entity_id: "story_spec_0001",
      workspace_type: "story",
      relation: "story_amendment",
    });

    act(() => {
      useLinkedWorkspaceAmendmentStore.getState().begin({
        entity_id: "story_spec_0001",
        workspace_type: "story",
        relation: "story_amendment",
      });
    });
    expect(screen.getByRole("button", { name: "发起关联修订" })).toBeDisabled();

    act(() => {
      useLinkedWorkspaceAmendmentStore
        .getState()
        .consume(linkedWorkspaceAmendmentSnapshotFixture());
    });
    expect(
      screen.getByRole("link", { name: "打开已创建的 Story Workspace" }),
    ).toBeInTheDocument();
  });

  it("sends exactly one mutation until a matching authoritative response arrives", async () => {
    const repair = repairAwaitingConfirmationFixture();
    mockCodingWs();
    const repairApi = mockPlanRepairWs();
    setChildWorkspaceState(repair.childSessionId);
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      status: "awaiting_plan_amendment",
      stage: "code_review",
      activePlanRepair: repair,
      timelineNodes: repair.timelineNodes,
    });

    render(
      <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />,
    );

    await userEvent.dblClick(
      screen.getByRole("button", { name: "确认修订并恢复执行" }),
    );
    act(() => {
      useCodingWorkspaceStore.setState({
        activePlanRepair: structuredClone(repair),
      });
    });
    expect(screen.getByRole("button", { name: "取消修订" })).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "取消修订" }));
    await userEvent.click(screen.getByRole("button", { name: "要求重新生成" }));

    expect(repairApi.confirmPlanAmendment).toHaveBeenCalledTimes(1);
    expect(repairApi.cancelPlanAmendment).not.toHaveBeenCalled();
    expect(repairApi.sendHumanConfirm).not.toHaveBeenCalled();
    expect(screen.getByText("正在提交 Repair 操作，等待 Child Workspace 响应。"))
      .toBeInTheDocument();

    act(() => {
      useWorkspaceStore.getState().setProtocolError({
        code: "PLAN_AMENDMENT_CONFIRMATION_FAILED",
        message: "amendment conflict",
      });
    });
    expect(screen.getByRole("button", { name: "取消修订" })).toBeEnabled();
  });

  it("does not release an in-flight mutation merely because the socket disconnects", async () => {
    const repair = repairAwaitingConfirmationFixture();
    mockCodingWs();
    const repairApi = mockPlanRepairWs();
    setChildWorkspaceState(repair.childSessionId);
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      status: "awaiting_plan_amendment",
      stage: "code_review",
      activePlanRepair: repair,
      timelineNodes: repair.timelineNodes,
    });
    const view = render(
      <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />,
    );

    await userEvent.click(screen.getByRole("button", { name: "确认修订并恢复执行" }));
    mockPlanRepairWs({ connectionStatus: "disconnected" });
    view.rerender(
      <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />,
    );
    mockPlanRepairWs({ connectionStatus: "connected" });
    view.rerender(
      <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />,
    );

    expect(screen.getByRole("button", { name: "取消修订" })).toBeDisabled();
    expect(repairApi.confirmPlanAmendment).toHaveBeenCalledTimes(1);
  });

  it("surfaces child workspace server errors and clears them after recovered state", async () => {
    const repair = repairAwaitingConfirmationFixture();
    mockCodingWs();
    mockPlanRepairWs();
    setChildWorkspaceState(repair.childSessionId);
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      status: "awaiting_plan_amendment",
      stage: "code_review",
      activePlanRepair: repair,
      timelineNodes: repair.timelineNodes,
    });

    render(
      <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />,
    );

    await userEvent.click(screen.getByRole("button", { name: "确认修订并恢复执行" }));
    act(() => {
      useWorkspaceStore.getState().setProtocolError({
        code: "PLAN_AMENDMENT_CONFIRMATION_FAILED",
        message: "amendment conflict",
      });
    });
    expect(
      screen.getByText("PLAN_AMENDMENT_CONFIRMATION_FAILED: amendment conflict"),
    ).toBeInTheDocument();

    act(() => {
      useWorkspaceStore.getState().setSessionState({
        session_id: repair.childSessionId,
        workspace_type: "work_item",
        stage: "human_confirm",
        messages: [],
        checkpoints: [],
        artifact: null,
        providers: { author: "claude_code", reviewer: "codex" },
        timeline_nodes: [],
        active_node_id: null,
      });
    });
    expect(
      screen.queryByText("PLAN_AMENDMENT_CONFIRMATION_FAILED: amendment conflict"),
    ).not.toBeInTheDocument();

    act(() => {
      useWorkspaceStore.getState().setError("Child Workspace 执行失败");
    });
    expect(screen.getByText("Child Workspace 执行失败")).toBeInTheDocument();

    act(() => {
      useWorkspaceStore.getState().setSessionState({
        session_id: repair.childSessionId,
        workspace_type: "work_item",
        stage: "human_confirm",
        messages: [],
        checkpoints: [],
        artifact: null,
        providers: { author: "claude_code", reviewer: "codex" },
        timeline_nodes: [],
        active_node_id: null,
      });
    });
    expect(screen.queryByText("Child Workspace 执行失败")).not.toBeInTheDocument();
  });

  it("disables mutation actions while disconnected but keeps the workspace link available", () => {
    const repair = repairAwaitingConfirmationFixture();
    mockCodingWs();
    mockPlanRepairWs({ connectionStatus: "disconnected" });
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      status: "awaiting_plan_amendment",
      stage: "code_review",
      activePlanRepair: repair,
      timelineNodes: repair.timelineNodes,
    });

    render(
      <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />,
    );

    for (const name of [
      "确认修订并恢复执行",
      "要求重新生成",
      "调整修订范围",
      "取消修订",
    ]) {
      expect(screen.getByRole("button", { name })).toBeDisabled();
    }
    expect(
      screen.getByRole("link", { name: "在完整 Work Item Workspace 中打开" }),
    ).toHaveAttribute("target", "_blank");
    expect(
      screen.getByText("Child Workspace 正在连接，Repair 操作暂不可用。"),
    ).toBeInTheDocument();
  });

  it("shows a visible error when a child workspace action cannot be sent", async () => {
    const repair = repairAwaitingConfirmationFixture();
    mockCodingWs();
    mockPlanRepairWs({ confirmPlanAmendment: vi.fn(() => false) });
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      status: "awaiting_plan_amendment",
      stage: "code_review",
      activePlanRepair: repair,
      timelineNodes: repair.timelineNodes,
    });

    render(
      <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />,
    );

    await userEvent.click(screen.getByRole("button", { name: "确认修订并恢复执行" }));
    expect(
      screen.getByText("Plan Repair 操作发送失败，请检查 Child Workspace 连接。"),
    ).toBeInTheDocument();
  });

  it("releases an in-flight action when a newer same-stage child snapshot arrives", async () => {
    const repair = repairAwaitingConfirmationFixture();
    mockCodingWs();
    mockPlanRepairWs({ sessionSnapshotGeneration: 1 });
    setChildWorkspaceState(repair.childSessionId);
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      status: "awaiting_plan_amendment",
      stage: "code_review",
      activePlanRepair: repair,
      timelineNodes: repair.timelineNodes,
    });
    const view = render(
      <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />,
    );

    await userEvent.click(screen.getByRole("button", { name: "确认修订并恢复执行" }));
    expect(screen.getByRole("button", { name: "取消修订" })).toBeDisabled();

    mockPlanRepairWs({ sessionSnapshotGeneration: 2 });
    view.rerender(
      <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />,
    );
    expect(screen.getByRole("button", { name: "取消修订" })).toBeEnabled();
  });

  it("clears a local action error when a newer same-stage child snapshot arrives", async () => {
    const repair = repairAwaitingConfirmationFixture();
    mockCodingWs();
    mockPlanRepairWs({
      confirmPlanAmendment: vi.fn(() => false),
      sessionSnapshotGeneration: 1,
    });
    setChildWorkspaceState(repair.childSessionId);
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      status: "awaiting_plan_amendment",
      stage: "code_review",
      activePlanRepair: repair,
      timelineNodes: repair.timelineNodes,
    });
    const view = render(
      <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />,
    );

    await userEvent.click(screen.getByRole("button", { name: "确认修订并恢复执行" }));
    expect(screen.getByText("Plan Repair 操作发送失败，请检查 Child Workspace 连接。"))
      .toBeInTheDocument();

    mockPlanRepairWs({
      confirmPlanAmendment: vi.fn(() => false),
      sessionSnapshotGeneration: 2,
    });
    view.rerender(
      <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />,
    );
    expect(screen.queryByText("Plan Repair 操作发送失败，请检查 Child Workspace 连接。"))
      .not.toBeInTheDocument();
  });

  it("scopes child transport state and old errors to the active repair session", async () => {
    const repairA = repairAwaitingConfirmationFixture();
    const repairB = repairVariant(repairA, "B");
    mockCodingWs();
    const repairApi = mockPlanRepairWs({ connectionStatus: "connecting" });
    setChildWorkspaceState(repairA.childSessionId);
    useWorkspaceStore.getState().setProtocolError({
      code: "PLAN_AMENDMENT_CONFIRMATION_FAILED",
      message: "A conflict",
    });
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      status: "awaiting_plan_amendment",
      stage: "code_review",
      activePlanRepair: repairB,
      timelineNodes: repairB.timelineNodes,
    });

    render(
      <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />,
    );

    expect(screen.queryByText(/A conflict/)).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "确认修订并恢复执行" }))
      .toBeDisabled();
    await userEvent.click(
      screen.getByRole("button", { name: "确认修订并恢复执行" }),
    );
    expect(repairApi.confirmPlanAmendment).not.toHaveBeenCalled();
  });

  it("clears a scoped local action error on a new repair snapshot, switch, or end", async () => {
    const repairA = repairAwaitingConfirmationFixture();
    const repairB = repairVariant(repairA, "B");
    mockCodingWs();
    mockPlanRepairWs({ confirmPlanAmendment: vi.fn(() => false) });
    useCodingWorkspaceStore.setState({
      ...readyCodingState(),
      status: "awaiting_plan_amendment",
      stage: "code_review",
      activePlanRepair: repairA,
      timelineNodes: repairA.timelineNodes,
    });
    render(
      <CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />,
    );

    await userEvent.click(screen.getByRole("button", { name: "确认修订并恢复执行" }));
    expect(screen.getByText("Plan Repair 操作发送失败，请检查 Child Workspace 连接。"))
      .toBeInTheDocument();

    act(() => {
      useCodingWorkspaceStore.setState({
        activePlanRepair: {
          ...repairA,
          request: { ...repairA.request, updated_at: "2026-07-20T00:10:00Z" },
        },
      });
    });
    expect(screen.queryByText("Plan Repair 操作发送失败，请检查 Child Workspace 连接。"))
      .not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "确认修订并恢复执行" }));
    expect(screen.getByText("Plan Repair 操作发送失败，请检查 Child Workspace 连接。"))
      .toBeInTheDocument();

    act(() => {
      useCodingWorkspaceStore.setState({ activePlanRepair: repairB });
    });
    expect(screen.queryByText("Plan Repair 操作发送失败，请检查 Child Workspace 连接。"))
      .not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "确认修订并恢复执行" }));
    expect(screen.getByText("Plan Repair 操作发送失败，请检查 Child Workspace 连接。"))
      .toBeInTheDocument();

    act(() => {
      useCodingWorkspaceStore.setState({ activePlanRepair: null, status: "running" });
    });
    expect(screen.queryByText("Plan Repair 操作发送失败，请检查 Child Workspace 连接。"))
      .not.toBeInTheDocument();
  });
});

function setChildWorkspaceState(sessionId: string) {
  useWorkspaceStore.getState().setSessionState({
    session_id: sessionId,
    workspace_type: "work_item",
    stage: "human_confirm",
    messages: [],
    checkpoints: [],
    artifact: null,
    providers: { author: "claude_code", reviewer: "codex" },
    timeline_nodes: [],
    active_node_id: null,
  });
}

function repairVariant(
  repair: ReturnType<typeof repairAwaitingConfirmationFixture>,
  suffix: string,
) {
  const amendmentId = `plan_amendment_${suffix}`;
  const requestId = `plan_repair_request_${suffix}`;
  return {
    ...repair,
    childSessionId: `workspace_session_repair_${suffix}`,
    request: { ...repair.request, id: requestId, updated_at: `2026-07-20T00:0${suffix.length}:00Z` },
    amendment: repair.amendment
      ? { ...repair.amendment, id: amendmentId, repair_request_id: requestId }
      : null,
    link: {
      ...repair.link,
      id: `workspace_session_link_${suffix}`,
      child_session_id: `workspace_session_repair_${suffix}`,
      trigger: {
        ...repair.link.trigger,
        repair_request_id: requestId,
        amendment_id: amendmentId,
      },
    },
  };
}
