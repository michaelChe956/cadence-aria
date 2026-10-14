import type {
  CodingAttemptStatus,
  PlanCodingExecutionState,
  PlanCodingProgressDto,
  PlanCodingTargetDto,
  PlanCodingUnitProgressDto,
  PlanWorkspaceLifecycleState,
} from "../api/types";

/**
 * N6（REQ-CS-06）：plan 聚合进度的纯投影——只把 N3 冻结 DTO 映射成展示文案/
 * 入口，不重算 ready、plan completion 或依赖满足性（后端是唯一事实源）。
 */

export type PlanCodingUnitProgressView = {
  unitId: string;
  logicalWorkItemId: string;
  targetRepositoryId: string;
  attemptId: string;
  unitStatus: PlanCodingUnitProgressDto["unit_status"];
  executionState: PlanCodingExecutionState;
  /** execution_state 的展示文案；initialized 明确不是 provider 运行。 */
  executionStateLabel: string;
  unsatisfiedDependencyIds: string[];
  waitingReason: string | null;
  /** REQ-CS-06 三种等待文案；无等待事实时为 null。 */
  waitingLabel: string | null;
};

export type PlanCodingTargetProgressView = {
  targetRepositoryId: string;
  attemptId: string;
  attemptStatus: CodingAttemptStatus;
  leaseState: string;
  worktreePath: string | null;
};

export type PlanCodingProgressView = {
  planId: string;
  boundPlanRevisionId: string;
  phase: string | null;
  primaryAttemptId: string | null;
  selectedUnitId: string | null;
  selectedAttemptId: string | null;
  /** 只透传 DTO 事实：primary Completed 不代表 plan 完成。 */
  planComplete: boolean;
  completedUnitCount: number;
  units: PlanCodingUnitProgressView[];
  targets: PlanCodingTargetProgressView[];
  /** H5：workspace_lifecycle 只读透传（旧响应无此事实时为 null）。 */
  workspaceLifecycle: PlanWorkspaceLifecycleView | null;
};

export type PlanWorkspaceLifecycleView = {
  generation: number;
  existingMemberCount: number;
  missingMemberCount: number;
  state: PlanWorkspaceLifecycleState;
};

// H5（设计 C/D）：plan 聚合标题区整组删除入口的文案派生——全部成员在场时
// 「删除本计划全部 Coding 工作区(N)」；部分删除后切「删除剩余 Coding 工作
// 区(E)」并给出「本轮授权失效:已删 M/总」；无 workspace_lifecycle 事实
// （H4 前）按 target attempts 计数兜底，零 target 时无计数（服务端预检权威）。
export type PlanCodingGroupDeleteEntryView = {
  label: string;
  invalidationSummary: string | null;
};

export function planCodingGroupDeleteEntry(
  view: PlanCodingProgressView,
): PlanCodingGroupDeleteEntryView {
  const lifecycle = view.workspaceLifecycle;
  if (lifecycle && lifecycle.missingMemberCount > 0) {
    const total = lifecycle.existingMemberCount + lifecycle.missingMemberCount;
    return {
      label: `删除剩余 Coding 工作区(${lifecycle.existingMemberCount})`,
      invalidationSummary: `本轮授权失效:已删 ${lifecycle.missingMemberCount}/${total}`,
    };
  }
  if (lifecycle) {
    return {
      label: `删除本计划全部 Coding 工作区(${lifecycle.existingMemberCount})`,
      invalidationSummary: null,
    };
  }
  const targetCount = view.targets.length;
  return {
    label:
      targetCount > 0
        ? `删除本计划全部 Coding 工作区(${targetCount})`
        : "删除本计划全部 Coding 工作区",
    invalidationSummary: null,
  };
}

// execution_state 值域文案：initialized=初始化桶物化（无 provider 事实），
// running=provider 真实运行——两者必须可区分；invalidated=plan-run 授权失效
// 投影（成员删除/精确 purge 后），与 unit 事实状态无关。
const EXECUTION_STATE_LABELS: Record<string, string> = {
  initialized: "已初始化",
  running: "provider 运行中",
  waiting_dependency: "等待依赖",
  waiting_slot: "等待执行槽位",
  paused: "安全暂停",
  completed: "已完成",
  invalidated: "授权已失效",
};

function executionStateLabel(state: string): string {
  return EXECUTION_STATE_LABELS[state] ?? state;
}

// REQ-CS-06 冻结等待文案。
const WAITING_REASON_LABELS: Record<string, string> = {
  dependency_not_ready: "等待依赖完成",
  execution_slot_occupied: "等待执行槽位",
  plan_coding_continuation_not_enabled: "下一项编排尚未启用",
};

/** 等待文案：waiting_reason 优先，缺失时仅按 execution_state 兜底投影。 */
export function planCodingWaitingLabel(
  unit: PlanCodingUnitProgressDto,
): string | null {
  if (unit.waiting_reason) {
    return WAITING_REASON_LABELS[unit.waiting_reason] ?? unit.waiting_reason;
  }
  if (unit.execution_state === "waiting_dependency") {
    return WAITING_REASON_LABELS.dependency_not_ready;
  }
  if (unit.execution_state === "waiting_slot") {
    return WAITING_REASON_LABELS.execution_slot_occupied;
  }
  if (unit.execution_state === "paused") {
    return WAITING_REASON_LABELS.plan_coding_continuation_not_enabled;
  }
  return null;
}

function unitView(unit: PlanCodingUnitProgressDto): PlanCodingUnitProgressView {
  return {
    unitId: unit.unit_id,
    logicalWorkItemId: unit.logical_work_item_id,
    targetRepositoryId: unit.target_repository_id,
    attemptId: unit.attempt_id,
    unitStatus: unit.unit_status,
    executionState: unit.execution_state,
    executionStateLabel: executionStateLabel(unit.execution_state),
    unsatisfiedDependencyIds: [...unit.unsatisfied_dependency_ids],
    waitingReason: unit.waiting_reason,
    waitingLabel: planCodingWaitingLabel(unit),
  };
}

function targetView(target: PlanCodingTargetDto): PlanCodingTargetProgressView {
  return {
    targetRepositoryId: target.target_repository_id,
    attemptId: target.attempt_id,
    attemptStatus: target.attempt_status,
    leaseState: target.lease_state,
    worktreePath: target.worktree_path,
  };
}

export function toPlanCodingProgressView(
  dto: PlanCodingProgressDto,
): PlanCodingProgressView {
  return {
    planId: dto.plan_id,
    boundPlanRevisionId: dto.bound_plan_revision_id,
    phase: dto.phase,
    primaryAttemptId: dto.primary_attempt_id,
    selectedUnitId: dto.selected_unit_id,
    selectedAttemptId: dto.selected_attempt_id,
    // 只透传：completed 计数是展示派生（completed units 计数），planComplete
    // 本身永远取 DTO 字段，不因 primary/局部完成而冒充。
    planComplete: dto.plan_complete,
    completedUnitCount: dto.units.filter(
      (unit) => unit.execution_state === "completed",
    ).length,
    units: dto.units.map(unitView),
    targets: dto.target_attempts.map(targetView),
    workspaceLifecycle: dto.workspace_lifecycle
      ? {
          generation: dto.workspace_lifecycle.generation,
          existingMemberCount: dto.workspace_lifecycle.existing_member_count,
          missingMemberCount: dto.workspace_lifecycle.missing_member_count,
          state: dto.workspace_lifecycle.state,
        }
      : null,
  };
}
