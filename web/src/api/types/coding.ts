import type {
  ChoiceOption,
  ExecutionEvent,
  WorkspaceChoiceRequestSource,
} from "./workspace";
import type {
  ProviderConfigSnapshot,
  WorkItemExecutionPlan,
  WorkItemHandoff,
  WorkspaceProviderName,
} from "./common";

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
  work_item_execution_plan: WorkItemExecutionPlan | null;
  work_item_handoff: WorkItemHandoff | null;
  require_execution_plan_confirm: boolean;
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
  | { type: "continue_rework"; extra_context?: string | null }
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
      work_item_execution_plan: WorkItemExecutionPlan | null;
      work_item_handoff: WorkItemHandoff | null;
      require_execution_plan_confirm: boolean;
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
