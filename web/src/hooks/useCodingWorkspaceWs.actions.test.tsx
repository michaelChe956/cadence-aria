import { act } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { useCodingWorkspaceStore } from "../state/coding-workspace-store";
import {
  MockWebSocket,
  blockedGate,
  codingSessionState,
  executionPlan,
  installCodingWorkspaceWsTestHooks,
  renderCodingHook,
} from "./useCodingWorkspaceWs.test-utils";

describe("useCodingWorkspaceWs actions and reconnect", () => {
  installCodingWorkspaceWsTestHooks();

  it("sends coding client actions when the socket is open", () => {
    const harness = renderCodingHook();

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.startCoding();
      harness.api.sendContextNote("补充上下文");
      harness.api.sendProviderSelect("author", "codex");
      harness.api.sendProviderSelect("tester", "fake");
      harness.api.sendPermissionModeSelect("tester", "supervised");
      harness.api.confirmStageGate("testing");
      harness.api.finalConfirm();
      harness.api.abortAttempt();
      harness.api.sendPing();
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({ type: "start_coding" }),
      JSON.stringify({ type: "context_note", content: "补充上下文" }),
      JSON.stringify({ type: "provider_select", role: "author", provider: "codex" }),
      JSON.stringify({ type: "provider_select", role: "tester", provider: "fake" }),
      JSON.stringify({
        type: "permission_mode_select",
        role: "tester",
        permission_mode: "supervised",
      }),
      JSON.stringify({ type: "stage_gate_confirm", stage: "testing" }),
      JSON.stringify({ type: "final_confirm" }),
      JSON.stringify({ type: "abort_attempt" }),
      JSON.stringify({ type: "coding_ping" }),
    ]);
  });

  it("respond gate waits for server snapshot before resolving gate", () => {
    const harness = renderCodingHook();
    useCodingWorkspaceStore.getState().addPendingGate(blockedGate());

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.respondGate("gate_0001", "retry_review");
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({
        type: "gate_response",
        gate_id: "gate_0001",
        action_id: "retry_review",
        extra_context: null,
      }),
    ]);
    expect(useCodingWorkspaceStore.getState().pendingGates).toMatchObject([
      {
        gate_id: "gate_0001",
        submitting: true,
        errorCode: null,
      },
    ]);

    act(() => {
      harness.ws.receive({
        type: "coding_protocol_error",
        code: "coding_gate_response_failed",
        message: "Gate response failed",
      });
    });

    expect(useCodingWorkspaceStore.getState().pendingGates).toMatchObject([
      {
        gate_id: "gate_0001",
        submitting: false,
        errorCode: "coding_gate_response_failed",
      },
    ]);

    act(() => {
      harness.api.respondGate("gate_0001", "retry_review");
      harness.ws.receive(codingSessionState({ pending_gates: [] }));
    });

    expect(useCodingWorkspaceStore.getState().pendingGates).toHaveLength(0);

    act(() => {
      useCodingWorkspaceStore.getState().addPendingGate(
        blockedGate({
          gate_id: "gate_0002",
          available_actions: [
            {
              action_id: "manual_continue",
              label: "人工继续",
              action_type: "manual_continue",
            },
          ],
        }),
      );
      harness.ws.sent.length = 0;
      harness.api.respondGate("gate_0002", "manual_continue", "   ");
    });

    expect(harness.ws.sent).toEqual([]);
    expect(useCodingWorkspaceStore.getState().pendingGates).toMatchObject([
      {
        gate_id: "gate_0002",
        submitting: false,
        errorCode: "coding_gate_extra_context_required",
      },
    ]);

    act(() => {
      harness.api.respondGate("gate_0002", "manual_continue", " operator accepted risk ");
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({
        type: "gate_response",
        gate_id: "gate_0002",
        action_id: "manual_continue",
        extra_context: "operator accepted risk",
      }),
    ]);
    expect(useCodingWorkspaceStore.getState().pendingGates).toMatchObject([
      {
        gate_id: "gate_0002",
        submitting: true,
        errorCode: null,
      },
    ]);
  });

  it("sends continue rework message with trimmed context", () => {
    const harness = renderCodingHook();

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.continueRework("  继续按 analyst findings 返修  ");
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({
        type: "continue_rework",
        extra_context: "继续按 analyst findings 返修",
      }),
    ]);
  });

  it("restores pending coding choices from session snapshots", () => {
    const harness = renderCodingHook();

    act(() => {
      harness.ws.open();
      harness.ws.receive(
        codingSessionState({
          status: "waiting_for_human",
          stage: "coding",
          pending_choices: [
            {
              gate_id: "coding_choice_gate_0001",
              choice_id: "choice_0001",
              attempt_id: "coding_attempt_0001",
              node_id: "coding_node_0001",
              stage: "coding",
              role: "coder",
              provider: "codex",
              source: "request_user_input",
              prompt: "请选择实现范围",
              options: [
                {
                  id: "backend_first",
                  label: "先做后端",
                  description: "TASK-001 到 TASK-009",
                },
              ],
              allow_multiple: false,
              allow_free_text: true,
              status: "open",
              response: null,
              created_at: "2026-06-14T00:00:00Z",
              updated_at: "2026-06-14T00:00:00Z",
            },
          ],
        }),
      );
    });

    expect(useCodingWorkspaceStore.getState().chatEntries).toMatchObject([
      {
        id: "choice_request:choice_0001",
        type: "choice_request",
        role: "coder",
        content: "请选择实现范围",
        resolved: false,
        metadata: {
          request_id: "choice_0001",
          source: "request_user_input",
          allow_free_text: true,
        },
      },
    ]);
  });

  it("waits for coding choice ack before resolving the choice entry", () => {
    const harness = renderCodingHook();

    act(() => {
      harness.ws.open();
      harness.ws.receive({
        type: "coding_choice_request",
        id: "choice_0001",
        prompt: "请选择实现范围",
        source: "request_user_input",
        options: [{ id: "backend_first", label: "先做后端" }],
        allow_multiple: false,
        allow_free_text: true,
      });
      harness.ws.sent.length = 0;
      harness.api.respondChoice("choice_0001", ["backend_first"], "先控制范围");
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({
        type: "choice_response",
        id: "choice_0001",
        selected_option_ids: ["backend_first"],
        free_text: "先控制范围",
      }),
    ]);
    expect(
      useCodingWorkspaceStore
        .getState()
        .chatEntries.find((entry) => entry.id === "choice_request:choice_0001")?.resolved,
    ).not.toBe(true);

    act(() => {
      harness.ws.receive({
        type: "coding_choice_response_ack",
        id: "choice_0001",
        selected_option_ids: ["backend_first"],
        free_text: "先控制范围",
      });
    });

    expect(
      useCodingWorkspaceStore
        .getState()
        .chatEntries.find((entry) => entry.id === "choice_request:choice_0001"),
    ).toMatchObject({
      resolved: true,
      metadata: {
        response: {
          selected_option_ids: ["backend_first"],
          free_text: "先控制范围",
        },
      },
    });
  });

  it("sends heartbeat pings while connected", () => {
    vi.useFakeTimers();
    const harness = renderCodingHook();

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      vi.advanceTimersByTime(25_000);
    });

    expect(harness.ws.sent).toEqual([JSON.stringify({ type: "coding_ping" })]);
    harness.unmount();
    vi.useRealTimers();
  });

  it("reconnects after an unexpected socket close", () => {
    vi.useFakeTimers();
    const harness = renderCodingHook();

    act(() => {
      harness.ws.open();
      harness.ws.close(1006);
    });

    expect(useCodingWorkspaceStore.getState().connectionStatus).toBe("reconnecting");

    act(() => {
      vi.advanceTimersByTime(1_000);
    });

    expect(MockWebSocket.instances).toHaveLength(2);

    act(() => {
      MockWebSocket.instances[1].open();
    });

    expect(useCodingWorkspaceStore.getState().connectionStatus).toBe("connected");
    expect(MockWebSocket.instances[1].sent).toEqual([
      JSON.stringify({
        type: "coding_hello",
        attempt_id: "coding_attempt_0001",
        last_seen_node_id: null,
      }),
    ]);
    harness.unmount();
    vi.useRealTimers();
  });

  it("hydrates work item execution plan from coding session state", () => {
    const harness = renderCodingHook();

    act(() => {
      harness.ws.receive({
        ...codingSessionState(),
        work_item_execution_plan: executionPlan(),
        work_item_handoff: null,
        require_execution_plan_confirm: false,
      });
    });

    expect(useCodingWorkspaceStore.getState().workItemExecutionPlan).toMatchObject({
      status: "draft",
      goal: "实现后端 API",
      allowed_write_scopes: ["src/product/**"],
    });
    expect(useCodingWorkspaceStore.getState().workItemHandoff).toBeNull();
    expect(useCodingWorkspaceStore.getState().requireExecutionPlanConfirm).toBe(false);
  });
});
