import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  getIssueLifecycle,
  listProductIssues,
  listProjects,
} from "../api/client";
import { listCodebases } from "../api/codebases";
import { getLogicalCodebaseBootstrap } from "../api/logical-codebase-bootstrap";
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
    >
  >;
  /** C4 Task 9：LC 目录读取（默认真实 API；测试注入伪实现）。 */
  listCodebases?: typeof listCodebases;
  /** C4 Task 9：bootstrap 纯投影读取（默认真实 API；测试注入伪实现）。 */
  getLogicalCodebaseBootstrap?: typeof getLogicalCodebaseBootstrap;
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
  // P3（REQ-WIGA-07）：到期 tick——最近到期单次失效定时器触发后递增，
  // 使可见投影在无 REST 的情况下剔除过期 info。
  const [recentExpiryTick, setRecentExpiryTick] = useState(0);
  // 首次成功 hydration（含空目录）建立的稳定身份基线；失败不置基线。
  // hydration 在一轮补读静默收敛（无 pending dirty）后才关闭，初始 GET
  // 在途期间的失效唤醒归入同一静默批次。
  const knownRecentIdentitiesRef = useRef<Set<string> | null>(null);
  const hydrationClosedRef = useRef(false);
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
  const inbox = useMemo(
    () => [
      ...selectObservedInbox(records),
      ...infoItems,
      ...codingInfoItems,
      ...visibleRecentItems,
      ...c1WaitingItems,
      ...lcBootstrapItems,
    ],
    [records, infoItems, codingInfoItems, visibleRecentItems, c1WaitingItems, lcBootstrapItems],
  );
  const countedRecords = useMemo(
    () => records.filter((record) => watchedSessionIds.includes(record.sessionId)),
    [records, watchedSessionIds],
  );
  const countedInbox = useMemo(() => selectObservedInbox(countedRecords), [countedRecords]);

  useEffect(() => {
    let alive = true;
    let debounceTimer: CatalogRefreshTimer | null = null;
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

    const runCatalogRefresh = () => {
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
              // 本轮在途期间又收到唤醒：结束后立即补跑一次，以新一轮 GET
              // 校正被唤醒期间的事实（旧 HTTP 结果不覆盖新事实）。
              dirty = false;
              runCatalogRefresh();
            } else {
              hydrationClosedRef.current = true;
              if (refreshIntervalMs > 0) {
                periodicTimer = scheduleRefresh(runCatalogRefresh, refreshIntervalMs);
              }
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

    return () => {
      alive = false;
      snapshotHintRef.current = () => undefined;
      unsubscribeInvalidation();
      if (debounceTimer !== null) {
        cancelRefresh(debounceTimer);
      }
      if (periodicTimer !== null) {
        cancelRefresh(periodicTimer);
      }
    };
  }, [cancelRefresh, getBootstrap, getCodebases, getLifecycle, getProductIssues, getProjects, refreshIntervalMs, scheduleRefresh]);

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
