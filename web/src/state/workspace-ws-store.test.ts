import { beforeEach, describe, expect, it } from "vitest";
import type { NodeDetail, WorkItemPlanCandidateDto } from "../api/types";
import type { ChatEntry } from "./chat-entries";
import {
  emptyWorkspaceContentCache,
  workspaceContentCacheValues,
} from "./workspace-content-cache";
import { selectPrepareContextNotes, useWorkspaceStore } from "./workspace-ws-store";

function makeWorkItemPlanCandidate(
  overrides: Partial<WorkItemPlanCandidateDto> = {},
): WorkItemPlanCandidateDto {
  return {
    plan: {
      plan_id: "plan_001",
      project_id: "project_001",
      issue_id: "issue_001",
      title: "Plan 001",
      source_story_spec_ids: [],
      source_design_spec_ids: [],
      options: {
        include_integration_tests: false,
        include_e2e_tests: false,
        force_frontend_backend_split: false,
        require_execution_plan_confirm: false,
      },
      status: "draft",
      work_item_ids: [],
      repository_profile_ref: null,
      verification_plan_ids: [],
      dependency_graph: [],
      created_from_provider_run: null,
      validator_findings: [],
      review_summary: null,
      created_at: "2026-06-17T00:00:00Z",
      updated_at: "2026-06-17T00:00:00Z",
    },
    work_items: [],
    verification_plans: [],
    repository_profile: null,
    validator_findings: [],
    ...overrides,
  };
}

function makeOutlineArtifactPayload() {
  return {
    outline: {
      id: "outline_version_001",
      plan_id: "plan_001",
      strategy_summary: "Split frontend and backend work.",
      work_items: [
        {
          outline_id: "outline_backend",
          title: "Backend flow",
          kind: "backend",
          sequence_hint: 1,
          depends_on_outline_ids: [],
          exclusive_write_scopes: ["src/product"],
          forbidden_write_scopes: [],
          context_budget: {
            target_context_k: "medium",
            max_summary_chars: 4000,
            max_handoff_chars: 2000,
            max_code_context_chars: 12000,
            max_context_file_refs: 12,
            max_traceability_refs: 12,
            max_dependency_handoffs: 4,
          },
          required_handoff_from_outline_ids: [],
          verification_strategy: "cargo test --locked",
          risk_notes: [],
        },
      ],
      dependency_graph: [],
      risks: [],
      handoff_plan: [],
      created_at: "2026-06-23T00:00:00Z",
      updated_at: "2026-06-23T00:00:00Z",
    },
    design_context_gaps: [],
    validator_findings: [],
    context_blockers: [],
    current_generation_round_id: "round_001",
    selected_generation_mode: null,
  };
}

function makeDraftArtifactPayload() {
  return {
    draft_record: {
      draft_id: "draft_backend_001",
      plan_id: "plan_001",
      generation_round_id: "round_001",
      outline_id: "outline_backend",
      batch_id: null,
      candidate: {
        outline_id: "outline_backend",
        title: "Backend flow",
        kind: "backend",
        implementation_context: "Implement backend state transitions.",
        exclusive_write_scopes: ["src/product"],
        forbidden_write_scopes: [],
        depends_on_outline_ids: [],
        required_handoff_from_outline_ids: [],
        verification_plan: {
          commands: [],
          manual_checks: [],
          required_gates: [],
          risk_notes: [],
        },
        handoff_summary: "Backend state is ready for frontend.",
      },
      status: "draft",
      active: true,
      superseded: false,
      superseded_by_draft_id: null,
      supersede_reason: null,
      copied_from_draft_id: null,
      generated_from_node_id: "node_draft",
      accepted_by_node_id: null,
      created_at: "2026-06-23T00:00:00Z",
      updated_at: "2026-06-23T00:00:00Z",
    },
    validator_findings: [],
    can_accept: true,
  };
}

function makeCompileArtifactPayload() {
  return {
    compile_id: "compile_001",
    generation_round_id: "round_001",
    status: "committed",
    plan_commit_state: "committed",
    work_item_ids: ["work_item_backend"],
    verification_plan_ids: ["verification_backend"],
    child_session_ids: ["session_child_backend"],
    validator_findings: [],
  };
}

function makeContextBlockerArtifactPayload() {
  return {
    context_blockers: [],
    design_context_gaps: [],
    exploration_summary:
      "Outline 自动重跑后仍校验失败，已停止继续生成。主要问题：duplicate_outline_id - outline id outline_backend_session is duplicated。请终止当前流程并重新创建 Work Item Plan。",
    allowed_actions: ["provide_context", "abort"],
  };
}

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
              severity: "optional",
              message: "建议补充说明",
              evidence: "当前版本可用",
              impact: "不影响下一阶段",
              required_action: "补充说明段落",
            },
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
        findings: [expect.objectContaining({ message: "建议补充说明" })],
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

  it("rebuilds work item plan author stream from author_run timeline node content", () => {
    const store = useWorkspaceStore.getState();

    store.setSessionState({
      session_id: "session_work_item_plan_progress",
      workspace_type: "work_item_plan",
      stage: "running",
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude_code", reviewer: "codex" },
      timeline_nodes: [
        {
          node_id: "timeline_node_work_item_plan_author",
          node_type: "author_run",
          agent: "claude_code",
          stage: "running",
          round: null,
          status: "active",
          title: "Work Item Plan 生成",
          summary: null,
          started_at: "2026-06-19T10:00:00Z",
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
      active_node_id: "timeline_node_work_item_plan_author",
      artifact_versions: [],
      timeline_node_details: {
        timeline_node_work_item_plan_author: makeNodeDetail({
          node_id: "timeline_node_work_item_plan_author",
          node_type: "author_run",
          agent_role: "author",
          provider: { name: "claude_code", model: "claude-opus-4" },
          streaming_content: "Fake Work Item Plan streaming draft",
        }),
      },
      active_run_id: null,
    });

    expect(useWorkspaceStore.getState().chatEntries).toEqual([
      expect.objectContaining({
        type: "provider_stream",
        role: "author",
        content: "Fake Work Item Plan streaming draft",
        node_id: "timeline_node_work_item_plan_author",
        metadata: expect.objectContaining({ provider: "claude_code" }),
      }),
    ]);
  });

  it("rebuilds work item plan outline provider events as author messages", () => {
    const store = useWorkspaceStore.getState();

    store.setSessionState({
      session_id: "session_work_item_plan_outline",
      workspace_type: "work_item_plan",
      stage: "running",
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude_code", reviewer: "codex" },
      timeline_nodes: [
        {
          node_id: "timeline_node_outline",
          node_type: "work_item_plan_outline_run",
          agent: "claude_code",
          stage: "running",
          round: null,
          status: "active",
          title: "WorkItemPlan Outline 生成",
          summary: null,
          started_at: "2026-06-23T10:00:00Z",
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
      active_node_id: "timeline_node_outline",
      artifact_versions: [],
      timeline_node_details: {
        timeline_node_outline: makeNodeDetail({
          node_id: "timeline_node_outline",
          node_type: "work_item_plan_outline_run",
          agent_role: "author",
          provider: { name: "claude_code", model: "claude-opus-4" },
          execution_events: [
            {
              event_id: "provider",
              node_id: "timeline_node_outline",
              agent: "claude_code",
              kind: "provider",
              status: "started",
              title: "Claude Code provider started",
              detail: null,
              command: null,
              cwd: "/tmp/repo",
              output: null,
              exit_code: null,
            },
          ],
        }),
      },
      active_run_id: null,
    });

    expect(useWorkspaceStore.getState().chatEntries).toEqual([
      expect.objectContaining({
        type: "execution_event",
        role: "author",
        content: "Claude Code provider started",
        node_id: "timeline_node_outline",
        metadata: expect.objectContaining({ provider: "claude_code" }),
      }),
    ]);
  });

  it("does not rebuild work item plan provider stream from start_generation nodes", () => {
    const store = useWorkspaceStore.getState();

    store.setSessionState({
      session_id: "session_work_item_plan_start_generation",
      workspace_type: "work_item_plan",
      stage: "running",
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude_code", reviewer: "codex" },
      timeline_nodes: [
        {
          node_id: "timeline_node_start_generation",
          node_type: "start_generation",
          agent: null,
          stage: "prepare_context",
          round: null,
          status: "completed",
          title: "开始生成",
          summary: null,
          started_at: "2026-06-19T10:00:00Z",
          completed_at: "2026-06-19T10:00:00Z",
          duration_ms: 0,
          artifact_ref: null,
          provider_config_snapshot: {
            author: "claude_code",
            reviewer: "codex",
            review_rounds: 1,
          },
        },
      ],
      active_node_id: "timeline_node_start_generation",
      artifact_versions: [],
      timeline_node_details: {
        timeline_node_start_generation: makeNodeDetail({
          node_id: "timeline_node_start_generation",
          node_type: "start_generation",
          streaming_content: "Fake Work Item Plan streaming draft",
        }),
      },
      active_run_id: null,
    });

    expect(
      useWorkspaceStore
        .getState()
        .chatEntries.some((entry) => entry.type === "provider_stream"),
    ).toBe(false);
    expect(useWorkspaceStore.getState().chatEntries).toContainEqual(
      expect.objectContaining({
        type: "start_generation",
        role: "system",
        node_id: "timeline_node_start_generation",
      }),
    );
  });

  it("restores completed work item plan author stream while focusing active author confirm", () => {
    const store = useWorkspaceStore.getState();

    store.setSessionState({
      session_id: "session_work_item_plan_author_confirm",
      workspace_type: "work_item_plan",
      stage: "author_confirm",
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude_code", reviewer: "codex" },
      timeline_nodes: [
        {
          node_id: "timeline_node_work_item_plan_author",
          node_type: "author_run",
          agent: "claude_code",
          stage: "running",
          round: null,
          status: "completed",
          title: "Work Item Plan 生成",
          summary: "WorkItemPlan provider 输出完成",
          started_at: "2026-06-21T10:00:00Z",
          completed_at: "2026-06-21T10:01:00Z",
          duration_ms: 60_000,
          artifact_ref: "artifact_version_001",
          provider_config_snapshot: {
            author: "claude_code",
            reviewer: "codex",
            review_rounds: 1,
          },
        },
        {
          node_id: "timeline_node_author_confirm",
          node_type: "author_confirm",
          agent: null,
          stage: "author_confirm",
          round: null,
          status: "active",
          title: "Author 结果确认",
          summary: "WorkItemPlan 候选已生成，等待确认",
          started_at: "2026-06-21T10:01:00Z",
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
      active_node_id: "timeline_node_author_confirm",
      artifact_versions: [],
      timeline_node_details: {
        timeline_node_work_item_plan_author: makeNodeDetail({
          node_id: "timeline_node_work_item_plan_author",
          node_type: "author_run",
          agent_role: "author",
          provider: { name: "claude_code", model: "claude-opus-4" },
          streaming_content: "Fake Work Item Plan streaming draft",
        }),
        timeline_node_author_confirm: makeNodeDetail({
          node_id: "timeline_node_author_confirm",
          node_type: "author_confirm",
          streaming_content: "",
        }),
      },
      active_run_id: null,
    });

    expect(useWorkspaceStore.getState().activeNodeId).toBe("timeline_node_author_confirm");
    expect(useWorkspaceStore.getState().chatEntries).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          type: "provider_stream",
          role: "author",
          node_id: "timeline_node_work_item_plan_author",
          content: "Fake Work Item Plan streaming draft",
        }),
        expect.objectContaining({
          type: "stage_change",
          role: "system",
          node_id: "timeline_node_author_confirm",
        }),
      ]),
    );
  });

  it.each([
    ["story", "已按 reviewer 意见返修 Story Spec"],
    ["design", "已按 reviewer 意见返修 Design Spec"],
    ["work_item", "已按 reviewer 意见返修 Work Item"],
    ["work_item_plan", "正在返修 Work Item Plan"],
  ])("rebuilds revision nodes as author chat entries for %s workspaces", (workspaceType, streamContent) => {
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
          streaming_content: streamContent,
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
        content: streamContent,
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

  it("rebuilds only the latest provider prompt event for a node", () => {
    const store = useWorkspaceStore.getState();
    const firstPrompt = "delta prompt";
    const secondPrompt = "full prompt ".repeat(200);

    store.setSessionState({
      session_id: "session_revision_prompt_replace",
      workspace_type: "design",
      stage: "revision",
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "codex", reviewer: "claude_code" },
      timeline_nodes: [
        {
          node_id: "timeline_node_revision",
          node_type: "revision",
          agent: "codex",
          stage: "revision",
          round: 1,
          status: "failed",
          title: "返修 Round 1",
          summary: "运行已中止",
          started_at: "2026-05-26T10:00:00Z",
          completed_at: "2026-05-26T10:01:00Z",
          duration_ms: 60_000,
          artifact_ref: null,
          provider_config_snapshot: {
            author: "codex",
            reviewer: "claude_code",
            review_rounds: 1,
          },
        },
      ],
      active_node_id: "timeline_node_revision",
      artifact_versions: [],
      timeline_node_details: {
        timeline_node_revision: makeNodeDetail({
          node_id: "timeline_node_revision",
          node_type: "revision",
          agent_role: "author",
          provider: { name: "codex", model: "codex" },
          prompt: secondPrompt,
          execution_events: [
            {
              event_id: "revision_prompt_delta",
              node_id: "timeline_node_revision",
              agent: "codex",
              kind: "output",
              status: "started",
              title: "Provider Prompt",
              detail: "发送给 Workspace provider 的完整提示词",
              command: null,
              cwd: null,
              output: firstPrompt,
              exit_code: null,
            },
            {
              event_id: "revision_prompt_full",
              node_id: "timeline_node_revision",
              agent: "codex",
              kind: "output",
              status: "started",
              title: "Provider Prompt",
              detail: "发送给 Workspace provider 的完整提示词",
              command: null,
              cwd: null,
              output: secondPrompt,
              exit_code: null,
            },
          ],
        }),
      },
      active_run_id: null,
    });
    store.rebuildChatEntries();

    const promptEntries = useWorkspaceStore
      .getState()
      .chatEntries.filter(
        (entry) => entry.type === "execution_event" && entry.content.includes("Provider Prompt"),
      );
    expect(promptEntries).toHaveLength(1);
    expect(promptEntries[0]).toMatchObject({
      id: "timeline_node_revision:provider-prompt",
      content_size: secondPrompt.length,
      metadata: expect.objectContaining({ event_id: "revision_prompt_full" }),
    });
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
      .chatEntries.find((entry) => entry.id === "timeline_node_001:provider-prompt");
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

  it("sets workItemPlanCandidate from a session state with candidate artifact", () => {
    const store = useWorkspaceStore.getState();
    const candidate = makeWorkItemPlanCandidate();

    store.setSessionState({
      session_id: "session_candidate",
      workspace_type: "work_item_plan",
      stage: "author_confirm",
      messages: [],
      checkpoints: [],
      artifact: { candidate },
      providers: { author: "claude_code", reviewer: null },
    });

    expect(useWorkspaceStore.getState().workItemPlanCandidate).toEqual(candidate);
    expect(useWorkspaceStore.getState().artifact).toBeNull();
  });

  it("sets markdown artifact from a session state with markdown artifact", () => {
    const store = useWorkspaceStore.getState();

    store.setSessionState({
      session_id: "session_markdown",
      workspace_type: "story",
      stage: "author_confirm",
      messages: [],
      checkpoints: [],
      artifact: { markdown: "# Story", diff: null },
      providers: { author: "claude_code", reviewer: null },
    });

    expect(useWorkspaceStore.getState().artifact).toBe("# Story");
    expect(useWorkspaceStore.getState().workItemPlanCandidate).toBeNull();
  });

  it("supports legacy string artifact in session state", () => {
    const store = useWorkspaceStore.getState();

    store.setSessionState({
      session_id: "session_legacy",
      workspace_type: "story",
      stage: "author_confirm",
      messages: [],
      checkpoints: [],
      artifact: "# Legacy Story",
      providers: { author: "claude_code", reviewer: null },
    });

    expect(useWorkspaceStore.getState().artifact).toBe("# Legacy Story");
    expect(useWorkspaceStore.getState().workItemPlanCandidate).toBeNull();
  });

  it("updates workItemPlanCandidate via setWorkItemPlanCandidate", () => {
    const store = useWorkspaceStore.getState();
    const candidate = makeWorkItemPlanCandidate();

    store.setWorkItemPlanCandidate(candidate);

    expect(useWorkspaceStore.getState().workItemPlanCandidate).toEqual(candidate);
  });

  it("stores work item plan outline payload from session state", () => {
    const store = useWorkspaceStore.getState();
    const outlineCandidate = makeOutlineArtifactPayload();

    store.setSessionState({
      session_id: "session_outline_artifact",
      workspace_type: "work_item_plan",
      stage: "author_confirm",
      messages: [],
      checkpoints: [],
      artifact: { outline_candidate: outlineCandidate } as never,
      providers: { author: "claude_code", reviewer: null },
    });

    expect((useWorkspaceStore.getState() as never as { workItemPlanArtifact: unknown }).workItemPlanArtifact).toEqual({
      type: "outline_candidate",
      payload: outlineCandidate,
    });
    expect(useWorkspaceStore.getState().workItemPlanCandidate).toBeNull();
    expect(useWorkspaceStore.getState().artifact).toBeNull();
  });

  it("stores draft payload without clearing artifact history", () => {
    const store = useWorkspaceStore.getState();
    const draftCandidate = makeDraftArtifactPayload();

    store.setSessionState({
      session_id: "session_draft_artifact",
      workspace_type: "work_item_plan",
      stage: "author_confirm",
      messages: [],
      checkpoints: [],
      artifact: { draft_candidate: draftCandidate } as never,
      providers: { author: "claude_code", reviewer: null },
      artifact_version_summaries: [
        {
          version: 1,
          generated_by: "claude_code",
          reviewed_by: null,
          review_verdict: null,
          confirmed_by: null,
          is_current: true,
          created_at: "2026-06-23T00:00:00Z",
          source_node_id: "node_draft",
        },
      ],
    });

    expect((useWorkspaceStore.getState() as never as { workItemPlanArtifact: unknown }).workItemPlanArtifact).toEqual({
      type: "draft_candidate",
      payload: draftCandidate,
    });
    expect(useWorkspaceStore.getState().artifactVersions).toHaveLength(1);
  });

  it("stores compile report payload from session state", () => {
    const store = useWorkspaceStore.getState();
    const compileReport = makeCompileArtifactPayload();

    store.setSessionState({
      session_id: "session_compile_artifact",
      workspace_type: "work_item_plan",
      stage: "human_confirm",
      messages: [],
      checkpoints: [],
      artifact: { compile_report: compileReport } as never,
      providers: { author: "claude_code", reviewer: null },
    });

    expect((useWorkspaceStore.getState() as never as { workItemPlanArtifact: unknown }).workItemPlanArtifact).toEqual({
      type: "compile_report",
      payload: compileReport,
    });
    expect(useWorkspaceStore.getState().artifact).toBeNull();
  });

  it("uses work item plan context blocker summary for the human confirm gate prompt", () => {
    const contextBlocker = makeContextBlockerArtifactPayload();

    useWorkspaceStore.getState().setSessionState({
      session_id: "session_outline_blocker",
      workspace_type: "work_item_plan",
      stage: "human_confirm",
      messages: [],
      checkpoints: [],
      artifact: { context_blocker: contextBlocker } as never,
      providers: { author: "claude_code", reviewer: null },
      active_node_id: "node_context_blocker",
      timeline_nodes: [
        {
          node_id: "node_context_blocker",
          node_type: "work_item_plan_context_blocker",
          agent: null,
          stage: "human_confirm",
          round: null,
          status: "active",
          title: "WorkItemPlan 上下文确认",
          summary: "Outline 校验失败，请终止后重新创建 Work Item Plan",
          started_at: "2026-06-23T00:00:00Z",
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
    });

    const gatePrompt = useWorkspaceStore
      .getState()
      .chatEntries.find((entry) => entry.type === "gate_prompt");
    expect(gatePrompt).toMatchObject({
      content: contextBlocker.exploration_summary,
    });
  });

  it("tracks typed work item plan artifact versions", () => {
    const store = useWorkspaceStore.getState();
    const outlineCandidate = makeOutlineArtifactPayload();

    store.setWorkItemPlanArtifact(
      {
        type: "outline_candidate",
        payload: outlineCandidate,
      },
      7,
    );

    expect(
      (useWorkspaceStore.getState() as never as { workItemPlanArtifactVersions: unknown[] })
        .workItemPlanArtifactVersions,
    ).toEqual([
      expect.objectContaining({
        version: 7,
        artifact: {
          type: "outline_candidate",
          payload: outlineCandidate,
        },
      }),
    ]);
  });

  it("keeps unknown work item plan node types available for fallback rendering", () => {
    const store = useWorkspaceStore.getState();

    store.setSessionState({
      session_id: "session_unknown_node",
      workspace_type: "work_item_plan",
      stage: "human_confirm",
      messages: [],
      checkpoints: [],
      artifact: null,
      providers: { author: "claude_code", reviewer: null },
      active_node_id: "node_future",
      timeline_nodes: [
        {
          node_id: "node_future",
          node_type: "work_item_plan_future_phase",
          agent: null,
          stage: "human_confirm",
          round: null,
          status: "active",
          title: "Future phase",
          summary: null,
          started_at: "2026-06-23T00:00:00Z",
          completed_at: null,
          duration_ms: null,
          artifact_ref: null,
          provider_config_snapshot: {
            author: "claude_code",
            reviewer: null,
            review_rounds: 0,
          },
        } as never,
      ],
    });

    expect(useWorkspaceStore.getState().activeNodeId).toBe("node_future");
    expect(useWorkspaceStore.getState().timelineNodes[0]?.node_type).toBe(
      "work_item_plan_future_phase",
    );
    expect(useWorkspaceStore.getState().nodeDetails.node_future?.node_type).toBe(
      "work_item_plan_future_phase",
    );
  });
});
