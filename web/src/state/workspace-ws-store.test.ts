import { describe, expect, it, vi } from "vitest";
import type { ChatEntry } from "./chat-entries";
import {
  emptyWorkspaceContentCache,
  workspaceContentCacheValues,
} from "./workspace-content-cache";
import {
  selectPrepareContextNotes,
  useWorkspaceStore,
  type TimelineNode,
} from "./workspace-ws-store";
import type { WorkspaceSessionStatePayload } from "./workspace-ws-store-types";
import {
  installWorkspaceStoreTestHooks,
  makeCompileArtifactPayload,
  makeContextBlockerArtifactPayload,
  makeDraftArtifactPayload,
  makeNodeDetail,
  makeOutlineArtifactPayload,
  makeWorkItemPlanCandidate,
} from "./workspace-ws-store.test-utils";

describe("workspace ws store base state", () => {
  installWorkspaceStoreTestHooks();

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

  it.each(["story", "design", "work_item"] as const)(
    "rebuilds %s review diagnostics from hydrated node detail",
    (workspaceType) => {
      const store = useWorkspaceStore.getState();
      const diagnostic = {
        code: "invalid_json",
        message: "Reviewer 输出不是合法 JSON",
        repair_attempted: true,
        repair_succeeded: false,
        raw_output_preview: "未校验内容",
      };
      useWorkspaceStore.setState({
        sessionId: `workspace_session_${workspaceType}`,
        workspaceType,
        timelineNodes: [
          {
            node_id: `node-review-${workspaceType}`,
            node_type: "reviewer_run",
            agent: "codex",
            stage: "cross_review",
            round: 1,
            status: "completed",
            title: "Review Round 1",
            summary: "需要人工检查",
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
      });

      store.setNodeDetail(
        makeNodeDetail({
          node_id: `node-review-${workspaceType}`,
          node_type: "reviewer_run",
          verdict: {
            verdict: "needs_human",
            comments: "可信 Reviewer comments",
            summary: "需要人工检查",
            findings: [],
            review_gate: "user_triage_required",
            structured_output_diagnostic: diagnostic,
          },
        }),
      );

      expect(
        useWorkspaceStore
          .getState()
          .chatEntries.find((entry) => entry.type === "review_verdict")?.metadata,
      ).toMatchObject({
        verdict: "needs_human",
        comments: "可信 Reviewer comments",
        summary: "需要人工检查",
        findings: [],
        review_gate: "user_triage_required",
        structured_output_diagnostic: diagnostic,
      });
    },
  );

  it("rebuilds user triage gate prompts with review metadata from hydrated node detail", () => {
    const store = useWorkspaceStore.getState();
    useWorkspaceStore.setState({
      sessionId: "workspace_session_0001",
      stage: "human_confirm",
      timelineNodes: [
        {
          node_id: "node-review-1",
          node_type: "reviewer_run",
          agent: "codex",
          stage: "cross_review",
          round: 1,
          status: "completed",
          title: "Review Round 1",
          summary: "返修意图需要人工判断",
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
        {
          node_id: "node-human-1",
          node_type: "human_confirm",
          agent: null,
          stage: "human_confirm",
          round: 1,
          status: "paused",
          title: "人工确认",
          summary: "等待用户裁决",
          started_at: "2026-05-20T00:01:00Z",
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
    });

    store.setNodeDetail(
      makeNodeDetail({
        node_id: "node-review-1",
        node_type: "reviewer_run",
        streaming_content: "Reviewer 要求返修但未输出 finding",
        verdict: {
          verdict: "needs_human",
          comments: "请补齐异常路径说明。",
          summary: "返修意图需要人工判断",
          findings: [
            {
              severity: "suggestion",
              message: "建议补充说明；不影响下一阶段",
              evidence: "当前版本可用",
              required_action: "补充说明段落",
            }
          ],
          review_gate: "user_triage_required",
        },
      }),
    );

    const gatePrompt = useWorkspaceStore
      .getState()
      .chatEntries.find((entry) => entry.type === "gate_prompt");
    expect(gatePrompt).toMatchObject({
      content: "需要人工确认",
      metadata: expect.objectContaining({
        comments: "请补齐异常路径说明。",
        review_gate: "user_triage_required",
        findings: [expect.objectContaining({ message: "建议补充说明；不影响下一阶段" })],
      }),
    });
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
});

// REQ-UI37-18：静默判据取“最近一次引擎事件”而非节点起点，因此 store 必须在
// timeline_node_updated 与 stream_chunk 两类事件上刷新节点的 last_event_at。
describe("workspace ws timeline node event clock", () => {
  installWorkspaceStoreTestHooks();

  const timelineNode = (overrides: Partial<TimelineNode> = {}): TimelineNode => ({
    node_id: "node_1",
    node_type: "author_run",
    agent: "claude_code",
    stage: "running",
    round: null,
    status: "active",
    title: "Author 运行",
    summary: null,
    started_at: "2026-09-13T00:00:00Z",
    completed_at: null,
    duration_ms: null,
    artifact_ref: null,
    provider_config_snapshot: { author: "claude_code", reviewer: null, review_rounds: 1 },
    ...overrides,
  });

  it("refreshes a node last-event clock on stream chunks and status updates", () => {
    vi.useFakeTimers();
    try {
      vi.setSystemTime(new Date("2026-09-13T01:00:00Z"));
      const store = useWorkspaceStore.getState();
      store.setTimelineNodesForTest([timelineNode({ node_id: "n1" })]);

      store.appendStreamChunk("chunk", "n1");

      expect(useWorkspaceStore.getState().timelineNodes[0]?.last_event_at).toBe(
        "2026-09-13T01:00:00.000Z",
      );

      vi.setSystemTime(new Date("2026-09-13T01:04:00Z"));
      store.updateTimelineNode("n1", "active", "仍在运行", null);

      expect(useWorkspaceStore.getState().timelineNodes[0]?.last_event_at).toBe(
        "2026-09-13T01:04:00.000Z",
      );
    } finally {
      vi.useRealTimers();
    }
  });
});


// v38 复验 #1（重复「Review Round 1」卡）：活跃 run 期间重连/初帧 attach，服务端把
// snapshot 基线压到补发窗口首事件之前并重发窗口帧（src/web/workspace_session/
// attachment.rs activate_attachment_with_initial_frames + journal.rs active_run_window），
// 窗口内含客户端已消化的 timeline_node_created 重叠帧；前端对 session_state 无条件
// 拉低 event_seq 基线（useWorkspaceWs.ts），重叠 created 帧必然通过 seq 去重。此时
// addTimelineNode 若盲 push，同一 node_id 双写 → timeline 左栏渲染两张相同卡片。
describe("workspace ws timeline node replay idempotency", () => {
  installWorkspaceStoreTestHooks();

  const reviewerRunNode: TimelineNode = {
    node_id: "timeline_node_004",
    node_type: "reviewer_run",
    agent: "codex",
    stage: "cross_review",
    round: 1,
    status: "active",
    title: "Review Round 1",
    summary: null,
    started_at: "2026-09-05T21:30:00Z",
    completed_at: null,
    duration_ms: null,
    artifact_ref: "artifact_current",
    provider_config_snapshot: {
      author: "claude_code",
      reviewer: "codex",
      review_rounds: 1,
    },
  };

  function sessionSnapshot(timelineNodes: TimelineNode[]): WorkspaceSessionStatePayload {
    return {
      session_id: "session_replay_created",
      workspace_type: "story",
      stage: "cross_review",
      session_status: "open",
      flow_kind: "legacy",
      run_policy: "interactive",
      run_history: {
        seen_fingerprints: [],
        repairs_used: 0,
        manual_repairs_used: 0,
        transitions_used: 0,
        initial_review_count: 0,
        verification_review_count: 0,
      },
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude_code", reviewer: "codex" },
      timeline_nodes: timelineNodes,
      active_node_id: timelineNodes.at(-1)?.node_id ?? null,
      artifact_versions: [],
      timeline_node_details: {},
      timeline_node_summaries: {},
    };
  }

  it("ignores a replayed timeline_node_created for a node already in the snapshot", () => {
    const store = useWorkspaceStore.getState();
    store.setSessionState(sessionSnapshot([reviewerRunNode]));

    // 补发窗口的重叠 created 帧（session_state 分支与 timeline_node_created 分支的
    // 全部 store 副作用即 setSessionState + addTimelineNode）
    store.addTimelineNode(reviewerRunNode);

    const nodes = useWorkspaceStore.getState().timelineNodes;
    expect(nodes.filter((node) => node.node_id === "timeline_node_004")).toHaveLength(1);
    expect(nodes).toHaveLength(1);
  });

  it("does not regress an already-completed node when its initial created frame replays", () => {
    const store = useWorkspaceStore.getState();
    const completedNode = {
      ...reviewerRunNode,
      status: "completed",
      completed_at: "2026-09-05T21:31:00Z",
      duration_ms: 60_000,
    } as const;
    store.setSessionState(sessionSnapshot([completedNode]));

    // 重放帧是创建时刻的初态（active）；已存在的终态节点不得被回退
    store.addTimelineNode(reviewerRunNode);

    expect(useWorkspaceStore.getState().timelineNodes).toEqual([completedNode]);
    expect(useWorkspaceStore.getState().activeNodeId).toBe("timeline_node_004");
  });
});
