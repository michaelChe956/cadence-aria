import { act } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { PlanAmendmentManifest, PlanRepairSessionSnapshot } from "../api/types";
import { useCodingWorkspaceStore } from "../state/coding-workspace-store";
import {
  codingSessionState,
  installCodingWorkspaceWsTestHooks,
  renderCodingHook,
} from "./useCodingWorkspaceWs.test-utils";

describe("useCodingWorkspaceWs plan repair events", () => {
  installCodingWorkspaceWsTestHooks();

  it("links a repair child without changing the current coding websocket address", () => {
    const harness = renderCodingHook();
    const current = repairSnapshot();

    act(() => {
      harness.ws.receive(codingSessionState());
      harness.ws.receive({
        type: "plan_repair_required",
        request: current.request,
        session_link: current.link,
      });
    });

    const state = useCodingWorkspaceStore.getState();
    expect(harness.ws.url).toContain("/coding-attempts/coding_attempt_0001");
    expect(harness.ws.url).not.toContain("workspace_session_repair_0001");
    expect(state.attemptId).toBe("coding_attempt_0001");
    expect(state.status).toBe("awaiting_plan_amendment");
    expect(state.activePlanRepair?.childSessionId).toBe("workspace_session_repair_0001");
  });

  it("only clears the active repair for the matching applied amendment delivery", () => {
    const harness = renderCodingHook();
    const current = repairSnapshot();

    act(() => {
      harness.ws.receive(
        codingSessionState({
          status: "awaiting_plan_amendment",
          linked_plan_repair: current,
        }),
      );
      harness.ws.receive({
        type: "plan_amendment_updated",
        event_id: "coding_plan_amendment_updated_attempt_0001_plan_amendment_stale",
        amendment: amendment({
          id: "plan_amendment_stale",
          repair_request_id: "plan_repair_request_stale",
        }),
      });
    });

    expect(useCodingWorkspaceStore.getState().activePlanRepair).not.toBeNull();

    act(() => {
      harness.ws.receive({
        type: "plan_amendment_updated",
        event_id: "coding_plan_amendment_updated_attempt_0001_wrong_request",
        amendment: amendment({
          repair_request_id: "plan_repair_request_stale",
        }),
      });
    });

    expect(useCodingWorkspaceStore.getState().activePlanRepair).not.toBeNull();

    act(() => {
      harness.ws.receive({
        type: "plan_amendment_updated",
        event_id: "coding_plan_amendment_updated_attempt_0001_wrong_base",
        amendment: amendment({
          previous_plan_revision_id: "plan_revision_stale",
        }),
      });
    });

    expect(useCodingWorkspaceStore.getState().activePlanRepair).not.toBeNull();

    act(() => {
      harness.ws.receive({
        type: "plan_amendment_updated",
        event_id: "coding_plan_amendment_updated_attempt_0001_plan_amendment_0001",
        amendment: amendment(),
      });
    });

    expect(useCodingWorkspaceStore.getState().activePlanRepair).toBeNull();
  });
});

function repairSnapshot(
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
    timeline_nodes: [],
    error: null,
    ...overrides,
  };
}

function amendment(
  overrides: Partial<PlanAmendmentManifest> = {},
): PlanAmendmentManifest {
  return {
    id: "plan_amendment_0001",
    repair_request_id: "plan_repair_request_0001",
    previous_plan_revision_id: "plan_revision_0001",
    new_plan_revision_id: "plan_revision_0002",
    revised_work_items: {},
    superseded_revisions: [],
    dependency_graph_changes: [],
    contract_deltas: [],
    unaffected_units: [],
    revalidation_required_units: ["unit_run_0001"],
    stale_units: [],
    replacement_units: {},
    resume_target: {
      logical_work_item_id: "work_item_0001",
      mode: "revalidate",
    },
    created_at: "2026-07-18T00:06:00Z",
    ...overrides,
  };
}
