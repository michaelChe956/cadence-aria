import { beforeEach, describe, expect, it } from "vitest";
import type {
  CodeReviewReport,
  CodingGateRequired,
  CodingRoleRun,
  CodingTimelineNode,
  CodingWsOutMessage,
  GroupFinalReadinessSnapshot,
  WorkItemExecutionPlan,
} from "../api/types";
import { useCodingWorkspaceStore } from "./coding-workspace-store";

const providerConfig = {
  author: "fake" as const,
  reviewer: "fake" as const,
  review_rounds: 1,
};

const roleProviderConfig = {
  coder: "fake" as const,
  code_reviewer: "fake" as const,
  internal_reviewer: "fake" as const,
  review_rounds: 1,
  permission_modes: {
    coder: "supervised" as const,
    code_reviewer: "supervised" as const,
    internal_reviewer: "supervised" as const,
  },
};

function codingNode(overrides: Partial<CodingTimelineNode> = {}): CodingTimelineNode {
  return {
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
    ...overrides,
  };
}

function codeReview(overrides: Partial<CodeReviewReport> = {}): CodeReviewReport {
  return {
    id: "code_review_0001",
    attempt_id: "coding_attempt_0001",
    round: 1,
    verdict: "approve",
    findings: [],
    tested_evidence_refs: [],
    diff_refs: [],
    summary: "review ok",
    created_at: "2026-05-23T00:01:00Z",
    ...overrides,
  };
}

function blockedGate(overrides: Partial<CodingGateRequired> = {}): CodingGateRequired {
  return {
    gate_id: "gate_0001",
    kind: "blocked",
    title: "Review blocked",
    description: "Review payload invalid",
    stage: "code_review",
    role: "code_reviewer",
    reason_code: "review_payload_parse_error",
    evidence_refs: ["code-review.json"],
    raw_provider_output_ref: "provider-raw/code-review/review_0001.txt",
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

function roleRun(overrides: Partial<CodingRoleRun> = {}): CodingRoleRun {
  return {
    id: "coding_role_run_0001",
    attempt_id: "coding_attempt_0001",
    stage: "code_review",
    role: "code_reviewer",
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
    event_summary: {
      event_count: 2,
      last_event_at: "2026-06-13T00:00:02Z",
      last_event_type: "execution_event",
      last_event_title: "Task update",
      last_event_status: "running",
      terminal_event_type: null,
      terminal_reason: null,
    },
    recent_events: [
      {
        sequence: 2,
        event_type: "execution_event",
        created_at: "2026-06-13T00:00:02Z",
        title: "Task update",
        status: "running",
        detail: "No tasks found",
        truncated: false,
        artifact_ref: null,
      },
    ],
    ...overrides,
  };
}

function completeGroupFinalReadiness(): GroupFinalReadinessSnapshot {
  return {
    attempt_id: "coding_attempt_0001",
    status: "complete",
    units: [
      {
        unit_id: "coding_unit_0001",
        logical_work_item_id: "work_item_0001",
        unit_run_id: "coding_unit_run_0001",
        start_commit: "BASE",
        completion_commit: "C2",
        commit_shas: ["C1", "C2"],
        diff_ref: "diffs/coding_unit_0001.patch",
        empty_observation: false,
        code_review_report_id: "code_review_0001",
        review_verdict: "approve",
        review_summary: "review ok",
        review_findings: [
          {
            severity: "info",
            file_path: "src/web/types.rs",
            line: 545,
            message: "reviewed",
            required_action: null,
            source_stage: "code_review",
          },
        ],
        review_raw_provider_output_ref: "provider-raw/code-review.txt",
        handoff_revision_id: "handoff_revision_0001",
        plan_revision_id: "plan_revision_0001",
      },
    ],
    diagnostics: [],
    created_at: "2026-08-07T00:00:00Z",
  };
}

function incompleteGroupFinalReadiness(): GroupFinalReadinessSnapshot {
  return {
    attempt_id: "coding_attempt_0001",
    status: "incomplete",
    units: [],
    diagnostics: [
      {
        kind: "code_review_missing",
        unit_id: "coding_unit_0001",
        message: "unit review is missing",
      },
    ],
    created_at: "2026-08-07T00:00:00Z",
  };
}

function sessionState(
  overrides: Partial<Extract<CodingWsOutMessage, { type: "coding_session_state" }>> = {},
): Extract<CodingWsOutMessage, { type: "coding_session_state" }> {
  return {
    type: "coding_session_state",
    project_id: "project_0001",
    issue_id: "issue_0001",
    attempt_id: "coding_attempt_0001",
    attempt_scope: "work_item",
    work_item_group_id: null,
    current_work_item_id: "work_item_0001",
    active_unit_id: null,
    units: [],
    status: "running",
    stage: "coding",
    branch_name: "aria/work-items/work_item_0001/attempt-1",
    base_branch: "main",
    worktree_path: "/tmp/worktree",
    rework_count: 0,
    max_auto_rework: 2,
    head_commit: null,
    pushed_remote: null,
    role_provider_config_snapshot: roleProviderConfig,
    provider_config_snapshot: providerConfig,
    timeline_nodes: [codingNode()],
    active_node_id: "coding_node_0001",
    code_review_reports: [],
    review_request: null,
    internal_pr_review: null,
    pending_gates: [],
    pending_choices: [],
    chat_entries: [],
    work_item_markdown: null,
    verification_commands: [],
    work_item_execution_plan: null,
    linked_plan_repair: null,
    require_execution_plan_confirm: false,
    ...overrides,
  };
}

function codingSessionState(
  overrides: Partial<Extract<CodingWsOutMessage, { type: "coding_session_state" }>> = {},
): Extract<CodingWsOutMessage, { type: "coding_session_state" }> {
  return sessionState(overrides);
}

function executionPlan(): WorkItemExecutionPlan {
  return {
    id: "work_item_execution_plan_0001",
    project_id: "project_0001",
    issue_id: "issue_0001",
    work_item_id: "work_item_0001",
    attempt_id: "coding_attempt_0001",
    status: "draft",
    goal: "实现后端 API",
    allowed_write_scopes: ["src/product/**"],
    forbidden_write_scopes: ["web/**"],
    dependency_handoffs: [],
    story_refs: ["story_spec_0001"],
    design_refs: ["design_spec_0001"],
    openspec_refs: ["REQ-001"],
    superpowers_contract: "use superpowers:test-driven-development",
    tdd_contract: "先写失败测试，再写实现",
    verification_plan_ref: "verification_plan_work_item_0001",
    verification_summary: "provider supplied required gate verify_backend_unit",
    risk_notes: [],
    created_at: "2026-06-16T00:00:00Z",
    updated_at: "2026-06-16T00:00:00Z",
  };
}

describe("coding workspace store", () => {
  beforeEach(() => {
    useCodingWorkspaceStore.getState().reset();
  });

  it("hydrates complete group final readiness with review findings and ordered commits", () => {
    const store = useCodingWorkspaceStore.getState();

    store.setSessionState(
      {
        ...sessionState(),
        group_final_readiness: completeGroupFinalReadiness(),
      } as Extract<CodingWsOutMessage, { type: "coding_session_state" }>,
    );

    expect(useCodingWorkspaceStore.getState().groupFinalReadiness?.units[0].commit_shas).toEqual([
      "C1",
      "C2",
    ]);
    expect(useCodingWorkspaceStore.getState().groupFinalReadiness?.units[0].review_findings).toHaveLength(
      1,
    );
  });

  it("retains incomplete diagnostics from a session-state update", () => {
    const store = useCodingWorkspaceStore.getState();

    store.setSessionState(
      {
        ...sessionState(),
        group_final_readiness: incompleteGroupFinalReadiness(),
      } as Extract<CodingWsOutMessage, { type: "coding_session_state" }>,
    );

    expect(useCodingWorkspaceStore.getState().groupFinalReadiness?.status).toBe("incomplete");
    expect(useCodingWorkspaceStore.getState().groupFinalReadiness?.diagnostics).toEqual([
      {
        kind: "code_review_missing",
        unit_id: "coding_unit_0001",
        message: "unit review is missing",
      },
    ]);
  });

  it("initializes attempt state from a websocket session snapshot", () => {
    const store = useCodingWorkspaceStore.getState();

    store.setSessionState(sessionState({ code_review_reports: [codeReview()] }));

    const state = useCodingWorkspaceStore.getState();
    expect(state.projectId).toBe("project_0001");
    expect(state.issueId).toBe("issue_0001");
    expect(state.attemptId).toBe("coding_attempt_0001");
    expect(state.status).toBe("running");
    expect(state.stage).toBe("coding");
    expect(state.branchName).toBe("aria/work-items/work_item_0001/attempt-1");
    expect(state.providerConfigSnapshot).toEqual(providerConfig);
    expect(state.roleProviderConfigSnapshot).toEqual(roleProviderConfig);
    expect(state.timelineNodes).toHaveLength(1);
    expect(state.activeNodeId).toBe("coding_node_0001");
    expect(state.selectedNodeId).toBe("coding_node_0001");
    expect(state.codeReviewReports).toHaveLength(1);
  });

  it("restores group context from coding session snapshot", () => {
    const store = useCodingWorkspaceStore.getState();

    store.setSessionState({
      type: "coding_session_state",
      project_id: "project_0001",
      issue_id: "issue_0001",
      attempt_id: "coding_attempt_0001",
      attempt_scope: "work_item_group",
      work_item_group_id: "work_item_plan_0001",
      current_work_item_id: "work_item_0001",
      active_unit_id: "coding_unit_0001",
      units: [
        {
          unit_id: "coding_unit_0001",
          logical_work_item_id: "work_item_0001",
          work_item_revision_id: "work_item_revision_0001",
          dependency_logical_work_item_ids: [],
          order_index: 0,
          status: "running",
          summary: null,
          latest_handoff_revision_id: null,
          completion_commit: null,
        },
        {
          unit_id: "coding_unit_0002",
          logical_work_item_id: "work_item_0002",
          work_item_revision_id: "work_item_revision_0002",
          dependency_logical_work_item_ids: ["work_item_0001"],
          order_index: 1,
          status: "pending",
          summary: null,
          latest_handoff_revision_id: null,
          completion_commit: null,
        },
      ],
      status: "running",
      stage: "coding",
      branch_name: "aria/issues/issue_0001",
      base_branch: "main",
      worktree_path: null,
      rework_count: 0,
      max_auto_rework: 2,
      head_commit: null,
      pushed_remote: null,
      provider_config_snapshot: providerConfig,
      role_provider_config_snapshot: roleProviderConfig,
      chat_entries: [],
      timeline_nodes: [],
      active_node_id: null,
      code_review_reports: [],
      review_request: null,
      internal_pr_review: null,
      pending_gates: [],
      pending_choices: [],
      role_runs: [],
      work_item_markdown: null,
      verification_commands: [],
      work_item_execution_plan: null,
      linked_plan_repair: null,
      require_execution_plan_confirm: false,
    });

    expect(useCodingWorkspaceStore.getState().attemptScope).toBe("work_item_group");
    expect(useCodingWorkspaceStore.getState().units).toHaveLength(2);
    expect(useCodingWorkspaceStore.getState().currentWorkItemId).toBe("work_item_0001");
  });

  it("hydrates persisted coding chat entries from a websocket session snapshot", () => {
    const store = useCodingWorkspaceStore.getState();

    store.setSessionState(
      sessionState({
        chat_entries: [
          {
            id: "coding_chat_entry_coder_0001",
            attempt_id: "coding_attempt_0001",
            node_id: "coding_node_0002",
            role: "author",
            entry_type: { type: "assistant_message" },
            content: "Coder 已按 reviewer findings 修复",
            metadata: { source: "coding" },
            created_at: "2026-05-28T00:00:01Z",
          },
          {
            id: "coding_chat_entry_code_review_0001",
            attempt_id: "coding_attempt_0001",
            node_id: "coding_node_0003",
            role: "reviewer",
            entry_type: { type: "assistant_message" },
            content: "代码审查通过",
            metadata: { source: "code_review", verdict: "approve" },
            created_at: "2026-05-28T00:00:02Z",
          },
          {
            id: "coding_chat_entry_internal_0001",
            attempt_id: "coding_attempt_0001",
            node_id: "coding_node_0004",
            role: "reviewer",
            entry_type: { type: "assistant_message" },
            content: "PR 描述和影响范围完整",
            metadata: { source: "internal_pr_review", impact_scope: ["src/lib.rs"] },
            created_at: "2026-05-28T00:00:03Z",
          },
        ],
      }),
    );

    expect(useCodingWorkspaceStore.getState().chatEntries).toEqual([
      {
        id: "coding_chat_entry_coder_0001",
        type: "provider_stream",
        role: "coder",
        content: "Coder 已按 reviewer findings 修复",
        timestamp: "2026-05-28T00:00:01Z",
        node_id: "coding_node_0002",
        metadata: { source: "coding" },
      },
      {
        id: "coding_chat_entry_code_review_0001",
        type: "provider_stream",
        role: "code_reviewer",
        content: "代码审查通过",
        timestamp: "2026-05-28T00:00:02Z",
        node_id: "coding_node_0003",
        metadata: { source: "code_review", verdict: "approve" },
      },
      {
        id: "coding_chat_entry_internal_0001",
        type: "provider_stream",
        role: "internal_reviewer",
        content: "PR 描述和影响范围完整",
        timestamp: "2026-05-28T00:00:03Z",
        node_id: "coding_node_0004",
        metadata: { source: "internal_pr_review", impact_scope: ["src/lib.rs"] },
      },
    ]);
  });

  it("stores role runs from websocket session snapshots", () => {
    const store = useCodingWorkspaceStore.getState();
    store.setSessionState(sessionState({ role_runs: [roleRun()] }));

    expect(useCodingWorkspaceStore.getState().roleRuns).toHaveLength(1);
    expect(useCodingWorkspaceStore.getState().roleRuns[0]).toMatchObject({
      id: "coding_role_run_0001",
      role: "code_reviewer",
      run_no: 1,
    });
    expect(useCodingWorkspaceStore.getState().roleRuns[0].event_summary).toMatchObject({
      event_count: 2,
      last_event_title: "Task update",
    });
    expect(useCodingWorkspaceStore.getState().roleRuns[0].recent_events?.[0]).toMatchObject({
      detail: "No tasks found",
    });
  });

  it("keeps a failed automatic attempt when a later stream completion arrives", () => {
    const store = useCodingWorkspaceStore.getState();
    store.setSessionState(
      sessionState({
        role_runs: [
          roleRun({
            id: "coding_role_run_0002",
            status: "failed",
            trigger: "automatic_retry",
            reason_code: "provider_503",
            retry_metadata: {
              cycle_id: "provider_retry_cycle_0001",
              attempt_no: 2,
              prior_run_id: "coding_role_run_0001",
            },
          }),
        ],
      }),
    );

    store.replacePendingEntry({
      id: "coding_chat_entry_0002",
      type: "provider_stream",
      role: "code_reviewer",
      content: "迟到的 provider completion",
      timestamp: "2026-06-12T00:01:00Z",
      node_id: "coding_node_0003",
      metadata: { role_run_id: "coding_role_run_0002" },
    });

    expect(useCodingWorkspaceStore.getState().roleRuns[0]).toMatchObject({
      status: "failed",
      trigger: "automatic_retry",
      retry_metadata: { attempt_no: 2 },
    });
  });

  it("marks automatic retries exhausted only when their blocked gate is present", () => {
    const store = useCodingWorkspaceStore.getState();
    const thirdTransportFailure = roleRun({
      status: "failed",
      trigger: "automatic_retry",
      retry_metadata: {
        cycle_id: "provider_retry_cycle_0001",
        attempt_no: 3,
        prior_run_id: "coding_role_run_0002",
      },
      reason_code: "provider_503",
    });

    store.setSessionState(sessionState({ role_runs: [thirdTransportFailure] }));

    expect(useCodingWorkspaceStore.getState().roleRuns[0].retry_exhausted).not.toBe(true);

    store.addPendingGate(blockedGate({ reason_code: "code_review_provider_interrupted" }));

    expect(useCodingWorkspaceStore.getState().roleRuns[0].retry_exhausted).toBe(true);

    store.resolvePendingGate("gate_0001");

    expect(useCodingWorkspaceStore.getState().roleRuns[0].retry_exhausted).not.toBe(true);

    store.setSessionState(
      sessionState({
        role_runs: [thirdTransportFailure],
        pending_gates: [blockedGate({ reason_code: "code_review_provider_interrupted" })],
      }),
    );

    expect(useCodingWorkspaceStore.getState().roleRuns[0].retry_exhausted).toBe(true);

    store.setSessionState(
      sessionState({
        role_runs: [roleRun({ ...thirdTransportFailure, reason_code: "review_payload_parse_error" })],
        pending_gates: [blockedGate({ reason_code: "code_review_provider_interrupted" })],
      }),
    );

    expect(useCodingWorkspaceStore.getState().roleRuns[0].retry_exhausted).not.toBe(true);
  });

  it("adds and updates timeline nodes while clearing inactive active node", () => {
    const store = useCodingWorkspaceStore.getState();
    store.addTimelineNode(codingNode());

    store.updateTimelineNode("coding_node_0001", "completed", "代码编写完成", "2026-05-23T00:02:00Z");

    const state = useCodingWorkspaceStore.getState();
    expect(state.timelineNodes[0]).toMatchObject({
      status: "completed",
      summary: "代码编写完成",
      completed_at: "2026-05-23T00:02:00Z",
    });
    expect(state.activeNodeId).toBeNull();
  });

  it("deduplicates review reports and stores gate state", () => {
    const store = useCodingWorkspaceStore.getState();
    store.addCodeReviewReport(codeReview({ summary: "old" }));
    store.addCodeReviewReport(codeReview({ summary: "updated" }));
    store.addPendingGate({
      gate_id: "gate_0001",
      kind: "blocked",
      title: "需要人工决策",
      description: "测试失败次数达到上限",
      available_actions: [
        {
          action_id: "accept_risk",
          label: "接受风险",
          action_type: "accept_risk",
        },
      ],
    });

    expect(useCodingWorkspaceStore.getState().codeReviewReports).toEqual([
      codeReview({ summary: "updated" }),
    ]);
    expect(useCodingWorkspaceStore.getState().pendingGates).toHaveLength(1);

    store.resolvePendingGate("gate_0001");

    expect(useCodingWorkspaceStore.getState().pendingGates).toHaveLength(0);
  });

  it("stores blocked gate metadata", () => {
    const store = useCodingWorkspaceStore.getState();

    store.setSessionState(
      sessionState({
        pending_gates: [blockedGate()],
      }),
    );

    expect(useCodingWorkspaceStore.getState().pendingGates).toMatchObject([
      {
        gate_id: "gate_0001",
        reason_code: "review_payload_parse_error",
        evidence_refs: ["code-review.json"],
        raw_provider_output_ref: "provider-raw/code-review/review_0001.txt",
      },
    ]);

    store.addPendingGate(
      blockedGate({
        title: "Updated gate",
        reason_code: "review_payload_parse_error",
      }),
    );

    expect(useCodingWorkspaceStore.getState().pendingGates).toMatchObject([
      {
        gate_id: "gate_0001",
        title: "Updated gate",
        reason_code: "review_payload_parse_error",
      },
    ]);

    store.resolvePendingGate("gate_0001");

    expect(useCodingWorkspaceStore.getState().pendingGates).toHaveLength(0);
  });

  it("tracks gate submission without removing gate until snapshot confirms", () => {
    const store = useCodingWorkspaceStore.getState();
    store.addPendingGate(blockedGate());

    store.markGateSubmitting("gate_0001");

    expect(useCodingWorkspaceStore.getState().pendingGates).toMatchObject([
      {
        gate_id: "gate_0001",
        submitting: true,
        errorCode: null,
      },
    ]);

    store.setGateError("gate_0001", "coding_gate_response_failed");

    expect(useCodingWorkspaceStore.getState().pendingGates).toMatchObject([
      {
        gate_id: "gate_0001",
        submitting: false,
        errorCode: "coding_gate_response_failed",
      },
    ]);

    store.markGateSubmitting("gate_0001");
    store.setSessionState(sessionState({ pending_gates: [] }));

    expect(useCodingWorkspaceStore.getState().pendingGates).toHaveLength(0);
  });

  it("tracks provider streaming content as chat entries", () => {
    const store = useCodingWorkspaceStore.getState();
    store.addTimelineNode(codingNode({ id: "coding_node_0001", stage: "code_review" }));

    store.appendStreamChunk("hello", "coding_node_0001");
    store.appendStreamChunk(" world", "coding_node_0001");

    expect(useCodingWorkspaceStore.getState().streamingContent).toBe("hello world");
    expect(useCodingWorkspaceStore.getState().chatEntries).toMatchObject([
      {
        type: "provider_stream",
        role: "code_reviewer",
        content: "hello world",
        node_id: "coding_node_0001",
      },
    ]);

    store.completeStream("coding_node_0001");

    expect(useCodingWorkspaceStore.getState().streamingContent).toBeNull();
    expect(useCodingWorkspaceStore.getState().activeStreamNodeId).toBeNull();
  });

  it("uses coder role for reviewer-driven coder retry stream entries", () => {
    const store = useCodingWorkspaceStore.getState();
    store.addTimelineNode(
      codingNode({
        id: "coding_node_0004",
        stage: "coding",
        title: "代码编写",
        agent_role: "author",
      }),
    );

    store.appendStreamChunk("fixed reviewer findings", "coding_node_0004");

    expect(useCodingWorkspaceStore.getState().chatEntries).toMatchObject([
      {
        type: "provider_stream",
        role: "coder",
        content: "fixed reviewer findings",
        node_id: "coding_node_0004",
      },
    ]);
  });

  it("uses concrete commands for coding execution event chat titles and logs", () => {
    const store = useCodingWorkspaceStore.getState();

    store.addExecutionEvent({
      event_id: "command_cmd_001",
      node_id: "coding_node_0001",
      agent: "codex",
      kind: "command",
      status: "completed",
      title: "Command completed",
      detail: "exit code 0",
      command: "git diff --stat",
      cwd: "/tmp/repo",
      output: "ok\n",
      exit_code: 0,
    });

    expect(useCodingWorkspaceStore.getState().logs).toMatchObject([
      {
        id: "command_cmd_001",
        message: "git diff --stat",
      },
    ]);
    expect(useCodingWorkspaceStore.getState().chatEntries).toMatchObject([
      {
        id: "command_cmd_001",
        type: "execution_event",
        content: "git diff --stat",
        metadata: {
          title: "Command completed",
          command: "git diff --stat",
          output: "ok\n",
        },
      },
    ]);
  });

  it("labels provider prompt events with the current coding node title", () => {
    const store = useCodingWorkspaceStore.getState();
    store.addTimelineNode(codingNode({ id: "coding_node_0001", title: "代码编写" }));

    store.addExecutionEvent({
      event_id: "coding_node_0001_prompt",
      node_id: "coding_node_0001",
      agent: "codex",
      kind: "output",
      status: "started",
      title: "Provider Prompt",
      detail: "发送给 Coding provider 的完整提示词",
      command: null,
      cwd: null,
      output: "Coding Workspace\n请实现 climb_stairs",
      exit_code: null,
    });

    expect(useCodingWorkspaceStore.getState().chatEntries).toMatchObject([
      {
        id: "coding_node_0001_prompt",
        type: "execution_event",
        role: "coder",
        content: "代码编写 · Provider Prompt",
        node_id: "coding_node_0001",
        metadata: {
          output: "Coding Workspace\n请实现 climb_stairs",
        },
      },
    ]);
  });

  it("appends optimistic context notes and replaces them with backend chat entries", () => {
    const store = useCodingWorkspaceStore.getState();

    store.appendChatEntry({
      id: "pending_context_note_0001",
      type: "context_note",
      role: "user",
      content: "请覆盖空输入边界",
      timestamp: "2026-05-28T00:00:00Z",
      metadata: { pending: true },
    });
    store.replacePendingEntry({
      id: "coding_chat_entry_0001",
      type: "context_note",
      role: "user",
      content: "请覆盖空输入边界",
      timestamp: "2026-05-28T00:00:01Z",
      metadata: { context_note_id: "coding_context_note_0001" },
    });

    expect(useCodingWorkspaceStore.getState().chatEntries).toEqual([
      {
        id: "coding_chat_entry_0001",
        type: "context_note",
        role: "user",
        content: "请覆盖空输入边界",
        timestamp: "2026-05-28T00:00:01Z",
        metadata: { context_note_id: "coding_context_note_0001" },
      },
    ]);
  });

  it("switches artifact tab when selecting timeline nodes until the user locks the tab", () => {
    const store = useCodingWorkspaceStore.getState();
    store.setSessionState(
      sessionState({
        timeline_nodes: [
          codingNode({ id: "coding_node_0001", stage: "coding" }),
          codingNode({ id: "coding_node_0002", stage: "code_review" }),
        ],
        active_node_id: null,
      }),
    );

    store.setSelectedNode("coding_node_0002");

    expect(useCodingWorkspaceStore.getState().activeTab).toBe("review");

    useCodingWorkspaceStore.getState().setActiveTab("logs");
    useCodingWorkspaceStore.getState().setSelectedNode("coding_node_0001");

    expect(useCodingWorkspaceStore.getState().activeTab).toBe("logs");
  });

  it("syncs the artifact tab from the selected snapshot node unless the user locked it", () => {
    const store = useCodingWorkspaceStore.getState();
    store.setSessionState(
      sessionState({
        timeline_nodes: [
          codingNode({ id: "coding_node_0001", stage: "coding" }),
          codingNode({ id: "coding_node_0002", stage: "code_review" }),
        ],
        active_node_id: "coding_node_0002",
      }),
    );

    expect(useCodingWorkspaceStore.getState().selectedNodeId).toBe("coding_node_0002");
    expect(useCodingWorkspaceStore.getState().activeTab).toBe("review");

    useCodingWorkspaceStore.getState().setActiveTab("logs");
    useCodingWorkspaceStore.getState().setSessionState(
      sessionState({
        timeline_nodes: [
          codingNode({ id: "coding_node_0001", stage: "coding" }),
          codingNode({ id: "coding_node_0002", stage: "code_review" }),
        ],
        active_node_id: "coding_node_0002",
      }),
    );

    expect(useCodingWorkspaceStore.getState().activeTab).toBe("logs");
  });

  it("stores work item execution plan from coding session state", () => {
    const store = useCodingWorkspaceStore.getState();
    store.reset();

    // setSessionState 入参为 coding_session_state 消息：
    // Extract<CodingWsOutMessage, { type: "coding_session_state" }>
    store.setSessionState({
      ...codingSessionState(),
      work_item_execution_plan: executionPlan(),
    });

    expect(useCodingWorkspaceStore.getState().workItemExecutionPlan?.goal).toBe("实现后端 API");
  });
});
