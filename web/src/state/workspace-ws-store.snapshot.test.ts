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
import { observerStateFromSessionState } from "./workspace-observer-store";
import type {
  ExecutionEvent,
  TimelineNode,
  WorkspaceSessionStatePayload,
} from "./workspace-ws-store-types";
import { planRepairSnapshotFixture } from "./workspace-plan-repair-test-fixtures";

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
      session_status: "waiting_for_human",
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

  it("restores reviewerEnabled from the session snapshot projection", () => {
    const store = useWorkspaceStore.getState();
    // 初始默认 true；创建时未启用 review 的会话经 SessionState 投影恢复为 false。
    expect(useWorkspaceStore.getState().reviewerEnabled).toBe(true);

    store.setSessionState({
      session_id: "session_reviewer_off",
      workspace_type: "story",
      stage: "author_confirm",
      session_status: "waiting_for_human",
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
      reviewer_enabled_at_start: false,
    });
    expect(useWorkspaceStore.getState().reviewerEnabled).toBe(false);

    store.setSessionState({
      session_id: "session_reviewer_off",
      workspace_type: "story",
      stage: "author_confirm",
      session_status: "waiting_for_human",
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
      reviewer_enabled_at_start: true,
    });
    expect(useWorkspaceStore.getState().reviewerEnabled).toBe(true);

    // 旧会话（字段缺失/None）→ 不动，保持现值。
    store.setSessionState({
      session_id: "session_reviewer_off",
      workspace_type: "story",
      stage: "author_confirm",
      session_status: "waiting_for_human",
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
    });
    expect(useWorkspaceStore.getState().reviewerEnabled).toBe(true);
  });

  it("initializes timeline state from a session snapshot", () => {
    const store = useWorkspaceStore.getState();

    store.setSessionState({
      session_id: "session_003",
      workspace_type: "story",
      stage: "cross_review",
      session_status: "running",
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
      session_status: "confirmed",
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
      session_status: "running",
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
      session_status: "running",
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

  // F-47 REQ-NDR-01：快照应用是 merge 而非重建——已水合 detail 不得被空壳清空。
  // 三步断言结构与 F47Diag 实测探针一致（快照→水合→再快照），现状第三步回 0（红）。
  it("keeps a hydrated node detail when a later snapshot arrives without that detail", () => {
    const store = useWorkspaceStore.getState();
    const snapshot = f47DesignSnapshot({ timeline_node_details: {} });

    store.setSessionState(snapshot);
    expect(
      useWorkspaceStore.getState().nodeDetails.timeline_node_002?.execution_events ?? [],
    ).toHaveLength(0);
    expect(usageEntryCount()).toBe(0);

    store.setNodeDetail(
      makeNodeDetail({
        node_id: "timeline_node_002",
        session_id: "session_f47",
        status: "failed",
        streaming_content: "author 正文",
        execution_events: [usageEvent({ input_tokens: 14_512, output_tokens: 19_493 })],
      }),
    );
    expect(
      useWorkspaceStore.getState().nodeDetails.timeline_node_002.execution_events,
    ).toHaveLength(1);
    expect(usageEntryCount()).toBe(1);

    store.setSessionState(snapshot);
    expect(
      useWorkspaceStore.getState().nodeDetails.timeline_node_002.execution_events,
    ).toHaveLength(1);
    expect(usageEntryCount()).toBe(1);
  });

  // REQ-NDR-01 场景二：快照内联 detail 覆盖对应节点（merge 契约边界，防版本倒挂）。
  it("lets a snapshot inline detail override the hydrated detail of the same node", () => {
    const store = useWorkspaceStore.getState();
    store.setSessionState(f47DesignSnapshot({ timeline_node_details: {} }));
    store.setNodeDetail(
      makeNodeDetail({
        node_id: "timeline_node_002",
        session_id: "session_f47",
        streaming_content: "REST 水合输出",
      }),
    );

    store.setSessionState(
      f47DesignSnapshot({
        timeline_node_details: {
          timeline_node_002: makeNodeDetail({
            node_id: "timeline_node_002",
            session_id: "session_f47",
            streaming_content: "快照内联输出",
          }),
        },
      }),
    );

    expect(useWorkspaceStore.getState().nodeDetails.timeline_node_002.streaming_content).toBe(
      "快照内联输出",
    );
  });

  // REQ-NDR-01 + Review Focus 1：快照内联投影会把执行事件 output 置 null 瘦身
  // （build_session_state_node_detail）——内联优先不得把已水合的 usage 载荷抹掉，
  // 否则 work_item_plan 的 outline/draft/batch run 节点（内联白名单）会在下一个
  // 门/choice/重连帧上丢 token 行，且水合去重 ref 阻止二次拉取 → 不自愈。
  it("keeps hydrated event payloads when an inline snapshot detail overrides the node", () => {
    const store = useWorkspaceStore.getState();
    store.setSessionState(f47DesignSnapshot({ timeline_node_details: {} }));
    store.setNodeDetail(
      makeNodeDetail({
        node_id: "timeline_node_002",
        session_id: "session_f47",
        streaming_content: "完整正文（水合）",
        execution_events: [usageEvent({ input_tokens: 464, output_tokens: 21_136 })],
      }),
    );
    expect(usageEntryCount()).toBe(1);

    // 第二帧：该节点被内联（投影裁剪：正文只剩 preview、output 置 null）。
    store.setSessionState(
      f47DesignSnapshot({
        timeline_node_details: {
          timeline_node_002: makeNodeDetail({
            node_id: "timeline_node_002",
            session_id: "session_f47",
            streaming_content: "内联摘要",
            execution_events: [
              {
                ...usageEvent({ input_tokens: 464, output_tokens: 21_136 }),
                output: null,
              },
            ],
          }),
        },
      }),
    );

    const detail = useWorkspaceStore.getState().nodeDetails.timeline_node_002;
    // 内联优先：正文取内联（较新的投影）。
    expect(detail.streaming_content).toBe("内联摘要");
    // 但裁剪掉的 output 不覆盖已水合载荷 → token 行数据仍在。
    expect(detail.execution_events[0]?.output).toContain('"output_tokens":21136');
    expect(usageEntryCount()).toBe(1);
  });

  // merge 只在同会话内保留：跨会话节点 id 同构（timeline_node_00N），不得串场。
  it("does not carry hydrated details across a session switch", () => {
    const store = useWorkspaceStore.getState();
    store.setSessionState(f47DesignSnapshot({ timeline_node_details: {} }));
    store.setNodeDetail(
      makeNodeDetail({
        node_id: "timeline_node_002",
        session_id: "session_f47",
        streaming_content: "上一会话输出",
      }),
    );

    store.setSessionState(
      f47DesignSnapshot({
        session_id: "session_f48",
        timeline_node_details: {},
        timeline_nodes: [f47TimelineNode({ node_id: "timeline_node_002" })],
      }),
    );

    expect(
      useWorkspaceStore.getState().nodeDetails.timeline_node_002.streaming_content,
    ).toBe("");
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
      session_status: "running",
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

// F-47 快照 merge 用例的共用夹具：design 会话（author/reviewer 节点一律走 summary
// 分支、快照不内联 detail），与诊断报告实测的会话形态一致。
function f47TimelineNode(overrides: Partial<TimelineNode> = {}): TimelineNode {
  return {
    node_id: "timeline_node_002",
    node_type: "author_run",
    agent: "pi",
    stage: "running",
    round: null,
    status: "failed",
    title: "Author Run",
    summary: null,
    started_at: "2026-09-23T16:15:05.965Z",
    completed_at: "2026-09-23T16:16:44.926Z",
    duration_ms: 98_961,
    artifact_ref: null,
    provider_config_snapshot: {
      author: "pi",
      reviewer: "kimi_code",
      review_rounds: 1,
    },
    ...overrides,
  };
}

function f47DesignSnapshot(
  overrides: Partial<WorkspaceSessionStatePayload> = {},
): WorkspaceSessionStatePayload {
  return {
    session_id: "session_f47",
    workspace_type: "design",
    stage: "running",
    session_status: "running",
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
    providers: { author: "pi", reviewer: "kimi_code" },
    timeline_nodes: [f47TimelineNode()],
    active_node_id: null,
    artifact_versions: [],
    timeline_node_details: {},
    active_run_id: null,
    ...overrides,
  };
}

function usageEvent(payload: Record<string, number>): ExecutionEvent {
  return {
    event_id: "usage_author",
    kind: "usage",
    status: "completed",
    title: "author token usage",
    output: JSON.stringify({ role: "author", ...payload }),
  };
}

/** 对话流里带 token 读数的条目数——「token 行可见」的数据面等价断言。 */
function usageEntryCount(): number {
  return useWorkspaceStore
    .getState()
    .chatEntries.filter((entry) => (entry as ChatEntry).metadata?.usage !== undefined).length;
}

describe("plan repair snapshot mirroring", () => {
  installWorkspaceStoreTestHooks();

  function sessionStateBase(
    overrides: Partial<WorkspaceSessionStatePayload> = {},
  ): WorkspaceSessionStatePayload {
    return {
      session_id: "session_child",
      workspace_type: "work_item_plan",
      stage: "running",
      session_status: "running",
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
      providers: { author: "fake", reviewer: null },
      ...overrides,
    };
  }

  it("mirrors the plan_repair snapshot from session_state", () => {
    const snapshot = planRepairSnapshotFixture("session_child");
    useWorkspaceStore.getState().setSessionState(
      sessionStateBase({ plan_repair: snapshot }),
    );
    expect(useWorkspaceStore.getState().planRepair).toEqual(snapshot);
  });

  it("clears the mirror when a later snapshot has no plan repair", () => {
    const store = useWorkspaceStore.getState();
    store.setSessionState(
      sessionStateBase({ plan_repair: planRepairSnapshotFixture("session_child") }),
    );
    expect(useWorkspaceStore.getState().planRepair).not.toBeNull();

    store.setSessionState(sessionStateBase({ plan_repair: null }));
    expect(useWorkspaceStore.getState().planRepair).toBeNull();
  });

  it("defaults the mirror to null when the field is absent", () => {
    useWorkspaceStore.getState().setSessionState(sessionStateBase());
    expect(useWorkspaceStore.getState().planRepair).toBeNull();
  });

  it("mirrors plan repair into observer states for takeover sessions", () => {
    const snapshot = planRepairSnapshotFixture("session_child");
    const observerState = observerStateFromSessionState(
      sessionStateBase({ plan_repair: snapshot }) as never,
    );
    expect(observerState.planRepair).toEqual(snapshot);
  });
});
