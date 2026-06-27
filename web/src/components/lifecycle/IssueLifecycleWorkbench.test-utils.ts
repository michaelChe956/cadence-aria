import { beforeEach, vi } from "vitest";
import type { IssueWorkItemPlanDetailDto } from "../../api/types";
import { useLifecycleWorkbenchStore } from "../../state/lifecycle-workbench-store";
import {
  codingAttemptRecord,
  findDesignBySession,
  findSession,
  findStoryBySession,
  initialLifecycleData,
  isMockIssueWorkItemPlan,
  issueWorkItemPlanRecord,
  jsonResponse,
  projectRecord,
  repositoryRecord,
  workspaceSessionRecord,
  type MockLifecycleData,
} from "./IssueLifecycleWorkbench.test-data";

export * from "./IssueLifecycleWorkbench.test-data";

export function installIssueLifecycleWorkbenchTestHooks() {
  beforeEach(() => {
    useLifecycleWorkbenchStore.setState({
      focusedEntityId: null,
      isDrawerOpen: false,
    });
  });
}

export function lifecycleFetch(options?: {
  duplicateCardIds?: boolean;
  emptyLifecycle?: boolean;
  invalidLifecycle?: boolean;
  confirmedWorkItem?: boolean;
  issueDescription?: string;
  issueTitles?: string[];
  issueTitlesByProject?: Record<string, string>;
  projects?: Array<ReturnType<typeof projectRecord>>;
  repositoriesByProject?: Record<string, ReturnType<typeof repositoryRecord>[]>;
  projectResponses?: Array<Promise<Response>>;
  splitWorkItems?: boolean;
  workItemPlans?: unknown[];
  skippedIntegrationRisk?: boolean;
}) {
  const projects = [
    ...(options?.projects ?? [projectRecord("project_0001", "Aria")]),
  ];
  const repositoriesByProject = new Map<
    string,
    ReturnType<typeof repositoryRecord>[]
  >(
    Object.entries(
      options?.repositoriesByProject ?? { project_0001: [repositoryRecord()] },
    ),
  );
  const deletedIssueIdsByProject = new Map<string, Set<string>>();
  let projectCall = 0;
  const issueCallsByProject = new Map<string, number>();
  const latestIssueTitlesByProject = new Map<string, string>();
  const lifecycleByIssue = new Map<string, MockLifecycleData>();

  function lifecycleData(issueId: string) {
    const existing = lifecycleByIssue.get(issueId);
    if (existing) {
      return existing;
    }
    const initial = initialLifecycleData(
      issueId,
      options?.duplicateCardIds,
      options?.emptyLifecycle,
      options?.confirmedWorkItem,
      options?.splitWorkItems,
      options?.workItemPlans,
      options?.skippedIntegrationRisk,
    );
    lifecycleByIssue.set(issueId, initial);
    return initial;
  }

  return vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    if (url === "/api/projects" && init?.method === "POST") {
      const payload = JSON.parse(String(init.body)) as {
        name: string;
        description?: string | null;
      };
      const project = projectRecord(
        `project_${String(projects.length + 1).padStart(4, "0")}`,
        payload.name,
        payload.description ?? null,
      );
      projects.push(project);
      return jsonResponse(project);
    }
    const projectDeleteMatch = url.match(/^\/api\/projects\/([^/]+)$/);
    if (projectDeleteMatch && init?.method === "DELETE") {
      const projectId = projectDeleteMatch[1];
      const index = projects.findIndex(
        (project) => project.project_id === projectId,
      );
      if (index >= 0) {
        projects.splice(index, 1);
      }
      repositoriesByProject.delete(projectId);
      deletedIssueIdsByProject.delete(projectId);
      return jsonResponse({ status: "deleted" });
    }
    if (url === "/api/projects") {
      const response = options?.projectResponses?.[projectCall];
      projectCall += 1;
      return response ?? jsonResponse({ projects });
    }
    const repositoryDeleteMatch = url.match(
      /^\/api\/projects\/([^/]+)\/repositories\/([^/]+)$/,
    );
    if (repositoryDeleteMatch && init?.method === "DELETE") {
      const projectId = repositoryDeleteMatch[1];
      const repositoryId = repositoryDeleteMatch[2];
      repositoriesByProject.set(
        projectId,
        (repositoriesByProject.get(projectId) ?? []).filter(
          (repository) => repository.repository_id !== repositoryId,
        ),
      );
      return jsonResponse({ status: "deleted" });
    }
    const repositoryMatch = url.match(
      /^\/api\/projects\/([^/]+)\/repositories$/,
    );
    if (repositoryMatch) {
      const projectId = repositoryMatch[1];
      if (init?.method === "POST") {
        const payload = JSON.parse(String(init.body)) as {
          name: string;
          path: string;
          default_policy_preset?: string | null;
          default_provider_mode?: string | null;
        };
        const repositories = repositoriesByProject.get(projectId) ?? [];
        const repository = repositoryRecord({
          repository_id: `repository_${String(repositories.length + 1).padStart(4, "0")}`,
          project_id: projectId,
          name: payload.name,
          path: payload.path,
          default_policy_preset:
            payload.default_policy_preset ?? "manual-write",
          default_provider_mode: payload.default_provider_mode ?? "fake",
        });
        repositoriesByProject.set(projectId, [...repositories, repository]);
        return jsonResponse(repository);
      }
      return jsonResponse({
        repositories: repositoriesByProject.get(projectId) ?? [],
      });
    }
    if (
      url === "/api/projects/project_0001/issues" &&
      init?.method === "POST"
    ) {
      return jsonResponse({
        issue_id: "issue_0002",
        project_id: "project_0001",
        repo_id: "repository_0001",
        workspace_id: null,
        task_id: null,
        session_id: null,
        title: "新增安全提示",
        description: null,
        change_id: "new-security-hint",
        phase: "clarification",
        status: "draft",
        active_binding_id: null,
        artifacts: [],
        created_at: "2026-05-16T00:00:00Z",
        updated_at: "2026-05-16T00:00:00Z",
      });
    }
    const issueDeleteMatch = url.match(
      /^\/api\/projects\/([^/]+)\/issues\/([^/]+)$/,
    );
    if (issueDeleteMatch && init?.method === "DELETE") {
      const projectId = issueDeleteMatch[1];
      const issueId = issueDeleteMatch[2];
      const deletedIssueIds =
        deletedIssueIdsByProject.get(projectId) ?? new Set<string>();
      deletedIssueIds.add(issueId);
      deletedIssueIdsByProject.set(projectId, deletedIssueIds);
      lifecycleByIssue.delete(issueId);
      return jsonResponse({ status: "deleted" });
    }
    const storySpecDeleteMatch = url.match(
      /^\/api\/projects\/([^/]+)\/issues\/([^/]+)\/story-specs\/([^/]+)$/,
    );
    if (storySpecDeleteMatch && init?.method === "DELETE") {
      const issueId = storySpecDeleteMatch[2];
      const storySpecId = storySpecDeleteMatch[3];
      const lifecycle = lifecycleData(issueId);
      lifecycle.story_specs = lifecycle.story_specs.filter(
        (story) => story.story_spec_id !== storySpecId,
      );
      lifecycle.workspace_sessions = lifecycle.workspace_sessions.filter(
        (session) =>
          !(
            session.workspace_type === "story" &&
            session.entity_id === storySpecId
          ),
      );
      return jsonResponse({ status: "deleted" });
    }
    const designSpecDeleteMatch = url.match(
      /^\/api\/projects\/([^/]+)\/issues\/([^/]+)\/design-specs\/([^/]+)$/,
    );
    if (designSpecDeleteMatch && init?.method === "DELETE") {
      const issueId = designSpecDeleteMatch[2];
      const designSpecId = designSpecDeleteMatch[3];
      const lifecycle = lifecycleData(issueId);
      lifecycle.design_specs = lifecycle.design_specs.filter(
        (design) => design.design_spec_id !== designSpecId,
      );
      lifecycle.workspace_sessions = lifecycle.workspace_sessions.filter(
        (session) =>
          !(
            session.workspace_type === "design" &&
            session.entity_id === designSpecId
          ),
      );
      return jsonResponse({ status: "deleted" });
    }
    const workItemPlanDeleteMatch = url.match(
      /^\/api\/projects\/([^/]+)\/issues\/([^/]+)\/work-item-plans\/([^/]+)$/,
    );
    if (workItemPlanDeleteMatch && init?.method === "DELETE") {
      const issueId = workItemPlanDeleteMatch[2];
      const planId = workItemPlanDeleteMatch[3];
      const lifecycle = lifecycleData(issueId);
      const plan = lifecycle.work_item_plans.find(
        (candidate): candidate is IssueWorkItemPlanDetailDto =>
          isMockIssueWorkItemPlan(candidate) && candidate.id === planId,
      );
      const childWorkItemIds = new Set(plan?.work_item_ids ?? []);
      lifecycle.work_item_plans = lifecycle.work_item_plans.filter(
        (candidate) =>
          !isMockIssueWorkItemPlan(candidate) || candidate.id !== planId,
      );
      lifecycle.work_items = lifecycle.work_items.filter(
        (workItem) =>
          !childWorkItemIds.has(String(workItem.work_item_id ?? "")),
      );
      lifecycle.workspace_sessions = lifecycle.workspace_sessions.filter(
        (session) =>
          !(
            (session.workspace_type === "work_item_plan" &&
              session.entity_id === planId) ||
            (session.workspace_type === "work_item" &&
              childWorkItemIds.has(session.entity_id))
          ),
      );
      lifecycle.coding_attempts = lifecycle.coding_attempts.filter(
        (attempt) => !childWorkItemIds.has(attempt.work_item_id),
      );
      return jsonResponse({ status: "deleted" });
    }
    const workItemDeleteMatch = url.match(
      /^\/api\/projects\/([^/]+)\/issues\/([^/]+)\/work-items\/([^/]+)$/,
    );
    if (workItemDeleteMatch && init?.method === "DELETE") {
      const issueId = workItemDeleteMatch[2];
      const workItemId = workItemDeleteMatch[3];
      const lifecycle = lifecycleData(issueId);
      lifecycle.work_items = lifecycle.work_items.filter(
        (workItem) => workItem.work_item_id !== workItemId,
      );
      lifecycle.workspace_sessions = lifecycle.workspace_sessions.filter(
        (session) =>
          !(
            session.workspace_type === "work_item" &&
            session.entity_id === workItemId
          ),
      );
      lifecycle.coding_attempts = lifecycle.coding_attempts.filter(
        (attempt) => attempt.work_item_id !== workItemId,
      );
      return jsonResponse({ status: "deleted" });
    }
    const workspaceRunNextMatch = url.match(
      /^\/api\/workspace-sessions\/([^/]+)\/run-next$/,
    );
    if (workspaceRunNextMatch) {
      const payload = JSON.parse(String(init?.body ?? "{}")) as {
        user_prompt?: string;
      };
      const session = findSession(lifecycleByIssue, workspaceRunNextMatch[1]);
      if (session) {
        session.status = "waiting_for_human";
        session.messages = [
          ...session.messages,
          {
            role: "user",
            content: payload.user_prompt ?? "",
            created_at: "2026-05-16T00:00:00Z",
          },
          {
            role: "provider",
            content: "provider result",
            created_at: "2026-05-16T00:00:01Z",
          },
          {
            role: "reviewer",
            content: "reviewer result",
            created_at: "2026-05-16T00:00:02Z",
          },
        ];
      }
      return jsonResponse(session ?? {});
    }
    if (
      url === "/api/workspace-sessions/workspace_session_story_0001/message"
    ) {
      return jsonResponse({
        ...workspaceSessionRecord(
          "story",
          "story_spec_0001",
          "workspace_session_story_0001",
        ),
        messages: [
          {
            role: "user",
            content: "请补充验收标准",
            created_at: "2026-05-16T00:00:00Z",
          },
        ],
      });
    }
    const workspaceMessageMatch = url.match(
      /^\/api\/workspace-sessions\/([^/]+)\/message$/,
    );
    if (workspaceMessageMatch) {
      const session = findSession(lifecycleByIssue, workspaceMessageMatch[1]);
      return jsonResponse(session ?? {});
    }
    const workspaceConfirmMatch = url.match(
      /^\/api\/workspace-sessions\/([^/]+)\/confirm$/,
    );
    if (workspaceConfirmMatch) {
      const session = findSession(lifecycleByIssue, workspaceConfirmMatch[1]);
      if (session) {
        session.status = "confirmed";
      }
      const story = findStoryBySession(
        lifecycleByIssue,
        workspaceConfirmMatch[1],
      );
      if (story) {
        story.confirmation_status = "confirmed";
      }
      const design = findDesignBySession(
        lifecycleByIssue,
        workspaceConfirmMatch[1],
      );
      if (design) {
        design.confirmation_status = "confirmed";
      }
      return jsonResponse(session ?? {});
    }
    const storyGenerateMatch = url.match(
      /^\/api\/projects\/([^/]+)\/issues\/([^/]+)\/story-specs:generate$/,
    );
    if (storyGenerateMatch) {
      const issueId = storyGenerateMatch[2];
      const payload = JSON.parse(String(init?.body ?? "{}")) as {
        title: string;
        author_provider: "claude_code" | "codex" | "fake";
        reviewer_provider: "claude_code" | "codex" | "fake";
        review_rounds: number;
        superpowers_enabled: boolean;
        openspec_enabled: boolean;
      };
      const lifecycle = lifecycleData(issueId);
      const story = {
        story_spec_id: "story_spec_0001",
        issue_id: issueId,
        repository_id: "repository_0001",
        title: payload.title,
        current_version: null,
        current_markdown_preview: null,
        confirmation_status: "confirmed",
        artifact_versions: [],
      };
      const session = workspaceSessionRecord(
        "story",
        "story_spec_0001",
        "workspace_session_story_0001",
        {
          author_provider: payload.author_provider,
          reviewer_provider: payload.reviewer_provider,
          review_rounds: payload.review_rounds,
          superpowers_enabled: payload.superpowers_enabled,
          openspec_enabled: payload.openspec_enabled,
          status: "open",
        },
      );
      lifecycle.story_specs = [story];
      lifecycle.workspace_sessions = [session];
      return jsonResponse({ story_specs: [story], workspace_session: session });
    }
    const designGenerateMatch = url.match(
      /^\/api\/projects\/([^/]+)\/issues\/([^/]+)\/design-specs:generate$/,
    );
    if (designGenerateMatch) {
      const issueId = designGenerateMatch[2];
      const payload = JSON.parse(String(init?.body ?? "{}")) as {
        title: string;
        story_spec_ids: string[];
        author_provider: "claude_code" | "codex" | "fake";
        reviewer_provider: "claude_code" | "codex" | "fake";
        review_rounds: number;
        superpowers_enabled: boolean;
        openspec_enabled: boolean;
      };
      const lifecycle = lifecycleData(issueId);
      const designId = lifecycle.design_specs.some(
        (candidate) => candidate.design_spec_id === "design_spec_0001",
      )
        ? "design_spec_0002"
        : "design_spec_0001";
      const design = {
        design_spec_id: designId,
        issue_id: issueId,
        story_spec_ids: payload.story_spec_ids,
        title: payload.title,
        current_version: null,
        current_markdown_preview: null,
        confirmation_status: "confirmed",
        artifact_versions: [],
      };
      const session = workspaceSessionRecord(
        "design",
        designId,
        designId === "design_spec_0001"
          ? "workspace_session_design_0001"
          : "workspace_session_design_0002",
        {
          author_provider: payload.author_provider,
          reviewer_provider: payload.reviewer_provider,
          review_rounds: payload.review_rounds,
          superpowers_enabled: payload.superpowers_enabled,
          openspec_enabled: payload.openspec_enabled,
          status: "open",
        },
      );
      lifecycle.design_specs = [...lifecycle.design_specs, design];
      lifecycle.workspace_sessions.push(session);
      return jsonResponse({
        design_specs: [design],
        workspace_session: session,
      });
    }
    const prepareWorkItemPlanMatch = url.match(
      /^\/api\/projects\/([^/]+)\/issues\/([^/]+)\/work-item-plans:prepare$/,
    );
    if (prepareWorkItemPlanMatch) {
      const issueId = prepareWorkItemPlanMatch[2];
      const projectId = prepareWorkItemPlanMatch[1];
      const payload = JSON.parse(String(init?.body ?? "{}")) as {
        title: string;
        story_spec_ids: string[];
        design_spec_ids: string[];
        author_provider: "claude_code" | "codex" | "fake";
        reviewer_provider: "claude_code" | "codex" | "fake";
        review_rounds: number;
        superpowers_enabled: boolean;
        openspec_enabled: boolean;
        include_integration_tests?: boolean;
        include_e2e_tests?: boolean;
        force_frontend_backend_split?: boolean;
        require_execution_plan_confirm?: boolean;
      };
      const lifecycle = lifecycleData(issueId);
      const workItemPlan = issueWorkItemPlanRecord({
        id: "issue_plan_0001",
        project_id: projectId,
        issue_id: issueId,
        source_story_spec_ids: payload.story_spec_ids,
        source_design_spec_ids: payload.design_spec_ids,
        options: {
          include_integration_tests: payload.include_integration_tests ?? true,
          include_e2e_tests: payload.include_e2e_tests ?? false,
          force_frontend_backend_split:
            payload.force_frontend_backend_split ?? true,
          require_execution_plan_confirm:
            payload.require_execution_plan_confirm ?? false,
        },
        work_item_ids: [],
        status: "draft",
      });
      const session = workspaceSessionRecord(
        "work_item_plan",
        workItemPlan.id,
        "workspace_session_plan_group_0001",
        {
          author_provider: payload.author_provider,
          reviewer_provider: payload.reviewer_provider,
          review_rounds: payload.review_rounds,
          superpowers_enabled: payload.superpowers_enabled,
          openspec_enabled: payload.openspec_enabled,
          status: "open",
        },
      );
      lifecycle.work_item_plans = [workItemPlan];
      lifecycle.workspace_sessions.push(session);
      return jsonResponse({
        work_item_plan: workItemPlan,
        workspace_session: session,
      });
    }
    const codingAttemptCreateMatch = url.match(
      /^\/api\/projects\/([^/]+)\/issues\/([^/]+)\/work-items\/([^/]+)\/coding-attempts$/,
    );
    if (codingAttemptCreateMatch && init?.method === "POST") {
      const issueId = codingAttemptCreateMatch[2];
      const workItemId = codingAttemptCreateMatch[3];
      const lifecycle = lifecycleData(issueId);
      const attempt = codingAttemptRecord(workItemId);
      lifecycle.coding_attempts.push(attempt);
      const workItem = lifecycle.work_items.find(
        (candidate) => candidate.work_item_id === workItemId,
      );
      if (workItem) {
        workItem.latest_attempt = attempt;
        workItem.execution_status = "coding";
      }
      return jsonResponse(attempt);
    }
    const issuesMatch = url.match(/^\/api\/projects\/([^/]+)\/issues$/);
    if (issuesMatch) {
      const projectId = issuesMatch[1];
      const issueCall = issueCallsByProject.get(projectId) ?? 0;
      const title =
        options?.issueTitlesByProject?.[projectId] ??
        options?.issueTitles?.[issueCall] ??
        "登录会话过期";
      latestIssueTitlesByProject.set(projectId, title);
      issueCallsByProject.set(projectId, issueCall + 1);
      const issueId = options?.duplicateCardIds ? "shared_id" : "issue_0001";
      if (deletedIssueIdsByProject.get(projectId)?.has(issueId)) {
        return jsonResponse({ issues: [] });
      }
      if (projectId !== "project_0001") {
        if (deletedIssueIdsByProject.get(projectId)?.has("issue_0002")) {
          return jsonResponse({ issues: [] });
        }
        return jsonResponse({
          issues: title
            ? [
                {
                  issue_id: "issue_0002",
                  project_id: projectId,
                  repo_id: null,
                  workspace_id: null,
                  task_id: null,
                  session_id: null,
                  title,
                  description: options?.issueDescription ?? "描述",
                  change_id: "mobile-refresh",
                  phase: "clarification",
                  status: "draft",
                  active_binding_id: null,
                  artifacts: [],
                  created_at: "2026-05-16T00:00:00Z",
                  updated_at: "2026-05-16T00:00:00Z",
                },
              ]
            : [],
        });
      }
      return jsonResponse({
        issues: [
          {
            issue_id: issueId,
            project_id: "project_0001",
            repo_id: "repository_0001",
            workspace_id: null,
            task_id: null,
            session_id: null,
            title: options?.duplicateCardIds ? "重复 ID Issue" : title,
            description: options?.issueDescription ?? "描述",
            change_id: "login-session-expired",
            phase: "clarification",
            status: "draft",
            active_binding_id: null,
            artifacts: [],
            created_at: "2026-05-16T00:00:00Z",
            updated_at: "2026-05-16T00:00:00Z",
          },
        ],
      });
    }
    const lifecycleMatch = url.match(
      /^\/api\/issues\/([^/]+)\/lifecycle\?project_id=([^&]+)$/,
    );
    if (lifecycleMatch) {
      if (options?.invalidLifecycle) {
        return jsonResponse({});
      }
      const duplicate = options?.duplicateCardIds ?? false;
      const requestIssueId = lifecycleMatch[1];
      const projectId = lifecycleMatch[2];
      const issueId = duplicate ? "shared_id" : requestIssueId;
      const issueTitle = duplicate
        ? "重复 ID Issue"
        : latestIssueTitlesByProject.get(projectId) ??
          options?.issueTitlesByProject?.[projectId] ??
          "登录会话过期";
      if (projectId !== "project_0001") {
        return jsonResponse({
          issue: {
            issue_id: issueId,
            project_id: projectId,
            repo_id: null,
            workspace_id: null,
            task_id: null,
            session_id: null,
            title: issueTitle,
            description: options?.issueDescription ?? "描述",
            change_id: "mobile-refresh",
            phase: "clarification",
            status: "draft",
            active_binding_id: null,
            artifacts: [],
            created_at: "2026-05-16T00:00:00Z",
            updated_at: "2026-05-16T00:00:00Z",
          },
          story_specs: [],
          design_specs: [],
          work_item_plans: [],
          work_items: [],
          workspace_sessions: [],
          coding_attempts: [],
        });
      }
      const data = lifecycleData(issueId);
      return jsonResponse({
        issue: {
          issue_id: issueId,
          project_id: "project_0001",
          repo_id: "repository_0001",
          workspace_id: null,
          task_id: null,
          session_id: null,
          title: issueTitle,
          description: options?.issueDescription ?? "描述",
          change_id: "login-session-expired",
          phase: "clarification",
          status: "draft",
          active_binding_id: null,
          artifacts: [],
          created_at: "2026-05-16T00:00:00Z",
          updated_at: "2026-05-16T00:00:00Z",
        },
        story_specs: data.story_specs,
        design_specs: data.design_specs,
        work_item_plans: data.work_item_plans,
        work_items: data.work_items,
        workspace_sessions: data.workspace_sessions,
        coding_attempts: data.coding_attempts,
      });
    }
    return jsonResponse({});
  });
}
