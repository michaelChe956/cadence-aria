import { CircleDot, ListChecks } from "lucide-react";
import type { Issue } from "../../api/types";

export type IssueListPaneProps = {
  issues: Issue[];
  selectedIssueId: string | null;
  busy: boolean;
  onSelectIssue: (issueId: string) => void;
};

export function IssueListPane({ issues, selectedIssueId, busy, onSelectIssue }: IssueListPaneProps) {
  return (
    <section
      role="region"
      aria-label="Issue 列表面板"
      className="rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel)] p-3 shadow-sm"
    >
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
        <div>
          <h2 className="text-sm font-semibold text-[var(--aria-ink)]">Issue</h2>
          <p className="mt-0.5 text-xs font-medium text-[var(--aria-ink-muted)]">Legacy 队列</p>
        </div>
        <span className="inline-flex h-7 items-center gap-1.5 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-2 font-mono text-xs font-semibold text-[var(--aria-ink-muted)]">
          <ListChecks className="h-4 w-4" />
          {issues.length}
        </span>
      </div>
      {issues.length > 0 ? (
        <ul aria-label="Issue 列表" className="space-y-2">
          {issues.map((issue) => {
            const selected = issue.issue_id === selectedIssueId;
            return (
              <li key={issue.issue_id}>
                <button
                  type="button"
                  aria-pressed={selected}
                  disabled={busy}
                  onClick={() => onSelectIssue(issue.issue_id)}
                  className={
                    selected
                      ? "w-full rounded-md border border-[var(--aria-line-strong)] border-l-4 border-l-[var(--aria-primary)] bg-[var(--aria-panel-muted)] px-3 py-2 text-left shadow-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
                      : "w-full rounded-md border border-[var(--aria-line)] border-l-4 border-l-transparent bg-[var(--aria-panel)] px-3 py-2 text-left transition-colors hover:bg-[var(--aria-panel-muted)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] disabled:bg-[var(--aria-panel-muted)]"
                  }
                >
                  <span className="flex min-w-0 items-start gap-2">
                    <CircleDot className="mt-0.5 h-4 w-4 shrink-0 text-[var(--aria-primary)]" />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-sm font-semibold text-[var(--aria-ink)]">
                        {issue.title}
                      </span>
                      <span className="mt-1 flex flex-wrap items-center gap-1.5 font-mono text-[11px] font-medium text-[var(--aria-ink-muted)]">
                        <span className="break-all">{issue.issue_id}</span>
                        <span className={statusBadgeClass(issue.status)}>{issue.status}</span>
                        <span className="break-all">{issue.task_id ?? "未启动"}</span>
                      </span>
                    </span>
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
      ) : (
        <div className="rounded-md border border-dashed border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 py-5 text-sm font-medium text-[var(--aria-ink-muted)]">
          暂无 Issue
        </div>
      )}
    </section>
  );
}

function statusBadgeClass(status: string) {
  if (status === "blocked") {
    return "rounded border border-[var(--aria-warning)] bg-[var(--aria-warning-soft)] px-1.5 py-0.5 text-[var(--aria-warning)]";
  }
  return "rounded border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-1.5 py-0.5 text-[var(--aria-ink-muted)]";
}
