use super::*;
use crate::product::coding_models::{
    CodingExecutionUnit, CodingExecutionUnitStatus, CodingUnitRun, CodingUnitRunStatus,
};
use crate::product::models::{
    PlanDefectClass, PlanDefectRoute, RepairTargetKind, WorkItemPlanLineage,
    WorkItemProjectionBundle,
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

/// 从 Provider 输出中挑出真正携带 `plan_defect_findings` 的结论对象。
///
/// Provider 正文常夹带与结论无关的花括号片段（JS 解构 `{ foo }`、示例
/// `{"type": "module"}` 等），只信任第一个 `{` 会把散文误判为契约违规并把流程
/// 卡在人工分诊。因此逐个遍历平衡候选，并且只接受显式声明
/// `plan_defect_findings` 的对象：该字段带 `#[serde(default)]`，任何合法 JSON
/// 对象都能反序列化成空 findings，仅凭反序列化成功会让无关片段静默吞掉真实结论。
///
/// 契约要求结论位于输出末尾（同 `review_parser`），因此取**最后**一个可用的
/// 声明候选：Provider 可能先复述空示例 `{"plan_defect_findings": []}` 再给出
/// 真实结论，取首个会让真实结论被静默吞掉。
///
/// 边界：`extract_json_object_candidates` 只产出顶层平衡对象，因此被包在更外层
/// 花括号块中的结论不会被检查——契约要求结论为顶层对象，这里不做兼容。
fn extract_plan_defect_payload(
    provider_output: &str,
) -> Result<Option<ExecutionPlanDefectPayload>, CodingWorkspaceEngineError> {
    let mut selected = None;
    let mut declared_error = None;
    for json in extract_json_object_candidates(provider_output) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
            continue;
        };
        let declares_findings = value
            .as_object()
            .is_some_and(|object| object.contains_key("plan_defect_findings"));
        if !declares_findings {
            continue;
        }
        match serde_json::from_value::<ExecutionPlanDefectPayload>(value) {
            Ok(payload) => selected = Some(payload),
            // 声明了 `plan_defect_findings` 但结构非法：这是真实的契约违规，
            // 记录下来，待所有候选都不可用时报错。
            Err(error) => declared_error = Some(error),
        }
    }
    match (selected, declared_error) {
        (Some(payload), _) => Ok(Some(payload)),
        (None, Some(error)) => Err(CodingWorkspaceEngineError::ProviderStream(format!(
            "plan_defect_output_invalid: {error}"
        ))),
        (None, None) => Ok(None),
    }
}

pub(crate) fn parse_execution_plan_defects(
    source: PlanDefectSource,
    provider_output: &str,
) -> Result<ExecutionPlanDefectReport, CodingWorkspaceEngineError> {
    let Some(payload) = extract_plan_defect_payload(provider_output)? else {
        return Ok(ExecutionPlanDefectReport {
            source,
            findings: Vec::new(),
        });
    };
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
    review_findings_flow_decision_with_validator(findings, verdict, |finding| {
        validate_plan_defect_finding(finding, reviewer_projection)
    })
}

pub(crate) fn review_findings_flow_decision_with_validator<F>(
    findings: &[ReviewFinding],
    verdict: &ReviewVerdict,
    mut validate: F,
) -> CodeReviewFlowDecision
where
    F: FnMut(&ReviewFinding) -> Result<(), PlanRepairError>,
{
    if findings.iter().any(|finding| validate(finding).is_err()) {
        return CodeReviewFlowDecision::StopForHumanTriage;
    }
    let valid = findings.iter().collect::<Vec<_>>();
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

    pub(crate) fn authoritative_resolved_handoff_revision_ids(
        &self,
        attempt: &CodingExecutionAttempt,
        unit: &CodingExecutionUnit,
        lineage: &WorkItemPlanLineage,
    ) -> Result<Vec<String>, CodingWorkspaceEngineError> {
        let units =
            self.store
                .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let revision_store = WorkItemRevisionStore::new(self.store.paths());
        unit.dependency_logical_work_item_ids
            .iter()
            .map(|dependency_id| {
                let mut matches = units
                    .iter()
                    .filter(|candidate| candidate.logical_work_item_id == *dependency_id);
                let dependency = matches.next().ok_or_else(|| {
                    CodingWorkspaceEngineError::WorkItemHandoffMissing(dependency_id.clone())
                })?;
                if matches.next().is_some() {
                    return Err(ProductStoreError::Ambiguous {
                        kind: "coding_execution_unit",
                        id: dependency_id.clone(),
                    }
                    .into());
                }
                let handoff_id = dependency
                    .latest_handoff_revision_id
                    .as_deref()
                    .ok_or_else(|| {
                        CodingWorkspaceEngineError::WorkItemHandoffMissing(dependency_id.clone())
                    })?;
                let handoff =
                    revision_store.get_handoff_revision(lineage, dependency_id, handoff_id)?;
                let source_run = self
                    .store
                    .list_coding_unit_runs(attempt, &dependency.id)?
                    .into_iter()
                    .max_by_key(|run| run.execution_no);
                if dependency.status != CodingExecutionUnitStatus::Completed
                    || dependency.completion_commit.as_deref() != Some(handoff.commit_sha.as_str())
                    || handoff.id != format!("handoff_revision_{}", handoff.coding_unit_run_id)
                    || handoff.logical_work_item_id != dependency.logical_work_item_id
                    || handoff.work_item_revision_id != dependency.work_item_revision_id
                    || !source_run.is_some_and(|run| {
                        run.id == handoff.coding_unit_run_id
                            && run.unit_id == dependency.id
                            && run.work_item_revision_id == dependency.work_item_revision_id
                            && run.status == CodingUnitRunStatus::Completed
                            && run.completion_commit.as_deref() == Some(handoff.commit_sha.as_str())
                    })
                {
                    return Err(CodingWorkspaceEngineError::ProviderStream(format!(
                        "unit_run_handoff_binding_mismatch: {handoff_id}"
                    )));
                }
                Ok(handoff_id.to_string())
            })
            .collect()
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
        let resolved_handoff_revision_ids =
            self.authoritative_resolved_handoff_revision_ids(attempt, &active_unit, &lineage)?;
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
                    internal_reviewer_provider_renderer_version: None,
                    coder_projection_hash: bundle.coder_projection_hash.clone(),
                    reviewer_projection_hash: bundle.reviewer_projection_hash.clone(),
                    coder_execution_context_hash: None,
                    reviewer_execution_context_hash: None,
                    internal_reviewer_execution_context_hash: None,
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
