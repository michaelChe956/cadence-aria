import { useEffect, useMemo, useRef, useState } from "react";
import {
  ApiRequestError,
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
import {
  getActiveAggregateIndex,
  rebuildAggregateIndex,
} from "../../api/aggregate-index";
import {
  cancelAggregateInitialization,
  getAggregateInitialization,
  startAggregateInitialization,
} from "../../api/aggregate-initialization";
import { listLogicalCodebaseMembers } from "../../api/logicalCodebaseMembers";
import { deleteLogicalCodebase, listCodebases } from "../../api/codebases";
import { LogicalCodebaseRegistrationWizard } from "./LogicalCodebaseRegistrationWizard";
import {
  createPointerPublication,
  listPointerPublications,
  retryPointerPublicationRepo,
  revokePointerPublication,
} from "../../api/pointer-publication";
import type {
  AggregateIndexActiveResponse,
  CodebaseSummaryDto,
  LogicalCodebaseDto,
  AggregateInitializationOperationSnapshot,
  CodingAttemptAddress,
  IssueLifecycleResponse,
  LogicalCodebaseMemberDto,
  PointerPublicationDto,
  Project,
  Repository,
  CreateRepositoryRequest,
  RepositoryInitializationOperationSnapshot,
  WorkItemRepositoryGroup,
} from "../../api/types";
import {
  groupLifecycleCards,
  useLifecycleWorkbenchStore,
  type LifecycleCard as LifecycleCardData,
} from "../../state/lifecycle-workbench-store";
import { WorkbenchSurface } from "../shell/WorkbenchSurface";
import {
  CreateProjectDialog,
  type CreateProjectPayload,
} from "./CreateProjectDialog";
import { AddCodebaseDialog } from "./AddCodebaseDialog";
import { CreateRepositoryDialog } from "./CreateRepositoryDialog";
import {
  CreateLifecycleIssueDialog,
  type CreateLifecycleIssuePayload,
} from "./CreateLifecycleIssueDialog";
import { AggregateIndexCard } from "./AggregateIndexCard";
import { AggregateInitializationCard } from "./AggregateInitializationCard";
import { IssueLifecycleWorkbenchHeader } from "./IssueLifecycleWorkbenchHeader";
import { LifecycleCardDrawer } from "./LifecycleCardDrawer";
import { PointerPublicationPanel } from "./PointerPublicationPanel";
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
const POLL_INTERVAL_MS = 2_000;
export function IssueLifecycleWorkbench({
  focusEntityKey,
  onDrawerFocusChange,
  onOpenWorkspace = defaultOpenWorkspace,
  onOpenCodingWorkspace = defaultOpenCodingWorkspace,
}: {
  focusEntityKey?: string | null;
  onDrawerFocusChange?: (entityKey: string | null) => void;
  onOpenWorkspace?: (sessionId: string) => void;
  onOpenCodingWorkspace?: (address: CodingAttemptAddress) => void;
}) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [repositories, setRepositories] = useState<Repository[]>([]);
  const [codebases, setCodebases] = useState<CodebaseSummaryDto[]>([]);
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
  const [registrationDialogOpen, setRegistrationDialogOpen] = useState(false);
  const [registrationWizardLcId, setRegistrationWizardLcId] = useState<
    string | null
  >(null);
  // R8：逻辑代码库选中态（多 LC 并存时面板按选中 LC 分区）。
  const [selectedLogicalCodebaseId, setSelectedLogicalCodebaseId] = useState<
    string | null
  >(null);
  const [addCodebaseDialogOpen, setAddCodebaseDialogOpen] = useState(false);
  const [pendingWorkItemPlanLaunch, setPendingWorkItemPlanLaunch] =
    useState<PendingWorkItemPlanLaunch | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pointerPublications, setPointerPublications] = useState<
    PointerPublicationDto[]
  >([]);
  const [logicalCodebaseMembers, setLogicalCodebaseMembers] = useState<
    LogicalCodebaseMemberDto[]
  >([]);
  const [pointerPublicationBusy, setPointerPublicationBusy] = useState(false);
  const [aggregateIndex, setAggregateIndex] =
    useState<AggregateIndexActiveResponse | null>(null);
  const [aggregateIndexRebuilding, setAggregateIndexRebuilding] =
    useState(false);
  const [aggregateInitialization, setAggregateInitialization] =
    useState<AggregateInitializationOperationSnapshot | null>(null);
  const [aggregateInitializationBusy, setAggregateInitializationBusy] =
    useState(false);
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
        setCodebases([]);
        setLifecycles([]);
        setPointerPublications([]);
        setLogicalCodebaseMembers([]);
        setAggregateIndex(null);
        setSelectedLogicalCodebaseId(null);
        setAggregateInitialization(null);
        setFocusedIssueId(null);
        setSelectedCardKey(null);
        return;
      }

      const [repositoryResponse, codebaseResponse, issueResponse] =
        await Promise.all([
          listRepositories(projectId),
          listCodebases(projectId),
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
      setCodebases(codebaseResponse.codebases ?? []);
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
        setSelectedLogicalCodebaseId(null);
        setAggregateInitialization(null);
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
  // REQ-TGT-05：取当前聚焦 Issue 的 work_item_repository_groups，传给详情组件按仓渲染。
  // useMemo 守护：避免每次 render 新建 [] 引用触发 WorkItemRepositoryGroupSection 无谓重渲染（#4 收尾）。
  const focusedIssueIdForGroups = selectedIssueColumns.issue[0]?.issueId;
  const focusedWorkItemRepositoryGroups = useMemo<WorkItemRepositoryGroup[]>(
    () =>
      focusedIssueIdForGroups
        ? (lifecycles.find(
            (lifecycle) =>
              lifecycle.issue.issue_id === focusedIssueIdForGroups,
          )?.work_item_repository_groups ?? [])
        : [],
    [lifecycles, focusedIssueIdForGroups],
  );
  const focusedEntity = useMemo(
    () => findCardInColumns(allColumns, drawerFocusedEntityKey),
    [allColumns, drawerFocusedEntityKey],
  );
  const logicalCodebases = codebases.filter(
    (codebase) => codebase.kind === "logical",
  );
  // R8：选中态管理——显式选中优先（须仍存在），否则回退首个；修复「面板取首个」。
  const activeLogicalCodebaseId =
    selectedLogicalCodebaseId &&
    logicalCodebases.some(
      (codebase) => codebase.logical_codebase_id === selectedLogicalCodebaseId,
    )
      ? selectedLogicalCodebaseId
      : (logicalCodebases[0]?.logical_codebase_id ?? null);
  const wizardLogicalCodebaseId =
    registrationWizardLcId ?? activeLogicalCodebaseId;
  const selectedProject = projects.find(
    (project) => project.project_id === selectedProjectId,
  );
  const issueCount = allColumns.issue.length;
  const latestPointerPublication = useMemo<PointerPublicationDto | null>(() => {
    if (pointerPublications.length === 0) {
      return null;
    }
    return [...pointerPublications].sort((left, right) =>
      right.created_at.localeCompare(left.created_at),
    )[0];
  }, [pointerPublications]);

  const latestPointerPublicationId =
    latestPointerPublication?.status === "in_progress"
      ? latestPointerPublication.id
      : null;
  const latestCompletedPointerPublication = useMemo<PointerPublicationDto | null>(() => {
    const completed = pointerPublications.filter(
      (publication) => publication.status !== "in_progress",
    );
    if (completed.length === 0) {
      return null;
    }
    return [...completed].sort((left, right) =>
      right.created_at.localeCompare(left.created_at),
    )[0];
  }, [pointerPublications]);
  const showIncrementalHint =
    latestCompletedPointerPublication !== null &&
    logicalCodebaseMembers.length > latestCompletedPointerPublication.entries.length;
  // R8：按选中 LC 拉取成员/指针发布/聚合索引（多 LC 并存时面板数据随选中态切换）。
  // I1：LC/Project 切换时同步重置 aggregateInitialization，避免残留上一 LC 的 operation
  // （其轮询会带着陈旧 operation_id 打到新 LC 路径上）。
  useEffect(() => {
    setAggregateInitialization(null);
    if (!selectedProjectId || !activeLogicalCodebaseId) {
      setLogicalCodebaseMembers([]);
      setPointerPublications([]);
      setAggregateIndex(null);
      return;
    }
    let disposed = false;
    (async () => {
      try {
        const [membersResponse, publicationResponse] = await Promise.all([
          listLogicalCodebaseMembers(
            selectedProjectId,
            activeLogicalCodebaseId,
          ),
          listPointerPublications(selectedProjectId, activeLogicalCodebaseId),
        ]);
        if (disposed) {
          return;
        }
        setLogicalCodebaseMembers(membersResponse.members ?? []);
        setPointerPublications(publicationResponse ?? []);
        setAggregateIndex(
          (membersResponse.members ?? []).length > 0
            ? await getActiveAggregateIndex(
                selectedProjectId,
                activeLogicalCodebaseId,
              )
            : null,
        );
      } catch {
        // LC 作用域数据加载失败保持现状，交由全局 refresh 重试。
      }
    })();
    return () => {
      disposed = true;
    };
  }, [selectedProjectId, activeLogicalCodebaseId]);

  useEffect(() => {
    if (
      !selectedProjectId ||
      !activeLogicalCodebaseId ||
      !latestPointerPublicationId
    ) {
      return;
    }

    let disposed = false;
    let inFlight = false;
    const poll = async () => {
      if (inFlight) {
        return;
      }
      inFlight = true;
      try {
        const publications = await listPointerPublications(
          selectedProjectId,
          activeLogicalCodebaseId,
        );
        if (!disposed) {
          setPointerPublications(publications);
        }
      } catch {
        // 轮询失败保持现状，下一次间隔重试。
      } finally {
        inFlight = false;
      }
    };
    const interval = window.setInterval(() => void poll(), POLL_INTERVAL_MS);
    return () => {
      disposed = true;
      window.clearInterval(interval);
    };
  }, [latestPointerPublicationId, selectedProjectId, activeLogicalCodebaseId]);

  useEffect(() => {
    if (
      !selectedProjectId ||
      !activeLogicalCodebaseId ||
      !aggregateInitialization ||
      (aggregateInitialization.status !== "created" &&
        aggregateInitialization.status !== "running")
    ) {
      return;
    }

    let disposed = false;
    let inFlight = false;
    const poll = async () => {
      if (inFlight) {
        return;
      }
      inFlight = true;
      try {
        const operation = await getAggregateInitialization(
          selectedProjectId,
          activeLogicalCodebaseId,
          aggregateInitialization.operation_id,
        );
        if (!disposed) {
          setAggregateInitialization(operation);
        }
      } catch {
        // 轮询失败保持现状，下一次间隔重试。
      } finally {
        inFlight = false;
      }
    };
    const interval = window.setInterval(() => void poll(), POLL_INTERVAL_MS);
    return () => {
      disposed = true;
      window.clearInterval(interval);
    };
  }, [
    selectedProjectId,
    activeLogicalCodebaseId,
    aggregateInitialization?.operation_id,
    aggregateInitialization?.status,
  ]);

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
      logical_codebase_id: payload.logical_codebase_id,
    });
    setDialogOpen(false);
    await refresh();
  }

  function handleChooseSingleCodebase() {
    setAddCodebaseDialogOpen(false);
    setRepositoryDialogOpen(true);
  }

  async function handleCreatedLogicalCodebase(
    codebase: LogicalCodebaseDto,
  ) {
    setAddCodebaseDialogOpen(false);
    setRegistrationWizardLcId(codebase.id);
    setRegistrationDialogOpen(true);
    if (selectedProjectId) {
      await refresh(selectedProjectId);
    }
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

  function upsertPointerPublication(publication: PointerPublicationDto) {
    setPointerPublications((existing) => [
      ...existing.filter((item) => item.id !== publication.id),
      publication,
    ]);
  }

  async function handlePublishFull() {
    if (!selectedProjectId || !activeLogicalCodebaseId) {
      setError("缺少 Project 或逻辑代码库");
      return;
    }

    setPointerPublicationBusy(true);
    setError(null);
    try {
      const publication = await createPointerPublication(
        selectedProjectId,
        activeLogicalCodebaseId,
        "full",
      );
      upsertPointerPublication(publication);
    } catch (reason) {
      setError(errorMessage(reason, "全量发布失败"));
    } finally {
      setPointerPublicationBusy(false);
    }
  }

  async function handlePublishIncremental() {
    if (!selectedProjectId || !activeLogicalCodebaseId) {
      setError("缺少 Project 或逻辑代码库");
      return;
    }

    setPointerPublicationBusy(true);
    setError(null);
    try {
      const publication = await createPointerPublication(
        selectedProjectId,
        activeLogicalCodebaseId,
        "incremental",
      );
      upsertPointerPublication(publication);
    } catch (reason) {
      setError(errorMessage(reason, "增量发布失败"));
    } finally {
      setPointerPublicationBusy(false);
    }
  }

  async function handleRetryRepo(memberRepoId: string) {
    if (
      !selectedProjectId ||
      !activeLogicalCodebaseId ||
      !latestPointerPublication
    ) {
      setError("缺少 Project 或发布批次");
      return;
    }

    setPointerPublicationBusy(true);
    setError(null);
    try {
      const publication = await retryPointerPublicationRepo(
        selectedProjectId,
        activeLogicalCodebaseId,
        latestPointerPublication.id,
        memberRepoId,
      );
      upsertPointerPublication(publication);
    } catch (reason) {
      setError(errorMessage(reason, "重试失败"));
    } finally {
      setPointerPublicationBusy(false);
    }
  }

  async function handleRevokePublication() {
    if (
      !selectedProjectId ||
      !activeLogicalCodebaseId ||
      !latestPointerPublication
    ) {
      setError("缺少 Project 或发布批次");
      return;
    }

    setPointerPublicationBusy(true);
    setError(null);
    try {
      const publication = await revokePointerPublication(
        selectedProjectId,
        activeLogicalCodebaseId,
        latestPointerPublication.id,
      );
      upsertPointerPublication(publication);
    } catch (reason) {
      setError(errorMessage(reason, "撤回失败"));
    } finally {
      setPointerPublicationBusy(false);
    }
  }

  async function handleRebuildAggregateIndex() {
    if (!selectedProjectId || !activeLogicalCodebaseId) {
      setError("缺少 Project 或逻辑代码库");
      return;
    }

    setAggregateIndexRebuilding(true);
    setError(null);
    try {
      setAggregateIndex(
        await rebuildAggregateIndex(selectedProjectId, activeLogicalCodebaseId),
      );
    } catch (reason) {
      setError(
        reason instanceof ApiRequestError
          ? reason.message
          : errorMessage(reason, "重建聚合索引失败"),
      );
    } finally {
      setAggregateIndexRebuilding(false);
    }
  }

  async function handleStartAggregateInitialization() {
    if (!selectedProjectId || !activeLogicalCodebaseId) {
      setError("缺少 Project 或逻辑代码库");
      return;
    }

    setAggregateInitializationBusy(true);
    setError(null);
    try {
      const operation = await startAggregateInitialization(
        selectedProjectId,
        activeLogicalCodebaseId,
        crypto.randomUUID(),
      );
      setAggregateInitialization(operation);
    } catch (reason) {
      setError(
        reason instanceof ApiRequestError
          ? reason.message
          : errorMessage(reason, "启动聚合初始化失败"),
      );
    } finally {
      setAggregateInitializationBusy(false);
    }
  }

  async function handleCancelAggregateInitialization() {
    if (!selectedProjectId || !activeLogicalCodebaseId || !aggregateInitialization) {
      setError("缺少 Project 或初始化操作");
      return;
    }

    setAggregateInitializationBusy(true);
    setError(null);
    try {
      const operation = await cancelAggregateInitialization(
        selectedProjectId,
        activeLogicalCodebaseId,
        aggregateInitialization.operation_id,
        { reason: "user_cancelled", detail: null },
      );
      setAggregateInitialization(operation);
    } catch (reason) {
      setError(
        reason instanceof ApiRequestError
          ? reason.message
          : errorMessage(reason, "取消聚合初始化失败"),
      );
    } finally {
      setAggregateInitializationBusy(false);
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
    await deleteRepository(
      selectedProjectId,
      repositoryId,
      `delete-repository-${crypto.randomUUID()}`,
    );
    setSelectedCardKey(null);
    await refresh(selectedProjectId);
  }

  // R8：逻辑条目软删入口（二次确认；软删后刷新混合列表）。
  async function handleDeleteLogicalCodebase(logicalCodebaseId: string) {
    if (!selectedProjectId) {
      setError("缺少 Project");
      return;
    }

    const confirmed = window.confirm(
      "删除逻辑代码库将软删除其登记成员、聚合索引与指针发布上下文，且无法在界面内撤销。确认删除？",
    );
    if (!confirmed) {
      return;
    }

    setError(null);
    try {
      await deleteLogicalCodebase(selectedProjectId, logicalCodebaseId);
      if (selectedLogicalCodebaseId === logicalCodebaseId) {
        setSelectedLogicalCodebaseId(null);
      }
      await refresh(selectedProjectId);
    } catch (reason) {
      setError(errorMessage(reason, "删除逻辑代码库失败"));
    }
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
          codebases={codebases}
          repositories={repositories}
          selectedProjectId={selectedProjectId}
          issueCount={issueCount}
          busy={busy}
          onSelectProject={(projectId) => void handleSelectProject(projectId)}
          onCreateProject={() => setProjectDialogOpen(true)}
          onAddCodebase={() => setAddCodebaseDialogOpen(true)}
          onDeleteProject={(projectId) => void handleDeleteProject(projectId)}
          onDeleteRepository={(repositoryId) =>
            void handleDeleteRepository(repositoryId)
          }
          onDeleteLogicalCodebase={(logicalCodebaseId) =>
            void handleDeleteLogicalCodebase(logicalCodebaseId)
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
            <IssueLifecycleWorkbenchHeader
              projectName={selectedProject?.name}
              focusedIssueId={focusedIssueId}
              canCreateIssue={Boolean(selectedProjectId) && repositories.length > 0}
              onShowAll={() => setFocusedIssueId(null)}
              onRefresh={() => void refresh()}
              onCreateIssue={() => setDialogOpen(true)}
            />
          }
          main={
            <div className="space-y-3">
              {selectedProjectId ? (
                <div className="overflow-hidden rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)]">
                  <div className="flex items-center justify-between gap-3 border-b border-[var(--aria-line)] px-3 py-2">
                    <h2 className="text-sm font-semibold text-[var(--aria-ink)]">逻辑代码库</h2>
                    <button
                      type="button"
                      disabled={logicalCodebases.length === 0}
                      onClick={() => setRegistrationDialogOpen(true)}
                      className="rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 py-1.5 text-xs font-semibold text-white disabled:opacity-60"
                    >
                      登记成员
                    </button>
                  </div>
                  {logicalCodebases.length > 0 ? (
                    <div
                      role="tablist"
                      aria-label="逻辑代码库切换"
                      className="flex flex-wrap gap-2 border-b border-[var(--aria-line)] px-3 py-2"
                    >
                      {logicalCodebases.map((codebase) => {
                        const lcId =
                          codebase.logical_codebase_id ?? codebase.id;
                        const selected = lcId === activeLogicalCodebaseId;
                        return (
                          <button
                            key={codebase.id}
                            type="button"
                            role="tab"
                            aria-selected={selected}
                            data-testid={`lc-selector-${codebase.name}`}
                            onClick={() => setSelectedLogicalCodebaseId(lcId)}
                            className={
                              selected
                                ? "rounded-md border border-[var(--aria-primary)] bg-[var(--aria-panel-muted)] px-3 py-1 text-xs font-semibold text-[var(--aria-primary)] ring-2 ring-[var(--aria-primary)]"
                                : "rounded-md border border-[var(--aria-line)] px-3 py-1 text-xs font-semibold text-[var(--aria-ink-muted)]"
                            }
                          >
                            {codebase.name}
                          </button>
                        );
                      })}
                    </div>
                  ) : null}
                  {logicalCodebaseMembers.length > 0 ? (
                    <AggregateInitializationCard
                      operation={aggregateInitialization}
                      busy={aggregateInitializationBusy}
                      onStart={() => void handleStartAggregateInitialization()}
                      onCancel={() => void handleCancelAggregateInitialization()}
                    />
                  ) : null}
                  {aggregateIndex ? (
                    <AggregateIndexCard
                      index={aggregateIndex}
                      rebuilding={aggregateIndexRebuilding}
                      onRebuild={() => void handleRebuildAggregateIndex()}
                    />
                  ) : null}
                  <PointerPublicationPanel
                    publication={latestPointerPublication}
                    busy={pointerPublicationBusy}
                    showIncrementalHint={showIncrementalHint}
                    onPublishFull={() => void handlePublishFull()}
                    onPublishIncremental={() =>
                      void handlePublishIncremental()
                    }
                    onRetryRepo={(memberRepoId) =>
                      void handleRetryRepo(memberRepoId)
                    }
                    onRevoke={() => void handleRevokePublication()}
                  />
                </div>
              ) : null}
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
                  workItemRepositoryGroups={focusedWorkItemRepositoryGroups}
                  selectedKey={selectedCardKey}
                  onSelect={handleSelectCard}
                  onOpenFullIssue={handleOpenFullIssue}
                  onDelete={handleDeleteLifecycleCard}
                  deletingKey={deletingCardKey}
                />
              </div>
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
            deliverySummary={
              focusedEntity.kind === "issue"
                ? lifecycles.find(
                    (lifecycle) =>
                      lifecycle.issue.issue_id === focusedEntity.issueId,
                  )?.delivery_summary
                : undefined
            }
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
      {addCodebaseDialogOpen && selectedProjectId ? (
        <AddCodebaseDialog
          projectId={selectedProjectId}
          onChooseSingle={handleChooseSingleCodebase}
          onCreatedLogical={(codebase) =>
            void handleCreatedLogicalCodebase(codebase)
          }
          onClose={() => setAddCodebaseDialogOpen(false)}
        />
      ) : null}
      {registrationDialogOpen &&
      selectedProjectId &&
      wizardLogicalCodebaseId ? (
        <LogicalCodebaseRegistrationWizard
          projectId={selectedProjectId}
          logicalCodebaseId={wizardLogicalCodebaseId}
          onCompleted={() => refresh(selectedProjectId)}
          onClose={() => {
            setRegistrationDialogOpen(false);
            setRegistrationWizardLcId(null);
          }}
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
          codebases={codebases}
          listMembers={(logicalCodebaseId) =>
            selectedProjectId
              ? listLogicalCodebaseMembers(
                  selectedProjectId,
                  logicalCodebaseId,
                ).then((response) => response.members ?? [])
              : Promise.resolve([])
          }
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
