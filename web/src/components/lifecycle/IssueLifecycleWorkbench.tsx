import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { PanelLeftClose, PanelLeftOpen } from "lucide-react";
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
  getIssueLifecycle,
  getRepositoryInitialization,
  listProductIssues,
  listProjects,
  listRepositoryBranches,
  listRepositories,
} from "../../api/client";
import { rebuildAggregateIndex } from "../../api/aggregate-index";
import {
  cancelAggregateInitialization,
  startAggregateInitialization,
} from "../../api/aggregate-initialization";
import { listLogicalCodebaseMembers } from "../../api/logicalCodebaseMembers";
import { deleteLogicalCodebase, listCodebases } from "../../api/codebases";
import { LogicalCodebaseRegistrationWizard } from "./LogicalCodebaseRegistrationWizard";
import { useLogicalCodebaseScopeData } from "./useLogicalCodebaseScopeData";
import { useIssueLifecycleWorkbenchActions } from "./useIssueLifecycleWorkbenchActions";
import {
  createPointerPublication,
  retryPointerPublicationRepo,
  revokePointerPublication,
} from "../../api/pointer-publication";
import type {
  CodebaseSummaryDto,
  LogicalCodebaseDto,
  CodingAttemptAddress,
  IssueLifecycleResponse,
  PointerPublicationDto,
  ProductIssue,
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
import { useLifecycleInvalidationRefresh } from "./useLifecycleInvalidationRefresh";
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
import { IssueLifecycleWorkbenchHeader } from "./IssueLifecycleWorkbenchHeader";
import { LogicalCodebaseManagementPanel } from "./LogicalCodebaseManagementPanel";
import { LogicalCodebaseSummaryBar } from "./LogicalCodebaseSummaryBar";
import { IssueLifecycleWorkbenchDrawer } from "./IssueLifecycleWorkbenchDrawer";
import { ProjectSidebar } from "./ProjectSidebar";
import { WorkItemPlanOptionsDialog } from "./WorkItemPlanOptionsDialog";
import {
  useIssueLifecycleGeneration,
  type PendingWorkItemPlanLaunch,
} from "./useIssueLifecycleGeneration";
import { IssueQueue } from "./IssueQueue";
import {
  defaultCollapsedGroups,
  deriveIssueQueue,
  type IssueQueueGroupKey,
} from "./issue-queue-derivation";
import {
  lcSummaryStorageKey,
  queueCollapsedStorageKey,
  queueGroupsStorageKey,
  readStoredCollapsedGroups,
  readStoredLcSummaryExpanded,
  readStoredQueueCollapsed,
  writeStoredValue,
} from "./IssueLifecycleWorkbenchStorage";
import {
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
  waitForDeleteExitAnimation,
} from "./IssueLifecycleWorkbenchParts";
import { IssueLifecycleWorkbenchView } from "./IssueLifecycleWorkbenchView";
export { defaultLaunchTitle } from "./IssueLifecycleWorkbenchParts";

// 稳定空数组引用：避免每次 render 新建 [] 使 useMemo 依赖失效。
const EMPTY_GROUP_KEYS: IssueQueueGroupKey[] = [];

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
  const [pointerPublicationBusy, setPointerPublicationBusy] = useState(false);
  const [aggregateIndexRebuilding, setAggregateIndexRebuilding] =
    useState(false);
  const [aggregateInitializationBusy, setAggregateInitializationBusy] =
    useState(false);
  // Task 7：双密度外壳的队列状态。全部按 projectId 记忆，仅在首次遇到该 Project 时
  // 从 localStorage 回填；后续以内存 Map 为权威（2s 轮询不会重置这些 Map）。
  const [queueCollapsedByProject, setQueueCollapsedByProject] = useState<
    Record<string, boolean>
  >({});
  const [collapsedGroupsByProject, setCollapsedGroupsByProject] = useState<
    Record<string, IssueQueueGroupKey[]>
  >({});
  // 过滤文本与「已追加」组也按 projectId 隔离，避免切 Project 后沿用上一个的过滤态。
  const [queueFilterByProject, setQueueFilterByProject] = useState<
    Record<string, string>
  >({});
  const [showMoreGroupsByProject, setShowMoreGroupsByProject] = useState<
    Record<string, IssueQueueGroupKey[]>
  >({});
  // Task 8：运维摘要条展开态，同样按 projectId 记忆（内存 Map 优先，缺失时读 localStorage）。
  const [lcSummaryExpanded, setLcSummaryExpanded] = useState<
    Record<string, boolean>
  >({});
  const refreshRequestId = useRef(0);
  const createdIssuesRef = useRef<ProductIssue[]>([]);
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

  // F-29：confirm 成功后的 lifecycle invalidation 订阅——定向刷新对应 issue。
  useLifecycleInvalidationRefresh({
    selectedProjectId,
    lifecycles,
    refreshRequestId,
    setLifecycles,
    setError,
    setBusy,
  });

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

  async function refresh(
    projectIdOverride?: string | null,
    optimisticIssues: readonly ProductIssue[] = [],
  ) {
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

      const listedIssues = issueResponse.issues ?? [];
      const pendingCreatedIssues = createdIssuesRef.current.filter(
        (createdIssue) => !listedIssues.some((issue) => issue.issue_id === createdIssue.issue_id),
      );
      createdIssuesRef.current = pendingCreatedIssues;
      const materializedIssues = [...listedIssues, ...pendingCreatedIssues, ...optimisticIssues]
        .reduce<ProductIssue[]>((issues, issue) =>
          issues.some((candidate) => candidate.issue_id === issue.issue_id)
            ? issues
            : [...issues, issue],
        []);
      if (optimisticIssues.length > 0) {
        materializedIssues.sort((left, right) =>
          (right.updated_at ?? right.created_at).localeCompare(left.updated_at ?? left.created_at),
        );
      }
      const lifecycleResponses = await Promise.all(
        materializedIssues.map(async (issue) =>
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
          : (lifecycleResponses[0]?.issue.issue_id ?? null),
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
            (lifecycle) => lifecycle.issue.issue_id === focusedIssueIdForGroups,
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
  // Task 7：队列派生与双密度状态的有效值。内存 Map 优先，缺失时从 localStorage 回填
  // （分组折叠缺省 defaultCollapsedGroups()）。轮询只写 lifecycles，不碰这些 Map。
  const queueCollapsed = selectedProjectId
    ? (queueCollapsedByProject[selectedProjectId] ??
      readStoredQueueCollapsed(selectedProjectId))
    : false;
  const collapsedQueueGroups = useMemo<IssueQueueGroupKey[]>(
    () =>
      selectedProjectId
        ? (collapsedGroupsByProject[selectedProjectId] ??
          readStoredCollapsedGroups(selectedProjectId))
        : defaultCollapsedGroups(),
    [collapsedGroupsByProject, selectedProjectId],
  );
  const queueFilterText = selectedProjectId
    ? (queueFilterByProject[selectedProjectId] ?? "")
    : "";
  const showMoreQueueGroups = selectedProjectId
    ? (showMoreGroupsByProject[selectedProjectId] ?? EMPTY_GROUP_KEYS)
    : EMPTY_GROUP_KEYS;
  // 「显示更多」受控实现（闭合 Task 4 评审 Important-1）：命中组用无上限 perGroupLimit
  // 重派生后替换该组 rows，于是 rows.length === total，入口自然消失。
  const issueQueueGroups = useMemo(() => {
    const baseGroups = deriveIssueQueue(lifecycles, {
      filterText: queueFilterText,
    });
    if (showMoreQueueGroups.length === 0) {
      return baseGroups;
    }
    const expandedByKey = new Map(
      deriveIssueQueue(lifecycles, {
        filterText: queueFilterText,
        perGroupLimit: Number.MAX_SAFE_INTEGER,
      }).map((group) => [group.key, group]),
    );
    return baseGroups.map((group) =>
      showMoreQueueGroups.includes(group.key)
        ? (expandedByKey.get(group.key) ?? group)
        : group,
    );
  }, [lifecycles, queueFilterText, showMoreQueueGroups]);
  const queueTotalCount = issueQueueGroups.reduce(
    (sum, group) => sum + group.total,
    0,
  );
  // 队列行的 aria-busy 需要 issueId；页面只维护 cardKey（issue 卡的 key 形如
  // "issue:<issueId>:<issueId>"），这里反向取回当前正在删除的 Issue。
  const deletingIssueId =
    allColumns.issue.find(
      (card) => lifecycleCardKey(card) === deletingCardKey,
    )?.issueId ?? null;
  // R9 fix round 1【Important-2】：LC 作用域数据（成员/指针发布/聚合索引/轮询）抽取为
  // 独立 hook（纯搬运，无行为改动）。
  const {
    logicalCodebaseMembers,
    setLogicalCodebaseMembers,
    pointerPublications,
    setPointerPublications,
    aggregateIndex,
    setAggregateIndex,
    aggregateInitialization,
    setAggregateInitialization,
    latestPointerPublication,
    showIncrementalHint,
  } = useLogicalCodebaseScopeData({
    selectedProjectId,
    activeLogicalCodebaseId,
  });
  const {
    handlePublishFull,
    handlePublishIncremental,
    handleRetryRepo,
    handleRevokePublication,
    handleRebuildAggregateIndex,
    handleStartAggregateInitialization,
    handleCancelAggregateInitialization,
    handleDeleteProject,
    handleDeleteRepository,
    handleDeleteLogicalCodebase,
  } = useIssueLifecycleWorkbenchActions({
    selectedProjectId,
    activeLogicalCodebaseId,
    latestPointerPublication,
    aggregateInitialization,
    selectedLogicalCodebaseId,
    setSelectedProjectId,
    setFocusedIssueId,
    setSelectedCardKey,
    setSelectedLogicalCodebaseId,
    setError,
    setPointerPublicationBusy,
    setPointerPublications,
    setAggregateIndex,
    setAggregateIndexRebuilding,
    setAggregateInitialization,
    setAggregateInitializationBusy,
    refresh,
  });
  // Task 6 收尾：plan/story/design 创建入口（含 provider 快照与失败可见化）整块抽到
  // useIssueLifecycleGeneration——纯搬运，零行为变化。
  const {
    handleLaunchWorkspace,
    handleGenerateNext,
    handleGenerateForStage,
    handleConfirmWorkItemPlanOptions,
  } = useIssueLifecycleGeneration({
    selectedProjectId,
    selectedColumns: selectedIssueColumns,
    pendingWorkItemPlanLaunch,
    setPendingWorkItemPlanLaunch,
    setError,
    setSelectedCardKey,
    refresh,
    openDrawer,
    onOpenWorkspace,
  });
  // Task 8：运维摘要条的派生值。异常口径：聚合索引缺失（null，尚未建立）或
  // state !== "active"，或最近一次指针发布 status 含 failed/partial。
  const lcSummaryExpandedForProject = selectedProjectId
    ? (lcSummaryExpanded[selectedProjectId] ??
      readStoredLcSummaryExpanded(selectedProjectId))
    : false;
  const activeLogicalCodebaseName =
    logicalCodebases.find(
      (codebase) => codebase.logical_codebase_id === activeLogicalCodebaseId,
    )?.name ?? null;
  const lcSummaryHasWarning =
    (aggregateIndex === null || aggregateIndex.state !== "active") ||
    (latestPointerPublication !== null &&
      (latestPointerPublication.status.includes("failed") ||
        latestPointerPublication.status.includes("partial")));

  // Task 8：展开/折叠仅切换运维面板可见性——不发请求、不动选中态。
  function handleToggleLcSummary() {
    if (!selectedProjectId) {
      return;
    }
    const next = !lcSummaryExpandedForProject;
    setLcSummaryExpanded((existing) => ({
      ...existing,
      [selectedProjectId]: next,
    }));
    writeStoredValue(lcSummaryStorageKey(selectedProjectId), next ? "1" : "0");
  }

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
    // Task 2：选中任何子实体时同步聚焦其所属 Issue，
    // 保证左侧 Issue 卡高亮（selected = card.issueId === focusedIssueId）不丢失。
    setFocusedIssueId(card.issueId);
    if (card.kind === "issue") {
      closeDrawer();
      return;
    }
    openDrawer(cardKey);
  }

  // Task 7：队列行选择——找到对应 Issue 卡后复用 handleSelectCard（选择语义单一入口）。
  function handleSelectIssueFromQueue(issueId: string) {
    const card = allColumns.issue.find(
      (candidate) => candidate.issueId === issueId,
    );
    if (!card) {
      return;
    }
    handleSelectCard(card);
  }

  function handleGenerateStorySpecFromQueue(issueId: string) {
    const card = allColumns.issue.find(
      (candidate) => candidate.issueId === issueId,
    );
    if (!card) {
      setError("缺少 Issue");
      return;
    }
    void handleLaunchWorkspace("story", card);
  }

  // Task 7：折叠/展开仅切换密度——不动 focusedIssueId/selectedCardKey，不发请求。
  function handleToggleQueueCollapsed() {
    if (!selectedProjectId) {
      return;
    }
    const next = !queueCollapsed;
    setQueueCollapsedByProject((existing) => ({
      ...existing,
      [selectedProjectId]: next,
    }));
    writeStoredValue(
      queueCollapsedStorageKey(selectedProjectId),
      next ? "1" : "0",
    );
  }

  function handleToggleQueueGroup(key: IssueQueueGroupKey) {
    if (!selectedProjectId) {
      return;
    }
    const next = collapsedQueueGroups.includes(key)
      ? collapsedQueueGroups.filter((candidate) => candidate !== key)
      : [...collapsedQueueGroups, key];
    setCollapsedGroupsByProject((existing) => ({
      ...existing,
      [selectedProjectId]: next,
    }));
    writeStoredValue(
      queueGroupsStorageKey(selectedProjectId),
      JSON.stringify(next),
    );
  }

  function handleQueueFilterTextChange(text: string) {
    if (!selectedProjectId) {
      return;
    }
    setQueueFilterByProject((existing) => ({
      ...existing,
      [selectedProjectId]: text,
    }));
    // 过滤变化即重派生：「已追加」组复位（闭合 Task 4 评审 Important-2）。
    setShowMoreGroupsByProject((existing) =>
      (existing[selectedProjectId] ?? EMPTY_GROUP_KEYS).length === 0
        ? existing
        : { ...existing, [selectedProjectId]: [] },
    );
  }

  function handleShowMoreQueueGroup(key: IssueQueueGroupKey) {
    if (!selectedProjectId) {
      return;
    }
    setShowMoreGroupsByProject((existing) => {
      const current = existing[selectedProjectId] ?? EMPTY_GROUP_KEYS;
      if (current.includes(key)) {
        return existing;
      }
      return { ...existing, [selectedProjectId]: [...current, key] };
    });
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

  async function handleCreateIssue(payload: CreateLifecycleIssuePayload) {
    if (!selectedProjectId) {
      setError("缺少 Project");
      return;
    }

    const createdIssue = await createProductIssue(selectedProjectId, {
      title: payload.title,
      description: payload.description,
      change_id: null,
      repository_id: payload.repository_id,
      logical_codebase_id: payload.logical_codebase_id,
      base_branch: payload.base_branch,
    });
    createdIssuesRef.current = [
      ...createdIssuesRef.current.filter((issue) => issue.issue_id !== createdIssue.issue_id),
      createdIssue,
    ];
    setDialogOpen(false);
    await refresh(selectedProjectId, [createdIssue]);
  }

  function handleChooseSingleCodebase() {
    setAddCodebaseDialogOpen(false);
    setRepositoryDialogOpen(true);
  }

  async function handleCreatedLogicalCodebase(codebase: LogicalCodebaseDto) {
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

  const dialogs: ReactNode = (
    <>
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
          projectId={selectedProjectId}
          repositories={repositories}
          codebases={codebases}
          listBranches={(projectId, repositoryId) =>
            listRepositoryBranches(projectId, repositoryId)
          }
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
          defaultOptions={pendingWorkItemPlanLaunch.options}
          onConfirm={handleConfirmWorkItemPlanOptions}
          onClose={() => setPendingWorkItemPlanLaunch(null)}
        />
      ) : null}
    </>
  );

  return (
    <IssueLifecycleWorkbenchView
      projects={projects}
      codebases={codebases}
      repositories={repositories}
      selectedProjectId={selectedProjectId}
      issueCount={issueCount}
      busy={busy}
      error={error}
      selectedProject={selectedProject}
      focusedIssueId={focusedIssueId}
      selectedCardKey={selectedCardKey}
      deletingCardKey={deletingCardKey}
      selectedIssue={selectedIssueColumns.issue[0] ?? null}
      storySpecs={selectedIssueColumns.story_spec}
      designSpecs={selectedIssueColumns.design_spec}
      workItems={selectedIssueColumns.work_item}
      workItemRepositoryGroups={focusedWorkItemRepositoryGroups}
      issueQueueGroups={issueQueueGroups}
      queueCollapsed={queueCollapsed}
      queueTotalCount={queueTotalCount}
      collapsedQueueGroups={collapsedQueueGroups}
      queueFilterText={queueFilterText}
      logicalCodebases={logicalCodebases}
      activeLogicalCodebaseId={activeLogicalCodebaseId}
      activeLogicalCodebaseName={activeLogicalCodebaseName}
      lcSummaryExpanded={lcSummaryExpandedForProject}
      lcSummaryHasWarning={lcSummaryHasWarning}
      logicalCodebaseMembers={logicalCodebaseMembers}
      aggregateInitialization={aggregateInitialization}
      aggregateInitializationBusy={aggregateInitializationBusy}
      aggregateIndex={aggregateIndex}
      aggregateIndexRebuilding={aggregateIndexRebuilding}
      latestPointerPublication={latestPointerPublication}
      pointerPublicationBusy={pointerPublicationBusy}
      showIncrementalHint={showIncrementalHint}
      focusedEntity={focusedEntity}
      isDrawerOpen={isDrawerOpen}
      drawerWorkItems={focusedEntity ? lifecycles.find((lifecycle) => lifecycle.issue.issue_id === focusedEntity.issueId)?.work_items ?? [] : []}
      codingAttempts={focusedEntity ? lifecycles.find((lifecycle) => lifecycle.issue.issue_id === focusedEntity.issueId)?.coding_attempts ?? [] : []}
      deliverySummary={focusedEntity?.kind === "issue" ? lifecycles.find((lifecycle) => lifecycle.issue.issue_id === focusedEntity.issueId)?.delivery_summary : undefined}
      pendingWorkItemPlanLaunch={Boolean(pendingWorkItemPlanLaunch)}
      onSelectProject={(projectId) => void handleSelectProject(projectId)}
      onCreateProject={() => setProjectDialogOpen(true)}
      onAddCodebase={() => setAddCodebaseDialogOpen(true)}
      onDeleteProject={(projectId) => void handleDeleteProject(projectId)}
      onDeleteRepository={(repositoryId) => void handleDeleteRepository(repositoryId)}
      onDeleteLogicalCodebase={(logicalCodebaseId) => void handleDeleteLogicalCodebase(logicalCodebaseId)}
      onShowAll={() => setFocusedIssueId(null)}
      onRefresh={() => void refresh()}
      onCreateIssue={() => setDialogOpen(true)}
      onToggleLcSummary={handleToggleLcSummary}
      onSelectLogicalCodebase={setSelectedLogicalCodebaseId}
      onOpenRegistration={() => setRegistrationDialogOpen(true)}
      onStartAggregateInitialization={() => void handleStartAggregateInitialization()}
      onCancelAggregateInitialization={() => void handleCancelAggregateInitialization()}
      onRebuildAggregateIndex={() => void handleRebuildAggregateIndex()}
      onPublishFull={() => void handlePublishFull()}
      onPublishIncremental={() => void handlePublishIncremental()}
      onRetryRepo={(memberRepoId) => void handleRetryRepo(memberRepoId)}
      onRevoke={() => void handleRevokePublication()}
      onToggleQueueCollapsed={handleToggleQueueCollapsed}
      onToggleQueueGroup={handleToggleQueueGroup}
      onQueueFilterTextChange={handleQueueFilterTextChange}
      onSelectIssue={handleSelectIssueFromQueue}
      onGenerateStorySpec={handleGenerateStorySpecFromQueue}
      onDeleteIssue={(issueId) => void handleDeleteIssue(issueId)}
      onShowMoreQueueGroup={handleShowMoreQueueGroup}
      deletingIssueId={deletingIssueId}
      onSelectCard={handleSelectCard}
      onOpenFullIssue={handleOpenFullIssue}
      onDeleteCard={handleDeleteLifecycleCard}
      onGenerateForStage={handleGenerateForStage}
      onCloseDrawer={closeDrawer}
      onOpenWorkspaceFromDrawer={() => void handleOpenWorkspaceFromDrawer(focusedEntity!)}
      onOpenCodingWorkspaceFromDrawer={() => void handleOpenCodingWorkspaceFromDrawer(focusedEntity!)}
      onGenerateNext={() => void handleGenerateNext(focusedEntity!)}
      onDeleteFromDrawer={() => handleDeleteLifecycleCardFromDrawer(focusedEntity!)}
      dialogs={dialogs}
    />
  );
}
