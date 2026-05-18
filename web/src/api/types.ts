export type ApiError = {
  code: string;
  message: string;
  details: Record<string, unknown>;
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
  current_markdown_preview: string | null;
  confirmation_status: LifecycleConfirmationStatus;
};

export type DesignSpec = {
  design_spec_id: string;
  issue_id: string;
  story_spec_ids: string[];
  design_kind: "frontend" | "backend";
  title: string;
  current_version: number | null;
  current_markdown_preview: string | null;
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

export type WorkspaceProviderName = "claude_code" | "codex" | "fake";

export type ProviderWorkspaceConfig = {
  author_provider: WorkspaceProviderName;
  reviewer_provider: WorkspaceProviderName;
  review_rounds: number;
  superpowers_enabled: boolean;
  openspec_enabled: boolean;
};

export type ProviderWorkspaceConfigInput = Partial<ProviderWorkspaceConfig>;

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
  author_provider: WorkspaceProviderName;
  reviewer_provider: WorkspaceProviderName;
  review_rounds: number;
  superpowers_enabled: boolean;
  openspec_enabled: boolean;
  messages: WorkspaceMessage[];
};

export type GenerateStorySpecsRequest = ProviderWorkspaceConfigInput & {
  title: string;
};

export type GenerateStorySpecsResponse = {
  story_specs: StorySpec[];
  workspace_session: WorkspaceSession;
};

export type GenerateDesignSpecsRequest = ProviderWorkspaceConfigInput & {
  title: string;
  story_spec_ids: string[];
  design_kind: "frontend" | "backend";
};

export type GenerateDesignSpecsResponse = {
  design_specs: DesignSpec[];
  workspace_session: WorkspaceSession;
};

export type GenerateWorkItemsRequest = ProviderWorkspaceConfigInput & {
  title: string;
  story_spec_ids: string[];
  design_spec_ids: string[];
};

export type GenerateWorkItemsResponse = {
  work_items: LifecycleWorkItem[];
  workspace_session: WorkspaceSession;
};
