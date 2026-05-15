import {
  CheckCircle2,
  FileText,
  FolderKanban,
  GitBranch,
  Layers3,
  Play,
  Plus,
  RefreshCw,
  Settings,
} from "lucide-react";
import type { ReactNode } from "react";
import { useEffect, useMemo, useState } from "react";
import {
  createProductIssue,
  createProject,
  createRepository,
  listProductIssues,
  listProjects,
  listRepositories,
  startProductIssue,
} from "../../api/client";
import type { ProductIssue, Project, Repository } from "../../api/types";
import { WorkbenchSurface } from "../shell/WorkbenchSurface";
import type { ExecutionContext } from "../task/TaskManagementWorkbench";

type LifecycleStageId = "story_spec" | "design_spec" | "work_item" | "done";

const LIFECYCLE_STAGES: Array<{
  id: LifecycleStageId;
  label: string;
  title: string;
  description: string;
}> = [
  {
    id: "story_spec",
    label: "Story Spec",
    title: "需求澄清",
    description: "用户故事、成功标准、未决问题",
  },
  {
    id: "design_spec",
    label: "Design Spec",
    title: "需求澄清",
    description: "数据模型、接口契约、风险约束",
  },
  {
    id: "work_item",
    label: "Work Item",
    title: "代码开发",
    description: "工作项、计划、执行上下文",
  },
  {
    id: "done",
    label: "Done",
    title: "代码开发完成",
    description: "执行完成、产物归档、后续验收",
  },
];

export function ProjectManagementWorkbench({
  onOpenExecution,
}: {
  onOpenExecution: (context: ExecutionContext) => void;
}) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [selectedProjectId, setSelectedProjectId] = useState<string | null>(null);
  const [repositories, setRepositories] = useState<Repository[]>([]);
  const [issues, setIssues] = useState<ProductIssue[]>([]);
  const [selectedIssueId, setSelectedIssueId] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [workspaceDialogOpen, setWorkspaceDialogOpen] = useState(false);
  const [issueDialogOpen, setIssueDialogOpen] = useState(false);
  const [repositoryDialogOpen, setRepositoryDialogOpen] = useState(false);
  const [runIssue, setRunIssue] = useState<ProductIssue | null>(null);

  useEffect(() => {
    void refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function refresh() {
    setBusy(true);
    setError(null);
    try {
      const projectResponse = await listProjects();
      setProjects(projectResponse.projects);
      const nextProjectId = selectedProjectId ?? projectResponse.projects[0]?.project_id ?? null;
      setSelectedProjectId(nextProjectId);
      if (nextProjectId) {
        await refreshWorkspace(nextProjectId);
      } else {
        setRepositories([]);
        setIssues([]);
      }
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "load workspace workbench failed");
    } finally {
      setBusy(false);
    }
  }

  async function refreshWorkspace(projectId: string) {
    const [repositoryResponse, issueResponse] = await Promise.all([
      listRepositories(projectId),
      listProductIssues(projectId),
    ]);
    const nextRepositories = repositoryResponse.repositories ?? [];
    const nextIssues = issueResponse.issues ?? [];
    setRepositories(nextRepositories);
    setIssues(nextIssues);
    setSelectedIssueId((current) => {
      if (current && nextIssues.some((issue) => issue.issue_id === current)) {
        return current;
      }
      return nextIssues[0]?.issue_id ?? null;
    });
  }

  async function handleSelectProject(projectId: string) {
    setSelectedProjectId(projectId);
    setBusy(true);
    setError(null);
    try {
      await refreshWorkspace(projectId);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "load workspace failed");
    } finally {
      setBusy(false);
    }
  }

  async function handleCreateWorkspace(payload: { name: string; description: string | null }) {
    setBusy(true);
    setError(null);
    try {
      const project = await createProject(payload);
      setProjects((current) => [...current, project]);
      setSelectedProjectId(project.project_id);
      setRepositories([]);
      setIssues([]);
      setSelectedIssueId(null);
      setWorkspaceDialogOpen(false);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "create workspace failed");
    } finally {
      setBusy(false);
    }
  }

  async function handleCreateRepository(payload: { name: string; path: string }) {
    if (!selectedProjectId) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const repository = await createRepository(selectedProjectId, {
        name: payload.name,
        path: payload.path,
        default_policy_preset: null,
        default_provider_mode: null,
      });
      setRepositories((current) => [...current, repository]);
      setRepositoryDialogOpen(false);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "create repository failed");
    } finally {
      setBusy(false);
    }
  }

  async function handleCreateIssue(payload: { title: string; description: string | null }) {
    if (!selectedProjectId) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const issue = await createProductIssue(selectedProjectId, {
        title: payload.title,
        description: payload.description,
        change_id: null,
      });
      setIssues((current) => [issue, ...current]);
      setSelectedIssueId(issue.issue_id);
      setIssueDialogOpen(false);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "create issue failed");
    } finally {
      setBusy(false);
    }
  }

  async function handleStartIssue(repositoryId: string) {
    if (!selectedProjectId || !runIssue) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const started = await startProductIssue(selectedProjectId, runIssue.issue_id, {
        repository_id: repositoryId,
      });
      setRunIssue(null);
      onOpenExecution({
        issueId: started.issue_id,
        workspaceId: started.workspace_id,
        taskId: started.task_id,
      });
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "start issue failed");
    } finally {
      setBusy(false);
    }
  }

  const selectedProject = projects.find((project) => project.project_id === selectedProjectId) ?? null;
  const selectedIssue =
    issues.find((issue) => issue.issue_id === selectedIssueId) ?? issues[0] ?? null;
  const issuesByStage = useMemo(() => groupIssuesByStage(issues), [issues]);

  return (
    <>
      <WorkbenchSurface
        mainLabel="任务管理页面"
        header={
          <div className="flex min-w-0 flex-wrap items-center justify-between gap-3">
            <div className="flex min-w-0 flex-wrap items-center gap-3">
              <div className="flex items-center gap-2">
                <FolderKanban className="h-4 w-4 text-[var(--aria-primary)]" />
                <strong className="text-base font-semibold text-[var(--aria-ink)]">
                  Aria Web
                </strong>
              </div>
              <span className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-2 py-1 text-xs font-semibold text-[var(--aria-ink-muted)]">
                Issue 工作台
              </span>
              <span className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-2 py-1 font-mono text-xs font-semibold text-[var(--aria-ink-muted)]">
                Issue {issues.length}
              </span>
            </div>
          </div>
        }
        alert={error}
        main={
          <div className="grid min-h-[calc(100vh-6rem)] overflow-hidden rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel)] lg:grid-cols-[17rem_minmax(0,1fr)_24rem]">
            <WorkspaceRail
              projects={projects}
              selectedProjectId={selectedProjectId}
              activeIssueCount={issues.length}
              activeRepositoryCount={repositories.length}
              busy={busy}
              onSelectProject={handleSelectProject}
              onManageWorkspace={() => setWorkspaceDialogOpen(true)}
              onRefresh={refresh}
            />
            <IssueLifecycleBoard
              project={selectedProject}
              issuesByStage={issuesByStage}
              selectedIssueId={selectedIssue?.issue_id ?? null}
              repositories={repositories}
              onCreateIssue={() => setIssueDialogOpen(true)}
              onSelectIssue={setSelectedIssueId}
            />
            <IssueWorkspaceDriver
              project={selectedProject}
              issue={selectedIssue}
              repositories={repositories}
              busy={busy}
              onAddRepository={() => setRepositoryDialogOpen(true)}
              onRunIssue={setRunIssue}
            />
          </div>
        }
      />
      {workspaceDialogOpen ? (
        <WorkspaceManagementDialog
          projects={projects}
          selectedProjectId={selectedProjectId}
          busy={busy}
          onClose={() => setWorkspaceDialogOpen(false)}
          onCreateWorkspace={handleCreateWorkspace}
        />
      ) : null}
      {issueDialogOpen ? (
        <CreateIssueDialog
          busy={busy}
          workspaceName={selectedProject?.name ?? ""}
          onClose={() => setIssueDialogOpen(false)}
          onCreateIssue={handleCreateIssue}
        />
      ) : null}
      {repositoryDialogOpen ? (
        <CreateRepositoryDialog
          busy={busy}
          workspaceName={selectedProject?.name ?? ""}
          onClose={() => setRepositoryDialogOpen(false)}
          onCreateRepository={handleCreateRepository}
        />
      ) : null}
      {runIssue ? (
        <RunIssueDialog
          issue={runIssue}
          repositories={repositories}
          busy={busy}
          onClose={() => setRunIssue(null)}
          onStart={handleStartIssue}
        />
      ) : null}
    </>
  );
}

function WorkspaceRail({
  projects,
  selectedProjectId,
  activeIssueCount,
  activeRepositoryCount,
  busy,
  onSelectProject,
  onManageWorkspace,
  onRefresh,
}: {
  projects: Project[];
  selectedProjectId: string | null;
  activeIssueCount: number;
  activeRepositoryCount: number;
  busy: boolean;
  onSelectProject: (projectId: string) => Promise<void>;
  onManageWorkspace: () => void;
  onRefresh: () => Promise<void>;
}) {
  return (
    <nav
      aria-label="Workspace 选择"
      className="flex min-h-0 flex-col border-b border-[var(--aria-line)] bg-[var(--aria-panel-muted)] lg:border-b-0 lg:border-r"
    >
      <div className="border-b border-[var(--aria-line)] px-3 py-3">
        <div className="flex items-center justify-between gap-2">
          <div>
            <h2 className="text-sm font-semibold text-[var(--aria-ink)]">Workspace</h2>
            <p className="mt-0.5 text-[11px] font-medium text-[var(--aria-ink-muted)]">
              选择一个工作空间查看 Issue
            </p>
          </div>
          <button
            type="button"
            onClick={() => void onRefresh()}
            disabled={busy}
            className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md border border-[var(--aria-line-strong)] bg-[var(--aria-panel)] text-[var(--aria-ink)] transition-colors hover:bg-white focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
            title="刷新"
          >
            <RefreshCw className="h-4 w-4" />
            <span className="sr-only">刷新</span>
          </button>
        </div>
        <button
          type="button"
          onClick={onManageWorkspace}
          className="mt-3 inline-flex h-8 w-full items-center justify-center rounded-md border border-[var(--aria-line-strong)] bg-[var(--aria-panel)] px-3 text-xs font-semibold text-[var(--aria-ink)] transition-colors hover:bg-white focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
        >
          <Settings className="mr-1.5 h-3.5 w-3.5" />
          管理 Workspace
        </button>
      </div>
      <div className="min-h-0 flex-1 overflow-auto p-2">
        {projects.length > 0 ? (
          <ul className="space-y-1.5" aria-label="Workspace 列表">
            {projects.map((project) => {
              const active = project.project_id === selectedProjectId;
              return (
                <li key={project.project_id}>
                  <button
                    type="button"
                    aria-label={`切换到 ${project.name}`}
                    aria-current={active ? "true" : undefined}
                    onClick={() => void onSelectProject(project.project_id)}
                    disabled={busy && !active}
                    className={
                      active
                        ? "w-full rounded-md border border-[var(--aria-primary)] bg-[var(--aria-panel)] px-3 py-2.5 text-left outline-none ring-2 ring-[var(--aria-primary)]"
                        : "w-full rounded-md border border-transparent px-3 py-2.5 text-left transition-colors hover:border-[var(--aria-line)] hover:bg-[var(--aria-panel)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] disabled:text-[var(--aria-ink-muted)]"
                    }
                  >
                    <span className="block truncate text-sm font-semibold text-[var(--aria-ink)]">
                      {project.name}
                    </span>
                    <span className="mt-1 block truncate text-[11px] font-medium text-[var(--aria-ink-muted)]">
                      {project.description ?? project.project_id}
                    </span>
                    <span className="mt-2 flex flex-wrap gap-1.5 text-[11px] font-semibold text-[var(--aria-ink-muted)]">
                      <span className="rounded border border-[var(--aria-line)] bg-[var(--aria-panel)] px-1.5 py-0.5">
                        {active ? `${activeIssueCount} Issue` : "Issue"}
                      </span>
                      <span className="rounded border border-[var(--aria-line)] bg-[var(--aria-panel)] px-1.5 py-0.5">
                        {active ? `${activeRepositoryCount} Repo` : "Repo"}
                      </span>
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        ) : (
          <div className="rounded-md border border-dashed border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 py-8 text-center text-sm font-medium text-[var(--aria-ink-muted)]">
            暂无 Workspace
          </div>
        )}
      </div>
    </nav>
  );
}

function IssueLifecycleBoard({
  project,
  issuesByStage,
  selectedIssueId,
  repositories,
  onCreateIssue,
  onSelectIssue,
}: {
  project: Project | null;
  issuesByStage: Record<LifecycleStageId, ProductIssue[]>;
  selectedIssueId: string | null;
  repositories: Repository[];
  onCreateIssue: () => void;
  onSelectIssue: (issueId: string) => void;
}) {
  return (
    <section
      role="region"
      aria-label="Issue 生命周期看板"
      className="min-w-0 border-b border-[var(--aria-line)] bg-[var(--aria-panel)] lg:border-b-0 lg:border-r"
    >
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-[var(--aria-line)] px-4 py-3">
        <div className="min-w-0">
          <h1 className="text-base font-semibold text-[var(--aria-ink)]">Issue 生命周期</h1>
          <p className="mt-0.5 truncate text-xs font-medium text-[var(--aria-ink-muted)]">
            {project?.name ?? "未选择 Workspace"}
          </p>
        </div>
        <button
          type="button"
          disabled={!project}
          onClick={onCreateIssue}
          className="inline-flex h-8 items-center justify-center rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 text-sm font-semibold text-white transition-colors hover:bg-cyan-700 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] disabled:border-[var(--aria-line)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
        >
          <Plus className="mr-1 h-4 w-4" />
          新建 Issue
        </button>
      </div>
      <div className="grid min-h-[calc(100vh-12rem)] gap-3 overflow-auto p-3 xl:grid-cols-4">
        {LIFECYCLE_STAGES.map((stage) => (
          <LifecycleColumn
            key={stage.id}
            stage={stage}
            issues={issuesByStage[stage.id]}
            selectedIssueId={selectedIssueId}
            repositories={repositories}
            onSelectIssue={onSelectIssue}
          />
        ))}
      </div>
    </section>
  );
}

function LifecycleColumn({
  stage,
  issues,
  selectedIssueId,
  repositories,
  onSelectIssue,
}: {
  stage: (typeof LIFECYCLE_STAGES)[number];
  issues: ProductIssue[];
  selectedIssueId: string | null;
  repositories: Repository[];
  onSelectIssue: (issueId: string) => void;
}) {
  return (
    <section
      role="region"
      aria-label={`${stage.label} 阶段`}
      className="min-h-72 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-2.5"
    >
      <div className="mb-3">
        <div className="flex items-center justify-between gap-2">
          <h2 className="text-sm font-semibold text-[var(--aria-ink)]">{stage.label}</h2>
          <span className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-2 py-0.5 font-mono text-[11px] font-semibold text-[var(--aria-ink-muted)]">
            {issues.length}
          </span>
        </div>
        <p className="mt-0.5 text-[11px] font-medium text-[var(--aria-ink-muted)]">
          {stage.title} · {stage.description}
        </p>
      </div>
      <ul className="space-y-2" aria-label={`${stage.label} Issue 卡片`}>
        {issues.map((issue) => (
          <li key={issue.issue_id}>
            <IssueCard
              issue={issue}
              selected={issue.issue_id === selectedIssueId}
              repositoryName={repoName(issue.repo_id, repositories)}
              onSelect={() => onSelectIssue(issue.issue_id)}
            />
          </li>
        ))}
      </ul>
    </section>
  );
}

function IssueCard({
  issue,
  selected,
  repositoryName,
  onSelect,
}: {
  issue: ProductIssue;
  selected: boolean;
  repositoryName: string | null;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      aria-pressed={selected}
      onClick={onSelect}
      className={
        selected
          ? "w-full rounded-md border border-[var(--aria-primary)] bg-[var(--aria-panel)] p-3 text-left outline-none ring-2 ring-[var(--aria-primary)]"
          : "w-full rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] p-3 text-left transition-colors hover:bg-white focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
      }
    >
      <span className="block truncate text-sm font-semibold text-[var(--aria-ink)]">
        {issue.title}
      </span>
      <span className="mt-1 flex flex-wrap items-center gap-1.5 font-mono text-[11px] font-medium text-[var(--aria-ink-muted)]">
        <span>{issue.issue_id}</span>
        <span className="rounded border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-1.5 py-0.5">
          {issue.status}
        </span>
      </span>
      <span className="mt-2 block text-xs font-medium leading-5 text-[var(--aria-ink-muted)]">
        {artifactSummary(issue)}
      </span>
      <span className="mt-2 block truncate text-[11px] font-medium text-[var(--aria-ink-muted)]">
        {repositoryName ?? "待选择代码库"}
      </span>
    </button>
  );
}

function IssueWorkspaceDriver({
  project,
  issue,
  repositories,
  busy,
  onAddRepository,
  onRunIssue,
}: {
  project: Project | null;
  issue: ProductIssue | null;
  repositories: Repository[];
  busy: boolean;
  onAddRepository: () => void;
  onRunIssue: (issue: ProductIssue) => void;
}) {
  const issueRepositoryName = issue ? repoName(issue.repo_id, repositories) : null;
  const canRun = Boolean(issue && issue.status !== "completed" && repositories.length > 0);
  return (
    <section
      role="region"
      aria-label="Issue 驱动 Workspace"
      className="min-w-0 bg-[var(--aria-panel)]"
    >
      <div className="border-b border-[var(--aria-line)] px-4 py-3">
        <h2 className="text-sm font-semibold text-[var(--aria-ink)]">Issue 驱动 Workspace</h2>
        <p className="mt-0.5 truncate text-xs font-medium text-[var(--aria-ink-muted)]">
          {project?.name ?? "未选择 Workspace"}
        </p>
      </div>
      <div className="grid gap-4 p-4">
        {issue ? (
          <div className="grid gap-3">
            <div className="min-w-0">
              <h3 className="truncate text-base font-semibold text-[var(--aria-ink)]">
                {issue.title}
              </h3>
              <div className="mt-1 flex flex-wrap gap-2 font-mono text-[11px] font-medium text-[var(--aria-ink-muted)]">
                <span>{issue.issue_id}</span>
                <span>{issue.phase}</span>
                <span>{issue.status}</span>
              </div>
            </div>
            <LifecycleRail issue={issue} />
            <button
              type="button"
              disabled={busy || !canRun}
              onClick={() => onRunIssue(issue)}
              className="inline-flex h-9 items-center justify-center rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 text-sm font-semibold text-white transition-colors hover:bg-cyan-700 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] disabled:border-[var(--aria-line)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
            >
              <Play className="mr-1.5 h-4 w-4" />
              运行 Issue
            </button>
            <ArtifactBlock
              title="Story Spec"
              icon={<FileText className="h-4 w-4" />}
              body={issue.description ?? `${issue.title} 的用户故事和成功标准待补齐。`}
              meta={issue.phase === "clarification" ? "当前需求澄清产物" : "已进入后续阶段"}
            />
            <ArtifactBlock
              title="Design Spec"
              icon={<Layers3 className="h-4 w-4" />}
              body={`围绕 ${issue.change_id} 展示数据模型、接口契约、共享组件与风险约束。`}
              meta={issue.status === "draft" ? "等待 story spec 确认后生成" : "设计产物可审阅"}
            />
            <ArtifactBlock
              title="Work Item"
              icon={<GitBranch className="h-4 w-4" />}
              body={
                issue.repo_id
                  ? `绑定 ${issueRepositoryName ?? issue.repo_id}，可进入计划、编码、测试和 review。`
                  : "运行前需要从当前 Workspace 的代码库中选择唯一 repo。"
              }
              meta={issue.active_binding_id ?? "暂无 active binding"}
            />
          </div>
        ) : (
          <div className="flex min-h-44 items-center justify-center rounded-md border border-dashed border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-4 text-center text-sm font-medium text-[var(--aria-ink-muted)]">
            请选择一个 Issue
          </div>
        )}
        <section className="grid gap-3 border-t border-[var(--aria-line)] pt-4">
          <div className="flex items-center justify-between gap-3">
            <div>
              <h3 className="text-sm font-semibold text-[var(--aria-ink)]">Workspace 代码库</h3>
              <p className="mt-0.5 text-xs font-medium text-[var(--aria-ink-muted)]">
                Issue 运行时只从当前 Workspace 选择
              </p>
            </div>
            <button
              type="button"
              onClick={onAddRepository}
              disabled={!project}
              className="inline-flex h-8 shrink-0 items-center justify-center rounded-md border border-[var(--aria-line-strong)] bg-[var(--aria-panel)] px-3 text-xs font-semibold text-[var(--aria-ink)] transition-colors hover:bg-[var(--aria-panel-muted)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
            >
              <Plus className="mr-1 h-3.5 w-3.5" />
              添加代码库
            </button>
          </div>
          {repositories.length > 0 ? (
            <ul className="space-y-2" aria-label="Workspace 代码库列表">
              {repositories.map((repository) => (
                <li
                  key={repository.repository_id}
                  className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 py-2"
                >
                  <div className="flex items-center justify-between gap-2">
                    <span className="truncate text-sm font-semibold text-[var(--aria-ink)]">
                      {repository.name}
                    </span>
                    <span className="font-mono text-[11px] font-medium text-[var(--aria-ink-muted)]">
                      {repository.repository_id}
                    </span>
                  </div>
                  <p className="mt-1 truncate font-mono text-[11px] font-medium text-[var(--aria-ink-muted)]">
                    {repository.path}
                  </p>
                </li>
              ))}
            </ul>
          ) : (
            <div className="rounded-md border border-dashed border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 py-6 text-center text-sm font-medium text-[var(--aria-ink-muted)]">
              当前 Workspace 还没有代码库
            </div>
          )}
        </section>
      </div>
    </section>
  );
}

function LifecycleRail({ issue }: { issue: ProductIssue }) {
  const activeStage = stageForIssue(issue);
  const activeIndex = LIFECYCLE_STAGES.findIndex((stage) => stage.id === activeStage);
  return (
    <nav aria-label="Issue 生命周期轨道" className="rounded-md border border-[var(--aria-line)]">
      <ol className="grid gap-0 sm:grid-cols-4">
        {LIFECYCLE_STAGES.map((stage, index) => {
          const active = stage.id === activeStage;
          const completed = index < activeIndex || issue.status === "completed";
          return (
            <li
              key={stage.id}
              aria-current={active ? "step" : undefined}
              className={
                active
                  ? "flex items-center gap-2 border-b border-[var(--aria-line)] bg-[var(--aria-primary-soft)] px-3 py-2 text-sm font-semibold text-[var(--aria-ink)] sm:border-b-0 sm:border-r"
                  : "flex items-center gap-2 border-b border-[var(--aria-line)] px-3 py-2 text-sm font-medium text-[var(--aria-ink-muted)] last:border-b-0 sm:border-b-0 sm:border-r sm:last:border-r-0"
              }
            >
              {completed ? (
                <CheckCircle2 className="h-4 w-4 text-emerald-600" />
              ) : (
                <span className="h-2 w-2 rounded-full bg-[var(--aria-line-strong)]" />
              )}
              {stage.label}
            </li>
          );
        })}
      </ol>
    </nav>
  );
}

function ArtifactBlock({
  title,
  icon,
  body,
  meta,
}: {
  title: string;
  icon: ReactNode;
  body: string;
  meta: string;
}) {
  return (
    <section className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-3">
      <div className="mb-2 flex items-center justify-between gap-2">
        <h3 className="flex items-center gap-2 text-sm font-semibold text-[var(--aria-ink)]">
          {icon}
          {title}
        </h3>
        <span className="rounded border border-[var(--aria-line)] bg-[var(--aria-panel)] px-2 py-0.5 text-[11px] font-semibold text-[var(--aria-ink-muted)]">
          spec 产物
        </span>
      </div>
      <p className="text-sm font-medium leading-6 text-[var(--aria-ink-muted)]">{body}</p>
      <p className="mt-2 font-mono text-[11px] font-medium text-[var(--aria-ink-muted)]">{meta}</p>
    </section>
  );
}

function WorkspaceManagementDialog({
  projects,
  selectedProjectId,
  busy,
  onClose,
  onCreateWorkspace,
}: {
  projects: Project[];
  selectedProjectId: string | null;
  busy: boolean;
  onClose: () => void;
  onCreateWorkspace: (payload: { name: string; description: string | null }) => Promise<void>;
}) {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  return (
    <DialogFrame title="Workspace 管理" onClose={onClose}>
      <div className="grid gap-4">
        <ul aria-label="Workspace 列表" className="max-h-48 space-y-2 overflow-auto">
          {projects.map((project) => (
            <li
              key={project.project_id}
              className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 py-2"
            >
              <div className="flex items-center justify-between gap-2">
                <span className="text-sm font-semibold text-[var(--aria-ink)]">{project.name}</span>
                {project.project_id === selectedProjectId ? (
                  <span className="text-xs font-semibold text-[var(--aria-primary)]">active</span>
                ) : null}
              </div>
              <p className="font-mono text-[11px] text-[var(--aria-ink-muted)]">
                {project.project_id}
              </p>
            </li>
          ))}
        </ul>
        <form
          className="grid gap-3 border-t border-[var(--aria-line)] pt-3"
          onSubmit={(event) => {
            event.preventDefault();
            if (name.trim()) {
              void onCreateWorkspace({
                name: name.trim(),
                description: description.trim() || null,
              });
            }
          }}
        >
          <label className="grid gap-1 text-xs font-semibold text-[var(--aria-ink-muted)]">
            Workspace 名称
            <input
              value={name}
              onChange={(event) => setName(event.target.value)}
              className="h-9 rounded-md border border-[var(--aria-line-strong)] bg-[var(--aria-panel)] px-3 text-sm text-[var(--aria-ink)] outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
            />
          </label>
          <label className="grid gap-1 text-xs font-semibold text-[var(--aria-ink-muted)]">
            Workspace 描述
            <textarea
              rows={2}
              value={description}
              onChange={(event) => setDescription(event.target.value)}
              className="rounded-md border border-[var(--aria-line-strong)] bg-[var(--aria-panel)] px-3 py-2 text-sm text-[var(--aria-ink)] outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
            />
          </label>
          <button
            type="submit"
            disabled={busy || !name.trim()}
            className="inline-flex h-9 items-center justify-center justify-self-start rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 text-sm font-semibold text-white disabled:border-[var(--aria-line)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
          >
            创建 Workspace
          </button>
        </form>
      </div>
    </DialogFrame>
  );
}

function CreateIssueDialog({
  workspaceName,
  busy,
  onClose,
  onCreateIssue,
}: {
  workspaceName: string;
  busy: boolean;
  onClose: () => void;
  onCreateIssue: (payload: { title: string; description: string | null }) => Promise<void>;
}) {
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  return (
    <DialogFrame title="新建 Issue" onClose={onClose}>
      <form
        className="grid gap-3"
        onSubmit={(event) => {
          event.preventDefault();
          if (title.trim()) {
            void onCreateIssue({
              title: title.trim(),
              description: description.trim() || null,
            });
          }
        }}
      >
        <p className="text-sm font-medium text-[var(--aria-ink-muted)]">
          创建到激活 Workspace：{workspaceName}
        </p>
        <label className="grid gap-1 text-xs font-semibold text-[var(--aria-ink-muted)]">
          Issue 标题
          <input
            aria-label="Issue 标题"
            value={title}
            onChange={(event) => setTitle(event.target.value)}
            className="h-9 rounded-md border border-[var(--aria-line-strong)] bg-[var(--aria-panel)] px-3 text-sm text-[var(--aria-ink)] outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          />
        </label>
        <label className="grid gap-1 text-xs font-semibold text-[var(--aria-ink-muted)]">
          Issue 描述
          <textarea
            aria-label="Issue 描述"
            rows={4}
            value={description}
            onChange={(event) => setDescription(event.target.value)}
            className="rounded-md border border-[var(--aria-line-strong)] bg-[var(--aria-panel)] px-3 py-2 text-sm text-[var(--aria-ink)] outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          />
        </label>
        <button
          type="submit"
          disabled={busy || !title.trim()}
          className="inline-flex h-9 items-center justify-center justify-self-start rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 text-sm font-semibold text-white disabled:border-[var(--aria-line)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
        >
          创建
        </button>
      </form>
    </DialogFrame>
  );
}

function CreateRepositoryDialog({
  workspaceName,
  busy,
  onClose,
  onCreateRepository,
}: {
  workspaceName: string;
  busy: boolean;
  onClose: () => void;
  onCreateRepository: (payload: { name: string; path: string }) => Promise<void>;
}) {
  const [name, setName] = useState("");
  const [path, setPath] = useState("");
  return (
    <DialogFrame title="添加代码库" onClose={onClose}>
      <form
        className="grid gap-3"
        onSubmit={(event) => {
          event.preventDefault();
          if (name.trim() && path.trim()) {
            void onCreateRepository({ name: name.trim(), path: path.trim() });
          }
        }}
      >
        <p className="text-sm font-medium text-[var(--aria-ink-muted)]">
          添加到 Workspace：{workspaceName}
        </p>
        <label className="grid gap-1 text-xs font-semibold text-[var(--aria-ink-muted)]">
          代码库名称
          <input
            aria-label="代码库名称"
            value={name}
            onChange={(event) => setName(event.target.value)}
            className="h-9 rounded-md border border-[var(--aria-line-strong)] bg-[var(--aria-panel)] px-3 text-sm text-[var(--aria-ink)] outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          />
        </label>
        <label className="grid gap-1 text-xs font-semibold text-[var(--aria-ink-muted)]">
          代码库路径
          <input
            aria-label="代码库路径"
            value={path}
            onChange={(event) => setPath(event.target.value)}
            className="h-9 rounded-md border border-[var(--aria-line-strong)] bg-[var(--aria-panel)] px-3 text-sm text-[var(--aria-ink)] outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          />
        </label>
        <button
          type="submit"
          disabled={busy || !name.trim() || !path.trim()}
          className="inline-flex h-9 items-center justify-center justify-self-start rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 text-sm font-semibold text-white disabled:border-[var(--aria-line)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
        >
          添加
        </button>
      </form>
    </DialogFrame>
  );
}

function RunIssueDialog({
  issue,
  repositories,
  busy,
  onClose,
  onStart,
}: {
  issue: ProductIssue;
  repositories: Repository[];
  busy: boolean;
  onClose: () => void;
  onStart: (repositoryId: string) => Promise<void>;
}) {
  const [repositoryId, setRepositoryId] = useState(
    issue.repo_id ?? repositories[0]?.repository_id ?? "",
  );
  return (
    <DialogFrame title="运行 Issue" onClose={onClose}>
      <form
        className="grid gap-3"
        onSubmit={(event) => {
          event.preventDefault();
          if (repositoryId) {
            void onStart(repositoryId);
          }
        }}
      >
        <p className="text-sm font-medium text-[var(--aria-ink-muted)]">{issue.title}</p>
        <label className="grid gap-1 text-xs font-semibold text-[var(--aria-ink-muted)]">
          运行代码库
          <select
            aria-label="运行代码库"
            value={repositoryId}
            onChange={(event) => setRepositoryId(event.target.value)}
            className="h-9 rounded-md border border-[var(--aria-line-strong)] bg-[var(--aria-panel)] px-3 text-sm text-[var(--aria-ink)] outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          >
            {repositories.map((repository) => (
              <option key={repository.repository_id} value={repository.repository_id}>
                {repository.name} · {repository.repository_id}
              </option>
            ))}
          </select>
        </label>
        <button
          type="submit"
          disabled={busy || !repositoryId}
          className="inline-flex h-9 items-center justify-center justify-self-start rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 text-sm font-semibold text-white disabled:border-[var(--aria-line)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
        >
          开始运行
        </button>
      </form>
    </DialogFrame>
  );
}

function DialogFrame({
  title,
  children,
  onClose,
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
}) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/30 px-4 py-6">
      <section
        role="dialog"
        aria-modal="true"
        aria-label={title}
        className="w-full max-w-lg rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel)] p-4 shadow-xl"
      >
        <div className="mb-4 flex items-center justify-between gap-3">
          <h2 className="text-base font-semibold text-[var(--aria-ink)]">{title}</h2>
          <button
            type="button"
            onClick={onClose}
            className="inline-flex h-8 items-center justify-center rounded-md border border-[var(--aria-line-strong)] px-3 text-sm font-semibold text-[var(--aria-ink)] hover:bg-[var(--aria-panel-muted)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          >
            关闭
          </button>
        </div>
        {children}
      </section>
    </div>
  );
}

function groupIssuesByStage(issues: ProductIssue[]): Record<LifecycleStageId, ProductIssue[]> {
  return {
    story_spec: issues.filter((issue) => stageForIssue(issue) === "story_spec"),
    design_spec: issues.filter((issue) => stageForIssue(issue) === "design_spec"),
    work_item: issues.filter((issue) => stageForIssue(issue) === "work_item"),
    done: issues.filter((issue) => stageForIssue(issue) === "done"),
  };
}

function stageForIssue(issue: ProductIssue): LifecycleStageId {
  if (issue.status === "completed") {
    return "done";
  }
  if (issue.phase === "development") {
    return "work_item";
  }
  if (issue.status === "draft") {
    return "story_spec";
  }
  return "design_spec";
}

function artifactSummary(issue: ProductIssue) {
  const stage = stageForIssue(issue);
  if (stage === "story_spec") {
    return "Story Spec 待确认";
  }
  if (stage === "design_spec") {
    return "Design Spec 审阅中";
  }
  if (stage === "work_item") {
    return "Work Item 执行中";
  }
  return "代码开发已完成";
}

function repoName(repoId: string | null, repositories: Repository[]) {
  if (!repoId) {
    return null;
  }
  return repositories.find((repository) => repository.repository_id === repoId)?.name ?? repoId;
}
