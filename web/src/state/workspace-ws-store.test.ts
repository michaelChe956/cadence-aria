import { beforeEach, describe, expect, it } from "vitest";
import type { NodeDetail } from "../api/types";
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

    store.setProviderLocked(null);

    expect(useWorkspaceStore.getState().providerLocked).toBe(false);
    expect(useWorkspaceStore.getState().providerSnapshot).toBeNull();
  });
});
