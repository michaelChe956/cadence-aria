// web/src/components/chat-workspace/cockpit/ContractChecklistView.tsx
import { Check, CornerDownRight, X } from "lucide-react";
import type {
  ContractChecklist,
  ContractChecklistRow,
} from "../../../state/plan-approval-projection";

export interface ContractChecklistViewProps {
  checklist: ContractChecklist;
  findingTargetFor: (row: ContractChecklistRow) => string | null;
  onJumpToFinding: (entryId: string) => void;
}

export function ContractChecklistView({
  checklist,
  findingTargetFor,
  onJumpToFinding,
}: ContractChecklistViewProps) {
  if (!checklist.hasData) {
    return (
      <div
        data-testid="contract-checklist-view"
        className="text-sm text-[var(--aria-ink-muted)]"
      >
        当前计划没有 capability / 契约条目（无 plan projection 或 contract_flow 为空）。
      </div>
    );
  }

  return (
    <div
      data-testid="contract-checklist-view"
      className="flex min-h-0 flex-col gap-3"
    >
      <div className="flex flex-wrap items-center gap-2 text-xs">
        <span
          data-testid="contract-checklist-gap-count"
          className="aria-chip aria-num border-[var(--aria-line-strong)] text-[var(--aria-ink-muted)]"
        >
          缺口 {checklist.gapCount} 条 / 共 {checklist.rows.length} 条
        </span>
        <span className="text-[var(--aria-ink-muted)]">
          满足判定以引擎编译期 missing_capabilities 为准，非前端重算
        </span>
      </div>
      <div className="min-h-0 space-y-3 overflow-auto">
        {checklist.rows.map((row) => {
          const target = row.satisfied ? null : findingTargetFor(row);
          return (
            <article
              key={row.key}
              className="rounded-md border border-[var(--aria-line)] bg-white p-3"
            >
              <div className="flex min-w-0 flex-wrap items-start justify-between gap-2">
                <h3 className="aria-mono min-w-0 break-all text-sm font-semibold text-[var(--aria-ink)]">
                  {row.capability}
                </h3>
                {row.satisfied ? (
                  <span
                    data-testid="checklist-row-satisfied"
                    className="aria-chip border-[var(--aria-topo-node-done-border)] bg-[var(--aria-topo-node-done-bg)] text-[var(--aria-topo-node-done-fg)]"
                  >
                    <Check className="h-3.5 w-3.5" aria-hidden="true" />
                    满足
                  </span>
                ) : (
                  <span
                    data-testid="checklist-row-gap"
                    className="aria-chip border-[var(--aria-topo-node-failed-border)] bg-[var(--aria-topo-node-failed-bg)] text-[var(--aria-topo-node-failed-fg)]"
                  >
                    <X className="h-3.5 w-3.5" aria-hidden="true" />
                    缺口
                  </span>
                )}
              </div>
              <p className="aria-mono mt-1 break-all text-xs text-[var(--aria-ink-muted)]">
                {row.from} → {row.to} · {row.contractId}
              </p>
              <div className="mt-2 grid gap-2 text-xs sm:grid-cols-2">
                <div className="rounded border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-2">
                  <h4 className="text-[11px] font-semibold text-[var(--aria-ink-muted)]">
                    Required
                  </h4>
                  <p className="mt-1 break-all text-[var(--aria-ink)]">
                    {row.to} 需要 {row.capability}
                  </p>
                </div>
                <div
                  className={`rounded border p-2 ${
                    row.satisfied
                      ? "border-[var(--aria-topo-node-done-border)] bg-[var(--aria-topo-node-done-bg)]"
                      : "border-[var(--aria-topo-node-failed-border)] bg-[var(--aria-topo-node-failed-bg)]"
                  }`}
                >
                  <h4 className="text-[11px] font-semibold text-[var(--aria-ink-muted)]">
                    Provided
                  </h4>
                  <p className="mt-1 text-[var(--aria-ink)]">
                    {row.satisfied
                      ? `已由 ${row.from} 提供`
                      : `上游 ${row.from} 未提供`}
                  </p>
                </div>
              </div>
              {row.satisfied ? null : row.matchedFindings.length > 0 ? (
                <ul className="mt-2 space-y-1 text-xs text-[var(--aria-ink-muted)]">
                  {row.matchedFindings.map((finding) => (
                    <li key={`${finding.code}-${finding.message}`} className="break-all">
                      <span className="aria-mono">{finding.code}</span>
                      {finding.severity ? `（${finding.severity}）` : ""}：{finding.message}
                    </li>
                  ))}
                </ul>
              ) : null}
              {row.satisfied ? null : target ? (
                <button
                  type="button"
                  onClick={() => onJumpToFinding(target)}
                  className="mt-2 inline-flex min-h-11 items-center gap-1 rounded-md border border-amber-200 bg-white px-3 text-xs font-semibold text-amber-700 hover:bg-amber-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
                >
                  <CornerDownRight className="h-3.5 w-3.5" aria-hidden="true" />
                  查看 finding
                </button>
              ) : (
                <p className="mt-2 text-xs text-[var(--aria-ink-muted)]">
                  无关联 finding（以门禁 finding 为准）
                </p>
              )}
            </article>
          );
        })}
      </div>
    </div>
  );
}
