import type { TimelineNode } from "./workspace";
import type {
  DependencyContractEdge,
  PlanProjectionBundle,
  ProjectionValidationReport,
} from "./work-item-plan";

export type PlanDefectClass =
  | "implementation_defect"
  | "verification_incomplete"
  | "current_work_item_invalid"
  | "upstream_contract_invalid"
  | "dependency_graph_invalid"
  | "design_amendment_required"
  | "story_amendment_required"
  | "operational_blocker";

export type RepairTarget = {
  kind: "current_work_item" | "upstream_work_item" | "subgraph";
  logical_work_item_ids: string[];
  work_item_revision_ids: string[];
};

export type PlanDefectEvidence = {
  kind: string;
  source_ref: string;
  message: string;
};

export type PlanRepairRequest = {
  id: string;
  plan_id: string;
  base_plan_revision_id: string;
  trigger_attempt_id: string;
  trigger_unit_run_id: string;
  trigger_review_id: string | null;
  trigger_finding_id: string;
  amendment_id: string | null;
  defect_class: PlanDefectClass;
  reason_code: string;
  repair_target: RepairTarget;
  contract_refs: string[];
  capability_refs: string[];
  evidence: PlanDefectEvidence[];
  fingerprint: string;
  status:
    | "open"
    | "in_progress"
    | "awaiting_confirmation"
    | "published"
    | "applied"
    | "cancelled"
    | "failed";
  created_at: string;
  updated_at: string;
};

export type WorkspaceSessionLink = {
  id: string;
  relation: "plan_repair" | "story_amendment" | "design_amendment";
  parent_session_id: string;
  child_session_id: string;
  trigger: {
    attempt_id: string;
    unit_run_id: string;
    review_id: string | null;
    finding_id: string;
    repair_request_id: string;
    amendment_id: string;
    fingerprint: string;
    base_plan_revision_id: string;
  };
  return_context: {
    original_attempt_id: string;
    original_unit_run_id: string;
    timeline_anchor_id: string;
    original_route: string;
  };
  created_at: string;
};

export type ContractDeltaKind =
  | "informative_only"
  | "implementation_guidance"
  | "compatible_contract_extension"
  | "breaking_contract_change"
  | "topology_change";

export type PlanAmendmentManifest = {
  id: string;
  repair_request_id: string;
  previous_plan_revision_id: string;
  new_plan_revision_id: string;
  revised_work_items: Record<
    string,
    {
      previous_revision_id: string;
      next_revision_id: string;
      delta_kind: ContractDeltaKind;
    }
  >;
  superseded_revisions: string[];
  dependency_graph_changes: Array<{
    kind: "edge_added" | "edge_removed" | "edge_replaced";
    previous: DependencyContractEdge | null;
    next: DependencyContractEdge | null;
  }>;
  contract_deltas: Array<{
    logical_work_item_id: string;
    previous_revision_id: string;
    next_revision_id: string;
    kind: ContractDeltaKind;
    added_contracts: string[];
    removed_contracts: string[];
    added_capabilities: string[];
    removed_capabilities: string[];
    changed_capabilities: string[];
    added_capability_associations: Array<{ contract_id: string; capability: string }>;
    removed_capability_associations: Array<{ contract_id: string; capability: string }>;
    acceptance_changed: boolean;
    verification_changed: boolean;
    write_policy_changed: boolean;
  }>;
  unaffected_units: string[];
  revalidation_required_units: string[];
  stale_units: string[];
  replacement_units: Record<string, string[]>;
  resume_target: {
    logical_work_item_id: string;
    mode: "reexecute" | "revalidate" | "await_handoff";
  };
  created_at: string;
};

export type ContractValidationFinding = {
  code: string;
  severity: "Warning" | "Error";
  logical_work_item_id: string | null;
  contract_ref: string | null;
  capability_ref: string | null;
  message: string;
};

export type PlanValidationReportArtifact = {
  id: string;
  plan_id: string;
  plan_revision_id: string;
  plan_projection_bundle_id: string;
  contract_validation: {
    findings: ContractValidationFinding[];
  };
  projection_validation: ProjectionValidationReport;
  created_at: string;
};

export type ImpactExplanationPath = {
  from: string;
  to: string;
  contract_id: string;
  capability_refs: string[];
};

export type ContractImpactReport = {
  unaffected: string[];
  direct_revalidation: string[];
  direct_stale: string[];
  conditional_downstream: string[];
  explanation_paths: ImpactExplanationPath[];
};

export type WorkItemPlanReviewComplete = {
  verdict: "pass" | "revise" | "revise_batch" | "needs_human" | "plan_reopen_required";
  review_scope: "outline" | "item" | "batch";
  target_outline_id: string | null;
  generation_round_id: string;
  draft_id: string | null;
  batch_id: string | null;
  review_action:
    | "continue"
    | "revise_outline"
    | "revise_current_item"
    | "revise_batch"
    | "human_triage";
  gates: Array<
    | "requires_current_item_revision"
    | "requires_batch_revision"
    | "requires_plan_reopen"
  >;
  affects_items?: Array<{
    outline_index: number | null;
    target_outline_id: string | null;
  }>;
  warnings?: string[];
};

export type PlanRepairPackageIdentity = {
  request_id: string;
  amendment_id: string;
  plan_id: string;
  base_plan_revision_id: string;
  next_plan_revision_id: string;
  projection_bundle_id: string;
  validation_report_id: string;
  review_attestation_id: string;
  reviewed_plan_revision_id: string;
  review_generation_round_id: string;
  candidate_package_artifact_id: string;
  candidate_package_fingerprint: string;
};

export type PlanRepairImpactScopeReview = {
  system_minimum_impact_scope: string[];
  proposed_accepted_impact_scope: string[];
  risk_acceptance_reason: string;
  candidate_package_fingerprint: string;
  review_generation_round_id: string;
};

export type PlanRepairSessionSnapshot = {
  request: PlanRepairRequest;
  link: WorkspaceSessionLink;
  stage:
    | "triaging"
    | "authoring_revision"
    | "validating_contract"
    | "generating_projections"
    | "plan_review"
    | "awaiting_confirmation"
    | "published"
    | "amendment_conflict"
    | "applying_amendment"
    | "amendment_apply_failed"
    | "completed"
    | "failed";
  projection: PlanProjectionBundle | null;
  amendment: PlanAmendmentManifest | null;
  validation: PlanValidationReportArtifact | null;
  impact: ContractImpactReport | null;
  plan_review: WorkItemPlanReviewComplete | null;
  package_identity: PlanRepairPackageIdentity | null;
  candidate_package_artifact_id: string | null;
  impact_scope_review: PlanRepairImpactScopeReview | null;
  timeline_nodes: TimelineNode[];
  error: string | null;
};
