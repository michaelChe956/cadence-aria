import { FolderKanban, Plus } from "lucide-react";
import type { Project } from "../../api/types";

export function ProjectSidebar({
  projects,
  selectedProjectId,
  issueCount,
  busy,
  onSelectProject,
  onCreateProject,
}: {
  projects: Project[];
  selectedProjectId: string | null;
  issueCount: number;
  busy: boolean;
  onSelectProject: (projectId: string) => void;
  onCreateProject: () => void;
}) {
  return (
    <nav
      aria-label="Project 切换"
      className="flex min-h-0 flex-col border-r border-[var(--aria-line)] bg-[var(--aria-panel-muted)]"
    >
      <div className="border-b border-[var(--aria-line)] p-3">
        <div className="mb-3 flex items-center justify-between gap-2">
          <div className="flex min-w-0 items-center gap-2">
            <FolderKanban className="h-4 w-4 shrink-0 text-[var(--aria-primary)]" />
            <h2 className="truncate text-sm font-semibold text-[var(--aria-ink)]">Projects</h2>
          </div>
          <span className="rounded border border-[var(--aria-line)] bg-[var(--aria-panel)] px-1.5 py-0.5 font-mono text-[11px] text-[var(--aria-ink-muted)]">
            {projects.length}
          </span>
        </div>
        <button
          type="button"
          onClick={onCreateProject}
          className="inline-flex h-8 w-full items-center justify-center rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 text-xs font-semibold text-white"
        >
          <Plus className="mr-1 h-4 w-4" />
          新建 Project
        </button>
      </div>

      <div className="min-h-0 flex-1 overflow-auto p-2">
        {projects.length === 0 ? (
          <div className="rounded-md border border-dashed border-[var(--aria-line)] bg-[var(--aria-panel)] p-3 text-sm text-[var(--aria-ink-muted)]">
            <p className="font-semibold text-[var(--aria-ink)]">还没有 Project</p>
            <p className="mt-1 text-xs">先创建 Project，再绑定代码库和 Issue。</p>
          </div>
        ) : (
          <ul className="space-y-1">
            {projects.map((project) => {
              const selected = project.project_id === selectedProjectId;
              return (
                <li key={project.project_id}>
                  <button
                    type="button"
                    aria-label={project.name}
                    aria-pressed={selected}
                    disabled={busy}
                    onClick={() => onSelectProject(project.project_id)}
                    className={
                      selected
                        ? "w-full rounded-md border border-[var(--aria-primary)] bg-[var(--aria-panel)] px-3 py-2 text-left ring-2 ring-[var(--aria-primary)]"
                        : "w-full rounded-md border border-transparent px-3 py-2 text-left hover:border-[var(--aria-line)] hover:bg-[var(--aria-panel)] disabled:opacity-60"
                    }
                  >
                    <span className="block truncate text-sm font-semibold text-[var(--aria-ink)]">
                      {project.name}
                    </span>
                    <span className="mt-1 block truncate font-mono text-[11px] text-[var(--aria-ink-muted)]">
                      {project.project_id}
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </div>

      <div className="border-t border-[var(--aria-line)] p-3">
        <dl className="grid gap-2 text-xs">
          <div>
            <dt className="font-semibold text-[var(--aria-ink-muted)]">当前 Issue</dt>
            <dd className="mt-1 font-mono text-[var(--aria-ink)]">{issueCount}</dd>
          </div>
        </dl>
      </div>
    </nav>
  );
}
