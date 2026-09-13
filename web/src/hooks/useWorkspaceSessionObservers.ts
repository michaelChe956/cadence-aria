import { useEffect, useMemo, useRef, useState } from "react";
import {
  getIssueLifecycle,
  listProductIssues,
  listProjects,
} from "../api/client";
import type {
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
  ) => Promise<Pick<IssueLifecycleResponse, "workspace_sessions">>;
  createController?: WorkspaceObserverControllerFactory;
}

export type WorkspaceObserverControllerFactory = (
  onRecordsChange: (records: readonly WorkspaceObserverRecord[]) => void,
) => WorkspaceObserverController;

export interface WorkspaceSessionObserverResult {
  records: readonly WorkspaceObserverRecord[];
  inbox: readonly CockpitInboxItem[];
  watchedSessionIds: readonly string[];
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
  } = options;
  const [sessions, setSessions] = useState<readonly WorkspaceSessionSummary[]>([]);
  const [observerRecords, setObserverRecords] = useState<readonly WorkspaceObserverRecord[]>([]);
  const [catalogLoaded, setCatalogLoaded] = useState(false);
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
    () => selectWatchedSessionIds(sessions, watchLimit),
    [sessions, watchLimit],
  );
  const observedSessionIds = useMemo(
    () => watchedSessionIds.filter((sessionId) => sessionId !== currentSessionId),
    [currentSessionId, watchedSessionIds],
  );
  const records = useMemo(() => {
    const observed = observerRecords.filter((record) =>
      watchedSessionIds.includes(record.sessionId),
    );
    if (
      currentSessionId !== null &&
      currentSessionState !== null &&
      (!catalogLoaded || watchedSessionIds.includes(currentSessionId))
    ) {
      return [{ sessionId: currentSessionId, state: currentSessionState }, ...observed];
    }
    return observed;
  }, [
    catalogLoaded,
    currentSessionId,
    currentSessionState,
    observerRecords,
    watchedSessionIds,
  ]);
  const inbox = useMemo(() => selectObservedInbox(records), [records]);

  useEffect(() => {
    let alive = true;
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
          setCatalogLoaded(true);
        }
      } catch {
        if (alive) {
          setSessions([]);
          setCatalogLoaded(true);
        }
      }
    })();

    return () => {
      alive = false;
    };
  }, [getLifecycle, getProductIssues, getProjects]);

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

  return { records, inbox, watchedSessionIds };
}
