import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  getIssueLifecycle,
  listProductIssues,
  listProjects,
} from "../api/client";
import { listCodebases } from "../api/codebases";
import { getLogicalCodebaseBootstrap } from "../api/logical-codebase-bootstrap";
import {
  listProjectRepositoryInitializationWaitingItems,
} from "../api/repository-initialization";
import type {
  CodingAttempt,
  CodingFinalConfirmInfoItem,
  IssueLifecycleResponse,
  PlanConfirmedInfoItem,
  ProductIssueListResponse,
  WorkspaceSessionSummary,
} from "../api/types";
import type { CodebaseSummaryDto } from "../api/types/codebases";
import {
  c1WaitingItem,
  logicalCodebaseBootstrapItem,
} from "../state/workspace-cockpit-projection";
import {
  WATCHED_SESSION_STATUSES,
  createObserverController,
  selectObservedInbox,
  selectWatchedSessionIds,
  type WorkspaceObserverController,
  type WorkspaceObserverRecord,
} from "../state/workspace-observer-store";
import {
  codingFinalConfirmInfoItem,
  planConfirmedInfoItem,
  type CockpitInboxItem,
} from "../state/workspace-cockpit-projection";
import {
  codingAttentionIdentity,
  codingAttentionItem,
} from "../state/workspace-cockpit-projection";
import {
  INFO_TTL_MS,
  RECENT_COMPLETION_WINDOW_MS,
  isRecentCompletionVisible,
  recentCompletionIdentity,
  recentCompletionInboxItem,
} from "../state/recent-completion";
import { subscribeToLifecycleInvalidation } from "../state/lifecycle-workbench-store";
import type { WorkspaceWsState } from "../state/workspace-ws-store";

export type CatalogRefreshTimer = number;

/** P3（REQ-WIGA-07）：invalidation/snapshot hint/轮询汇入的 trailing debounce
 * ——只合并唤醒节流，不改变周期轮询节奏。 */
const CATALOG_REFRESH_DEBOUNCE_MS = 250;
/** P3（REQ-WIGA-07）：观察者请求的服务端近期目录上界（1..=32）。 */
const RECENT_COMPLETION_REQUEST_LIMIT = 32;
/** P2（无限轮询修复）：dirty 快路径补跑的最小间隔（ms）。取 1s：远小于
 * 最小轮询档 5s（真实失效唤醒不被吞），又大于 WS 高频帧的典型突发节奏，
 * 足以打断「轮 → dirty → 轮」的零间隔连续档。 */
const DIRTY_RERUN_MIN_DELAY_MS = 1_000;
/** P3（无限轮询修复）：空闲退避档——无进行中会话/待处理事实时，周期轮询
 * 退到的最大档（与设置面板 60s 同档；按退避间隔处理，不改用户设置值）。 */
const IDLE_BACKOFF_INTERVAL_MS = 60_000;

const scheduleCatalogRefresh = (callback: () => void, delayMs: number): CatalogRefreshTimer =>
  window.setTimeout(callback, delayMs);
const cancelCatalogRefresh = (timer: CatalogRefreshTimer): void => window.clearTimeout(timer);

export interface WorkspaceSessionObserverOptions {
  currentSessionId: string | null;
  currentSessionState?: WorkspaceWsState | null;
  watchLimit: number;
  refreshIntervalMs: number;
  listProjects?: typeof listProjects;
  listProductIssues?: (projectId: string) => Promise<ProductIssueListResponse>;
  getIssueLifecycle?: (
    issueId: string,
    projectId: string,
    options?: { recentSince: string; recentLimit?: number },
  ) => Promise<
    Pick<
      IssueLifecycleResponse,
      | "workspace_sessions"
      | "coding_attempts"
      | "plan_confirmed_info"
      | "coding_final_confirm_info"
      | "recent_completion_info"
      | "c1_waiting_items"
      | "coding_attention_items"
    >
  >;
  /** C4 Task 9：LC 目录读取（默认真实 API；测试注入伪实现）。 */
  listCodebases?: typeof listCodebases;
  /** C4 Task 9：bootstrap 纯投影读取（默认真实 API；测试注入伪实现）。 */
  getLogicalCodebaseBootstrap?: typeof getLogicalCodebaseBootstrap;
  /** C5 Task 6：project 级 repository 初始化失败等待项读取（默认真实
   * API；测试注入伪实现；失败保留上一轮 durable 快照）。 */
  listProjectRepositoryInitializationWaitingItems?: typeof listProjectRepositoryInitializationWaitingItems;
  createController?: WorkspaceObserverControllerFactory;
  scheduleCatalogRefresh?: (callback: () => void, delayMs: number) => CatalogRefreshTimer;
  cancelCatalogRefresh?: (timer: CatalogRefreshTimer) => void;
}

export type WorkspaceObserverControllerFactory = (
  onRecordsChange: (records: readonly WorkspaceObserverRecord[]) => void,
  onSnapshotHint?: () => void,
) => WorkspaceObserverController;

export interface WorkspaceSessionObserverResult {
  records: readonly WorkspaceObserverRecord[];
  inbox: readonly CockpitInboxItem[];
  watchedSessionIds: readonly string[];
  countedInbox: readonly CockpitInboxItem[];
  watchSession(sessionId: string): void;
  /** Task 11：会话所属 issue 的最新活跃 coding attempt（无则 null）。 */
  codingAttemptForSession(sessionId: string): CodingAttempt | null;
  /** P3（REQ-WIGA-07）：展示收件箱 = 可操作条目 + 可见完成 info（`inbox`
   * 保留为同义别名，兼容既有调用点）。 */
  displayItems: readonly CockpitInboxItem[];
  /** P3（REQ-WIGA-07）：可操作（gate/choice/stopped/error/sc_failed）计数；
   * info 恒不计数。 */
  actionableCount: number;
  /** P3（REQ-WIGA-07）：首次成功 hydration 之后新增的稳定身份完成事实
   * （提示候选；once/TTL 绑定在 Task 3 落地）。 */
  notificationCandidates: readonly CockpitInboxItem[];
}


export function useWorkspaceSessionObservers(options: WorkspaceSessionObserverOptions): WorkspaceSessionObserverResult {
  const {
    currentSessionId,
    currentSessionState = null,
    watchLimit,
    refreshIntervalMs,
    listProjects: getProjects = listProjects,
    listProductIssues: getProductIssues = listProductIssues,
    getIssueLifecycle: getLifecycle = getIssueLifecycle,
    listCodebases: getCodebases = listCodebases,
    getLogicalCodebaseBootstrap: getBootstrap = getLogicalCodebaseBootstrap,
    listProjectRepositoryInitializationWaitingItems: getProjectWaitingItems =
      listProjectRepositoryInitializationWaitingItems,
    createController,
    scheduleCatalogRefresh: scheduleRefresh = scheduleCatalogRefresh,
    cancelCatalogRefresh: cancelRefresh = cancelCatalogRefresh,
  } = options;
  const [sessions, setSessions] = useState<readonly WorkspaceSessionSummary[]>([]);
  const [observerRecords, setObserverRecords] = useState<
    readonly WorkspaceObserverRecord[]
  >([]);
  const [extraSessionIds, setExtraSessionIds] = useState<readonly string[]>([]);
  const [codingAttempts, setCodingAttempts] = useState<readonly CodingAttempt[]>([]);
  const [planConfirmedInfos, setPlanConfirmedInfos] = useState<readonly PlanConfirmedInfoItem[]>(
    [],
  );
  // P2 WIGA Task 9（REQ-WIGA-07/R5）：coding FinalConfirm info 携带其 issue 的
  // 会话集合（过滤 watched 用），不按 attempt id 伪造 sessionId。
  const [codingFinalConfirmInfos, setCodingFinalConfirmInfos] = useState<
    readonly { info: CodingFinalConfirmInfoItem; sessionIds: readonly string[] }[]
  >([]);
  // P3（REQ-WIGA-07）：K 外近期完成目录条目（含 completionIdentity）与首次
  // hydration 之后的提示候选。
  const [recentCompletionItems, setRecentCompletionItems] = useState<
    readonly CockpitInboxItem[]
  >([]);
  const [notificationCandidates, setNotificationCandidates] = useState<
    readonly CockpitInboxItem[]
  >([]);
  // P4-d（REQ-UI37-19）：durable 待处理事项全局目录（全部 lifecycle 条目，
  // 无 watched/TTL/上限过滤）——进 display/count，按 [project,issue,key]
  // 身份去重；事实关闭后随下一轮目录消失。
  const [codingAttentionItems, setCodingAttentionItems] = useState<
    readonly CockpitInboxItem[]
  >([]);
  // C1 Task 9（enrollment-recovery-surface）：issue 级 durable 恢复等待项
  // （孤儿候选/lease 三态/Failed advance/intent 停等/换代历史）——只读补读
  // 投影，动作经页面接线的 C1 REST 发送器出站。
  const [c1WaitingItems, setC1WaitingItems] = useState<readonly CockpitInboxItem[]>(
    [],
  );
  // C4 Task 9（LC 冷启动加固）：LC 级 durable 冷启动等待/失败通知——与目录
  // 同源补读（listCodebases → bootstrap GET），按稳定 notice key 去重，
  // 不进入错误计数；动作经页面接线的 bootstrap action REST 出站。
  const [lcBootstrapItems, setLcBootstrapItems] = useState<readonly CockpitInboxItem[]>(
    [],
  );
  // C5 Task 6：project 级 repository 初始化失败等待项（无 issue 归属，
  // id 即后端稳定 id）——与 issue 级等待项同源合并展示；读取失败时保留
  // 上一轮 durable 快照，不清空既有等待项。
  const [projectC1WaitingItems, setProjectC1WaitingItems] = useState<
    readonly CockpitInboxItem[]
  >([]);
  // P3（REQ-WIGA-07）：到期 tick——最近到期单次失效定时器触发后递增，
  // 使可见投影在无 REST 的情况下剔除过期 info。
  const [recentExpiryTick, setRecentExpiryTick] = useState(0);
  // 首次成功 hydration（含空目录）建立的稳定身份基线；失败不置基线。
  // hydration 在一轮补读静默收敛（无 pending dirty）后才关闭，初始 GET
  // 在途期间的失效唤醒归入同一静默批次。
  const knownRecentIdentitiesRef = useRef<Set<string> | null>(null);
  // P4-d（REQ-UI37-19）：已见过的待处理事项身份（提醒每 key 恰一次）。
  const knownAttentionIdentitiesRef = useRef<Set<string> | null>(null);
  const hydrationClosedRef = useRef(false);
  // C5 Task 6：上一轮 project 等待项 durable 快照（GET 失败时保留）。
  const projectC1ItemsRef = useRef<readonly CockpitInboxItem[]>([]);
  // P3（无限轮询修复）：最近一轮目录是否存在进行中事实（活跃会话/等待项/
  // LC 通知/待处理事项）——决定下一轮周期用用户档还是空闲退避档；初始
  // true（未见事实前保守维持用户节奏），失败轮保留上一已知值。
  const hasActiveFactsRef = useRef(true);
  const snapshotHintRef = useRef<() => void>(() => undefined);
  const controllerRef = useRef<WorkspaceObserverController | null>(null);

  if (controllerRef.current === null) {
    controllerRef.current = createController
      ? createController(setObserverRecords, () => snapshotHintRef.current())
      : createObserverController(undefined, setObserverRecords, {
          refreshIntervalMs,
          reconnectDelayMs: 1_000,
          onSnapshotHint: () => snapshotHintRef.current(),
        });
  }

  const watchedSessionIds = useMemo(
    () =>
      Array.from(
        new Set([
          ...selectWatchedSessionIds(sessions, watchLimit),
          ...extraSessionIds,
        ]),
      ),
    [extraSessionIds, sessions, watchLimit],
  );
  const observedSessionIds = useMemo(
    () => watchedSessionIds.filter((sessionId) => sessionId !== currentSessionId),
    [currentSessionId, watchedSessionIds],
  );
  const records = useMemo(() => {
    const observed = observerRecords.filter((record) =>
      watchedSessionIds.includes(record.sessionId),
    );
    if (currentSessionId !== null && currentSessionState !== null) {
      return [{ sessionId: currentSessionId, state: currentSessionState }, ...observed];
    }
    return observed;
  }, [currentSessionId, currentSessionState, observerRecords, watchedSessionIds]);

  // P1 WIGA Task 9（REQ-WIGA-07）：只读 plan 确认 info——只保留当前 watched
  // session 的条目，按 durable key 去重；不进 countedInbox（计数/批量隔离）。
  const infoItems = useMemo(() => {
    const byKey = new Map<string, CockpitInboxItem>();
    for (const info of planConfirmedInfos) {
      if (!watchedSessionIds.includes(info.session_id) || byKey.has(info.key)) {
        continue;
      }
      byKey.set(info.key, planConfirmedInfoItem(info));
    }
    return Array.from(byKey.values());
  }, [planConfirmedInfos, watchedSessionIds]);
  // coding info 只投影 watched session 所属 issue 的完成事实，按稳定 key
  // 去重；同样不进 countedInbox（与 plan info 同一隔离规则）。
  const codingInfoItems = useMemo(() => {
    const byKey = new Map<string, CockpitInboxItem>();
    for (const { info, sessionIds } of codingFinalConfirmInfos) {
      const watched = sessionIds.some((sessionId) =>
        watchedSessionIds.includes(sessionId),
      );
      if (!watched || byKey.has(info.key)) {
        continue;
      }
      byKey.set(info.key, codingFinalConfirmInfoItem(info));
    }
    return Array.from(byKey.values());
  }, [codingFinalConfirmInfos, watchedSessionIds]);
  // P3（REQ-WIGA-07）：TTL 内可见的近期完成事实——未来/坏时间 fail-closed
  // 不展示，已到期随失效 tick 剔除；展示按 occurred_at DESC（与服务端
  // durable 排序同向），到期条目不因刷新/重连返回同 key 而复活。
  const visibleRecentItems = useMemo(() => {
    const nowMs = Date.now();
    return recentCompletionItems
      .filter((item) => isRecentCompletionVisible(item.createdAt ?? "", nowMs))
      .sort(
        (left, right) =>
          Date.parse(right.createdAt ?? "") - Date.parse(left.createdAt ?? ""),
      );
  }, [recentCompletionItems, recentExpiryTick]);
  // C5 Task 6：project 级等待项与 issue 级等待项按稳定 id 合并去重
  //（project 条目 id 即后端稳定 id，不与 issue 级前缀 id 冲突）。
  const waitingInbox = useMemo(() => {
    const seen = new Set(c1WaitingItems.map((item) => item.id));
    return [
      ...c1WaitingItems,
      ...projectC1WaitingItems.filter((item) => !seen.has(item.id)),
    ];
  }, [c1WaitingItems, projectC1WaitingItems]);
  const inbox = useMemo(
    () => [
      ...selectObservedInbox(records),
      ...infoItems,
      ...codingInfoItems,
      ...visibleRecentItems,
      ...waitingInbox,
      ...lcBootstrapItems,
      ...codingAttentionItems,
    ],
    [
      records,
      infoItems,
      codingInfoItems,
      visibleRecentItems,
      waitingInbox,
      lcBootstrapItems,
      codingAttentionItems,
    ],
  );
  const countedRecords = useMemo(
    () => records.filter((record) => watchedSessionIds.includes(record.sessionId)),
    [records, watchedSessionIds],
  );
  // P4-d（REQ-UI37-19）：durable 待处理事项不受 watched 过滤，直接并入可
  // 操作列表（计数 = watched 会话条目 + 全部待处理事项）。
  const countedInbox = useMemo(
    () => [...selectObservedInbox(countedRecords), ...codingAttentionItems],
    [countedRecords, codingAttentionItems],
  );

  useEffect(() => {
    let alive = true;
    let debounceTimer: CatalogRefreshTimer | null = null;
    // P2/F9（无限轮询修复）：dirty 补跑的**独立槽位**——此前它与普通 250ms
    // 防抖共用一个槽位，补跑等待期内的任何唤醒都会把它取消并改排 250ms ⇒
    // 「补跑至少间隔 1s」被完全绕过（连续档回归）。两者必须分开。
    let dirtyTimer: CatalogRefreshTimer | null = null;
    let periodicTimer: CatalogRefreshTimer | null = null;
    let inFlight = false;
    let dirty = false;

    // P3（REQ-WIGA-07）：近期完成目录的应用——hydration 未关闭时（首轮与
    // 初始在途期间的 dirty 重跑）只扩大静默基线；关闭后新稳定身份成为
    // 提示候选；同 key 状态升级不重复候选。
    const applyRecentCatalog = (
      items: readonly CockpitInboxItem[],
      identities: readonly string[],
    ) => {
      const known = knownRecentIdentitiesRef.current ?? new Set<string>();
      knownRecentIdentitiesRef.current = known;
      const nowMs = Date.now();
      const candidates: CockpitInboxItem[] = [];
      for (let index = 0; index < identities.length; index += 1) {
        // P3（REQ-WIGA-07）：TTL 外条目（未来/坏时间/已到期）不进基线也
        // 不进候选；已到期由失效定时器从展示剔除，不因同 key 返回复活。
        if (!isRecentCompletionVisible(items[index].createdAt ?? "", nowMs)) {
          continue;
        }
        const identity = identities[index];
        if (!hydrationClosedRef.current) {
          known.add(identity);
        } else if (!known.has(identity)) {
          known.add(identity);
          candidates.push(items[index]);
        }
      }
      // 候选按客户端生命周期累积（每身份至多一条）；Shell 以 identity 去重
      // 提醒，Task 3 绑定 TTL/once。
      setNotificationCandidates((previous) => [...previous, ...candidates]);
      setRecentCompletionItems(items);
    };

    // P4-d（REQ-UI37-19）：待处理事项目录的应用——与近期完成同用的
    // hydration 静默基线，但无 TTL/上限过滤；事实关闭后条目随下一轮
    // 目录消失（known 集只增不减，同 key 不重复产生提醒候选）。
    const applyAttentionCatalog = (
      items: readonly CockpitInboxItem[],
      identities: readonly string[],
    ) => {
      const known = knownAttentionIdentitiesRef.current ?? new Set<string>();
      knownAttentionIdentitiesRef.current = known;
      const candidates: CockpitInboxItem[] = [];
      for (let index = 0; index < identities.length; index += 1) {
        const identity = identities[index];
        if (!hydrationClosedRef.current) {
          known.add(identity);
        } else if (!known.has(identity)) {
          known.add(identity);
          candidates.push(items[index]);
        }
      }
      setNotificationCandidates((previous) => [...previous, ...candidates]);
      setCodingAttentionItems(items);
    };

    const clearPeriodicTimer = () => {
      if (periodicTimer !== null) {
        cancelRefresh(periodicTimer);
        periodicTimer = null;
      }
    };
    const clearDebounceTimer = () => {
      if (debounceTimer !== null) {
        cancelRefresh(debounceTimer);
        debounceTimer = null;
      }
    };
    const clearDirtyTimer = () => {
      if (dirtyTimer !== null) {
        cancelRefresh(dirtyTimer);
        dirtyTimer = null;
      }
    };

    // P3（无限轮询修复）：下一轮周期排程——空闲（无进行中事实）退避到
    // 60s 档；document.hidden 时暂停（不排下一轮），恢复可见时补跑。
    const scheduleNextPeriodicRefresh = () => {
      clearPeriodicTimer();
      if (refreshIntervalMs <= 0 || document.hidden) {
        return;
      }
      const intervalMs = hasActiveFactsRef.current
        ? refreshIntervalMs
        : Math.max(refreshIntervalMs, IDLE_BACKOFF_INTERVAL_MS);
      periodicTimer = scheduleRefresh(runCatalogRefresh, intervalMs);
    };

    // P2/F9（无限轮询修复）：dirty 补跑在**独立槽位**上至少间隔
    // DIRTY_RERUN_MIN_DELAY_MS 再启动；等待期内的新唤醒合并为同一次补跑，
    // 不叠加、也不得把它提前（此前共用防抖槽时会被 250ms 档顶掉）。
    const scheduleDirtyRerun = () => {
      clearDirtyTimer();
      // 普通 250ms 防抖若已排程，会早于补跑的最小间隔执行 ⇒ 撤销（补跑是
      // 全量 GET，会带上这段等待期内的任何新事实）。
      clearDebounceTimer();
      dirtyTimer = scheduleRefresh(() => {
        dirtyTimer = null;
        runCatalogRefresh();
      }, DIRTY_RERUN_MIN_DELAY_MS);
    };

    const runCatalogRefresh = () => {
      // P3（无限轮询修复）：页面不可见 ⇒ 暂停——丢弃唤醒、不发起请求；
      // 恢复可见时 visibilitychange 立刻补一轮，事实不丢。
      if (document.hidden) {
        return;
      }
      if (inFlight) {
        dirty = true;
        return;
      }
      inFlight = true;
      void (async () => {
        try {
          const { projects } = await getProjects();
          const listedIssues = await Promise.all(
            projects.map(async (project) => ({
              projectId: project.project_id,
              issues: (await getProductIssues(project.project_id)).issues,
            })),
          );
          const lifecycleContexts = listedIssues.flatMap(({ projectId, issues }) =>
            issues.map((issue) => ({ projectId, issueId: issue.issue_id })),
          );
          const lifecycles = await Promise.all(
            listedIssues.flatMap(({ projectId, issues }) =>
              issues.map(async (issue) =>
                getLifecycle(issue.issue_id, projectId, {
                  recentSince: new Date(
                    Date.now() - RECENT_COMPLETION_WINDOW_MS,
                  ).toISOString(),
                  recentLimit: RECENT_COMPLETION_REQUEST_LIMIT,
                }),
              ),
            ),
          );
          if (alive) {
            setSessions(lifecycles.flatMap((lifecycle) => lifecycle.workspace_sessions));
            // C1 Task 9：durable 恢复等待项与列表同源投影（按 issue 归属）。
            const nextC1Items: CockpitInboxItem[] = [];
            lifecycles.forEach((lifecycle, index) => {
              const { projectId, issueId } = lifecycleContexts[index];
              for (const waiting of lifecycle.c1_waiting_items ?? []) {
                nextC1Items.push(c1WaitingItem(waiting, projectId, issueId));
              }
            });
            setC1WaitingItems(nextC1Items);
            // C5 Task 6：project 级 repository 初始化失败等待项与目录同源
            // 补读（每轮 refresh/失效唤醒都重读）；单个 project 读取失败
            // 只保留上一轮该 project 的 durable 快照，不清空既有等待项。
            const previousProjectItems = projectC1ItemsRef.current;
            const nextProjectItems: CockpitInboxItem[] = [];
            await Promise.all(
              projects.map(async (project) => {
                try {
                  const waitingItems = await getProjectWaitingItems(
                    project.project_id,
                  );
                  for (const waiting of waitingItems) {
                    nextProjectItems.push(
                      c1WaitingItem(
                        waiting,
                        project.project_id,
                        waiting.issue_id ?? undefined,
                      ),
                    );
                  }
                } catch {
                  for (const previous of previousProjectItems) {
                    if (previous.c1Info?.projectId === project.project_id) {
                      nextProjectItems.push(previous);
                    }
                  }
                }
              }),
            );
            projectC1ItemsRef.current = nextProjectItems;
            setProjectC1WaitingItems(nextProjectItems);
            // C4 Task 9：LC 冷启动通知与目录同源补读——每个 project 的 LC
            // bootstrap 纯投影 GET；SSE/失效唤醒只触发本补读，页面关闭后
            // 重新打开仍能看到同一等待事实（按稳定 notice key 去重）。
            // 单个 LC 读取失败只跳过该项（等待事实保留上一轮），不拆掉
            // 整个目录观察窗。
            const nextBootstrapItems: CockpitInboxItem[] = [];
            await Promise.all(
              projects.map(async (project) => {
                let codebases: CodebaseSummaryDto[];
                try {
                  ({ codebases } = await getCodebases(project.project_id));
                } catch {
                  return;
                }
                for (const codebase of codebases) {
                  // P4（无限轮询修复）：bootstrap 是 LC 级端点——仅
                  // kind === "logical" 的条目请求；普通单仓用仓库 id 打
                  // LC 端点必然 404 且被静默吞掉，纯浪费。
                  if (codebase.kind !== "logical") {
                    continue;
                  }
                  const lcId = codebase.logical_codebase_id ?? codebase.id;
                  try {
                    const projection = await getBootstrap(project.project_id, lcId);
                    for (const notice of projection.notices) {
                      nextBootstrapItems.push(
                        logicalCodebaseBootstrapItem(notice, projection),
                      );
                    }
                  } catch {
                    // 保留上一轮同 key 事实；下轮目录刷新重试。
                  }
                }
              }),
            );
            setLcBootstrapItems(nextBootstrapItems);
            // P0 1.3（REQ-WIGA-05）Task 11：会话→issue→attempt 发现通道（目录
            // 轮询副产物，无额外请求）；驾驶舱按需拉 attempt snapshot 作答 choice。
            setCodingAttempts(
              lifecycles.flatMap((lifecycle) => lifecycle.coding_attempts ?? []),
            );
            // P3（REQ-WIGA-07）：新服务端以有界近期目录为完成事实主源（按完整
            // project/issue/kind/key 归一化，K 外可见）；只有旧响应（缺
            // recent_completion_info）才回落 watched plan/coding info 投影，
            // 绝不把旧响应认作 K 外全量。
            const legacyPlanInfos: PlanConfirmedInfoItem[] = [];
            const legacyCodingInfos: {
              info: CodingFinalConfirmInfoItem;
              sessionIds: readonly string[];
            }[] = [];
            const recentByIdentity = new Map<string, CockpitInboxItem>();
            for (const lifecycle of lifecycles) {
              if (lifecycle.recent_completion_info !== undefined) {
                for (const entry of lifecycle.recent_completion_info) {
                  recentByIdentity.set(
                    recentCompletionIdentity(entry),
                    recentCompletionInboxItem(entry),
                  );
                }
              } else {
                legacyPlanInfos.push(...(lifecycle.plan_confirmed_info ?? []));
                legacyCodingInfos.push(
                  ...(lifecycle.coding_final_confirm_info ?? []).map((info) => ({
                    info,
                    sessionIds: lifecycle.workspace_sessions.map(
                      (session) => session.workspace_session_id,
                    ),
                  })),
                );
              }
            }
            setPlanConfirmedInfos(legacyPlanInfos);
            setCodingFinalConfirmInfos(legacyCodingInfos);
            // P4-d（REQ-UI37-19）：全部 lifecycle 的待处理事项无 watched/
            // TTL/上限过滤进入 display/count；同 [project,issue,key] 身份
            // 只保留一条（重复响应不重复计数/提醒）。
            const attentionByIdentity = new Map<string, CockpitInboxItem>();
            for (const lifecycle of lifecycles) {
              for (const attention of lifecycle.coding_attention_items ?? []) {
                const identity = codingAttentionIdentity(attention);
                if (!attentionByIdentity.has(identity)) {
                  attentionByIdentity.set(identity, codingAttentionItem(attention));
                }
              }
            }
            // P3（无限轮询修复）：本轮「进行中事实」判定（活跃会话 / issue
            // 与 project 等待项 / LC 冷启动通知 / 待处理事项）——全部复用
            // 本轮目录数据，不新增接口；失败轮保留上一已知值。
            hasActiveFactsRef.current =
              lifecycles.some((lifecycle) =>
                lifecycle.workspace_sessions.some((session) =>
                  WATCHED_SESSION_STATUSES.has(session.status),
                ),
              ) ||
              nextC1Items.length > 0 ||
              nextProjectItems.length > 0 ||
              nextBootstrapItems.length > 0 ||
              attentionByIdentity.size > 0;
            applyAttentionCatalog(
              Array.from(attentionByIdentity.values()),
              Array.from(attentionByIdentity.keys()),
            );
            applyRecentCatalog(
              Array.from(recentByIdentity.values()),
              Array.from(recentByIdentity.keys()),
            );
          }
        } catch {
          // 保留上一次成功目录与候选，避免瞬时 REST 失败拆除整个观察窗；
          // 失败不置首次 hydration 基线。
        } finally {
          inFlight = false;
          if (alive) {
            if (dirty) {
              // 本轮在途期间又收到唤醒：结束后补跑一次，以新一轮 GET
              // 校正被唤醒期间的事实（旧 HTTP 结果不覆盖新事实）。
              dirty = false;
              // P2（无限轮询修复）：补跑不再零延迟——经 scheduleDirtyRerun
              // 至少间隔 DIRTY_RERUN_MIN_DELAY_MS，打断连续档。
              scheduleDirtyRerun();
              // P2（无限轮询修复）：dirty 分支同样排下一轮周期定时器，
              // 周期节奏不被快路径旁路。
              scheduleNextPeriodicRefresh();
            } else {
              hydrationClosedRef.current = true;
              scheduleNextPeriodicRefresh();
            }
          }
        }
      })();
    };

    // invalidation / WS snapshot hint 汇入 ~250ms trailing debounce；一轮
    // catalog 请求在途时只标 dirty，不并发第二层请求。
    const queueCatalogRefresh = () => {
      if (!alive) {
        return;
      }
      if (inFlight) {
        dirty = true;
        return;
      }
      // P2/F9：dirty 补跑等待期内的新唤醒**并入**已排的补跑（补跑是全量
      // GET，会带上这期间的新事实）——既不取消它，也不另排 250ms 档（那会把
      // 补跑提前到最小间隔之前 ⇒ 「补跑至少间隔 1s」被绕过、连续档回归）。
      if (dirtyTimer !== null) {
        return;
      }
      if (debounceTimer !== null) {
        cancelRefresh(debounceTimer);
      }
      debounceTimer = scheduleRefresh(() => {
        debounceTimer = null;
        runCatalogRefresh();
      }, CATALOG_REFRESH_DEBOUNCE_MS);
    };

    snapshotHintRef.current = queueCatalogRefresh;
    runCatalogRefresh();
    const unsubscribeInvalidation = subscribeToLifecycleInvalidation(queueCatalogRefresh);

    // P3（无限轮询修复）：页面可见性——隐藏即暂停（撤销已排定时器，在途
    // 请求自然收敛），回到可见立刻补一轮并恢复周期。
    const handleVisibilityChange = () => {
      if (!alive) {
        return;
      }
      if (document.hidden) {
        clearPeriodicTimer();
        clearDebounceTimer();
        // P2/F9：隐藏即暂停——已排的 dirty 补跑同样撤销（回到可见时立刻
        // 补一轮，事实不丢）。
        clearDirtyTimer();
        return;
      }
      runCatalogRefresh();
    };
    document.addEventListener("visibilitychange", handleVisibilityChange);

    return () => {
      alive = false;
      snapshotHintRef.current = () => undefined;
      unsubscribeInvalidation();
      document.removeEventListener("visibilitychange", handleVisibilityChange);
      if (debounceTimer !== null) {
        cancelRefresh(debounceTimer);
      }
      if (dirtyTimer !== null) {
        cancelRefresh(dirtyTimer);
      }
      if (periodicTimer !== null) {
        cancelRefresh(periodicTimer);
      }
    };
  }, [cancelRefresh, getBootstrap, getCodebases, getLifecycle, getProductIssues, getProjectWaitingItems, getProjects, refreshIntervalMs, scheduleRefresh]);

  // P3（REQ-WIGA-07）：最近到期单次失效定时器——按可见条目的最早
  // occurred_at + INFO_TTL_MS 触发一次 tick，让页面不刷新也能移除过期
  // info；触发后由可见 memo 重排下一轮最早到期，不发起 REST。卸载清理。
  useEffect(() => {
    let nearestExpiryMs: number | null = null;
    for (const item of visibleRecentItems) {
      const occurredMs = Date.parse(item.createdAt ?? "");
      if (!Number.isFinite(occurredMs)) {
        continue;
      }
      const expiryMs = occurredMs + INFO_TTL_MS;
      if (nearestExpiryMs === null || expiryMs < nearestExpiryMs) {
        nearestExpiryMs = expiryMs;
      }
    }
    if (nearestExpiryMs === null) {
      return;
    }
    const timer = scheduleRefresh(
      () => setRecentExpiryTick((tick) => tick + 1),
      Math.max(0, nearestExpiryMs - Date.now()),
    );
    return () => cancelRefresh(timer);
  }, [cancelRefresh, scheduleRefresh, visibleRecentItems]);

  useEffect(() => {
    controllerRef.current?.updateRefreshIntervalMs(refreshIntervalMs);
  }, [refreshIntervalMs]);

  useEffect(() => {
    void controllerRef.current?.replaceWatchedSessionIds(observedSessionIds);
  }, [observedSessionIds]);

  useEffect(
    () => () => {
      controllerRef.current?.dispose();
      controllerRef.current = null;
    },
    [],
  );

  const watchSession = useCallback((sessionId: string) => {
    setExtraSessionIds((previous) =>
      previous.includes(sessionId) ? previous : [...previous, sessionId],
    );
  }, []);

  // Task 11：会话归属 issue 的最新活跃（created/running）coding attempt；无会话
  // 目录条目或无活跃 attempt 时 null（不猜、不触发 coding 首启）。
  const codingAttemptForSession = useCallback(
    (sessionId: string): CodingAttempt | null => {
      const issueId =
        sessions.find((session) => session.workspace_session_id === sessionId)?.issue_id ?? null;
      if (issueId === null) {
        return null;
      }
      const candidates = codingAttempts.filter(
        (attempt) =>
          attempt.issue_id === issueId &&
          (attempt.status === "created" || attempt.status === "running"),
      );
      return candidates.at(-1) ?? null;
    },
    [codingAttempts, sessions],
  );

  return {
    records,
    inbox,
    countedInbox,
    watchedSessionIds,
    watchSession,
    codingAttemptForSession,
    // P3（REQ-WIGA-07）：`inbox` 保留为 displayItems 同义别名；可操作计数
    // 与提示候选从显式三分数据取。
    displayItems: inbox,
    actionableCount: countedInbox.length,
    notificationCandidates,
  };
}
