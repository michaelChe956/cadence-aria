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
      className="fixed inset-x-4 top-24 z-30 mx-auto max-w-xl rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel)] p-4 text-[var(--aria-ink)] shadow-lg"
    >
      <div className="mb-4 flex items-start justify-between gap-4">
        <div>
          <h2 className="text-lg font-semibold text-[var(--aria-ink)]">选择 workspace</h2>
          <p className="mt-1 font-mono text-xs font-medium text-[var(--aria-ink-muted)]">{issue.issue_id}</p>
        </div>
        <button
          type="button"
          aria-label="关闭 Start"
          onClick={onCancel}
          className="inline-flex h-9 w-9 items-center justify-center rounded-md border border-[var(--aria-line-strong)] bg-[var(--aria-panel)] text-[var(--aria-ink-muted)] transition-colors hover:bg-[var(--aria-panel-muted)] hover:text-[var(--aria-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
        >
          <X className="h-4 w-4" />
        </button>
      </div>

      <div className="grid gap-3">
        <label className="text-xs font-semibold text-[var(--aria-ink-muted)]">
          启动 workspace
          <select
            aria-label="启动 workspace"
            className="mt-1 h-9 w-full rounded-md border border-[var(--aria-line-strong)] bg-[var(--aria-panel)] px-3 text-sm font-medium text-[var(--aria-ink)] outline-none transition-colors focus-visible:border-[var(--aria-primary)] focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
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
          className="inline-flex h-9 items-center justify-center rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-4 text-sm font-semibold text-white transition-opacity hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] disabled:border-[var(--aria-line)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
        >
          <Play className="mr-1 h-4 w-4" />
          确认 Start
        </button>
      </div>
    </section>
  );
}
