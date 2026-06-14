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
  latest_attempt: CodingAttempt | null;
  artifact_versions: ArtifactVersion[];
};

export type CodingAttemptStatus =
  | "created"
  | "running"
  | "waiting_for_human"
  | "blocked"
  | "completed"
  | "failed"
  | "aborted";

export type CodingExecutionStage =
  | "prepare_context"
  | "worktree_prepare"
  | "coding"
  | "testing"
  | "code_review"
  | "rework"
  | "review_request"
  | "internal_pr_review"
  | "final_confirm";

export type CodingAttempt = {
  attempt_id: string;
  work_item_id: string;
  attempt_no: number;
  status: CodingAttemptStatus;
  stage: CodingExecutionStage;
  branch_name: string;
  base_branch: string;
  worktree_path: string | null;
  rework_count: number;
  head_commit: string | null;
  push_status: "not_pushed" | "pushed" | "failed" | null;
  review_request_url: string | null;
  created_at: string;
  updated_at: string;
};

export type CodingTimelineNodeStatus = "pending" | "running" | "completed" | "failed" | "blocked";
export type CodingAgentRole = "author" | "tester" | "reviewer" | "git" | "system";
export type CodingProviderRole =
  | "coder"
  | "tester"
  | "analyst"
  | "code_reviewer"
  | "internal_reviewer";
export type CodingProviderSelectRole = "author" | "reviewer" | CodingProviderRole;
export type CodingProviderPermissionMode = "auto" | "supervised";
export type CodingRoleRunStatus =
  | "running"
  | "completed"
  | "failed"
  | "blocked"
  | "superseded"
  | "aborted";
export type CodingRoleRunTrigger =
  | "initial"
  | "retry_test_plan"
  | "rerun_missing_steps"
  | "retry_review"
  | "retry_analyst"
  | "retry_internal_review"
  | "manual_rerun";

export type CodingRoleRunEventType =
  | "provider_prompt"
  | "provider_start"
  | "text_delta"
  | "execution_event"
  | "tool_call"
  | "tool_result"
  | "status_changed"
  | "permission_request"
  | "choice_request"
  | "message_complete"
  | "provider_failed"
  | "timeout"
  | "aborted"
  | "persistence_warning";

export type CodingRoleRunEventSummary = {
  event_count: number;
  last_event_at?: string | null;
  last_event_type?: CodingRoleRunEventType | null;
  last_event_title?: string | null;
  last_event_status?: string | null;
  terminal_event_type?: CodingRoleRunEventType | null;
  terminal_reason?: string | null;
};

export type CodingRoleRunEventPreview = {
  sequence: number;
  event_type: CodingRoleRunEventType;
  created_at: string;
  title?: string | null;
  status?: string | null;
  detail?: string | null;
  truncated: boolean;
  artifact_ref?: string | null;
};

export type CodingRolePermissionModes = {
  coder: CodingProviderPermissionMode;
  tester: CodingProviderPermissionMode;
  analyst: CodingProviderPermissionMode;
  code_reviewer: CodingProviderPermissionMode;
  internal_reviewer: CodingProviderPermissionMode;
};

export type CodingTimelineNode = {
  id: string;
  attempt_id: string;
  stage: CodingExecutionStage;
  title: string;
  status: CodingTimelineNodeStatus;
  agent_role: CodingAgentRole | null;
  summary: string | null;
  started_at: string;
  completed_at: string | null;
  artifact_refs: string[];
};

export type CodingRoleProviderConfigSnapshot = {
  coder: WorkspaceProviderName;
  tester: WorkspaceProviderName;
  analyst: WorkspaceProviderName;
  code_reviewer: WorkspaceProviderName;
  internal_reviewer: WorkspaceProviderName;
  review_rounds: number;
  permission_modes: CodingRolePermissionModes;
};

export type CodingRoleRun = {
  id: string;
  attempt_id: string;
  stage: CodingExecutionStage;
  role: CodingProviderRole;
  run_no: number;
  status: CodingRoleRunStatus;
  trigger: CodingRoleRunTrigger;
  node_id: string | null;
  started_at: string;
  completed_at: string | null;
  supersedes_run_id?: string | null;
  superseded_by_run_id?: string | null;
  reason_code?: string | null;
  raw_provider_output_refs: string[];
  artifact_refs: string[];
  event_summary?: CodingRoleRunEventSummary | null;
  recent_events?: CodingRoleRunEventPreview[];
};

export type TestCommandStatus = "passed" | "failed" | "timed_out" | "blocked";
export type TestingOverallStatus =
  | "passed"
  | "passed_with_warnings"
  | "failed"
  | "skipped_by_user_decision"
  | "blocked";

export type TestCommand = {
  command: string[];
  cwd: string;
  exit_code: number | null;
  duration_ms: number;
  stdout_ref: string;
  stderr_ref: string;
  status: TestCommandStatus;
};

export type TestPlanTool =
  | "run_command"
  | "read_file"
  | "list_files"
  | "search_code"
  | "provider_managed";
export type TestPlanRiskLevel = "low" | "medium" | "high";

export type TestPlanStep = {
  id: string;
  title: string;
  intent: string;
  required: boolean;
  tool: TestPlanTool;
  risk_level: TestPlanRiskLevel;
  command_or_tool_input: unknown;
  evidence_expectation: string;
  related_requirements?: string[];
  related_design_constraints?: string[];
  related_work_item_tasks?: string[];
};

export type TestingStepResult = {
  step_id: string;
  status: TestCommandStatus;
  evidence_refs?: string[];
  command?: string[] | null;
  provider_analysis?: string | null;
};

export type TestingUnplannedEvidence = {
  tool_use_id: string;
  tool_name: string;
  status: TestCommandStatus;
  evidence_refs?: string[];
  provider_analysis?: string | null;
};

export type TestingReport = {
  id: string;
  attempt_id: string;
  role_run_id?: string | null;
  run_no?: number | null;
  commands: TestCommand[];
  overall_status: TestingOverallStatus;
  provider_claim: unknown | null;
  backend_verified: boolean;
  started_at: string;
  completed_at: string | null;
  plan_id?: string | null;
  plan_summary?: string | null;
  steps?: TestingStepResult[];
  unplanned_commands?: TestCommand[];
  unplanned_evidence?: TestingUnplannedEvidence[];
  missing_required_steps?: string[];
  skipped_required_steps?: string[];
  context_warnings?: string[];
  raw_provider_output_ref?: string | null;
};

export type CodingReviewVerdict = "approve" | "request_changes" | "blocked";
export type FindingSeverity = "error" | "warning" | "info";

export type ReviewFinding = {
  severity: FindingSeverity;
  file_path: string | null;
  line: number | null;
  message: string;
  required_action: string | null;
  source_stage: CodingExecutionStage;
  evidence?: string[];
  related_requirements?: string[];
  related_design_constraints?: string[];
  related_work_item_tasks?: string[];
};

export type CodeReviewReport = {
  id: string;
  attempt_id: string;
  round: number;
  verdict: CodingReviewVerdict;
  findings: ReviewFinding[];
  tested_evidence_refs: string[];
  diff_refs: string[];
  summary: string;
  created_at: string;
  raw_provider_output_ref?: string | null;
  role_run_id?: string | null;
  run_no?: number | null;
};

export type ReviewRequestKind =
  | "git_branch_only"
  | "gitlab_merge_request"
  | "github_pull_request"
  | "manual_external_request";
export type RemoteKind = "github" | "gitlab" | "generic_git" | "unknown";
export type PushStatus = "not_pushed" | "pushed" | "failed";

export type ReviewRequest = {
  id: string;
  attempt_id: string;
  kind: ReviewRequestKind;
  remote_kind: RemoteKind;
  remote: string;
  base_branch: string;
  branch_name: string;
  commit_sha: string;
  push_status: PushStatus;
  external_url: string | null;
  manual_instructions: string[];
  created_at: string;
  updated_at: string;
};

export type InternalPrReview = {
  id: string;
  attempt_id: string;
  review_request_id: string;
  verdict: CodingReviewVerdict;
  findings: ReviewFinding[];
  impact_scope: string[];
  pr_description: string;
  commit_message_suggestion: string;
  tested_evidence_refs: string[];
  diff_refs: string[];
  summary: string;
  created_at: string;
  raw_provider_output_ref?: string | null;
  role_run_id?: string | null;
  run_no?: number | null;
};

export type AnalystVerdict = "needs_fix" | "needs_human_input" | "no_issue";
export type AnalystDecisionVerdict =
  | "needs_fix"
  | "rerun_testing"
  | "proceed"
  | "human_required"
  | "blocked";
export type AnalystDecisionNextStage =
  | "coding"
  | "testing"
  | "code_review"
  | "review_request"
  | "internal_pr_review"
  | "final_confirm"
  | "human_gate";

export type AnalystReworkInstructions = {
  summary: string;
  required_changes: string[];
  verification_expectations: string[];
};

export type AnalystHumanGateRecommendation = {
  reason_code?: string | null;
  available_actions: string[];
};

export type AnalystDecisionRecord = {
  id: string;
  attempt_id: string;
  source_stage: CodingExecutionStage;
  rework_round: number;
  verdict: AnalystDecisionVerdict;
  next_stage: AnalystDecisionNextStage;
  reason: string;
  evidence_refs: string[];
  raw_provider_output_refs: string[];
  rework_instructions?: AnalystReworkInstructions | null;
  human_gate?: AnalystHumanGateRecommendation | null;
  created_at: string;
  parse_error?: string | null;
  role_run_id?: string | null;
  run_no?: number | null;
};

export type CodingEntryType =
  | { type: "user_message" }
  | { type: "assistant_message" }
  | { type: "tool_call"; tool_name: string; input: unknown }
  | { type: "tool_result"; tool_use_id: string; output: string; is_error: boolean }
  | { type: "stage_gate"; stage: CodingExecutionStage; countdown_seconds: number }
  | { type: "analyst_verdict"; verdict: AnalystVerdict }
  | { type: "stage_summary"; stage: CodingExecutionStage; summary: string }
  | { type: "system_event"; event_type: string; message: string };

export type CodingChatEntry = {
  id: string;
  attempt_id: string;
  node_id: string | null;
  role: CodingAgentRole;
  entry_type: CodingEntryType;
  content: string | null;
  metadata: Record<string, unknown> | null;
  created_at: string;
};

export type CodingGateActionType =
  | "continue_rework"
  | "confirm_stage"
  | "accept_risk"
  | "abort"
  | "retry_push"
  | "manual_fix"
  | "retry_test_plan"
  | "rerun_missing_steps"
  | "provide_context"
  | "manual_continue"
  | "retry_review"
  | "retry_analyst"
  | "retry_internal_review"
  | "send_raw_output_to_analyst"
  | "accept_testing_result"
  | "rerun_testing";
export type CodingGateKind = "permission" | "stage_gate" | "blocked" | "final_confirm";

export type CodingGateAction = {
  action_id: string;
  label: string;
  action_type: CodingGateActionType;
};

export type CodingGateRequired = {
  gate_id: string;
  kind: CodingGateKind;
  title: string;
  description: string;
  stage?: CodingExecutionStage | null;
  role?: CodingProviderRole | null;
  expires_at?: string | null;
  provider_snapshot?: CodingRoleProviderConfigSnapshot | null;
  available_actions: CodingGateAction[];
  reason_code?: string | null;
  evidence_refs?: string[];
  raw_provider_output_ref?: string | null;
};

export type CodingChoiceGateStatus = "open" | "resolved" | "stale" | "cancelled";

export type CodingChoiceGateResponse = {
  selected_option_ids: string[];
  free_text?: string | null;
  responded_at: string;
};

export type CodingChoiceGate = {
  gate_id: string;
  choice_id: string;
  attempt_id: string;
  node_id?: string | null;
  stage: CodingExecutionStage;
  role: CodingProviderRole;
  provider: WorkspaceProviderName;
  source: WorkspaceChoiceRequestSource;
  prompt: string;
  options: ChoiceOption[];
  allow_multiple: boolean;
  allow_free_text: boolean;
  status: CodingChoiceGateStatus;
  response?: CodingChoiceGateResponse | null;
  created_at: string;
  updated_at: string;
};

export type CodingAttemptSnapshotResponse = {
  attempt: CodingAttempt;
  provider_config_snapshot: ProviderConfigSnapshot;
  timeline_nodes: CodingTimelineNode[];
  active_node_id: string | null;
  testing_report: TestingReport | null;
  code_review_reports: CodeReviewReport[];
  review_request: ReviewRequest | null;
  internal_pr_review: InternalPrReview | null;
  pending_gates: CodingGateRequired[];
  pending_choices: CodingChoiceGate[];
  latest_analyst_decision: AnalystDecisionRecord | null;
  role_runs?: CodingRoleRun[];
};

export type CodingAttemptDiffResponse = {
  attempt_id: string;
  base_branch: string;
  worktree_path: string;
  diff: string;
};

export type ArtifactContentResponse = {
  artifact_ref: string;
  artifact_kind: string;
  producer_node: string | null;
  path: string;
  content_type: string;
  content: string;
};

export type CodingWsInMessage =
  | { type: "coding_hello"; attempt_id: string; last_seen_node_id?: string | null }
  | { type: "start_coding" }
  | { type: "context_note"; content: string }
  | { type: "permission_response"; id: string; approved: boolean; reason?: string | null }
  | {
      type: "choice_response";
      id: string;
      selected_option_ids: string[];
      free_text?: string | null;
    }
  | {
      type: "gate_response";
      gate_id: string;
      action_id: string;
      extra_context?: string | null;
    }
  | { type: "provider_select"; role: CodingProviderSelectRole; provider: WorkspaceProviderName }
  | {
      type: "permission_mode_select";
      role: CodingProviderRole;
      permission_mode: CodingProviderPermissionMode;
    }
  | { type: "stage_gate_confirm"; stage: CodingExecutionStage }
  | { type: "final_confirm" }
  | { type: "abort_attempt" }
  | { type: "request_manual_pause" }
  | { type: "coding_ping" };

export type CodingWsOutMessage =
  | ({
      type: "coding_session_state";
      attempt_id: string;
      status: CodingAttemptStatus;
      stage: CodingExecutionStage;
      branch_name: string;
      base_branch: string;
      worktree_path: string | null;
      rework_count: number;
      max_auto_rework: number;
      head_commit: string | null;
      pushed_remote: string | null;
      role_provider_config_snapshot: CodingRoleProviderConfigSnapshot;
      chat_entries: CodingChatEntry[];
    } & Omit<CodingAttemptSnapshotResponse, "attempt">)
  | { type: "coding_stage_change"; stage: CodingExecutionStage }
  | { type: "coding_timeline_node_created"; node: CodingTimelineNode }
  | {
      type: "coding_timeline_node_updated";
      node_id: string;
      status: CodingTimelineNodeStatus;
      summary?: string | null;
      completed_at?: string | null;
    }
  | { type: "coding_execution_event"; event: ExecutionEvent }
  | {
      type: "coding_permission_request";
      id: string;
      tool_name: string;
      description: string;
      risk_level: "low" | "medium" | "high";
    }
  | {
      type: "coding_choice_request";
      id: string;
      prompt: string;
      source: WorkspaceChoiceRequestSource;
      options: ChoiceOption[];
      allow_multiple: boolean;
      allow_free_text: boolean;
    }
  | {
      type: "coding_choice_response_ack";
      id: string;
      selected_option_ids: string[];
      free_text?: string | null;
    }
  | { type: "coding_stream_chunk"; content: string; node_id?: string | null }
  | { type: "coding_message_complete"; node_id?: string | null }
  | { type: "testing_report_update"; report: TestingReport }
  | { type: "code_review_complete"; report: CodeReviewReport }
  | { type: "review_request_update"; review_request: ReviewRequest }
  | { type: "internal_pr_review_complete"; review: InternalPrReview }
  | { type: "coding_gate_required"; gate: CodingGateRequired }
  | { type: "coding_chat_entry_created"; entry: CodingChatEntry }
  | {
      type: "coding_provider_config_updated";
      role: CodingProviderRole;
      provider: WorkspaceProviderName;
    }
  | { type: "coding_protocol_error"; code: string; message: string }
  | { type: "coding_pong" };

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
  coding_attempts: CodingAttempt[];
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
export type AuthorDecision = "accept" | "reject";

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
  | {
      type: "choice_response";
      id: string;
      selected_option_ids: string[];
      free_text?: string | null;
    }
  | { type: "review_decision_response"; decision: string; extra_context?: string | null }
  | { type: "author_decision"; decision: AuthorDecision }
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
  | "author_confirm"
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

export type WorkspaceReviewFinding = {
  severity: WorkspaceReviewFindingSeverity;
  message: string;
  evidence: string;
  impact: string;
  required_action: string;
};

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
  findings?: WorkspaceReviewFinding[];
  review_gate?: ReviewGate;
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

export type ChoiceOption = {
  id: string;
  label: string;
  description?: string | null;
};

export type WorkspaceChoiceRequestSource =
  | "ask_user_question"
  | "request_user_input"
  | "text_fallback"
  | "provider_choice";

export type ArtifactVersion = {
  version: number;
  markdown: string;
  generated_by: WorkspaceProviderName;
  reviewed_by?: WorkspaceProviderName | null;
  review_verdict?: ReviewVerdictType | null;
  confirmed_by?: string | null;
  is_current?: boolean;
  created_at: string;
  source_node_id: string;
};

export type ArtifactVersionSummary = Omit<ArtifactVersion, "markdown"> & { markdown?: string };

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

export type WorkspaceNodeDetailResponse = NodeDetail;

export type WorkspacePromptResponse = {
  node_id: string;
  prompt: string;
};

export type WorkspaceEventOutputResponse = {
  node_id: string;
  event_id: string;
  output: string;
};

export type WorkspaceArtifactVersionResponse = {
  version: number;
  markdown: string;
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
  | {
      type: "choice_request";
      id: string;
      prompt: string;
      options: ChoiceOption[];
      allow_multiple: boolean;
      allow_free_text: boolean;
      source: WorkspaceChoiceRequestSource;
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
      findings?: WorkspaceReviewFinding[];
      review_gate?: ReviewGate;
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
      artifact_version_summaries?: ArtifactVersionSummary[];
      timeline_node_details: Record<string, NodeDetail>;
      active_run_id: string | null;
    }
  | { type: "error"; message: string }
  | { type: "protocol_error"; code: string; message: string; context?: unknown }
  | { type: "provider_locked"; snapshot: ProviderConfigSnapshot; locked_at: string }
  | { type: "pong" };
