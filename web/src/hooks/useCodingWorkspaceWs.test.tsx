import { act, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useCodingWorkspaceStore } from "../state/coding-workspace-store";
import { useCodingWorkspaceWs } from "./useCodingWorkspaceWs";

class MockWebSocket {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
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

type CodingWsApi = ReturnType<typeof useCodingWorkspaceWs>;

function renderCodingHook(attemptId = "coding_attempt_0001") {
  let api: CodingWsApi | undefined;

  function Harness() {
    api = useCodingWorkspaceWs(attemptId);
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

describe("useCodingWorkspaceWs", () => {
  beforeEach(() => {
    MockWebSocket.instances = [];
    vi.stubGlobal("WebSocket", MockWebSocket);
    useCodingWorkspaceStore.getState().reset();
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("connects to the coding attempt websocket and sends hello on open", () => {
    const harness = renderCodingHook();

    act(() => {
      harness.ws.open();
    });

    expect(harness.ws.url).toBe("ws://localhost:3000/ws/coding-attempts/coding_attempt_0001");
    expect(harness.ws.sent).toEqual([
      JSON.stringify({
        type: "coding_hello",
        attempt_id: "coding_attempt_0001",
        last_seen_node_id: null,
      }),
    ]);
    expect(useCodingWorkspaceStore.getState().connectionStatus).toBe("connected");
  });

  it("applies coding session state and timeline updates from websocket messages", () => {
    const harness = renderCodingHook();

    act(() => {
      harness.ws.receive({
        type: "coding_session_state",
        attempt_id: "coding_attempt_0001",
        status: "running",
        stage: "coding",
        branch_name: "aria/work-items/work_item_0001/attempt-1",
        base_branch: "main",
        worktree_path: "/tmp/worktree",
        rework_count: 0,
        max_auto_rework: 2,
        head_commit: null,
        pushed_remote: null,
        role_provider_config_snapshot: {
          coder: "fake",
          tester: "fake",
          analyst: "fake",
          code_reviewer: "fake",
          internal_reviewer: "fake",
          review_rounds: 1,
        },
        provider_config_snapshot: { author: "fake", reviewer: "fake", review_rounds: 1 },
        chat_entries: [],
        timeline_nodes: [],
        active_node_id: null,
        testing_report: null,
        code_review_reports: [],
        review_request: null,
        internal_pr_review: null,
        pending_gates: [],
      });
      harness.ws.receive({
        type: "coding_timeline_node_created",
        node: {
          id: "coding_node_0001",
          attempt_id: "coding_attempt_0001",
          stage: "coding",
          title: "代码编写",
          status: "running",
          agent_role: "author",
          summary: null,
          started_at: "2026-05-23T00:00:00Z",
          completed_at: null,
          artifact_refs: [],
        },
      });
      harness.ws.receive({
        type: "coding_timeline_node_updated",
        node_id: "coding_node_0001",
        status: "completed",
        summary: "代码编写完成",
        completed_at: "2026-05-23T00:01:00Z",
      });
      harness.ws.receive({
        type: "coding_provider_config_updated",
        role: "tester",
        provider: "codex",
      });
    });

    const state = useCodingWorkspaceStore.getState();
    expect(state.status).toBe("running");
    expect(state.stage).toBe("coding");
    expect(state.timelineNodes[0]).toMatchObject({
      id: "coding_node_0001",
      status: "completed",
      summary: "代码编写完成",
    });
    expect(state.roleProviderConfigSnapshot?.tester).toBe("codex");
  });

  it("records coding execution events from websocket messages", () => {
    const harness = renderCodingHook();

    act(() => {
      harness.ws.receive({
        type: "coding_execution_event",
        event: {
          event_id: "execution_event_0001",
          node_id: "coding_node_0001",
          agent: "tester",
          kind: "command",
          status: "completed",
          title: "cargo test",
          command: "cargo test --locked",
          output: "test result ok",
          exit_code: 0,
        },
      });
    });

    const state = useCodingWorkspaceStore.getState();
    expect(state.logs).toMatchObject([
      {
        id: "execution_event_0001",
        nodeId: "coding_node_0001",
        message: "test result ok",
      },
    ]);
    expect(state.chatEntries).toMatchObject([
      {
        id: "execution_event_0001",
        type: "execution_event",
        role: "system",
        content: "cargo test",
        node_id: "coding_node_0001",
      },
    ]);
  });

  it("optimistically appends context notes and replaces them with backend chat entries", () => {
    const harness = renderCodingHook();

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.sendContextNote("请覆盖空输入边界");
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({ type: "context_note", content: "请覆盖空输入边界" }),
    ]);
    expect(useCodingWorkspaceStore.getState().chatEntries).toMatchObject([
      {
        type: "context_note",
        role: "user",
        content: "请覆盖空输入边界",
        metadata: { pending: true },
      },
    ]);

    act(() => {
      harness.ws.receive({
        type: "coding_chat_entry_created",
        entry: {
          id: "coding_chat_entry_0001",
          attempt_id: "coding_attempt_0001",
          node_id: null,
          role: "author",
          entry_type: { type: "user_message" },
          content: "请覆盖空输入边界",
          metadata: { context_note_id: "coding_context_note_0001" },
          created_at: "2026-05-28T00:00:01Z",
        },
      });
    });

    expect(useCodingWorkspaceStore.getState().chatEntries).toEqual([
      {
        id: "coding_chat_entry_0001",
        type: "context_note",
        role: "user",
        content: "请覆盖空输入边界",
        timestamp: "2026-05-28T00:00:01Z",
        node_id: undefined,
        metadata: { context_note_id: "coding_context_note_0001" },
      },
    ]);
  });

  it("maps coding tool calls and analyst verdict chat entries to role-specific entries", () => {
    const harness = renderCodingHook();

    act(() => {
      harness.ws.receive({
        type: "coding_chat_entry_created",
        entry: {
          id: "coding_chat_entry_tool_0001",
          attempt_id: "coding_attempt_0001",
          node_id: "coding_node_0002",
          role: "tester",
          entry_type: {
            type: "tool_call",
            tool_name: "run_command",
            input: { command: ["pytest"] },
          },
          content: null,
          metadata: { tool_use_id: "toolu_0001" },
          created_at: "2026-05-28T00:00:02Z",
        },
      });
      harness.ws.receive({
        type: "coding_chat_entry_created",
        entry: {
          id: "coding_chat_entry_analyst_0001",
          attempt_id: "coding_attempt_0001",
          node_id: "coding_node_0003",
          role: "system",
          entry_type: {
            type: "analyst_verdict",
            verdict: "needs_fix",
          },
          content: "测试仍失败",
          metadata: { fix_hints: ["补充 n=10 测试"] },
          created_at: "2026-05-28T00:00:03Z",
        },
      });
    });

    expect(useCodingWorkspaceStore.getState().chatEntries).toMatchObject([
      {
        id: "coding_chat_entry_tool_0001",
        type: "execution_event",
        role: "tester",
        content: "run_command",
        metadata: {
          tool_name: "run_command",
          input: { command: ["pytest"] },
          tool_use_id: "toolu_0001",
        },
      },
      {
        id: "coding_chat_entry_analyst_0001",
        type: "analyst_verdict",
        role: "analyst",
        content: "测试仍失败",
        metadata: {
          verdict: "needs_fix",
          fix_hints: ["补充 n=10 测试"],
        },
      },
    ]);
  });

  it("sends coding client actions when the socket is open", () => {
    const harness = renderCodingHook();

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.startCoding();
      harness.api.sendContextNote("补充上下文");
      harness.api.sendProviderSelect("author", "codex");
      harness.api.sendProviderSelect("tester", "fake");
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
      JSON.stringify({ type: "stage_gate_confirm", stage: "testing" }),
      JSON.stringify({ type: "final_confirm" }),
      JSON.stringify({ type: "abort_attempt" }),
      JSON.stringify({ type: "coding_ping" }),
    ]);
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
});
