import { act, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import { useWorkspaceWs } from "./useWorkspaceWs";

class MockWebSocket {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  static instances: MockWebSocket[] = [];

  readonly sent: string[] = [];
  readonly url: string;
  readyState = MockWebSocket.CONNECTING;
  onopen: ((event: Event) => void) | null = null;
  onclose: ((event: CloseEvent) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent<string>) => void) | null = null;

  constructor(url: string) {
    this.url = url;
    MockWebSocket.instances.push(this);
  }

  send(data: string) {
    this.sent.push(data);
  }

  close(code = 1000) {
    this.readyState = MockWebSocket.CLOSED;
    this.onclose?.(new CloseEvent("close", { code }));
  }

  open() {
    this.readyState = MockWebSocket.OPEN;
    this.onopen?.(new Event("open"));
  }

  receive(data: unknown) {
    this.onmessage?.(new MessageEvent("message", { data: JSON.stringify(data) }));
  }
}

type WorkspaceWsApi = ReturnType<typeof useWorkspaceWs>;

function renderWorkspaceHook(sessionId = "session_001") {
  let api: WorkspaceWsApi | undefined;

  function Harness() {
    api = useWorkspaceWs(sessionId);
    return null;
  }

  const view = render(<Harness />);
  return {
    ...view,
    get api() {
      if (!api) throw new Error("hook not rendered");
      return api;
    },
    get ws() {
      const ws = MockWebSocket.instances[0];
      if (!ws) throw new Error("websocket not created");
      return ws;
    },
  };
}

describe("useWorkspaceWs", () => {
  beforeEach(() => {
    MockWebSocket.instances = [];
    vi.stubGlobal("WebSocket", MockWebSocket);
    useWorkspaceStore.getState().reset();
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("stores permission requests and provider status from websocket messages", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.receive({
        type: "permission_request",
        id: "perm_001",
        tool_name: "bash",
        description: "Run cargo test",
        risk_level: "medium",
      });
      harness.ws.receive({
        type: "provider_status",
        status: "waiting_approval",
      });
    });

    expect(useWorkspaceStore.getState().pendingPermissions).toEqual([
      {
        id: "perm_001",
        tool_name: "bash",
        description: "Run cargo test",
        risk_level: "medium",
      },
    ]);
    expect(useWorkspaceStore.getState().providerStatus).toBe("waiting_approval");
  });

  it("stores execution events from websocket messages", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.receive({
        type: "execution_event",
        event: {
          event_id: "command_cmd_001",
          kind: "command",
          status: "completed",
          title: "Command completed",
          detail: "exit code 0",
          command: "pwd",
          cwd: "/tmp/repo",
          output: "/tmp/repo\n",
          exit_code: 0,
        },
      });
    });

    expect(useWorkspaceStore.getState().executionEvents).toEqual([
      {
        event_id: "command_cmd_001",
        kind: "command",
        status: "completed",
        title: "Command completed",
        detail: "exit code 0",
        command: "pwd",
        cwd: "/tmp/repo",
        output: "/tmp/repo\n",
        exit_code: 0,
      },
    ]);
  });

  it("stores timeline websocket messages by node", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.receive({
        type: "timeline_node_created",
        node: {
          node_id: "timeline_node_001",
          node_type: "reviewer_run",
          agent: "codex",
          stage: "cross_review",
          round: 1,
          status: "active",
          title: "Review Round 1",
          summary: null,
          started_at: "2026-05-19T00:00:00Z",
          completed_at: null,
          duration_ms: null,
          artifact_ref: "artifact_current",
          provider_config_snapshot: {
            author: "claude_code",
            reviewer: "codex",
            review_rounds: 2,
          },
        },
      });
      harness.ws.receive({
        type: "stream_chunk",
        role: "reviewer",
        content: "review output",
        node_id: "timeline_node_001",
      });
      harness.ws.receive({
        type: "review_complete",
        node_id: "timeline_node_001",
        round: 1,
        verdict: "pass",
        comments: "审核通过",
        summary: "可以确认",
      });
    });

    const state = useWorkspaceStore.getState();
    expect(state.selectedNodeId).toBe("timeline_node_001");
    expect(state.nodeDetails.timeline_node_001.streaming_content).toBe("review output");
    expect(state.nodeDetails.timeline_node_001.verdict?.summary).toBe("可以确认");
  });

  it("stores protocol errors and provider lock events from websocket messages", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.receive({
        type: "protocol_error",
        code: "INVALID_MESSAGE_FOR_STAGE",
        message: "阶段不允许",
      });
      harness.ws.receive({
        type: "provider_locked",
        snapshot: { author: "claude_code", reviewer: "codex", review_rounds: 1 },
        locked_at: "2026-05-20T14:35:00Z",
      });
      harness.ws.receive({ type: "pong" });
    });

    const state = useWorkspaceStore.getState();
    expect(state.protocolError).toEqual({
      code: "INVALID_MESSAGE_FOR_STAGE",
      message: "阶段不允许",
    });
    expect(state.providerLocked).toBe(true);
    expect(state.providerSnapshot).toEqual({
      author: "claude_code",
      reviewer: "codex",
      review_rounds: 1,
    });
  });

  it("sends review decision responses when connected", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.sendReviewDecision("continue_with_context", " 补充边界条件 ");
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({
        type: "review_decision_response",
        decision: "continue_with_context",
        extra_context: "补充边界条件",
      }),
    ]);
  });

  it("sends permission responses and resolves the pending request when connected", () => {
    const harness = renderWorkspaceHook();
    useWorkspaceStore.getState().addPermissionRequest({
      id: "perm_001",
      tool_name: "bash",
      description: "Run cargo test",
      risk_level: "medium",
    });

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.respondPermission("perm_001", true, " approved ");
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({
        type: "permission_response",
        id: "perm_001",
        approved: true,
        reason: "approved",
      }),
    ]);
    expect(useWorkspaceStore.getState().pendingPermissions).toHaveLength(0);
  });

  it("sends context notes when connected", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.sendContextNote("补充上下文");
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({ type: "context_note", content: "补充上下文" }),
    ]);
  });

  it("sends start generation with provider snapshot when connected", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.sendStartGeneration(
        { author: "claude_code", reviewer: "codex", review_rounds: 1 },
        true,
      );
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({
        type: "start_generation",
        provider_config: { author: "claude_code", reviewer: "codex", review_rounds: 1 },
        reviewer_enabled: true,
      }),
    ]);
    expect(useWorkspaceStore.getState().providerStatus).toBe("running");
  });

  it("sends hello and ping lifecycle messages when connected", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.sendHello("session_001", "timeline_node_001");
      harness.api.sendPing();
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({
        type: "hello",
        session_id: "session_001",
        last_seen_node_id: "timeline_node_001",
      }),
      JSON.stringify({ type: "ping" }),
    ]);
  });

  it("sends revision path decisions with trimmed optional context", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.sendSelectRevisionPath("revise-with-context", " 补充边界条件 ");
      harness.api.sendSelectRevisionPath("skip-to-human", "   ");
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({
        type: "select_revision_path",
        path: "revise-with-context",
        extra_context: "补充边界条件",
      }),
      JSON.stringify({
        type: "select_revision_path",
        path: "skip-to-human",
        extra_context: null,
      }),
    ]);
  });

  it("sends human confirm decisions with nullable payload", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.sendHumanConfirm("request-change", { reason: "需要补充" });
      harness.api.sendHumanConfirm("confirm");
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({
        type: "human_confirm",
        decision: "request-change",
        payload: { reason: "需要补充" },
      }),
      JSON.stringify({
        type: "human_confirm",
        decision: "confirm",
        payload: null,
      }),
    ]);
  });

  it("keeps deprecated sendMessage as a context note sender", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.sendMessage("旧入口上下文");
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({ type: "context_note", content: "旧入口上下文" }),
    ]);
    expect(warn).toHaveBeenCalledWith(
      "sendMessage is deprecated, use sendContextNote or sendStartGeneration",
    );
  });

  it("keeps deprecated startGeneration as a warning-only no-op", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.startGeneration();
    });

    expect(harness.ws.sent).toHaveLength(0);
    expect(warn).toHaveBeenCalledWith("startGeneration() without args is deprecated");
  });

  it("sends hello automatically with the last active node when connected", () => {
    useWorkspaceStore.setState({ activeNodeId: "timeline_node_001" });
    const harness = renderWorkspaceHook("session_001");

    act(() => {
      harness.ws.open();
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({
        type: "hello",
        session_id: "session_001",
        last_seen_node_id: "timeline_node_001",
      }),
    ]);
  });

  it("sends ping every 25 seconds while connected", () => {
    vi.useFakeTimers();
    const harness = renderWorkspaceHook("session_001");

    act(() => {
      harness.ws.open();
    });
    act(() => {
      vi.advanceTimersByTime(25_000);
    });

    expect(harness.ws.sent).toContain(JSON.stringify({ type: "ping" }));
  });

  it("closes stale sockets after 60 seconds without any server message", () => {
    vi.useFakeTimers();
    const harness = renderWorkspaceHook("session_001");

    act(() => {
      harness.ws.open();
    });
    act(() => {
      vi.advanceTimersByTime(75_000);
    });

    expect(harness.ws.readyState).toBe(MockWebSocket.CLOSED);
    expect(useWorkspaceStore.getState().connectionStatus).toBe("disconnected");
  });

  it("opens a replacement websocket after an abnormal close", () => {
    vi.useFakeTimers();
    const harness = renderWorkspaceHook("session_001");

    act(() => {
      harness.ws.open();
      harness.ws.close(1006);
    });
    act(() => {
      vi.advanceTimersByTime(1000);
    });

    expect(MockWebSocket.instances).toHaveLength(2);
    expect(MockWebSocket.instances[1].url).toBe(harness.ws.url);
  });

  it("keeps pending permission requests when the socket is not open", () => {
    const harness = renderWorkspaceHook();
    useWorkspaceStore.getState().addPermissionRequest({
      id: "perm_001",
      tool_name: "bash",
      description: "Run cargo test",
      risk_level: "medium",
    });

    act(() => {
      harness.api.respondPermission("perm_001", false, "denied");
    });

    expect(harness.ws.sent).toHaveLength(0);
    expect(useWorkspaceStore.getState().pendingPermissions).toHaveLength(1);
  });
});
