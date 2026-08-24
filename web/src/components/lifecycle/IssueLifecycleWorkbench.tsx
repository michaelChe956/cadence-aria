import { useEffect, useMemo, useRef, useState } from "react";
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
  generateDesignSpecs,
  generateStorySpecs,
  getIssueLifecycle,
  getRepositoryInitialization,
  prepareWorkItemPlan,
  listProductIssues,
  listProjects,
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
import { IssueLifecycleWorkbenchHeader } from "./IssueLifecycleWorkbenchHeader";
import { LogicalCodebaseManagementPanel } from "./LogicalCodebaseManagementPanel";
import { LogicalCodebaseSummaryBar } from "./LogicalCodebaseSummaryBar";
import { IssueLifecycleWorkbenchDrawer } from "./IssueLifecycleWorkbenchDrawer";
import { ProjectSidebar } from "./ProjectSidebar";
import {
  WorkItemPlanOptionsDialog,
  type WorkItemPlanOptionsFormValue,
} from "./WorkItemPlanOptionsDialog";
import { IssueQueue } from "./IssueQueue";
import {
  defaultCollapsedGroups,
  deriveIssueQueue,
  ISSUE_QUEUE_GROUP_ORDER,
  type IssueQueueGroupKey,
} from "./issue-queue-derivation";
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
  toDrawerEntity,
  waitForDeleteExitAnimation,
} from "./IssueLifecycleWorkbenchParts";
import type { WorkbenchStageKey } from "./StageStepper";
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

// 稳定空数组引用：避免每次 render 新建 [] 使 useMemo 依赖失效。
const EMPTY_GROUP_KEYS: IssueQueueGroupKey[] = [];

// Task 7：双密度外壳的持久化键（按 projectId 记忆）。
function queueCollapsedStorageKey(projectId: string) {
  return `aria.workbench.queueCollapsed.${projectId}`;
}

function queueGroupsStorageKey(projectId: string) {
  return `aria.workbench.groups.${projectId}`;
}

// Task 8：运维摘要条展开状态的持久化键（按 projectId 记忆）。
function lcSummaryStorageKey(projectId: string) {
  return `aria.workbench.lcSummary.${projectId}`;
}

// localStorage 不可用（隐私模式/配额超限）时静默降级为仅内存记忆。
function readStoredValue(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

function writeStoredValue(key: string, value: string) {
  try {
    window.localStorage.setItem(key, value);
  } catch {
    /* localStorage 不可用：静默降级 */
  }
}

function readStoredQueueCollapsed(projectId: string): boolean {
  return readStoredValue(queueCollapsedStorageKey(projectId)) === "1";
}

// Task 8：摘要条默认折叠——仅显式写入 "1" 才回填为展开。
function readStoredLcSummaryExpanded(projectId: string): boolean {
  return readStoredValue(lcSummaryStorageKey(projectId)) === "1";
}

function readStoredCollapsedGroups(projectId: string): IssueQueueGroupKey[] {
  const raw = readStoredValue(queueGroupsStorageKey(projectId));
  if (raw === null) {
    return defaultCollapsedGroups();
  }
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) {
      return defaultCollapsedGroups();
    }
    return parsed.filter((value): value is IssueQueueGroupKey =>
      ISSUE_QUEUE_GROUP_ORDER.includes(value as IssueQueueGroupKey),
    );
  } catch {
    return defaultCollapsedGroups();
  }
}
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
        const nextKey = lifecycleEntityKey("design_spec", card.issueId, nextId);
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

  // Task 6：阶段工作区空阶段主按钮接线——复用现有生成链路，不新增 API：
  // story -> 用当前 Issue 卡走 handleLaunchWorkspace("story")；
  // design -> 用最新 Story 卡走 handleGenerateNext（内部走 generateDesignSpecs）；
  // work_item -> 用最新 Design 卡走 handleGenerateNext（打开 Work Item Plan 配置弹窗）。
  function handleGenerateForStage(stage: WorkbenchStageKey) {
    if (stage === "story") {
      const issueCard = selectedIssueColumns.issue[0];
      if (!issueCard) {
        setError("缺少 Issue");
        return;
      }
      void handleLaunchWorkspace("story", issueCard);
      return;
    }

    const sourceCard =
      stage === "design"
        ? selectedIssueColumns.story_spec.at(-1)
        : selectedIssueColumns.design_spec.at(-1);
    if (!sourceCard) {
      setError(stage === "design" ? "缺少 Story Spec" : "缺少 Design Spec");
      return;
    }
    void handleGenerateNext(sourceCard);
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
    if (
      !selectedProjectId ||
      !activeLogicalCodebaseId ||
      !aggregateInitialization
    ) {
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
    const response = await prepareWorkItemPlan(
      selectedProjectId,
      card.issueId,
      {
        title: defaultLaunchTitle({ target: "work_item", card }),
        story_spec_ids: card.raw.story_spec_ids,
        design_spec_ids: [card.id],
        ...options,
      },
    );
    await refresh(selectedProjectId);
    setPendingWorkItemPlanLaunch(null);
    onOpenWorkspace(response.workspace_session.workspace_session_id);
  }

  return (
    <>
      <div
        data-testid="workbench-shell"
        className="grid h-[100dvh] min-h-0 bg-[var(--aria-bg)] text-[var(--aria-ink)] lg:grid-cols-[17rem_minmax(0,1fr)]"
      >
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
              canCreateIssue={
                Boolean(selectedProjectId) && repositories.length > 0
              }
              onShowAll={() => setFocusedIssueId(null)}
              onRefresh={() => void refresh()}
              onCreateIssue={() => setDialogOpen(true)}
            />
          }
          main={
            <div className="space-y-3">
              {selectedProjectId && logicalCodebases.length > 0 ? (
                <div className="space-y-2">
                  {/* Task 8：默认仅一行摘要条；展开后原样渲染既有管理面板（面板内部零改动）。 */}
                  <LogicalCodebaseSummaryBar
                    summary={{
                      lcName: activeLogicalCodebaseName,
                      indexState: aggregateIndex?.state ?? null,
                      publicationStatus:
                        latestPointerPublication?.status ?? null,
                      hasWarning: lcSummaryHasWarning,
                    }}
                    expanded={lcSummaryExpandedForProject}
                    onToggle={handleToggleLcSummary}
                  />
                  {lcSummaryExpandedForProject ? (
                    <LogicalCodebaseManagementPanel
                      logicalCodebases={logicalCodebases}
                      activeLogicalCodebaseId={activeLogicalCodebaseId}
                      onSelectLogicalCodebase={setSelectedLogicalCodebaseId}
                      onOpenRegistration={() => setRegistrationDialogOpen(true)}
                      logicalCodebaseMembers={logicalCodebaseMembers}
                      aggregateInitialization={aggregateInitialization}
                      aggregateInitializationBusy={aggregateInitializationBusy}
                      onStartAggregateInitialization={() =>
                        void handleStartAggregateInitialization()
                      }
                      onCancelAggregateInitialization={() =>
                        void handleCancelAggregateInitialization()
                      }
                      aggregateIndex={aggregateIndex}
                      aggregateIndexRebuilding={aggregateIndexRebuilding}
                      onRebuildAggregateIndex={() =>
                        void handleRebuildAggregateIndex()
                      }
                      latestPointerPublication={latestPointerPublication}
                      pointerPublicationBusy={pointerPublicationBusy}
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
                  ) : null}
                </div>
              ) : null}
              {/* Task 7：双密度布局——队列固定 w-72（折叠为 w-10 细轨），工作区弹性充满；
                  两侧各自 min-h-0 + 内部 overflow-y-auto，不产生页面级双滚动条。 */}
              <div className="flex h-[calc(100dvh-6rem)] min-h-0 gap-3">
                {queueCollapsed ? (
                  <div
                    data-testid="issue-queue-collapsed-rail"
                    className="flex w-10 shrink-0 flex-col items-center gap-2 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] py-2"
                  >
                    <button
                      type="button"
                      aria-label="展开 Issue 队列"
                      aria-expanded={false}
                      onClick={handleToggleQueueCollapsed}
                      className="inline-flex h-7 w-7 shrink-0 cursor-pointer items-center justify-center rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] text-[var(--aria-ink-muted)] transition-colors duration-200 hover:border-[var(--aria-primary)] hover:text-[var(--aria-primary)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
                    >
                      <PanelLeftOpen className="h-4 w-4" />
                    </button>
                    <span
                      data-testid="issue-queue-rail-count"
                      className="shrink-0 rounded border border-[var(--aria-line)] bg-[var(--aria-panel)] px-1 py-0.5 font-mono text-[11px] text-[var(--aria-ink-muted)]"
                    >
                      {queueTotalCount}
                    </span>
                  </div>
                ) : (
                  <div
                    data-testid="issue-queue-column"
                    className="grid w-72 min-h-0 shrink-0 grid-rows-[auto_minmax(0,1fr)] gap-2"
                  >
                    <button
                      type="button"
                      aria-label="折叠 Issue 队列"
                      aria-expanded
                      onClick={handleToggleQueueCollapsed}
                      className="inline-flex h-7 shrink-0 cursor-pointer items-center justify-center gap-1.5 self-start rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-2 text-[11px] font-semibold text-[var(--aria-ink-muted)] transition-colors duration-200 hover:border-[var(--aria-primary)] hover:text-[var(--aria-primary)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
                    >
                      <PanelLeftClose className="h-3.5 w-3.5" />
                      折叠队列
                    </button>
                    <IssueQueue
                      groups={issueQueueGroups}
                      focusedIssueId={focusedIssueId}
                      collapsedGroups={collapsedQueueGroups}
                      onToggleGroup={handleToggleQueueGroup}
                      filterText={queueFilterText}
                      onFilterTextChange={handleQueueFilterTextChange}
                      onSelectIssue={handleSelectIssueFromQueue}
                      onGenerateStorySpec={handleGenerateStorySpecFromQueue}
                      onDeleteIssue={(issueId) =>
                        void handleDeleteIssue(issueId)
                      }
                      onShowMoreGroup={handleShowMoreQueueGroup}
                      deletingIssueId={deletingIssueId}
                    />
                  </div>
                )}
                {/* flex 行容器默认 align-items:stretch：工作区得到确定高度，其内部
                    overflow-y-auto 才能生效（而非把页面撑高）。 */}
                <div className="flex min-h-0 min-w-0 flex-1">
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
                    onGenerateForStage={handleGenerateForStage}
                    deletingKey={deletingCardKey}
                  />
                </div>
              </div>
            </div>
          }
        />
      </div>
      {isDrawerOpen && focusedEntity ? (
        <IssueLifecycleWorkbenchDrawer
          focusedEntity={focusedEntity}
          workItems={
            lifecycles.find(
              (lifecycle) => lifecycle.issue.issue_id === focusedEntity.issueId,
            )?.work_items ?? []
          }
          codingAttempts={
            lifecycles.find(
              (lifecycle) => lifecycle.issue.issue_id === focusedEntity.issueId,
            )?.coding_attempts ?? []
          }
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
          onOpenCodingWorkspace={() =>
            void handleOpenCodingWorkspaceFromDrawer(focusedEntity)
          }
          onGenerateNext={() => void handleGenerateNext(focusedEntity)}
          onDelete={() => handleDeleteLifecycleCardFromDrawer(focusedEntity)}
        />
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
