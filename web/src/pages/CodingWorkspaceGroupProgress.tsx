import { useEffect, useState } from "react";
import { getPlanCodingProgress } from "../api/client";
import type { CodingExecutionUnit } from "../api/types";
import { toPlanCodingProgressView, type PlanCodingProgressView } from "./plan-coding-progress";

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
 */
export function CodingWorkspaceGroupProgress({
  planId,
  currentWorkItemId,
  units,
  projectId,
  issueId,
  selectedAttemptId,
  refreshKey,
}: {
  planId: string | null;
  currentWorkItemId: string | null;
  units: CodingExecutionUnit[];
  projectId?: string;
  issueId?: string;
  selectedAttemptId?: string;
  refreshKey?: string | number | null;
}) {
  const [aggregate, setAggregate] = useState<PlanCodingProgressView | null>(null);
  const aggregateEligible = Boolean(projectId && issueId && planId);

  useEffect(() => {
    if (!aggregateEligible || !projectId || !issueId || !planId) {
      setAggregate(null);
      return;
    }
    let cancelled = false;
    getPlanCodingProgress(projectId, issueId, planId)
      .then((dto) => {
        if (!cancelled) {
          setAggregate(toPlanCodingProgressView(dto));
        }
      })
      .catch(() => {
        // 聚合不可用（如 v1 旧 group 无 confirmed plan run）不阻塞局部展示。
        if (!cancelled) {
          setAggregate(null);
        }
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [aggregateEligible, projectId, issueId, planId, refreshKey]);

  if (!planId || units.length === 0) {
    return null;
  }

  if (aggregate) {
    return (
      <section
        data-testid="plan-coding-progress"
        className="grid min-h-11 shrink-0 grid-cols-[minmax(0,1fr)_auto] gap-x-3 gap-y-1 border-b border-[var(--aria-line)] bg-[var(--aria-panel)] px-4 py-2 text-xs md:grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] md:items-center"
      >
        <div className="min-w-0">
          <div className="font-semibold text-[var(--aria-ink)]">WorkItemGroup</div>
          <div className="truncate font-mono text-[11px] text-[var(--aria-ink-muted)]">
            {planId}
          </div>
        </div>
        <div className="shrink-0 text-right font-mono text-[var(--aria-ink-muted)]">
          {aggregate.completedUnitCount} / {aggregate.units.length} ·{" "}
          {aggregate.planComplete ? "plan 完成" : "plan 未完成"}
        </div>
        <div className="col-span-2 min-w-0 truncate font-mono text-[11px] text-[var(--aria-ink-muted)] md:col-span-1 md:text-right">
          {selectedAttemptId ? `selected ${selectedAttemptId}` : (aggregate.phase ?? "-")}
        </div>
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
    );
  }

  const activeIndex = currentUnitIndex(currentWorkItemId, units);
  const progressValue = activeIndex >= 0 ? activeIndex + 1 : 0;

  return (
    <section data-testid="onboarding-anchor-coding-progress" className="grid min-h-11 shrink-0 grid-cols-[minmax(0,1fr)_auto] gap-x-3 gap-y-1 border-b border-[var(--aria-line)] bg-[var(--aria-panel)] px-4 py-2 text-xs md:grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] md:items-center">
      <div className="min-w-0">
        <div className="font-semibold text-[var(--aria-ink)]">WorkItemGroup</div>
        <div className="truncate font-mono text-[11px] text-[var(--aria-ink-muted)]">
          {planId}
        </div>
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
