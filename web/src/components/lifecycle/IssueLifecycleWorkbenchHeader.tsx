import { Plus, RefreshCw } from "lucide-react";

export function IssueLifecycleWorkbenchHeader({
  projectName,
  focusedIssueId,
  canCreateIssue,
  onShowAll,
  onRefresh,
  onCreateIssue,
}: {
  projectName?: string;
  focusedIssueId: string | null;
  canCreateIssue: boolean;
  onShowAll: () => void;
  onRefresh: () => void;
  onCreateIssue: () => void;
}) {
  return (
    <div className="flex min-w-0 flex-wrap items-center justify-between gap-3">
      <div className="min-w-0">
        <h1 className="truncate text-base font-semibold text-[var(--aria-ink)]">
          Issue 生命周期工作台
        </h1>
        <p className="truncate text-xs text-[var(--aria-ink-muted)]">
          {projectName ?? "未选择 Project"}
        </p>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        {focusedIssueId ? (
          <button
            type="button"
            onClick={onShowAll}
            className="inline-flex h-8 items-center rounded-md border border-[var(--aria-line)] px-3 text-xs font-semibold text-[var(--aria-ink)]"
          >
            显示全部
          </button>
        ) : null}
        <button
          type="button"
          aria-label="刷新"
          onClick={onRefresh}
          className="inline-flex h-8 w-8 items-center justify-center rounded-md border border-[var(--aria-line)] text-[var(--aria-ink-muted)]"
        >
          <RefreshCw className="h-4 w-4" />
        </button>
        <button
          type="button"
          disabled={!canCreateIssue}
          onClick={onCreateIssue}
          className="inline-flex h-8 items-center rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 text-xs font-semibold text-white disabled:border-[var(--aria-line)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
        >
          <Plus className="mr-1 h-4 w-4" />
          新建 Issue
        </button>
      </div>
    </div>
  );
}
