use super::*;
use crate::product::coding_models::{CodingUnitRun, CodingUnitRunStatus};
use crate::product::models::{
    PlanDefectClass, PlanDefectRoute, RepairTargetKind, WorkItemProjectionBundle,
};
use crate::product::plan_repair::{
    PlanDefectConfidence, PlanDefectFinding, PlanRepairError, normalize_blocker_route,
};
use crate::product::work_item_projection::{
    CoderExecutionEnvelope, CompiledWorkItemProjections, RenderedExecutionContext,
    ReviewerExecutionEnvelope, ReviewerWorkItemProjection, projection_hashes, renderer_for,
};
use crate::product::work_item_revision_store::WorkItemRevisionStore;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PlanDefectSource {
    Coder,
    Tester,
    CodeReviewer,
    GroupReviewer,
}

impl PlanDefectSource {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::Coder => "coder",
            Self::Tester => "tester",
            Self::CodeReviewer => "code_reviewer",
            Self::GroupReviewer => "group_reviewer",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExecutionPlanDefectReport {
    pub(crate) source: PlanDefectSource,
    pub(crate) findings: Vec<PlanDefectFinding>,
}

#[derive(Debug, Deserialize)]
struct ExecutionPlanDefectPayload {
    #[serde(default)]
    plan_defect_findings: Vec<serde_json::Value>,
}

pub(crate) fn parse_execution_plan_defects(
    source: PlanDefectSource,
    provider_output: &str,
) -> Result<ExecutionPlanDefectReport, CodingWorkspaceEngineError> {
    let Some(json) = extract_json_object(provider_output) else {
        return Ok(ExecutionPlanDefectReport {
            source,
            findings: Vec::new(),
        });
    };
    let payload = serde_json::from_str::<ExecutionPlanDefectPayload>(json).map_err(|error| {
        CodingWorkspaceEngineError::ProviderStream(format!("plan_defect_output_invalid: {error}"))
    })?;
    let findings = payload
        .plan_defect_findings
        .into_iter()
        .enumerate()
        .map(|(index, mut value)| {
            let object = value.as_object_mut().ok_or_else(|| {
                CodingWorkspaceEngineError::ProviderStream(
                    "plan_defect_finding_invalid: expected object".to_string(),
                )
            })?;
            object.entry("finding_id".to_string()).or_insert_with(|| {
                serde_json::Value::String(format!(
                    "{}_plan_defect_{:04}",
                    source.label(),
                    index + 1
                ))
            });
            let finding = serde_json::from_value::<PlanDefectFinding>(value).map_err(|error| {
                CodingWorkspaceEngineError::ProviderStream(format!(
                    "plan_defect_finding_invalid: {error}"
                ))
            })?;
            finding.validate().map_err(|error| {
                CodingWorkspaceEngineError::ProviderStream(format!(
                    "plan_defect_finding_invalid: {error:?}"
                ))
            })?;
            Ok(finding)
        })
        .collect::<Result<Vec<_>, CodingWorkspaceEngineError>>()?;
    Ok(ExecutionPlanDefectReport { source, findings })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodeReviewFlowDecision {
    RunCoderFix,
    RetryVerification,
    StartPlanRepair,
    StartStoryAmendment,
    StartDesignAmendment,
    OpenOperationalGate,
    StopForHumanTriage,
    ContinueAfterApprove,
}

pub(crate) fn code_review_flow_decision(
    report: &CodeReviewReport,
    reviewer_projection: &ReviewerWorkItemProjection,
) -> CodeReviewFlowDecision {
    review_findings_flow_decision(&report.findings, &report.verdict, reviewer_projection)
}

pub(crate) fn review_findings_flow_decision(
    findings: &[ReviewFinding],
    verdict: &ReviewVerdict,
    reviewer_projection: &ReviewerWorkItemProjection,
) -> CodeReviewFlowDecision {
    let valid = findings
        .iter()
        .filter(|finding| validate_plan_defect_finding(finding, reviewer_projection).is_ok())
        .collect::<Vec<_>>();
    for (class, decision) in [
        (
            PlanDefectClass::StoryAmendmentRequired,
            CodeReviewFlowDecision::StartStoryAmendment,
        ),
        (
            PlanDefectClass::DesignAmendmentRequired,
            CodeReviewFlowDecision::StartDesignAmendment,
        ),
    ] {
        if valid.iter().any(|finding| finding.defect_class == class) {
            return decision;
        }
    }
    if valid.iter().any(|finding| {
        matches!(
            finding.defect_class,
            PlanDefectClass::CurrentWorkItemInvalid
                | PlanDefectClass::UpstreamContractInvalid
                | PlanDefectClass::DependencyGraphInvalid
        )
    }) {
        return CodeReviewFlowDecision::StartPlanRepair;
    }
    if valid
        .iter()
        .any(|finding| finding.defect_class == PlanDefectClass::OperationalBlocker)
    {
        return CodeReviewFlowDecision::OpenOperationalGate;
    }
    if valid
        .iter()
        .any(|finding| finding.defect_class == PlanDefectClass::VerificationIncomplete)
    {
        return CodeReviewFlowDecision::RetryVerification;
    }
    if findings
        .iter()
        .any(|finding| validate_plan_defect_finding(finding, reviewer_projection).is_err())
    {
        return CodeReviewFlowDecision::StopForHumanTriage;
    }
    match verdict {
        ReviewVerdict::RequestChanges => CodeReviewFlowDecision::RunCoderFix,
        ReviewVerdict::Blocked if review_findings_have_actionable_findings(findings) => {
            CodeReviewFlowDecision::RunCoderFix
        }
        ReviewVerdict::Blocked => CodeReviewFlowDecision::StopForHumanTriage,
        ReviewVerdict::Approve => CodeReviewFlowDecision::ContinueAfterApprove,
    }
}

pub(crate) fn validate_plan_defect_finding(
    finding: &ReviewFinding,
    reviewer_projection: &ReviewerWorkItemProjection,
) -> Result<(), PlanRepairError> {
    if finding.defect_class == PlanDefectClass::ImplementationDefect {
        if finding.recommended_route == PlanDefectRoute::CoderRework
            && finding.reason_code.is_none()
            && finding.contract_refs.is_empty()
            && finding.capability_refs.is_empty()
            && finding.repair_target.is_none()
            && finding.confidence.is_none()
            && finding.plan_defect_evidence.is_empty()
        {
            return Ok(());
        }
        return Err(invalid_finding(
            "implementation finding contains plan defect fields",
        ));
    }
    let reason_code = finding
        .reason_code
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| invalid_finding("plan defect reason code is missing"))?;
    if !matches!(
        finding.confidence,
        Some(PlanDefectConfidence::Medium | PlanDefectConfidence::High)
    ) || finding
        .contract_refs
        .iter()
        .chain(finding.capability_refs.iter())
        .chain(finding.evidence.iter())
        .any(|value| value.trim().is_empty())
        || finding.plan_defect_evidence.iter().any(|evidence| {
            evidence.kind.trim().is_empty()
                || evidence.source_ref.trim().is_empty()
                || evidence.message.trim().is_empty()
        })
    {
        return Err(invalid_finding(
            "plan defect confidence or references are invalid",
        ));
    }
    let canonical = PlanDefectFinding {
        finding_id: "review_finding_validation".to_string(),
        severity: match finding.severity {
            FindingSeverity::Error => crate::product::plan_repair::PlanDefectSeverity::Error,
            FindingSeverity::Warning => crate::product::plan_repair::PlanDefectSeverity::Warning,
            FindingSeverity::Info => {
                return Err(invalid_finding("plan defect severity cannot be info"));
            }
        },
        defect_class: finding.defect_class.clone(),
        reason_code: reason_code.to_string(),
        message: finding.message.clone(),
        evidence: finding.plan_defect_evidence.clone(),
        contract_refs: finding.contract_refs.clone(),
        capability_refs: finding.capability_refs.clone(),
        repair_target: finding.repair_target.clone(),
        recommended_route: finding.recommended_route.clone(),
        confidence: finding.confidence.clone().expect("validated confidence"),
    };
    canonical.validate()?;

    let rule = reviewer_projection
        .blocker_routing
        .iter()
        .find(|rule| rule.reason_code == reason_code)
        .ok_or_else(|| invalid_finding("reviewer projection blocker rule is missing"))?;
    let normalized = normalize_blocker_route(rule.route.clone());
    if normalized.route != finding.recommended_route
        || normalized.required_target_kind
            != finding
                .repair_target
                .as_ref()
                .map(|target| target.kind.clone())
        || !finding
            .contract_refs
            .iter()
            .all(|reference| rule.target_contract_refs.contains(reference))
    {
        return Err(invalid_finding(
            "reviewer projection blocker route does not match",
        ));
    }
    if finding.defect_class == PlanDefectClass::UpstreamContractInvalid {
        let target = finding
            .repair_target
            .as_ref()
            .filter(|target| target.kind == RepairTargetKind::UpstreamWorkItem)
            .ok_or_else(|| invalid_finding("upstream plan defect target is invalid"))?;
        let contract_matches = reviewer_projection
            .input_contract_checks
            .iter()
            .any(|contract| {
                finding.contract_refs.contains(&contract.contract_id)
                    && target
                        .logical_work_item_ids
                        .contains(&contract.provider_logical_work_item_id)
                    && finding
                        .capability_refs
                        .iter()
                        .all(|capability| contract.required_capabilities.contains(capability))
            });
        if !contract_matches {
            return Err(invalid_finding(
                "upstream contract or capability is not projected",
            ));
        }
    }
    Ok(())
}

fn invalid_finding(message: &str) -> PlanRepairError {
    PlanRepairError::InvalidFinding(message.to_string())
}

impl CodingWorkspaceEngine {
    pub(crate) fn reviewer_projection_for_attempt(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<ReviewerWorkItemProjection, CodingWorkspaceEngineError> {
        if attempt.scope != crate::product::coding_models::CodingAttemptScope::WorkItemGroup {
            return Ok(empty_reviewer_projection());
        }
        let run = self.store.get_active_unit_run(attempt)?;
        let plan_id = attempt.work_item_group_id.as_deref().ok_or_else(|| {
            CodingWorkspaceEngineError::ProviderStream(
                "unit_run_projection_binding_missing: plan id".to_string(),
            )
        })?;
        let revision_store = WorkItemRevisionStore::new(self.store.paths());
        let lineage =
            revision_store.get_plan_lineage(&attempt.project_id, &attempt.issue_id, plan_id)?;
        let bundle =
            revision_store.get_work_item_projection_bundle(&lineage, &run.projection_bundle_id)?;
        if bundle.work_item_revision_id != run.work_item_revision_id
            || bundle.canonical_contract_hash != run.canonical_contract_hash
            || bundle.compiler_version != run.projection_compiler_version
            || bundle.coder_projection_hash != run.coder_projection_hash
            || bundle.reviewer_projection_hash != run.reviewer_projection_hash
            || bundle.reviewer_projection.work_item_revision_id != run.work_item_revision_id
        {
            return Err(CodingWorkspaceEngineError::ProviderStream(format!(
                "unit_run_projection_binding_mismatch: {}",
                run.id
            )));
        }
        Ok(bundle.reviewer_projection)
    }

    pub(crate) fn render_coder_unit_run_context(
        &self,
        attempt: &CodingExecutionAttempt,
        provider: &ProviderName,
        previous_actionable_review: Option<String>,
    ) -> Result<Option<RenderedExecutionContext>, CodingWorkspaceEngineError> {
        let Some((run, bundle)) = self.active_unit_run_projection(attempt)? else {
            return Ok(None);
        };
        let repository_state_ref = attempt
            .head_commit
            .clone()
            .unwrap_or_else(|| attempt.base_branch.clone());
        let rendered = renderer_for(provider)
            .render_coder(
                &bundle.coder_projection,
                &CoderExecutionEnvelope {
                    repository_state_ref,
                    resolved_handoff_revision_ids: run.resolved_handoff_revision_ids.clone(),
                    unit_run_id: run.id.clone(),
                    previous_actionable_review,
                    start_commit: run.start_commit.clone(),
                },
            )
            .map_err(|error| {
                CodingWorkspaceEngineError::ProviderStream(format!(
                    "coder_projection_render_failed: {error}"
                ))
            })?;
        self.store.bind_unit_run_execution_context(
            attempt,
            &run.id,
            CodingProviderRole::Coder,
            &rendered,
        )?;
        Ok(Some(rendered))
    }

    pub(crate) fn render_reviewer_unit_run_context(
        &self,
        attempt: &CodingExecutionAttempt,
        provider: &ProviderName,
    ) -> Result<Option<RenderedExecutionContext>, CodingWorkspaceEngineError> {
        let Some((run, bundle)) = self.active_unit_run_projection(attempt)? else {
            return Ok(None);
        };
        let repository_state_ref = attempt
            .head_commit
            .clone()
            .unwrap_or_else(|| attempt.base_branch.clone());
        let test_evidence_refs = self
            .store
            .list_testing_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .into_iter()
            .map(|report| report.id)
            .collect();
        let rendered = renderer_for(provider)
            .render_reviewer(
                &bundle.reviewer_projection,
                &ReviewerExecutionEnvelope {
                    unit_run_id: run.id.clone(),
                    diff_ref: format!("{repository_state_ref}..worktree"),
                    test_evidence_refs,
                    handoff_revision_ids: run.resolved_handoff_revision_ids.clone(),
                    contract_delta_refs: Vec::new(),
                    completion_commit: repository_state_ref,
                },
            )
            .map_err(|error| {
                CodingWorkspaceEngineError::ProviderStream(format!(
                    "reviewer_projection_render_failed: {error}"
                ))
            })?;
        self.store.bind_unit_run_execution_context(
            attempt,
            &run.id,
            CodingProviderRole::CodeReviewer,
            &rendered,
        )?;
        Ok(Some(rendered))
    }

    fn active_unit_run_projection(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<Option<(CodingUnitRun, WorkItemProjectionBundle)>, CodingWorkspaceEngineError> {
        if attempt.scope != crate::product::coding_models::CodingAttemptScope::WorkItemGroup {
            return Ok(None);
        }
        let plan_id = attempt.work_item_group_id.as_deref().ok_or_else(|| {
            CodingWorkspaceEngineError::ProviderStream(
                "unit_run_projection_binding_missing: plan id".to_string(),
            )
        })?;
        let active_unit = self
            .store
            .get_active_coding_unit(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .ok_or_else(|| {
                CodingWorkspaceEngineError::ProviderStream(
                    "unit_run_projection_binding_missing: active unit".to_string(),
                )
            })?;
        let revision_store = WorkItemRevisionStore::new(self.store.paths());
        let lineage =
            revision_store.get_plan_lineage(&attempt.project_id, &attempt.issue_id, plan_id)?;
        let revision = revision_store.get_work_item_revision(
            &lineage,
            &active_unit.logical_work_item_id,
            &active_unit.work_item_revision_id,
        )?;
        let bundle = revision_store
            .get_work_item_projection_bundle(&lineage, &revision.work_item_projection_bundle_id)?;
        validate_projection_bundle(&active_unit.work_item_revision_id, &revision, &bundle)?;
        let resolved_handoff_revision_ids = active_unit
            .dependency_logical_work_item_ids
            .iter()
            .map(|dependency_id| {
                let dependency = self
                    .store
                    .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)?
                    .into_iter()
                    .find(|unit| unit.logical_work_item_id == *dependency_id)
                    .ok_or_else(|| {
                        CodingWorkspaceEngineError::WorkItemHandoffMissing(dependency_id.clone())
                    })?;
                let handoff_id = dependency.latest_handoff_revision_id.ok_or_else(|| {
                    CodingWorkspaceEngineError::WorkItemHandoffMissing(dependency_id.clone())
                })?;
                let handoff =
                    revision_store.get_handoff_revision(&lineage, dependency_id, &handoff_id)?;
                let source_run = self
                    .store
                    .list_coding_unit_runs(attempt, &dependency.id)?
                    .into_iter()
                    .find(|run| run.id == handoff.coding_unit_run_id);
                if handoff.work_item_revision_id != dependency.work_item_revision_id
                    || !source_run.is_some_and(|run| {
                        run.work_item_revision_id == dependency.work_item_revision_id
                            && run.status == CodingUnitRunStatus::Completed
                    })
                {
                    return Err(CodingWorkspaceEngineError::ProviderStream(format!(
                        "unit_run_handoff_binding_mismatch: {handoff_id}"
                    )));
                }
                Ok(handoff_id)
            })
            .collect::<Result<Vec<_>, CodingWorkspaceEngineError>>()?;
        let providers = self.store.get_role_provider_config_snapshot(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        let coder_renderer_version = renderer_for(&providers.coder)
            .renderer_version()
            .to_string();
        let reviewer_renderer_version = renderer_for(&providers.code_reviewer)
            .renderer_version()
            .to_string();
        let run = match self.store.get_active_unit_run(attempt) {
            Ok(run) => run,
            Err(ProductStoreError::NotFound {
                kind: "coding_unit_run",
                ..
            }) => {
                let units = self.store.list_coding_units(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                )?;
                let existing_count = units.iter().try_fold(0usize, |count, unit| {
                    Ok::<_, ProductStoreError>(
                        count + self.store.list_coding_unit_runs(attempt, &unit.id)?.len(),
                    )
                })?;
                let now = Utc::now().to_rfc3339();
                let run = CodingUnitRun {
                    id: next_sequential_id("coding_unit_run", existing_count),
                    unit_id: active_unit.id.clone(),
                    execution_no: self
                        .store
                        .list_coding_unit_runs(attempt, &active_unit.id)?
                        .len() as u32
                        + 1,
                    work_item_revision_id: revision.id.clone(),
                    resolved_handoff_revision_ids: resolved_handoff_revision_ids.clone(),
                    canonical_contract_hash: revision.canonical_contract_hash.clone(),
                    projection_bundle_id: bundle.id.clone(),
                    projection_compiler_version: bundle.compiler_version.clone(),
                    coder_provider_renderer_version: coder_renderer_version.clone(),
                    reviewer_provider_renderer_version: reviewer_renderer_version.clone(),
                    coder_projection_hash: bundle.coder_projection_hash.clone(),
                    reviewer_projection_hash: bundle.reviewer_projection_hash.clone(),
                    coder_execution_context_hash: None,
                    reviewer_execution_context_hash: None,
                    status: CodingUnitRunStatus::Running,
                    unit_rework_count: 0,
                    verification_retry_count: 0,
                    operational_retry_count: 0,
                    plan_repair_count: 0,
                    start_commit: attempt.head_commit.clone(),
                    completion_commit: None,
                    created_at: now.clone(),
                    updated_at: now,
                };
                self.store.load_or_create_coding_unit_run(attempt, &run)?
            }
            Err(error) => return Err(error.into()),
        };
        if run.unit_id != active_unit.id
            || run.work_item_revision_id != revision.id
            || run.resolved_handoff_revision_ids != resolved_handoff_revision_ids
            || run.canonical_contract_hash != revision.canonical_contract_hash
            || run.projection_bundle_id != bundle.id
            || run.projection_compiler_version != bundle.compiler_version
            || run.coder_projection_hash != bundle.coder_projection_hash
            || run.reviewer_projection_hash != bundle.reviewer_projection_hash
            || run.coder_provider_renderer_version != coder_renderer_version
            || run.reviewer_provider_renderer_version != reviewer_renderer_version
        {
            return Err(CodingWorkspaceEngineError::ProviderStream(format!(
                "unit_run_projection_binding_mismatch: {}",
                run.id
            )));
        }
        Ok(Some((run, bundle)))
    }
}

pub(crate) fn validate_projection_bundle(
    expected_revision_id: &str,
    revision: &crate::product::models::WorkItemRevision,
    bundle: &WorkItemProjectionBundle,
) -> Result<(), CodingWorkspaceEngineError> {
    let hashes = projection_hashes(&CompiledWorkItemProjections {
        human: bundle.human_projection.clone(),
        coder: bundle.coder_projection.clone(),
        reviewer: bundle.reviewer_projection.clone(),
    })
    .map_err(|error| {
        CodingWorkspaceEngineError::ProviderStream(format!(
            "unit_run_projection_hash_failed: {error}"
        ))
    })?;
    if revision.id != expected_revision_id
        || revision.work_item_projection_bundle_id != bundle.id
        || bundle.work_item_revision_id != revision.id
        || bundle.canonical_contract_hash != revision.canonical_contract_hash
        || bundle.compiler_version.trim().is_empty()
        || bundle.coder_projection.work_item_revision_id != revision.id
        || bundle.reviewer_projection.work_item_revision_id != revision.id
        || bundle.human_projection_hash != hashes.human
        || bundle.coder_projection_hash != hashes.coder
        || bundle.reviewer_projection_hash != hashes.reviewer
    {
        return Err(CodingWorkspaceEngineError::ProviderStream(format!(
            "unit_run_projection_binding_mismatch: {}",
            bundle.id
        )));
    }
    Ok(())
}

pub(crate) fn empty_reviewer_projection() -> ReviewerWorkItemProjection {
    ReviewerWorkItemProjection {
        work_item_revision_id: String::new(),
        criterion_refs: Vec::new(),
        requirement_matrix: Vec::new(),
        scope_policy: crate::product::work_item_contract::WorkItemWritePolicy {
            exclusive_scopes: Vec::new(),
            forbidden_scopes: Vec::new(),
        },
        input_contract_checks: Vec::new(),
        output_contract_checks: Vec::new(),
        verification_evidence_rules: Vec::new(),
        blocker_routing: Vec::new(),
    }
}
