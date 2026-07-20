import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { repairAwaitingConfirmationFixture } from "../components/coding-workspace/plan-repair-test-fixtures";
import { useCodingWorkspaceStore } from "../state/coding-workspace-store";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import { CodingWorkspacePage } from "./CodingWorkspacePage";
import {
  CODING_ATTEMPT_ADDRESS,
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

  it("sends the four inline repair decisions through the child workspace transport", async () => {
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

    await userEvent.click(screen.getByRole("button", { name: "确认修订并恢复执行" }));
    await userEvent.click(screen.getByRole("button", { name: "要求重新生成" }));
    await userEvent.click(screen.getByRole("button", { name: "调整修订范围" }));
    await userEvent.click(screen.getByRole("button", { name: "取消修订" }));

    expect(repairApi.confirmPlanAmendment).toHaveBeenCalledTimes(1);
    expect(repairApi.confirmPlanAmendment).toHaveBeenCalledWith(repair.amendment?.id);
    expect(repairApi.sendHumanConfirm).toHaveBeenNthCalledWith(
      1,
      "request-change",
      { description: "要求重新生成 Plan Repair 修订" },
    );
    expect(repairApi.sendHumanConfirm).toHaveBeenNthCalledWith(
      2,
      "request-change",
      { description: "调整 Plan Repair 修订范围" },
    );
    expect(repairApi.cancelPlanAmendment).toHaveBeenCalledWith(
      repair.amendment?.id,
      "用户取消修订",
    );
  });

  it("surfaces child workspace server errors and clears them after recovered state", async () => {
    const repair = repairAwaitingConfirmationFixture();
    mockCodingWs();
    mockPlanRepairWs();
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
});
