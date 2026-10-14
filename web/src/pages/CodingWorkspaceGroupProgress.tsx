import { useMemo } from "react";
import type { CodingExecutionUnit } from "../api/types";
import { usePlanCodingProgress } from "../hooks/usePlanCodingProgress";
import {
  planCodingGroupDeleteEntry,
  toPlanCodingProgressView,
  type PlanCodingProgressView,
} from "./plan-coding-progress";


function currentUnitIndex(
  currentWorkItemId: string | null,
  units: CodingExecutionUnit[],
): number {
  if (units.length === 0) {
    return -1;
  }
  if (currentWorkItemId) {
    const matchedIndex = units.findIndex(
      (unit) => unit.logical_work_item_id === currentWorkItemId,
    );
    if (matchedIndex >= 0) {
      return matchedIndex;
    }
  }
  const completedCount = units.filter((unit) => unit.status === "completed").length;
  return Math.min(completedCount, units.length - 1);
}

/**
 * N6（REQ-CS-06）：group 进度条。projectId/issueId 在场时组件自己 GET plan 聚合
 * （bound target attempts 全景：owning 仓/依赖/槽位/暂停文案），selected WS 仍是
 * 局部实时通道；GET 缺席或失败时静默降级回既有单仓局部投影，不新增 plan 全局
 * 事件框架。refreshKey 变化（手动刷新/运行时快照更新）触发重拉。
 *
 * H5（设计 C/D）：plan 聚合标题区承载整组删除入口——聚合/局部/零成员 slim
 * 三分支都渲染该入口，恢复出口不因聚合缺席或 units 清空而消失；部分删除后
 * （workspace_lifecycle.missing>0）切「删除剩余」文案并展示「本轮授权失效」
 * 摘要，主恢复动作即清剩余。
 *
 * 两个分支都携带 onboarding 第 13 步锚点 `onboarding-anchor-coding-progress`
 * （聚合分支由外层包裹 div 提供，局部分支沿 section 本体）。
 */
export function CodingWorkspaceGroupProgress({
  planId,
  currentWorkItemId,
  units,
  projectId,
  issueId,
  selectedAttemptId,
  refreshKey,
  onDeleteAllCodingWorkspaces,
  groupDeleteBusy = false,
  deleteBusy = false,
}: {
  planId: string | null;
  currentWorkItemId: string | null;
  units: CodingExecutionUnit[];
  projectId?: string;
  issueId?: string;
  selectedAttemptId?: string;
  refreshKey?: string | number | null;
  /** H5：整组删除动作（确认流由页面 handler 持有）；未接线时不渲染入口。 */
  onDeleteAllCodingWorkspaces?: () => void;
  /** 整组删除请求进行中：禁重复点击（设计 C：请求中禁重复点击）。 */
  groupDeleteBusy?: boolean;
  /** P3：单删请求在途——互斥语义同步到入口可见态（与 groupDeleteBusy 取或）。 */
  deleteBusy?: boolean;
}) {
  const aggregateDto = usePlanCodingProgress({ projectId, issueId, planId, refreshKey });
  const aggregate = useMemo(
    () => (aggregateDto ? toPlanCodingProgressView(aggregateDto) : null),
    [aggregateDto],
  );
  const deleteEntry = useMemo(
    () => (aggregate ? planCodingGroupDeleteEntry(aggregate) : null),
    [aggregate],
  );

  const groupDeleteButton = (label: string) =>
    onDeleteAllCodingWorkspaces ? (
      <button
        type="button"
        data-testid="plan-coding-delete-all"
        disabled={groupDeleteBusy || deleteBusy}
        onClick={() => onDeleteAllCodingWorkspaces()}
        className="inline-flex h-7 shrink-0 items-center rounded-md border border-[var(--aria-danger)] bg-white px-2 text-[11px] font-semibold text-[var(--aria-danger)] hover:bg-red-50 disabled:opacity-50"
      >
        {label}
      </button>
    ) : null;

  if (!planId) {
    return null;
  }

  if (units.length === 0 && !aggregate) {
    // 零成员 slim 行：snapshot 已确认 group plan_id 时唯一恢复入口仍在
    // （设计 C：不因 units.length===0 隐藏恢复入口），范围与计数由服务端预检权威。
    if (!onDeleteAllCodingWorkspaces) {
      return null;
    }
    return (
      <section
        data-testid="onboarding-anchor-coding-progress"
        className="grid min-h-11 shrink-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-x-3 gap-y-1 border-b border-[var(--aria-line)] bg-[var(--aria-panel)] px-4 py-2 text-xs"
      >
        <div className="flex min-w-0 items-center justify-between gap-2">
          <div className="min-w-0">
            <div className="font-semibold text-[var(--aria-ink)]">WorkItemGroup</div>
            <div className="truncate font-mono text-[11px] text-[var(--aria-ink-muted)]">
              {planId}
            </div>
          </div>
          {groupDeleteButton("删除本计划全部 Coding 工作区")}
        </div>
      </section>
    );
  }


  if (aggregate) {
    return (
      <div data-testid="onboarding-anchor-coding-progress">
        <section
          data-testid="plan-coding-progress"
          className="grid min-h-11 shrink-0 grid-cols-[minmax(0,1fr)_auto] gap-x-3 gap-y-1 border-b border-[var(--aria-line)] bg-[var(--aria-panel)] px-4 py-2 text-xs md:grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] md:items-center"
        >
          <div className="flex min-w-0 items-center justify-between gap-2">
            <div className="min-w-0">
              <div className="font-semibold text-[var(--aria-ink)]">WorkItemGroup</div>
              <div className="truncate font-mono text-[11px] text-[var(--aria-ink-muted)]">
                {planId}
              </div>
            </div>
            {deleteEntry ? groupDeleteButton(deleteEntry.label) : null}
          </div>
          <div className="shrink-0 text-right font-mono text-[var(--aria-ink-muted)]">
            {aggregate.completedUnitCount} / {aggregate.units.length} ·{" "}
            {aggregate.planComplete ? "plan 完成" : "plan 未完成"}
          </div>
          <div className="col-span-2 min-w-0 truncate font-mono text-[11px] text-[var(--aria-ink-muted)] md:col-span-1 md:text-right">
            {selectedAttemptId ? `selected ${selectedAttemptId}` : (aggregate.phase ?? "-")}
          </div>
          {deleteEntry?.invalidationSummary ? (
            <div
              data-testid="plan-coding-invalidation-summary"
              className="col-span-2 text-[11px] font-semibold text-[var(--aria-danger)] md:col-span-3"
            >
              {deleteEntry.invalidationSummary}——主恢复动作:删除剩余 Coding 工作区
            </div>
          ) : null}
          <ul className="col-span-2 grid min-w-0 gap-1 md:col-span-3">
            {aggregate.units.map((unit) => (
              <li
                key={unit.unitId}
                data-testid={`plan-coding-unit-${unit.logicalWorkItemId}`}
                className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 font-mono text-[11px] text-[var(--aria-ink-muted)]"
              >
                <span className="font-semibold text-[var(--aria-ink)]">
                  {unit.logicalWorkItemId}
                </span>
                <span>@{unit.targetRepositoryId}</span>
                <span>{unit.executionStateLabel}</span>
                {unit.waitingLabel ? (
                  <span className="text-[var(--aria-primary)]">{unit.waitingLabel}</span>
                ) : null}
                {unit.unsatisfiedDependencyIds.length > 0 ? (
                  <span>依赖 {unit.unsatisfiedDependencyIds.join(", ")}</span>
                ) : null}
              </li>
            ))}
          </ul>
        </section>
      </div>
    );
  }

  const activeIndex = currentUnitIndex(currentWorkItemId, units);
  const progressValue = activeIndex >= 0 ? activeIndex + 1 : 0;

  return (
    <section data-testid="onboarding-anchor-coding-progress" className="grid min-h-11 shrink-0 grid-cols-[minmax(0,1fr)_auto] gap-x-3 gap-y-1 border-b border-[var(--aria-line)] bg-[var(--aria-panel)] px-4 py-2 text-xs md:grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] md:items-center">
      <div className="flex min-w-0 items-center justify-between gap-2">
        <div className="min-w-0">
          <div className="font-semibold text-[var(--aria-ink)]">WorkItemGroup</div>
          <div className="truncate font-mono text-[11px] text-[var(--aria-ink-muted)]">
            {planId}
          </div>
        </div>
        {groupDeleteButton("删除本计划全部 Coding 工作区")}
      </div>
      <div className="shrink-0 text-right font-mono text-[var(--aria-ink-muted)]">
        {progressValue} / {units.length}
      </div>
      <div className="col-span-2 min-w-0 truncate font-mono text-[11px] text-[var(--aria-ink-muted)] md:col-span-1 md:text-right">
        {currentWorkItemId ?? "-"}
      </div>
    </section>
  );
}
