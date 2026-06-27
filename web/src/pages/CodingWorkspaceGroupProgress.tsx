import type { CodingExecutionUnit } from "../api/types";

function currentUnitIndex(
  currentWorkItemId: string | null,
  units: CodingExecutionUnit[],
): number {
  if (units.length === 0) {
    return -1;
  }
  if (currentWorkItemId) {
    const matchedIndex = units.findIndex(
      (unit) => unit.work_item_id === currentWorkItemId,
    );
    if (matchedIndex >= 0) {
      return matchedIndex;
    }
  }
  const completedCount = units.filter((unit) => unit.status === "completed").length;
  return Math.min(completedCount, units.length - 1);
}

export function CodingWorkspaceGroupProgress({
  planId,
  currentWorkItemId,
  units,
}: {
  planId: string | null;
  currentWorkItemId: string | null;
  units: CodingExecutionUnit[];
}) {
  if (!planId || units.length === 0) {
    return null;
  }

  const activeIndex = currentUnitIndex(currentWorkItemId, units);
  const progressValue = activeIndex >= 0 ? activeIndex + 1 : 0;

  return (
    <section className="grid min-h-11 shrink-0 grid-cols-[minmax(0,1fr)_auto] gap-x-3 gap-y-1 border-b border-[var(--aria-line)] bg-[var(--aria-panel)] px-4 py-2 text-xs md:grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] md:items-center">
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
