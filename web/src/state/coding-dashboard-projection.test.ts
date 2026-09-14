import { describe, expect, it } from "vitest";
import type { CodingExecutionUnit, CodingTimelineNode } from "../api/types";
import {
  CODING_BUDGET_CODING_TOTAL_MS,
  CODING_BUDGET_NEAR_EXHAUSTION_MS,
  CODING_BUDGET_WORK_ITEM_TOTAL_MS,
  CODING_UNIT_STATE_LABELS,
  codingUnitState,
  selectCodingBudgetGates,
  selectCodingDependencyChain,
  selectCodingTopology,
} from "./coding-dashboard-projection";

function unit(overrides: Partial<CodingExecutionUnit> = {}): CodingExecutionUnit {
  return {
    unit_id: "unit_0001",
    logical_work_item_id: "wi_0001",
    work_item_revision_id: "rev_0001",
    dependency_logical_work_item_ids: [],
    order_index: 0,
    status: "pending",
    summary: "单元一",
    latest_handoff_revision_id: null,
    completion_commit: null,
    ...overrides,
  };
}

function codingNode(overrides: Partial<CodingTimelineNode> = {}): CodingTimelineNode {
  return {
    id: "node_coding_1",
    attempt_id: "coding_attempt_0001",
    stage: "coding",
    title: "Coder",
    status: "running",
    agent_role: "author",
    summary: null,
    started_at: "2026-09-14T08:00:00Z",
    completed_at: null,
    artifact_refs: [],
    ...overrides,
  };
}

describe("codingUnitState", () => {
  it("maps every coding unit status onto the six topology states", () => {
    expect(codingUnitState("running")).toBe("running");
    expect(codingUnitState("waiting_for_human")).toBe("awaiting_triage");
    expect(codingUnitState("completed")).toBe("done");
    expect(codingUnitState("failed")).toBe("failed");
    for (const status of ["blocked", "blocked_by_plan_defect", "awaiting_amendment", "needs_revalidation"] as const) {
      expect(codingUnitState(status)).toBe("blocked");
    }
    for (const status of ["pending", "stale", "superseded", "skipped"] as const) {
      expect(codingUnitState(status)).toBe("pending");
    }
  });

  it("labels each of the six topology states", () => {
    expect(Object.keys(CODING_UNIT_STATE_LABELS)).toEqual([
      "running",
      "pending",
      "blocked",
      "done",
      "failed",
      "awaiting_triage",
    ]);
  });
});

describe("selectCodingTopology", () => {
  it("dedupes work item revisions and labels dependency edges as satisfied or blocking", () => {
    const topology = selectCodingTopology([
      unit({ unit_id: "u1", logical_work_item_id: "wi_a", status: "completed", order_index: 0 }),
      unit({ unit_id: "u1r2", logical_work_item_id: "wi_a", status: "superseded", order_index: 0 }),
      unit({
        unit_id: "u2",
        logical_work_item_id: "wi_b",
        status: "running",
        summary: null,
        order_index: 1,
        dependency_logical_work_item_ids: ["wi_a"],
      }),
      unit({
        unit_id: "u3",
        logical_work_item_id: "wi_c",
        status: "blocked",
        order_index: 2,
        dependency_logical_work_item_ids: ["wi_b"],
      }),
    ]);

    expect(topology.nodes.map((node) => node.workItemId)).toEqual(["wi_a", "wi_b", "wi_c"]);
    expect(topology.nodes[0]).toMatchObject({ state: "done", unitId: "u1" });
    expect(topology.nodes[1]).toMatchObject({ state: "running", title: "wi_b" });
    expect(topology.edges).toEqual([
      { fromWorkItemId: "wi_a", toWorkItemId: "wi_b", state: "satisfied" },
      { fromWorkItemId: "wi_b", toWorkItemId: "wi_c", state: "blocking" },
    ]);
  });

  it("treats an edge as satisfied once its downstream unit is complete", () => {
    const topology = selectCodingTopology([
      unit({ logical_work_item_id: "wi_a", status: "failed" }),
      unit({ logical_work_item_id: "wi_b", status: "completed", dependency_logical_work_item_ids: ["wi_a"] }),
    ]);

    expect(topology.edges).toEqual([
      { fromWorkItemId: "wi_a", toWorkItemId: "wi_b", state: "satisfied" },
    ]);
  });
});

describe("selectCodingDependencyChain", () => {
  const units = [
    unit({ logical_work_item_id: "wi_a", status: "completed", order_index: 0 }),
    unit({ logical_work_item_id: "wi_b", status: "failed", order_index: 1 }),
    unit({ logical_work_item_id: "wi_c", status: "running", order_index: 2, dependency_logical_work_item_ids: ["wi_b"] }),
    unit({ logical_work_item_id: "wi_d", status: "pending", order_index: 3, dependency_logical_work_item_ids: ["wi_c"] }),
    unit({ logical_work_item_id: "wi_e", status: "pending", order_index: 4, dependency_logical_work_item_ids: ["wi_a"] }),
  ];

  it("expands upstream providers and downstream consumers and pinpoints blocking sources", () => {
    const chain = selectCodingDependencyChain(units, "wi_c");
    expect(chain?.upstream.map((node) => node.workItemId)).toEqual(["wi_b"]);
    expect(chain?.downstream.map((node) => node.workItemId)).toEqual(["wi_d"]);
    expect(chain?.blockingSources.map((node) => node.workItemId)).toEqual(["wi_b"]);

    const chainD = selectCodingDependencyChain(units, "wi_d");
    expect(chainD?.upstream.map((node) => node.workItemId)).toEqual(["wi_c", "wi_b"]);
    expect(chainD?.blockingSources.map((node) => node.workItemId)).toEqual(["wi_c", "wi_b"]);
  });

  it("reports no blocking sources when the whole upstream is done", () => {
    expect(selectCodingDependencyChain(units, "wi_e")?.blockingSources).toEqual([]);
  });

  it("returns null for an unknown work item", () => {
    expect(selectCodingDependencyChain(units, "wi_missing")).toBeNull();
  });
});

describe("selectCodingBudgetGates", () => {
  const NOW = Date.parse("2026-09-14T09:30:00Z");

  it("derives both budget gates from frontend timing over coding stage nodes", () => {
    const gates = selectCodingBudgetGates(
      [
        codingNode({ id: "c1", started_at: "2026-09-14T08:00:00Z" }),
        codingNode({ id: "c2", started_at: "2026-09-14T09:00:00Z" }),
      ],
      NOW,
    );
    expect(gates.map((gate) => gate.kind)).toEqual(["work_item", "coding"]);

    const workItem = gates[0];
    expect(workItem).toMatchObject({
      totalMs: CODING_BUDGET_WORK_ITEM_TOTAL_MS,
      anchorAtMs: Date.parse("2026-09-14T09:00:00Z"),
      elapsedMs: 1_800_000,
      remainingMs: CODING_BUDGET_WORK_ITEM_TOTAL_MS - 1_800_000,
      nearExhaustion: false,
    });

    const coding = gates[1];
    expect(coding).toMatchObject({
      totalMs: CODING_BUDGET_CODING_TOTAL_MS,
      anchorAtMs: Date.parse("2026-09-14T08:00:00Z"),
      elapsedMs: CODING_BUDGET_CODING_TOTAL_MS,
      remainingMs: 0,
      ratio: 1,
      nearExhaustion: true,
    });
  });

  it("freezes elapsed at the last coding node completion instead of counting past it", () => {
    const gates = selectCodingBudgetGates(
      [codingNode({ started_at: "2026-09-14T08:00:00Z", completed_at: "2026-09-14T08:20:00Z", status: "completed" })],
      NOW,
    );
    expect(gates[0]?.elapsedMs).toBe(1_200_000);
  });

  it("returns no gates before the coding stage begins", () => {
    expect(selectCodingBudgetGates([], NOW)).toEqual([]);
    expect(
      selectCodingBudgetGates([codingNode({ stage: "code_review", started_at: "2026-09-14T08:00:00Z" })], NOW),
    ).toEqual([]);
  });

  it("marks near exhaustion within the last ten minutes", () => {
    const gates = selectCodingBudgetGates(
      [codingNode({ started_at: "2026-09-14T08:00:00Z" })],
      Date.parse("2026-09-14T08:52:00Z"),
    );
    expect(gates[0]?.remainingMs).toBe(CODING_BUDGET_WORK_ITEM_TOTAL_MS - 3_120_000);
    expect(gates[0]?.remainingMs).toBeLessThanOrEqual(CODING_BUDGET_NEAR_EXHAUSTION_MS);
    expect(gates[0]?.nearExhaustion).toBe(true);
    expect(gates[1]?.nearExhaustion).toBe(false);
  });
});
