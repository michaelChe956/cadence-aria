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

  it.each([
    ["link id", (value: PlanRepairSessionSnapshot) => ({
      ...value,
      link: { ...value.link, id: "workspace_session_link_collision" },
    })],
    ["trigger unit run", (value: PlanRepairSessionSnapshot) => ({
      ...value,
      link: {
        ...value.link,
        trigger: { ...value.link.trigger, unit_run_id: "unit_run_collision" },
      },
    })],
    ["trigger review", (value: PlanRepairSessionSnapshot) => ({
      ...value,
      link: {
        ...value.link,
        trigger: { ...value.link.trigger, review_id: "code_review_collision" },
      },
    })],
    ["trigger finding", (value: PlanRepairSessionSnapshot) => ({
      ...value,
      link: {
        ...value.link,
        trigger: { ...value.link.trigger, finding_id: "finding_collision" },
      },
    })],
    ["return unit run", (value: PlanRepairSessionSnapshot) => ({
      ...value,
      link: {
        ...value.link,
        return_context: {
          ...value.link.return_context,
          original_unit_run_id: "unit_run_collision",
        },
      },
    })],
    ["timeline anchor", (value: PlanRepairSessionSnapshot) => ({
      ...value,
      link: {
        ...value.link,
        return_context: {
          ...value.link.return_context,
          timeline_anchor_id: "finding_collision",
        },
      },
    })],
    ["original route", (value: PlanRepairSessionSnapshot) => ({
      ...value,
      link: {
        ...value.link,
        return_context: {
          ...value.link.return_context,
          original_route: "/workbench/projects/collision",
        },
      },
    })],
  ])("rejects %s collisions for the current durable repair identity", (_name, mutate) => {
    const store = useCodingWorkspaceStore.getState();
    const current = snapshot();
    store.setSessionState(codingSessionState(current));
    const before = useCodingWorkspaceStore.getState().activePlanRepair;
    const collision = mutate(current);

    store.setPlanRepairRequired({
      request: collision.request,
      session_link: collision.link,
    });
    store.updatePlanRepairSession(collision);

    expect(useCodingWorkspaceStore.getState().activePlanRepair).toEqual(before);
  });

  it("keeps a different repair request stable until the current durable repair clears", () => {
    const store = useCodingWorkspaceStore.getState();
    const current = snapshot();
    const next = differentRepairSnapshot(current);
    store.setSessionState(codingSessionState(current));

    store.setPlanRepairRequired({ request: next.request, session_link: next.link });

    expect(useCodingWorkspaceStore.getState().activePlanRepair?.request.id).toBe(
      current.request.id,
    );
  });

  it("accepts a new repair only after a durable terminal snapshot clears the current repair", () => {
    const store = useCodingWorkspaceStore.getState();
    const current = snapshot();
    const next = differentRepairSnapshot(current);
    store.setSessionState(codingSessionState(current));

    store.updatePlanRepairSession({
      ...current,
      request: {
        ...current.request,
        status: "cancelled",
        updated_at: "2026-07-18T00:08:00Z",
      },
      stage: "failed",
    });
    expect(useCodingWorkspaceStore.getState().activePlanRepair).toBeNull();

    store.setPlanRepairRequired({ request: next.request, session_link: next.link });
    expect(useCodingWorkspaceStore.getState().activePlanRepair?.request.id).toBe(
      next.request.id,
    );
  });

  it("rejects the legacy empty amendment identity instead of treating it as a wildcard", () => {
    const store = useCodingWorkspaceStore.getState();
    const current = snapshot({
      link: {
        ...snapshot().link,
        trigger: {
          ...snapshot().link.trigger,
          amendment_id: "",
        },
      },
    });
    store.setSessionState(codingSessionState(current));

    expect(useCodingWorkspaceStore.getState().activePlanRepair).toBeNull();
  });

  it("rejects an initial repair route with a forged project and issue scope", () => {
    const store = useCodingWorkspaceStore.getState();
    const forged = snapshot({
      link: {
        ...snapshot().link,
        return_context: {
          ...snapshot().link.return_context,
          original_route: "/forged/coding/coding_attempt_0001",
        },
      },
    });

    store.setSessionState(codingSessionState(forged));

    expect(useCodingWorkspaceStore.getState().activePlanRepair).toBeNull();
  });

  it("reconciles parent reconnect without losing richer child stage, history, or timeline", () => {
    const store = useCodingWorkspaceStore.getState();
    const current = snapshot({
      stage: "validating_contract",
      request: {
        ...snapshot().request,
        status: "in_progress",
      },
    });
    store.setSessionState(codingSessionState(current));
    store.setPlanRepairHistory(current, {
      entries: [historyEntry("unit_run_live")],
    });
    store.addPlanRepairTimelineNode(current, liveTimelineNode({
      node_id: "plan_repair_node_live",
      started_at: "2026-07-18T00:07:00Z",
    }));

    store.setSessionState(codingSessionState({
      ...current,
      stage: "authoring_revision",
      timeline_nodes: current.timeline_nodes,
    }));

    expect(useCodingWorkspaceStore.getState().activePlanRepair).toMatchObject({
      stage: "validating_contract",
      history: { entries: [expect.objectContaining({ id: "unit_run_live" })] },
      timelineNodes: expect.arrayContaining([
        expect.objectContaining({ id: "plan_repair_node_live" }),
      ]),
    });
  });

  it("clears a completed repair from an authoritative resumed parent reconnect", () => {
    const store = useCodingWorkspaceStore.getState();
    const current = snapshot();
    store.setSessionState(codingSessionState(current));

    store.setSessionState({
      ...codingSessionState(current),
      status: "running",
      linked_plan_repair: null,
    });

    expect(useCodingWorkspaceStore.getState().activePlanRepair).toBeNull();
  });

  it("preserves repair state when a paused parent reconnect omits the child snapshot", () => {
    const store = useCodingWorkspaceStore.getState();
    const current = snapshot();
    store.setSessionState(codingSessionState(current));

    store.setSessionState({
      ...codingSessionState(current),
      linked_plan_repair: null,
    });

    expect(useCodingWorkspaceStore.getState().activePlanRepair?.request.id).toBe(
      current.request.id,
    );
  });

  it("prefers a newer completed snapshot node over an older live active node", () => {
    const store = useCodingWorkspaceStore.getState();
    const current = snapshot({ timeline_nodes: [] });
    store.setSessionState(codingSessionState(current));
    store.addPlanRepairTimelineNode(
      current,
      liveTimelineNode({ status: "active", completed_at: null }),
    );

    store.updatePlanRepairSession({
      ...current,
      request: { ...current.request, updated_at: "2026-07-18T00:08:00Z" },
      timeline_nodes: [
        liveTimelineNode({
          status: "completed",
          summary: "权威完成",
          completed_at: "2026-07-18T00:07:30Z",
        }),
      ],
    });

    expect(useCodingWorkspaceStore.getState().activePlanRepair?.timelineNodes).toEqual([
      expect.objectContaining({ status: "completed", summary: "权威完成" }),
    ]);
  });

  it("does not let a late active create regress a terminal plan repair snapshot", () => {
      const store = useCodingWorkspaceStore.getState();
      const base = snapshot();
      const authoritative = snapshot({
        request: { ...base.request, updated_at: "2026-07-18T00:08:00Z" },
        timeline_nodes: [
          liveTimelineNode({
            status: "completed",
            summary: "权威完成",
            completed_at: "2026-07-18T00:07:30Z",
          }),
        ],
      });
      store.setSessionState(codingSessionState(authoritative));

      store.addPlanRepairTimelineNode(
        authoritative,
        liveTimelineNode({
          status: "active",
          summary: "迟到运行中",
          started_at: "2026-07-18T00:04:00Z",
          completed_at: null,
        }),
      );

      expect(useCodingWorkspaceStore.getState().activePlanRepair?.timelineNodes).toEqual([
        expect.objectContaining({ status: "completed", summary: "权威完成" }),
      ]);
  });

  it("does not let a late paused update regress a failed plan repair snapshot", () => {
      const store = useCodingWorkspaceStore.getState();
      const base = snapshot();
      const authoritative = snapshot({
        request: { ...base.request, updated_at: "2026-07-18T00:08:00Z" },
        timeline_nodes: [
          liveTimelineNode({
            status: "failed",
            summary: "权威失败",
            completed_at: "2026-07-18T00:07:30Z",
          }),
        ],
      });
      store.setSessionState(codingSessionState(authoritative));

      store.updatePlanRepairTimelineNode(
        authoritative,
        "plan_repair_node_0001",
        "paused",
        "迟到暂停",
        null,
      );

      expect(useCodingWorkspaceStore.getState().activePlanRepair?.timelineNodes).toEqual([
        expect.objectContaining({ status: "failed", summary: "权威失败" }),
      ]);
  });

  it.each(["story_amendment", "design_amendment"] as const)(
    "keeps %s child timelines outside the coding plan repair aggregator",
    (relation) => {
      const linked = snapshot({
        link: { ...snapshot().link, relation },
      });

      useCodingWorkspaceStore.getState().setSessionState(codingSessionState(linked));

      expect(useCodingWorkspaceStore.getState().activePlanRepair).toBeNull();
    },
  );

  it("rejects a late unknown create at or below the authoritative snapshot watermark", () => {
    const store = useCodingWorkspaceStore.getState();
    const authoritative = snapshot({
      request: { ...snapshot().request, updated_at: "2026-07-18T00:08:00Z" },
      timeline_nodes: [],
    });
    store.setSessionState(codingSessionState(authoritative));

    store.addPlanRepairTimelineNode(
      authoritative,
      liveTimelineNode({
        node_id: "late_unknown",
        started_at: "2026-07-18T00:04:00Z",
      }),
    );

    expect(useCodingWorkspaceStore.getState().activePlanRepair?.timelineNodes).toEqual([]);
  });

  it("drops an omitted old live node but keeps one newer than the snapshot watermark", () => {
    const store = useCodingWorkspaceStore.getState();
    const current = snapshot({ timeline_nodes: [] });
    store.setSessionState(codingSessionState(current));
    store.addPlanRepairTimelineNode(
      current,
      liveTimelineNode({ node_id: "old_live", started_at: "2026-07-18T00:04:00Z" }),
    );
    store.addPlanRepairTimelineNode(
      current,
      liveTimelineNode({ node_id: "new_live", started_at: "2026-07-18T00:09:00Z" }),
    );

    store.updatePlanRepairSession({
      ...current,
      request: { ...current.request, updated_at: "2026-07-18T00:08:00Z" },
      timeline_nodes: [],
    });

    expect(
      useCodingWorkspaceStore.getState().activePlanRepair?.timelineNodes.map((node) => node.id),
    ).toEqual(["new_live"]);
  });

  it.each(["completed", "failed"] as const)(
    "keeps an omitted %s node only when its last event is newer than the snapshot watermark",
    (status) => {
      const store = useCodingWorkspaceStore.getState();
      const current = snapshot({ timeline_nodes: [] });
      store.setSessionState(codingSessionState(current));
      store.addPlanRepairTimelineNode(
        current,
        liveTimelineNode({ node_id: "old_terminal", started_at: "2026-07-18T00:04:00Z" }),
      );
      store.updatePlanRepairTimelineNode(
        current,
        "old_terminal",
        status,
        "old terminal",
        "2026-07-18T00:07:00Z",
      );
      store.addPlanRepairTimelineNode(
        current,
        liveTimelineNode({ node_id: "new_terminal", started_at: "2026-07-18T00:09:00Z" }),
      );
      store.updatePlanRepairTimelineNode(
        current,
        "new_terminal",
        status,
        "new terminal",
        "2026-07-18T00:10:00Z",
      );

      store.updatePlanRepairSession({
        ...current,
        request: { ...current.request, updated_at: "2026-07-18T00:08:00Z" },
        timeline_nodes: [],
      });

      expect(
        useCodingWorkspaceStore
          .getState()
          .activePlanRepair?.timelineNodes.map((node) => [node.id, node.status]),
      ).toEqual([["new_terminal", status]]);
    },
  );

  it("does not downgrade stage for an equal-version child snapshot", () => {
    const store = useCodingWorkspaceStore.getState();
    const current = snapshot({ stage: "validating_contract" });
    store.setSessionState(codingSessionState(current));

    store.updatePlanRepairSession({ ...current, stage: "authoring_revision" });

    expect(useCodingWorkspaceStore.getState().activePlanRepair?.stage).toBe(
      "validating_contract",
    );
  });
});

function differentRepairSnapshot(
  current: PlanRepairSessionSnapshot,
): PlanRepairSessionSnapshot {
  return {
    ...current,
    request: {
      ...current.request,
      id: "plan_repair_request_0002",
      trigger_unit_run_id: "unit_run_0002",
      trigger_review_id: "code_review_0002",
      trigger_finding_id: "finding_0002",
      amendment_id: "plan_amendment_0002",
      fingerprint: "repair_fingerprint_0002",
      updated_at: "2026-07-18T00:09:00Z",
    },
    link: {
      ...current.link,
      id: "workspace_session_link_0002",
      child_session_id: "workspace_session_repair_0002",
      trigger: {
        ...current.link.trigger,
        unit_run_id: "unit_run_0002",
        review_id: "code_review_0002",
        finding_id: "finding_0002",
        repair_request_id: "plan_repair_request_0002",
        amendment_id: "plan_amendment_0002",
        fingerprint: "repair_fingerprint_0002",
      },
      return_context: {
        ...current.link.return_context,
        original_unit_run_id: "unit_run_0002",
        timeline_anchor_id: "finding_0002",
      },
    },
    timeline_nodes: [],
  };
}

function historyEntry(id: string) {
  return {
    kind: "unit_run" as const,
    id,
    logical_work_item_id: "work_item_0001",
    related_revision_id: "work_item_revision_0001",
    summary: "runtime history",
    created_at: "2026-07-18T00:07:00Z",
  };
}

function liveTimelineNode(
  overrides: Partial<PlanRepairSessionSnapshot["timeline_nodes"][number]> = {},
): PlanRepairSessionSnapshot["timeline_nodes"][number] {
  return {
    node_id: "plan_repair_node_shared",
    node_type: "author_run",
    agent: "fake",
    stage: "running",
    round: 1,
    status: "active",
    title: "生成 Plan Amendment",
    summary: null,
    started_at: "2026-07-18T00:06:00Z",
    completed_at: null,
    duration_ms: null,
    artifact_ref: null,
    provider_config_snapshot: providerConfig,
    retry: null,
    ...overrides,
  };
}
