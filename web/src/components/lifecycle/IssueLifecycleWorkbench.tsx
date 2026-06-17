import { PanelRightOpen, Plus, RefreshCw } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import {
  createCodingAttempt,
  createProject,
  createProductIssue,
  createRepository,
  deleteDesignSpec,
  deleteProductIssue,
  deleteProject,
  deleteRepository,
  deleteStorySpec,
  deleteWorkItem,
  generateDesignSpecs,
  generateStorySpecs,
  getIssueLifecycle,
  prepareWorkItemPlan,
  listProductIssues,
  listProjects,
  listRepositories,
} from "../../api/client";
import type {
  IssueLifecycleResponse,
  LifecycleWorkItem,
  ProductIssue,
  Project,
  Repository,
  CreateRepositoryRequest,
  WorkspaceSession,
} from "../../api/types";
import {
  groupLifecycleCards,
  useLifecycleWorkbenchStore,
  type LifecycleCard as LifecycleCardData,
  type LifecycleColumns,
} from "../../state/lifecycle-workbench-store";
import { WorkbenchSurface } from "../shell/WorkbenchSurface";
import {
  CreateProjectDialog,
  type CreateProjectPayload,
} from "./CreateProjectDialog";
import { CreateRepositoryDialog } from "./CreateRepositoryDialog";
import {
  CreateLifecycleIssueDialog,
  type CreateLifecycleIssuePayload,
} from "./CreateLifecycleIssueDialog";
import { LifecycleCard } from "./LifecycleCard";
import { LifecycleCardDrawer, type DrawerEntity } from "./LifecycleCardDrawer";
import { ProjectSidebar } from "./ProjectSidebar";
type ProviderWorkspaceLaunchTarget = "story" | "design" | "work_item";
const DELETE_EXIT_ANIMATION_MS = 220;

export function IssueLifecycleWorkbench({
  focusEntityId,
  onDrawerFocusChange,
  onOpenWorkspace = defaultOpenWorkspace,
  onOpenCodingWorkspace = defaultOpenCodingWorkspace,
}: {
  focusEntityId?: string | null;
  onDrawerFocusChange?: (entityId: string | null) => void;
  onOpenWorkspace?: (sessionId: string) => void;
  onOpenCodingWorkspace?: (attemptId: string) => void;
}) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [repositories, setRepositories] = useState<Repository[]>([]);
  const [lifecycles, setLifecycles] = useState<IssueLifecycleResponse[]>([]);
  const [selectedProjectId, setSelectedProjectId] = useState<string | null>(
    null,
  );
  const [focusedIssueId, setFocusedIssueId] = useState<string | null>(null);
  const [selectedCardKey, setSelectedCardKey] = useState<string | null>(null);
  const [deletingCardKey, setDeletingCardKey] = useState<string | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [projectDialogOpen, setProjectDialogOpen] = useState(false);
  const [repositoryDialogOpen, setRepositoryDialogOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const refreshRequestId = useRef(0);
  const drawerFocusedEntityId = useLifecycleWorkbenchStore(
    (state) => state.focusedEntityId,
  );
  const isDrawerOpen = useLifecycleWorkbenchStore(
    (state) => state.isDrawerOpen,
  );
  const openDrawer = useLifecycleWorkbenchStore((state) => state.openDrawer);
  const closeDrawer = useLifecycleWorkbenchStore((state) => state.closeDrawer);

  useEffect(() => {
    void refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (focusEntityId === undefined) {
      return;
    }
    if (focusEntityId) {
      openDrawer(focusEntityId);
      return;
    }
    closeDrawer();
  }, [closeDrawer, focusEntityId, openDrawer]);

  useEffect(() => {
    if (!onDrawerFocusChange) {
      return;
    }
    onDrawerFocusChange(isDrawerOpen ? drawerFocusedEntityId : null);
  }, [drawerFocusedEntityId, isDrawerOpen, onDrawerFocusChange]);

  async function refresh(projectIdOverride?: string | null) {
    const requestId = refreshRequestId.current + 1;
    refreshRequestId.current = requestId;

    setBusy(true);
    setError(null);
    try {
      const projectResponse = await listProjects();
      if (!isLatestRefresh(requestId)) {
        return;
      }

      const projectId =
        projectIdOverride ??
        (selectedProjectId &&
        projectResponse.projects.some(
          (project) => project.project_id === selectedProjectId,
        )
          ? selectedProjectId
          : projectResponse.projects[0]?.project_id) ??
        null;
      const projectChanged = projectId !== selectedProjectId;
      setProjects(projectResponse.projects);
      setSelectedProjectId(projectId);

      if (!projectId) {
        setRepositories([]);
        setLifecycles([]);
        setFocusedIssueId(null);
        setSelectedCardKey(null);
        return;
      }

      const [repositoryResponse, issueResponse] = await Promise.all([
        listRepositories(projectId),
        listProductIssues(projectId),
      ]);
      if (!isLatestRefresh(requestId)) {
        return;
      }

      const lifecycleResponses = await Promise.all(
        (issueResponse.issues ?? []).map(async (issue) =>
          normalizeLifecycleResponse(
            await getIssueLifecycle(issue.issue_id, projectId),
            issue,
          ),
        ),
      );
      if (!isLatestRefresh(requestId)) {
        return;
      }

      setRepositories(repositoryResponse.repositories ?? []);
      setLifecycles(lifecycleResponses);
      setFocusedIssueId(
        focusedIssueId &&
          lifecycleResponses.some(
            (lifecycle) => lifecycle.issue.issue_id === focusedIssueId,
          )
          ? focusedIssueId
          : lifecycleResponses[0]?.issue.issue_id ?? null,
      );
      if (projectChanged) {
        setSelectedCardKey(null);
      }
    } catch (reason) {
      if (isLatestRefresh(requestId)) {
        setError(
          reason instanceof Error
            ? reason.message
            : "load lifecycle workbench failed",
        );
      }
    } finally {
      if (isLatestRefresh(requestId)) {
        setBusy(false);
      }
    }
  }

  function isLatestRefresh(requestId: number) {
    return requestId === refreshRequestId.current;
  }

  const allColumns = useMemo(
    () => groupLifecycleCards(lifecycles),
    [lifecycles],
  );
  const selectedIssueColumns = useMemo(
    () => selectedLifecycleColumns(allColumns, focusedIssueId),
    [allColumns, focusedIssueId],
  );
  const focusedEntity = useMemo(
    () => findCardInColumns(allColumns, drawerFocusedEntityId),
    [allColumns, drawerFocusedEntityId],
  );
  const selectedProject = projects.find(
    (project) => project.project_id === selectedProjectId,
  );
  const issueCount = allColumns.issue.length;

  async function handleSelectProject(projectId: string) {
    if (projectId === selectedProjectId) {
      return;
    }
    setSelectedProjectId(projectId);
    await refresh(projectId);
  }

  function handleSelectCard(card: LifecycleCardData) {
    setSelectedCardKey(lifecycleCardKey(card));
    if (card.kind === "issue") {
      setFocusedIssueId(card.issueId);
      closeDrawer();
      return;
    }
    openDrawer(card.id);
  }

  function handleOpenFullIssue(card: LifecycleCardData) {
    setSelectedCardKey(lifecycleCardKey(card));
    setFocusedIssueId(card.issueId);
    openDrawer(card.id);
  }

  async function handleOpenWorkspaceFromDrawer(card: LifecycleCardData) {
    const session = findWorkspaceSession(lifecycles, card);
    if (!session) {
      setError("缺少 Workspace Session");
      return;
    }
    setError(null);
    await refresh(selectedProjectId);
    onOpenWorkspace(session.workspace_session_id);
  }

  async function handleOpenCodingWorkspaceFromDrawer(card: LifecycleCardData) {
    if (!selectedProjectId || card.kind !== "work_item") {
      setError("缺少 Project 或 Work Item");
      return;
    }

    if (card.raw.latest_attempt) {
      setError(null);
      onOpenCodingWorkspace(card.raw.latest_attempt.attempt_id);
      return;
    }

    setError(null);
    const attempt = await createCodingAttempt(
      selectedProjectId,
      card.issueId,
      card.id,
    );
    await refresh(selectedProjectId);
    onOpenCodingWorkspace(attempt.attempt_id);
  }

  async function handleGenerateNext(card: LifecycleCardData) {
    if (!selectedProjectId) {
      setError("缺少 Project 或生命周期实体");
      return;
    }

    if (card.kind === "story_spec") {
      const response = await generateDesignSpecs(
        selectedProjectId,
        card.issueId,
        {
          title: defaultLaunchTitle({ target: "design", card }),
          story_spec_ids: [card.id],
          design_kind: "frontend",
        },
      );
      const nextId = response.design_specs[0]?.design_spec_id;
      await refresh(selectedProjectId);
      if (nextId) {
        setSelectedCardKey(`design_spec:${nextId}`);
        openDrawer(nextId);
      }
      return;
    }

    if (card.kind === "design_spec") {
      const response = await prepareWorkItemPlan(
        selectedProjectId,
        card.issueId,
        {
          title: defaultLaunchTitle({ target: "work_item", card }),
          story_spec_ids: card.raw.story_spec_ids,
          design_spec_ids: [card.id],
          include_integration_tests: true,
          include_e2e_tests: false,
          force_frontend_backend_split: true,
          require_execution_plan_confirm: false,
        },
      );
      await refresh(selectedProjectId);
      onOpenWorkspace(response.workspace_session.workspace_session_id);
      return;
    }

    setError("当前实体不支持生成下一阶段");
  }

  async function handleCreateIssue(payload: CreateLifecycleIssuePayload) {
    if (!selectedProjectId) {
      setError("缺少 Project");
      return;
    }

    await createProductIssue(selectedProjectId, {
      title: payload.title,
      description: payload.description,
      change_id: null,
      repository_id: payload.repository_id,
    });
    setDialogOpen(false);
    await refresh();
  }

  async function handleCreateProject(payload: CreateProjectPayload) {
    const project = await createProject(payload);
    setProjectDialogOpen(false);
    await refresh(project.project_id);
  }

  async function handleCreateRepository(payload: CreateRepositoryRequest) {
    if (!selectedProjectId) {
      setError("缺少 Project");
      return;
    }

    await createRepository(selectedProjectId, payload);
    setRepositoryDialogOpen(false);
    await refresh(selectedProjectId);
  }

  async function handleDeleteProject(projectId: string) {
    setError(null);
    await deleteProject(projectId);
    if (projectId === selectedProjectId) {
      setSelectedProjectId(null);
      setFocusedIssueId(null);
      setSelectedCardKey(null);
    }
    await refresh(null);
  }

  async function handleDeleteRepository(repositoryId: string) {
    if (!selectedProjectId) {
      setError("缺少 Project");
      return;
    }

    setError(null);
    await deleteRepository(selectedProjectId, repositoryId);
    setSelectedCardKey(null);
    await refresh(selectedProjectId);
  }

  async function handleDeleteIssue(issueId: string) {
    if (!selectedProjectId) {
      setError("缺少 Project");
      return;
    }

    const cardKey = `issue:${issueId}`;
    setDeletingCardKey(cardKey);
    setError(null);
    try {
      await Promise.all([
        deleteProductIssue(selectedProjectId, issueId),
        waitForDeleteExitAnimation(),
      ]);
      if (focusedIssueId === issueId) {
        setFocusedIssueId(null);
      }
      setSelectedCardKey(null);
      await refresh(selectedProjectId);
    } catch (reason) {
      setError(errorMessage(reason, "删除 Issue 失败"));
    } finally {
      setDeletingCardKey(null);
    }
  }

  async function handleDeleteLifecycleCard(card: LifecycleCardData) {
    if (!selectedProjectId) {
      setError("缺少 Project");
      return;
    }

    let deleteRequest: Promise<{ status: string }>;
    if (card.kind === "story_spec") {
      deleteRequest = deleteStorySpec(selectedProjectId, card.issueId, card.id);
    } else if (card.kind === "design_spec") {
      deleteRequest = deleteDesignSpec(
        selectedProjectId,
        card.issueId,
        card.id,
      );
    } else if (card.kind === "work_item") {
      deleteRequest = deleteWorkItem(selectedProjectId, card.issueId, card.id);
    } else {
      setError("Issue 请从 Issue 卡片列表删除");
      return;
    }

    const cardKey = lifecycleCardKey(card);
    setDeletingCardKey(cardKey);
    setError(null);
    try {
      await Promise.all([deleteRequest, waitForDeleteExitAnimation()]);
      if (selectedCardKey === cardKey) {
        setSelectedCardKey(null);
      }
      if (drawerFocusedEntityId === card.id) {
        closeDrawer();
      }
      await refresh(selectedProjectId);
    } catch (reason) {
      setError(errorMessage(reason, "删除生命周期实体失败"));
    } finally {
      setDeletingCardKey(null);
    }
  }

  function handleDeleteWorkItemFromDrawer(card: LifecycleCardData) {
    if (card.kind !== "work_item") {
      return;
    }
    const confirmed = window.confirm(
      "删除 Work Item 会同时删除关联的 Coding Workspace、日志和 worktree，且无法撤销。",
    );
    if (!confirmed) {
      return;
    }
    void handleDeleteLifecycleCard(card);
  }

  async function handleLaunchWorkspace(
    target: ProviderWorkspaceLaunchTarget,
    card: LifecycleCardData,
  ) {
    if (!selectedProjectId) {
      setError("缺少 Project 或生命周期卡片");
      return;
    }

    if (target === "story") {
      const response = await generateStorySpecs(
        selectedProjectId,
        card.issueId,
        {
          title: defaultLaunchTitle({ target, card }),
        },
      );
      setSelectedCardKey(
        `story_spec:${response.story_specs[0]?.story_spec_id ?? ""}`,
      );
      await refresh(selectedProjectId);
      if (response.workspace_session) {
        onOpenWorkspace(response.workspace_session.workspace_session_id);
      }
      return;
    }

    if (target === "design" && card.kind === "story_spec") {
      const response = await generateDesignSpecs(
        selectedProjectId,
        card.issueId,
        {
          title: defaultLaunchTitle({ target, card }),
          story_spec_ids: [card.id],
          design_kind: "frontend",
        },
      );
      setSelectedCardKey(
        `design_spec:${response.design_specs[0]?.design_spec_id ?? ""}`,
      );
      await refresh(selectedProjectId);
      if (response.workspace_session) {
        onOpenWorkspace(response.workspace_session.workspace_session_id);
      }
      return;
    }

    if (target === "work_item" && card.kind === "design_spec") {
      const response = await prepareWorkItemPlan(
        selectedProjectId,
        card.issueId,
        {
          title: defaultLaunchTitle({ target, card }),
          story_spec_ids: card.raw.story_spec_ids,
          design_spec_ids: [card.id],
          include_integration_tests: true,
          include_e2e_tests: false,
          force_frontend_backend_split: true,
          require_execution_plan_confirm: false,
        },
      );
      await refresh(selectedProjectId);
      onOpenWorkspace(response.workspace_session.workspace_session_id);
      return;
    }

    setError("当前卡片不能启动该 Workspace");
  }

  return (
    <>
      <div className="grid min-h-screen bg-[var(--aria-bg)] text-[var(--aria-ink)] lg:grid-cols-[17rem_minmax(0,1fr)]">
        <ProjectSidebar
          projects={projects}
          repositories={repositories}
          selectedProjectId={selectedProjectId}
          issueCount={issueCount}
          busy={busy}
          onSelectProject={(projectId) => void handleSelectProject(projectId)}
          onCreateProject={() => setProjectDialogOpen(true)}
          onCreateRepository={() => setRepositoryDialogOpen(true)}
          onDeleteProject={(projectId) => void handleDeleteProject(projectId)}
          onDeleteRepository={(repositoryId) =>
            void handleDeleteRepository(repositoryId)
          }
        />
        <WorkbenchSurface
          mainLabel="Issue 生命周期工作台"
          statusBar={
            busy ? (
              <span className="text-xs font-semibold text-[var(--aria-ink-muted)]">
                加载中
              </span>
            ) : null
          }
          alert={error}
          header={
            <div className="flex min-w-0 flex-wrap items-center justify-between gap-3">
              <div className="min-w-0">
                <h1 className="truncate text-base font-semibold text-[var(--aria-ink)]">
                  Issue 生命周期工作台
                </h1>
                <p className="truncate text-xs text-[var(--aria-ink-muted)]">
                  {selectedProject?.name ?? "未选择 Project"}
                </p>
              </div>
              <div className="flex flex-wrap items-center gap-2">
                {focusedIssueId ? (
                  <button
                    type="button"
                    onClick={() => setFocusedIssueId(null)}
                    className="inline-flex h-8 items-center rounded-md border border-[var(--aria-line)] px-3 text-xs font-semibold text-[var(--aria-ink)]"
                  >
                    显示全部
                  </button>
                ) : null}
                <button
                  type="button"
                  aria-label="刷新"
                  onClick={() => void refresh()}
                  className="inline-flex h-8 w-8 items-center justify-center rounded-md border border-[var(--aria-line)] text-[var(--aria-ink-muted)]"
                >
                  <RefreshCw className="h-4 w-4" />
                </button>
                <button
                  type="button"
                  disabled={!selectedProjectId || repositories.length === 0}
                  onClick={() => setDialogOpen(true)}
                  className="inline-flex h-8 items-center rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 text-xs font-semibold text-white disabled:border-[var(--aria-line)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
                >
                  <Plus className="mr-1 h-4 w-4" />
                  新建 Issue
                </button>
              </div>
            </div>
          }
          main={
            <div className="grid min-h-[calc(100vh-6rem)] gap-3 lg:grid-cols-[minmax(18rem,24rem)_minmax(0,1fr)]">
              <IssueCardList
                cards={allColumns.issue}
                selectedKey={selectedCardKey}
                onSelect={handleSelectCard}
                onGenerateStorySpec={(card) =>
                  void handleLaunchWorkspace("story", card)
                }
                onDeleteIssue={(issueId) => void handleDeleteIssue(issueId)}
                deletingKey={deletingCardKey}
              />
              <IssueLifecycleDetail
                issue={selectedIssueColumns.issue[0] ?? null}
                storySpecs={selectedIssueColumns.story_spec}
                designSpecs={selectedIssueColumns.design_spec}
                workItems={selectedIssueColumns.work_item}
                selectedKey={selectedCardKey}
                onSelect={handleSelectCard}
                onOpenFullIssue={handleOpenFullIssue}
                onDelete={handleDeleteLifecycleCard}
                deletingKey={deletingCardKey}
              />
            </div>
          }
        />
      </div>
      {isDrawerOpen && focusedEntity ? (
        <div className="fixed right-0 top-0 z-50 h-full w-[min(480px,100vw)] shadow-xl">
          <LifecycleCardDrawer
            entity={toDrawerEntity(
              focusedEntity,
              lifecycles.find(
                (lifecycle) =>
                  lifecycle.issue.issue_id === focusedEntity.issueId,
              )?.work_items ?? [],
            )}
            onClose={closeDrawer}
            onOpenWorkspace={() =>
              void handleOpenWorkspaceFromDrawer(focusedEntity)
            }
            onOpenCodingWorkspace={
              focusedEntity.kind === "work_item" &&
              focusedEntity.raw.plan_status === "confirmed"
                ? () => void handleOpenCodingWorkspaceFromDrawer(focusedEntity)
                : undefined
            }
            onGenerateNext={
              focusedEntity.status === "confirmed" &&
              (focusedEntity.kind === "story_spec" ||
                focusedEntity.kind === "design_spec")
                ? () => void handleGenerateNext(focusedEntity)
                : undefined
            }
            onDelete={
              focusedEntity.kind === "work_item"
                ? () => handleDeleteWorkItemFromDrawer(focusedEntity)
                : undefined
            }
          />
        </div>
      ) : null}
      {projectDialogOpen ? (
        <CreateProjectDialog
          onCreate={handleCreateProject}
          onClose={() => setProjectDialogOpen(false)}
        />
      ) : null}
      {repositoryDialogOpen ? (
        <CreateRepositoryDialog
          onCreate={handleCreateRepository}
          onClose={() => setRepositoryDialogOpen(false)}
        />
      ) : null}
      {dialogOpen ? (
        <CreateLifecycleIssueDialog
          repositories={repositories}
          onCreate={handleCreateIssue}
          onClose={() => setDialogOpen(false)}
        />
      ) : null}
    </>
  );
}

function IssueCardList({
  cards,
  selectedKey,
  deletingKey,
  onSelect,
  onGenerateStorySpec,
  onDeleteIssue,
}: {
  cards: LifecycleCardData[];
  selectedKey: string | null;
  deletingKey: string | null;
  onSelect: (card: LifecycleCardData) => void;
  onGenerateStorySpec: (card: LifecycleCardData) => void;
  onDeleteIssue: (issueId: string) => void;
}) {
  return (
    <section
      role="region"
      aria-label="Issue 卡片列表"
      className="min-h-0 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-3"
    >
      <div className="mb-3 flex items-center justify-between gap-2">
        <div>
          <h2 className="text-sm font-semibold text-[var(--aria-ink)]">
            Issues
          </h2>
          <p className="mt-0.5 text-xs text-[var(--aria-ink-muted)]">
            选择 Issue 后查看它的 Story、Design 和 Work Item。
          </p>
        </div>
        <span className="rounded border border-[var(--aria-line)] bg-[var(--aria-panel)] px-2 py-0.5 font-mono text-[11px] text-[var(--aria-ink-muted)]">
          {cards.length}
        </span>
      </div>
      {cards.length === 0 ? (
        <div className="rounded-md border border-dashed border-[var(--aria-line)] bg-[var(--aria-panel)] p-4 text-sm text-[var(--aria-ink-muted)]">
          当前 Project 还没有 Issue。
        </div>
      ) : (
        <ul className="space-y-2">
          {cards.map((card) => (
            <li key={`${card.kind}:${card.id}`}>
              <LifecycleCard
                card={card}
                selected={selectedKey === lifecycleCardKey(card)}
                deleting={deletingKey === lifecycleCardKey(card)}
                onSelect={() => onSelect(card)}
                onGenerateStorySpec={() => onGenerateStorySpec(card)}
                onDelete={() => onDeleteIssue(card.id)}
              />
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function IssueLifecycleDetail({
  issue,
  storySpecs,
  designSpecs,
  workItems,
  selectedKey,
  deletingKey,
  onSelect,
  onOpenFullIssue,
  onDelete,
}: {
  issue: LifecycleCardData | null;
  storySpecs: LifecycleCardData[];
  designSpecs: LifecycleCardData[];
  workItems: LifecycleCardData[];
  selectedKey: string | null;
  deletingKey: string | null;
  onSelect: (card: LifecycleCardData) => void;
  onOpenFullIssue: (card: LifecycleCardData) => void;
  onDelete: (card: LifecycleCardData) => void;
}) {
  const allWorkItems = workItems.map((card) => card.raw as LifecycleWorkItem);
  if (!issue) {
    return (
      <section
        role="region"
        aria-label="Issue 生命周期详情"
        className="flex min-h-96 items-center justify-center rounded-md border border-dashed border-[var(--aria-line)] bg-[var(--aria-panel)] p-6 text-center"
      >
        <div className="max-w-sm">
          <h2 className="text-sm font-semibold text-[var(--aria-ink)]">
            选择一个 Issue
          </h2>
          <p className="mt-2 text-sm leading-6 text-[var(--aria-ink-muted)]">
            Story Spec、Design Spec 和 Work Item 都会作为该 Issue
            的内容展示在这里。
          </p>
        </div>
      </section>
    );
  }
  const showFullIssueAction = issue.preview
    ? shouldShowFullIssueAction(issue.preview)
    : false;

  return (
    <section
      role="region"
      aria-label="Issue 生命周期详情"
      className="min-h-0 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)]"
    >
      <div className="border-b border-[var(--aria-line)] px-4 py-3">
        <div className="flex min-w-0 flex-wrap items-start justify-between gap-3">
          <div className="min-w-0">
            <p className="text-xs font-semibold uppercase text-[var(--aria-ink-muted)]">
              Selected Issue
            </p>
            <h2 className="mt-1 truncate text-base font-semibold text-[var(--aria-ink)]">
              {issue.title}
            </h2>
            {issue.preview ? (
              <div className="relative mt-2 max-w-3xl">
                <p
                  data-testid="selected-issue-preview"
                  className={[
                    "whitespace-pre-wrap break-words text-sm leading-6 text-[var(--aria-ink-muted)]",
                    showFullIssueAction ? "line-clamp-6" : "",
                  ].join(" ")}
                >
                  {issue.preview}
                </p>
                {showFullIssueAction ? (
                  <div className="pointer-events-none absolute inset-x-0 bottom-0 h-8 bg-gradient-to-b from-transparent to-[var(--aria-panel)]" />
                ) : null}
              </div>
            ) : null}
            {showFullIssueAction ? (
              <button
                type="button"
                onClick={() => onOpenFullIssue(issue)}
                className="mt-2 inline-flex h-8 cursor-pointer items-center gap-1.5 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-2.5 text-xs font-semibold text-[var(--aria-primary)] hover:border-[var(--aria-primary)] hover:bg-[var(--aria-panel-muted)]"
              >
                <PanelRightOpen className="h-3.5 w-3.5" />
                查看完整 Issue
              </button>
            ) : null}
          </div>
        </div>
        <div className="mt-3 flex flex-wrap gap-1.5 font-mono text-[11px] text-[var(--aria-ink-muted)]">
          <span className="rounded border border-[var(--aria-line)] px-1.5 py-0.5">
            {issue.id}
          </span>
          <span className="rounded border border-[var(--aria-line)] px-1.5 py-0.5">
            {issue.status}
          </span>
        </div>
      </div>
      <div className="grid gap-3 p-3 xl:grid-cols-3">
        <LifecycleContentSection
          title="Story Spec"
          ariaLabel="Story Spec 内容"
          cards={storySpecs}
          selectedKey={selectedKey}
          deletingKey={deletingKey}
          onSelect={onSelect}
          onDelete={onDelete}
        />
        <LifecycleContentSection
          title="Design Spec"
          ariaLabel="Design Spec 内容"
          cards={designSpecs}
          selectedKey={selectedKey}
          deletingKey={deletingKey}
          onSelect={onSelect}
          onDelete={onDelete}
        />
        <LifecycleContentSection
          title="Work Item"
          ariaLabel="Work Item 内容"
          cards={workItems}
          selectedKey={selectedKey}
          deletingKey={deletingKey}
          onSelect={onSelect}
          onDelete={onDelete}
          allWorkItems={allWorkItems}
        />
      </div>
    </section>
  );
}

function shouldShowFullIssueAction(preview: string) {
  return preview.split(/\r?\n/u).length > 6 || preview.length > 520;
}

function LifecycleContentSection({
  title,
  ariaLabel,
  cards,
  selectedKey,
  deletingKey,
  onSelect,
  onDelete,
  allWorkItems,
}: {
  title: string;
  ariaLabel: string;
  cards: LifecycleCardData[];
  selectedKey: string | null;
  deletingKey: string | null;
  onSelect: (card: LifecycleCardData) => void;
  onDelete: (card: LifecycleCardData) => void;
  allWorkItems?: LifecycleWorkItem[];
}) {
  return (
    <section
      role="region"
      aria-label={ariaLabel}
      className="min-h-72 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-2"
    >
      <div className="mb-3 flex items-center justify-between gap-2">
        <h3 className="text-sm font-semibold text-[var(--aria-ink)]">
          {title}
        </h3>
        <span className="rounded border border-[var(--aria-line)] bg-[var(--aria-panel)] px-2 py-0.5 font-mono text-[11px] text-[var(--aria-ink-muted)]">
          {cards.length}
        </span>
      </div>
      {cards.length === 0 ? (
        <div className="rounded-md border border-dashed border-[var(--aria-line)] bg-[var(--aria-panel)] p-3 text-sm text-[var(--aria-ink-muted)]">
          暂无内容
        </div>
      ) : (
        <ul className="space-y-2">
          {cards.map((card) => (
            <li key={`${card.kind}:${card.id}`}>
              <LifecycleCard
                card={card}
                selected={selectedKey === lifecycleCardKey(card)}
                deleting={deletingKey === lifecycleCardKey(card)}
                onSelect={() => onSelect(card)}
                onDelete={() => onDelete(card)}
                allWorkItems={allWorkItems}
              />
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function defaultOpenWorkspace(sessionId: string) {
  window.location.assign(
    `/workbench/workspace/${encodeURIComponent(sessionId)}`,
  );
}

function waitForDeleteExitAnimation() {
  return new Promise<void>((resolve) => {
    window.setTimeout(resolve, DELETE_EXIT_ANIMATION_MS);
  });
}

function errorMessage(reason: unknown, fallback: string) {
  return reason instanceof Error ? reason.message : fallback;
}

function defaultOpenCodingWorkspace(attemptId: string) {
  window.location.assign(`/workbench/coding/${encodeURIComponent(attemptId)}`);
}

function normalizeLifecycleResponse(
  lifecycle: unknown,
  issue: ProductIssue,
): IssueLifecycleResponse {
  if (
    !isRecord(lifecycle) ||
    !isRecord(lifecycle.issue) ||
    lifecycle.issue.issue_id !== issue.issue_id ||
    !Array.isArray(lifecycle.story_specs) ||
    !Array.isArray(lifecycle.design_specs) ||
    !Array.isArray(lifecycle.work_items) ||
    !Array.isArray(lifecycle.workspace_sessions) ||
    !Array.isArray(lifecycle.coding_attempts)
  ) {
    throw new Error("invalid lifecycle response");
  }

  return lifecycle as IssueLifecycleResponse;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function lifecycleCardKey(card: LifecycleCardData) {
  return `${card.kind}:${card.id}`;
}

function selectedLifecycleColumns(
  columns: LifecycleColumns,
  focusedIssueId: string | null,
): LifecycleColumns {
  if (!focusedIssueId) {
    return { issue: [], story_spec: [], design_spec: [], work_item: [] };
  }

  return {
    issue: columns.issue.filter((card) => card.issueId === focusedIssueId),
    story_spec: columns.story_spec.filter(
      (card) => card.issueId === focusedIssueId,
    ),
    design_spec: columns.design_spec.filter(
      (card) => card.issueId === focusedIssueId,
    ),
    work_item: columns.work_item.filter(
      (card) => card.issueId === focusedIssueId,
    ),
  };
}

function findCardInColumns(
  columns: LifecycleColumns,
  entityId: string | null,
): LifecycleCardData | null {
  if (!entityId) {
    return null;
  }

  return (
    [
      ...columns.issue,
      ...columns.story_spec,
      ...columns.design_spec,
      ...columns.work_item,
    ].find((card) => card.id === entityId) ?? null
  );
}

function toDrawerEntity(
  card: LifecycleCardData,
  allWorkItems?: LifecycleWorkItem[],
): DrawerEntity {
  const base = {
    id: card.id,
    kind: card.kind,
    title: card.title,
    status: card.status,
    version: card.version,
  };

  if (card.kind === "issue") {
    return {
      ...base,
      description: card.raw.description ?? undefined,
      artifacts: card.raw.artifacts,
      phase: card.raw.phase,
      createdAt: card.raw.created_at,
    };
  }

  if (card.kind === "story_spec" || card.kind === "design_spec") {
    return {
      ...base,
      artifactVersions: card.artifactVersions,
    };
  }

  return {
    ...base,
    artifactVersions: card.artifactVersions,
    latestAttempt: card.raw.latest_attempt,
    workItemKind: card.raw.kind,
    dependsOn: card.raw.depends_on,
    exclusiveWriteScopes: card.raw.exclusive_write_scopes,
    forbiddenWriteScopes: card.raw.forbidden_write_scopes,
    contextBudget: card.raw.context_budget,
    requiredHandoffFrom: card.raw.required_handoff_from,
    verificationPlanRef: card.raw.verification_plan_ref,
    requireExecutionPlanConfirm: card.raw.require_execution_plan_confirm,
    executionPlanStatus: card.raw.execution_plan_status,
    handoffSummaryRef: card.raw.handoff_summary_ref,
    completionCommit: card.raw.completion_commit,
    completionDiffSummaryRef: card.raw.completion_diff_summary_ref,
    allWorkItems,
  };
}

export function defaultLaunchTitle(launchTarget: {
  target: ProviderWorkspaceLaunchTarget;
  card: LifecycleCardData;
}) {
  const title = compactLifecycleTitle(launchTarget.card.title);

  if (launchTarget.target === "story") {
    return `${title} Story Spec`;
  }
  if (launchTarget.target === "design") {
    return `${title} Design Spec`;
  }
  return `${title} Work Item`;
}

function compactLifecycleTitle(title: string) {
  const normalizedTitle = title.trim();
  const baseTitle = normalizedTitle
    .replace(/(?:\s+(?:Story Spec|Design Spec|Work Item))+$/u, "")
    .trim();

  return baseTitle || normalizedTitle;
}

function findWorkspaceSession(
  lifecycles: IssueLifecycleResponse[],
  card: LifecycleCardData,
): WorkspaceSession | null {
  const workspaceType = workspaceTypeForCard(card);
  if (!workspaceType) {
    return null;
  }

  return (
    lifecycles
      .find((lifecycle) => lifecycle.issue.issue_id === card.issueId)
      ?.workspace_sessions.find(
        (session) =>
          session.entity_id === card.id &&
          session.workspace_type === workspaceType,
      ) ?? null
  );
}

function workspaceTypeForCard(
  card: LifecycleCardData,
): WorkspaceSession["workspace_type"] | null {
  if (card.kind === "story_spec") {
    return "story";
  }
  if (card.kind === "design_spec") {
    return "design";
  }
  if (card.kind === "work_item") {
    return "work_item";
  }
  return null;
}
