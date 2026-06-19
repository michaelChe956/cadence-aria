import { describe, expect, it } from "vitest";
import type {
  AnalystDecisionRecord,
  CodeReviewReport,
  CodingGateRequired,
  CodingAttempt,
  CodingAttemptSnapshotResponse,
  CodingWsInMessage,
  CodingWsOutMessage,
  GenerateWorkItemsRequest,
  IssueLifecycleResponse,
  IssueWorkItemPlanDetailDto,
  InternalPrReview,
  LifecycleWorkItem,
  NodeDetail,
  PrepareWorkItemPlanRequest,
  PrepareWorkItemPlanResponse,
  TestingReport,
  TimelineNodeType,
  WorkItemExecutionPlan,
  WsInMessage,
  WsOutMessage,
} from "./types";

describe("workspace websocket protocol types", () => {
  it("accepts protocol v2 inbound messages", () => {
    const note: WsInMessage = { type: "context_note", content: "补充上下文" };
    const start: WsInMessage = {
      type: "start_generation",
      provider_config: { author: "claude_code", reviewer: "codex", review_rounds: 1 },
      reviewer_enabled: true,
    };
    const human: WsInMessage = {
      type: "human_confirm",
      decision: "request-change",
      payload: { description: "补充验收标准" },
    };

    expect(note.type).toBe("context_note");
    expect(start.type).toBe("start_generation");
    expect(human.decision).toBe("request-change");
  });

  it("accepts protocol v2 outbound messages", () => {
    const error: WsOutMessage = {
      type: "protocol_error",
      code: "INVALID_MESSAGE_FOR_STAGE",
      message: "context_note not allowed in running",
      context: { stage: "running" },
    };
    const locked: WsOutMessage = {
      type: "provider_locked",
      snapshot: { author: "claude_code", reviewer: "codex", review_rounds: 1 },
      locked_at: "2026-05-20T00:00:00Z",
    };

    expect(error.code).toBe("INVALID_MESSAGE_FOR_STAGE");
    expect(locked.snapshot.author).toBe("claude_code");
  });

  it("describes node details used by session snapshots", () => {
    const nodeType: TimelineNodeType = "author_run";
    const detail: NodeDetail = {
      node_id: "timeline_node_001",
      session_id: "workspace_session_0001",
      node_type: nodeType,
      status: "completed",
      agent_role: "author",
      provider: { name: "claude_code", model: "claude_code" },
      messages: [],
      streaming_content: "# Story",
      execution_events: [],
      permission_events: [],
      verdict: null,
      artifact_ref: { artifact_id: "artifact_version_001", version: 1 },
      is_revision: false,
      base_artifact_ref: null,
      started_at: "2026-05-20T00:00:00Z",
      ended_at: "2026-05-20T00:01:00Z",
    };

    expect(detail.node_type).toBe("author_run");
    expect(detail.artifact_ref?.version).toBe(1);
  });

  it("describes coding attempts returned by lifecycle responses", () => {
    const attempt: CodingAttempt = {
      attempt_id: "coding_attempt_0001",
      work_item_id: "work_item_0001",
      attempt_no: 1,
      status: "created",
      stage: "prepare_context",
      branch_name: "aria/work-items/work_item_0001/attempt-1",
      base_branch: "master",
      worktree_path: null,
      rework_count: 0,
      head_commit: null,
      push_status: null,
      review_request_url: null,
      created_at: "2026-05-23T00:00:00Z",
      updated_at: "2026-05-23T00:00:00Z",
    };
    const lifecycle = {
      issue: {} as IssueLifecycleResponse["issue"],
      story_specs: [],
      design_specs: [],
      work_item_plans: [],
      work_items: [
        {
          work_item_id: "work_item_0001",
          issue_id: "issue_0001",
          repository_id: "repository_0001",
          story_spec_ids: [],
          design_spec_ids: [],
          title: "实现爬楼梯",
          plan_status: "confirmed",
          execution_status: "pending",
          latest_attempt: attempt,
          artifact_versions: [],
          work_item_set_id: null,
          kind: "backend",
          sequence_hint: null,
          depends_on: [],
          exclusive_write_scopes: [],
          forbidden_write_scopes: [],
          context_budget: {
            target_context_k: "30-50",
            max_summary_chars: 20000,
            max_handoff_chars: 12000,
            max_code_context_chars: 30000,
            max_context_file_refs: 80,
            max_traceability_refs: 40,
            max_dependency_handoffs: 3,
          },
          required_handoff_from: [],
          verification_plan_ref: null,
          require_execution_plan_confirm: false,
          execution_plan_status: "not_started",
          handoff_summary_ref: null,
          completion_commit: null,
          completion_diff_summary_ref: null,
        },
      ],
      workspace_sessions: [],
      coding_attempts: [attempt],
    } satisfies IssueLifecycleResponse;

    expect(lifecycle.work_items[0].latest_attempt?.attempt_id).toBe("coding_attempt_0001");
    expect(lifecycle.coding_attempts[0].stage).toBe("prepare_context");
  });

  it("accepts lifecycle responses with issue work item plan detail fields", () => {
    const plan = {
      id: "issue_plan_0001",
      project_id: "project_0001",
      issue_id: "issue_0001",
      source_story_spec_ids: ["story_spec_0001"],
      source_design_spec_ids: ["design_spec_0001"],
      options: {
        include_integration_tests: true,
        include_e2e_tests: false,
        force_frontend_backend_split: true,
        require_execution_plan_confirm: false,
      },
      status: "draft",
      work_item_ids: ["work_item_frontend", "work_item_backend"],
      repository_profile_ref: null,
      verification_plan_ids: [],
      dependency_graph: [],
      validator_findings: [],
      created_at: "2026-06-19T00:00:00Z",
      updated_at: "2026-06-19T00:00:00Z",
    } satisfies IssueWorkItemPlanDetailDto;
    const response = {
      issue: {} as IssueLifecycleResponse["issue"],
      story_specs: [],
      design_specs: [],
      work_item_plans: [plan],
      work_items: [],
      workspace_sessions: [],
      coding_attempts: [],
    } satisfies IssueLifecycleResponse;

    expect(response.work_item_plans[0].work_item_ids).toEqual([
      "work_item_frontend",
      "work_item_backend",
    ]);
  });

  it("describes coding attempt snapshots and websocket messages", () => {
    const attempt: CodingAttempt = {
      attempt_id: "coding_attempt_0001",
      work_item_id: "work_item_0001",
      attempt_no: 1,
      status: "running",
      stage: "worktree_prepare",
      branch_name: "aria/work-items/work_item_0001/attempt-1",
      base_branch: "master",
      worktree_path: "/tmp/repo/.worktrees/aria-work-items/work_item_0001/attempt-1",
      rework_count: 0,
      head_commit: null,
      push_status: null,
      review_request_url: null,
      created_at: "2026-05-23T00:00:00Z",
      updated_at: "2026-05-23T00:00:01Z",
    };
    const snapshot: CodingAttemptSnapshotResponse = {
      attempt,
      provider_config_snapshot: { author: "fake", reviewer: "fake", review_rounds: 1 },
      timeline_nodes: [
        {
          id: "coding_node_0001",
          attempt_id: "coding_attempt_0001",
          stage: "worktree_prepare",
          title: "准备 worktree",
          status: "running",
          agent_role: "git",
          summary: null,
          started_at: "2026-05-23T00:00:01Z",
          completed_at: null,
          artifact_refs: [],
        },
      ],
      active_node_id: "coding_node_0001",
      testing_report: null,
      code_review_reports: [],
      review_request: null,
      internal_pr_review: null,
      pending_gates: [],
      pending_choices: [],
      work_item_execution_plan: null,
      work_item_handoff: null,
      require_execution_plan_confirm: false,
      latest_analyst_decision: {
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
      },
    };
    const outbound: Extract<CodingWsOutMessage, { type: "coding_session_state" }> = {
      type: "coding_session_state",
      attempt_id: "coding_attempt_0001",
      status: "running",
      stage: "worktree_prepare",
      branch_name: "aria/work-items/work_item_0001/attempt-1",
      base_branch: "master",
      worktree_path: null,
      rework_count: 0,
      max_auto_rework: 2,
      head_commit: null,
      pushed_remote: null,
      role_provider_config_snapshot: {
        coder: "fake",
        tester: "fake",
        analyst: "fake",
        code_reviewer: "fake",
        internal_reviewer: "fake",
        review_rounds: 1,
        permission_modes: {
          coder: "supervised",
          tester: "auto",
          analyst: "auto",
          code_reviewer: "supervised",
          internal_reviewer: "supervised",
        },
      },
      provider_config_snapshot: { author: "fake", reviewer: "fake", review_rounds: 1 },
      chat_entries: [],
      work_item_execution_plan: null,
      work_item_handoff: null,
      require_execution_plan_confirm: false,
      timeline_nodes: snapshot.timeline_nodes,
      role_runs: [
        {
          id: "coding_role_run_0001",
          attempt_id: "coding_attempt_0001",
          stage: "testing",
          role: "tester",
          run_no: 1,
          status: "running",
          trigger: "initial",
          node_id: "coding_node_0003",
          started_at: "2026-06-13T00:00:00Z",
          completed_at: null,
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
        },
      ],
      active_node_id: "coding_node_0001",
      testing_report: null,
      code_review_reports: [],
      review_request: null,
      internal_pr_review: null,
      pending_gates: [],
      pending_choices: [],
      latest_analyst_decision: snapshot.latest_analyst_decision,
    };
    const inbound: CodingWsInMessage = { type: "start_coding" };

    expect(snapshot.active_node_id).toBe("coding_node_0001");
    expect(outbound.type).toBe("coding_session_state");
    expect(outbound.role_runs?.[0].event_summary?.event_count).toBe(2);
    expect(outbound.role_runs?.[0].recent_events?.[0].detail).toBe("No tasks found");
    expect(outbound.latest_analyst_decision?.next_stage).toBe("coding");
    expect(inbound.type).toBe("start_coding");
  });

  it("accepts analyst decision records for coding workspace display", () => {
    const decision: AnalystDecisionRecord = {
      id: "analyst_decision_0002",
      attempt_id: "coding_attempt_0001",
      source_stage: "code_review",
      rework_round: 2,
      verdict: "proceed",
      next_stage: "review_request",
      reason: "CodeReviewer 通过，进入 ReviewRequest",
      evidence_refs: ["code_review_0001.json"],
      raw_provider_output_refs: ["provider-raw/code_review/code_review_0001.txt"],
      rework_instructions: null,
      human_gate: {
        reason_code: "manual_triage",
        available_actions: ["provide_context", "manual_continue", "abort"],
      },
      created_at: "2026-06-12T00:01:00Z",
      parse_error: null,
    };

    expect(decision.verdict).toBe("proceed");
    expect(decision.next_stage).toBe("review_request");
    expect(decision.human_gate?.available_actions).toContain("manual_continue");
  });

  it("accepts plan based testing reports and blocked gate metadata", () => {
    const report: TestingReport = {
      id: "testing_report_0001",
      attempt_id: "coding_attempt_0001",
      commands: [],
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
      skipped_required_steps: ["manual_browser"],
      context_warnings: ["missing_design_spec"],
      raw_provider_output_ref: "provider-raw/testing/execute_test_plan_0001.txt",
      overall_status: "passed_with_warnings",
      provider_claim: null,
      backend_verified: true,
      started_at: "2026-06-10T00:00:00Z",
      completed_at: "2026-06-10T00:00:01Z",
    };
    const gate: CodingGateRequired = {
      gate_id: "coding_gate_0001",
      kind: "blocked",
      title: "Testing blocked",
      description: "Required test step missing",
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
    };

    expect(report.overall_status).toBe("passed_with_warnings");
    expect((report.steps ?? [])[0].step_id).toBe("api_smoke");
    expect(gate.reason_code).toBe("missing_required_test_step");
    expect(gate.available_actions[0].action_type).toBe("rerun_missing_steps");
  });

  it("accepts retry analyst gate actions", () => {
    const action: import("./types").CodingGateAction = {
      action_id: "retry_analyst",
      label: "重试 Analyst",
      action_type: "retry_analyst",
    };

    expect(action.action_type).toBe("retry_analyst");
  });

  it("accepts role run metadata on analyst decisions", () => {
    const decision: AnalystDecisionRecord = {
      id: "analyst_decision_0001",
      attempt_id: "coding_attempt_0001",
      source_stage: "testing",
      rework_round: 1,
      verdict: "human_required",
      next_stage: "human_gate",
      reason: "Analyst 输出不是有效 JSON",
      evidence_refs: [],
      raw_provider_output_refs: [],
      created_at: "2026-06-13T00:00:00Z",
      role_run_id: "coding_role_run_0001",
      run_no: 1,
    };

    expect(decision.role_run_id).toBe("coding_role_run_0001");
    expect(decision.run_no).toBe(1);
  });

  it("accepts prepare work item plan request and response shapes", () => {
    const request: PrepareWorkItemPlanRequest = {
      title: "爬楼梯 Work Item Plan",
      story_spec_ids: ["story_spec_0001"],
      design_spec_ids: ["design_spec_0001"],
      include_integration_tests: false,
      include_e2e_tests: false,
      force_frontend_backend_split: false,
      require_execution_plan_confirm: true,
      author_provider: "claude_code",
      reviewer_provider: "codex",
      review_rounds: 1,
      superpowers_enabled: true,
      openspec_enabled: true,
    };
    const response: PrepareWorkItemPlanResponse = {
      work_item_plan: {
        id: "issue_plan_0001",
        project_id: "project_0001",
        issue_id: "issue_0001",
        source_story_spec_ids: request.story_spec_ids ?? [],
        source_design_spec_ids: request.design_spec_ids ?? [],
        options: {
          include_integration_tests: false,
          include_e2e_tests: false,
          force_frontend_backend_split: false,
          require_execution_plan_confirm: true,
        },
        status: "draft",
        work_item_ids: ["work_item_0001"],
        repository_profile_ref: null,
        verification_plan_ids: [],
        dependency_graph: [],
        validator_findings: [],
        created_at: "2026-06-17T00:00:00Z",
        updated_at: "2026-06-17T00:00:00Z",
      },
      workspace_session: {
        workspace_session_id: "workspace_session_plan_group_0001",
        issue_id: "issue_0001",
        entity_id: "issue_plan_0001",
        workspace_type: "work_item_plan",
        status: "open",
        author_provider: "claude_code",
        reviewer_provider: "codex",
        review_rounds: 1,
        superpowers_enabled: true,
        openspec_enabled: true,
        messages: [],
      },
    };

    expect(response.work_item_plan.id).toBe("issue_plan_0001");
    expect(response.workspace_session.entity_id).toBe(response.work_item_plan.id);
    expect(response.workspace_session.workspace_type).toBe("work_item_plan");
  });

  it("accepts role run metadata on review reports", () => {
    const report: CodeReviewReport = {
      id: "code_review_0001",
      attempt_id: "coding_attempt_0001",
      round: 1,
      verdict: "approve",
      findings: [],
      tested_evidence_refs: [],
      diff_refs: [],
      summary: "review ok",
      created_at: "2026-06-13T00:00:00Z",
      raw_provider_output_ref: "provider-raw/code_review/code_review_0001.txt",
      role_run_id: "coding_role_run_0001",
      run_no: 1,
    };

    const internal: InternalPrReview = {
      id: "internal_review_0001",
      attempt_id: "coding_attempt_0001",
      review_request_id: "review_request_0001",
      verdict: "approve",
      findings: [],
      impact_scope: ["src/lib.rs"],
      pr_description: "PR",
      commit_message_suggestion: "feat: work",
      tested_evidence_refs: [],
      diff_refs: [],
      summary: "internal ok",
      created_at: "2026-06-13T00:00:01Z",
      raw_provider_output_ref: "provider-raw/internal_pr_review/internal_pr_review_0001.txt",
      role_run_id: "coding_role_run_0002",
      run_no: 1,
    };

    expect(report.run_no).toBe(1);
    expect(internal.role_run_id).toBe("coding_role_run_0002");
  });

  it("describes work item execution plan and handoff in coding snapshots", () => {
    const plan: WorkItemExecutionPlan = {
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

    expect(plan.allowed_write_scopes).toEqual(["src/product/**"]);
  });
});

describe("work item split lifecycle types", () => {
  it("describes split work item lifecycle metadata", () => {
    const workItem = {
      work_item_id: "work_item_0001",
      issue_id: "issue_0001",
      repository_id: "repository_0001",
      story_spec_ids: ["story_spec_0001"],
      design_spec_ids: ["design_spec_0001"],
      title: "后端 API",
      plan_status: "confirmed",
      execution_status: "pending",
      latest_attempt: null,
      artifact_versions: [],
      work_item_set_id: "work_item_set_0001",
      kind: "backend",
      sequence_hint: 10,
      depends_on: [],
      exclusive_write_scopes: ["src/product/**"],
      forbidden_write_scopes: ["web/**"],
      context_budget: {
        target_context_k: "30-50",
        max_summary_chars: 20000,
        max_handoff_chars: 12000,
        max_code_context_chars: 30000,
        max_context_file_refs: 80,
        max_traceability_refs: 40,
        max_dependency_handoffs: 3,
      },
      required_handoff_from: [],
      verification_plan_ref: "verification_plan_work_item_0001",
      require_execution_plan_confirm: false,
      execution_plan_status: "not_started",
      handoff_summary_ref: null,
      completion_commit: null,
      completion_diff_summary_ref: null,
    } satisfies LifecycleWorkItem;

    const request = {
      title: "登录会话拆分实现",
      story_spec_ids: ["story_spec_0001"],
      design_spec_ids: ["design_spec_0001"],
      include_integration_tests: true,
      include_e2e_tests: false,
      force_frontend_backend_split: true,
      require_execution_plan_confirm: false,
    } satisfies GenerateWorkItemsRequest;

    expect(workItem.kind).toBe("backend");
    expect(request.include_integration_tests).toBe(true);
  });
});
