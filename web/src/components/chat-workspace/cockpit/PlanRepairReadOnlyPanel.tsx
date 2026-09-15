// web/src/components/chat-workspace/cockpit/PlanRepairReadOnlyPanel.tsx
// DEF-3（REQ-UI37-16 第 3 项）：amendment / plan-repair 子会话只读呈现面板。
// 只读边界落在签名层：组件不接收任何回调 props；渲染树内不出现
// button / input / textarea / select / form / a[href]（测试逐断言）。
import type { PlanRepairPanelModel } from "../../../state/plan-approval-projection";

export interface PlanRepairReadOnlyPanelProps {
  model: PlanRepairPanelModel;
}

export function PlanRepairReadOnlyPanel({ model }: PlanRepairReadOnlyPanelProps) {
  return (
    <div
      data-testid="plan-repair-read-only-panel"
      className="flex min-h-0 flex-col gap-3 overflow-auto text-sm"
    >
      <div className="flex flex-wrap items-center gap-2">
        <span className="aria-chip border-[var(--aria-gate-pending-border)] bg-[var(--aria-gate-pending-bg)] text-[var(--aria-gate-pending-fg)]">
          {model.stageLabel}
        </span>
        <span className="aria-chip aria-mono border-[var(--aria-line-strong)] text-[var(--aria-ink-muted)]">
          request {model.requestStatus}
        </span>
        <span className="aria-mono text-xs text-[var(--aria-ink-muted)]">
          只读呈现 · 子会话语义由引擎持有
        </span>
      </div>

      <section aria-label="修复请求">
        <h3 className="text-xs font-semibold text-[var(--aria-ink-muted)]">修复请求</h3>
        <dl className="mt-1 grid gap-x-4 gap-y-1 text-xs sm:grid-cols-[auto_1fr]">
          <dt className="text-[var(--aria-ink-muted)]">缺陷类别</dt>
          <dd className="aria-mono break-all">{model.defectClass}</dd>
          <dt className="text-[var(--aria-ink-muted)]">原因代码</dt>
          <dd className="aria-mono break-all">{model.reasonCode}</dd>
          <dt className="text-[var(--aria-ink-muted)]">触发 finding</dt>
          <dd className="aria-mono break-all">{model.triggerFindingId}</dd>
        </dl>
      </section>

      <section aria-label="修复轮次">
        <h3 className="text-xs font-semibold text-[var(--aria-ink-muted)]">修复轮次</h3>
        {model.rounds.length === 0 ? (
          <p data-testid="plan-repair-rounds-empty" className="mt-1 text-xs text-[var(--aria-ink-muted)]">
            本连接尚未观测到子会话轮次。
          </p>
        ) : (
          <ol className="mt-1 space-y-1">
            {model.rounds.map((round) => (
              <li
                key={round.nodeId}
                className="flex flex-wrap items-baseline gap-2 text-xs"
              >
                <span
                  className={
                    round.status === "completed"
                      ? "aria-chip border-[var(--aria-topo-node-done-border)] bg-[var(--aria-topo-node-done-bg)] text-[var(--aria-topo-node-done-fg)]"
                      : round.status === "failed"
                        ? "aria-chip border-[var(--aria-topo-node-failed-border)] bg-[var(--aria-topo-node-failed-bg)] text-[var(--aria-topo-node-failed-fg)]"
                        : "aria-chip border-[var(--aria-topo-node-running-border)] bg-[var(--aria-topo-node-running-bg)] text-[var(--aria-topo-node-running-fg)]"
                  }
                >
                  {round.status}
                </span>
                <span className="text-[var(--aria-ink)]">{round.title}</span>
                {round.summary ? (
                  <span className="text-[var(--aria-ink-muted)]">{round.summary}</span>
                ) : null}
                <span className="aria-mono aria-num text-[11px] text-[var(--aria-ink-muted)]">
                  {round.startedAt ?? "—"} → {round.completedAt ?? "…"}
                </span>
              </li>
            ))}
          </ol>
        )}
      </section>

      <section aria-label="typed feedback">
        <h3 className="text-xs font-semibold text-[var(--aria-ink-muted)]">Typed feedback</h3>
        {model.feedbackTurn ? (
          <p className="aria-mono mt-1 break-all text-xs">
            turn {model.feedbackTurn.turnId} · {model.feedbackTurn.status}
            {model.feedbackTurn.commandId ? ` · command ${model.feedbackTurn.commandId}` : ""}
            <span className="aria-num"> · 剩余修复轮次 {model.feedbackTurn.remainingBudget}</span>
          </p>
        ) : (
          <p className="mt-1 text-xs text-[var(--aria-ink-muted)]">
            当前连接无进行中的 typed feedback turn。
          </p>
        )}
      </section>

      <section aria-label="amendment manifest">
        <h3 className="text-xs font-semibold text-[var(--aria-ink-muted)]">
          Amendment manifest
        </h3>
        {model.amendment ? (
          <dl className="mt-1 grid gap-x-4 gap-y-1 text-xs sm:grid-cols-[auto_1fr]">
            <dt className="text-[var(--aria-ink-muted)]">manifest</dt>
            <dd className="aria-mono break-all">{model.amendment.id}</dd>
            <dt className="text-[var(--aria-ink-muted)]">计划修订</dt>
            <dd className="aria-mono break-all">
              {model.amendment.previousPlanRevisionId} → {model.amendment.newPlanRevisionId}
            </dd>
            <dt className="text-[var(--aria-ink-muted)]">规模</dt>
            <dd className="aria-num">
              修订工作项 {model.amendment.revisedWorkItemCount} · 契约变更{" "}
              {model.amendment.contractDeltaCount} · 被替代修订{" "}
              {model.amendment.supersededCount} · 依赖图变更{" "}
              {model.amendment.dependencyGraphChangeCount}
            </dd>
            <dt className="text-[var(--aria-ink-muted)]">恢复目标</dt>
            <dd className="aria-mono break-all">{model.amendment.resumeTarget}</dd>
          </dl>
        ) : (
          <p className="mt-1 text-xs text-[var(--aria-ink-muted)]">
            尚未产出 amendment manifest。
          </p>
        )}
      </section>

      {model.error ? (
        <p role="alert" className="text-xs text-[var(--aria-danger)]">
          {model.error}
        </p>
      ) : null}
    </div>
  );
}
