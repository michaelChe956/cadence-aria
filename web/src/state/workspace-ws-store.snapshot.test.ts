import { describe, expect, it } from "vitest";
import type { ChatEntry } from "./chat-entries";
import {
  emptyWorkspaceContentCache,
  workspaceContentCacheValues,
} from "./workspace-content-cache";
import { selectPrepareContextNotes, useWorkspaceStore } from "./workspace-ws-store";
import {
  installWorkspaceStoreTestHooks,
  makeCompileArtifactPayload,
  makeContextBlockerArtifactPayload,
  makeDraftArtifactPayload,
  makeNodeDetail,
  makeOutlineArtifactPayload,
  makeWorkItemPlanCandidate,
} from "./workspace-ws-store.test-utils";

describe("workspace ws store snapshots", () => {
  installWorkspaceStoreTestHooks();

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
});
