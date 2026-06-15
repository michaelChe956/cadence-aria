import { act, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CodingGateRequired } from "../api/types";
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

function codingSessionState(overrides: Record<string, unknown> = {}) {
  return {
    type: "coding_session_state",
    attempt_id: "coding_attempt_0001",
    status: "running",
    stage: "testing",
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
      permission_modes: {
        coder: "supervised",
        tester: "auto",
        analyst: "auto",
        code_reviewer: "supervised",
        internal_reviewer: "supervised",
      },
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
    pending_choices: [],
    ...overrides,
  };
}

function blockedGate(overrides: Partial<CodingGateRequired> = {}): CodingGateRequired {
  return {
    gate_id: "gate_0001",
    kind: "blocked",
    title: "Review blocked",
    description: "Review payload parse failed",
    stage: "code_review",
    role: "code_reviewer",
    reason_code: "review_payload_parse_error",
    evidence_refs: ["code_review_0001.json"],
    raw_provider_output_ref: "provider-raw/code_review/code_review_0001.txt",
    available_actions: [
      {
        action_id: "retry_review",
        label: "重试审查",
        action_type: "retry_review",
      },
    ],
    ...overrides,
  };
}

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
          permission_modes: {
            coder: "supervised",
            tester: "auto",
            analyst: "auto",
            code_reviewer: "supervised",
            internal_reviewer: "supervised",
          },
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
        pending_choices: [],
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

  it("stores role runs from coding session snapshots", () => {
    const harness = renderCodingHook();

    act(() => {
      harness.ws.receive(
        codingSessionState({
          role_runs: [
            {
              id: "coding_role_run_0001",
              attempt_id: "coding_attempt_0001",
              stage: "testing",
              role: "tester",
              run_no: 1,
              status: "running",
              trigger: "initial",
              node_id: "coding_node_0003",
              started_at: "2026-06-12T00:00:00Z",
              completed_at: null,
              supersedes_run_id: null,
              superseded_by_run_id: null,
              reason_code: null,
              raw_provider_output_refs: [],
              artifact_refs: [],
            },
          ],
        }),
      );
    });

    expect(useCodingWorkspaceStore.getState().roleRuns).toHaveLength(1);
    expect(useCodingWorkspaceStore.getState().roleRuns[0]).toMatchObject({
      id: "coding_role_run_0001",
      role: "tester",
      run_no: 1,
    });
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
        message: "cargo test --locked",
      },
    ]);
    expect(state.chatEntries).toMatchObject([
      {
        id: "execution_event_0001",
        type: "execution_event",
        role: "system",
        content: "cargo test --locked",
        node_id: "coding_node_0001",
      },
    ]);
  });

  it("batches rapid coding stream chunks before updating chat entries", () => {
    vi.useFakeTimers();
    const harness = renderCodingHook();

    act(() => {
      harness.ws.receive({
        type: "coding_session_state",
        attempt_id: "coding_attempt_0001",
        status: "running",
        stage: "testing",
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
          permission_modes: {
            coder: "supervised",
            tester: "auto",
            analyst: "auto",
            code_reviewer: "supervised",
            internal_reviewer: "supervised",
          },
        },
        provider_config_snapshot: { author: "fake", reviewer: "fake", review_rounds: 1 },
        chat_entries: [],
        timeline_nodes: [
          {
            id: "coding_node_0003",
            attempt_id: "coding_attempt_0001",
            stage: "testing",
            title: "测试执行",
            status: "running",
            agent_role: "tester",
            summary: null,
            started_at: "2026-06-14T00:00:00Z",
            completed_at: null,
            artifact_refs: [],
          },
        ],
        active_node_id: "coding_node_0003",
        testing_report: null,
        code_review_reports: [],
        review_request: null,
        internal_pr_review: null,
        pending_gates: [],
        pending_choices: [],
      });
      harness.ws.receive({
        type: "coding_stream_chunk",
        content: "hel",
        node_id: "coding_node_0003",
      });
      harness.ws.receive({
        type: "coding_stream_chunk",
        content: "lo",
        node_id: "coding_node_0003",
      });
    });

    expect(useCodingWorkspaceStore.getState().chatEntries).toHaveLength(0);

    act(() => {
      vi.advanceTimersByTime(49);
    });

    expect(useCodingWorkspaceStore.getState().chatEntries).toHaveLength(0);

    act(() => {
      vi.advanceTimersByTime(1);
    });

    expect(useCodingWorkspaceStore.getState().chatEntries).toMatchObject([
      {
        type: "provider_stream",
        role: "tester",
        content: "hello",
        node_id: "coding_node_0003",
      },
    ]);

    harness.unmount();
    vi.useRealTimers();
  });

  it("ignores late provider output after a coding attempt is aborted", () => {
    const harness = renderCodingHook();

    act(() => {
      harness.ws.receive({
        type: "coding_session_state",
        attempt_id: "coding_attempt_0001",
        status: "aborted",
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
          permission_modes: {
            coder: "supervised",
            tester: "auto",
            analyst: "auto",
            code_reviewer: "supervised",
            internal_reviewer: "supervised",
          },
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
        pending_choices: [],
      });
      harness.ws.receive({
        type: "coding_stream_chunk",
        content: "late output",
        node_id: "coding_node_0001",
      });
      harness.ws.receive({
        type: "coding_execution_event",
        event: {
          event_id: "late_event",
          node_id: "coding_node_0001",
          agent: "codex",
          kind: "command",
          status: "completed",
          title: "late command",
          command: "git status",
          output: "late",
          exit_code: 0,
        },
      });
    });

    const state = useCodingWorkspaceStore.getState();
    expect(state.chatEntries).toHaveLength(0);
    expect(state.logs).toHaveLength(0);
  });

  it("records coding permission and choice requests and sends responses", () => {
    const harness = renderCodingHook();

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.ws.receive({
        type: "coding_permission_request",
        id: "permission_0001",
        tool_name: "shell",
        description: "Run uv test command",
        risk_level: "high",
      });
      harness.ws.receive({
        type: "coding_choice_request",
        id: "choice_0001",
        prompt: "Select implementation strategy",
        source: "provider_choice",
        options: [{ id: "dp", label: "Dynamic programming", description: "Iterative" }],
        allow_multiple: false,
        allow_free_text: true,
      });
      harness.api.respondPermission("permission_0001", true);
      harness.api.respondChoice("choice_0001", ["dp"], "use iterative dp");
      harness.ws.receive({
        type: "coding_choice_response_ack",
        id: "choice_0001",
        selected_option_ids: ["dp"],
        free_text: "use iterative dp",
      });
    });

    expect(useCodingWorkspaceStore.getState().chatEntries).toMatchObject([
      {
        id: "permission_request:permission_0001",
        type: "permission_request",
        role: "system",
        content: "shell · Run uv test command",
        metadata: {
          request_id: "permission_0001",
          tool_name: "shell",
          description: "Run uv test command",
          risk_level: "high",
          approved: true,
        },
        resolved: true,
      },
      {
        id: "choice_request:choice_0001",
        type: "choice_request",
        role: "system",
        content: "Select implementation strategy",
        metadata: {
          request_id: "choice_0001",
          prompt: "Select implementation strategy",
          source: "provider_choice",
          options: [{ id: "dp", label: "Dynamic programming", description: "Iterative" }],
          allow_multiple: false,
          allow_free_text: true,
          response: {
            selected_option_ids: ["dp"],
            free_text: "use iterative dp",
          },
        },
        resolved: true,
      },
    ]);
    expect(harness.ws.sent).toEqual([
      JSON.stringify({
        type: "permission_response",
        id: "permission_0001",
        approved: true,
        reason: null,
      }),
      JSON.stringify({
        type: "choice_response",
        id: "choice_0001",
        selected_option_ids: ["dp"],
        free_text: "use iterative dp",
      }),
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
});
