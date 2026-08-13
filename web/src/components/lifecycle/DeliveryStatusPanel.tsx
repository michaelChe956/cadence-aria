import { GitBranch } from "lucide-react";
import type { DeliveryEntryDto, IssueDeliverySummaryDto } from "../../api/types";

const OVERALL_LABELS: Record<IssueDeliverySummaryDto["overall"], string> = {
  all_pushed: "已全部交付",
  partial: "部分交付",
  none: "无 Work Item，不可交付",
};

const OVERALL_BADGE_CLASSES: Record<
  IssueDeliverySummaryDto["overall"],
  string
> = {
  all_pushed:
    "border-[var(--aria-success)] bg-[var(--aria-success-soft)] text-[var(--aria-success)]",
  partial:
    "border-[var(--aria-warning)] bg-[var(--aria-warning-soft)] text-[var(--aria-warning)]",
  none: "border-[var(--aria-line)] bg-[var(--aria-panel-muted)] text-[var(--aria-ink-muted)]",
};

export function DeliveryStatusPanel({
  summary,
}: {
  summary: IssueDeliverySummaryDto;
}) {
  return (
    <section
      data-testid="delivery-status-panel"
      className="border-b border-[var(--aria-line)] px-4 py-3"
    >
      <div className="mb-2 flex items-center gap-2">
        <span
          data-testid="delivery-status-badge"
          data-status={summary.overall}
          className={`inline-flex items-center rounded border px-2 py-1 text-xs font-semibold ${OVERALL_BADGE_CLASSES[summary.overall]}`}
        >
          {OVERALL_LABELS[summary.overall]}
        </span>
      </div>
      {summary.overall !== "none" && summary.entries.length > 0 ? (
        <div className="space-y-2">
          {summary.entries.map((entry) => (
            <DeliveryEntryRow key={entry.work_item_id} entry={entry} />
          ))}
        </div>
      ) : null}
    </section>
  );
}

function DeliveryEntryRow({ entry }: { entry: DeliveryEntryDto }) {
  const failed = entry.push_status === "failed";
  const pushed = entry.push_status === "pushed";
  const statusLabel = pushed ? "已推送" : failed ? "推送失败" : "未推送";

  return (
    <div
      data-testid="delivery-entry-row"
      data-status={failed ? "failed" : pushed ? "pushed" : "pending"}
      className={`rounded-md border px-2 py-2 text-xs ${
        failed
          ? "border-[var(--aria-danger)] bg-[var(--aria-danger-soft)]"
          : "border-[var(--aria-line)] bg-[var(--aria-panel-muted)]"
      }`}
    >
      <div className="flex items-center justify-between gap-2">
        <span className="font-semibold text-[var(--aria-ink)]">
          {entry.repository_name}
        </span>
        <span
          className={
            failed
              ? "font-semibold text-[var(--aria-danger)]"
              : pushed
                ? "font-semibold text-[var(--aria-success)]"
                : "text-[var(--aria-ink-muted)]"
          }
        >
          {statusLabel}
        </span>
      </div>
      <div className="mt-1 flex items-center gap-2 text-[var(--aria-ink-muted)]">
        <GitBranch className="h-3.5 w-3.5 shrink-0" />
        <span className="truncate">{entry.branch_name ?? "—"}</span>
        {entry.commit_sha ? (
          <span className="shrink-0 font-mono text-[11px]">
            {entry.commit_sha.slice(0, 7)}
          </span>
        ) : null}
      </div>
      {failed && entry.push_error ? (
        <div
          data-testid="delivery-entry-error"
          className="mt-1 text-[var(--aria-danger)]"
        >
          {entry.push_error}
        </div>
      ) : null}
    </div>
  );
}
