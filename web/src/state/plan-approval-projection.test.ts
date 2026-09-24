import { describe, expect, it } from "vitest";
import type {
  PlanProjectionBundle,
  PlanRepairSessionSnapshot,
  ProjectionValidationReport,
} from "../api/types";
import type { ChatEntry } from "./chat-entries";
import type { HumanGateTurnState, TimelineNode } from "./workspace-ws-store-types";
import { planRepairSnapshotFixture } from "./workspace-plan-repair-test-fixtures";
import {
  checklistFindingsFromPlanRepair,
  checklistFindingsFromValidation,
  planRepairStageLabel,
  selectContractChecklist,
  selectFindingTargetEntryId,
  selectPlanRepairPanel,
} from "./plan-approval-projection";

function planProjectionBundle(): PlanProjectionBundle {
  return {
    id: "bundle_0001",
    plan_revision_id: "plan_rev_0002",
    dependency_graph_revision_id: "graph_rev_0001",
    work_item_projection_bundle_refs: [],
    human_group_projection: {
      plan_id: "plan_0001",
      goal: "交付指标导出",
      split_reason: "前后端分离",
      work_items: [],
      contract_flow: [
        {
          from: "wi_backend",
          to: "wi_frontend",
          contract_id: "contract_metrics",
          required_capabilities: ["cap_export_csv", "cap_export_json"],
          provided_capabilities: ["cap_export_csv"],
          missing_capabilities: ["cap_export_json"],
        },
        {
          from: "wi_worker",
          to: "wi_backend",
          contract_id: "contract_queue",
          required_capabilities: ["cap_push_job"],
          provided_capabilities: ["cap_push_job"],
          missing_capabilities: [],
        },
      ],
      risks: [],
      source_refs: [],
      normative: false,
      used_by_provider: false,
    },
    coder_group_context: {
      plan_id: "plan_0001",
      ordered_logical_work_item_ids: [],
      dependency_edges: [],
      group_write_scopes: {},
    },
    reviewer_group_matrix: {
      plan_id: "plan_0001",
      work_items: [],
      dependency_edges: [],
      design_traceability_refs: [],
    },
    human_group_projection_hash: "h1",
    coder_group_context_hash: "h2",
    reviewer_group_matrix_hash: "h3",
    compiler_version: "v1",
    created_at: "2026-09-14T08:00:00Z",
  };
}

function validationReport(): ProjectionValidationReport {
  return {
    findings: [
      {
        code: "CONTRACT_CAPABILITY_MISSING",
        projection: "human_group",
        contract_ref: "contract_metrics",
        message: "cap_export_json 未由上游提供",
      },
      {
        code: "PROJECTION_STALE",
        projection: "coder_group",
        contract_ref: null,
        message: "coder 投影过期",
      },
    ],
  };
}

describe("selectContractChecklist", () => {
  it("derives one row per required capability with satisfied flags and gap count", () => {
    const checklist = selectContractChecklist(planProjectionBundle(), []);
    expect(checklist.hasData).toBe(true);
    expect(checklist.rows.map((row) => row.key)).toEqual([
      "contract_metrics::cap_export_csv",
      "contract_metrics::cap_export_json",
      "contract_queue::cap_push_job",
    ]);
    expect(checklist.rows[0]).toMatchObject({
      contractId: "contract_metrics",
      from: "wi_backend",
      to: "wi_frontend",
      capability: "cap_export_csv",
      satisfied: true,
    });
    expect(checklist.rows[1]).toMatchObject({
      capability: "cap_export_json",
      satisfied: false,
    });
    expect(checklist.gapCount).toBe(1);
  });

  it("matches validation findings by contract ref and capability ref", () => {
    const checklist = selectContractChecklist(
      planProjectionBundle(),
      checklistFindingsFromValidation(validationReport()),
    );
    const gapRow = checklist.rows[1];
    expect(gapRow.matchedFindings).toHaveLength(1);
    expect(gapRow.matchedFindings[0]).toMatchObject({
      code: "CONTRACT_CAPABILITY_MISSING",
      contractRef: "contract_metrics",
    });
    expect(checklist.rows[0].matchedFindings).toHaveLength(0);
    expect(checklist.rows[2].matchedFindings).toHaveLength(0);
  });

  it("returns an empty checklist without a bundle", () => {
    const checklist = selectContractChecklist(null, []);
    expect(checklist).toEqual({ rows: [], gapCount: 0, hasData: false });
  });
});

describe("checklistFindingsFromPlanRepair", () => {
  it("flattens contract validation findings with severity and capability refs", () => {
    const snapshot = planRepairSnapshotFixture("session_child", {
      validation: {
        id: "validation_0001",
        plan_id: "plan_0001",
        plan_revision_id: "plan_rev_0002",
        plan_projection_bundle_id: "bundle_0001",
        contract_validation: {
          findings: [
            {
              code: "BREAKING_CONTRACT_CHANGE",
              severity: "Error",
              logical_work_item_id: "wi_backend",
              contract_ref: "contract_metrics",
              capability_ref: "cap_export_json",
              message: "破坏性契约变更",
            },
          ],
        },
        projection_validation: { findings: [] },
        created_at: "2026-09-14T09:00:00Z",
      },
    });
    const findings = checklistFindingsFromPlanRepair(snapshot);
    expect(findings).toEqual([
      {
        code: "BREAKING_CONTRACT_CHANGE",
        severity: "Error",
        contractRef: "contract_metrics",
        capabilityRef: "cap_export_json",
        message: "破坏性契约变更",
      },
    ]);
    expect(checklistFindingsFromPlanRepair(null)).toEqual([]);
  });
});

describe("selectFindingTargetEntryId", () => {
  function gateEntry(id: string, contractFields: Array<string | null>): ChatEntry {
    return {
      id,
      type: "gate_prompt",
      role: "system",
      content: "等待人工确认",
      timestamp: "2026-09-14T08:00:00Z",
      metadata: {
        findings: contractFields.map((contract_field) => ({
          class: "repairable",
          fingerprint: "fp",
          category: null,
          severity: "major",
          message: "契约缺口",
          evidence: null,
          required_action: null,
          contract_field,
        })),
      },
    };
  }

  it("resolves the latest gate entry whose findings mention the contract", () => {
    const entries: ChatEntry[] = [
      gateEntry("gate_old", ["contract_metrics"]),
      {
        id: "review_1",
        type: "review_verdict",
        role: "reviewer",
        content: "需要修改",
        timestamp: "2026-09-14T08:00:01Z",
        metadata: { findings: [{ contract_field: "contract_queue" }] },
      },
      gateEntry("gate_new", ["contract_other", "contract_metrics"]),
    ];
    expect(selectFindingTargetEntryId(entries, "contract_metrics")).toBe("gate_new");
    expect(selectFindingTargetEntryId(entries, "contract_queue")).toBe("review_1");
    expect(selectFindingTargetEntryId(entries, "contract_unknown")).toBeNull();
  });

  it("skips entries without findings metadata", () => {
    const entries: ChatEntry[] = [
      {
        id: "gate_bare",
        type: "gate_prompt",
        role: "system",
        content: "等待人工确认",
        timestamp: "2026-09-14T08:00:00Z",
      },
    ];
    expect(selectFindingTargetEntryId(entries, "contract_metrics")).toBeNull();
  });

  it("does not invent a target for review findings without contract_field", () => {
    const entries: ChatEntry[] = [
      {
        id: "review_without_contract_field",
        type: "review_verdict",
        role: "reviewer",
        content: "需要补充契约实现",
        timestamp: "2026-09-14T08:00:00Z",
        metadata: {
          findings: [{ severity: "must_fix", message: "缺少契约实现" }],
        },
      },
    ];
    expect(selectFindingTargetEntryId(entries, "contract_metrics")).toBeNull();
  });

  // F-49 A9：门卡与结论卡带同一批 findings，而门卡总在数组末尾——倒序命中即返回
  // 落点偏门卡，而门卡此前不显示 findings（跳过去看不到缺口）。结论卡展开渲染
  // findings 分组，落点优先结论卡；门卡（F-49 B3 后带折叠列表）作兜底。
  it("prefers the review verdict card over a later gate card carrying the same finding", () => {
    const entries: ChatEntry[] = [
      {
        id: "review_1",
        type: "review_verdict",
        role: "reviewer",
        content: "需要修改",
        timestamp: "2026-09-14T08:00:01Z",
        metadata: { findings: [{ contract_field: "contract_metrics" }] },
      },
      gateEntry("gate_new", ["contract_metrics"]),
    ];

    expect(selectFindingTargetEntryId(entries, "contract_metrics")).toBe("review_1");
  });

  it("falls back to the gate card when no review verdict carries the finding", () => {
    const entries: ChatEntry[] = [gateEntry("gate_new", ["contract_metrics"])];

    expect(selectFindingTargetEntryId(entries, "contract_metrics")).toBe("gate_new");
  });
});

describe("planRepairStageLabel", () => {
  it("labels every stage and null", () => {
    expect(planRepairStageLabel("awaiting_confirmation")).toBe("等待最终确认");
    expect(planRepairStageLabel("published")).toBe("已出版");
    expect(planRepairStageLabel("amendment_apply_failed")).toBe("修订应用失败");
    expect(planRepairStageLabel(null)).toBe("未知阶段");
  });
});

describe("selectPlanRepairPanel", () => {
  function snapshot(childSessionId: string): PlanRepairSessionSnapshot {
    return {
      request: {
        id: "repair_0001",
        plan_id: "plan_0001",
        base_plan_revision_id: "plan_rev_0001",
        trigger_attempt_id: "coding_attempt_0001",
        trigger_unit_run_id: "unit_run_0001",
        trigger_review_id: null,
        trigger_finding_id: "finding_0001",
        amendment_id: "amend_0001",
        defect_class: "upstream_contract_invalid",
        reason_code: "CONTRACT_CAPABILITY_MISSING",
        repair_target: {
          kind: "current_work_item",
          logical_work_item_ids: ["wi_0001"],
          work_item_revision_ids: ["rev_0001"],
        },
        contract_refs: ["contract_metrics"],
        capability_refs: ["cap_export_json"],
        evidence: [],
        fingerprint: "fp_0001",
        status: "awaiting_confirmation",
        created_at: "2026-09-14T08:00:00Z",
        updated_at: "2026-09-14T09:00:00Z",
      },
      link: {
        id: "link_0001",
        relation: "plan_repair",
        parent_session_id: "session_parent",
        child_session_id: childSessionId,
        trigger: {
          attempt_id: "coding_attempt_0001",
          unit_run_id: "unit_run_0001",
          review_id: null,
          finding_id: "finding_0001",
          repair_request_id: "repair_0001",
          amendment_id: "amend_0001",
          fingerprint: "fp_0001",
          base_plan_revision_id: "plan_rev_0001",
        },
        return_context: {
          original_attempt_id: "coding_attempt_0001",
          original_unit_run_id: "unit_run_0001",
          timeline_anchor_id: "anchor_0001",
          original_route: "/workbench/projects/p1/issues/i1/coding/coding_attempt_0001",
        },
        created_at: "2026-09-14T08:00:00Z",
      },
      stage: "awaiting_confirmation",
      projection: null,
      amendment: {
        id: "amend_0001",
        repair_request_id: "repair_0001",
        previous_plan_revision_id: "plan_rev_0001",
        new_plan_revision_id: "plan_rev_0002",
        revised_work_items: {
          wi_backend: {
            previous_revision_id: "rev_0001",
            next_revision_id: "rev_0002",
            delta_kind: "breaking_contract_change",
          },
        },
        superseded_revisions: ["rev_0001"],
        dependency_graph_changes: [],
        contract_deltas: [
          {
            logical_work_item_id: "wi_backend",
            previous_revision_id: "rev_0001",
            next_revision_id: "rev_0002",
            kind: "breaking_contract_change",
            added_contracts: [],
            removed_contracts: [],
            added_capabilities: ["cap_export_json"],
            removed_capabilities: [],
            changed_capabilities: [],
            added_capability_associations: [],
            removed_capability_associations: [],
            acceptance_changed: false,
            verification_changed: false,
            write_policy_changed: false,
          },
        ],
        unaffected_units: [],
        revalidation_required_units: ["wi_frontend"],
        stale_units: [],
        replacement_units: {},
        resume_target: {
          logical_work_item_id: "wi_backend",
          mode: "revalidate",
        },
        created_at: "2026-09-14T09:00:00Z",
      },
      validation: null,
      impact: null,
      plan_review: null,
      package_identity: null,
      candidate_package_artifact_id: null,
      impact_scope_review: null,
      timeline_nodes: [],
      error: null,
    };
  }

  const nodes: TimelineNode[] = [
    {
      node_id: "timeline_node_001",
      node_type: "author_run",
      agent: null,
      stage: "running",
      round: 1,
      status: "completed",
      title: "修订编写",
      summary: "已产出修订",
      started_at: "2026-09-14T08:10:00Z",
      completed_at: "2026-09-14T08:20:00Z",
      duration_ms: 600000,
      artifact_ref: null,
      provider_config_snapshot: { author: "fake", reviewer: null, review_rounds: 1 },
      retry: null,
    },
    {
      node_id: "timeline_node_002",
      node_type: "plan_review",
      agent: null,
      stage: "running",
      round: 1,
      status: "active",
      title: "Plan Review",
      summary: null,
      started_at: "2026-09-14T08:20:00Z",
      completed_at: null,
      duration_ms: null,
      artifact_ref: null,
      provider_config_snapshot: { author: "fake", reviewer: null, review_rounds: 1 },
      retry: null,
    },
  ];

  const turn: HumanGateTurnState = {
    turn_id: "turn_0001",
    command_id: "cmd_0001",
    remaining_budget: 1,
    status: "open",
    artifact_ref: null,
    failure_class: null,
    failure_message: null,
    opened_at: "2026-09-14T08:30:00Z",
    inlineError: null,
  };

  it("is visible only when the snapshot belongs to the current child session", () => {
    expect(
      selectPlanRepairPanel(snapshot("session_child"), "session_child", nodes, turn).visible,
    ).toBe(true);
    expect(
      selectPlanRepairPanel(snapshot("session_child"), "session_parent", nodes, turn).visible,
    ).toBe(false);
    expect(selectPlanRepairPanel(null, "session_child", nodes, turn).visible).toBe(false);
    expect(selectPlanRepairPanel(snapshot("session_child"), null, nodes, turn).visible).toBe(false);
  });

  it("projects request metadata, rounds, feedback turn and amendment summary", () => {
    const model = selectPlanRepairPanel(
      snapshot("session_child"),
      "session_child",
      nodes,
      turn,
    );
    expect(model.stageLabel).toBe("等待最终确认");
    expect(model.requestStatus).toBe("awaiting_confirmation");
    expect(model.defectClass).toBe("upstream_contract_invalid");
    expect(model.reasonCode).toBe("CONTRACT_CAPABILITY_MISSING");
    expect(model.triggerFindingId).toBe("finding_0001");
    expect(model.rounds).toEqual([
      {
        nodeId: "timeline_node_001",
        title: "修订编写",
        summary: "已产出修订",
        status: "completed",
        startedAt: "2026-09-14T08:10:00Z",
        completedAt: "2026-09-14T08:20:00Z",
      },
      {
        nodeId: "timeline_node_002",
        title: "Plan Review",
        summary: null,
        status: "active",
        startedAt: "2026-09-14T08:20:00Z",
        completedAt: null,
      },
    ]);
    expect(model.feedbackTurn).toEqual({
      turnId: "turn_0001",
      commandId: "cmd_0001",
      status: "open",
      remainingBudget: 1,
      openedAt: "2026-09-14T08:30:00Z",
    });
    expect(model.amendment).toEqual({
      id: "amend_0001",
      previousPlanRevisionId: "plan_rev_0001",
      newPlanRevisionId: "plan_rev_0002",
      revisedWorkItemCount: 1,
      contractDeltaCount: 1,
      supersededCount: 1,
      dependencyGraphChangeCount: 0,
      resumeTarget: "wi_backend · revalidate",
    });
    expect(model.error).toBeNull();
  });

  it("returns an all-empty model when not visible", () => {
    const model = selectPlanRepairPanel(snapshot("session_child"), "session_other", nodes, turn);
    expect(model).toEqual({
      visible: false,
      stageLabel: null,
      requestStatus: null,
      defectClass: null,
      reasonCode: null,
      triggerFindingId: null,
      amendment: null,
      rounds: [],
      feedbackTurn: null,
      error: null,
    });
  });

  it("tolerates a missing amendment manifest", () => {
    const bare = { ...snapshot("session_child"), amendment: null };
    const model = selectPlanRepairPanel(bare, "session_child", [], null);
    expect(model.amendment).toBeNull();
    expect(model.feedbackTurn).toBeNull();
    expect(model.rounds).toEqual([]);
  });
});
