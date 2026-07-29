import { act } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type {
  PlanAmendmentManifest,
  PlanRepairSessionSnapshot,
} from "../api/types";
import { useCodingWorkspaceStore } from "../state/coding-workspace-store";
import {
  installWorkspaceWsTestHooks,
  renderWorkspaceHook,
} from "./useWorkspaceWs.test-utils";

describe("useWorkspaceWs plan repair projection", () => {
  installWorkspaceWsTestHooks();

  it("projects the active child snapshot, timeline, and shared history DTO", () => {
    const current = childRepairSnapshot();
    setActiveRepair(current);
    const harness = renderWorkspaceHook(current.link.child_session_id);
    const updated = childRepairSnapshot({
      stage: "authoring_revision",
      request: {
        ...current.request,
        status: "in_progress",
        updated_at: "2026-07-18T00:06:00Z",
      },
      timeline_nodes: [],
    });
    const node = {
      ...current.timeline_nodes[0],
      started_at: "2026-07-18T00:06:30Z",
    };

    act(() => {
      harness.ws.receive(workspaceSessionState(updated));
      harness.ws.receive({ type: "timeline_node_created", node });
      harness.ws.receive({ type: "timeline_node_created", node });
      harness.ws.receive({
        type: "timeline_node_updated",
        node_id: node.node_id,
        status: "completed",
        summary: "Plan Amendment 已生成",
        completed_at: "2026-07-18T00:07:00Z",
      });
      harness.ws.receive({
        type: "artifact_update",
        version: 3,
        work_item_revision_history: {
          entries: [
            {
              kind: "unit_run",
              id: "unit_run_0001",
              logical_work_item_id: "work_item_0001",
              related_revision_id: "work_item_revision_0001",
              summary: "Coder 发现契约不匹配",
              created_at: "2026-07-18T00:00:00Z",
            },
          ],
        },
      });
    });

    const coding = useCodingWorkspaceStore.getState();
    expect(coding.activePlanRepair).toMatchObject({
      childSessionId: "workspace_session_repair_0001",
      stage: "authoring_revision",
      history: {
        entries: [expect.objectContaining({ kind: "unit_run", id: "unit_run_0001" })],
      },
    });
    expect(coding.activePlanRepair?.timelineNodes).toEqual([
      expect.objectContaining({
        id: "plan_repair_node_0001",
        status: "completed",
        summary: "Plan Amendment 已生成",
      }),
    ]);
    expect(
      coding.timelineNodes.filter((item) => item.id === "plan_repair_node_0001"),
    ).toHaveLength(1);
  });

  it("ignores a child snapshot that does not match the active repair link", () => {
    const current = childRepairSnapshot();
    setActiveRepair(current);
    const harness = renderWorkspaceHook(current.link.child_session_id);
    const stale = childRepairSnapshot({
      request: {
        ...current.request,
        id: "plan_repair_request_stale",
        updated_at: "2026-07-18T00:01:00Z",
      },
      link: {
        ...current.link,
        id: "workspace_session_link_stale",
        child_session_id: "workspace_session_repair_stale",
        trigger: {
          ...current.link.trigger,
          repair_request_id: "plan_repair_request_stale",
        },
      },
      stage: "triaging",
      timeline_nodes: [],
    });

    act(() => {
      harness.ws.receive(workspaceSessionState(stale));
    });

    expect(useCodingWorkspaceStore.getState().activePlanRepair).toMatchObject({
      childSessionId: "workspace_session_repair_0001",
      stage: "awaiting_confirmation",
    });
  });

  it.each([
    ["link id", (value: PlanRepairSessionSnapshot) => ({
      ...value,
      link: { ...value.link, id: "workspace_session_link_collision" },
    })],
    ["trigger", (value: PlanRepairSessionSnapshot) => ({
      ...value,
      link: {
        ...value.link,
        trigger: { ...value.link.trigger, unit_run_id: "unit_run_collision" },
      },
    })],
    ["return context", (value: PlanRepairSessionSnapshot) => ({
      ...value,
      link: {
        ...value.link,
        return_context: {
          ...value.link.return_context,
          original_route: "/workbench/projects/collision",
        },
      },
    })],
  ])("fails closed for %s collisions before applying child artifacts", (_name, mutate) => {
    const current = childRepairSnapshot();
    setActiveRepair(current);
    const harness = renderWorkspaceHook(current.link.child_session_id);

    act(() => {
      harness.ws.receive(workspaceSessionState(mutate(current)));
      harness.ws.receive({
        type: "artifact_update",
        version: 3,
        work_item_revision_history: {
          entries: [
            {
              kind: "unit_run",
              id: "unit_run_collision",
              logical_work_item_id: "work_item_0001",
              related_revision_id: "work_item_revision_0001",
              summary: "stale child artifact",
              created_at: "2026-07-18T00:06:00Z",
            },
          ],
        },
      });
    });

    expect(useCodingWorkspaceStore.getState().activePlanRepair).toMatchObject({
      link: current.link,
      history: null,
    });
  });

  it("keeps an out-of-order timeline event when a later snapshot omits it", () => {
    const current = childRepairSnapshot({
      stage: "triaging",
      request: {
        ...childRepairSnapshot().request,
        status: "in_progress",
        updated_at: "2026-07-18T00:04:00Z",
      },
      timeline_nodes: [],
    });
    setActiveRepair(current);
    const harness = renderWorkspaceHook(current.link.child_session_id);
    const node = {
      ...childRepairSnapshot().timeline_nodes[0],
      started_at: "2026-07-18T00:07:00Z",
    };
    const laterSnapshot = childRepairSnapshot({
      stage: "authoring_revision",
      request: {
        ...current.request,
        updated_at: "2026-07-18T00:06:00Z",
      },
      timeline_nodes: [],
    });

    act(() => {
      harness.ws.receive({ type: "timeline_node_created", node });
      harness.ws.receive(workspaceSessionState(laterSnapshot));
    });

    expect(useCodingWorkspaceStore.getState().activePlanRepair).toMatchObject({
      stage: "authoring_revision",
      timelineNodes: [expect.objectContaining({ id: "plan_repair_node_0001" })],
    });
  });

  it("restores the manifest from authoritative child artifact versions", () => {
    const current = childRepairSnapshot();
    const manifest = amendmentManifest();
    setActiveRepair(current);
    const harness = renderWorkspaceHook(current.link.child_session_id);

    act(() => {
      harness.ws.receive(workspaceSessionState(current, [
        {
          version: 4,
          plan_amendment_manifest: manifest,
          generated_by: "fake",
          reviewed_by: null,
          review_verdict: null,
          confirmed_by: null,
          is_current: true,
          created_at: manifest.created_at,
          source_node_id: "plan_repair_node_0001",
        },
      ]));
    });

    expect(useCodingWorkspaceStore.getState().activePlanRepair?.amendment).toEqual(
      manifest,
    );
  });
});

function setActiveRepair(current: PlanRepairSessionSnapshot) {
  useCodingWorkspaceStore.setState({
    attemptId: "coding_attempt_0001",
    activePlanRepair: {
      ...current,
      childSessionId: current.link.child_session_id,
      childTimelineNodes: current.timeline_nodes,
      timelineNodes: [],
      timelineWatermark: current.request.updated_at,
      history: null,
    },
  } as never);
}

function childRepairSnapshot(
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
        node_type: "author_run",
        agent: "fake",
        stage: "running",
        round: 1,
        status: "active",
        title: "生成 Plan Amendment",
        summary: null,
        started_at: "2026-07-18T00:04:00Z",
        completed_at: null,
        duration_ms: null,
        artifact_ref: "plan_amendment_0001",
        provider_config_snapshot: {
          author: "fake",
          reviewer: "fake",
          review_rounds: 1,
        },
        retry: null,
      },
    ],
    error: null,
    ...overrides,
  };
}

function amendmentManifest(): PlanAmendmentManifest {
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
  };
}

function workspaceSessionState(
  planRepair: PlanRepairSessionSnapshot,
  artifactVersions: unknown[] = [],
) {
  return {
    type: "session_state",
    session_id: planRepair.link.child_session_id,
    workspace_type: "work_item_plan",
    stage: "running",
    superpowers_enabled: true,
    openspec_enabled: true,
    messages: [],
    checkpoints: [],
    artifact: null,
    providers: { author: "fake", reviewer: "fake" },
    timeline_nodes: planRepair.timeline_nodes,
    active_node_id: planRepair.timeline_nodes[0]?.node_id ?? null,
    artifact_versions: artifactVersions,
    artifact_version_summaries: [],
    timeline_node_details: {},
    timeline_node_summaries: {},
    active_run_id: null,
    human_presentation_revisions: [],
    recoverable_interrupted_run: null,
    plan_repair: planRepair,
  };
}
