import { Plus, RefreshCw } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import {
  createProject,
  createProductIssue,
  createRepository,
  deleteProductIssue,
  deleteProject,
  deleteRepository,
  generateDesignSpecs,
  generateStorySpecs,
  generateWorkItems,
  getIssueLifecycle,
  listProductIssues,
  listProjects,
  listRepositories,
} from "../../api/client";
import type {
  IssueLifecycleResponse,
  ProductIssue,
  Project,
  Repository,
  CreateRepositoryRequest,
  WorkspaceSession,
} from "../../api/types";
import {
  groupLifecycleCards,
  useLifecycleWorkbenchStore,
  visibleLifecycle,
  type LifecycleCard as LifecycleCardData,
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
import { LifecycleCardDrawer, type DrawerEntity } from "./LifecycleCardDrawer";
import { LifecycleColumn } from "./LifecycleColumn";
import { ProjectSidebar } from "./ProjectSidebar";
type ProviderWorkspaceLaunchTarget = "story" | "design" | "work_item";

export function IssueLifecycleWorkbench({
  focusEntityId,
  onDrawerFocusChange,
  onOpenWorkspace = defaultOpenWorkspace,
}: {
  focusEntityId?: string | null;
  onDrawerFocusChange?: (entityId: string | null) => void;
  onOpenWorkspace?: (sessionId: string) => void;
}) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [repositories, setRepositories] = useState<Repository[]>([]);
  const [lifecycles, setLifecycles] = useState<IssueLifecycleResponse[]>([]);
  const [selectedProjectId, setSelectedProjectId] = useState<string | null>(
    null,
  );
  const [focusedIssueId, setFocusedIssueId] = useState<string | null>(null);
  const [selectedCardKey, setSelectedCardKey] = useState<string | null>(null);
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
      if (projectChanged) {
        setFocusedIssueId(null);
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
  const columns = useMemo(
    () => visibleLifecycle(allColumns, focusedIssueId),
    [allColumns, focusedIssueId],
  );
  const focusedEntity = useMemo(
    () => findCardInColumns(allColumns, drawerFocusedEntityId),
    [allColumns, drawerFocusedEntityId],
  );
  const selectedProject = projects.find(
    (project) => project.project_id === selectedProjectId,
  );
  const issueCount = columns.issue.length;

  async function handleSelectProject(projectId: string) {
    if (projectId === selectedProjectId) {
      return;
    }
    setSelectedProjectId(projectId);
    await refresh(projectId);
  }

  function handleSelectCard(card: LifecycleCardData) {
    setSelectedCardKey(lifecycleCardKey(card));
    openDrawer(card.id);
    if (card.kind === "issue") {
      setFocusedIssueId(card.issueId);
    }
  }

  async function handleOpenWorkspaceFromDrawer(card: LifecycleCardData) {
    const session = findWorkspaceSession(lifecycles, card);
    if (!session) {
      setError("缺少 Workspace Session");
      return;
    }
    setError(null);
    closeDrawer();
    await refresh(selectedProjectId);
    onOpenWorkspace(session.workspace_session_id);
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
      const response = await generateWorkItems(
        selectedProjectId,
        card.issueId,
        {
          title: defaultLaunchTitle({ target: "work_item", card }),
          story_spec_ids: card.raw.story_spec_ids,
          design_spec_ids: [card.id],
        },
      );
      const nextId = response.work_items[0]?.work_item_id;
      await refresh(selectedProjectId);
      if (nextId) {
        setSelectedCardKey(`work_item:${nextId}`);
        openDrawer(nextId);
      }
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

    setError(null);
    await deleteProductIssue(selectedProjectId, issueId);
    if (focusedIssueId === issueId) {
      setFocusedIssueId(null);
    }
    setSelectedCardKey(null);
    await refresh(selectedProjectId);
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
      const response = await generateWorkItems(
        selectedProjectId,
        card.issueId,
        {
          title: defaultLaunchTitle({ target, card }),
          story_spec_ids: card.raw.story_spec_ids,
          design_spec_ids: [card.id],
        },
      );
      setSelectedCardKey(
        `work_item:${response.work_items[0]?.work_item_id ?? ""}`,
      );
      await refresh(selectedProjectId);
      if (response.workspace_session) {
        onOpenWorkspace(response.workspace_session.workspace_session_id);
      }
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
            <div className="grid min-h-[calc(100vh-6rem)] gap-3 overflow-auto rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] p-3 xl:grid-cols-4">
              <LifecycleColumn
                title="Issue"
                ariaLabel="Issue 列"
                cards={columns.issue}
                selectedKey={selectedCardKey}
                onSelect={handleSelectCard}
                onGenerateStorySpec={(card) =>
                  void handleLaunchWorkspace("story", card)
                }
                onDeleteIssue={(issueId) => void handleDeleteIssue(issueId)}
              />
              <LifecycleColumn
                title="Story Spec"
                ariaLabel="Story Spec 列"
                cards={columns.story_spec}
                selectedKey={selectedCardKey}
                onSelect={handleSelectCard}
              />
              <LifecycleColumn
                title="Design Spec"
                ariaLabel="Design Spec 列"
                cards={columns.design_spec}
                selectedKey={selectedCardKey}
                onSelect={handleSelectCard}
              />
              <LifecycleColumn
                title="Work Item"
                ariaLabel="Work Item 列"
                cards={columns.work_item}
                selectedKey={selectedCardKey}
                onSelect={handleSelectCard}
              />
            </div>
          }
        />
      </div>
      {isDrawerOpen && focusedEntity ? (
        <div className="fixed right-0 top-0 z-50 h-full w-[min(480px,100vw)] shadow-xl">
          <LifecycleCardDrawer
            entity={toDrawerEntity(focusedEntity)}
            onClose={closeDrawer}
            onOpenWorkspace={() =>
              void handleOpenWorkspaceFromDrawer(focusedEntity)
            }
            onGenerateNext={
              focusedEntity.status === "confirmed" &&
              (focusedEntity.kind === "story_spec" ||
                focusedEntity.kind === "design_spec")
                ? () => void handleGenerateNext(focusedEntity)
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

function defaultOpenWorkspace(sessionId: string) {
  window.location.assign(
    `/workbench/workspace/${encodeURIComponent(sessionId)}`,
  );
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
    !Array.isArray(lifecycle.workspace_sessions)
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

function findCardInColumns(
  columns: ReturnType<typeof visibleLifecycle>,
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

function toDrawerEntity(card: LifecycleCardData): DrawerEntity {
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

  return base;
}

function defaultLaunchTitle(launchTarget: {
  target: ProviderWorkspaceLaunchTarget;
  card: LifecycleCardData;
}) {
  if (launchTarget.target === "story") {
    return `${launchTarget.card.title} Story Spec`;
  }
  if (launchTarget.target === "design") {
    return `${launchTarget.card.title} Design Spec`;
  }
  return `${launchTarget.card.title} Work Item`;
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
