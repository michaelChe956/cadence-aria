export type ApiError = {
  code: string;
  message: string;
  details: Record<string, unknown>;
};

export type CreateTaskRequest = {
  request_text: string;
  change_id: string;
  policy_preset: string;
  provider_mode: string;
  timeout_secs: number;
};

export type CreateTaskResponse = {
  task_id: string;
  session_id: string;
  change_id: string;
  phase: string;
};

export type PendingProviderStep = {
  node_id: string;
  provider_type: string;
  runtime_role: string;
  adapter_role: string;
  prompt: string;
  input_summary: unknown;
  canonical_input_refs: string[];
  context_files: string[];
  output_schema: string;
  allowed_write_scope: string[];
  forbidden_actions: string[];
  verification_commands: string[];
  checkpoint_id: string;
};

export type WebWorkspaceProjection = {
  workspace_root: string;
  active_task_id: string | null;
  active_session_id: string | null;
  overview: Record<string, unknown>;
  sessions: unknown[];
  timeline: Array<Record<string, unknown>>;
  artifact_index: Array<Record<string, unknown>>;
  diagnostics: Array<Record<string, unknown>>;
  available_actions: string[];
  pending_provider_step: PendingProviderStep | null;
  selected_node_context: {
    node_id: string | null;
    overview: Record<string, unknown>;
    inputs: unknown[];
    run: unknown[];
    outputs: unknown[];
    diffs: unknown[];
  };
  git_summary: {
    workspace_path: string;
    branch: string | null;
    head: string | null;
    dirty: boolean;
    dirty_files: string[];
  };
  event_cursor: number;
};

export type WebEvent = {
  cursor: number;
  event_type: string;
  task_id: string | null;
  payload: unknown;
};

export type Project = {
  project_id: string;
  name: string;
  description: string | null;
  created_at: string;
  updated_at: string;
  last_opened_at: string | null;
};

export type Repository = {
  repository_id: string;
  project_id: string;
  name: string;
  path: string;
  repo_hash: string;
  runtime_root: string;
  default_policy_preset: string;
  default_provider_mode: string;
  created_at: string;
  updated_at: string;
};

export type RepositoryListResponse = {
  repositories: Repository[];
};

export type CreateRepositoryRequest = {
  name: string;
  path: string;
  default_policy_preset?: string | null;
  default_provider_mode?: string | null;
};

export type ProductIssue = {
  issue_id: string;
  project_id: string;
  repo_id: string | null;
  workspace_id: string | null;
  task_id: string | null;
  session_id: string | null;
  title: string;
  description: string | null;
  change_id: string;
  phase: "clarification" | "development" | "acceptance";
  status: "draft" | "in_progress" | "completed" | "blocked";
  active_binding_id: string | null;
  artifacts?: ProductIssueArtifact[];
  created_at: string;
  updated_at: string;
};

export type ProductIssueArtifact = {
  artifact_ref: string;
  artifact_kind: string;
  producer_node: string | null;
  path: string;
  summary: string;
  stage: "story_spec" | "design_spec" | "work_item" | "done";
};

export type ProductIssueListResponse = {
  issues: ProductIssue[];
};

export type CreateProductIssueRequest = {
  title: string;
  description?: string | null;
  change_id?: string | null;
  repository_id: string;
};

export type LifecycleConfirmationStatus =
  | "draft"
  | "in_review"
  | "confirmed"
  | "change_requested"
  | "blocked";

export type StorySpec = {
  story_spec_id: string;
  issue_id: string;
  repository_id: string;
  title: string;
  current_version: number | null;
  confirmation_status: LifecycleConfirmationStatus;
};

export type DesignSpec = {
  design_spec_id: string;
  issue_id: string;
  story_spec_ids: string[];
  design_kind: "frontend" | "backend";
  title: string;
  current_version: number | null;
  confirmation_status: LifecycleConfirmationStatus;
};

export type LifecycleWorkItem = {
  work_item_id: string;
  issue_id: string;
  repository_id: string;
  story_spec_ids: string[];
  design_spec_ids: string[];
  title: string;
  plan_status: "not_started" | "draft" | "confirmed" | "change_requested";
  execution_status: "pending" | "planning" | "coding" | "completed" | "blocked";
};

export type IssueLifecycleResponse = {
  issue: ProductIssue;
  story_specs: StorySpec[];
  design_specs: DesignSpec[];
  work_items: LifecycleWorkItem[];
  workspace_sessions: WorkspaceSession[];
};

export type WorkspaceMessage = {
  role: string;
  content: string;
  created_at: string;
};

export type WorkspaceSession = {
  workspace_session_id: string;
  issue_id: string;
  entity_id: string;
  workspace_type: "story" | "design" | "work_item";
  status:
    | "open"
    | "running"
    | "waiting_for_human"
    | "confirmed"
    | "change_requested"
    | "blocked_provider_unavailable"
    | "terminated";
  author_provider: "claude_code" | "codex" | "fake";
  reviewer_provider: "claude_code" | "codex" | "fake";
  review_rounds: number;
  superpowers_enabled: boolean;
  openspec_enabled: boolean;
  messages: WorkspaceMessage[];
};

export type StartProductIssueRequest = {
  workspace_id?: string | null;
  repository_id?: string | null;
  policy_preset?: string | null;
  provider_mode?: string | null;
  timeout_secs?: number | null;
};

export type StartProductIssueResponse = {
  issue_id: string;
  project_id: string;
  repository_id: string;
  workspace_id: string;
  task_id: string;
  session_id: string;
  status: string;
};

export type ProductWebEvent = WebEvent & {
  project_id?: string | null;
  issue_id?: string | null;
  binding_id?: string | null;
};

export type TaskListResponse = {
  tasks: Array<{
    task_id: string;
    change_id: string | null;
    phase: string | null;
    updated_at?: string | null;
  }>;
};

export type ArtifactContentResponse = {
  artifact_ref: string;
  artifact_kind: string;
  producer_node: string | null;
  path: string;
  content_type: "markdown" | "json" | "source" | "test" | "log" | "unknown";
  content: string;
};

export type FileContentResponse = {
  path: string;
  content_type: string;
  content: string;
};

export type FileDiffResponse = {
  base_checkpoint: string;
  path: string;
  diff: string;
};

export type ProviderOutputChunk = {
  node_id: string;
  provider_run_id: string;
  stream: "stdout" | "stderr";
  text: string;
  structured_output?: unknown;
  manual_gate?: string;
  retry_attempt?: number;
};

export type StopTaskResponse = {
  status: string;
  task_id: string;
};

export type RollbackPreviewResponse = {
  checkpoint_id: string;
  git_head: string | null;
  dirty: boolean;
  turns_to_drop: number;
  node_runs_to_drop: number;
  provider_runs_to_drop: number;
  artifacts_to_drop: number;
  files_may_change: string[];
};

export type Workspace = {
  workspace_id: string;
  name: string;
  path: string;
  default_policy_preset: string;
  default_provider_mode: string;
  created_at: string;
  updated_at: string;
};

export type WorkspaceListResponse = {
  workspaces: Workspace[];
};

export type CreateWorkspaceRequest = {
  name: string;
  path: string;
  default_policy_preset?: string | null;
  default_provider_mode?: string | null;
};

export type Issue = {
  issue_id: string;
  title: string;
  description: string | null;
  status: string;
  workspace_id: string | null;
  task_id: string | null;
  session_id: string | null;
  change_id: string;
  created_at: string;
  updated_at: string;
};

export type IssueListResponse = {
  issues: Issue[];
};

export type CreateIssueRequest = {
  title: string;
  description?: string | null;
  change_id?: string | null;
};

export type StartIssueRequest = {
  workspace_id: string;
  policy_preset?: string | null;
  provider_mode?: string | null;
  timeout_secs?: number | null;
};

export type StartIssueResponse = {
  issue_id: string;
  workspace_id: string;
  task_id: string;
  session_id: string;
  status: string;
};
