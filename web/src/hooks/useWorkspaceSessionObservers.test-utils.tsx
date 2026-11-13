// useWorkspaceSessionObservers 测试夹具（共享 summary / observerOptions /
// renderObserverHook）。
//
// 2026-11-13：useWorkspaceSessionObservers.test.tsx 超出 large_file_guard
// 登记上限（tests/it_core/large_file_guard.rs）⇒ 按主题拆成
// bootstrap / attention / polling 三个测试文件，公共夹具抽到本文件；
// 用例与断言逐字保留在各测试文件里，行为零变化。
import { render } from "@testing-library/react";
import type { WorkspaceSessionSummary } from "../api/types";
import {
  type WorkspaceSessionObserverOptions,
  type WorkspaceSessionObserverResult,
  useWorkspaceSessionObservers,
} from "./useWorkspaceSessionObservers";

export function summary(workspace_session_id: string): WorkspaceSessionSummary {
  return {
    workspace_session_id,
    issue_id: "issue_1",
    entity_id: `entity_${workspace_session_id}`,
    workspace_type: "work_item",
    status: "running",
    author_provider: "claude_code",
    reviewer_provider: "codex",
    review_rounds: 1,
    automation: { owner: "client", enrollment_id: null, policy_revision: null, enabled: false },
    superpowers_enabled: false,
    openspec_enabled: false,
  };
}

export function observerOptions(overrides: Partial<WorkspaceSessionObserverOptions> = {}) {
  return {
    currentSessionId: "s1",
    watchLimit: 2,
    refreshIntervalMs: 15_000,
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
        base_branch: null,
        workspace_id: null,
        task_id: null,
        session_id: null,
        title: "Issue",
        description: null,
        change_id: "change_1",
        phase: "development" as const,
        status: "in_progress" as const,
        active_binding_id: null,
        created_at: "2026-09-14T00:00:00Z",
        updated_at: "2026-09-14T00:00:00Z",
      }],
    }),
    getIssueLifecycle: async () => ({
      workspace_sessions: [summary("s1"), summary("s2"), summary("s3")],
      coding_attempts: [],
    }),
    ...overrides,
  } satisfies WorkspaceSessionObserverOptions;
}

export function renderObserverHook(initialOptions: WorkspaceSessionObserverOptions) {
  let result: WorkspaceSessionObserverResult | undefined;

  function Harness({ options }: { options: WorkspaceSessionObserverOptions }) {
    result = useWorkspaceSessionObservers(options);
    return null;
  }

  const view = render(<Harness options={initialOptions} />);
  return {
    ...view,
    rerender(options: WorkspaceSessionObserverOptions) {
      view.rerender(<Harness options={options} />);
    },
    get result() {
      if (!result) throw new Error("hook result unavailable");
      return result;
    },
  };
}
