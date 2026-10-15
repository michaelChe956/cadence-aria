import type {
  CodingAttemptStatus,
  CodingAttentionItemDto,
  CodingExecutionUnit,
  CodingExecutionUnitStatus,
  CodingTimelineNode,
  PlanCodingProgressDto,
} from "../api/types";
import { ATTEMPT_STATUS_LABELS, STAGE_LABELS } from "./coding-execution-labels";
import type { CockpitFlowState } from "./workspace-cockpit-projection";

export type CodingUnitState = CockpitFlowState;

// 仪表盘组件统一从本模块取状态词汇（T2/T3），避免组件横向依赖 cockpit 投影。
export { topologyTokenName } from "./workspace-cockpit-projection";

// 12 态 → 六态（REQ-UI37-14：六态复用 1a 拓扑口径，无新色相）。
// waiting_for_human 是编码侧人工卡壳（gate/permission/choice），投影为 awaiting_triage
// 脉冲态，与对话侧「needs_human → awaiting_triage」同构；blocked_by_plan_defect /
// awaiting_amendment / needs_revalidation 都是「被计划或上游挡住」的阻塞族；
// stale / superseded / skipped 尚未产出或已被替代，视作未开始。
export function codingUnitState(status: CodingExecutionUnitStatus): CodingUnitState {
  switch (status) {
    case "running":
      return "running";
    case "waiting_for_human":
      return "awaiting_triage";
    case "completed":
      return "done";
    case "failed":
      return "failed";
    case "blocked":
    case "blocked_by_plan_defect":
    case "awaiting_amendment":
    case "needs_revalidation":
      return "blocked";
    default:
      return "pending"; // pending / stale / superseded / skipped
  }
}

// 六态中文标签：拓扑图与依赖链视图共用的单一定义点（REQ-UI37-15）。
export const CODING_UNIT_STATE_LABELS: Readonly<Record<CodingUnitState, string>> = {
  running: "执行中",
  pending: "待开始",
  blocked: "被阻塞",
  done: "已完成",
  failed: "失败",
  awaiting_triage: "等待分诊",
};

export type CodingTopologyEdgeState = "satisfied" | "blocking";

export interface CodingTopologyNode {
  workItemId: string;
  unitId: string;
  title: string;
  orderIndex: number;
  state: CodingUnitState;
  completionCommit: string | null;
}

export interface CodingTopologyEdge {
  fromWorkItemId: string;
  toWorkItemId: string;
  state: CodingTopologyEdgeState;
}

export interface CodingTopology {
  nodes: readonly CodingTopologyNode[];
  edges: readonly CodingTopologyEdge[];
}

function isRevisionShadowed(status: CodingExecutionUnitStatus): boolean {
  return status === "superseded" || status === "stale";
}

// 拓扑建图所需的 unit 最小结构面：session units（CodingExecutionUnit）与
// plan 聚合投影（selectCodingDashboardUnits）都结构化满足。
export type CodingTopologyUnitInput = Pick<
  CodingExecutionUnit,
  | "unit_id"
  | "logical_work_item_id"
  | "dependency_logical_work_item_ids"
  | "order_index"
  | "status"
  | "summary"
  | "completion_commit"
>;

/**
 * Slice B（仪表盘跨仓依赖投影）：group attempt 的拓扑/依赖链数据源切换。
 * plan 聚合可用（绑定 ≥2 target attempt）时以 plan 全图 units 建图——跨桶
 * WI 以桶内 unit_status 走既有 12→六态投影（等待依赖=灰 pending 卡），全量
 * 直接依赖建边（跨仓下游自动出现）；无聚合或 v1 单 attempt 原样回落
 * session units，桶内口径零变化。
 */
export function selectCodingDashboardUnits(
  sessionUnits: readonly CodingExecutionUnit[],
  aggregate: PlanCodingProgressDto | null,
): readonly CodingTopologyUnitInput[] {
  if (!aggregate || aggregate.target_attempts.length <= 1) {
    return sessionUnits;
  }
  return aggregate.units.map((unit) => ({
    unit_id: unit.unit_id,
    logical_work_item_id: unit.logical_work_item_id,
    dependency_logical_work_item_ids: unit.dependency_logical_work_item_ids,
    order_index: unit.order_index,
    status: unit.unit_status,
    summary: unit.summary,
    completion_commit: null,
  }));
}

// 同一 logical_work_item_id 的多 unit = 同一 work item 的多轮修订。
// session_state.units 中修订新者居后：有效（非 superseded/stale）条目里取最后一条；
// 全部被取代时取最后一条（拓扑仍可见，状态为 pending）。
function latestRevision(
  current: CodingTopologyUnitInput | undefined,
  next: CodingTopologyUnitInput,
): CodingTopologyUnitInput {
  if (!current) return next;
  const currentShadowed = isRevisionShadowed(current.status);
  const nextShadowed = isRevisionShadowed(next.status);
  if (currentShadowed && !nextShadowed) return next;
  if (!currentShadowed && nextShadowed) return current;
  return next;
}

// P1-d（REQ-MTG-13）：仪表盘 attempt 全态状态条的展示模型与投影——多仓取聚合
// target_attempts（待办 kind 取同源 attention_items），单仓/聚合缺席回落当前
// session attempt。只读展示层，不参与 unit 六态拓扑判据（codingUnitState 仍
// 是拓扑唯一口径），unit 已完成与 attempt 待确认可同时呈现。
export interface CodingDashboardAttempt {
  attemptId: string;
  repositoryName: string | null;
  status: CodingAttemptStatus;
  stage: string | null;
  /** 11 态文案（ATTEMPT_STATUS_LABELS）；WaitingForHuman＋FinalConfirm 专用短语。 */
  statusLabel: string;
  /** STAGE_LABELS 文案；最终确认等待已折进 statusLabel，置 null 防复读。 */
  stageLabel: string | null;
  /** 本 attempt 的待办 kind（首见序去重），供状态条徽标。 */
  attentionKinds: readonly string[];
}

/** 单仓/聚合缺席时的 session attempt 最小输入面。 */
export interface CodingDashboardSessionAttemptInput {
  attemptId: string | null;
  status: CodingAttemptStatus | null;
  stage: string | null;
}

// WaitingForHuman＋FinalConfirm 是聚合确认等待，不与编码侧人工卡壳（等待人工/
// 分诊）混称（REQ-MTG-13）；短语已含 stage 语义，stageLabel 不再复读「最终确认」。
export const CODING_ATTEMPT_FINAL_CONFIRM_WAIT_LABEL = "等待最终确认";

function attemptStatusLabel(status: CodingAttemptStatus, stage: string | null): string {
  if (status === "waiting_for_human" && stage === "final_confirm") {
    return CODING_ATTEMPT_FINAL_CONFIRM_WAIT_LABEL;
  }
  return ATTEMPT_STATUS_LABELS[status] ?? status;
}

function attemptStageLabel(status: CodingAttemptStatus, stage: string | null): string | null {
  if (stage === null || (status === "waiting_for_human" && stage === "final_confirm")) {
    return null;
  }
  return STAGE_LABELS[stage] ?? stage;
}

function attentionKindsByAttempt(
  items: readonly CodingAttentionItemDto[] | undefined,
): Map<string, string[]> {
  const byAttempt = new Map<string, string[]>();
  for (const item of items ?? []) {
    if (!item.attempt_id) {
      continue;
    }
    const kinds = byAttempt.get(item.attempt_id);
    if (kinds) {
      if (!kinds.includes(item.kind)) {
        kinds.push(item.kind);
      }
    } else {
      byAttempt.set(item.attempt_id, [item.kind]);
    }
  }
  return byAttempt;
}

export function selectCodingDashboardAttempts(
  aggregate: PlanCodingProgressDto | null,
  session: CodingDashboardSessionAttemptInput,
): readonly CodingDashboardAttempt[] {
  if (aggregate && aggregate.target_attempts.length > 0) {
    const kindsByAttempt = attentionKindsByAttempt(aggregate.attention_items);
    return aggregate.target_attempts.map((target) => {
      const stage = target.stage ?? null;
      return {
        attemptId: target.attempt_id,
        repositoryName: target.repository_name ?? null,
        status: target.attempt_status,
        stage,
        statusLabel: attemptStatusLabel(target.attempt_status, stage),
        stageLabel: attemptStageLabel(target.attempt_status, stage),
        attentionKinds: kindsByAttempt.get(target.attempt_id) ?? [],
      };
    });
  }
  if (!session.attemptId || !session.status) {
    return [];
  }
  const sessionStage = session.stage ?? null;
  return [
    {
      attemptId: session.attemptId,
      repositoryName: null,
      status: session.status,
      stage: sessionStage,
      statusLabel: attemptStatusLabel(session.status, sessionStage),
      stageLabel: attemptStageLabel(session.status, sessionStage),
      attentionKinds: [],
    },
  ];
}

export function selectCodingTopology(units: readonly CodingTopologyUnitInput[]): CodingTopology {
  const representative = new Map<string, CodingTopologyUnitInput>();
  for (const item of units) {
    representative.set(item.logical_work_item_id, latestRevision(representative.get(item.logical_work_item_id), item));
  }

  const nodes = [...representative.values()]
    .sort((left, right) => left.order_index - right.order_index || left.logical_work_item_id.localeCompare(right.logical_work_item_id))
    .map((item) => ({
      workItemId: item.logical_work_item_id,
      unitId: item.unit_id,
      title: item.summary ?? item.logical_work_item_id,
      orderIndex: item.order_index,
      state: codingUnitState(item.status),
      completionCommit: item.completion_commit,
    }));

  const nodeByWorkItemId = new Map(nodes.map((node) => [node.workItemId, node]));
  const edges: CodingTopologyEdge[] = [];
  const seenEdges = new Set<string>();
  for (const node of nodes) {
    const item = representative.get(node.workItemId);
    if (!item) continue;
    for (const dependencyId of item.dependency_logical_work_item_ids) {
      const upstream = nodeByWorkItemId.get(dependencyId);
      if (!upstream || seenEdges.has(`${dependencyId}->${node.workItemId}`)) continue;
      seenEdges.add(`${dependencyId}->${node.workItemId}`);
      edges.push({
        fromWorkItemId: dependencyId,
        toWorkItemId: node.workItemId,
        state: upstream.state === "done" || node.state === "done" ? "satisfied" : "blocking",
      });
    }
  }

  return { nodes, edges };
}

export interface CodingDependencyChain {
  workItemId: string;
  upstream: readonly CodingTopologyNode[];
  downstream: readonly CodingTopologyNode[];
  blockingSources: readonly CodingTopologyNode[];
}

export function selectCodingDependencyChain(
  units: readonly CodingTopologyUnitInput[],
  workItemId: string,
): CodingDependencyChain | null {
  const { nodes, edges } = selectCodingTopology(units);
  const nodeByWorkItemId = new Map(nodes.map((node) => [node.workItemId, node]));
  const focus = nodeByWorkItemId.get(workItemId);
  if (!focus) return null;

  const upstream = closure(nodes, edges, workItemId, "upstream");
  const downstream = closure(nodes, edges, workItemId, "downstream");

  return {
    workItemId,
    upstream,
    downstream,
    blockingSources: upstream.filter((node) => node.state !== "done"),
  };
}

// BFS 闭包：direction 决定沿边走哪一端（上游 = from 端，下游 = to 端）；
// 逐层扩展保证传递性（直接 + 间接），visited 以起始点 + 首见序去重，顺序稳定可测。
function closure(
  nodes: readonly CodingTopologyNode[],
  edges: readonly CodingTopologyEdge[],
  startWorkItemId: string,
  direction: "upstream" | "downstream",
): readonly CodingTopologyNode[] {
  const nodeByWorkItemId = new Map(nodes.map((node) => [node.workItemId, node]));
  const visited = new Set<string>([startWorkItemId]);
  const queue = [startWorkItemId];
  const result: CodingTopologyNode[] = [];
  while (queue.length > 0) {
    const current = queue.shift() as string;
    for (const edge of edges) {
      const next =
        direction === "upstream"
          ? edge.toWorkItemId === current
            ? edge.fromWorkItemId
            : null
          : edge.fromWorkItemId === current
            ? edge.toWorkItemId
            : null;
      if (next === null || visited.has(next)) continue;
      visited.add(next);
      const node = nodeByWorkItemId.get(next);
      if (node) {
        result.push(node);
        queue.push(next);
      }
    }
  }
  return result;
}

export const CODING_BUDGET_WORK_ITEM_TOTAL_MS = 3_600_000;
export const CODING_BUDGET_CODING_TOTAL_MS = 5_400_000;
export const CODING_BUDGET_NEAR_EXHAUSTION_MS = 600_000;

export interface CodingBudgetGate {
  kind: "work_item" | "coding";
  label: string;
  totalMs: number;
  anchorAtMs: number | null;
  endMs: number;
  elapsedMs: number;
  remainingMs: number;
  ratio: number;
  nearExhaustion: boolean;
  /** 末个 coding 节点已 completed：预算冻结，前端不再推进也不再播报。 */
  frozen: boolean;
}

// 60/90min 为 driver 约定口径、纯前端常量（引擎不发布预算事件，REQ-UI37-15）：
// - coding 门（attempt 级）：锚点 = 首个 coding 阶段节点 started_at，重修不重置；
// - work_item 门（当前 work item）：锚点 = 最近一轮 coding 节点 started_at（Coder 重修轮重置）；
// - 完成后冻结在最末 coding 节点 completed_at，不再随 nowMs 增长。
export function selectCodingBudgetGates(
  timelineNodes: readonly CodingTimelineNode[],
  nowMs: number,
): readonly CodingBudgetGate[] {
  const codingNodes = timelineNodes.filter((node) => node.stage === "coding");
  if (codingNodes.length === 0) return [];
  const first = codingNodes[0];
  const last = codingNodes[codingNodes.length - 1];
  if (!first || !last) return [];
  const endMs = last.completed_at ? Date.parse(last.completed_at) : nowMs;
  const frozen = last.completed_at !== null;

  const workItemAnchor = Date.parse(last.started_at);
  const codingAnchor = Date.parse(first.started_at);
  if (Number.isNaN(workItemAnchor) || Number.isNaN(codingAnchor) || Number.isNaN(endMs)) return [];

  return [
    buildBudgetGate("work_item", "Work Item 预算门（60min）", CODING_BUDGET_WORK_ITEM_TOTAL_MS, workItemAnchor, endMs, frozen),
    buildBudgetGate("coding", "Coding 预算门（90min）", CODING_BUDGET_CODING_TOTAL_MS, codingAnchor, endMs, frozen),
  ];
}

function buildBudgetGate(
  kind: "work_item" | "coding",
  label: string,
  totalMs: number,
  anchorAtMs: number,
  endMs: number,
  frozen: boolean,
): CodingBudgetGate {
  const elapsedMs = Math.min(Math.max(0, endMs - anchorAtMs), totalMs);
  const remainingMs = totalMs - elapsedMs;
  return {
    kind,
    label,
    totalMs,
    anchorAtMs,
    endMs,
    elapsedMs,
    remainingMs,
    ratio: Math.min(1, elapsedMs / totalMs),
    nearExhaustion: remainingMs <= CODING_BUDGET_NEAR_EXHAUSTION_MS,
    frozen,
  };
}
