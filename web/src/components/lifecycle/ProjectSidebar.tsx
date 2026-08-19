import { FolderKanban, GitBranch, Plus, Trash2 } from "lucide-react";
import type { CodebaseSummaryDto, Project, Repository } from "../../api/types";

export function ProjectSidebar({
  projects,
  codebases,
  repositories,
  selectedProjectId,
  issueCount,
  busy,
  onSelectProject,
  onCreateProject,
  onAddCodebase,
  onDeleteProject,
  onDeleteRepository,
  onDeleteLogicalCodebase,
}: {
  projects: Project[];
  codebases: CodebaseSummaryDto[];
  repositories: Repository[];
  selectedProjectId: string | null;
  issueCount: number;
  busy: boolean;
  onSelectProject: (projectId: string) => void;
  onCreateProject: () => void;
  onAddCodebase: () => void;
  onDeleteProject: (projectId: string) => void;
  onDeleteRepository: (repositoryId: string) => void;
  /// R8：逻辑代码库软删入口（调用方负责二次确认）。
  onDeleteLogicalCodebase: (logicalCodebaseId: string) => void;
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
                  <li key={project.project_id} className="flex items-stretch gap-1">
                    <button
                      type="button"
                      aria-label={project.name}
                      aria-pressed={selected}
                      disabled={busy}
                      onClick={() => onSelectProject(project.project_id)}
                      className={
                        selected
                          ? "min-w-0 flex-1 rounded-md border border-[var(--aria-primary)] bg-[var(--aria-panel)] px-3 py-2 text-left ring-2 ring-[var(--aria-primary)]"
                          : "min-w-0 flex-1 rounded-md border border-transparent px-3 py-2 text-left hover:border-[var(--aria-line)] hover:bg-[var(--aria-panel)] disabled:opacity-60"
                      }
                    >
                      <span className="block truncate text-sm font-semibold text-[var(--aria-ink)]">
                        {project.name}
                      </span>
                      <span className="mt-1 block truncate font-mono text-[11px] text-[var(--aria-ink-muted)]">
                        {project.project_id}
                      </span>
                    </button>
                    <button
                      type="button"
                      aria-label={`删除 Project ${project.name}`}
                      disabled={busy}
                      onClick={() => onDeleteProject(project.project_id)}
                      className="inline-flex h-auto w-8 shrink-0 items-center justify-center rounded-md border border-[var(--aria-line)] text-[var(--aria-ink-muted)] hover:border-[var(--aria-danger)] hover:text-[var(--aria-danger)] disabled:opacity-60"
                    >
                      <Trash2 className="h-4 w-4" />
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
              {codebases.length}
            </span>
          </div>
          <button
            type="button"
            disabled={!selectedProjectId}
            onClick={onAddCodebase}
            className="mb-3 inline-flex h-8 w-full items-center justify-center rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 text-xs font-semibold text-[var(--aria-ink)] disabled:text-[var(--aria-ink-muted)]"
          >
            <Plus className="mr-1 h-4 w-4" />
            添加代码库
          </button>
          {selectedProjectId ? (
            codebases.length === 0 ? (
              <div className="rounded-md border border-dashed border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-2 text-xs text-[var(--aria-ink-muted)]">
                <p className="font-semibold text-[var(--aria-ink)]">还没有代码库</p>
                <p className="mt-1">先添加代码库，Issue 才能绑定代码上下文。</p>
              </div>
            ) : (
              <ul className="space-y-2">
                {codebases.map((codebase) => {
                  const repository = repositories.find(
                    (candidate) =>
                      candidate.repository_id === codebase.repository_id,
                  );
                  return (
                    <li
                      key={codebase.id}
                      data-testid="codebase-list-item"
                      className="flex items-start gap-2 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-2"
                    >
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-1.5">
                          <span
                            data-testid={`codebase-kind-${codebase.name}`}
                            className={
                              codebase.kind === "logical"
                                ? "rounded border border-[var(--aria-primary)] px-1 py-0.5 font-mono text-[10px] font-semibold text-[var(--aria-primary)]"
                                : "rounded border border-[var(--aria-line)] px-1 py-0.5 font-mono text-[10px] text-[var(--aria-ink-muted)]"
                            }
                          >
                            {codebase.kind === "logical" ? "逻辑" : "单仓"}
                          </span>
                          <p className="truncate text-xs font-semibold text-[var(--aria-ink)]">
                            {codebase.name}
                          </p>
                        </div>
                        <p className="mt-1 truncate font-mono text-[11px] text-[var(--aria-ink-muted)]">
                          {codebase.kind === "logical"
                            ? `成员：${codebase.member_count ?? 0}`
                            : (repository?.path ?? "")}
                        </p>
                      </div>
                      {codebase.kind === "logical" && codebase.logical_codebase_id ? (
                        <button
                          type="button"
                          aria-label={`删除逻辑代码库 ${codebase.name}`}
                          disabled={busy}
                          onClick={() =>
                            onDeleteLogicalCodebase(
                              codebase.logical_codebase_id ?? "",
                            )
                          }
                          className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md border border-[var(--aria-line)] text-[var(--aria-ink-muted)] hover:border-[var(--aria-danger)] hover:text-[var(--aria-danger)] disabled:opacity-60"
                        >
                          <Trash2 className="h-3.5 w-3.5" />
                        </button>
                      ) : null}
                      {codebase.kind === "single_repo" && codebase.repository_id ? (
                        <button
                          type="button"
                          aria-label={`删除代码库 ${codebase.name}`}
                          disabled={busy}
                          onClick={() =>
                            onDeleteRepository(codebase.repository_id ?? "")
                          }
                          className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md border border-[var(--aria-line)] text-[var(--aria-ink-muted)] hover:border-[var(--aria-danger)] hover:text-[var(--aria-danger)] disabled:opacity-60"
                        >
                          <Trash2 className="h-3.5 w-3.5" />
                        </button>
                      ) : null}
                    </li>
                  );
                })}
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
