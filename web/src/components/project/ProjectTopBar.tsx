import { FolderKanban, RefreshCw } from "lucide-react";
import type { Project } from "../../api/types";
import { ProjectSwitcher } from "./ProjectSwitcher";

export type ProjectTopBarProps = {
  projects: Project[];
  selectedProjectId: string | null;
  issueCount: number;
  busy: boolean;
  onSelectProject: (projectId: string) => void;
  onRefresh: () => void;
};

export function ProjectTopBar({
  projects,
  selectedProjectId,
  issueCount,
  busy,
  onSelectProject,
  onRefresh,
}: ProjectTopBarProps) {
  return (
    <div className="flex min-w-0 flex-wrap items-center justify-between gap-3">
      <div className="flex min-w-0 flex-wrap items-center gap-3">
        <div className="flex items-center gap-2">
          <FolderKanban className="h-4 w-4 text-[var(--aria-primary)]" />
          <strong className="text-base font-semibold text-[var(--aria-ink)]">Aria Web</strong>
        </div>
        <span className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-2 py-1 text-xs font-semibold text-[var(--aria-ink-muted)]">
          项目工作台
        </span>
        <span className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-2 py-1 font-mono text-xs font-semibold text-[var(--aria-ink-muted)]">
          Issue {issueCount}
        </span>
      </div>
      <div className="flex min-w-0 flex-wrap items-center justify-end gap-2">
        <ProjectSwitcher
          projects={projects}
          selectedProjectId={selectedProjectId}
          disabled={busy}
          onSelectProject={onSelectProject}
        />
        <button
          type="button"
          onClick={onRefresh}
          disabled={busy}
          className="inline-flex h-8 items-center justify-center rounded-md border border-[var(--aria-line-strong)] bg-[var(--aria-panel)] px-3 text-sm font-semibold text-[var(--aria-ink)] transition-colors hover:bg-[var(--aria-panel-muted)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
        >
          <RefreshCw className="mr-1 h-4 w-4" />
          刷新
        </button>
      </div>
    </div>
  );
}
