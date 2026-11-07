import { FolderKanban, GitBranch, Package, Plus, Settings, Trash2 } from "lucide-react";
import type { CodebaseSummaryDto, LogicalCodebaseListEntryDto, Project, Repository } from "../../api/types";

export function ProjectSidebar({
  projects,
  codebases,
  deletedLogicalCodebases,
  repositories,
  selectedProjectId,
  issueCount,
  busy,
  onSelectProject,
  onCreateProject,
  onAddCodebase,
  onOpenProviderManagement,
  onDeleteProject,
  onDeleteRepository,
  onDeleteLogicalCodebase,
  onRestoreLogicalCodebase,
  onPurgeLogicalCodebase,
}: {
  projects: Project[];
  codebases: CodebaseSummaryDto[];
  /** 2026-10-29：已软删（墓碑）LC，渲染在「已删除」分区。 */
  deletedLogicalCodebases: LogicalCodebaseListEntryDto[];
  repositories: Repository[];
  selectedProjectId: string | null;
  issueCount: number;
  busy: boolean;
  onSelectProject: (projectId: string) => void;
  onCreateProject: () => void;
  onAddCodebase: () => void;
  /** 2026-11-04：打开独立的「Provider 管理」面板（provider 能力核验已全局化，
   *  不再寄居在逻辑代码库抽屉里）。 */
  onOpenProviderManagement: () => void;
  onDeleteProject: (projectId: string) => void;
  onDeleteRepository: (repositoryId: string) => void;
  /// R8：逻辑代码库软删入口（调用方负责二次确认）。
  onDeleteLogicalCodebase: (logicalCodebaseId: string) => void;
  /// 2026-10-29：已软删 LC 的恢复入口。
  onRestoreLogicalCodebase: (logicalCodebaseId: string) => void;
  /// 2026-10-29：已软删 LC 的彻底删除入口（调用方负责二次确认与不可恢复提示）。
  onPurgeLogicalCodebase: (logicalCodebaseId: string) => void;
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
          data-testid="onboarding-anchor-project-create"
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
            data-testid="onboarding-anchor-codebase-add"
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

        {/* 2026-11-04：Provider 管理独立入口——provider 的探针/核验/更新/
            安装/版本不再寄居在逻辑代码库抽屉里，改由侧栏这个平级分区进入
            独立面板。2026-11-05 机器级化（REQ-PCR-08）：能力面与项目无关 ⇒
            没有选中项目（甚至零项目）时入口也保持可点。 */}
        <section
          aria-label="Provider 管理"
          data-testid="provider-management-section"
          className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] p-3"
        >
          <div className="mb-3 flex items-center justify-between gap-2">
            <div className="flex min-w-0 items-center gap-2">
              <Package className="h-4 w-4 shrink-0 text-[var(--aria-primary)]" />
              <h3 className="truncate text-sm font-semibold text-[var(--aria-ink)]">
                Provider
              </h3>
            </div>
          </div>
          <button
            type="button"
            data-testid="provider-management-open"
            onClick={onOpenProviderManagement}
            className="inline-flex h-8 w-full items-center justify-center rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 text-xs font-semibold text-[var(--aria-ink)] transition-colors duration-200 hover:border-[var(--aria-primary)] hover:text-[var(--aria-primary)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          >
            <Settings className="mr-1 h-4 w-4" />
            Provider 管理
          </button>
        </section>

        {/* 2026-10-29：已软删（墓碑）LC 分区——只有这里出现「恢复」与「彻底删除」，
            存活条目绝无这两个按钮。 */}
        {selectedProjectId && deletedLogicalCodebases.length > 0 ? (
          <section
            aria-label="已删除的逻辑代码库"
            data-testid="deleted-logical-codebases"
            className="rounded-md border border-dashed border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-3"
          >
            <div className="mb-2 flex items-center justify-between gap-2">
              <h3 className="truncate text-sm font-semibold text-[var(--aria-ink-muted)]">已删除</h3>
              <span className="rounded border border-[var(--aria-line)] bg-[var(--aria-panel)] px-1.5 py-0.5 font-mono text-[11px] text-[var(--aria-ink-muted)]">
                {deletedLogicalCodebases.length}
              </span>
            </div>
            <ul className="space-y-2">
              {deletedLogicalCodebases.map((codebase) => (
                <li
                  key={codebase.id}
                  data-testid="deleted-codebase-list-item"
                  className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] p-2"
                >
                  <p className="truncate text-xs font-semibold text-[var(--aria-ink)]">
                    {codebase.name}
                  </p>
                  <p className="mt-1 truncate font-mono text-[11px] text-[var(--aria-ink-muted)]">
                    删除于 {codebase.deleted_at ?? "未知时间"}
                  </p>
                  <div className="mt-2 flex items-center gap-1">
                    <button
                      type="button"
                      aria-label={`恢复逻辑代码库 ${codebase.name}`}
                      disabled={busy}
                      onClick={() => onRestoreLogicalCodebase(codebase.id)}
                      className="inline-flex h-7 items-center rounded-md border border-[var(--aria-line)] px-2 text-[11px] font-semibold text-[var(--aria-ink)] hover:border-[var(--aria-primary)] hover:text-[var(--aria-primary)] disabled:opacity-60"
                    >
                      恢复
                    </button>
                    <button
                      type="button"
                      aria-label={`彻底删除逻辑代码库 ${codebase.name}`}
                      disabled={busy}
                      onClick={() => onPurgeLogicalCodebase(codebase.id)}
                      className="inline-flex h-7 items-center rounded-md border border-[var(--aria-line)] px-2 text-[11px] font-semibold text-[var(--aria-ink-muted)] hover:border-[var(--aria-danger)] hover:text-[var(--aria-danger)] disabled:opacity-60"
                    >
                      彻底删除
                    </button>
                  </div>
                </li>
              ))}
            </ul>
          </section>
        ) : null}
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
