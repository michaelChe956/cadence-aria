import { LoaderCircle, TriangleAlert } from "lucide-react";
import type {
  AggregateIndexActiveResponse,
  AggregateIndexState,
} from "../../api/types";

const STATE_LABELS: Record<AggregateIndexState, string> = {
  active: "索引可用",
  stale: "索引已过期",
  degraded: "索引降级",
  rebuilding: "正在重建索引",
  missing: "尚未建立索引",
};

const STATE_BADGE_CLASSES: Record<AggregateIndexState, string> = {
  active:
    "border-[var(--aria-success)] bg-[var(--aria-success-soft)] text-[var(--aria-success)]",
  stale:
    "border-[var(--aria-warning)] bg-[var(--aria-warning-soft)] text-[var(--aria-warning)]",
  degraded:
    "border-[var(--aria-danger)] bg-[var(--aria-danger-soft)] text-[var(--aria-danger)]",
  rebuilding:
    "border-[var(--aria-primary)] bg-[var(--aria-primary-soft)] text-[var(--aria-primary)]",
  missing:
    "border-[var(--aria-line)] bg-[var(--aria-panel-muted)] text-[var(--aria-ink-muted)]",
};

export type AggregateIndexCardProps = {
  index: AggregateIndexActiveResponse;
  rebuilding: boolean;
  onRebuild: () => void;
};

export function AggregateIndexCard({
  index,
  rebuilding,
  onRebuild,
}: AggregateIndexCardProps) {
  const isRebuilding = rebuilding || index.state === "rebuilding";

  return (
    <section
      data-testid="aggregate-index-card"
      className="border-b border-[var(--aria-line)] px-4 py-3"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-sm font-semibold text-[var(--aria-ink)]">
            聚合索引
          </span>
          <span
            data-testid="aggregate-index-status"
            data-state={index.state}
            className={`inline-flex items-center rounded border px-2 py-1 text-xs font-semibold ${STATE_BADGE_CLASSES[index.state]}`}
          >
            {index.state}
          </span>
          <span className="text-xs text-[var(--aria-ink-muted)]">
            {STATE_LABELS[index.state]}
          </span>
        </div>
        <button
          type="button"
          disabled={isRebuilding}
          onClick={onRebuild}
          className="inline-flex h-8 items-center rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 text-xs font-semibold text-white disabled:border-[var(--aria-line)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
        >
          {isRebuilding ? (
            <LoaderCircle
              data-testid="aggregate-index-spinner"
              className="mr-1 h-4 w-4 animate-spin"
              aria-hidden="true"
            />
          ) : null}
          重建索引
        </button>
      </div>

      {index.revision !== null && index.revision !== undefined ? (
        <p className="mt-2 text-xs text-[var(--aria-ink-muted)]">
          成员版本：{index.revision}
          {index.indexed_at ? ` · 索引时间：${index.indexed_at}` : ""}
        </p>
      ) : null}

      {index.state === "stale" ? (
        <div
          data-testid="aggregate-index-stale-notice"
          className="mt-2 flex items-center gap-2 rounded-md border border-[var(--aria-warning)] bg-[var(--aria-warning-soft)] px-3 py-2 text-xs text-[var(--aria-warning)]"
        >
          <TriangleAlert className="h-4 w-4 shrink-0" aria-hidden="true" />
          索引已过期，请重建后再使用聚合上下文。
        </div>
      ) : null}

      {index.warning ? (
        <div className="mt-2 rounded-md border border-[var(--aria-warning)] bg-[var(--aria-warning-soft)] px-3 py-2 text-xs text-[var(--aria-warning)]">
          {index.warning}
        </div>
      ) : null}
    </section>
  );
}
