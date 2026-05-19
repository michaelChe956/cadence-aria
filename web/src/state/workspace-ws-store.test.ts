import { beforeEach, describe, expect, it } from "vitest";
import { useWorkspaceStore } from "./workspace-ws-store";

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
          node_type: "generation",
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

  it("groups stream chunks and execution events by timeline node", () => {
    const store = useWorkspaceStore.getState();
    store.addTimelineNode({
      node_id: "timeline_node_002",
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
    expect(detail.streamingContent).toBe("");
    expect(detail.messages).toEqual([
      expect.objectContaining({
        id: "msg_002",
        role: "assistant",
        content: "review output",
      }),
    ]);
    expect(detail.executionEvents).toHaveLength(1);
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
});
