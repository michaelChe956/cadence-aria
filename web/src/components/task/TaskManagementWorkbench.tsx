import { FileText, Plus, Play, RefreshCw } from "lucide-react";
import { useEffect, useState } from "react";
import {
  createIssue,
  createWorkspace,
  listIssues,
  listWorkspaces,
  startIssue,
} from "../../api/client";
import type { Issue, Workspace } from "../../api/types";
import { WorkspaceManager } from "../workspace/WorkspaceManager";
import { StartIssueDialog } from "./StartIssueDialog";

export type ExecutionContext = {
  issueId: string;
  workspaceId: string;
  taskId: string;
};

export function TaskManagementWorkbench({
  onOpenExecution,
}: {
  onOpenExecution: (context: ExecutionContext) => void;
}) {
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [issues, setIssues] = useState<Issue[]>([]);
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [startingIssue, setStartingIssue] = useState<Issue | null>(null);

  useEffect(() => {
    let cancelled = false;
    setBusy(true);
    setError(null);
    Promise.all([listWorkspaces(), listIssues()])
      .then(([workspaceResponse, issueResponse]) => {
        if (cancelled) {
          return;
        }
        setWorkspaces(workspaceResponse.workspaces);
        setIssues(issueResponse.issues);
      })
      .catch((reason) => {
        if (!cancelled) {
          setError(reason instanceof Error ? reason.message : "load task management failed");
        }
      })
      .finally(() => {
        if (!cancelled) {
          setBusy(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function handleCreateWorkspace(payload: { name: string; path: string }) {
    setBusy(true);
    setError(null);
    try {
      const workspace = await createWorkspace(payload);
      setWorkspaces((current) => [...current, workspace]);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "create workspace failed");
    } finally {
      setBusy(false);
    }
  }

  async function handleCreateIssue() {
    if (title.trim() === "" || workspaces.length === 0) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const issue = await createIssue({
        title: title.trim(),
        description: description.trim() || null,
      });
      setIssues((current) => [issue, ...current]);
      setTitle("");
      setDescription("");
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "create issue failed");
    } finally {
      setBusy(false);
    }
  }

  async function handleStartIssue(workspaceId: string) {
    if (!startingIssue) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const response = await startIssue(startingIssue.issue_id, { workspace_id: workspaceId });
      onOpenExecution({
        issueId: response.issue_id,
        workspaceId: response.workspace_id,
        taskId: response.task_id,
      });
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "start issue failed");
    } finally {
      setBusy(false);
    }
  }

  const canCreateIssue = title.trim() !== "" && workspaces.length > 0 && !busy;

  return (
    <div className="min-h-screen bg-[#F8FAFC] text-[#241B2F]">
      <header
        role="banner"
        className="sticky top-0 z-20 flex min-h-16 flex-wrap items-center justify-between gap-3 border-b-2 border-slate-200 bg-white/92 px-4 py-3 shadow-[0_8px_28px_rgba(15,23,42,0.08)] backdrop-blur md:px-6 lg:px-8"
      >
        <div>
          <strong className="text-lg text-[#241B2F]">Aria Web</strong>
          <span className="ml-3 hidden text-sm font-semibold text-[#5E516B] sm:inline">
            task workbench
          </span>
        </div>
        <button
          type="button"
          onClick={() => window.location.reload()}
          className="inline-flex h-9 w-9 items-center justify-center rounded-lg border-2 border-slate-200 bg-white text-slate-700 shadow-[0_4px_0_rgba(15,23,42,0.08)] transition-colors hover:bg-slate-50 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-cyan-200"
          aria-label="刷新"
        >
          <RefreshCw className="h-4 w-4" />
        </button>
      </header>

      {error ? (
        <div
          role="alert"
          className="border-b-2 border-rose-200 bg-rose-100 px-4 py-2 text-sm font-semibold text-rose-800 md:px-6 lg:px-8"
        >
          {error}
        </div>
      ) : null}

      <main
        aria-label="任务管理工作台"
        className="grid min-h-[calc(100vh-4rem)] grid-cols-1 gap-5 px-4 py-5 md:px-6 lg:grid-cols-[minmax(0,1fr)_24rem] lg:px-8 xl:grid-cols-[minmax(0,1fr)_28rem]"
      >
        <section
          role="region"
          aria-label="任务管理"
          className="min-w-0 rounded-lg border-2 border-slate-200 bg-white p-4 shadow-[0_10px_0_rgba(15,23,42,0.06),0_18px_38px_rgba(15,23,42,0.08)] md:p-5"
        >
          <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
            <div>
              <h1 className="text-2xl font-black text-[#241B2F]">任务管理</h1>
              <p className="mt-1 text-sm font-semibold text-[#5E516B]">issue 队列</p>
            </div>
            <span className="inline-flex items-center gap-2 rounded-lg border-2 border-indigo-200 bg-indigo-50 px-3 py-1 text-xs font-black text-indigo-950">
              <FileText className="h-4 w-4" />
              {issues.length}
            </span>
          </div>

          <form
            className="mb-5 grid gap-3 rounded-lg border-2 border-dashed border-slate-200 bg-slate-50 p-3"
            onSubmit={(event) => {
              event.preventDefault();
              void handleCreateIssue();
            }}
          >
            <label className="text-xs font-black text-slate-800">
              issue 标题
              <input
                aria-label="issue 标题"
                className="mt-1 w-full rounded-lg border-2 border-slate-200 bg-white px-3 py-2 text-sm font-semibold text-slate-950 shadow-inner shadow-slate-200/70 outline-none transition-colors placeholder:text-slate-300 focus-visible:border-orange-400 focus-visible:ring-4 focus-visible:ring-orange-200"
                value={title}
                onChange={(event) => setTitle(event.target.value)}
              />
            </label>
            <label className="text-xs font-black text-slate-800">
              issue 描述
              <textarea
                aria-label="issue 描述"
                rows={3}
                className="mt-1 w-full resize-y rounded-lg border-2 border-slate-200 bg-white px-3 py-2 text-sm font-semibold leading-6 text-slate-950 shadow-inner shadow-slate-200/70 outline-none transition-colors placeholder:text-slate-300 focus-visible:border-orange-400 focus-visible:ring-4 focus-visible:ring-orange-200"
                value={description}
                onChange={(event) => setDescription(event.target.value)}
              />
            </label>
            <button
              type="submit"
              disabled={!canCreateIssue}
              className="inline-flex items-center justify-center justify-self-start rounded-lg border-2 border-indigo-600 bg-indigo-500 px-4 py-2 text-sm font-black text-white shadow-[0_5px_0_rgba(67,56,202,0.36)] transition-colors hover:bg-indigo-400 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-indigo-200 disabled:border-slate-300 disabled:bg-slate-200 disabled:text-slate-500 disabled:shadow-none"
            >
              <Plus className="mr-1 h-4 w-4" />
              新建 issue
            </button>
          </form>

          <ul aria-label="issue 列表" className="space-y-3">
            {issues.map((issue) => (
              <li
                key={issue.issue_id}
                className="rounded-lg border-2 border-slate-200 bg-white px-4 py-3 shadow-[0_5px_0_rgba(15,23,42,0.06)]"
              >
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div className="min-w-0">
                    <h2 className="truncate text-base font-black text-[#241B2F]">
                      {issue.title}
                    </h2>
                    <div className="mt-1 flex flex-wrap gap-2 font-mono text-xs font-bold text-[#7A6C83]">
                      <span>{issue.issue_id}</span>
                      <span>{issue.status}</span>
                      {issue.task_id ? <span>{issue.task_id}</span> : null}
                    </div>
                  </div>
                  <button
                    type="button"
                    disabled={busy || workspaces.length === 0}
                    onClick={() => setStartingIssue(issue)}
                    className="inline-flex items-center justify-center rounded-lg border-2 border-orange-600 bg-orange-500 px-3 py-2 text-sm font-black text-white shadow-[0_5px_0_rgba(154,52,18,0.42)] transition-colors hover:bg-orange-400 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-orange-200 disabled:border-slate-300 disabled:bg-slate-200 disabled:text-slate-500 disabled:shadow-none"
                  >
                    <Play className="mr-1 h-4 w-4" />
                    Start
                  </button>
                </div>
                {issue.description ? (
                  <p className="mt-2 text-sm font-semibold leading-6 text-[#5E516B]">
                    {issue.description}
                  </p>
                ) : null}
              </li>
            ))}
          </ul>
        </section>

        <aside className="space-y-5">
          <WorkspaceManager
            workspaces={workspaces}
            busy={busy}
            onCreateWorkspace={handleCreateWorkspace}
          />
        </aside>
      </main>

      <StartIssueDialog
        issue={startingIssue}
        workspaces={workspaces}
        busy={busy}
        onCancel={() => setStartingIssue(null)}
        onConfirm={handleStartIssue}
      />
    </div>
  );
}
