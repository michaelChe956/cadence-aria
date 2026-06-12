import { beforeEach, describe, expect, it } from "vitest";
import type {
  AnalystDecisionRecord,
  CodeReviewReport,
  CodingGateRequired,
  CodingTimelineNode,
  CodingWsOutMessage,
  TestingReport,
} from "../api/types";
import { useCodingWorkspaceStore } from "./coding-workspace-store";

const providerConfig = {
  author: "fake" as const,
  reviewer: "fake" as const,
  review_rounds: 1,
};

const roleProviderConfig = {
  coder: "fake" as const,
  tester: "fake" as const,
  analyst: "fake" as const,
  code_reviewer: "fake" as const,
  internal_reviewer: "fake" as const,
  review_rounds: 1,
  permission_modes: {
    coder: "supervised" as const,
    tester: "auto" as const,
    analyst: "auto" as const,
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

function testingReport(overrides: Partial<TestingReport> = {}): TestingReport {
  return {
    id: "testing_report_0001",
    attempt_id: "coding_attempt_0001",
    commands: [],
    overall_status: "passed_with_warnings",
    provider_claim: null,
    backend_verified: true,
    started_at: "2026-06-10T00:00:00Z",
    completed_at: "2026-06-10T00:00:01Z",
    plan_id: "test_plan_0001",
    plan_summary: "API smoke and security review",
    steps: [
      {
        step_id: "api_smoke",
        status: "passed",
        evidence_refs: ["stdout.log"],
        command: ["cargo", "test", "--locked", "--lib", "api_smoke"],
        provider_analysis: "API smoke passed",
      },
    ],
    unplanned_commands: [],
    missing_required_steps: ["security"],
    skipped_required_steps: [],
    context_warnings: ["missing_design_spec"],
    raw_provider_output_ref: "provider-raw/testing/execute_test_plan_0001.txt",
    ...overrides,
  };
}

function analystDecision(
  overrides: Partial<AnalystDecisionRecord> = {},
): AnalystDecisionRecord {
  return {
    id: "analyst_decision_0001",
    attempt_id: "coding_attempt_0001",
    source_stage: "testing",
    rework_round: 1,
    verdict: "needs_fix",
    next_stage: "coding",
    reason: "required 测试步骤被跳过，需要回到 Coder",
    evidence_refs: ["testing_report_0001.json"],
    raw_provider_output_refs: ["provider-raw/testing/execute_test_plan_0001.txt"],
    rework_instructions: null,
    human_gate: null,
    created_at: "2026-06-12T00:00:00Z",
    parse_error: null,
    ...overrides,
  };
}

function blockedGate(overrides: Partial<CodingGateRequired> = {}): CodingGateRequired {
  return {
    gate_id: "gate_0001",
    kind: "blocked",
    title: "Testing blocked",
    description: "Required step missing",
    stage: "testing",
    role: "tester",
    reason_code: "missing_required_test_step",
    evidence_refs: ["stdout.log"],
    raw_provider_output_ref: "provider-raw/testing/execute_test_plan_0001.txt",
    available_actions: [
      {
        action_id: "rerun_missing_steps",
        label: "重新执行缺失步骤",
        action_type: "rerun_missing_steps",
      },
    ],
    ...overrides,
  };
}

function sessionState(
  overrides: Partial<Extract<CodingWsOutMessage, { type: "coding_session_state" }>> = {},
): Extract<CodingWsOutMessage, { type: "coding_session_state" }> {
  return {
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
    role_provider_config_snapshot: roleProviderConfig,
    provider_config_snapshot: providerConfig,
    timeline_nodes: [codingNode()],
    active_node_id: "coding_node_0001",
    testing_report: null,
    code_review_reports: [],
    review_request: null,
    internal_pr_review: null,
    pending_gates: [],
    latest_analyst_decision: null,
    chat_entries: [],
    ...overrides,
  };
}

describe("coding workspace store", () => {
  beforeEach(() => {
    useCodingWorkspaceStore.getState().reset();
  });

  it("initializes attempt state from a websocket session snapshot", () => {
    const store = useCodingWorkspaceStore.getState();

    store.setSessionState(sessionState({ code_review_reports: [codeReview()] }));

    const state = useCodingWorkspaceStore.getState();
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

  it("hydrates persisted coding chat entries from a websocket session snapshot", () => {
    const store = useCodingWorkspaceStore.getState();

    store.setSessionState(
      sessionState({
        chat_entries: [
          {
            id: "coding_chat_entry_analyst_0001",
            attempt_id: "coding_attempt_0001",
            node_id: "coding_node_0002",
            role: "system",
            entry_type: { type: "analyst_verdict", verdict: "no_issue" },
            content: "测试阶段无问题",
            metadata: { source: "rework" },
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
        id: "coding_chat_entry_analyst_0001",
        type: "analyst_verdict",
        role: "analyst",
        content: "测试阶段无问题",
        timestamp: "2026-05-28T00:00:01Z",
        node_id: "coding_node_0002",
        metadata: { source: "rework", verdict: "no_issue" },
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

  it("stores plan based testing reports and blocked gate metadata", () => {
    const store = useCodingWorkspaceStore.getState();

    store.setSessionState(
      sessionState({
        testing_report: testingReport(),
        pending_gates: [blockedGate()],
      }),
    );

    expect(useCodingWorkspaceStore.getState().testingReport).toMatchObject({
      plan_summary: "API smoke and security review",
      missing_required_steps: ["security"],
      raw_provider_output_ref: "provider-raw/testing/execute_test_plan_0001.txt",
    });
    expect(useCodingWorkspaceStore.getState().testingReport?.steps?.[0]).toMatchObject({
      step_id: "api_smoke",
      evidence_refs: ["stdout.log"],
    });
    expect(useCodingWorkspaceStore.getState().pendingGates).toMatchObject([
      {
        gate_id: "gate_0001",
        reason_code: "missing_required_test_step",
        evidence_refs: ["stdout.log"],
        raw_provider_output_ref: "provider-raw/testing/execute_test_plan_0001.txt",
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

  it("stores latest analyst decision from session snapshots", () => {
    const store = useCodingWorkspaceStore.getState();

    store.setSessionState(
      sessionState({
        testing_report: testingReport(),
        latest_analyst_decision: analystDecision({
          reason: "测试阻塞已归因，要求 Coder 补齐浏览器步骤",
        }),
      }),
    );

    expect(useCodingWorkspaceStore.getState().latestAnalystDecision).toMatchObject({
      id: "analyst_decision_0001",
      source_stage: "testing",
      verdict: "needs_fix",
      next_stage: "coding",
      reason: "测试阻塞已归因，要求 Coder 补齐浏览器步骤",
    });

    store.setSessionState(sessionState({ latest_analyst_decision: null }));

    expect(useCodingWorkspaceStore.getState().latestAnalystDecision).toBeNull();
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
    store.addTimelineNode(codingNode({ id: "coding_node_0001", stage: "testing" }));

    store.appendStreamChunk("hello", "coding_node_0001");
    store.appendStreamChunk(" world", "coding_node_0001");

    expect(useCodingWorkspaceStore.getState().streamingContent).toBe("hello world");
    expect(useCodingWorkspaceStore.getState().chatEntries).toMatchObject([
      {
        type: "provider_stream",
        role: "tester",
        content: "hello world",
        node_id: "coding_node_0001",
      },
    ]);

    store.completeStream("coding_node_0001");

    expect(useCodingWorkspaceStore.getState().streamingContent).toBeNull();
    expect(useCodingWorkspaceStore.getState().activeStreamNodeId).toBeNull();
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
          codingNode({ id: "coding_node_0002", stage: "testing" }),
        ],
        active_node_id: null,
      }),
    );

    store.setSelectedNode("coding_node_0002");

    expect(useCodingWorkspaceStore.getState().activeTab).toBe("tests");

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
          codingNode({ id: "coding_node_0002", stage: "testing" }),
        ],
        active_node_id: "coding_node_0002",
      }),
    );

    expect(useCodingWorkspaceStore.getState().selectedNodeId).toBe("coding_node_0002");
    expect(useCodingWorkspaceStore.getState().activeTab).toBe("tests");

    useCodingWorkspaceStore.getState().setActiveTab("logs");
    useCodingWorkspaceStore.getState().setSessionState(
      sessionState({
        timeline_nodes: [
          codingNode({ id: "coding_node_0001", stage: "coding" }),
          codingNode({ id: "coding_node_0002", stage: "testing" }),
        ],
        active_node_id: "coding_node_0002",
      }),
    );

    expect(useCodingWorkspaceStore.getState().activeTab).toBe("logs");
  });
});
