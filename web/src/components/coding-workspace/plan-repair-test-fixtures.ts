import type {
  LinkedWorkspaceSessionSnapshot,
  PlanAmendmentManifest,
  PlanProjectionBundle,
} from "../../api/types";
import type { PlanRepairSessionState } from "../../state/plan-repair-session";

export function repairAwaitingConfirmationFixture(): PlanRepairSessionState {
  const projection = planProjectionFixture();
  const amendment = planAmendmentFixture();
  return {
    request: {
      id: "plan_repair_request_0001",
      plan_id: "plan_0001",
      base_plan_revision_id: "plan_revision_0001",
      trigger_attempt_id: "coding_attempt_0001",
      trigger_unit_run_id: "unit_run_0001",
      trigger_review_id: "review_0001",
      trigger_finding_id: "finding_0001",
      amendment_id: amendment.id,
      defect_class: "upstream_contract_invalid",
      reason_code: "missing_failure_message",
      repair_target: {
        kind: "subgraph",
        logical_work_item_ids: ["WI-01", "WI-02"],
        work_item_revision_ids: ["wi_revision_01_v1", "wi_revision_02_v1"],
      },
      contract_refs: ["domain_result"],
      capability_refs: ["failure_message"],
      evidence: [
        {
          kind: "review_finding",
          source_ref: "review_0001#finding_0001",
          message: "领域错误缺少可供接口层使用的 failure_message。",
        },
      ],
      fingerprint: "repair_fingerprint_0001",
      status: "awaiting_confirmation",
      created_at: "2026-07-20T00:00:00Z",
      updated_at: "2026-07-20T00:05:00Z",
    },
    link: {
      id: "workspace_session_link_0001",
      relation: "plan_repair",
      parent_session_id: "coding_attempt_0001",
      child_session_id: "workspace_session_repair_0001",
      trigger: {
        attempt_id: "coding_attempt_0001",
        unit_run_id: "unit_run_0001",
        review_id: "review_0001",
        finding_id: "finding_0001",
        repair_request_id: "plan_repair_request_0001",
        amendment_id: amendment.id,
        fingerprint: "repair_fingerprint_0001",
        base_plan_revision_id: "plan_revision_0001",
      },
      return_context: {
        original_attempt_id: "coding_attempt_0001",
        original_unit_run_id: "unit_run_0001",
        timeline_anchor_id: "coding_node_review_0001",
        original_route:
          "/workbench/projects/project_0001/issues/issue_0001/coding/coding_attempt_0001",
      },
      created_at: "2026-07-20T00:00:00Z",
    },
    stage: "awaiting_confirmation",
    projection,
    amendment,
    validation: {
      id: "validation_report_0002",
      plan_id: "plan_0001",
      plan_revision_id: "plan_revision_0002",
      plan_projection_bundle_id: projection.id,
      contract_validation: { findings: [] },
      projection_validation: { findings: [] },
      created_at: "2026-07-20T00:04:00Z",
    },
    impact: {
      unaffected: ["WI-03"],
      direct_revalidation: ["WI-02"],
      direct_stale: ["WI-01"],
      conditional_downstream: ["WI-03"],
      explanation_paths: [
        {
          from: "WI-01",
          to: "WI-02",
          contract_id: "domain_result",
          capability_refs: ["failure_message"],
        },
      ],
    },
    plan_review: {
      verdict: "pass",
      review_scope: "batch",
      target_outline_id: null,
      generation_round_id: "generation_round_0002",
      draft_id: null,
      batch_id: "batch_0002",
      review_action: "continue",
      gates: [],
    },
    package_identity: {
      request_id: "plan_repair_request_0001",
      amendment_id: amendment.id,
      plan_id: "plan_0001",
      base_plan_revision_id: "plan_revision_0001",
      next_plan_revision_id: "plan_revision_0002",
      projection_bundle_id: projection.id,
      validation_report_id: "validation_report_0002",
      review_attestation_id: "review_attestation_0002",
      reviewed_plan_revision_id: "plan_revision_0002",
      review_generation_round_id: "generation_round_0002",
      candidate_package_artifact_id: "candidate_package_0002",
      candidate_package_fingerprint: "candidate_package_fingerprint_0002",
    },
    candidate_package_artifact_id: "candidate_package_0002",
    impact_scope_review: {
      system_minimum_impact_scope: ["WI-01", "WI-02"],
      proposed_accepted_impact_scope: ["WI-01", "WI-02"],
      risk_acceptance_reason: "完整覆盖直接陈旧与重新验证单元。",
      candidate_package_fingerprint: "candidate_package_fingerprint_0002",
      review_generation_round_id: "generation_round_0002",
    },
    error: null,
    childSessionId: "workspace_session_repair_0001",
    childTimelineNodes: [
      {
        node_id: "plan_repair_node_author_0001",
        node_type: "revision",
        stage: "revision",
        title: "修订 Work Item Contract",
        status: "completed",
        summary: "新增 failure_message",
        started_at: "2026-07-20T00:01:00Z",
        completed_at: "2026-07-20T00:02:00Z",
        artifact_ref: "candidate_package_0002",
        provider_config_snapshot: {
          author: "fake",
          reviewer: "fake",
          review_rounds: 1,
        },
      },
      {
        node_id: "plan_repair_node_confirm_0001",
        node_type: "human_confirm",
        stage: "human_confirm",
        title: "确认 Plan 修订",
        status: "active",
        summary: "等待一次性确认",
        started_at: "2026-07-20T00:05:00Z",
        completed_at: null,
        artifact_ref: projection.id,
        provider_config_snapshot: {
          author: "fake",
          reviewer: "fake",
          review_rounds: 1,
        },
      },
    ],
    timelineNodes: [
      {
        id: "plan_repair_node_author_0001",
        attempt_id: "coding_attempt_0001",
        stage: "coding",
        title: "修订 Work Item Contract",
        status: "completed",
        agent_role: "author",
        summary: "新增 failure_message",
        started_at: "2026-07-20T00:01:00Z",
        completed_at: "2026-07-20T00:02:00Z",
        artifact_refs: ["candidate_package_0002"],
      },
      {
        id: "plan_repair_node_confirm_0001",
        attempt_id: "coding_attempt_0001",
        stage: "final_confirm",
        title: "确认 Plan 修订",
        status: "running",
        agent_role: "system",
        summary: "等待一次性确认",
        started_at: "2026-07-20T00:05:00Z",
        completed_at: null,
        artifact_refs: [projection.id],
      },
    ],
    history: {
      entries: [
        {
          kind: "contract_delta",
          id: "history_delta_0001",
          logical_work_item_id: "WI-01",
          related_revision_id: "wi_revision_01_v2",
          summary: "新增 failure_message",
          created_at: "2026-07-20T00:03:00Z",
        },
      ],
    },
  };
}

export function linkedWorkspaceAmendmentSnapshotFixture(
  workspaceType: "story" | "design" = "story",
): LinkedWorkspaceSessionSnapshot {
  const repair = repairAwaitingConfirmationFixture();
  const relation = workspaceType === "story" ? "story_amendment" : "design_amendment";
  return {
    link: {
      ...repair.link,
      id: `workspace_session_link_${relation}_0001`,
      relation,
      parent_session_id: repair.childSessionId,
      child_session_id: `workspace_session_${relation}_0001`,
      return_context: {
        ...repair.link.return_context,
        original_route: `/workbench/workspace/${repair.childSessionId}`,
      },
    },
    workspace_type: workspaceType,
    artifact_version_id: null,
    timeline_nodes: [],
    selected_timeline_node_id: null,
    human_confirm_state: "open",
  };
}

export function planAmendmentFixture(): PlanAmendmentManifest {
  return {
    id: "plan_amendment_0001",
    repair_request_id: "plan_repair_request_0001",
    previous_plan_revision_id: "plan_revision_0001",
    new_plan_revision_id: "plan_revision_0002",
    revised_work_items: {
      "WI-01": {
        previous_revision_id: "wi_revision_01_v1",
        next_revision_id: "wi_revision_01_v2",
        delta_kind: "compatible_contract_extension",
      },
    },
    superseded_revisions: ["wi_revision_01_v1"],
    dependency_graph_changes: [],
    contract_deltas: [
      {
        logical_work_item_id: "WI-01",
        previous_revision_id: "wi_revision_01_v1",
        next_revision_id: "wi_revision_01_v2",
        kind: "compatible_contract_extension",
        added_contracts: [],
        removed_contracts: [],
        added_capabilities: ["failure_message"],
        removed_capabilities: [],
        changed_capabilities: ["domain_error"],
        added_capability_associations: [
          { contract_id: "domain_result", capability: "failure_message" },
        ],
        removed_capability_associations: [],
        acceptance_changed: true,
        verification_changed: true,
        write_policy_changed: false,
      },
    ],
    unaffected_units: ["WI-03"],
    revalidation_required_units: ["WI-02"],
    stale_units: ["WI-01"],
    replacement_units: {},
    resume_target: { logical_work_item_id: "WI-01", mode: "reexecute" },
    created_at: "2026-07-20T00:03:00Z",
  };
}

function planProjectionFixture(): PlanProjectionBundle {
  return {
    id: "plan_projection_bundle_0002",
    plan_revision_id: "plan_revision_0002",
    dependency_graph_revision_id: "dependency_graph_revision_0002",
    work_item_projection_bundle_refs: [
      "work_item_projection_bundle_01_v2",
      "work_item_projection_bundle_02_v1",
    ],
    human_group_projection: {
      plan_id: "plan_0001",
      goal: "修复领域错误契约并恢复 Coding Attempt",
      split_reason: "只修订受 failure_message 影响的最小子图。",
      work_items: [
        {
          logical_work_item_id: "WI-01",
          title: "初始化领域模型",
          goal: "向领域错误输出补充 failure_message。",
          depends_on: [],
          provides: ["domain_result.failure_message"],
          scope_summary: {
            owned_scopes: ["src/domain/**"],
            forbidden_scopes: ["web/**"],
          },
        },
        {
          logical_work_item_id: "WI-02",
          title: "验证接口错误响应",
          goal: "重新验证接口错误响应映射。",
          depends_on: ["WI-01"],
          provides: ["api_error_response"],
          scope_summary: {
            owned_scopes: ["src/api/**"],
            forbidden_scopes: [],
          },
        },
      ],
      contract_flow: [
        {
          from: "WI-01",
          to: "WI-02",
          contract_id: "domain_result",
          required_capabilities: ["failure_message"],
          provided_capabilities: ["failure_message"],
          missing_capabilities: [],
        },
      ],
      risks: ["接口错误文案需保持兼容"],
      source_refs: ["design_spec_0001#domain-errors"],
      normative: false,
      used_by_provider: false,
    },
    coder_group_context: {
      plan_id: "plan_0001",
      ordered_logical_work_item_ids: ["WI-01", "WI-02"],
      dependency_edges: [],
      group_write_scopes: {
        "WI-01": {
          exclusive_scopes: ["src/domain/**"],
          forbidden_scopes: ["web/**"],
        },
      },
    },
    reviewer_group_matrix: {
      plan_id: "plan_0001",
      work_items: [
        {
          logical_work_item_id: "WI-01",
          criterion_refs: ["AC-FAILURE-MESSAGE"],
          input_contract_refs: [],
          output_contract_refs: ["domain_result"],
        },
        {
          logical_work_item_id: "WI-02",
          criterion_refs: ["AC-API-ERROR"],
          input_contract_refs: ["domain_result"],
          output_contract_refs: ["api_error_response"],
        },
      ],
      dependency_edges: [],
      design_traceability_refs: [
        {
          source_type: "design_spec",
          source_id: "design_spec_0001",
          requirement_id: "REQ-ERROR-001",
        },
      ],
    },
    human_group_projection_hash: "human_hash_0002",
    coder_group_context_hash: "coder_hash_0002",
    reviewer_group_matrix_hash: "reviewer_hash_0002",
    compiler_version: "projection-compiler-1",
    created_at: "2026-07-20T00:03:00Z",
  };
}
