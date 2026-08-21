import type { RealProviderName } from "./provider";

export type RepositoryRegistrationErrorDetails = Record<string, unknown> & {
  stage?: string;
  provider?: RealProviderName | null;
  command?: string | null;
  reason_code?: string;
  stderr_summary?: string | null;
  changed_paths?: string[];
  retryable?: boolean;
  action?: string;
};

export type ApiError = {
  code: string;
  message: string;
  details: RepositoryRegistrationErrorDetails;
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

export type RepositoryDeletionReceipt = {
  physical_repository_id: string;
  logical_repository_id: string | null;
  checkout_id: string | null;
  tombstone_operation_id: string | null;
  deleted_at: string;
  legacy_delete: boolean;
};

export type CreateRepositoryRequest = {
  name: string;
  path: string;
  default_policy_preset?: string | null;
  default_provider_mode?: string | null;
};

export type RepositoryInitializationSource =
  | "online_clone"
  | "online_update"
  | "offline";

export type RepositoryInitializationCommandStatus = "completed";

export type RepositoryInitializationCommand = {
  index: number;
  command: string;
  status: RepositoryInitializationCommandStatus;
};

export type RepositoryInitializationSummary = {
  source: RepositoryInitializationSource;
  commands: RepositoryInitializationCommand[];
  warnings: string[];
  changed_paths: string[];
  git_finalize_warning: string | null;
  completed_at: string;
};

export type CreateRepositoryResponse = {
  repository: Repository;
  initialization: RepositoryInitializationSummary;
};

export type RepositoryInitializationOperationStatus =
  | "created"
  | "running"
  | "completed"
  | "failed";

export type RepositoryInitializationStepId =
  | "cadence_skills"
  | "pre_check"
  | "rule_config"
  | "mcp_configuration"
  | "project_rules_examples"
  | "git_finalize";

export type RepositoryInitializationStepStatus =
  | "pending"
  | "running"
  | "completed"
  | "failed";

export type RepositoryInitializationStep = {
  step_id: RepositoryInitializationStepId;
  status: RepositoryInitializationStepStatus;
};

export type RepositoryInitializationOperationSnapshot = {
  operation_id: string;
  status: RepositoryInitializationOperationStatus;
  steps: RepositoryInitializationStep[];
  current_step: RepositoryInitializationStepId | null;
  failed_step: RepositoryInitializationStepId | null;
  result: CreateRepositoryResponse | null;
  error: ApiError | null;
  created_at: string;
  updated_at: string;
  completed_at: string | null;
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
  /// v1.3：逻辑代码库归属；逻辑 issue 时必填，repository_id 为其 active primary 成员。
  logical_codebase_id?: string | null;
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
  commit_sha: string | null;
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
  max_code_context_chars: number;
  max_context_file_refs: number;
  max_traceability_refs: number;
};

export type WorkspaceProviderName = RealProviderName | "fake";

export type ProviderWorkspaceConfig = {
  author_provider: WorkspaceProviderName;
  reviewer_provider: WorkspaceProviderName;
  review_rounds: number;
  superpowers_enabled: boolean;
  openspec_enabled: boolean;
};

export type ProviderWorkspaceConfigInput = Partial<ProviderWorkspaceConfig>;

export type ProviderPermissionMode = "auto" | "supervised";

export type ProviderConfigSnapshot = {
  author: WorkspaceProviderName;
  reviewer?: WorkspaceProviderName | null;
  review_rounds: number;
  permission_modes?: {
    author: ProviderPermissionMode;
    reviewer: ProviderPermissionMode;
  };
};

export type StructuredFeedback = {
  feedback_types: string[];
  description: string;
  target_artifact_version?: number | null;
};

export type RevisionPath = "revise" | "revise-with-context" | "skip-to-human";
export type HumanConfirmDecision = "confirm" | "request-change" | "terminate";

/**
 * author_decision 可发送的决策形式（spec-design-dialog-revision T1/T8）。
 * - "accept"/"reject"：兼容保留（reject 仅 WorkItemPlan outline 等存量分支使用）；
 * - "revise"：携带反馈的对话式修订，发送时构造 `{revise: {feedback}}`；
 * - "accept_with_review"/"accept_finalize"：确认分流（送审/直接定稿）。
 */
export type AuthorDecisionChoice =
  | "accept"
  | "reject"
  | "revise"
  | "accept_with_review"
  | "accept_finalize";

/** WS 线格式：字符串变体或 `{revise: {feedback}}`（serde externally tagged）。 */
export type AuthorDecision =
  | AuthorDecisionChoice
  | { revise: { feedback: string } };

export type ReviewVerdictType = "pass" | "revise" | "needs_human";
export type WorkspaceReviewFindingSeverity = "blocking" | "must_fix" | "suggestion";
export type ReviewGate =
  | "requires_revision"
  | "user_confirm_allowed"
  | "user_triage_required";
