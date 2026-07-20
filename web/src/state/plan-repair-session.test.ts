import { beforeEach, describe, expect, it } from "vitest";
import type { CodingWsOutMessage, PlanRepairSessionSnapshot } from "../api/types";
import { useCodingWorkspaceStore } from "./coding-workspace-store";

const providerConfig = {
  author: "fake" as const,
  reviewer: "fake" as const,
  review_rounds: 1,
};

function snapshot(
  overrides: Partial<PlanRepairSessionSnapshot> = {},
): PlanRepairSessionSnapshot {
  return {
    request: {
      id: "plan_repair_request_0001",
      plan_id: "work_item_plan_0001",
      base_plan_revision_id: "plan_revision_0001",
      trigger_attempt_id: "coding_attempt_0001",
      trigger_unit_run_id: "unit_run_0001",
      trigger_review_id: "code_review_0001",
      trigger_finding_id: "finding_0001",
      amendment_id: "plan_amendment_0001",
      defect_class: "upstream_contract_invalid",
      reason_code: "contract_mismatch",
      repair_target: {
        kind: "upstream_work_item",
        logical_work_item_ids: ["work_item_0001"],
        work_item_revision_ids: ["work_item_revision_0001"],
      },
      contract_refs: ["contract.api.users"],
      capability_refs: ["capability.users.read"],
      evidence: [],
      fingerprint: "repair_fingerprint_0001",
      status: "awaiting_confirmation",
      created_at: "2026-07-18T00:00:00Z",
      updated_at: "2026-07-18T00:05:00Z",
    },
    link: {
      id: "workspace_session_link_0001",
      relation: "plan_repair",
      parent_session_id: "coding_attempt_0001",
      child_session_id: "workspace_session_repair_0001",
      trigger: {
        attempt_id: "coding_attempt_0001",
        unit_run_id: "unit_run_0001",
        review_id: "code_review_0001",
        finding_id: "finding_0001",
        repair_request_id: "plan_repair_request_0001",
        amendment_id: "plan_amendment_0001",
        fingerprint: "repair_fingerprint_0001",
        base_plan_revision_id: "plan_revision_0001",
      },
      return_context: {
        original_attempt_id: "coding_attempt_0001",
        original_unit_run_id: "unit_run_0001",
        timeline_anchor_id: "finding_0001",
        original_route:
          "/workbench/projects/project_0001/issues/issue_0001/coding/coding_attempt_0001",
      },
      created_at: "2026-07-18T00:00:01Z",
    },
    stage: "awaiting_confirmation",
    projection: null,
    amendment: null,
    validation: null,
    impact: null,
    plan_review: null,
    package_identity: null,
    candidate_package_artifact_id: null,
    impact_scope_review: null,
    timeline_nodes: [
      {
        node_id: "plan_repair_node_0001",
        node_type: "human_confirm",
        agent: null,
        stage: "human_confirm",
        round: null,
        status: "paused",
        title: "确认 Plan Amendment",
        summary: "等待确认影响范围",
        started_at: "2026-07-18T00:04:00Z",
        completed_at: null,
        artifact_ref: "plan_amendment_0001",
        provider_config_snapshot: providerConfig,
        retry: null,
      },
    ],
    error: null,
    ...overrides,
  };
}

function codingSessionState(
  linkedPlanRepair: PlanRepairSessionSnapshot,
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
    status: "awaiting_plan_amendment",
    stage: "testing",
    branch_name: "aria/work-items/work_item_0001/attempt-1",
    base_branch: "main",
    worktree_path: "/tmp/worktree",
    rework_count: 0,
    max_auto_rework: 2,
    head_commit: null,
    pushed_remote: null,
    role_provider_config_snapshot: {
      coder: "fake",
      tester_plan: "fake",
      tester_execute: "fake",
      code_reviewer: "fake",
      internal_reviewer: "fake",
      review_rounds: 1,
      permission_modes: {
        coder: "supervised",
        tester: "auto",
        code_reviewer: "supervised",
        internal_reviewer: "supervised",
      },
    },
    provider_config_snapshot: providerConfig,
    chat_entries: [],
    timeline_nodes: [],
    active_node_id: null,
    testing_report: null,
    code_review_reports: [],
    review_request: null,
    internal_pr_review: null,
    pending_gates: [],
    pending_choices: [],
    role_runs: [],
    work_item_markdown: null,
    verification_commands: [],
    work_item_execution_plan: null,
    work_item_handoff: null,
    linked_plan_repair: linkedPlanRepair,
    require_execution_plan_confirm: false,
  };
}

describe("plan repair session aggregation", () => {
  beforeEach(() => {
    useCodingWorkspaceStore.getState().reset();
  });

  it("deduplicates child timeline nodes restored by reconnect", () => {
    const store = useCodingWorkspaceStore.getState();

    store.setSessionState(codingSessionState(snapshot()));
    store.setSessionState(codingSessionState(snapshot()));

    const state = useCodingWorkspaceStore.getState();
    expect(state.timelineNodes.filter((node) => node.id === "plan_repair_node_0001"))
      .toHaveLength(1);
    expect(state.activePlanRepair?.timelineNodes).toHaveLength(1);
  });

  it("does not downgrade a richer child stage on duplicate repair-required delivery", () => {
    const store = useCodingWorkspaceStore.getState();
    const validatingSnapshot = snapshot({
      request: {
        ...snapshot().request,
        status: "in_progress",
      },
      stage: "validating_contract",
    });

    store.setSessionState(codingSessionState(validatingSnapshot));
    store.setPlanRepairRequired({
      request: validatingSnapshot.request,
      session_link: validatingSnapshot.link,
    });

    expect(useCodingWorkspaceStore.getState().activePlanRepair?.stage).toBe(
      "validating_contract",
    );
  });

  it("restores the linked repair snapshot without changing the parent coding identity", () => {
    const store = useCodingWorkspaceStore.getState();

    store.setSessionState(codingSessionState(snapshot()));

    const state = useCodingWorkspaceStore.getState();
    expect(state.attemptId).toBe("coding_attempt_0001");
    expect(state.projectId).toBe("project_0001");
    expect(state.issueId).toBe("issue_0001");
    expect(state.activePlanRepair).toMatchObject({
      childSessionId: "workspace_session_repair_0001",
      stage: "awaiting_confirmation",
    });
    expect(state.timelineNodes).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ id: "plan_repair_node_0001" }),
      ]),
    );
  });
});
