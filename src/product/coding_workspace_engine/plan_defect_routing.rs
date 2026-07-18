use super::*;
use crate::product::plan_repair::{PlanDefectFinding, PlanDefectSeverity};
use crate::product::work_item_projection::ReviewerWorkItemProjection;
use crate::product::work_item_revision_store::WorkItemRevisionStore;

pub(crate) fn internal_review_flow_decision(
    review: &InternalPrReview,
    reviewer_projection: &ReviewerWorkItemProjection,
) -> CodeReviewFlowDecision {
    let _source = PlanDefectSource::GroupReviewer;
    review_findings_flow_decision(&review.findings, &review.verdict, reviewer_projection)
}

impl CodingWorkspaceEngine {
    pub(crate) fn reviewer_projection_for_internal_review(
        &self,
        attempt: &CodingExecutionAttempt,
        review: &InternalPrReview,
    ) -> Result<ReviewerWorkItemProjection, CodingWorkspaceEngineError> {
        if attempt.scope != crate::product::coding_models::CodingAttemptScope::WorkItemGroup {
            return self.reviewer_projection_for_attempt(attempt);
        }
        let Some(logical_id) = review.findings.iter().find_map(|finding| {
            (finding.defect_class != crate::product::models::PlanDefectClass::ImplementationDefect)
                .then_some(finding.repair_target.as_ref())
                .flatten()
                .and_then(|target| target.logical_work_item_ids.first())
        }) else {
            return Ok(empty_reviewer_projection());
        };
        let run = self
            .store
            .list_unit_runs_by_logical_id(attempt, logical_id)?
            .into_iter()
            .filter(|run| {
                run.status == crate::product::coding_models::CodingUnitRunStatus::Completed
            })
            .max_by_key(|run| run.execution_no)
            .ok_or_else(|| {
                CodingWorkspaceEngineError::ProviderStream(format!(
                    "group_review_unit_run_missing: {logical_id}"
                ))
            })?;
        let plan_id = attempt.work_item_group_id.as_deref().ok_or_else(|| {
            CodingWorkspaceEngineError::ProviderStream(
                "group_review_plan_binding_missing".to_string(),
            )
        })?;
        let revision_store = WorkItemRevisionStore::new(self.store.paths());
        let lineage =
            revision_store.get_plan_lineage(&attempt.project_id, &attempt.issue_id, plan_id)?;
        let revision = revision_store.get_work_item_revision(
            &lineage,
            logical_id,
            &run.work_item_revision_id,
        )?;
        let bundle =
            revision_store.get_work_item_projection_bundle(&lineage, &run.projection_bundle_id)?;
        validate_projection_bundle(&run.work_item_revision_id, &revision, &bundle)?;
        if run.canonical_contract_hash != bundle.canonical_contract_hash
            || run.projection_compiler_version != bundle.compiler_version
            || run.reviewer_projection_hash != bundle.reviewer_projection_hash
        {
            return Err(CodingWorkspaceEngineError::ProviderStream(format!(
                "group_review_projection_binding_mismatch: {}",
                run.id
            )));
        }
        Ok(bundle.reviewer_projection)
    }
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

fn execution_finding_adapter(
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
            PlanDefectSource::Tester => CodingExecutionStage::Testing,
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
