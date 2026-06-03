import { act, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ChatEntry } from "../state/chat-entries";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import { useWorkspaceWs } from "./useWorkspaceWs";

class MockWebSocket {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  static instances: MockWebSocket[] = [];

  readonly sent: string[] = [];
  readonly closeCodes: number[] = [];
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
    this.closeCodes.push(code);
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

  it("stores choice requests from websocket messages", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.receive({
        type: "choice_request",
        id: "choice_001",
        prompt: "请选择下一步",
        options: [
          { id: "continue", label: "继续" },
          { id: "stop", label: "停止" },
        ],
        allow_multiple: false,
        allow_free_text: true,
        source: "ask_user_question",
      });
    });

    expect(useWorkspaceStore.getState().chatEntries.at(-1)).toMatchObject({
      id: "choice_request:choice_001",
      type: "choice_request",
      role: "system",
      content: "请选择下一步",
      metadata: {
        request_id: "choice_001",
        allow_multiple: false,
        allow_free_text: true,
        source: "ask_user_question",
      },
    });
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

  it("uses the concrete command as execution event chat title", () => {
    const harness = renderWorkspaceHook();
    const command =
      "/usr/bin/zsh -lc \"sed -n '1,220p' /home/michael/.codex/superpowers/skills/using-superpowers/SKILL.md\"";

    act(() => {
      harness.ws.receive({
        type: "execution_event",
        event: {
          event_id: "command_cmd_001",
          kind: "command",
          status: "completed",
          title: "Command completed",
          detail: "exit code 0",
          command,
          cwd: "/tmp/repo",
          output: "ok\n",
          exit_code: 0,
        },
      });
    });

    expect(useWorkspaceStore.getState().chatEntries.at(-1)).toMatchObject({
      type: "execution_event",
      content: command,
    });
  });

  it("annotates reviewer stream and tool calls with the reviewer provider", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.receive({
        type: "timeline_node_created",
        node: {
          node_id: "timeline_node_reviewer",
          node_type: "reviewer_run",
          agent: "codex",
          stage: "cross_review",
          round: 1,
          status: "active",
          title: "Review Round 1",
          summary: null,
          started_at: "2026-05-26T10:00:00Z",
          completed_at: null,
          duration_ms: null,
          artifact_ref: "artifact_current",
          provider_config_snapshot: {
            author: "claude_code",
            reviewer: "codex",
            review_rounds: 1,
          },
        },
      });
      harness.ws.receive({
        type: "stream_chunk",
        role: "reviewer",
        content: "reviewing",
        node_id: "timeline_node_reviewer",
      });
      harness.ws.receive({
        type: "execution_event",
        event: {
          event_id: "command_cmd_001",
          node_id: "timeline_node_reviewer",
          agent: "codex",
          kind: "command",
          status: "completed",
          title: "Command completed",
          detail: "exit code 0",
          command: "git diff --stat",
          cwd: "/tmp/repo",
          output: "ok\n",
          exit_code: 0,
        },
      });
    });

    expect(useWorkspaceStore.getState().chatEntries).toEqual([
      expect.objectContaining({
        type: "provider_stream",
        role: "reviewer",
        metadata: expect.objectContaining({ provider: "codex" }),
      }),
      expect.objectContaining({
        type: "execution_event",
        role: "reviewer",
        content: "git diff --stat",
        metadata: expect.objectContaining({ agent: "codex" }),
      }),
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

  it("maps websocket events into chat entries", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.receive({
        type: "session_state",
        session_id: "session_chat",
        workspace_type: "story",
        stage: "prepare_context",
        superpowers_enabled: false,
        openspec_enabled: false,
        messages: [],
        checkpoints: [],
        artifact: null,
        providers: { author: "claude_code", reviewer: "codex" },
        timeline_nodes: [
          {
            node_id: "timeline_node_000",
            node_type: "context_note",
            agent: null,
            stage: "prepare_context",
            round: null,
            status: "completed",
            title: "补充上下文",
            summary: null,
            started_at: "2026-05-21T09:59:00Z",
            completed_at: "2026-05-21T09:59:05Z",
            duration_ms: null,
            artifact_ref: null,
            provider_config_snapshot: {
              author: "claude_code",
              reviewer: "codex",
              review_rounds: 1,
            },
          },
          {
            node_id: "timeline_node_001",
            node_type: "author_run",
            agent: "claude_code",
            stage: "running",
            round: null,
            status: "active",
            title: "Story Spec 生成",
            summary: null,
            started_at: "2026-05-21T10:00:00Z",
            completed_at: null,
            duration_ms: null,
            artifact_ref: null,
            provider_config_snapshot: {
              author: "claude_code",
              reviewer: "codex",
              review_rounds: 1,
            },
          },
        ],
        active_node_id: "timeline_node_001",
        artifact_versions: [],
        timeline_node_details: {
          timeline_node_000: {
            node_id: "timeline_node_000",
            session_id: "session_chat",
            node_type: "context_note",
            status: "completed",
            agent_role: null,
            provider: null,
            prompt: null,
            messages: [],
            streaming_content: "需要支持手机号登录",
            execution_events: [],
            permission_events: [],
            verdict: null,
            artifact_ref: null,
            is_revision: false,
            base_artifact_ref: null,
            started_at: "2026-05-21T09:59:00Z",
            ended_at: "2026-05-21T09:59:05Z",
          },
          timeline_node_001: {
            node_id: "timeline_node_001",
            session_id: "session_chat",
            node_type: "author_run",
            status: "active",
            agent_role: "author",
            provider: { name: "claude_code", model: "claude-opus-4" },
            prompt: null,
            messages: [],
            streaming_content: "",
            execution_events: [],
            permission_events: [],
            verdict: null,
            artifact_ref: null,
            is_revision: false,
            base_artifact_ref: null,
            started_at: "2026-05-21T10:00:00Z",
            ended_at: null,
          },
        },
        active_run_id: "run-001",
      });
      harness.ws.receive({
        type: "provider_locked",
        snapshot: { author: "claude_code", reviewer: "codex", review_rounds: 1 },
        locked_at: "2026-05-21T10:00:00Z",
      });
      harness.ws.receive({ type: "stage_change", stage: "running" });
      harness.ws.receive({
        type: "stream_chunk",
        role: "author",
        content: "第一段",
        node_id: "timeline_node_001",
      });
      harness.ws.receive({
        type: "stream_chunk",
        role: "author",
        content: "第二段",
        node_id: "timeline_node_001",
      });
      harness.ws.receive({
        type: "message_complete",
        message_id: "msg_001",
        checkpoint_id: "checkpoint_001",
        node_id: "timeline_node_001",
      });
      harness.ws.receive({
        type: "execution_event",
        event: {
          event_id: "exec-1",
          node_id: "timeline_node_001",
          agent: "claude_code",
          kind: "command",
          status: "completed",
          title: "读取认证模块",
          detail: "exit code 0",
          command: "sed -n '1,120p' src/auth.rs",
          cwd: "/repo",
          output: null,
          exit_code: 0,
        },
      });
      harness.ws.receive({
        type: "permission_request",
        id: "permission-1",
        tool_name: "shell",
        description: "cargo test",
        risk_level: "medium",
      });
      harness.ws.receive({
        type: "artifact_update",
        version: 1,
        markdown: "# Story",
      });
      harness.ws.receive({
        type: "review_complete",
        node_id: "timeline_node_001",
        round: 1,
        verdict: "pass",
        comments: "审核通过",
        summary: "可以确认",
      });
      harness.ws.receive({ type: "error", message: "阶段不允许" });
    });

    const state = useWorkspaceStore.getState();
    expect(state.chatEntries.map((entry) => entry.type)).toEqual([
      "context_note",
      "start_generation",
      "stage_change",
      "provider_stream",
      "execution_event",
      "permission_request",
      "artifact_update",
      "review_verdict",
      "error",
    ]);
    expect(state.chatEntries[0]).toMatchObject({
      role: "user",
      content: "需要支持手机号登录",
      node_id: "timeline_node_000",
    });
    expect(state.chatEntries[3]).toMatchObject({
      role: "author",
      content: "第一段第二段",
      node_id: "timeline_node_001",
    });
    expect(state.chatEntries[5].metadata).toMatchObject({
      request_id: "permission-1",
      risk_level: "medium",
    });
    expect(state.chatEntries[7]).toMatchObject({
      role: "reviewer",
      content: "可以确认",
    });
    expect(state.activeStreamEntryId).toBeNull();
  });

  it("adds a gate prompt entry when the stage changes to human_confirm", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.receive({
        type: "session_state",
        session_id: "session_chat",
        workspace_type: "story",
        stage: "running",
        superpowers_enabled: false,
        openspec_enabled: false,
        messages: [],
        checkpoints: [],
        artifact: null,
        providers: { author: "claude_code", reviewer: "codex" },
        timeline_nodes: [
          {
            node_id: "timeline_node_human_confirm",
            node_type: "human_confirm",
            agent: null,
            stage: "human_confirm",
            round: null,
            status: "active",
            title: "人工确认",
            summary: "等待人工确认",
            started_at: "2026-05-21T10:03:00Z",
            completed_at: null,
            duration_ms: null,
            artifact_ref: null,
            provider_config_snapshot: {
              author: "claude_code",
              reviewer: "codex",
              review_rounds: 1,
            },
          },
        ],
        active_node_id: "timeline_node_human_confirm",
        artifact_versions: [],
        timeline_node_details: {},
        active_run_id: null,
      });
      harness.ws.receive({ type: "stage_change", stage: "human_confirm" });
    });

    const state = useWorkspaceStore.getState();
    expect(state.chatEntries.at(-1)).toMatchObject({
      type: "gate_prompt",
      role: "system",
      node_id: "timeline_node_human_confirm",
    });
  });

  it("ignores late provider stream chunks after an abort returns to prepare_context", () => {
    const harness = renderWorkspaceHook();

    act(() => {
      harness.ws.receive({
        type: "session_state",
        session_id: "session_abort",
        workspace_type: "story",
        stage: "running",
        superpowers_enabled: false,
        openspec_enabled: false,
        messages: [],
        checkpoints: [],
        artifact: null,
        providers: { author: "claude_code", reviewer: "codex" },
        timeline_nodes: [],
        active_node_id: null,
        artifact_versions: [],
        timeline_node_details: {},
        active_run_id: "run-1",
      });
      harness.ws.receive({ type: "stage_change", stage: "prepare_context" });
      harness.ws.receive({
        type: "stream_chunk",
        role: "author",
        content: "late output",
        node_id: "timeline_node_aborted",
      });
    });

    expect(
      useWorkspaceStore
        .getState()
        .chatEntries.some((entry) => entry.type === "provider_stream"),
    ).toBe(false);
    expect(useWorkspaceStore.getState().streamingContent).toBe("");
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

  it("marks stale choice requests rejected when the server reports an unmatched choice id", () => {
    const harness = renderWorkspaceHook();
    useWorkspaceStore.getState().appendChatEntry({
      id: "choice_request:choice_001",
      type: "choice_request",
      role: "system",
      content: "请选择下一步",
      timestamp: "2026-05-26T10:00:00Z",
      metadata: {
        request_id: "choice_001",
        options: [{ id: "continue", label: "继续" }],
      },
    } as ChatEntry);
    useWorkspaceStore.getState().resolveChoiceRequest("choice_001", ["continue"], null);

    act(() => {
      harness.ws.receive({
        type: "protocol_error",
        code: "CHOICE_ID_UNMATCHED",
        message: "ChoiceResponse id=choice_001 not found in pending",
        context: { choice_id: "choice_001" },
      });
    });

    expect(useWorkspaceStore.getState().chatEntries).toEqual([
      expect.objectContaining({
        id: "choice_request:choice_001",
        resolved: true,
        metadata: expect.objectContaining({
          rejected: true,
          rejection_reason: "ChoiceResponse id=choice_001 not found in pending",
        }),
      }),
      expect.objectContaining({
        type: "error",
        content: "CHOICE_ID_UNMATCHED · ChoiceResponse id=choice_001 not found in pending",
      }),
    ]);
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
    const info = vi.spyOn(console, "info").mockImplementation(() => undefined);
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
    expect(info).toHaveBeenCalledWith("[permission] sending response", {
      id: "perm_001",
      approved: true,
    });
    expect(useWorkspaceStore.getState().pendingPermissions).toHaveLength(0);
  });

  it("sends choice responses and resolves the pending choice when connected", () => {
    const harness = renderWorkspaceHook();
    useWorkspaceStore.getState().appendChatEntry({
      id: "choice_request:choice_001",
      type: "choice_request",
      role: "system",
      content: "请选择下一步",
      timestamp: "2026-05-26T10:00:00Z",
      metadata: {
        request_id: "choice_001",
        options: [{ id: "continue", label: "继续" }],
      },
    } as ChatEntry);

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.sendChoiceResponse("choice_001", ["continue"], null);
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({
        type: "choice_response",
        id: "choice_001",
        selected_option_ids: ["continue"],
        free_text: null,
      }),
    ]);
    expect(useWorkspaceStore.getState().chatEntries.at(-1)).toMatchObject({
      type: "choice_response",
      content: "已选择：继续",
    });
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

  it("syncs provider selection locally after sending it", () => {
    const harness = renderWorkspaceHook();
    useWorkspaceStore.setState({
      providers: { author: "claude_code", reviewer: "codex" },
    });

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.selectProvider("author", "codex");
    });

    expect(harness.ws.sent).toEqual([
      JSON.stringify({ type: "provider_select", role: "author", provider: "codex" }),
    ]);
    expect(useWorkspaceStore.getState().providers).toEqual({
      author: "codex",
      reviewer: "codex",
    });
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

  it("marks the latest gate prompt resolved when a human confirm decision is sent", () => {
    const harness = renderWorkspaceHook();
    useWorkspaceStore.getState().appendChatEntry({
      id: "gate-1",
      type: "gate_prompt",
      role: "system",
      content: "等待人工确认",
      timestamp: "2026-05-21T10:00:00Z",
    });

    act(() => {
      harness.ws.open();
      harness.ws.sent.length = 0;
      harness.api.sendHumanConfirm("terminate");
    });

    expect(useWorkspaceStore.getState().chatEntries).toEqual([
      expect.objectContaining({
        id: "gate-1",
        resolved: true,
        resolution: "terminate",
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
    expect(harness.ws.closeCodes).toContain(4000);
    expect(useWorkspaceStore.getState().connectionStatus).toBe("disconnected");
  });

  it("keeps the socket open during revision even when server messages are quiet", () => {
    vi.useFakeTimers();
    const harness = renderWorkspaceHook("session_001");

    act(() => {
      harness.ws.open();
      harness.ws.receive({ type: "stage_change", stage: "revision" });
    });
    act(() => {
      vi.advanceTimersByTime(75_000);
    });

    expect(harness.ws.readyState).toBe(MockWebSocket.OPEN);
    expect(useWorkspaceStore.getState().connectionStatus).toBe("connected");
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

  it("keeps reconnecting after a replacement websocket errors", () => {
    vi.useFakeTimers();
    vi.spyOn(Math, "random").mockReturnValue(0.5);
    const harness = renderWorkspaceHook("session_001");

    act(() => {
      harness.ws.open();
      harness.ws.close(1006);
    });
    act(() => {
      vi.advanceTimersByTime(1000);
    });

    const replacement = MockWebSocket.instances[1];
    expect(replacement).toBeDefined();
    act(() => {
      replacement.onerror?.(new Event("error"));
    });
    expect(useWorkspaceStore.getState().connectionStatus).toBe("disconnected");

    act(() => {
      vi.advanceTimersByTime(2000);
    });

    expect(MockWebSocket.instances).toHaveLength(3);
  });

  it("keeps reconnecting when a replacement websocket stays connecting", () => {
    vi.useFakeTimers();
    vi.spyOn(Math, "random").mockReturnValue(0.5);
    const harness = renderWorkspaceHook("session_001");

    act(() => {
      harness.ws.open();
      harness.ws.close(1006);
    });
    act(() => {
      vi.advanceTimersByTime(1000);
    });

    expect(MockWebSocket.instances[1].readyState).toBe(MockWebSocket.CONNECTING);
    act(() => {
      vi.advanceTimersByTime(5000);
    });
    expect(useWorkspaceStore.getState().connectionStatus).toBe("disconnected");

    act(() => {
      vi.advanceTimersByTime(2000);
    });
    expect(MockWebSocket.instances).toHaveLength(3);
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
