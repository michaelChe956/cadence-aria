import type {
  CodingAttempt,
  IssueWorkItemPlanDetailDto,
  LifecycleWorkItem,
  WorkspaceSession,
} from "../../api/types";
import type { LifecycleCard as LifecycleCardData } from "../../state/lifecycle-workbench-store";

export type MockLifecycleData = {
  story_specs: Array<Record<string, unknown>>;
  design_specs: Array<Record<string, unknown>>;
  work_item_plans: unknown[];
  work_items: Array<Record<string, unknown>>;
  workspace_sessions: WorkspaceSession[];
  coding_attempts: CodingAttempt[];
};

export function isMockIssueWorkItemPlan(
  value: unknown,
): value is IssueWorkItemPlanDetailDto {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const candidate = value as { id?: unknown; work_item_ids?: unknown };
  return (
    typeof candidate.id === "string" &&
    Array.isArray(candidate.work_item_ids) &&
    candidate.work_item_ids.every((item) => typeof item === "string")
  );
}

export function jsonResponse(body: unknown) {
  return Promise.resolve(jsonResponseValue(body));
}

export function jsonResponseValue(body: unknown) {
  return new Response(JSON.stringify(body), { status: 200 });
}

export function projectsBody() {
  return {
    projects: [projectRecord("project_0001", "Aria")],
  };
}

export function projectRecord(
  projectId: string,
  name: string,
  description: string | null = null,
) {
  return {
    project_id: projectId,
    name,
    description,
    created_at: "2026-05-16T00:00:00Z",
    updated_at: "2026-05-16T00:00:00Z",
    last_opened_at: null,
  };
}

export function repositoryRecord(
  overrides?: Partial<ReturnType<typeof repositoryRecordShape>>,
) {
  return {
    ...repositoryRecordShape(),
    ...(overrides ?? {}),
  };
}

export function repositoryRecordShape() {
  return {
    repository_id: "repository_0001",
    project_id: "project_0001",
    name: "Aria Repo",
    path: "/tmp/aria",
    repo_hash: "hash",
    runtime_root: "/tmp/aria/.aria/runtime",
    default_policy_preset: "manual-write",
    default_provider_mode: "fake",
    created_at: "2026-05-16T00:00:00Z",
    updated_at: "2026-05-16T00:00:00Z",
  };
}

export function workspaceSessionRecord(
  workspaceType: "story" | "design" | "work_item" | "work_item_plan",
  entityId: string,
  sessionId: string,
  overrides?: Partial<WorkspaceSession>,
): WorkspaceSession {
  return {
    ...workspaceSessionRecordShape(workspaceType, entityId, sessionId),
    ...(overrides ?? {}),
  };
}

export function workspaceSessionRecordShape(
  workspaceType: "story" | "design" | "work_item" | "work_item_plan",
  entityId: string,
  sessionId: string,
): WorkspaceSession {
  return {
    workspace_session_id: sessionId,
    issue_id: "issue_0001",
    entity_id: entityId,
    workspace_type: workspaceType,
    status: "waiting_for_human",
    author_provider: "codex",
    reviewer_provider: "claude_code",
    review_rounds: 2,
    superpowers_enabled: true,
    openspec_enabled: true,
    messages: [],
  };
}

export function initialLifecycleData(
  issueId: string,
  duplicate: boolean | undefined,
  empty: boolean | undefined,
  confirmedWorkItem: boolean | undefined,
  splitWorkItems: boolean | undefined,
  workItemPlans: unknown[] | undefined,
  skippedIntegrationRisk: boolean | undefined,
  codingAttempts?: CodingAttempt[],
): MockLifecycleData {
  if (empty) {
    return {
      story_specs: [],
      design_specs: [],
      work_item_plans: [],
      work_items: [],
      workspace_sessions: [],
      coding_attempts: [],
    };
  }

  const storyId = duplicate ? "shared_id" : "story_spec_0001";
  const workItems = splitWorkItems
    ? [
        workItemRecord({
          work_item_id: "work_item_backend",
          issue_id: issueId,
          title: "后端 API",
          kind: "backend",
          plan_status: "confirmed",
          execution_status: "pending",
          depends_on: [],
          validator_findings: skippedIntegrationRisk
            ? [
                {
                  finding_id: "finding_0001",
                  level: "warning",
                  code: "integration_or_e2e_skipped_risk",
                  message: "integration or e2e work item was skipped",
                  affected_scopes: [],
                },
              ]
            : undefined,
        }),
        workItemRecord({
          work_item_id: "work_item_frontend",
          issue_id: issueId,
          title: "前端 UI",
          kind: "frontend",
          plan_status: "confirmed",
          execution_status: "pending",
          depends_on: ["work_item_backend"],
        }),
      ]
    : [
        workItemRecord({
          issue_id: issueId,
          plan_status: confirmedWorkItem ? "confirmed" : "draft",
          artifact_versions: [
            {
              version: 1,
              markdown: "## 实施计划\n\n[TASK-001] 实现会话过期提示组件。",
              generated_by: "claude_code",
              reviewed_by: "codex",
              review_verdict: "pass",
              confirmed_by: confirmedWorkItem ? "human" : null,
              created_at: "2026-05-20T00:02:00Z",
              source_node_id: "timeline_node_work_item_001",
            },
          ],
        }),
      ];

  const defaultWorkItemPlans =
    workItems.length > 0
      ? [
          issueWorkItemPlanRecord({
            issue_id: issueId,
            work_item_ids: workItems.map((item) => item.work_item_id),
            status: confirmedWorkItem ? "confirmed" : "draft",
          }),
        ]
      : [];

  return {
    story_specs: [
      {
        story_spec_id: storyId,
        issue_id: issueId,
        repository_id: "repository_0001",
        title: duplicate ? "重复 ID Story" : "会话过期提示",
        current_version: 1,
        current_markdown_preview: "## 功能需求\n\n[REQ-001] 显示会话过期提示。",
        confirmation_status: "confirmed",
        artifact_versions: [],
      },
    ],
    design_specs: [
      {
        design_spec_id: "design_spec_0001",
        issue_id: issueId,
        story_spec_ids: [storyId],
        title: "前端提示设计",
        current_version: 1,
        current_markdown_preview: "## 关键决策\n\n[DEC-001] 使用全局提示条。",
        confirmation_status: "confirmed",
        artifact_versions: [],
      },
    ],
    work_item_plans: workItemPlans ?? defaultWorkItemPlans,
    work_items: workItems,
    workspace_sessions: [
      workspaceSessionRecord("story", storyId, "workspace_session_story_0001"),
      workspaceSessionRecord(
        "design",
        "design_spec_0001",
        "workspace_session_design_0001",
      ),
      workspaceSessionRecord(
        "work_item",
        "work_item_0001",
        "workspace_session_work_item_0001",
      ),
      workspaceSessionRecord(
        "work_item_plan",
        "issue_plan_0001",
        "workspace_session_plan_group_0001",
      ),
    ],
    coding_attempts: codingAttempts ?? [],
  };
}

export function codingAttemptRecord(workItemId: string): CodingAttempt {
  return {
    attempt_id: "coding_attempt_0001",
    work_item_id: workItemId,
    attempt_scope: "work_item",
    work_item_group_id: null,
    current_work_item_id: workItemId,
    active_unit_id: null,
    attempt_no: 1,
    status: "created",
    stage: "prepare_context",
    branch_name: `aria/work-items/${workItemId}/attempt-1`,
    base_branch: "main",
    worktree_path: null,
    rework_count: 0,
    head_commit: null,
    push_status: null,
    review_request_url: null,
    created_at: "2026-05-23T00:00:00Z",
    updated_at: "2026-05-23T00:00:00Z",
  };
}

export function codingGroupAttemptRecord(planId: string): CodingAttempt {
  return {
    attempt_id: "coding_attempt_0001",
    work_item_id: "work_item_backend",
    attempt_scope: "work_item_group",
    work_item_group_id: planId,
    current_work_item_id: "work_item_backend",
    active_unit_id: "coding_unit_0001",
    attempt_no: 1,
    status: "created",
    stage: "prepare_context",
    branch_name: `aria/issues/issue_0001/${planId}/attempt-1`,
    base_branch: "main",
    worktree_path: null,
    rework_count: 0,
    head_commit: null,
    push_status: null,
    review_request_url: null,
    created_at: "2026-05-23T00:00:00Z",
    updated_at: "2026-05-23T00:00:00Z",
  };
}

export function issueWorkItemPlanRecord(
  overrides: Partial<IssueWorkItemPlanDetailDto> = {},
): IssueWorkItemPlanDetailDto {
  return {
    id: "issue_plan_0001",
    issue_id: "issue_0001",
    project_id: "project_0001",
    status: "draft",
    source_story_spec_ids: ["story_spec_0001"],
    source_design_spec_ids: ["design_spec_0001"],
    work_item_ids: ["work_item_0001"],
    verification_plan_ids: [],
    dependency_graph: [],
    repository_profile_ref: null,
    options: {
      include_integration_tests: true,
      include_e2e_tests: false,
      force_frontend_backend_split: true,
      require_execution_plan_confirm: false,
    },
    validator_findings: [],
    created_at: "2026-05-20T00:00:00Z",
    updated_at: "2026-05-20T00:00:00Z",
    ...overrides,
  };
}

export function workItemRecord(
  overrides: Partial<LifecycleWorkItem> = {},
): LifecycleWorkItem {
  return {
    work_item_id: "work_item_0001",
    issue_id: "issue_0001",
    repository_id: "repository_0001",
    story_spec_ids: ["story_spec_0001"],
    design_spec_ids: ["design_spec_0001"],
    title: "实现提示组件",
    plan_status: "draft",
    execution_status: "planning",
    latest_attempt: null,
    artifact_versions: [
      {
        version: 1,
        markdown: "## 实施计划\n\n[TASK-001] 实现会话过期提示组件。",
        generated_by: "claude_code",
        reviewed_by: "codex",
        review_verdict: "pass",
        confirmed_by: null,
        created_at: "2026-05-20T00:02:00Z",
        source_node_id: "timeline_node_work_item_001",
      },
    ],
    work_item_set_id: null,
    kind: "backend",
    sequence_hint: null,
    depends_on: [],
    exclusive_write_scopes: [],
    forbidden_write_scopes: [],
    context_budget: {
      target_context_k: "30-50",
      max_summary_chars: 20000,
      max_handoff_chars: 12000,
      max_code_context_chars: 30000,
      max_context_file_refs: 80,
      max_traceability_refs: 40,
      max_dependency_handoffs: 3,
    },
    required_handoff_from: [],
    verification_plan_ref: null,
    require_execution_plan_confirm: false,
    execution_plan_status: "not_started",
    handoff_summary_ref: null,
    completion_commit: null,
    completion_diff_summary_ref: null,
    ...overrides,
  };
}

export function findSession(
  lifecycles: Map<
    string,
    {
      workspace_sessions: WorkspaceSession[];
    }
  >,
  sessionId: string,
) {
  for (const lifecycle of lifecycles.values()) {
    const session = lifecycle.workspace_sessions.find(
      (candidate) => candidate.workspace_session_id === sessionId,
    );
    if (session) {
      return session;
    }
  }
  return null;
}

export function findStoryBySession(
  lifecycles: Map<
    string,
    {
      story_specs: Array<Record<string, unknown>>;
      workspace_sessions: WorkspaceSession[];
    }
  >,
  sessionId: string,
) {
  for (const lifecycle of lifecycles.values()) {
    const session = lifecycle.workspace_sessions.find(
      (candidate) =>
        candidate.workspace_session_id === sessionId &&
        candidate.workspace_type === "story",
    );
    if (session) {
      return (
        lifecycle.story_specs.find(
          (story) => story.story_spec_id === session.entity_id,
        ) ?? null
      );
    }
  }
  return null;
}

export function findDesignBySession(
  lifecycles: Map<
    string,
    {
      design_specs: Array<Record<string, unknown>>;
      workspace_sessions: WorkspaceSession[];
    }
  >,
  sessionId: string,
) {
  for (const lifecycle of lifecycles.values()) {
    const session = lifecycle.workspace_sessions.find(
      (candidate) =>
        candidate.workspace_session_id === sessionId &&
        candidate.workspace_type === "design",
    );
    if (session) {
      return (
        lifecycle.design_specs.find(
          (design) => design.design_spec_id === session.entity_id,
        ) ?? null
      );
    }
  }
  return null;
}

export function lifecycleCardTitle(
  kind: LifecycleCardData["kind"],
  title: string,
): LifecycleCardData {
  return {
    kind,
    title,
  } as LifecycleCardData;
}

export function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((promiseResolve, promiseReject) => {
    resolve = promiseResolve;
    reject = promiseReject;
  });
  return { promise, resolve, reject };
}
