use super::*;
use crate::product::coding_models::CodingUnitRun;
use crate::product::models::{PlanDefectRoute, RepairTargetKind};
use crate::product::plan_repair::{
    PlanDefectFinding, PlanDefectSeverity, PlanRepairError, normalize_blocker_route,
};
use crate::product::work_item_projection::ReviewerWorkItemProjection;
use crate::product::work_item_revision_store::WorkItemRevisionStore;

#[derive(Debug, Clone)]
pub(crate) struct GroupReviewerProjectionBinding {
    pub(crate) logical_work_item_id: String,
    pub(crate) projection: ReviewerWorkItemProjection,
}

#[derive(Debug, Clone)]
pub(crate) struct AuthoritativeGroupReviewerBinding {
    pub(crate) run: CodingUnitRun,
    pub(crate) projection_binding: GroupReviewerProjectionBinding,
}

pub(crate) fn internal_review_flow_decision(
    review: &InternalPrReview,
    reviewer_projection: &ReviewerWorkItemProjection,
) -> CodeReviewFlowDecision {
    let _source = PlanDefectSource::GroupReviewer;
    review_findings_flow_decision(&review.findings, &review.verdict, reviewer_projection)
}

pub(crate) fn internal_review_flow_decision_with_bindings(
    review: &InternalPrReview,
    bindings: &[GroupReviewerProjectionBinding],
) -> CodeReviewFlowDecision {
    let _source = PlanDefectSource::GroupReviewer;
    review_findings_flow_decision_with_validator(&review.findings, &review.verdict, |finding| {
        validate_group_reviewer_finding(finding, bindings)
    })
}

impl CodingWorkspaceEngine {
    pub(crate) fn internal_review_flow_decision_for_attempt(
        &self,
        attempt: &CodingExecutionAttempt,
        review: &InternalPrReview,
    ) -> Result<CodeReviewFlowDecision, CodingWorkspaceEngineError> {
        if attempt.scope != crate::product::coding_models::CodingAttemptScope::WorkItemGroup {
            let projection = self.reviewer_projection_for_attempt(attempt)?;
            return Ok(internal_review_flow_decision(review, &projection));
        }
        let bindings = self.group_reviewer_projection_bindings(attempt)?;
        Ok(internal_review_flow_decision_with_bindings(
            review, &bindings,
        ))
    }

    pub(crate) fn group_reviewer_projection_bindings(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<Vec<GroupReviewerProjectionBinding>, CodingWorkspaceEngineError> {
        Ok(self
            .authoritative_group_reviewer_bindings(attempt)?
            .into_iter()
            .map(|binding| binding.projection_binding)
            .collect())
    }

    pub(crate) fn authoritative_group_reviewer_bindings(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<Vec<AuthoritativeGroupReviewerBinding>, CodingWorkspaceEngineError> {
        let plan_id = attempt.work_item_group_id.as_deref().ok_or_else(|| {
            CodingWorkspaceEngineError::ProviderStream(
                "group_review_plan_binding_missing".to_string(),
            )
        })?;
        let revision_store = WorkItemRevisionStore::new(self.store.paths());
        let lineage =
            revision_store.get_plan_lineage(&attempt.project_id, &attempt.issue_id, plan_id)?;
        let mut units =
            self.store
                .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        units.sort_by(|left, right| {
            left.order_index
                .cmp(&right.order_index)
                .then_with(|| left.id.cmp(&right.id))
        });
        let mut bindings = Vec::with_capacity(units.len());
        for unit in units {
            let run = self
                .store
                .list_coding_unit_runs(attempt, &unit.id)?
                .into_iter()
                .max_by_key(|run| run.execution_no)
                .ok_or_else(|| {
                    CodingWorkspaceEngineError::ProviderStream(format!(
                        "group_review_unit_run_missing: {}",
                        unit.logical_work_item_id
                    ))
                })?;
            if run.status != crate::product::coding_models::CodingUnitRunStatus::Completed {
                return Err(CodingWorkspaceEngineError::ProviderStream(format!(
                    "group_review_unit_run_not_completed: {}",
                    run.id
                )));
            }
            let revision = revision_store.get_work_item_revision(
                &lineage,
                &unit.logical_work_item_id,
                &run.work_item_revision_id,
            )?;
            let bundle = revision_store
                .get_work_item_projection_bundle(&lineage, &run.projection_bundle_id)?;
            validate_projection_bundle(&run.work_item_revision_id, &revision, &bundle)?;
            let resolved_handoff_revision_ids =
                self.authoritative_resolved_handoff_revision_ids(attempt, &unit, &lineage)?;
            if run.resolved_handoff_revision_ids != resolved_handoff_revision_ids {
                return Err(CodingWorkspaceEngineError::ProviderStream(format!(
                    "group_review_handoff_binding_mismatch: {}",
                    run.id
                )));
            }
            if run.work_item_revision_id != unit.work_item_revision_id
                || run.canonical_contract_hash != bundle.canonical_contract_hash
                || run.projection_compiler_version != bundle.compiler_version
                || run.reviewer_projection_hash != bundle.reviewer_projection_hash
            {
                return Err(CodingWorkspaceEngineError::ProviderStream(format!(
                    "group_review_projection_binding_mismatch: {}",
                    run.id
                )));
            }
            bindings.push(AuthoritativeGroupReviewerBinding {
                run,
                projection_binding: GroupReviewerProjectionBinding {
                    logical_work_item_id: unit.logical_work_item_id,
                    projection: bundle.reviewer_projection,
                },
            });
        }
        Ok(bindings)
    }
}

pub(crate) fn unique_authoritative_group_reviewer_binding(
    finding: &ReviewFinding,
    bindings: &[AuthoritativeGroupReviewerBinding],
) -> Result<AuthoritativeGroupReviewerBinding, CodingWorkspaceEngineError> {
    let projection_bindings = bindings
        .iter()
        .map(|binding| binding.projection_binding.clone())
        .collect::<Vec<_>>();
    let matches = matching_group_reviewer_bindings(finding, &projection_bindings);
    let matched = match matches.as_slice() {
        [] => {
            return Err(CodingWorkspaceEngineError::ProviderStream(
                "plan_repair_group_trigger binding is missing".to_string(),
            ));
        }
        [matched] => *matched,
        _ => {
            return Err(CodingWorkspaceEngineError::ProviderStream(
                "plan_repair_group_trigger binding is ambiguous".to_string(),
            ));
        }
    };
    let mut authoritative = bindings.iter().filter(|binding| {
        binding.projection_binding.logical_work_item_id == matched.logical_work_item_id
            && binding.projection_binding.projection.work_item_revision_id
                == matched.projection.work_item_revision_id
    });
    let selected = authoritative.next().cloned().ok_or_else(|| {
        CodingWorkspaceEngineError::ProviderStream(
            "plan_repair_group_trigger binding is missing".to_string(),
        )
    })?;
    if authoritative.next().is_some() {
        return Err(CodingWorkspaceEngineError::ProviderStream(
            "plan_repair_group_trigger binding is ambiguous".to_string(),
        ));
    }
    Ok(selected)
}

fn validate_group_reviewer_finding(
    finding: &ReviewFinding,
    bindings: &[GroupReviewerProjectionBinding],
) -> Result<(), PlanRepairError> {
    if finding.defect_class == crate::product::models::PlanDefectClass::ImplementationDefect {
        return validate_plan_defect_finding(finding, &empty_reviewer_projection());
    }
    let matches = matching_group_reviewer_bindings(finding, bindings);
    if matches.is_empty() {
        return Err(PlanRepairError::InvalidFinding(
            "group reviewer projection blocker rule is missing".to_string(),
        ));
    }
    let mut signatures = matches
        .iter()
        .filter_map(|binding| blocker_rule_signature(finding, &binding.projection));
    let Some(first) = signatures.next() else {
        return Err(PlanRepairError::InvalidFinding(
            "group reviewer projection blocker rule is missing".to_string(),
        ));
    };
    if signatures.any(|signature| signature != first) {
        return Err(PlanRepairError::InvalidFinding(
            "group reviewer projection blocker rule is ambiguous".to_string(),
        ));
    }
    Ok(())
}

fn matching_group_reviewer_bindings<'a>(
    finding: &ReviewFinding,
    bindings: &'a [GroupReviewerProjectionBinding],
) -> Vec<&'a GroupReviewerProjectionBinding> {
    let candidates = match finding.repair_target.as_ref() {
        Some(target) => {
            let all_targets_exist = target.logical_work_item_ids.iter().all(|logical_id| {
                bindings.iter().any(|binding| {
                    binding.logical_work_item_id == *logical_id
                        && target
                            .work_item_revision_ids
                            .contains(&binding.projection.work_item_revision_id)
                })
            });
            if !all_targets_exist {
                return Vec::new();
            }
            match target.kind {
                RepairTargetKind::UpstreamWorkItem => bindings.iter().collect::<Vec<_>>(),
                RepairTargetKind::CurrentWorkItem | RepairTargetKind::Subgraph => bindings
                    .iter()
                    .filter(|binding| {
                        target
                            .logical_work_item_ids
                            .contains(&binding.logical_work_item_id)
                            && target
                                .work_item_revision_ids
                                .contains(&binding.projection.work_item_revision_id)
                    })
                    .collect::<Vec<_>>(),
            }
        }
        None => bindings.iter().collect::<Vec<_>>(),
    };
    candidates
        .into_iter()
        .filter(|binding| validate_plan_defect_finding(finding, &binding.projection).is_ok())
        .collect()
}

fn blocker_rule_signature(
    finding: &ReviewFinding,
    projection: &ReviewerWorkItemProjection,
) -> Option<(PlanDefectRoute, Option<RepairTargetKind>, Vec<String>)> {
    let reason_code = finding.reason_code.as_deref()?;
    let rule = projection
        .blocker_routing
        .iter()
        .find(|rule| rule.reason_code == reason_code)?;
    let normalized = normalize_blocker_route(rule.route.clone());
    let mut refs = rule.target_contract_refs.clone();
    refs.sort();
    refs.dedup();
    Some((normalized.route, normalized.required_target_kind, refs))
}

pub(crate) fn execution_plan_defect_flow_decision(
    report: &ExecutionPlanDefectReport,
    reviewer_projection: &ReviewerWorkItemProjection,
) -> CodeReviewFlowDecision {
    let findings = report
        .findings
        .iter()
        .map(|finding| execution_finding_adapter(&report.source, finding))
        .collect::<Vec<_>>();
    review_findings_flow_decision(&findings, &ReviewVerdict::Blocked, reviewer_projection)
}

impl CodeReviewFlowDecision {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::RunCoderFix => "run_coder_fix",
            Self::RetryVerification => "retry_verification",
            Self::StartPlanRepair => "start_plan_repair",
            Self::StartStoryAmendment => "start_story_amendment",
            Self::StartDesignAmendment => "start_design_amendment",
            Self::OpenOperationalGate => "open_operational_gate",
            Self::StopForHumanTriage => "stop_for_human_triage",
            Self::ContinueAfterApprove => "continue_after_approve",
        }
    }
}

pub(crate) fn execution_finding_adapter(
    source: &PlanDefectSource,
    finding: &PlanDefectFinding,
) -> ReviewFinding {
    ReviewFinding {
        severity: match finding.severity {
            PlanDefectSeverity::Error => FindingSeverity::Error,
            PlanDefectSeverity::Warning => FindingSeverity::Warning,
        },
        file_path: None,
        line: None,
        message: finding.message.clone(),
        required_action: None,
        source_stage: match source {
            PlanDefectSource::Coder => CodingExecutionStage::Coding,
            PlanDefectSource::CodeReviewer => CodingExecutionStage::CodeReview,
            PlanDefectSource::GroupReviewer => CodingExecutionStage::InternalPrReview,
        },
        evidence: finding
            .evidence
            .iter()
            .map(|evidence| evidence.source_ref.clone())
            .collect(),
        plan_defect_evidence: finding.evidence.clone(),
        related_requirements: Vec::new(),
        related_design_constraints: Vec::new(),
        related_work_item_tasks: Vec::new(),
        defect_class: finding.defect_class.clone(),
        reason_code: Some(finding.reason_code.clone()),
        contract_refs: finding.contract_refs.clone(),
        capability_refs: finding.capability_refs.clone(),
        repair_target: finding.repair_target.clone(),
        recommended_route: finding.recommended_route.clone(),
        confidence: Some(finding.confidence.clone()),
    }
}
