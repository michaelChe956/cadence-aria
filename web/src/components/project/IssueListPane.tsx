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
      className="rounded-lg border-2 border-slate-200 bg-white p-4 shadow-[0_8px_0_rgba(15,23,42,0.06),0_18px_34px_rgba(15,23,42,0.08)]"
    >
      <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-xl font-black text-[#241B2F]">Issue</h1>
          <p className="mt-1 text-sm font-semibold text-[#5E516B]">Legacy 队列</p>
        </div>
        <span className="inline-flex items-center gap-2 rounded-lg border-2 border-slate-200 bg-slate-50 px-3 py-1 text-xs font-black text-slate-700">
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
                      ? "w-full rounded-lg border-2 border-cyan-500 bg-cyan-50 px-3 py-3 text-left shadow-[0_5px_0_rgba(6,182,212,0.20)] focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-cyan-200"
                      : "w-full rounded-lg border-2 border-slate-200 bg-white px-3 py-3 text-left shadow-[0_4px_0_rgba(15,23,42,0.06)] transition-colors hover:bg-slate-50 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-cyan-200 disabled:bg-slate-100"
                  }
                >
                  <span className="flex min-w-0 items-start gap-2">
                    <CircleDot className="mt-0.5 h-4 w-4 shrink-0 text-cyan-700" />
                    <span className="min-w-0">
                      <span className="block truncate text-sm font-black text-[#241B2F]">
                        {issue.title}
                      </span>
                      <span className="mt-1 flex flex-wrap gap-2 font-mono text-[11px] font-bold text-slate-600">
                        <span>{issue.issue_id}</span>
                        <span>{issue.status}</span>
                      </span>
                    </span>
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
      ) : (
        <div className="rounded-lg border-2 border-dashed border-slate-200 bg-slate-50 px-3 py-5 text-sm font-semibold text-slate-500">
          暂无 Issue
        </div>
      )}
    </section>
  );
}
