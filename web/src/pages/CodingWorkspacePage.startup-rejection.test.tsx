import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MockWebSocket, codingSessionState } from "../hooks/useCodingWorkspaceWs.test-utils";
import { useCodingWorkspaceStore } from "../state/coding-workspace-store";
import { useOperationAuditStore } from "../state/operation-audit-store";
import { CodingWorkspacePage } from "./CodingWorkspacePage";
import { CODING_ATTEMPT_ADDRESS } from "./CodingWorkspacePage.test-utils";

vi.mock("../api/client", () => ({
  confirmWorkItemExecutionPlan: vi.fn(),
  deleteCodingAttempt: vi.fn(),
  getCodingAttemptDiff: vi.fn(),
  requestWorkItemExecutionPlanChange: vi.fn(),
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

describe("CodingWorkspacePage startup rejection wire flow", () => {
  beforeEach(() => {
    MockWebSocket.instances = [];
    vi.stubGlobal("WebSocket", MockWebSocket);
    useCodingWorkspaceStore.getState().reset();
    useOperationAuditStore.getState().reset();
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it.each([
    ["coding_message_not_allowed", "当前阶段不允许开始 Coding"],
    ["SC_CODING_REQUIRES_ADVANCE", "请先在对话侧完成 advance"],
    ["coding_runner_already_started", "Coding runner 已在运行"],
    ["work_item_execution_plan_not_confirmed", "请先确认执行计划"],
  ] as const)(
    "marks %s as rejected and presents its startup feedback on the first wire rejection",
    async (code, copy) => {
      render(<CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />);

      const ws = MockWebSocket.instances[0];
      if (!ws) throw new Error("coding websocket was not created");
      act(() => {
        ws.open();
        ws.receive(codingSessionState({ status: "created", stage: "prepare_context" }));
      });
      ws.sent.length = 0;

      await userEvent.click(screen.getByRole("button", { name: "开始 Coding" }));
      act(() => {
        ws.receive({
          type: "coding_protocol_error",
          code,
          message: "server rejected startup",
        });
      });

      expect(useOperationAuditStore.getState().records.at(-1)).toMatchObject({
        sessionId: CODING_ATTEMPT_ADDRESS.attemptId,
        operation: "start_coding",
        source: "coding",
        outcome: "rejected",
        detail: code,
      });
      expect(screen.getByRole("status")).toHaveTextContent(copy);
      expect(screen.getByRole("button", { name: "开始 Coding" })).toBeDisabled();

      fireEvent.keyDown(document, { code: "KeyA", ctrlKey: true });
      expect(ws.sent).toEqual([JSON.stringify({ type: "start_coding" })]);
    },
  );

  it("terminalizes a successful startup before another command receives the same rejection code", async () => {
    render(<CodingWorkspacePage address={CODING_ATTEMPT_ADDRESS} onBack={vi.fn()} />);

    const ws = MockWebSocket.instances[0];
    if (!ws) throw new Error("coding websocket was not created");
    act(() => {
      ws.open();
      ws.receive(codingSessionState({ status: "created", stage: "prepare_context" }));
    });
    ws.sent.length = 0;

    await userEvent.click(screen.getByRole("button", { name: "开始 Coding" }));
    act(() => {
      ws.receive(codingSessionState({ status: "running", stage: "review_request" }));
    });

    expect(useOperationAuditStore.getState().records.at(-1)).toMatchObject({
      operation: "start_coding",
      outcome: "completed",
      detail: null,
    });

    await userEvent.type(screen.getByLabelText("补充 Coding 上下文"), "补充上下文");
    await userEvent.click(screen.getByRole("button", { name: "发送上下文" }));
    act(() => {
      ws.receive({
        type: "coding_protocol_error",
        code: "coding_message_not_allowed",
        message: "context note rejected",
      });
    });

    expect(screen.queryByRole("status")).toBeNull();
    expect(screen.getByRole("button", { name: "继续 Coding" })).toBeEnabled();
  });
});
