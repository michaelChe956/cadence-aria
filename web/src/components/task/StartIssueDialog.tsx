import { Play, X } from "lucide-react";
import { useEffect, useState } from "react";
import type { Issue, Workspace } from "../../api/types";

export function StartIssueDialog({
  issue,
  workspaces,
  busy,
  onCancel,
  onConfirm,
}: {
  issue: Issue | null;
  workspaces: Workspace[];
  busy: boolean;
  onCancel: () => void;
  onConfirm: (workspaceId: string) => void | Promise<void>;
}) {
  const [workspaceId, setWorkspaceId] = useState("");

  useEffect(() => {
    setWorkspaceId(workspaces[0]?.workspace_id ?? "");
  }, [issue, workspaces]);

  if (!issue) {
    return null;
  }

  return (
    <section
      role="dialog"
      aria-label="选择 workspace"
      className="fixed inset-x-4 top-24 z-30 mx-auto max-w-xl rounded-lg border-2 border-orange-300 bg-white p-4 shadow-[0_18px_0_rgba(249,115,22,0.14),0_28px_60px_rgba(36,27,47,0.24)]"
    >
      <div className="mb-4 flex items-start justify-between gap-4">
        <div>
          <h2 className="text-lg font-black text-[#241B2F]">选择 workspace</h2>
          <p className="mt-1 font-mono text-xs font-bold text-[#7A6C83]">{issue.issue_id}</p>
        </div>
        <button
          type="button"
          aria-label="关闭 Start"
          onClick={onCancel}
          className="inline-flex h-9 w-9 items-center justify-center rounded-lg border-2 border-rose-200 bg-white text-rose-800 shadow-[0_4px_0_rgba(190,24,93,0.12)] transition-colors hover:bg-rose-50 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-rose-200"
        >
          <X className="h-4 w-4" />
        </button>
      </div>

      <div className="grid gap-3">
        <label className="text-xs font-black text-orange-900">
          启动 workspace
          <select
            aria-label="启动 workspace"
            className="mt-1 w-full rounded-lg border-2 border-orange-100 bg-white px-3 py-2 text-sm font-semibold text-orange-950 shadow-inner shadow-orange-200/60 outline-none transition-colors focus-visible:border-orange-400 focus-visible:ring-4 focus-visible:ring-orange-200"
            value={workspaceId}
            onChange={(event) => setWorkspaceId(event.target.value)}
          >
            {workspaces.map((workspace) => (
              <option key={workspace.workspace_id} value={workspace.workspace_id}>
                {workspace.name} · {workspace.workspace_id}
              </option>
            ))}
          </select>
        </label>
        <button
          type="button"
          disabled={busy || workspaceId === ""}
          onClick={() => void onConfirm(workspaceId)}
          className="inline-flex items-center justify-center rounded-lg border-2 border-orange-600 bg-orange-500 px-4 py-2 text-sm font-black text-white shadow-[0_5px_0_rgba(154,52,18,0.42)] transition-colors hover:bg-orange-400 focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-orange-200 disabled:border-slate-300 disabled:bg-slate-200 disabled:text-slate-500 disabled:shadow-none"
        >
          <Play className="mr-1 h-4 w-4" />
          确认 Start
        </button>
      </div>
    </section>
  );
}
