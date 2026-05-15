import { ExternalLink, FileText } from "lucide-react";
import type { Issue } from "../../api/types";
import type { ExecutionContext } from "../task/TaskManagementWorkbench";
import { IssuePhaseRail } from "./IssuePhaseRail";

export type IssueDetailPaneProps = {
  issue: Issue | null;
  onOpenExecution: (context: ExecutionContext) => void;
};

export function IssueDetailPane({ issue, onOpenExecution }: IssueDetailPaneProps) {
  const executable = Boolean(issue?.workspace_id && issue.task_id);
  const disabledReason = issue ? executionDisabledReason(issue) : null;

  return (
    <section
      role="region"
      aria-label="Issue 详情"
      className="min-w-0 rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel)] p-4 shadow-sm"
    >
      {issue ? (
        <div className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_13rem]">
          <div className="min-w-0">
            <div className="mb-3 flex flex-wrap items-start justify-between gap-3">
              <div className="min-w-0">
                <h2 className="truncate text-lg font-semibold text-[var(--aria-ink)]">
                  {issue.title}
                </h2>
                <div className="mt-1 flex flex-wrap gap-2 font-mono text-xs font-medium text-[var(--aria-ink-muted)]">
                  <span>{issue.issue_id}</span>
                  <span className="rounded border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-1.5 py-0.5">
                    {issue.status}
                  </span>
                  {issue.task_id ? <span>{issue.task_id}</span> : null}
                </div>
              </div>
              <div className="flex max-w-full flex-col items-start gap-1 sm:items-end">
                <button
                  type="button"
                  disabled={!executable}
                  onClick={() => {
                    if (issue.workspace_id && issue.task_id) {
                      onOpenExecution({
                        issueId: issue.issue_id,
                        workspaceId: issue.workspace_id,
                        taskId: issue.task_id,
                      });
                    }
                  }}
                  className="inline-flex h-9 items-center justify-center rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 text-sm font-semibold text-white transition-colors hover:bg-cyan-700 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] disabled:border-[var(--aria-line)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
                >
                  <ExternalLink className="mr-1 h-4 w-4" />
                  打开执行
                </button>
                {disabledReason ? (
                  <p className="text-xs font-medium text-[var(--aria-ink-muted)]">
                    {disabledReason}
                  </p>
                ) : null}
              </div>
            </div>
            <p className="min-h-16 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 py-3 text-sm font-medium leading-6 text-[var(--aria-ink-muted)]">
              {issue.description || "暂无描述"}
            </p>
            <dl className="mt-4 grid gap-3 text-sm sm:grid-cols-2">
              <div>
                <dt className="text-[11px] font-semibold uppercase text-[var(--aria-ink-muted)]">
                  Change ID
                </dt>
                <dd className="mt-1 break-all font-mono text-xs font-medium text-[var(--aria-ink)]">
                  {issue.change_id}
                </dd>
              </div>
              <div>
                <dt className="text-[11px] font-semibold uppercase text-[var(--aria-ink-muted)]">
                  Workspace
                </dt>
                <dd className="mt-1 break-all font-mono text-xs font-medium text-[var(--aria-ink)]">
                  {issue.workspace_id ?? "-"}
                </dd>
              </div>
              <div>
                <dt className="text-[11px] font-semibold uppercase text-[var(--aria-ink-muted)]">
                  Task
                </dt>
                <dd className="mt-1 break-all font-mono text-xs font-medium text-[var(--aria-ink)]">
                  {issue.task_id ?? "-"}
                </dd>
              </div>
            </dl>
          </div>
          <IssuePhaseRail status={issue.status} />
        </div>
      ) : (
        <div className="flex min-h-56 items-center justify-center rounded-md border border-dashed border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-4 py-8 text-center">
          <div>
            <FileText className="mx-auto h-6 w-6 text-[var(--aria-ink-muted)]" />
            <p className="mt-2 text-sm font-medium text-[var(--aria-ink-muted)]">
              请选择一个 Issue
            </p>
          </div>
        </div>
      )}
    </section>
  );
}

function executionDisabledReason(issue: Issue) {
  if (!issue.workspace_id && !issue.task_id) {
    return "缺少 Workspace 与 Task，无法打开执行";
  }
  if (!issue.workspace_id) {
    return "缺少 Workspace，无法打开执行";
  }
  if (!issue.task_id) {
    return "未启动 Task，无法打开执行";
  }
  return null;
}
