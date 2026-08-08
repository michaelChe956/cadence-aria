import type {
  ChoiceOption,
  ExecutionEvent,
  WorkspaceChoiceRequestSource,
} from "./workspace";
import type {
  ProviderConfigSnapshot,
  WorkItemExecutionPlan,
  WorkspaceProviderName,
} from "./common";
import type {
  PlanAmendmentManifest,
  PlanRepairRequest,
  PlanRepairSessionSnapshot,
  WorkspaceSessionLink,
} from "./coding-plan-repair";

export type {
  ContractDeltaKind,
  ContractImpactReport,
  ContractValidationFinding,
  ImpactExplanationPath,
  PlanAmendmentManifest,
  PlanDefectClass,
  PlanDefectEvidence,
  PlanRepairImpactScopeReview,
  PlanRepairPackageIdentity,
  PlanRepairRequest,
  PlanRepairSessionSnapshot,
  PlanValidationReportArtifact,
  RepairTarget,
  WorkItemPlanReviewComplete,
  WorkspaceSessionLink,
} from "./coding-plan-repair";

export type CodingAttemptStatus =
  | "created"
  | "running"
  | "waiting_for_human"
  | "blocked"
  | "awaiting_plan_amendment"
  | "applying_plan_amendment"
  | "amendment_apply_failed"
  | "completed"
  | "failed"
  | "aborted";

export type CodingAttemptScope = "work_item" | "work_item_group";

export type CodingAttemptAddress = {
  projectId: string;
  issueId: string;
  attemptId: string;
};

export type CodingExecutionStage =
  | "prepare_context"
  | "worktree_prepare"
  | "coding"
  | "code_review"
  | "review_request"
  | "internal_pr_review"
  | "final_confirm";

export type CodingExecutionUnitStatus =
  | "pending"
  | "running"
  | "waiting_for_human"
  | "completed"
  | "failed"
  | "blocked"
  | "blocked_by_plan_defect"
  | "awaiting_amendment"
  | "needs_revalidation"
  | "stale"
  | "superseded"
  | "skipped";

export type CodingExecutionUnit = {
  unit_id: string;
  logical_work_item_id: string;
  work_item_revision_id: string;
  dependency_logical_work_item_ids: string[];
  order_index: number;
  status: CodingExecutionUnitStatus;
  summary: string | null;
  latest_handoff_revision_id: string | null;
  completion_commit: string | null;
};

export type CodingAttempt = {
  project_id: string;
  issue_id: string;
  attempt_id: string;
  work_item_id: string;
  attempt_scope: CodingAttemptScope;
  work_item_group_id: string | null;
  current_work_item_id: string | null;
  active_unit_id: string | null;
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
export type CodingAgentRole = "author" | "reviewer" | "git" | "system";
export type CodingProviderRole = "coder" | "code_reviewer" | "internal_reviewer";
export type CodingProviderSelectRole =
  | "author"
  | "reviewer"
  | CodingProviderRole;
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
  | "automatic_retry"
  | "manual_retry"
  | "retry_review"
  | "retry_internal_review";

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

export type CodingRoleRunRetryMetadata = {
  cycle_id: string;
  attempt_no: number;
  prior_run_id?: string | null;
};

export type CodingRolePermissionModes = {
  coder: CodingProviderPermissionMode;
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
  retry_metadata?: CodingRoleRunRetryMetadata | null;
  retry_exhausted?: boolean;
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
  push_error: string | null;
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

export type GroupReviewArtifactRef = {
  id: string;
  raw_provider_output_refs: string[];
};

export type GroupReviewArtifactProjection = {
  shard_reports: GroupReviewArtifactRef[];
  reduction_reports: GroupReviewArtifactRef[];
};

export type GroupFinalReadinessStatus = "complete" | "incomplete";

export type GroupFinalReadinessDiagnosticKind =
  | "unit_run_missing"
  | "completion_commit_missing"
  | "code_review_missing"
  | "handoff_missing"
  | "plan_binding_mismatch"
  | "identity_mismatch";

export type GroupFinalReadinessDiagnostic = {
  kind: GroupFinalReadinessDiagnosticKind;
  unit_id: string | null;
  message: string;
};

export type GroupFinalReadinessUnit = {
  unit_id: string;
  logical_work_item_id: string;
  unit_run_id: string | null;
  start_commit: string | null;
  completion_commit: string | null;
  commit_shas: string[];
  diff_ref: string;
  empty_observation: boolean;
  code_review_report_id: string | null;
  review_verdict: CodingReviewVerdict | null;
  review_summary: string | null;
  review_findings: ReviewFinding[] | null;
  review_raw_provider_output_ref: string | null;
  handoff_revision_id: string | null;
  plan_revision_id: string | null;
};

export type GroupFinalReadinessSnapshot = {
  attempt_id: string;
  status: GroupFinalReadinessStatus;
  units: GroupFinalReadinessUnit[];
  diagnostics: GroupFinalReadinessDiagnostic[];
  created_at: string;
};

export type CodingEntryType =
  | { type: "user_message" }
  | { type: "assistant_message" }
  | { type: "tool_call"; tool_name: string; input: unknown }
  | { type: "tool_result"; tool_use_id: string; output: string; is_error: boolean }
  | { type: "stage_gate"; stage: CodingExecutionStage; countdown_seconds: number }
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
  | "send_to_coder"
  | "confirm_stage"
  | "accept_risk"
  | "abort"
  | "retry_push"
  | "manual_fix"
  | "provide_context"
  | "manual_continue"
  | "retry_coding"
  | "retry_review"
  | "retry_internal_review"
  | "retry_group_review_shard"
  | "retry_group_reduction";
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
  diagnostic?: CodingGateDiagnostic | null;
};

export type CodingGateDiagnostic = {
  actual_value?: string | null;
  limit?: string | null;
  phase: string;
  run_failure_code: string;
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
  attempt_scope: CodingAttemptScope;
  work_item_group_id: string | null;
  current_work_item_id: string | null;
  active_unit_id: string | null;
  units: CodingExecutionUnit[];
  provider_config_snapshot: ProviderConfigSnapshot;
  timeline_nodes: CodingTimelineNode[];
  active_node_id: string | null;
  code_review_reports: CodeReviewReport[];
  review_request: ReviewRequest | null;
  internal_pr_review: InternalPrReview | null;
  group_review_artifacts?: GroupReviewArtifactProjection | null;
  group_final_readiness?: GroupFinalReadinessSnapshot | null;
  pending_gates: CodingGateRequired[];
  pending_choices: CodingChoiceGate[];
  role_runs?: CodingRoleRun[];
  work_item_execution_plan: WorkItemExecutionPlan | null;
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
  | { type: "provider_select"; role: CodingProviderSelectRole; provider: WorkspaceProviderName }
  | {
      type: "permission_mode_select";
      role: CodingProviderRole;
      permission_mode: CodingProviderPermissionMode;
    }
  | { type: "max_auto_rework_select"; max_auto_rework: number }
  | { type: "stage_gate_confirm"; stage: CodingExecutionStage }
  | { type: "final_confirm" }
  | { type: "abort_attempt" }
  | { type: "request_manual_pause" }
  | { type: "coding_ping" };

export type CodingWsOutMessage =
  | ({
      type: "coding_session_state";
      project_id: string;
      issue_id: string;
      attempt_id: string;
      attempt_scope: CodingAttemptScope;
      work_item_group_id: string | null;
      current_work_item_id: string | null;
      active_unit_id: string | null;
      units: CodingExecutionUnit[];
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
      work_item_markdown: string | null;
      verification_commands: string[];
      work_item_execution_plan: WorkItemExecutionPlan | null;
      linked_plan_repair: PlanRepairSessionSnapshot | null;
      group_review_artifacts?: GroupReviewArtifactProjection | null;
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
  | { type: "coding_pong" }
  | {
      type: "plan_repair_required";
      request: PlanRepairRequest;
      session_link: WorkspaceSessionLink | null;
    }
  | {
      type: "plan_amendment_updated";
      event_id: string;
      amendment: PlanAmendmentManifest;
    };
