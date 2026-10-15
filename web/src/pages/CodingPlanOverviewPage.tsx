import { useMemo } from "react";
import { ArrowLeft, ClipboardList } from "lucide-react";
import type { CodingAttentionItemDto } from "../api/types";
import { usePlanCodingProgress } from "../hooks/usePlanCodingProgress";
import {
  ATTEMPT_STATUS_LABELS,
  STAGE_LABELS,
} from "../state/coding-execution-labels";
import { PlanCodingFinalConfirm } from "./PlanCodingFinalConfirm";
import { toPlanCodingProgressView } from "./plan-coding-progress";

/**
 * P1-b（REQ-MTG-12）：只读 plan 总览（owner 选 A：列表+单详情切换）——按
 * order_index 列出全部 WI（逻辑 ID、summary 缺省回落 ID、仓库、unit 进度、
 * attempt 状态、依赖/等待原因、待办徽标、详情链接），标题显示执行/确认 N/M；
 * 总览可见时 5 秒轮询（非当前仓更新无需手动刷新）；除 plan 聚合确认外不提供
 * 任何业务写操作。单详情切换由调用方导航（P1-c 接线）：点 WI 行/徽标只回调
 * onOpenAttempt——同一时刻只进一个 attempt 详情，返回后仍在总览；详情按
 * attempt 地址走既有 REST/WS 能力，本页不自建第二套详情。
 */
export type PlanOverviewOpenTarget = {
  attemptId: string;
  workItemId: string;
  gateId?: string;
  choiceId?: string;
};

export function CodingPlanOverviewPage({
  projectId,
  issueId,
  planId,
  onBack,
  onOpenAttempt,
}: {
  projectId: string;
  issueId: string;
  planId: string;
  onBack?: () => void;
  onOpenAttempt: (target: PlanOverviewOpenTarget) => void;
}) {
  const { data, loading, error, refetch } = usePlanCodingProgress({
    projectId,
    issueId,
    planId,
    pollIntervalMs: 5_000,
  });
  const view = useMemo(() => (data ? toPlanCodingProgressView(data) : null), [
    data,
  ]);
  const orderedUnits = useMemo(
    () =>
      view
        ? [...view.units].sort((left, right) => left.orderIndex - right.orderIndex)
        : [],
    [view],
  );
  const badgesByAttempt = useMemo(() => {
    const byAttempt = new Map<string, CodingAttentionItemDto[]>();
    if (view) {
      for (const item of view.attentionItems) {
        if (!item.attempt_id) {
          continue;
        }
        const existing = byAttempt.get(item.attempt_id);
        if (existing) {
          existing.push(item);
        } else {
          byAttempt.set(item.attempt_id, [item]);
        }
      }
    }
    return byAttempt;
  }, [view]);

  return (
    <div
      data-testid="plan-overview-page"
      className="flex min-h-0 flex-1 flex-col bg-[var(--aria-panel)]"
    >
      <header className="flex h-11 shrink-0 items-center gap-2 border-b border-[var(--aria-line)] px-4 text-xs">
        {onBack ? (
          <button
            type="button"
            aria-label="返回"
            onClick={onBack}
            className="inline-flex h-7 w-7 items-center justify-center rounded-md text-[var(--aria-ink-muted)] transition-colors duration-200 hover:bg-[var(--aria-panel-muted)]"
          >
            <ArrowLeft className="h-4 w-4" aria-hidden="true" />
          </button>
        ) : null}
        <ClipboardList className="h-4 w-4" aria-hidden="true" />
        <span className="font-semibold text-[var(--aria-ink)]">
          Coding Plan 总览
        </span>
        <span className="min-w-0 truncate font-mono text-[11px] text-[var(--aria-ink-muted)]">
          {planId}
        </span>
      </header>
      {loading && !view ? (
        <div
          data-testid="plan-overview-loading"
          className="px-4 py-3 text-xs text-[var(--aria-ink-muted)]"
        >
          plan 聚合进度加载中…
        </div>
      ) : null}
      {error && !view ? (
        <div
          data-testid="plan-overview-error"
          role="alert"
          className="flex items-center gap-2 px-4 py-3 text-xs text-[var(--aria-danger)]"
        >
          <span>plan 聚合进度获取失败：{error}</span>
          <button
            type="button"
            data-testid="plan-overview-retry"
            onClick={refetch}
            className="inline-flex h-7 items-center rounded-md border border-[var(--aria-line)] bg-white px-2 text-[11px] font-semibold text-[var(--aria-ink)] hover:bg-slate-50"
          >
            重试
          </button>
        </div>
      ) : null}
      {view ? (
        <>
          <section
            data-testid="plan-overview-stats"
            className="flex min-w-0 flex-wrap items-center gap-x-4 gap-y-2 border-b border-[var(--aria-line)] px-4 py-2 text-xs"
          >
            <div className="font-semibold text-[var(--aria-ink)]">
              执行 {view.completedUnitCount}/{view.units.length}
              {view.planComplete ? " · 全部 WI 执行完成" : ""}
            </div>
            <div className="text-[var(--aria-ink-muted)]">
              人工确认{" "}
              {view.finalConfirm?.confirmedAttemptIds.length ?? 0}/
              {view.targets.length}
              {view.finalConfirm?.state === "done" ? " · 全部人工确认" : ""}
            </div>
            {view.finalConfirm ? (
              <PlanCodingFinalConfirm
                projectId={projectId}
                issueId={issueId}
                planId={planId}
                finalConfirm={view.finalConfirm}
                targetCount={view.targets.length}
                unitCount={view.units.length}
                onConfirmed={refetch}
              />
            ) : null}
          </section>
          <ul
            data-testid="plan-overview-units"
            className="grid min-h-0 flex-1 content-start gap-1 overflow-auto px-4 py-2"
          >
            {orderedUnits.map((unit) => {
              const badges = badgesByAttempt.get(unit.attemptId) ?? [];
              const attemptStatusLabel = unit.attemptStatus
                ? ATTEMPT_STATUS_LABELS[unit.attemptStatus] ?? unit.attemptStatus
                : "-";
              const stageLabel = unit.stage
                ? STAGE_LABELS[unit.stage] ?? unit.stage
                : null;
              return (
                <li
                  key={unit.unitId}
                  data-testid={`plan-overview-unit-${unit.logicalWorkItemId}`}
                  className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 py-2 text-xs"
                >
                  <span className="font-mono font-semibold text-[var(--aria-ink)]">
                    {unit.logicalWorkItemId}
                  </span>
                  <span className="min-w-0 max-w-64 truncate text-[var(--aria-ink)]">
                    {unit.summary ?? unit.logicalWorkItemId}
                  </span>
                  <span className="font-mono text-[11px] text-[var(--aria-ink-muted)]">
                    @{unit.repositoryName ?? unit.targetRepositoryId}
                  </span>
                  <span className="text-[var(--aria-ink-muted)]">
                    {unit.executionStateLabel}
                  </span>
                  <span className="text-[var(--aria-ink-muted)]">
                    {attemptStatusLabel}
                    {stageLabel ? ` · ${stageLabel}` : ""}
                  </span>
                  {unit.unsatisfiedDependencyIds.length > 0 ? (
                    <span className="font-mono text-[11px] text-[var(--aria-ink-muted)]">
                      依赖 {unit.unsatisfiedDependencyIds.join("、")}
                    </span>
                  ) : null}
                  {unit.waitingLabel ? (
                    <span className="text-[var(--aria-primary)]">
                      {unit.waitingLabel}
                    </span>
                  ) : null}
                  {badges.map((badge) => (
                    <button
                      key={badge.key}
                      type="button"
                      data-testid={`plan-overview-attention-${badge.key}`}
                      title={badge.summary}
                      onClick={() =>
                        onOpenAttempt({
                          attemptId: unit.attemptId,
                          workItemId: unit.logicalWorkItemId,
                          gateId: badge.focus?.gate_id,
                          choiceId: badge.focus?.choice_id,
                        })
                      }
                      className="inline-flex h-6 shrink-0 items-center rounded-full border border-[var(--aria-primary)] bg-white px-2 text-[10px] font-semibold text-[var(--aria-primary)]"
                    >
                      待办 · {badge.title}
                    </button>
                  ))}
                  <button
                    type="button"
                    data-testid={`plan-overview-open-${unit.logicalWorkItemId}`}
                    onClick={() =>
                      onOpenAttempt({
                        attemptId: unit.attemptId,
                        workItemId: unit.logicalWorkItemId,
                      })
                    }
                    className="ml-auto inline-flex h-6 shrink-0 items-center rounded-md border border-[var(--aria-line)] bg-white px-2 text-[10px] font-semibold text-[var(--aria-ink)] hover:bg-slate-50"
                  >
                    详情
                  </button>
                </li>
              );
            })}
          </ul>
        </>
      ) : null}
    </div>
  );
}
