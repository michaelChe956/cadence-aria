import { Plus, RefreshCw } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { SpecGenerationMode } from "../../api/groupChat";
import {
  createCodingAttempt,
  createGroupCodingAttempt,
  createProject,
  createProductIssue,
  createRepository,
  deleteDesignSpec,
  deleteProductIssue,
  deleteProject,
  deleteRepository,
  deleteStorySpec,
  deleteWorkItem,
  deleteWorkItemPlan,
  generateDesignSpecs,
  generateStorySpecs,
  getIssueLifecycle,
  getRepositoryInitialization,
  prepareWorkItemPlan,
  listProductIssues,
  listProjects,
  listRepositories,
} from "../../api/client";
import type {
  CodingAttemptAddress,
  IssueLifecycleResponse,
  Project,
  Repository,
  CreateRepositoryRequest,
  RepositoryInitializationOperationSnapshot,
} from "../../api/types";
import {
  groupLifecycleCards,
  useLifecycleWorkbenchStore,
  type LifecycleCard as LifecycleCardData,
} from "../../state/lifecycle-workbench-store";
import { GroupChatWorkbenchCard } from "../chat-room/GroupChatWorkbenchCard";
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
import { LifecycleCardDrawer } from "./LifecycleCardDrawer";
import { ProjectSidebar } from "./ProjectSidebar";
import {
  WorkItemPlanOptionsDialog,
  type WorkItemPlanOptionsFormValue,
} from "./WorkItemPlanOptionsDialog";
import {
  IssueCardList,
  IssueLifecycleDetail,
  defaultLaunchTitle,
  defaultOpenCodingWorkspace,
  defaultOpenWorkspace,
  errorMessage,
  findCardInColumns,
  findWorkspaceSession,
  lifecycleCardKey,
  lifecycleEntityKey,
  normalizeLifecycleResponse,
  resolveGroupCodingAttempt,
  selectedLifecycleColumns,
  toDrawerEntity,
  waitForDeleteExitAnimation,
} from "./IssueLifecycleWorkbenchParts";
export { defaultLaunchTitle } from "./IssueLifecycleWorkbenchParts";
type ProviderWorkspaceLaunchTarget = "story" | "design" | "work_item";
type PendingWorkItemPlanLaunch = {
  card: LifecycleCardData;
};
const DEFAULT_WORK_ITEM_PLAN_OPTIONS = {
  include_integration_tests: true,
  include_e2e_tests: false,
  force_frontend_backend_split: true,
  require_execution_plan_confirm: false,
} satisfies WorkItemPlanOptionsFormValue;
export function IssueLifecycleWorkbench({
  focusEntityKey,
  onDrawerFocusChange,
  onOpenWorkspace = defaultOpenWorkspace,
  onOpenCodingWorkspace = defaultOpenCodingWorkspace,
  onOpenGroupChat = (sessionId) =>
    window.location.assign(`/group-chat/${encodeURIComponent(sessionId)}`),
  specGenerationMode = "pipeline",
}: {
  focusEntityKey?: string | null;
  onDrawerFocusChange?: (entityKey: string | null) => void;
  onOpenWorkspace?: (sessionId: string) => void;
  onOpenCodingWorkspace?: (address: CodingAttemptAddress) => void;
  onOpenGroupChat?: (sessionId: string) => void;
  specGenerationMode?: SpecGenerationMode;
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
  const [pendingWorkItemPlanLaunch, setPendingWorkItemPlanLaunch] =
    useState<PendingWorkItemPlanLaunch | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const refreshRequestId = useRef(0);
  const drawerFocusedEntityKey = useLifecycleWorkbenchStore(
    (state) => state.focusedEntityKey,
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
    if (focusEntityKey === undefined) {
      return;
    }
    if (focusEntityKey) {
      openDrawer(focusEntityKey);
      return;
    }
    closeDrawer();
  }, [closeDrawer, focusEntityKey, openDrawer]);

  useEffect(() => {
    if (!onDrawerFocusChange) {
      return;
    }
    onDrawerFocusChange(isDrawerOpen ? drawerFocusedEntityKey : null);
  }, [drawerFocusedEntityKey, isDrawerOpen, onDrawerFocusChange]);

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
    () => findCardInColumns(allColumns, drawerFocusedEntityKey),
    [allColumns, drawerFocusedEntityKey],
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
    const cardKey = lifecycleCardKey(card);
    setSelectedCardKey(cardKey);
    if (card.kind === "issue") {
      setFocusedIssueId(card.issueId);
      closeDrawer();
      return;
    }
    openDrawer(cardKey);
  }

  function handleOpenFullIssue(card: LifecycleCardData) {
    const cardKey = lifecycleCardKey(card);
    setSelectedCardKey(cardKey);
    setFocusedIssueId(card.issueId);
    openDrawer(cardKey);
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
    if (
      !selectedProjectId ||
      (card.kind !== "work_item" && card.kind !== "work_item_group")
    ) {
      setError("缺少 Project 或 Work Item");
      return;
    }

    if (card.kind === "work_item") {
      if (card.raw.latest_attempt) {
        setError(null);
        onOpenCodingWorkspace({
          projectId: selectedProjectId,
          issueId: card.issueId,
          attemptId: card.raw.latest_attempt.attempt_id,
        });
        return;
      }

      setError(null);
      const attempt = await createCodingAttempt(
        selectedProjectId,
        card.issueId,
        card.id,
      );
      await refresh(selectedProjectId);
      onOpenCodingWorkspace({
        projectId: selectedProjectId,
        issueId: card.issueId,
        attemptId: attempt.attempt_id,
      });
      return;
    }

    const lifecycle = lifecycles.find(
      (candidate) => candidate.issue.issue_id === card.issueId,
    );
    const latestGroupAttempt = resolveGroupCodingAttempt(
      card.raw,
      lifecycle?.coding_attempts ?? [],
      card.id,
    );

    if (latestGroupAttempt) {
      setError(null);
      onOpenCodingWorkspace({
        projectId: selectedProjectId,
        issueId: card.issueId,
        attemptId: latestGroupAttempt.attempt_id,
      });
      return;
    }

    setError(null);
    const attempt = await createGroupCodingAttempt(
      selectedProjectId,
      card.issueId,
      card.id,
    );
    await refresh(selectedProjectId);
    onOpenCodingWorkspace({
      projectId: selectedProjectId,
      issueId: card.issueId,
      attemptId: attempt.attempt_id,
    });
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
        },
      );
      const nextId = response.design_specs[0]?.design_spec_id;
      await refresh(selectedProjectId);
      if (nextId) {
        const nextKey = lifecycleEntityKey(
          "design_spec",
          card.issueId,
          nextId,
        );
        setSelectedCardKey(nextKey);
        openDrawer(nextKey);
      }
      return;
    }

    if (card.kind === "design_spec") {
      setError(null);
      setPendingWorkItemPlanLaunch({ card });
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

  async function handleStartRepositoryInitialization(
    payload: CreateRepositoryRequest,
  ): Promise<RepositoryInitializationOperationSnapshot> {
    if (!selectedProjectId) {
      const message = "缺少 Project";
      setError(message);
      throw new Error(message);
    }

    return createRepository(selectedProjectId, payload);
  }

  async function handleFetchRepositoryInitialization(operationId: string) {
    if (!selectedProjectId) {
      throw new Error("缺少 Project");
    }

    return getRepositoryInitialization(selectedProjectId, operationId);
  }

  async function handleRepositoryInitializationCompleted() {
    if (selectedProjectId) {
      await refresh(selectedProjectId);
    }
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

    const cardKey = lifecycleEntityKey("issue", issueId, issueId);
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
    } else if (card.kind === "work_item_group") {
      deleteRequest = deleteWorkItemPlan(
        selectedProjectId,
        card.issueId,
        card.id,
      );
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
      if (drawerFocusedEntityKey === cardKey) {
        closeDrawer();
      }
      await refresh(selectedProjectId);
    } catch (reason) {
      setError(errorMessage(reason, "删除生命周期实体失败"));
    } finally {
      setDeletingCardKey(null);
    }
  }

  function handleDeleteLifecycleCardFromDrawer(card: LifecycleCardData) {
    if (card.kind !== "work_item" && card.kind !== "work_item_group") {
      return;
    }
    const message =
      card.kind === "work_item_group"
        ? "删除 Work Item Group 会同时删除子 Work Item、关联 Coding Workspace、日志和 worktree，且无法撤销。"
        : "删除 Work Item 会同时删除关联的 Coding Workspace、日志和 worktree，且无法撤销。";
    const confirmed = window.confirm(message);
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
      const storySpecId = response.story_specs[0]?.story_spec_id;
      setSelectedCardKey(
        storySpecId
          ? lifecycleEntityKey("story_spec", card.issueId, storySpecId)
          : null,
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
        },
      );
      const designSpecId = response.design_specs[0]?.design_spec_id;
      setSelectedCardKey(
        designSpecId
          ? lifecycleEntityKey("design_spec", card.issueId, designSpecId)
          : null,
      );
      await refresh(selectedProjectId);
      if (response.workspace_session) {
        onOpenWorkspace(response.workspace_session.workspace_session_id);
      }
      return;
    }

    if (target === "work_item" && card.kind === "design_spec") {
      setError(null);
      setPendingWorkItemPlanLaunch({ card });
      return;
    }

    setError("当前卡片不能启动该 Workspace");
  }

  async function handleConfirmWorkItemPlanOptions(
    options: WorkItemPlanOptionsFormValue,
  ) {
    if (!selectedProjectId || !pendingWorkItemPlanLaunch) {
      setError("缺少 Project 或 Design Spec");
      return;
    }

    const { card } = pendingWorkItemPlanLaunch;
    if (card.kind !== "design_spec") {
      setError("当前实体不能生成 Work Item Plan");
      return;
    }

    setError(null);
    const response = await prepareWorkItemPlan(selectedProjectId, card.issueId, {
      title: defaultLaunchTitle({ target: "work_item", card }),
      story_spec_ids: card.raw.story_spec_ids,
      design_spec_ids: [card.id],
      ...options,
    });
    await refresh(selectedProjectId);
    setPendingWorkItemPlanLaunch(null);
    onOpenWorkspace(response.workspace_session.workspace_session_id);
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
              {specGenerationMode === "group_chat" &&
              selectedIssueColumns.issue[0] &&
              selectedProjectId ? (
                <GroupChatWorkbenchCard
                  projectId={selectedProjectId}
                  issueId={selectedIssueColumns.issue[0].issueId}
                  issueTitle={selectedIssueColumns.issue[0].title}
                  workspaceSessions={
                    lifecycles.find(
                      (lifecycle) =>
                        lifecycle.issue.issue_id ===
                        selectedIssueColumns.issue[0]?.issueId,
                    )?.workspace_sessions ?? []
                  }
                  artifactVersions={[
                    ...selectedIssueColumns.story_spec.flatMap((card) =>
                      card.kind === "story_spec" ? card.raw.artifact_versions : [],
                    ),
                    ...selectedIssueColumns.design_spec.flatMap((card) =>
                      card.kind === "design_spec" ? card.raw.artifact_versions : [],
                    ),
                  ]}
                  onOpenSession={onOpenGroupChat}
                />
              ) : (
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
              )}
            </div>
          }
        />
      </div>
      {isDrawerOpen && focusedEntity ? (
        <div className="fixed right-0 top-0 z-50 h-full w-[min(480px,100vw)] shadow-xl">
          <LifecycleCardDrawer
            key={lifecycleCardKey(focusedEntity)}
            entity={toDrawerEntity(
              focusedEntity,
              lifecycles.find(
                (lifecycle) =>
                  lifecycle.issue.issue_id === focusedEntity.issueId,
              )?.work_items ?? [],
              lifecycles.find(
                (lifecycle) =>
                  lifecycle.issue.issue_id === focusedEntity.issueId,
              )?.coding_attempts ?? [],
            )}
            onClose={closeDrawer}
            onOpenWorkspace={() =>
              void handleOpenWorkspaceFromDrawer(focusedEntity)
            }
            onOpenCodingWorkspace={
              ((focusedEntity.kind === "work_item" &&
                focusedEntity.raw.plan_status === "confirmed") ||
                (focusedEntity.kind === "work_item_group" &&
                  focusedEntity.raw.status === "confirmed"))
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
              focusedEntity.kind === "work_item" ||
              focusedEntity.kind === "work_item_group"
                ? () => handleDeleteLifecycleCardFromDrawer(focusedEntity)
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
          onCreate={handleStartRepositoryInitialization}
          onFetchOperation={handleFetchRepositoryInitialization}
          onInitializationCompleted={handleRepositoryInitializationCompleted}
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
      {pendingWorkItemPlanLaunch ? (
        <WorkItemPlanOptionsDialog
          defaultOptions={DEFAULT_WORK_ITEM_PLAN_OPTIONS}
          onConfirm={handleConfirmWorkItemPlanOptions}
          onClose={() => setPendingWorkItemPlanLaunch(null)}
        />
      ) : null}
    </>
  );
}
