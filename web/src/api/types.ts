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
  artifact_versions: ArtifactVersion[];
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
  artifact_versions: ArtifactVersion[];
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

export type ProviderConfigSnapshot = {
  author: WorkspaceProviderName;
  reviewer?: WorkspaceProviderName | null;
  review_rounds: number;
};

export type StructuredFeedback = {
  feedback_types: string[];
  description: string;
  target_artifact_version?: number | null;
};

export type RevisionPath = "revise" | "revise-with-context" | "skip-to-human";
export type HumanConfirmDecision = "confirm" | "request-change" | "terminate";

export type WsInMessage =
  | { type: "user_message"; content: string }
  | { type: "context_note"; content: string }
  | {
      type: "start_generation";
      provider_config: ProviderConfigSnapshot;
      reviewer_enabled: boolean;
    }
  | { type: "rollback"; checkpoint_id: string }
  | { type: "confirm" }
  | { type: "provider_select"; role: string; provider: WorkspaceProviderName }
  | { type: "permission_response"; id: string; approved: boolean; reason?: string | null }
  | { type: "review_decision_response"; decision: string; extra_context?: string | null }
  | { type: "select_revision_path"; path: RevisionPath; extra_context?: string | null }
  | { type: "request_revision"; feedback: StructuredFeedback }
  | { type: "human_confirm"; decision: HumanConfirmDecision; payload?: unknown }
  | { type: "abort" }
  | { type: "hello"; session_id: string; last_seen_node_id?: string | null }
  | { type: "ping" };

export type TimelineNodeType =
  | "prepare_context"
  | "context_note"
  | "start_generation"
  | "author_run"
  | "reviewer_run"
  | "review_decision"
  | "revision"
  | "human_confirm"
  | "aborted_by_disconnect"
  | "protocol_error"
  | "completed";

export type TimelineNodeStatus = "active" | "paused" | "completed" | "failed" | "skipped";
export type ProviderStatus =
  | "starting"
  | "running"
  | "waiting_approval"
  | "completed"
  | "failed"
  | "aborted";
export type ExecutionEventKind = "provider" | "turn" | "command" | "output" | "artifact";
export type ExecutionEventStatus =
  | "started"
  | "running"
  | "waiting_approval"
  | "completed"
  | "failed"
  | "aborted";
export type ReviewVerdictType = "pass" | "revise" | "needs_human";

export type WsMessage = {
  id: string;
  role: string;
  content: string;
  checkpoint_id?: string | null;
  created_at: string;
};

export type WsCheckpoint = {
  id: string;
  message_index: number;
  stage: string;
  created_at: string;
};

export type WsProviderConfig = {
  author: WorkspaceProviderName;
  reviewer?: WorkspaceProviderName | null;
};

export type ProviderDefaults = {
  author: WorkspaceProviderName;
  reviewer?: WorkspaceProviderName | null;
  review_rounds: number;
};

export type TimelineNode = {
  node_id: string;
  node_type: TimelineNodeType;
  agent?: WorkspaceProviderName | null;
  stage: string;
  round?: number | null;
  status: TimelineNodeStatus;
  title: string;
  summary?: string | null;
  started_at: string;
  completed_at?: string | null;
  duration_ms?: number | null;
  artifact_ref?: string | null;
  provider_config_snapshot: ProviderConfigSnapshot;
};

export type ReviewVerdict = {
  verdict: ReviewVerdictType;
  comments: string;
  summary: string;
};

export type ExecutionEvent = {
  event_id: string;
  node_id?: string | null;
  agent?: WorkspaceProviderName | string | null;
  kind: ExecutionEventKind;
  status: ExecutionEventStatus;
  title: string;
  detail?: string | null;
  command?: string | null;
  cwd?: string | null;
  output?: string | null;
  exit_code?: number | null;
};

export type ArtifactVersion = {
  version: number;
  markdown: string;
  generated_by: WorkspaceProviderName;
  reviewed_by?: WorkspaceProviderName | null;
  review_verdict?: ReviewVerdictType | null;
  confirmed_by?: string | null;
  created_at: string;
  source_node_id: string;
};

export type ProviderSnapshot = {
  name: string;
  model: string;
};

export type ArtifactRef = {
  artifact_id: string;
  version: number;
};

export type PermissionEvent = {
  request_id: string;
  request: unknown;
  response: unknown | null;
  ts: string;
};

export type NodeDetail = {
  node_id: string;
  session_id: string;
  node_type: TimelineNodeType;
  status: TimelineNodeStatus;
  agent_role: "author" | "reviewer" | null;
  provider: ProviderSnapshot | null;
  prompt?: string | null;
  messages: WsMessage[];
  streaming_content: string;
  execution_events: ExecutionEvent[];
  permission_events: PermissionEvent[];
  verdict: ReviewVerdict | null;
  artifact_ref: ArtifactRef | null;
  is_revision: boolean;
  base_artifact_ref: ArtifactRef | null;
  started_at: string;
  ended_at: string | null;
};

export type WsOutMessage =
  | { type: "stream_chunk"; role: string; content: string; node_id?: string | null }
  | {
      type: "message_complete";
      message_id: string;
      checkpoint_id: string;
      node_id?: string | null;
    }
  | { type: "stage_change"; stage: string }
  | { type: "artifact_update"; version: number; markdown: string; diff?: string | null }
  | { type: "provider_select_request"; stage: string; defaults: ProviderDefaults }
  | {
      type: "permission_request";
      id: string;
      tool_name: string;
      description: string;
      risk_level: "low" | "medium" | "high";
    }
  | { type: "provider_status"; status: ProviderStatus }
  | { type: "execution_event"; event: ExecutionEvent }
  | { type: "timeline_node_created"; node: TimelineNode }
  | {
      type: "timeline_node_updated";
      node_id: string;
      status: TimelineNodeStatus;
      summary?: string | null;
      completed_at?: string | null;
    }
  | {
      type: "review_complete";
      node_id: string;
      round: number;
      verdict: ReviewVerdictType;
      comments: string;
      summary: string;
    }
  | { type: "review_decision_required"; node_id: string; round: number; options: string[] }
  | {
      type: "session_state";
      session_id: string;
      workspace_type: string;
      stage: string;
      superpowers_enabled: boolean;
      openspec_enabled: boolean;
      messages: WsMessage[];
      checkpoints: WsCheckpoint[];
      artifact: string | null;
      providers: WsProviderConfig;
      timeline_nodes: TimelineNode[];
      active_node_id: string | null;
      artifact_versions: ArtifactVersion[];
      timeline_node_details: Record<string, NodeDetail>;
      active_run_id: string | null;
    }
  | { type: "error"; message: string }
  | { type: "protocol_error"; code: string; message: string; context?: unknown }
  | { type: "provider_locked"; snapshot: ProviderConfigSnapshot; locked_at: string }
  | { type: "pong" };
