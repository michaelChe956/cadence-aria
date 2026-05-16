import { FolderKanban, GitBranch, Plus } from "lucide-react";
import type { Project, Repository } from "../../api/types";

export function ProjectSidebar({
  projects,
  repositories,
  selectedProjectId,
  issueCount,
  busy,
  onSelectProject,
  onCreateProject,
  onCreateRepository,
}: {
  projects: Project[];
  repositories: Repository[];
  selectedProjectId: string | null;
  issueCount: number;
  busy: boolean;
  onSelectProject: (projectId: string) => void;
  onCreateProject: () => void;
  onCreateRepository: () => void;
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

      <div className="min-h-0 flex-1 space-y-3 overflow-auto p-2">
        <section>
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
        </section>

        <section
          aria-label="当前 Project 代码库"
          className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] p-3"
        >
          <div className="mb-3 flex items-center justify-between gap-2">
            <div className="flex min-w-0 items-center gap-2">
              <GitBranch className="h-4 w-4 shrink-0 text-[var(--aria-primary)]" />
              <h3 className="truncate text-sm font-semibold text-[var(--aria-ink)]">代码库</h3>
            </div>
            <span className="rounded border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-1.5 py-0.5 font-mono text-[11px] text-[var(--aria-ink-muted)]">
              {repositories.length}
            </span>
          </div>
          <button
            type="button"
            disabled={!selectedProjectId}
            onClick={onCreateRepository}
            className="mb-3 inline-flex h-8 w-full items-center justify-center rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 text-xs font-semibold text-[var(--aria-ink)] disabled:text-[var(--aria-ink-muted)]"
          >
            <Plus className="mr-1 h-4 w-4" />
            添加代码库
          </button>
          {selectedProjectId ? (
            repositories.length === 0 ? (
              <div className="rounded-md border border-dashed border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-2 text-xs text-[var(--aria-ink-muted)]">
                <p className="font-semibold text-[var(--aria-ink)]">还没有代码库</p>
                <p className="mt-1">先添加代码库，Issue 才能绑定代码上下文。</p>
              </div>
            ) : (
              <ul className="space-y-2">
                {repositories.map((repository) => (
                  <li
                    key={repository.repository_id}
                    className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-2"
                  >
                    <p className="truncate text-xs font-semibold text-[var(--aria-ink)]">
                      {repository.name}
                    </p>
                    <p className="mt-1 truncate font-mono text-[11px] text-[var(--aria-ink-muted)]">
                      {repository.path}
                    </p>
                  </li>
                ))}
              </ul>
            )
          ) : (
            <p className="rounded-md border border-dashed border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-2 text-xs text-[var(--aria-ink-muted)]">
              先选择 Project
            </p>
          )}
        </section>
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
