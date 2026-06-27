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

export type WorkItemKind =
  | "backend"
  | "frontend"
  | "integration"
  | "e2e"
  | "docs"
  | "infra"
  | "other";

export type WorkItemExecutionPlanStatus =
  | "not_started"
  | "draft"
  | "confirmed"
  | "change_requested";

export type WorkItemDependencyHandoffRef = {
  work_item_id: string;
  summary_ref: string | null;
  summary: string | null;
  commit_sha: string | null;
};

export type WorkItemHandoff = {
  handoff_id: string;
  work_item_id: string;
  summary: string;
  handoff_summary_ref: string | null;
  dependency_handoffs: WorkItemDependencyHandoffRef[];
  verification_summary: string | null;
  created_at: string;
  updated_at: string;
};

export type WorkItemExecutionPlan = {
  id: string;
  project_id: string;
  issue_id: string;
  work_item_id: string;
  attempt_id: string;
  status: WorkItemExecutionPlanStatus;
  goal: string;
  allowed_write_scopes: string[];
  forbidden_write_scopes: string[];
  dependency_handoffs: WorkItemDependencyHandoffRef[];
  story_refs: string[];
  design_refs: string[];
  openspec_refs: string[];
  superpowers_contract: string;
  tdd_contract: string;
  verification_plan_ref: string;
  verification_summary: string;
  risk_notes: string[];
  created_at: string;
  updated_at: string;
};

export type WorkItemContextBudget = {
  target_context_k: string;
  max_summary_chars: number;
  max_handoff_chars: number;
  max_code_context_chars: number;
  max_context_file_refs: number;
  max_traceability_refs: number;
  max_dependency_handoffs: number;
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
export type AuthorDecision = "accept" | "reject";

export type ReviewVerdictType = "pass" | "revise" | "needs_human";
export type WorkspaceReviewFindingSeverity =
  | "blocking"
  | "must_fix"
  | "strong_recommend_fix"
  | "suggestion"
  | "minor"
  | "optional";
export type ReviewGate =
  | "requires_revision"
  | "user_confirm_allowed"
  | "user_triage_required";
