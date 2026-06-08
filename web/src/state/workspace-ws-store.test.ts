import { beforeEach, describe, expect, it } from "vitest";
import type { NodeDetail } from "../api/types";
import type { ChatEntry } from "./chat-entries";
import {
  emptyWorkspaceContentCache,
  workspaceContentCacheValues,
} from "./workspace-content-cache";
import { selectPrepareContextNotes, useWorkspaceStore } from "./workspace-ws-store";

function makeNodeDetail(overrides: Partial<NodeDetail> = {}): NodeDetail {
  return {
    node_id: "timeline_node_001",
    session_id: "session_001",
    node_type: "author_run",
    status: "completed",
    agent_role: "author",
    provider: { name: "claude_code", model: "claude-opus-4" },
    messages: [],
    streaming_content: "",
    execution_events: [],
    permission_events: [],
    verdict: null,
    artifact_ref: null,
    is_revision: false,
    base_artifact_ref: null,
    started_at: "2026-05-20T14:30:00Z",
    ended_at: null,
    ...overrides,
  };
}

describe("workspace ws store", () => {
  beforeEach(() => {
    useWorkspaceStore.getState().reset();
  });

  it("clears partial streaming content when an active run is aborted", () => {
    const store = useWorkspaceStore.getState();
    store.appendStreamChunk("partial output");

    store.setStage("prepare_context");

    expect(useWorkspaceStore.getState().streamingContent).toBe("");
  });

  it("keeps streaming content while the stage remains running", () => {
    const store = useWorkspaceStore.getState();
    store.appendStreamChunk("partial output");

    store.setStage("running");

    expect(useWorkspaceStore.getState().streamingContent).toBe("partial output");
  });

  it("tracks stages visited by fast websocket transitions", () => {
    const store = useWorkspaceStore.getState();

    store.setStage("running");
    store.setStage("cross_review");
    store.setStage("human_confirm");

    expect(useWorkspaceStore.getState().visitedStages).toEqual([
      "prepare_context",
      "running",
      "author_confirm",
      "cross_review",
      "human_confirm",
    ]);
  });

  it("maps review decision and revision stages onto the cross review rail step", () => {
    const store = useWorkspaceStore.getState();

    store.setStage("running");
    store.setStage("cross_review");
    store.setStage("review_decision");
    store.setStage("revision");

    expect(useWorkspaceStore.getState().visitedStages).toEqual([
      "prepare_context",
      "running",
      "author_confirm",
      "cross_review",
    ]);
  });

  it("tracks and resolves pending permission requests", () => {
    const store = useWorkspaceStore.getState();
    store.addPermissionRequest({
      id: "perm_001",
      tool_name: "bash",
      description: "Run cargo test",
      risk_level: "medium",
    });

    expect(useWorkspaceStore.getState().pendingPermissions).toHaveLength(1);

    store.resolvePermissionRequest("perm_001");

    expect(useWorkspaceStore.getState().pendingPermissions).toHaveLength(0);
  });

  it("marks permission request entries resolved when a response is sent", () => {
    const store = useWorkspaceStore.getState();
    store.appendChatEntry({
      id: "permission-request-1",
      type: "permission_request",
      role: "system",
      content: "shell · cargo test",
      timestamp: "2026-05-26T10:00:00Z",
      metadata: { request_id: "perm_001" },
    });

    store.resolvePermissionRequest("perm_001", true);

    expect(useWorkspaceStore.getState().chatEntries).toEqual([
      expect.objectContaining({
        id: "permission-request-1",
        resolved: true,
        metadata: expect.objectContaining({ approved: true }),
      }),
      expect.objectContaining({
        type: "permission_response",
        role: "user",
        content: "已允许",
      }),
    ]);
  });

  it("marks choice request entries resolved and appends a choice response entry", () => {
    const store = useWorkspaceStore.getState();
    store.appendChatEntry({
      id: "choice-request-1",
      type: "choice_request",
      role: "system",
      content: "请选择下一步",
      timestamp: "2026-05-26T10:00:00Z",
      metadata: {
        request_id: "choice_001",
        options: [
          { id: "continue", label: "继续" },
          { id: "stop", label: "停止" },
        ],
      },
    } as ChatEntry);

    store.resolveChoiceRequest("choice_001", ["continue"], null);

    expect(useWorkspaceStore.getState().chatEntries).toEqual([
      expect.objectContaining({
        id: "choice-request-1",
        resolved: true,
        metadata: expect.objectContaining({
          response: { selected_option_ids: ["continue"], free_text: null },
        }),
      }),
      expect.objectContaining({
        type: "choice_response",
        role: "user",
        content: "已选择：继续",
      }),
    ]);
  });

  it("rejects stale choice requests and removes optimistic choice responses", () => {
    const store = useWorkspaceStore.getState();
    store.appendChatEntry({
      id: "choice-request-1",
      type: "choice_request",
      role: "system",
      content: "请选择下一步",
      timestamp: "2026-05-26T10:00:00Z",
      metadata: {
        request_id: "choice_001",
        options: [{ id: "continue", label: "继续" }],
      },
    } as ChatEntry);
    store.resolveChoiceRequest("choice_001", ["continue"], null);

    store.rejectChoiceRequest("choice_001", "ChoiceResponse id=choice_001 not found in pending");

    expect(useWorkspaceStore.getState().chatEntries).toEqual([
      expect.objectContaining({
        id: "choice-request-1",
        resolved: true,
        metadata: expect.objectContaining({
          rejected: true,
          rejection_reason: "ChoiceResponse id=choice_001 not found in pending",
        }),
      }),
    ]);
  });

  it("deduplicates pending permission requests by id", () => {
    const store = useWorkspaceStore.getState();

    store.addPermissionRequest({
      id: "perm_001",
      tool_name: "bash",
      description: "Run cargo test",
      risk_level: "medium",
    });
    store.addPermissionRequest({
      id: "perm_001",
      tool_name: "bash",
      description: "Run cargo clippy",
      risk_level: "high",
    });

    expect(useWorkspaceStore.getState().pendingPermissions).toEqual([
      {
        id: "perm_001",
        tool_name: "bash",
        description: "Run cargo clippy",
        risk_level: "high",
      },
    ]);
  });

  it("updates provider status independently from workspace stage", () => {
    const store = useWorkspaceStore.getState();

    store.setProviderStatus("waiting_approval");

    expect(useWorkspaceStore.getState().providerStatus).toBe("waiting_approval");
    expect(useWorkspaceStore.getState().stage).toBe("prepare_context");
  });

  it("evicts content cache entries by byte budget", () => {
    const store = useWorkspaceStore.getState();
    useWorkspaceStore.setState({
      contentCache: emptyWorkspaceContentCache(6),
    });

    store.setContentCacheEntry("a", "aaa", 1);
    store.setContentCacheEntry("b", "bbb", 2);
    store.touchContentCacheEntry("a", 3);
    store.setContentCacheEntry("c", "ccc", 4);

    expect(workspaceContentCacheValues(useWorkspaceStore.getState().contentCache)).toEqual({
      a: "aaa",
      c: "ccc",
    });
  });

  it("merges hydrated node detail and rebuilds chat entries", () => {
    const store = useWorkspaceStore.getState();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      timelineNodes: [
        {
          node_id: "node-1",
          node_type: "reviewer_run",
          agent: "codex",
          stage: "cross_review",
          round: 1,
          status: "completed",
          title: "Review Round 1",
          summary: "仅有可选建议",
          started_at: "2026-05-20T00:00:00Z",
          completed_at: "2026-05-20T00:01:00Z",
          duration_ms: 60_000,
          artifact_ref: null,
          provider_config_snapshot: {
            author: "claude_code",
            reviewer: "codex",
            review_rounds: 1,
          },
        },
      ],
      nodeDetails: {
        "node-1": makeNodeDetail({
          node_id: "node-1",
          node_type: "reviewer_run",
          streaming_content: "summary only",
        }),
      },
    });

    store.setNodeDetail(
      makeNodeDetail({
        node_id: "node-1",
        node_type: "reviewer_run",
        streaming_content: "complete review output",
        verdict: {
          verdict: "needs_human",
          comments: "完整 comments",
          summary: "仅有可选建议",
          findings: [],
          review_gate: "user_confirm_allowed",
        },
      }),
    );

    expect(useWorkspaceStore.getState().nodeDetails["node-1"].streaming_content).toBe(
      "complete review output",
    );
    expect(
      useWorkspaceStore
        .getState()
        .chatEntries.some((entry) => entry.content.includes("complete review output")),
    ).toBe(true);
  });

  it("upserts execution events by id so command completion replaces running state", () => {
    const store = useWorkspaceStore.getState();

    store.upsertExecutionEvent({
      event_id: "command_cmd_001",
      kind: "command",
      status: "started",
      title: "Command started",
      detail: null,
      command: "pwd",
      cwd: "/tmp/repo",
      output: null,
      exit_code: null,
    });
    store.upsertExecutionEvent({
      event_id: "command_cmd_001",
      kind: "command",
      status: "completed",
      title: "Command completed",
      detail: "exit code 0",
      command: "pwd",
      cwd: "/tmp/repo",
      output: "/tmp/repo\n",
      exit_code: 0,
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

  it("clears permission state when a session snapshot is applied", () => {
    const store = useWorkspaceStore.getState();
    store.addPermissionRequest({
      id: "perm_001",
      tool_name: "bash",
      description: "Run cargo test",
      risk_level: "medium",
    });
    store.setProviderStatus("waiting_approval");
    store.upsertExecutionEvent({
      event_id: "command_cmd_001",
      kind: "command",
      status: "started",
      title: "Command started",
      detail: null,
      command: "pwd",
      cwd: "/tmp/repo",
      output: null,
      exit_code: null,
    });

    store.setSessionState({
      session_id: "session_002",
      workspace_type: "documentation",
      stage: "human_confirm",
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "fake", reviewer: null },
    });

    expect(useWorkspaceStore.getState().pendingPermissions).toHaveLength(0);
    expect(useWorkspaceStore.getState().providerStatus).toBe("starting");
    expect(useWorkspaceStore.getState().executionEvents).toHaveLength(0);
    expect(useWorkspaceStore.getState().visitedStages).toEqual([
      "prepare_context",
      "running",
      "author_confirm",
      "cross_review",
      "human_confirm",
    ]);
  });

  it("initializes timeline state from a session snapshot", () => {
    const store = useWorkspaceStore.getState();

    store.setSessionState({
      session_id: "session_003",
      workspace_type: "story",
      stage: "cross_review",
      messages: [],
      checkpoints: [],
      artifact: "# Story",
      providers: { author: "claude_code", reviewer: "codex" },
      timeline_nodes: [
        {
          node_id: "timeline_node_001",
          node_type: "author_run",
          agent: "claude_code",
          stage: "running",
          round: null,
          status: "completed",
          title: "Story Spec 生成",
          summary: "生成完成",
          started_at: "2026-05-19T00:00:00Z",
          completed_at: "2026-05-19T00:00:01Z",
          duration_ms: null,
          artifact_ref: "artifact_current",
          provider_config_snapshot: {
            author: "claude_code",
            reviewer: "codex",
            review_rounds: 2,
          },
        },
      ],
      active_node_id: "timeline_node_001",
      artifact_versions: [],
    });

    expect(useWorkspaceStore.getState().timelineNodes).toHaveLength(1);
    expect(useWorkspaceStore.getState().activeNodeId).toBe("timeline_node_001");
    expect(useWorkspaceStore.getState().selectedNodeId).toBe("timeline_node_001");
  });

  it("uses artifact version summaries from session snapshots without requiring markdown", () => {
    const store = useWorkspaceStore.getState();

    store.setSessionState({
      session_id: "session_artifact_summaries",
      workspace_type: "story",
      stage: "completed",
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude_code", reviewer: "codex" },
      timeline_nodes: [],
      active_node_id: null,
      artifact_versions: [],
      artifact_version_summaries: [
        {
          version: 1,
          generated_by: "claude_code",
          reviewed_by: null,
          review_verdict: null,
          confirmed_by: null,
          is_current: true,
          created_at: "2026-05-26T10:01:00Z",
          source_node_id: "timeline_node_001",
        },
      ],
    });

    expect(useWorkspaceStore.getState().artifactVersions).toEqual([
      expect.objectContaining({ version: 1, source_node_id: "timeline_node_001" }),
    ]);
    expect("markdown" in useWorkspaceStore.getState().artifactVersions[0]).toBe(false);
  });

  it("preserves a valid selected timeline node when a later snapshot arrives", () => {
    const store = useWorkspaceStore.getState();
    const authorNode = {
      node_id: "timeline_node_002",
      node_type: "author_run" as const,
      agent: "fake" as const,
      stage: "running",
      round: null,
      status: "failed" as const,
      title: "Story Spec 生成",
      summary: "连接断开，运行已中止",
      started_at: "2026-05-20T14:30:00Z",
      completed_at: "2026-05-20T14:30:01Z",
      duration_ms: null,
      artifact_ref: null,
      provider_config_snapshot: {
        author: "fake" as const,
        reviewer: "fake" as const,
        review_rounds: 1,
      },
    };
    const abortedNode = {
      node_id: "timeline_node_003",
      node_type: "aborted_by_disconnect" as const,
      agent: null,
      stage: "prepare_context",
      round: null,
      status: "failed" as const,
      title: "运行因断开中止",
      summary: "last_active_run_id: run-1",
      started_at: "2026-05-20T14:30:02Z",
      completed_at: "2026-05-20T14:30:02Z",
      duration_ms: 0,
      artifact_ref: null,
      provider_config_snapshot: {
        author: "fake" as const,
        reviewer: "fake" as const,
        review_rounds: 1,
      },
    };

    store.setSessionState({
      session_id: "session_keep_selection",
      workspace_type: "story",
      stage: "prepare_context",
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "fake", reviewer: "fake" },
      timeline_nodes: [authorNode, abortedNode],
      active_node_id: "timeline_node_003",
      artifact_versions: [],
      timeline_node_details: {
        timeline_node_002: makeNodeDetail({
          node_id: "timeline_node_002",
          streaming_content: "E2E permission fixture stream\n",
        }),
      },
      active_run_id: null,
    });
    store.setSelectedNode("timeline_node_002");

    store.setSessionState({
      session_id: "session_keep_selection",
      workspace_type: "story",
      stage: "prepare_context",
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "fake", reviewer: "fake" },
      timeline_nodes: [authorNode, abortedNode],
      active_node_id: "timeline_node_003",
      artifact_versions: [],
      timeline_node_details: {
        timeline_node_002: makeNodeDetail({
          node_id: "timeline_node_002",
          streaming_content: "E2E permission fixture stream\n",
        }),
      },
      active_run_id: null,
    });

    expect(useWorkspaceStore.getState().selectedNodeId).toBe("timeline_node_002");
  });

  it("applies timeline node details and active run id from a session snapshot", () => {
    const store = useWorkspaceStore.getState();
    const detail = makeNodeDetail({
      node_id: "timeline_node_001",
      streaming_content: "输出内容",
    });

    store.setSessionState({
      session_id: "session_004",
      workspace_type: "story",
      stage: "running",
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude_code", reviewer: null },
      timeline_nodes: [
        {
          node_id: "timeline_node_001",
          node_type: "author_run",
          agent: "claude_code",
          stage: "running",
          round: null,
          status: "active",
          title: "Story Spec 生成",
          summary: null,
          started_at: "2026-05-20T14:30:00Z",
          completed_at: null,
          duration_ms: null,
          artifact_ref: null,
          provider_config_snapshot: {
            author: "claude_code",
            reviewer: null,
            review_rounds: 0,
          },
        },
      ],
      active_node_id: "timeline_node_001",
      artifact_versions: [],
      timeline_node_details: {
        timeline_node_001: detail,
      },
      active_run_id: "run-1",
    });

    const state = useWorkspaceStore.getState();
    expect(state.nodeDetails.timeline_node_001.streaming_content).toBe("输出内容");
    expect(state.activeRunId).toBe("run-1");
  });

  it("replaces stale node details and clears stale active run id from snapshots", () => {
    const store = useWorkspaceStore.getState();
    store.setSessionState({
      session_id: "session_005",
      workspace_type: "story",
      stage: "running",
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude_code", reviewer: null },
      timeline_nodes: [],
      active_node_id: null,
      artifact_versions: [],
      timeline_node_details: {
        stale_node: makeNodeDetail({ node_id: "stale_node", streaming_content: "旧输出" }),
      },
      active_run_id: "run-stale",
    });

    store.setSessionState({
      session_id: "session_005",
      workspace_type: "story",
      stage: "prepare_context",
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude_code", reviewer: null },
      timeline_nodes: [],
      active_node_id: null,
      artifact_versions: [],
      timeline_node_details: {},
      active_run_id: null,
    });

    const state = useWorkspaceStore.getState();
    expect(state.nodeDetails.stale_node).toBeUndefined();
    expect(state.activeRunId).toBeNull();
  });

  it("selectNodeDetail returns the requested snapshot detail", () => {
    const store = useWorkspaceStore.getState();
    const detail = makeNodeDetail({
      node_id: "timeline_node_006",
      streaming_content: "selector 输出",
    });

    store.setSessionState({
      session_id: "session_006",
      workspace_type: "story",
      stage: "running",
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude_code", reviewer: null },
      timeline_nodes: [],
      active_node_id: null,
      artifact_versions: [],
      timeline_node_details: {
        timeline_node_006: detail,
      },
      active_run_id: null,
    });

    expect(store.selectNodeDetail("timeline_node_006")?.streaming_content).toBe("selector 输出");
    expect(store.selectNodeDetail("missing")).toBeNull();
  });

  it("derives context notes from timeline node details", () => {
    const store = useWorkspaceStore.getState();

    store.setSessionState({
      session_id: "session_context_notes",
      workspace_type: "story",
      stage: "prepare_context",
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude_code", reviewer: "codex" },
      timeline_nodes: [
        {
          node_id: "note-1",
          node_type: "context_note",
          agent: null,
          stage: "prepare_context",
          round: null,
          status: "completed",
          title: "补充上下文",
          summary: null,
          started_at: "2026-05-20T00:00:00Z",
          completed_at: null,
          duration_ms: null,
          artifact_ref: null,
          provider_config_snapshot: {
            author: "claude_code",
            reviewer: "codex",
            review_rounds: 1,
          },
        },
        {
          node_id: "note-2",
          node_type: "context_note",
          agent: null,
          stage: "prepare_context",
          round: null,
          status: "completed",
          title: "补充上下文",
          summary: "第二条 fallback",
          started_at: "2026-05-20T00:00:01Z",
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
      active_node_id: null,
      artifact_versions: [],
      timeline_node_details: {
        "note-1": makeNodeDetail({
          node_id: "note-1",
          node_type: "context_note",
          agent_role: null,
          provider: null,
          streaming_content: "第一条",
        }),
      },
      active_run_id: null,
    });

    expect(selectPrepareContextNotes(useWorkspaceStore.getState())).toEqual([
      "第一条",
      "第二条 fallback",
    ]);
  });

  it("does not show a context note before backend acknowledgement", () => {
    expect(selectPrepareContextNotes(useWorkspaceStore.getState())).toEqual([]);
  });

  it("groups stream chunks and execution events by timeline node", () => {
    const store = useWorkspaceStore.getState();
    store.addTimelineNode({
      node_id: "timeline_node_002",
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
    });

    store.appendStreamChunk("review output", "timeline_node_002");
    store.upsertExecutionEvent({
      event_id: "turn_001",
      node_id: "timeline_node_002",
      agent: "codex",
      kind: "turn",
      status: "completed",
      title: "Review turn",
      detail: null,
      command: null,
      cwd: null,
      output: null,
      exit_code: null,
    });
    store.completeMessage("msg_002", "checkpoint_002", "timeline_node_002");

    const detail = useWorkspaceStore.getState().nodeDetails.timeline_node_002;
    expect(detail.streaming_content).toBe("");
    expect(detail.messages).toEqual([
      expect.objectContaining({
        id: "msg_002",
        role: "assistant",
        content: "review output",
      }),
    ]);
    expect(detail.execution_events).toHaveLength(1);
    expect(useWorkspaceStore.getState().streamingContent).toBe("");
  });

  it("rebuilds reviewer command events with command titles and provider metadata", () => {
    const store = useWorkspaceStore.getState();

    store.setSessionState({
      session_id: "session_reviewer_tools",
      workspace_type: "story",
      stage: "cross_review",
      messages: [],
      checkpoints: [],
      artifact: "# Draft",
      providers: { author: "claude_code", reviewer: "codex" },
      timeline_nodes: [
        {
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
      ],
      active_node_id: "timeline_node_reviewer",
      artifact_versions: [],
      timeline_node_details: {
        timeline_node_reviewer: makeNodeDetail({
          node_id: "timeline_node_reviewer",
          node_type: "reviewer_run",
          agent_role: "reviewer",
          provider: { name: "codex", model: "gpt-5" },
          streaming_content: "reviewing diff",
          execution_events: [
            {
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
          ],
        }),
      },
      active_run_id: "run-1",
    });
    store.rebuildChatEntries();

    expect(useWorkspaceStore.getState().chatEntries).toEqual([
      expect.objectContaining({
        type: "provider_stream",
        role: "reviewer",
        content: "reviewing diff",
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

  it.each([
    ["story", "Story Spec"],
    ["design", "Design Spec"],
    ["work_item", "Work Item"],
  ])("rebuilds revision nodes as author chat entries for %s workspaces", (workspaceType, label) => {
    const store = useWorkspaceStore.getState();
    store.reset();
    store.setSessionState({
      session_id: `session_revision_${workspaceType}`,
      workspace_type: workspaceType,
      stage: "revision",
      messages: [],
      checkpoints: [],
      artifact: "# Draft",
      providers: { author: "claude_code", reviewer: "codex" },
      timeline_nodes: [
        {
          node_id: "timeline_node_revision",
          node_type: "revision",
          agent: "claude_code",
          stage: "revision",
          round: 1,
          status: "completed",
          title: "返修 Round 1",
          summary: "生成完成",
          started_at: "2026-05-26T10:00:00Z",
          completed_at: "2026-05-26T10:01:00Z",
          duration_ms: 60_000,
          artifact_ref: "artifact_revision",
          provider_config_snapshot: {
            author: "claude_code",
            reviewer: "codex",
            review_rounds: 1,
          },
        },
      ],
      active_node_id: null,
      artifact_versions: [],
      timeline_node_details: {
        timeline_node_revision: makeNodeDetail({
          node_id: "timeline_node_revision",
          node_type: "revision",
          agent_role: null,
          provider: { name: "claude_code", model: "claude-opus-4" },
          streaming_content: `已按 reviewer 意见返修 ${label}`,
          is_revision: true,
        }),
      },
      active_run_id: null,
    });
    store.rebuildChatEntries();

    expect(useWorkspaceStore.getState().chatEntries).toEqual([
      expect.objectContaining({
        type: "provider_stream",
        role: "author",
        content: `已按 reviewer 意见返修 ${label}`,
        metadata: expect.objectContaining({ provider: "claude_code" }),
      }),
    ]);
  });

  it("deduplicates snapshot execution events before rebuilding chat entries", () => {
    const store = useWorkspaceStore.getState();

    store.setSessionState({
      session_id: "session_duplicate_events",
      workspace_type: "story",
      stage: "cross_review",
      messages: [],
      checkpoints: [],
      artifact: "# Draft",
      providers: { author: "claude_code", reviewer: "codex" },
      timeline_nodes: [
        {
          node_id: "timeline_node_reviewer",
          node_type: "reviewer_run",
          agent: "codex",
          stage: "cross_review",
          round: 1,
          status: "completed",
          title: "Review Round 1",
          summary: "需要返修",
          started_at: "2026-05-26T10:00:00Z",
          completed_at: "2026-05-26T10:01:00Z",
          duration_ms: 60_000,
          artifact_ref: "artifact_current",
          provider_config_snapshot: {
            author: "claude_code",
            reviewer: "codex",
            review_rounds: 1,
          },
        },
      ],
      active_node_id: null,
      artifact_versions: [],
      timeline_node_details: {
        timeline_node_reviewer: makeNodeDetail({
          node_id: "timeline_node_reviewer",
          node_type: "reviewer_run",
          agent_role: "reviewer",
          provider: { name: "codex", model: "gpt-5" },
          execution_events: [
            {
              event_id: "command_cmd_001",
              node_id: "timeline_node_reviewer",
              agent: "codex",
              kind: "command",
              status: "started",
              title: "Command started",
              detail: null,
              command: "git diff --stat",
              cwd: "/tmp/repo",
              output: null,
              exit_code: null,
            },
            {
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
          ],
        }),
      },
      active_run_id: null,
    });
    store.rebuildChatEntries();

    expect(
      useWorkspaceStore.getState().nodeDetails.timeline_node_reviewer.execution_events,
    ).toEqual([
      expect.objectContaining({
        event_id: "command_cmd_001",
        status: "completed",
        output: "ok\n",
      }),
    ]);
    expect(
      useWorkspaceStore
        .getState()
        .chatEntries.filter(
          (entry) => entry.id === "timeline_node_reviewer:execution-command_cmd_001",
        ),
    ).toHaveLength(1);
  });

  it.each(["story", "design", "work_item"])(
    "rebuilds system timeline nodes as chat anchors for %s workspaces",
    (workspaceType) => {
      const store = useWorkspaceStore.getState();
      store.reset();
      store.setSessionState({
        session_id: `session_review_decision_anchor_${workspaceType}`,
        workspace_type: workspaceType,
        stage: "review_decision",
        messages: [],
        checkpoints: [],
        artifact: "# Draft",
        providers: { author: "claude_code", reviewer: "codex" },
        timeline_nodes: [
          {
            node_id: "timeline_node_decision",
            node_type: "review_decision",
            agent: null,
            stage: "review_decision",
            round: 1,
            status: "completed",
            title: "Review Decision Round 1",
            summary: "已选择返修",
            started_at: "2026-05-26T10:02:00Z",
            completed_at: "2026-05-26T10:02:05Z",
            duration_ms: 5_000,
            artifact_ref: null,
            provider_config_snapshot: {
              author: "claude_code",
              reviewer: "codex",
              review_rounds: 1,
            },
          },
        ],
        active_node_id: null,
        artifact_versions: [],
        timeline_node_details: {},
        active_run_id: null,
      });
      store.rebuildChatEntries();

      expect(useWorkspaceStore.getState().chatEntries).toEqual([
        expect.objectContaining({
          id: "timeline_node_decision:timeline-anchor",
          type: "stage_change",
          role: "system",
          content: "Review Decision Round 1 · 已选择返修",
          node_id: "timeline_node_decision",
        }),
      ]);
    },
  );

  it("rebuilds provider prompt events from spec node prompt snapshots", () => {
    const store = useWorkspaceStore.getState();

    store.setSessionState({
      session_id: "session_story_prompt",
      workspace_type: "story",
      stage: "running",
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude_code", reviewer: "codex" },
      timeline_nodes: [
        {
          node_id: "timeline_node_author",
          node_type: "author_run",
          agent: "claude_code",
          stage: "running",
          round: null,
          status: "active",
          title: "Story 生成",
          summary: null,
          started_at: "2026-05-26T10:00:00Z",
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
      active_node_id: "timeline_node_author",
      artifact_versions: [],
      timeline_node_details: {
        timeline_node_author: makeNodeDetail({
          node_id: "timeline_node_author",
          node_type: "author_run",
          agent_role: "author",
          provider: { name: "claude_code", model: "claude-opus-4" },
          prompt: "[user]: 实现爬楼梯 Story Spec",
        }),
      },
      active_run_id: "run-1",
    });
    store.rebuildChatEntries();

    expect(useWorkspaceStore.getState().chatEntries).toEqual([
      expect.objectContaining({
        id: "timeline_node_author:provider-prompt",
        type: "execution_event",
        role: "author",
        content: "Story 生成 · Provider Prompt · 24 字符",
        content_ref: { kind: "provider_prompt", nodeId: "timeline_node_author" },
        content_size: 24,
        has_full_content: true,
        metadata: expect.objectContaining({
          title: "Provider Prompt",
          provider: "claude_code",
        }),
      }),
    ]);
  });

  it("does not duplicate artifact markdown in rebuilt chat entry metadata", () => {
    const store = useWorkspaceStore.getState();
    const hugeMarkdown = "# Artifact\n" + "content\n".repeat(10_000);

    store.setSessionState({
      session_id: "session_huge_artifact",
      workspace_type: "story",
      stage: "running",
      messages: [],
      checkpoints: [],
      artifact: hugeMarkdown,
      providers: { author: "claude_code", reviewer: "codex" },
      timeline_nodes: [
        {
          node_id: "timeline_node_001",
          node_type: "author_run",
          agent: "claude_code",
          stage: "running",
          round: null,
          status: "completed",
          title: "Story 生成",
          summary: "生成完成",
          started_at: "2026-05-26T10:00:00Z",
          completed_at: "2026-05-26T10:01:00Z",
          duration_ms: 60_000,
          artifact_ref: "artifact_current",
          provider_config_snapshot: {
            author: "claude_code",
            reviewer: "codex",
            review_rounds: 1,
          },
        },
      ],
      active_node_id: "timeline_node_001",
      artifact_versions: [
        {
          version: 1,
          markdown: hugeMarkdown,
          generated_by: "claude_code",
          reviewed_by: null,
          review_verdict: null,
          confirmed_by: null,
          is_current: true,
          created_at: "2026-05-26T10:01:00Z",
          source_node_id: "timeline_node_001",
        },
      ],
      timeline_node_details: {
        timeline_node_001: makeNodeDetail(),
      },
      active_run_id: null,
    });
    store.rebuildChatEntries();

    const artifactEntry = useWorkspaceStore
      .getState()
      .chatEntries.find((entry) => entry.type === "artifact_update");
    expect(artifactEntry?.metadata?.markdown).toBeUndefined();
    expect(JSON.stringify(artifactEntry)).not.toContain(hugeMarkdown.slice(0, 100));
  });

  it("does not duplicate provider prompt output in rebuilt chat entry metadata", () => {
    const store = useWorkspaceStore.getState();
    const hugePrompt = "[system]\n" + "prompt line\n".repeat(10_000);

    store.setSessionState({
      session_id: "session_huge_prompt",
      workspace_type: "story",
      stage: "running",
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude_code", reviewer: "codex" },
      timeline_nodes: [
        {
          node_id: "timeline_node_001",
          node_type: "author_run",
          agent: "claude_code",
          stage: "running",
          round: null,
          status: "active",
          title: "Story 生成",
          summary: null,
          started_at: "2026-05-26T10:00:00Z",
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
        timeline_node_001: makeNodeDetail({
          prompt: hugePrompt,
        }),
      },
      active_run_id: "run-1",
    });
    store.rebuildChatEntries();

    const promptEntry = useWorkspaceStore
      .getState()
      .chatEntries.find((entry) => entry.id === "timeline_node_001:provider-prompt");
    expect(promptEntry?.metadata?.output).toBeUndefined();
    expect(JSON.stringify(promptEntry)).not.toContain(hugePrompt.slice(0, 100));
  });

  it("does not duplicate provider prompt execution event output in rebuilt chat entry metadata", () => {
    const store = useWorkspaceStore.getState();
    const hugePrompt = "[system]\n" + "prompt line\n".repeat(10_000);

    store.setSessionState({
      session_id: "session_huge_prompt_event",
      workspace_type: "story",
      stage: "running",
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude_code", reviewer: "codex" },
      timeline_nodes: [
        {
          node_id: "timeline_node_001",
          node_type: "author_run",
          agent: "claude_code",
          stage: "running",
          round: null,
          status: "active",
          title: "Story 生成",
          summary: null,
          started_at: "2026-05-26T10:00:00Z",
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
        timeline_node_001: makeNodeDetail({
          execution_events: [
            {
              event_id: "timeline_node_001_prompt",
              node_id: "timeline_node_001",
              agent: "claude_code",
              kind: "output",
              status: "started",
              title: "Provider Prompt",
              detail: "发送给 Workspace provider 的完整提示词",
              command: null,
              cwd: null,
              output: hugePrompt,
              exit_code: null,
            },
          ],
        }),
      },
      active_run_id: "run-1",
    });
    store.rebuildChatEntries();

    const promptEntry = useWorkspaceStore
      .getState()
      .chatEntries.find((entry) => entry.id === "timeline_node_001:execution-timeline_node_001_prompt");
    expect(promptEntry?.metadata?.output).toBeUndefined();
    expect(JSON.stringify(promptEntry)).not.toContain(hugePrompt.slice(0, 100));
    expect(promptEntry).toEqual(
      expect.objectContaining({
        content_ref: { kind: "provider_prompt", nodeId: "timeline_node_001" },
        content_size: hugePrompt.length,
        has_full_content: true,
      }),
    );
  });

  it("rebuilds lightweight provider content entries from node summaries", () => {
    const store = useWorkspaceStore.getState();

    store.setSessionState({
      session_id: "session_summary_only",
      workspace_type: "story",
      stage: "running",
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "codex", reviewer: "claude_code" },
      timeline_nodes: [
        {
          node_id: "timeline_node_034",
          node_type: "author_run",
          agent: "codex",
          stage: "running",
          round: 1,
          status: "completed",
          title: "Large Provider Stream 33",
          summary: "large provider summary",
          started_at: "2026-06-06T00:00:00Z",
          completed_at: "2026-06-06T00:00:01Z",
          duration_ms: 1000,
          artifact_ref: null,
          provider_config_snapshot: {
            author: "codex",
            reviewer: "claude_code",
            review_rounds: 5,
          },
        },
      ],
      active_node_id: "timeline_node_034",
      artifact_versions: [],
      timeline_node_details: {},
      timeline_node_summaries: {
        timeline_node_034: {
          node_id: "timeline_node_034",
          node_type: "author_run",
          status: "completed",
          agent_role: "author",
          provider_name: "codex",
          prompt_size: 120_000,
          prompt_preview: "完整提示词 large-prompt-0 preview",
          stream_size: 80,
          stream_preview: "stream summary",
          execution_event_count: 1,
          has_large_outputs: true,
          artifact_ref: null,
          started_at: "2026-06-06T00:00:00Z",
          ended_at: "2026-06-06T00:00:01Z",
        },
      },
      active_run_id: "run-1",
    });

    const entries = useWorkspaceStore.getState().chatEntries;

    expect(entries).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          content: expect.stringContaining("Provider Prompt"),
          content_ref: { kind: "provider_prompt", nodeId: "timeline_node_034" },
          content_size: 120_000,
        }),
        expect.objectContaining({
          content: expect.stringContaining("Execution Output"),
          content_ref: {
            kind: "execution_output",
            nodeId: "timeline_node_034",
            eventId: "timeline_node_034_output",
          },
        }),
      ]),
    );
  });

  it("preserves active stream chunk order while avoiding duplicate historical entries", () => {
    const store = useWorkspaceStore.getState();
    store.setSessionState({
      session_id: "workspace_session_stream",
      workspace_type: "story",
      stage: "running",
      superpowers_enabled: true,
      openspec_enabled: true,
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "codex", reviewer: "claude_code" },
      timeline_nodes: [
        {
          node_id: "timeline_node_001",
          node_type: "author_run",
          agent: "codex",
          stage: "running",
          status: "active",
          title: "Story Spec 生成",
          started_at: "2026-06-06T00:00:00Z",
          provider_config_snapshot: {
            author: "codex",
            reviewer: "claude_code",
            review_rounds: 1,
          },
        },
      ],
      active_node_id: "timeline_node_001",
      artifact_versions: [],
      timeline_node_details: {},
      active_run_id: "run-1",
    });

    useWorkspaceStore
      .getState()
      .appendBufferedStreamChunk("A", "timeline_node_001", "author");
    useWorkspaceStore
      .getState()
      .appendBufferedStreamChunk("B", "timeline_node_001", "author");
    useWorkspaceStore.getState().flushBufferedStream("timeline_node_001");

    const streamEntry = useWorkspaceStore
      .getState()
      .chatEntries.find((entry) => entry.id === "timeline_node_001:stream-active");

    expect(streamEntry?.content).toBe("AB");
    expect(
      useWorkspaceStore
        .getState()
        .chatEntries.filter((entry) => entry.id === "timeline_node_001:stream-active"),
    ).toHaveLength(1);
  });

  it("clears buffered stream state after completing a node stream", () => {
    const store = useWorkspaceStore.getState();
    store.setSessionState({
      session_id: "workspace_session_stream_complete",
      workspace_type: "story",
      stage: "running",
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "codex", reviewer: "claude_code" },
      timeline_nodes: [
        {
          node_id: "timeline_node_001",
          node_type: "author_run",
          agent: "codex",
          stage: "running",
          status: "active",
          title: "Story Spec 生成",
          started_at: "2026-06-06T00:00:00Z",
          provider_config_snapshot: {
            author: "codex",
            reviewer: "claude_code",
            review_rounds: 1,
          },
        },
      ],
      active_node_id: "timeline_node_001",
      artifact_versions: [],
      timeline_node_details: {},
      active_run_id: "run-1",
    });

    store.appendStreamChunk("A", "timeline_node_001");
    store.appendStreamChunk("B", "timeline_node_001");
    store.appendBufferedStreamChunk("A", "timeline_node_001", "author");
    store.appendBufferedStreamChunk("B", "timeline_node_001", "author");

    store.completeBufferedStream("timeline_node_001", "msg_001", "checkpoint_001");

    const state = useWorkspaceStore.getState();
    const streamEntry = state.chatEntries.find(
      (entry) => entry.id === "timeline_node_001:stream-active",
    );
    expect(streamEntry?.content).toBe("AB");
    expect(state.nodeDetails.timeline_node_001.messages.at(-1)?.content).toBe("AB");
    expect(state.streamBuffers.timeline_node_001).toBeUndefined();
  });

  it("stores review verdict and pending decision by node", () => {
    const store = useWorkspaceStore.getState();
    store.setNodeVerdict("timeline_node_003", {
      verdict: "revise",
      comments: "需要补充失败路径",
      summary: "补充失败路径",
    });
    store.setPendingDecision({
      node_id: "timeline_node_004",
      round: 1,
      options: ["continue", "continue_with_context", "human_intervene"],
    });

    expect(useWorkspaceStore.getState().nodeDetails.timeline_node_003.verdict).toEqual({
      verdict: "revise",
      comments: "需要补充失败路径",
      summary: "补充失败路径",
    });
    expect(useWorkspaceStore.getState().pendingDecision?.node_id).toBe("timeline_node_004");
  });

  it("stores and clears protocol errors", () => {
    const store = useWorkspaceStore.getState();

    store.setProtocolError({ code: "INVALID_MESSAGE_FOR_STAGE", message: "阶段不允许" });

    expect(useWorkspaceStore.getState().protocolError).toEqual({
      code: "INVALID_MESSAGE_FOR_STAGE",
      message: "阶段不允许",
    });

    store.setProtocolError(null);

    expect(useWorkspaceStore.getState().protocolError).toBeNull();
  });

  it("stores and clears provider locked snapshots", () => {
    const store = useWorkspaceStore.getState();

    store.setProviderLocked({
      snapshot: { author: "claude_code", reviewer: "codex", review_rounds: 1 },
      locked_at: "2026-05-20T14:35:00Z",
    });

    expect(useWorkspaceStore.getState().providerLocked).toBe(true);
    expect(useWorkspaceStore.getState().providerSnapshot).toEqual({
      author: "claude_code",
      reviewer: "codex",
      review_rounds: 1,
    });
    expect(useWorkspaceStore.getState().providerLockedAt).toBe("2026-05-20T14:35:00Z");

    store.setProviderLocked(null);

    expect(useWorkspaceStore.getState().providerLocked).toBe(false);
    expect(useWorkspaceStore.getState().providerSnapshot).toBeNull();
    expect(useWorkspaceStore.getState().providerLockedAt).toBeNull();
  });

  it("records live artifact updates as artifact versions", () => {
    const store = useWorkspaceStore.getState();
    useWorkspaceStore.setState({
      providers: { author: "fake", reviewer: "codex" },
      activeNodeId: "node-author-1",
    });

    store.setArtifact("# Draft v1", 1);
    store.setArtifact("# Draft v2", 2);

    expect(useWorkspaceStore.getState().artifact).toBe("# Draft v2");
    expect(useWorkspaceStore.getState().artifactVersions).toMatchObject([
      {
        version: 1,
        markdown: "# Draft v1",
        generated_by: "fake",
        source_node_id: "node-author-1",
      },
      {
        version: 2,
        markdown: "# Draft v2",
        generated_by: "fake",
        source_node_id: "node-author-1",
      },
    ]);
  });

  it("resolves the latest unresolved gate prompt entry", () => {
    const store = useWorkspaceStore.getState();
    store.appendChatEntry({
      id: "gate-1",
      type: "gate_prompt",
      role: "system",
      content: "等待人工确认 1",
      timestamp: "2026-05-21T10:00:00Z",
    });
    store.appendChatEntry({
      id: "gate-2",
      type: "gate_prompt",
      role: "system",
      content: "等待人工确认 2",
      timestamp: "2026-05-21T10:01:00Z",
    });

    store.resolveGateEntry("request-change");
    const firstResolution = useWorkspaceStore.getState().chatEntries;
    expect(firstResolution[0]).toEqual(expect.objectContaining({ id: "gate-1" }));
    expect(firstResolution[0]).not.toHaveProperty("resolved");
    expect(firstResolution[1]).toEqual(
      expect.objectContaining({
        id: "gate-2",
        resolved: true,
        resolution: "request-change",
      }),
    );

    store.resolveGateEntry("confirm");
    expect(useWorkspaceStore.getState().chatEntries).toEqual([
      expect.objectContaining({
        id: "gate-1",
        resolved: true,
        resolution: "confirm",
      }),
      expect.objectContaining({
        id: "gate-2",
        resolved: true,
        resolution: "request-change",
      }),
    ]);
  });
});
