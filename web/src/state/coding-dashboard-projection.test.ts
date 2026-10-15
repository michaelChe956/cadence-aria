import { describe, expect, it } from "vitest";
import type {
  CodingAttentionItemDto,
  CodingAttemptStatus,
  CodingExecutionUnit,
  CodingTimelineNode,
  PlanCodingProgressDto,
  PlanCodingTargetDto,
  PlanCodingUnitProgressDto,
} from "../api/types";
import {
  CODING_BUDGET_CODING_TOTAL_MS,
  CODING_BUDGET_NEAR_EXHAUSTION_MS,
  CODING_BUDGET_WORK_ITEM_TOTAL_MS,
  CODING_UNIT_STATE_LABELS,
  codingUnitState,
  selectCodingBudgetGates,
  selectCodingDashboardAttempts,
  selectCodingDashboardUnits,
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

function planUnit(
  overrides: Partial<PlanCodingUnitProgressDto> = {},
): PlanCodingUnitProgressDto {
  return {
    unit_id: "coding_unit_0001",
    logical_work_item_id: "WI-001",
    target_repository_id: "repo_busi",
    attempt_id: "coding_attempt_busi_0001",
    unit_status: "pending",
    execution_state: "waiting_slot",
    unsatisfied_dependency_ids: [],
    waiting_reason: null,
    dependency_logical_work_item_ids: [],
    order_index: 0,
    summary: null,
    ...overrides,
  };
}

function planTarget(repositoryId: string, attemptId: string): PlanCodingTargetDto {
  return {
    target_repository_id: repositoryId,
    attempt_id: attemptId,
    attempt_status: "created",
    worktree_path: null,
    lease_state: "absent",
    // FE-1：PlanCodingTargetDto 新增 S3 逐仓事实九字段；本 fixture 只覆盖
    // execution/lease 事实，逐仓交付事实一律 null。
    repository_name: null,
    branch_name: null,
    base_branch: null,
    stage: null,
    head_commit: null,
    push_status: null,
    push_error: null,
    review_request_id: null,
    manual_recovery_reason: null,
  };
}

// owner 场景（issue_work_item_plan_0001）：busi 桶 session units 只有 WI-001，
// plan 全图 5 WI/4 仓；WI-002@api 依赖 busi 桶的 WI-001（跨桶）。
function multiRepoPlanProgressDto(): PlanCodingProgressDto {
  return {
    plan_id: "issue_work_item_plan_0001",
    bound_plan_revision_id: "plan_revision_0002",
    command_id: "plan_start_abc123",
    mode: "first_ready_unit",
    phase: "paused",
    primary_attempt_id: "coding_attempt_busi_0001",
    selected_unit_id: "coding_unit_0001",
    selected_attempt_id: "coding_attempt_busi_0001",
    target_attempts: [
      planTarget("repo_busi", "coding_attempt_busi_0001"),
      planTarget("repo_api", "coding_attempt_api_0001"),
      planTarget("repo_gateway", "coding_attempt_gateway_0001"),
      planTarget("repo_frontend", "coding_attempt_frontend_0001"),
    ],
    units: [
      planUnit({
        unit_id: "coding_unit_0001",
        logical_work_item_id: "WI-001",
        unit_status: "running",
        execution_state: "running",
        dependency_logical_work_item_ids: [],
        order_index: 0,
        summary: "busi 首个单元",
      }),
      planUnit({
        unit_id: "coding_unit_0002",
        logical_work_item_id: "WI-002",
        target_repository_id: "repo_api",
        attempt_id: "coding_attempt_api_0001",
        unit_status: "pending",
        execution_state: "waiting_dependency",
        unsatisfied_dependency_ids: ["WI-001"],
        waiting_reason: "dependency_not_ready",
        dependency_logical_work_item_ids: ["WI-001"],
        order_index: 1,
      }),
      planUnit({
        unit_id: "coding_unit_0003",
        logical_work_item_id: "WI-003",
        target_repository_id: "repo_gateway",
        attempt_id: "coding_attempt_gateway_0001",
        order_index: 2,
      }),
      planUnit({
        unit_id: "coding_unit_0004",
        logical_work_item_id: "WI-004",
        target_repository_id: "repo_frontend",
        attempt_id: "coding_attempt_frontend_0001",
        dependency_logical_work_item_ids: ["WI-003"],
        unsatisfied_dependency_ids: ["WI-003"],
        execution_state: "waiting_dependency",
        waiting_reason: "dependency_not_ready",
        order_index: 3,
      }),
      planUnit({
        unit_id: "coding_unit_0005",
        logical_work_item_id: "WI-005",
        target_repository_id: "repo_frontend",
        attempt_id: "coding_attempt_frontend_0001",
        dependency_logical_work_item_ids: ["WI-003", "WI-004"],
        unsatisfied_dependency_ids: ["WI-003", "WI-004"],
        execution_state: "waiting_dependency",
        waiting_reason: "dependency_not_ready",
        order_index: 4,
      }),
    ],
    plan_complete: false,
  };
}

describe("selectCodingDashboardUnits", () => {
  const busiSessionUnits = [
    unit({
      unit_id: "coding_unit_0001",
      logical_work_item_id: "WI-001",
      status: "running",
      summary: "登录",
    }),
  ];

  it("falls back to session units when no plan aggregate is available", () => {
    expect(selectCodingDashboardUnits(busiSessionUnits, null)).toBe(busiSessionUnits);
  });

  it("keeps session units for a single-target plan (v1 group attempt)", () => {
    const dto = multiRepoPlanProgressDto();
    dto.target_attempts = [dto.target_attempts[0]!];
    expect(selectCodingDashboardUnits(busiSessionUnits, dto)).toBe(busiSessionUnits);
  });

  it("projects the whole plan graph so cross-bucket WI appear as waiting cards with full edges", () => {
    const planUnits = selectCodingDashboardUnits(busiSessionUnits, multiRepoPlanProgressDto());

    expect(planUnits).toHaveLength(5);
    const topology = selectCodingTopology(planUnits);
    expect(topology.nodes.map((node) => node.workItemId)).toEqual([
      "WI-001",
      "WI-002",
      "WI-003",
      "WI-004",
      "WI-005",
    ]);
    // 跨桶 WI-002 不在本 attempt 桶内：以桶内 unit_status 投影为 pending 灰卡。
    expect(topology.nodes[1]).toMatchObject({ state: "pending", unitId: "coding_unit_0002" });
    // summary 回落标题；全量直接依赖建边，跨仓下游边自动出现。
    expect(topology.nodes[0]).toMatchObject({ title: "busi 首个单元", state: "running" });
    expect(topology.edges).toEqual([
      { fromWorkItemId: "WI-001", toWorkItemId: "WI-002", state: "blocking" },
      { fromWorkItemId: "WI-003", toWorkItemId: "WI-004", state: "blocking" },
      { fromWorkItemId: "WI-003", toWorkItemId: "WI-005", state: "blocking" },
      { fromWorkItemId: "WI-004", toWorkItemId: "WI-005", state: "blocking" },
    ]);

    // 依赖链跨仓语义：WI-001 的下游消费方现在包含跨桶 WI-002。
    const chain = selectCodingDependencyChain(planUnits, "WI-001");
    expect(chain?.downstream.map((node) => node.workItemId)).toEqual(["WI-002"]);
    expect(chain?.blockingSources).toEqual([]);
    // 反向：WI-002 的阻塞源是跨仓上游 WI-001。
    const downstreamChain = selectCodingDependencyChain(planUnits, "WI-002");
    expect(downstreamChain?.upstream.map((node) => node.workItemId)).toEqual(["WI-001"]);
    expect(downstreamChain?.blockingSources.map((node) => node.workItemId)).toEqual(["WI-001"]);
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
      frozen: false,
    });

    const coding = gates[1];
    expect(coding).toMatchObject({
      totalMs: CODING_BUDGET_CODING_TOTAL_MS,
      anchorAtMs: Date.parse("2026-09-14T08:00:00Z"),
      elapsedMs: CODING_BUDGET_CODING_TOTAL_MS,
      remainingMs: 0,
      ratio: 1,
      nearExhaustion: true,
      frozen: false,
    });
  });

  it("freezes elapsed at the last coding node completion instead of counting past it", () => {
    const gates = selectCodingBudgetGates(
      [codingNode({ started_at: "2026-09-14T08:00:00Z", completed_at: "2026-09-14T08:20:00Z", status: "completed" })],
      NOW,
    );
    expect(gates[0]?.elapsedMs).toBe(1_200_000);
    expect(gates[0]?.frozen).toBe(true);
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

function attentionItem(
  overrides: Partial<CodingAttentionItemDto> & { key: string },
): CodingAttentionItemDto {
  return {
    key: overrides.key,
    kind: "triage",
    project_id: "project_0001",
    issue_id: "issue_0001",
    title: "待处理",
    summary: "待处理事项",
    occurred_at: "2026-10-16T00:00:00Z",
    target: { type: "coding_attempt" },
    ...overrides,
  };
}

function attemptAggregate(
  targets: Array<Partial<PlanCodingTargetDto> & Pick<PlanCodingTargetDto, "attempt_id">>,
  attentionItems: CodingAttentionItemDto[] = [],
): PlanCodingProgressDto {
  const aggregate = multiRepoPlanProgressDto();
  aggregate.target_attempts = targets.map((overrides) => ({
    ...planTarget(overrides.target_repository_id ?? "repo_x", overrides.attempt_id),
    ...overrides,
  }));
  aggregate.attention_items = attentionItems;
  return aggregate;
}

// P1-d（REQ-MTG-13）红测口径：11 个 attempt 态逐一参数化；WaitingForHuman＋
// FinalConfirm 显示「等待最终确认」，不与编码侧人工卡壳（等待人工/分诊）混称；
// 多仓取聚合 target_attempts，单仓/聚合缺席回落当前 session attempt。
describe("selectCodingDashboardAttempts", () => {
  const IDLE_SESSION = { attemptId: null, status: null, stage: null };
  const ALL_ELEVEN_STATUSES: CodingAttemptStatus[] = [
    "created",
    "running",
    "waiting_for_human",
    "blocked",
    "awaiting_manual_recovery",
    "awaiting_plan_amendment",
    "applying_plan_amendment",
    "amendment_apply_failed",
    "completed",
    "failed",
    "aborted",
  ];

  it("parameterizes all eleven attempt statuses through the shared label tables", () => {
    const aggregate = attemptAggregate(
      ALL_ELEVEN_STATUSES.map((status, index) => ({
        attempt_id: `coding_attempt_${index}`,
        target_repository_id: `repo_${index}`,
        repository_name: `仓库${index}`,
        attempt_status: status,
        stage: status === "waiting_for_human" ? "final_confirm" : "coding",
      })),
    );
    const attempts = selectCodingDashboardAttempts(aggregate, IDLE_SESSION);

    expect(attempts.map((attempt) => attempt.status)).toEqual(ALL_ELEVEN_STATUSES);
    expect(attempts.map((attempt) => attempt.statusLabel)).toEqual([
      "已创建",
      "执行中",
      "等待最终确认",
      "已阻塞",
      "等待人工恢复",
      "等待计划修订",
      "应用计划修订",
      "修订应用失败",
      "已完成",
      "已失败",
      "已中止",
    ]);
    // 非 final_confirm 态的 stage 走 STAGE_LABELS；最终确认等待已把 stage 折进
    // statusLabel 短语（避免「等待最终确认 · 最终确认」复读）。
    expect(attempts.map((attempt) => attempt.stageLabel)).toEqual(
      ALL_ELEVEN_STATUSES.map((status) => (status === "waiting_for_human" ? null : "编码")),
    );
    expect(attempts[0]).toMatchObject({
      attemptId: "coding_attempt_0",
      repositoryName: "仓库0",
      stage: "coding",
    });
  });

  it("labels waiting_for_human at final_confirm as 等待最终确认 instead of triage/human wording", () => {
    const aggregate = attemptAggregate([
      {
        attempt_id: "coding_attempt_confirm",
        attempt_status: "waiting_for_human",
        stage: "final_confirm",
      },
      {
        attempt_id: "coding_attempt_gate",
        attempt_status: "waiting_for_human",
        stage: "coding",
      },
    ]);
    const [confirm, gate] = selectCodingDashboardAttempts(aggregate, IDLE_SESSION);

    expect(confirm?.statusLabel).toBe("等待最终确认");
    expect(confirm?.statusLabel).not.toContain("分诊");
    expect(confirm?.statusLabel).not.toContain("人工");
    // 其他 stage 的人工等待仍是通用文案，不冒充最终确认。
    expect(gate?.statusLabel).toBe("等待人工");
  });

  it("collects deduped attention kinds per attempt and skips items without an attempt", () => {
    const aggregate = attemptAggregate(
      [{ attempt_id: "coding_attempt_a" }, { attempt_id: "coding_attempt_b" }],
      [
        attentionItem({ key: "k1", kind: "choice", attempt_id: "coding_attempt_a" }),
        attentionItem({ key: "k2", kind: "triage", attempt_id: "coding_attempt_a" }),
        attentionItem({ key: "k3", kind: "choice", attempt_id: "coding_attempt_a" }),
        attentionItem({ key: "k4", kind: "blocked", attempt_id: "coding_attempt_b" }),
        attentionItem({ key: "k5", kind: "final_confirm", attempt_id: null }),
      ],
    );
    const attempts = selectCodingDashboardAttempts(aggregate, IDLE_SESSION);

    expect(attempts[0]?.attentionKinds).toEqual(["choice", "triage"]);
    expect(attempts[1]?.attentionKinds).toEqual(["blocked"]);
  });

  it("falls back to the current session attempt when no aggregate targets exist", () => {
    expect(
      selectCodingDashboardAttempts(null, {
        attemptId: "coding_attempt_session",
        status: "running",
        stage: "coding",
      }),
    ).toEqual([
      {
        attemptId: "coding_attempt_session",
        repositoryName: null,
        status: "running",
        stage: "coding",
        statusLabel: "执行中",
        stageLabel: "编码",
        attentionKinds: [],
      },
    ]);

    // 单仓 session 的最终确认等待同样使用「等待最终确认」。
    expect(
      selectCodingDashboardAttempts(null, {
        attemptId: "coding_attempt_session",
        status: "waiting_for_human",
        stage: "final_confirm",
      })[0]?.statusLabel,
    ).toBe("等待最终确认");
  });

  it("returns an empty strip when neither targets nor a live session attempt exist", () => {
    expect(selectCodingDashboardAttempts(null, IDLE_SESSION)).toEqual([]);
    expect(
      selectCodingDashboardAttempts(null, {
        attemptId: "coding_attempt_x",
        status: null,
        stage: null,
      }),
    ).toEqual([]);
    const emptyTargets = multiRepoPlanProgressDto();
    emptyTargets.target_attempts = [];
    expect(
      selectCodingDashboardAttempts(emptyTargets, {
        attemptId: null,
        status: "running",
        stage: null,
      }),
    ).toEqual([]);
  });
});
