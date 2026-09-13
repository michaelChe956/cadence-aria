import { act, render, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { WorkspaceSessionSummary } from "../api/types";
import {
  useWorkspaceSessionObservers,
  type WorkspaceSessionObserverOptions,
} from "./useWorkspaceSessionObservers";

function summary(workspace_session_id: string): WorkspaceSessionSummary {
  return {
    workspace_session_id,
    issue_id: "issue_1",
    entity_id: `entity_${workspace_session_id}`,
    workspace_type: "work_item",
    status: "running",
    author_provider: "claude_code",
    reviewer_provider: "codex",
    review_rounds: 1,
    superpowers_enabled: false,
    openspec_enabled: false,
  };
}

function renderObserverHook(options: WorkspaceSessionObserverOptions) {
  let result: ReturnType<typeof useWorkspaceSessionObservers> | undefined;

  function Harness() {
    result = useWorkspaceSessionObservers(options);
    return null;
  }

  return {
    ...render(<Harness />),
    get result() {
      if (!result) throw new Error("hook result unavailable");
      return result;
    },
  };
}

describe("useWorkspaceSessionObservers", () => {
  it("enumerates REST sessions and replaces the watch window immediately when K changes", async () => {
    const replaceWatchedSessionIds = vi.fn();
    const options: WorkspaceSessionObserverOptions = {
      currentSessionId: "s1",
      watchLimit: 2,
      listProjects: async () => ({
        projects: [
          {
            project_id: "project_1",
            name: "Project",
            description: null,
            created_at: "2026-09-14T00:00:00Z",
            updated_at: "2026-09-14T00:00:00Z",
            last_opened_at: null,
          },
        ],
      }),
      listProductIssues: async () => ({
        issues: [
          {
            issue_id: "issue_1",
            project_id: "project_1",
            repo_id: null,
            workspace_id: null,
            task_id: null,
            session_id: null,
            title: "Issue",
            description: null,
            change_id: "change_1",
            phase: "development",
            status: "in_progress",
            active_binding_id: null,
            created_at: "2026-09-14T00:00:00Z",
            updated_at: "2026-09-14T00:00:00Z",
          },
        ],
      }),
      getIssueLifecycle: async () => ({ workspace_sessions: [summary("s1"), summary("s2"), summary("s3")] }),
      createController: () => ({
        replaceWatchedSessionIds,
        records: () => [],
        dispose: vi.fn(),
      }),
    };
    const view = renderObserverHook(options);

    await waitFor(() => expect(view.result.watchedSessionIds).toEqual(["s1", "s2"]));
    expect(replaceWatchedSessionIds).toHaveBeenLastCalledWith(["s2"]);

    await act(async () => {
      view.unmount();
    });
  });

  it("does not open a duplicate observer socket for the current workspace session", async () => {
    const replaceWatchedSessionIds = vi.fn();
    const view = renderObserverHook({
      currentSessionId: "active",
      watchLimit: 2,
      listProjects: async () => ({
        projects: [{
          project_id: "project_1",
          name: "Project",
          description: null,
          created_at: "2026-09-14T00:00:00Z",
          updated_at: "2026-09-14T00:00:00Z",
          last_opened_at: null,
        }],
      }),
      listProductIssues: async () => ({
        issues: [{
          issue_id: "issue_1",
          project_id: "project_1",
          repo_id: null,
          workspace_id: null,
          task_id: null,
          session_id: null,
          title: "Issue",
          description: null,
          change_id: "change_1",
          phase: "development",
          status: "in_progress",
          active_binding_id: null,
          created_at: "2026-09-14T00:00:00Z",
          updated_at: "2026-09-14T00:00:00Z",
        }],
      }),
      getIssueLifecycle: async () => ({ workspace_sessions: [summary("active"), summary("other")] }),
      createController: () => ({
        replaceWatchedSessionIds,
        records: () => [],
        dispose: vi.fn(),
      }),
    });

    await waitFor(() => expect(view.result.watchedSessionIds).toEqual(["active", "other"]));
    expect(replaceWatchedSessionIds).toHaveBeenLastCalledWith(["other"]);
  });

  it("does not count the current session when it falls outside K", async () => {
    const replaceWatchedSessionIds = vi.fn();
    const view = renderObserverHook({
      currentSessionId: "outside",
      currentSessionState: {
        sessionId: "outside",
        sessionStatus: "stopped_needs_human",
        stage: "running",
        humanGateSnapshot: null,
        humanGateTurn: null,
        humanGateClosure: null,
        flowKind: null,
        pendingReviewerSummary: null,
        chatEntries: [],
        protocolError: null,
        error: null,
        advanceCommands: {},
      } as never,
      watchLimit: 1,
      listProjects: async () => ({
        projects: [{
          project_id: "project_1",
          name: "Project",
          description: null,
          created_at: "2026-09-14T00:00:00Z",
          updated_at: "2026-09-14T00:00:00Z",
          last_opened_at: null,
        }],
      }),
      listProductIssues: async () => ({
        issues: [{
          issue_id: "issue_1",
          project_id: "project_1",
          repo_id: null,
          workspace_id: null,
          task_id: null,
          session_id: null,
          title: "Issue",
          description: null,
          change_id: "change_1",
          phase: "development",
          status: "in_progress",
          active_binding_id: null,
          created_at: "2026-09-14T00:00:00Z",
          updated_at: "2026-09-14T00:00:00Z",
        }],
      }),
      getIssueLifecycle: async () => ({ workspace_sessions: [summary("watched"), summary("outside")] }),
      createController: () => ({ replaceWatchedSessionIds, records: () => [], dispose: vi.fn() }),
    });

    await waitFor(() => expect(view.result.watchedSessionIds).toEqual(["watched"]));
    expect(view.result.records).toEqual([]);
    expect(view.result.inbox).toEqual([]);
    expect(replaceWatchedSessionIds).toHaveBeenLastCalledWith(["watched"]);
  });
});
