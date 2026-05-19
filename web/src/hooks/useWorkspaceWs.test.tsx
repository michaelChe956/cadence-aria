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

  close() {
    this.readyState = MockWebSocket.CLOSED;
    this.onclose?.(new CloseEvent("close"));
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
          node_type: "review",
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
    expect(state.nodeDetails.timeline_node_001.streamingContent).toBe("review output");
    expect(state.nodeDetails.timeline_node_001.verdict?.summary).toBe("可以确认");
  });

  it("sends review decision responses when connected", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.open();
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

  it("sends a default start generation message when connected", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.open();
      harness.api.startGeneration();
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({ type: "user_message", content: "开始生成" }),
    ]);
    expect(useWorkspaceStore.getState().messages.at(-1)?.content).toBe("开始生成");
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
