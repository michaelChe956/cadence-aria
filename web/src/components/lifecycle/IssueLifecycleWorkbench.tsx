import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { PanelLeftClose, PanelLeftOpen } from "lucide-react";
import {
  createCodingAttempt,
  createGroupCodingAttempt,
  createProject,
  createProductIssue,
  createRepository,
  deleteProject,
  deleteRepository,
  getIssueLifecycle,
  getPlanCodingProgress,
  getRepositoryInitialization,
  listProductIssues,
  listProjects,
  listRepositoryBranches,
  listRepositories,
  startPlanCoding,
} from "../../api/client";
import { rebuildAggregateIndex } from "../../api/aggregate-index";
import {
  cancelAggregateInitialization,
  startAggregateInitialization,
} from "../../api/aggregate-initialization";
import { listLogicalCodebaseMembers } from "../../api/logicalCodebaseMembers";
import {
  deleteLogicalCodebase,
  listCodebases,
  listLogicalCodebases,
} from "../../api/codebases";
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
  LogicalCodebaseListEntryDto,
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
import { useWorkbenchOverlayDirectory } from "../../state/workbench-overlay-store";
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
  lifecycleWorkItemCard,
  normalizeLifecycleResponse,
  resolveGroupCodingAttempt,
  selectedLifecycleColumns,
} from "./IssueLifecycleWorkbenchParts";
import { useIssueLifecycleWorkbenchDeletion } from "./useIssueLifecycleWorkbenchDeletion";
import { IssueLifecycleWorkbenchView } from "./IssueLifecycleWorkbenchView";
export { defaultLaunchTitle } from "./IssueLifecycleWorkbenchParts";

// 稳定空数组引用：避免每次 render 新建 [] 使 useMemo 依赖失效。
const EMPTY_GROUP_KEYS: IssueQueueGroupKey[] = [];

// N6（REQ-CS-01）：多仓 plan 判定——plan 的 work items 落在 ≥2 个非兼容投影
// target 仓时走新一键 coding-start；单仓/无分仓投影保持 v1 旧路径不变。
function countPlanTargetRepositories(
  lifecycle: IssueLifecycleResponse | undefined,
  planWorkItemIds: readonly string[],
): number {
  if (!lifecycle) {
    return 0;
  }
  const planItems = new Set(planWorkItemIds);
  const targetIds = new Set<string>();
  for (const group of lifecycle.work_item_repository_groups) {
    if (!group.target_repository_id) {
      continue;
    }
    if (group.items.some((item) => planItems.has(item.work_item_id))) {
      targetIds.add(group.target_repository_id);
    }
  }
  return targetIds.size;
}

// 一次用户启动动作生成一次，同动作失败重试复用（后端按 command_id 幂等）。
function newPlanCodingCommandId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return `plan_start_ui_${crypto.randomUUID()}`;
  }
  return `plan_start_ui_${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}

export function IssueLifecycleWorkbench({
  focusEntityKey,
  onDrawerFocusChange,
  onOpenWorkspace = defaultOpenWorkspace,
  onOpenCodingWorkspace = defaultOpenCodingWorkspace,
  onReopenOnboarding,
}: {
  focusEntityKey?: string | null;
  onDrawerFocusChange?: (entityKey: string | null) => void;
  onOpenWorkspace?: (sessionId: string) => void;
  onOpenCodingWorkspace?: (address: CodingAttemptAddress) => void;
  /** UI-4：引导重开入口接线；由 AppShell 的唯一 useOnboarding 实例提供。 */
  onReopenOnboarding?: () => void;
}) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [repositories, setRepositories] = useState<Repository[]>([]);
  const [codebases, setCodebases] = useState<CodebaseSummaryDto[]>([]);
  // 2026-10-29：已软删（墓碑）LC——「已删除」分区的数据源（含 deleted_at）。
  const [deletedLogicalCodebases, setDeletedLogicalCodebases] = useState<
    LogicalCodebaseListEntryDto[]
  >([]);
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
  // N6：多仓 plan 一键启动的幂等 command（同动作重试复用）与在途防抖。
  const planCodingStartRef = useRef<{ planId: string; commandId: string } | null>(null);
  const planCodingStartInFlightRef = useRef(false);
  // H5（设计 C）：plan 抽屉辅入口整组删除的幂等 command 与在途防抖。
  const planGroupDeleteRef = useRef<{ planId: string; commandId: string } | null>(null);
  const [groupCodingDeleteBusy, setGroupCodingDeleteBusy] = useState(false);
  const drawerFocusedEntityKey = useLifecycleWorkbenchStore(
    (state) => state.focusedEntityKey,
  );
  const isDrawerOpen = useLifecycleWorkbenchStore(
    (state) => state.isDrawerOpen,
  );
  const openDrawer = useLifecycleWorkbenchStore((state) => state.openDrawer);
  const closeDrawer = useLifecycleWorkbenchStore((state) => state.closeDrawer);
  // UI-1：把已加载 lifecycle 目录同步进 overlay 地址簿——/workbench 抽屉按
  // planId/attemptId 解析 (projectId, issueId)；入口种子优先，此处同值覆盖
  // 不改变语义（见 workbench-overlay-store 头注释）。
  const syncWorkbenchOverlayDirectory = useWorkbenchOverlayDirectory(
    (state) => state.syncWorkbenchOverlayDirectory,
  );
  useEffect(() => {
    syncWorkbenchOverlayDirectory(lifecycles);
  }, [lifecycles, syncWorkbenchOverlayDirectory]);

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
        setDeletedLogicalCodebases([]);
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

      const [
        repositoryResponse,
        codebaseResponse,
        logicalCodebaseResponse,
        issueResponse,
      ] = await Promise.all([
        listRepositories(projectId),
        listCodebases(projectId),
        listLogicalCodebases(projectId, { includeDeleted: true }),
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
      // 存活 LC 仍在 /codebases 投影里（顺序与选择语义不变），这里只取墓碑条目。
      setDeletedLogicalCodebases(
        (logicalCodebaseResponse.logical_codebases ?? []).filter(
          (codebase) => codebase.deleted,
        ),
      );
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
  // B1（用户裁决 A，2026-10-10）：选中 issue 的交付状态聚合——详情区常驻
  // 面板数据源（与抽屉 deliverySummary 分开派生：抽屉锚定 drawer 实体，
  // 此处锚定当前选中 issue，抽屉关闭/残留 drawer key 时不串台）。
  const selectedIssueDeliverySummary = useMemo(
    () =>
      focusedIssueIdForGroups
        ? lifecycles.find(
            (lifecycle) => lifecycle.issue.issue_id === focusedIssueIdForGroups,
          )?.delivery_summary
        : undefined,
    [lifecycles, focusedIssueIdForGroups],
  );
  const focusedEntity = useMemo(() => {
    // 展示列只包含计划组卡；分仓卡片和深链接的单个 Work Item 从完整实体中解析。
    if (drawerFocusedEntityKey?.startsWith("work_item:")) {
      for (const lifecycle of lifecycles) {
        const item = lifecycle.work_items.find(
          (candidate) =>
            lifecycleEntityKey("work_item", candidate.issue_id, candidate.work_item_id) ===
            drawerFocusedEntityKey,
        );
        if (item) {
          return lifecycleWorkItemCard(item);
        }
      }
      return null;
    }
    return findCardInColumns(allColumns, drawerFocusedEntityKey);
  }, [allColumns, drawerFocusedEntityKey, lifecycles]);
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
    bootstrapProjection,
    refreshBootstrap,
    refreshScope,
    autoAdvanceBootstrap,
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
    handleRestoreLogicalCodebase,
    handlePurgeLogicalCodebase,
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

  // Task 8：展开/折叠仅切换运维面板可见性——不动选中态（bootstrap 补读见下方 effect）。
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

  // 冷启动/运维抽屉每次展开（管理面板挂载）时补读 bootstrap 投影：后端
  // 推进可能发生在抽屉关闭期间（如登记批内完成 identity/manifest_checkout），
  // 展开时对齐后端当前状态；稳定态不轮询，等显式动作/事件唤醒。
  useEffect(() => {
    if (lcSummaryExpandedForProject) {
      refreshBootstrap();
    }
  }, [lcSummaryExpandedForProject, refreshBootstrap]);

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

    // N6（REQ-CS-01）：多仓 confirmed plan 的一键启动——先 GET 聚合取 bound
    // revision 互证，再单发 POST coding-start，打开 selected（否则 primary，仅
    // 投影入口）workspace。请求中禁重复出站；同一启动动作失败重试复用同一
    // command_id。v1 单仓/无分仓投影 plan 保持旧创建端点不变。
    if (
      countPlanTargetRepositories(lifecycle, card.childWorkItemIds) >= 2 &&
      !planCodingStartInFlightRef.current
    ) {
      planCodingStartInFlightRef.current = true;
      setError(null);
      try {
        const progress = await getPlanCodingProgress(
          selectedProjectId,
          card.issueId,
          card.id,
        );
        const previous = planCodingStartRef.current;
        const commandId =
          previous && previous.planId === card.id
            ? previous.commandId
            : newPlanCodingCommandId();
        planCodingStartRef.current = { planId: card.id, commandId };
        const result = await startPlanCoding(
          selectedProjectId,
          card.issueId,
          card.id,
          {
            command_id: commandId,
            expected_plan_revision_id: progress.bound_plan_revision_id,
          },
        );
        const openAttemptId =
          result.selected_attempt_id ?? result.primary_attempt_id;
        if (!openAttemptId) {
          setError("尚无 ready 首项，plan 已绑定等待；请稍后刷新查看进度");
          return;
        }
        await refresh(selectedProjectId);
        onOpenCodingWorkspace({
          projectId: selectedProjectId,
          issueId: card.issueId,
          attemptId: openAttemptId,
        });
      } catch (reason) {
        setError(errorMessage(reason, "Plan Coding 启动失败"));
      } finally {
        planCodingStartInFlightRef.current = false;
      }
      return;
    }

    if (planCodingStartInFlightRef.current) {
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
      // REQ-MRE-01：LC 勾选集透传；单仓 null → undefined（key 不进请求体，
      // 服务端维持存量语义）。
      focus_repository_ids:
        payload.focus_repository_ids && payload.focus_repository_ids.length > 0
          ? payload.focus_repository_ids
          : undefined,
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


  const {
    handleDeleteIssue,
    handleDeleteLifecycleCard,
    handleDeleteLifecycleCardFromDrawer,
    handleDeleteAllCodingWorkspacesFromDrawer,
  } = useIssueLifecycleWorkbenchDeletion({
    selectedProjectId,
    focusedIssueId,
    selectedCardKey,
    drawerFocusedEntityKey,
    groupCodingDeleteBusy,
    planGroupDeleteRef,
    setError,
    setFocusedIssueId,
    setSelectedCardKey,
    setDeletingCardKey,
    setGroupCodingDeleteBusy,
    closeDrawer,
    refresh,
  });

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
          onCreatedLogical={() => {
            // 合并流：LC 创建后登记在弹窗内完成，这里只刷新列表（不再打开独立登记向导）。
            if (selectedProjectId) void refresh(selectedProjectId);
          }}
          onRegistrationCompleted={(logicalCodebaseId) => {
            // 登记批在后端落成员并推进 identity/manifest_checkout：目录与
            // LC 作用域（成员/发布/索引 + bootstrap 投影）都要重拉，否则
            // 管理抽屉沿用登记前的陈旧冷启动快照。
            void refresh(selectedProjectId);
            refreshScope();
            // 冷启动自动推进：登记完成后按依赖顺序推进剩余步骤（每个 LC
            // 只自动调一次；失败只提示不重试，可手动「一键推进」补）。
            void autoAdvanceBootstrap(logicalCodebaseId).then((message) => {
              if (message) {
                setError(`冷启动自动推进失败：${message}`);
              }
            });
          }}
          onClose={() => setAddCodebaseDialogOpen(false)}
        />
      ) : null}
      {registrationDialogOpen &&
      selectedProjectId &&
      activeLogicalCodebaseId ? (
        <LogicalCodebaseRegistrationWizard
          projectId={selectedProjectId}
          logicalCodebaseId={activeLogicalCodebaseId}
          onCompleted={(logicalCodebaseId) => {
            // 独立登记向导完成：与合并流同口径——目录 + LC 作用域重拉。
            void refresh(selectedProjectId);
            refreshScope();
            // 冷启动自动推进（与合并流同口径）。
            void autoAdvanceBootstrap(logicalCodebaseId).then((message) => {
              if (message) {
                setError(`冷启动自动推进失败：${message}`);
              }
            });
          }}
          onClose={() => setRegistrationDialogOpen(false)}
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
          automationAvailable={
            pendingWorkItemPlanLaunch.card.kind === "design_spec" &&
            pendingWorkItemPlanLaunch.card.raw.confirmation_status ===
              "confirmed"
          }
          automationTargetScope={
            selectedProjectId &&
            pendingWorkItemPlanLaunch.card.kind === "design_spec"
              ? {
                  projectId: selectedProjectId,
                  issueId: pendingWorkItemPlanLaunch.card.issueId,
                }
              : null
          }
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
      deletedLogicalCodebases={deletedLogicalCodebases}
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
      bootstrapProjection={bootstrapProjection}
      onBootstrapChanged={refreshBootstrap}
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
      selectedIssueDeliverySummary={selectedIssueDeliverySummary}
      pendingWorkItemPlanLaunch={Boolean(pendingWorkItemPlanLaunch)}
      onSelectProject={(projectId) => void handleSelectProject(projectId)}
      onCreateProject={() => setProjectDialogOpen(true)}
      onAddCodebase={() => setAddCodebaseDialogOpen(true)}
      onDeleteProject={(projectId) => void handleDeleteProject(projectId)}
      onDeleteRepository={(repositoryId) => void handleDeleteRepository(repositoryId)}
      onDeleteLogicalCodebase={(logicalCodebaseId) => void handleDeleteLogicalCodebase(logicalCodebaseId)}
      onRestoreLogicalCodebase={(logicalCodebaseId) => void handleRestoreLogicalCodebase(logicalCodebaseId)}
      onPurgeLogicalCodebase={(logicalCodebaseId) => void handlePurgeLogicalCodebase(logicalCodebaseId)}
      onShowAll={() => setFocusedIssueId(null)}
      onRefresh={() => void refresh()}
      onCreateIssue={() => setDialogOpen(true)}
      onReopenOnboarding={onReopenOnboarding}
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
      onDeleteAllCodingWorkspacesFromDrawer={() =>
        void handleDeleteAllCodingWorkspacesFromDrawer(focusedEntity!)
      }
      groupCodingDeleteBusy={groupCodingDeleteBusy}
      dialogs={dialogs}
    />
  );
}
