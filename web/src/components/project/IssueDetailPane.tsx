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

  return (
    <section
      role="region"
      aria-label="Issue 详情"
      className="min-w-0 rounded-lg border-2 border-slate-200 bg-white p-4 shadow-[0_8px_0_rgba(15,23,42,0.06),0_18px_34px_rgba(15,23,42,0.08)]"
    >
      {issue ? (
        <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_14rem]">
          <div className="min-w-0">
            <div className="mb-3 flex flex-wrap items-start justify-between gap-3">
              <div className="min-w-0">
                <h2 className="truncate text-2xl font-black text-[#241B2F]">{issue.title}</h2>
                <div className="mt-1 flex flex-wrap gap-2 font-mono text-xs font-bold text-slate-600">
                  <span>{issue.issue_id}</span>
                  <span>{issue.status}</span>
                  {issue.task_id ? <span>{issue.task_id}</span> : null}
                </div>
              </div>
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
                className="inline-flex items-center justify-center rounded-lg border-2 border-cyan-700 bg-cyan-600 px-3 py-2 text-sm font-black text-white shadow-[0_5px_0_rgba(8,145,178,0.34)] transition-colors hover:bg-cyan-500 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-cyan-200 disabled:border-slate-300 disabled:bg-slate-200 disabled:text-slate-500 disabled:shadow-none"
              >
                <ExternalLink className="mr-1 h-4 w-4" />
                打开执行
              </button>
            </div>
            <p className="min-h-16 rounded-lg border-2 border-slate-100 bg-slate-50 px-3 py-3 text-sm font-semibold leading-6 text-[#5E516B]">
              {issue.description || "暂无描述"}
            </p>
            <dl className="mt-4 grid gap-3 text-sm sm:grid-cols-2">
              <div>
                <dt className="text-xs font-black text-slate-500">Change</dt>
                <dd className="mt-1 break-all font-mono text-xs font-bold text-slate-800">
                  {issue.change_id}
                </dd>
              </div>
              <div>
                <dt className="text-xs font-black text-slate-500">Workspace</dt>
                <dd className="mt-1 break-all font-mono text-xs font-bold text-slate-800">
                  {issue.workspace_id ?? "-"}
                </dd>
              </div>
            </dl>
          </div>
          <IssuePhaseRail status={issue.status} />
        </div>
      ) : (
        <div className="flex min-h-56 items-center justify-center rounded-lg border-2 border-dashed border-slate-200 bg-slate-50 px-4 py-8 text-center">
          <div>
            <FileText className="mx-auto h-6 w-6 text-slate-400" />
            <p className="mt-2 text-sm font-bold text-slate-500">请选择一个 Issue</p>
          </div>
        </div>
      )}
    </section>
  );
}
