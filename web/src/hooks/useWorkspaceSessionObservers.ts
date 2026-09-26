import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  getIssueLifecycle,
  listProductIssues,
  listProjects,
} from "../api/client";
import type {
  CodingAttempt,
  IssueLifecycleResponse,
  ProductIssueListResponse,
  WorkspaceSessionSummary,
} from "../api/types";
import {
  createObserverController,
  selectObservedInbox,
  selectWatchedSessionIds,
  type WorkspaceObserverController,
  type WorkspaceObserverRecord,
} from "../state/workspace-observer-store";
import type { CockpitInboxItem } from "../state/workspace-cockpit-projection";
import type { WorkspaceWsState } from "../state/workspace-ws-store";

export type CatalogRefreshTimer = number;

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
  ) => Promise<
    Pick<IssueLifecycleResponse, "workspace_sessions" | "coding_attempts">
  >;
  createController?: WorkspaceObserverControllerFactory;
  scheduleCatalogRefresh?: (callback: () => void, delayMs: number) => CatalogRefreshTimer;
  cancelCatalogRefresh?: (timer: CatalogRefreshTimer) => void;
}

export type WorkspaceObserverControllerFactory = (
  onRecordsChange: (records: readonly WorkspaceObserverRecord[]) => void,
) => WorkspaceObserverController;

export interface WorkspaceSessionObserverResult {
  records: readonly WorkspaceObserverRecord[];
  inbox: readonly CockpitInboxItem[];
  watchedSessionIds: readonly string[];
  countedInbox: readonly CockpitInboxItem[];
  watchSession(sessionId: string): void;
  /** Task 11：会话所属 issue 的最新活跃 coding attempt（无则 null）。 */
  codingAttemptForSession(sessionId: string): CodingAttempt | null;
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
  const controllerRef = useRef<WorkspaceObserverController | null>(null);

  if (controllerRef.current === null) {
    controllerRef.current = createController
      ? createController(setObserverRecords)
      : createObserverController(undefined, setObserverRecords, {
          refreshIntervalMs,
          reconnectDelayMs: 1_000,
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
  const inbox = useMemo(() => selectObservedInbox(records), [records]);
  const countedRecords = useMemo(
    () => records.filter((record) => watchedSessionIds.includes(record.sessionId)),
    [records, watchedSessionIds],
  );
  const countedInbox = useMemo(() => selectObservedInbox(countedRecords), [countedRecords]);

  useEffect(() => {
    let alive = true;
    let refreshTimer: CatalogRefreshTimer | null = null;
    const refreshCatalog = () => {
      void (async () => {
        try {
          const { projects } = await getProjects();
          const listedIssues = await Promise.all(
            projects.map(async (project) => ({
              projectId: project.project_id,
              issues: (await getProductIssues(project.project_id)).issues,
            })),
          );
          const lifecycles = await Promise.all(
            listedIssues.flatMap(({ projectId, issues }) =>
              issues.map(async (issue) => getLifecycle(issue.issue_id, projectId)),
            ),
          );
          if (alive) {
            setSessions(lifecycles.flatMap((lifecycle) => lifecycle.workspace_sessions));
            // P0 1.3（REQ-WIGA-05）Task 11：会话→issue→attempt 发现通道（目录
            // 轮询副产物，无额外请求）；驾驶舱按需拉 attempt snapshot 作答 choice。
            setCodingAttempts(
              lifecycles.flatMap((lifecycle) => lifecycle.coding_attempts ?? []),
            );
          }
        } catch {
          // 保留上一次成功目录，避免瞬时 REST 失败拆除整个观察窗。
        } finally {
          if (alive && refreshIntervalMs > 0) {
            refreshTimer = scheduleRefresh(refreshCatalog, refreshIntervalMs);
          }
        }
      })();
    };

    refreshCatalog();

    return () => {
      alive = false;
      if (refreshTimer !== null) {
        cancelRefresh(refreshTimer);
      }
    };
  }, [cancelRefresh, getLifecycle, getProductIssues, getProjects, refreshIntervalMs, scheduleRefresh]);

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
  };
}
