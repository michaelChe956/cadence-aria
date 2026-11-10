use std::collections::{BTreeMap, BTreeSet};

use crate::product::models::{
    DependencyGraphRevision, PlanProjectionBundle, PlanRepairImpactScopeReview,
    PlanRepairRequestStatus, PlanRepairSessionSnapshotDto, PlanRepairSessionStage,
    PlanRevisionReason, RepairTargetKind, WorkItemProjectionBundle, WorkItemRevision,
};
use crate::product::plan_repair::{
    candidate_request_matches_review_status, load_plan_repair_candidate_package,
};
use crate::product::work_item_contract::{
    CanonicalWorkItemContract, ContractReviewerCoverageProjection, build_dependency_contract_graph,
    canonical_contract_hash, project_contract_reviewer_coverage,
};
use crate::product::work_item_projection::{
    CompiledPlanProjections, CompiledWorkItemProjections, ProjectionValidationReport,
    plan_projection_hashes, projection_hashes,
};
use crate::product::work_item_revision_store::WorkItemRevisionStore;
use crate::product::workspace_engine::WorkspaceEngine;
use serde_json::json;

pub(super) struct PlanReviewContext {
    pub(super) story_design_traceability: Vec<String>,
    pub(super) canonical_contract_candidates: Vec<CanonicalWorkItemContract>,
    pub(super) dependency_contract_graph: DependencyGraphRevision,
    pub(super) plan_projection_bundle_candidate: PlanProjectionBundle,
    pub(super) work_item_projection_bundle_candidates: Vec<WorkItemProjectionBundle>,
    pub(super) projection_validation_report: ProjectionValidationReport,
    pub(super) contract_delta: Vec<String>,
    pub(super) impact_analysis: Vec<String>,
    pub(super) repair_evidence: Vec<String>,
    pub(super) impact_scope_review: Option<PlanRepairImpactScopeReview>,
    pub(super) candidate_package_fingerprint: Option<String>,
}

pub(super) fn append_review_context_section(
    prompt: &mut String,
    title: &str,
    value: &impl serde::Serialize,
) -> Result<(), String> {
    prompt.push_str(&format!("\n### {title}\n"));
    prompt.push_str(
        &serde_json::to_string_pretty(value)
            .map_err(|error| format!("serialize Plan Review Context `{title}` failed: {error}"))?,
    );
    prompt.push('\n');
    Ok(())
}

pub(super) fn single_candidate_reviewer_coverage(
    items: &[crate::product::work_item_plan_compiler::PlanCandidateItemIr],
) -> Result<ContractReviewerCoverageProjection, String> {
    let contracts = items
        .iter()
        .map(|item| item.contract.clone())
        .collect::<Vec<_>>();
    let graph = build_dependency_contract_graph(&contracts).map_err(|report| {
        format!("single-candidate review dependency contract graph failed: {report:?}")
    })?;
    Ok(project_contract_reviewer_coverage(&graph))
}

/// F5-C：SC reviewer 防打转教学段。修订循环不收敛的第一根因已由 findings 回灌
/// （F5-A）修复，本段数导 reviewer 不对同一批未解决 findings 无限 revise：
/// 与上一轮实质相同且 author 已声明处理时改判 needs_human 并说明卡点。
pub(super) const SINGLE_CANDIDATE_REVIEW_REPETITION_TEACHING: &str = "\
[review_repetition_discipline]\
若本轮 findings 与你上一轮实质相同（同一批未解决问题，且 author 已在修订中声明处理或本轮输出仍未解决），不得再次返回 revise：改判 needs_human，并在 summary 中说明卡点（哪条 finding 为何无法通过自动修订收敛）。不要对同一批未解决 findings 无限第 N 次 revise。\
\n";

pub(super) fn append_single_candidate_contract_gap_teaching(
    prompt: &mut String,
    coverage: &ContractReviewerCoverageProjection,
) {
    for handoff in coverage
        .handoff_consumption
        .iter()
        .filter(|handoff| !handoff.consumed)
    {
        prompt.push_str(&format!(
            "- handoff gap evidence required: severity=must_fix category=contract_gap class_hint=repairable evidence: provider `{}`, contract_ref `{}`, consumers: [], consumed: false\n",
            handoff.provider, handoff.contract_ref,
        ));
    }
}

pub(super) fn single_candidate_dependency_graph(
    items: &[crate::product::work_item_plan_compiler::PlanCandidateItemIr],
) -> Result<serde_json::Value, String> {
    let mut item_ids = BTreeSet::new();
    for item in items {
        let item_id = item.contract.identity.logical_work_item_id.as_str();
        if item_id.trim().is_empty() {
            return Err(
                "single-candidate review IR contains an empty work item identity".to_string(),
            );
        }
        if !item_ids.insert(item_id.to_string()) {
            return Err(format!(
                "single-candidate review IR contains duplicate work item identity `{item_id}`"
            ));
        }
    }
    let mut remaining_dependencies = BTreeMap::new();
    let mut dependents = BTreeMap::<String, BTreeSet<String>>::new();
    let mut edges = Vec::new();
    for item in items {
        let item_id = item.contract.identity.logical_work_item_id.clone();
        let mut dependencies = BTreeSet::new();
        for dependency in &item.contract.depends_on {
            if !item_ids.contains(dependency) {
                return Err(format!(
                    "single-candidate review IR dependency `{dependency}` for `{item_id}` is missing"
                ));
            }
            if dependency == &item_id {
                return Err(format!(
                    "single-candidate review IR work item `{item_id}` depends on itself"
                ));
            }
            if dependencies.insert(dependency.clone()) {
                dependents
                    .entry(dependency.clone())
                    .or_default()
                    .insert(item_id.clone());
                edges.push(format!("{dependency} -> {item_id}"));
            }
        }
        remaining_dependencies.insert(item_id, dependencies);
    }
    edges.sort();
    let mut ready = remaining_dependencies
        .iter()
        .filter_map(|(item_id, dependencies)| dependencies.is_empty().then_some(item_id.clone()))
        .collect::<BTreeSet<_>>();
    let mut topological_order = Vec::with_capacity(items.len());
    while let Some(item_id) = ready.iter().next().cloned() {
        ready.remove(&item_id);
        topological_order.push(item_id.clone());
        for dependent in dependents.get(&item_id).into_iter().flatten() {
            let dependencies = remaining_dependencies
                .get_mut(dependent)
                .expect("dependent must be present in remaining dependency map");
            dependencies.remove(&item_id);
            if dependencies.is_empty() {
                ready.insert(dependent.clone());
            }
        }
    }
    if topological_order.len() != items.len() {
        return Err("single-candidate review IR dependency graph contains a cycle".to_string());
    }
    Ok(json!({
        "topological_order": topological_order,
        "edges": edges,
    }))
}

#[derive(Clone, Copy)]
pub(super) enum PlanReviewSource<'a> {
    InitialActive,
    PlanRepairCandidate(&'a PlanRepairSessionSnapshotDto),
}

impl<'a> PlanReviewSource<'a> {
    pub(super) fn for_engine(engine: &'a WorkspaceEngine) -> Self {
        match engine.plan_repair_snapshot.as_ref() {
            Some(snapshot) if snapshot.stage == PlanRepairSessionStage::PlanReview => {
                Self::PlanRepairCandidate(snapshot)
            }
            _ => Self::InitialActive,
        }
    }
}

pub(super) fn load_plan_review_context(
    engine: &WorkspaceEngine,
    projection: &PlanProjectionBundle,
    source: PlanReviewSource<'_>,
) -> Result<PlanReviewContext, String> {
    let lifecycle = engine
        .lifecycle_store
        .as_ref()
        .ok_or_else(|| "lifecycle_store unavailable for Plan Review Context".to_string())?;
    let store = WorkItemRevisionStore::new(lifecycle.app_paths());
    let lineage = store
        .get_plan_lineage(
            &engine.session.project_id,
            &engine.session.issue_id,
            &engine.session.entity_id,
        )
        .map_err(|error| format!("load Plan Review lineage failed: {error}"))?;
    let revision = store
        .get_plan_revision(
            &engine.session.project_id,
            &engine.session.issue_id,
            &lineage.id,
            &projection.plan_revision_id,
        )
        .map_err(|error| format!("load Plan Review revision failed: {error}"))?;
    let (
        contract_delta,
        impact_analysis,
        repair_evidence,
        impact_scope_review,
        candidate_package_artifact,
    ) = match source {
        PlanReviewSource::InitialActive => {
            if lineage.active_revision_id.as_deref() != Some(projection.plan_revision_id.as_str()) {
                return Err("Plan Review active revision binding mismatch".to_string());
            }
            if revision.reason != PlanRevisionReason::InitialCompile {
                return Err(
                    "Plan Review Context only supports initial plan publication".to_string()
                );
            }
            (
                vec!["initial_plan_publication: no previous contract delta".to_string()],
                Vec::new(),
                vec!["initial_plan_publication: no repair evidence".to_string()],
                None,
                None,
            )
        }
        PlanReviewSource::PlanRepairCandidate(snapshot) => {
            let request = &snapshot.request;
            let authoritative_request =
                store
                    .get_repair_request(&lineage, &request.id)
                    .map_err(|error| {
                        format!("load authoritative Plan Repair request failed: {error}")
                    })?;
            let candidate_package_id = snapshot
                .candidate_package_artifact_id
                .as_deref()
                .ok_or_else(|| {
                    "Plan Repair candidate package artifact id is missing".to_string()
                })?;
            let candidate_package =
                load_plan_repair_candidate_package(&store, &lineage, candidate_package_id)
                    .map_err(|error| {
                        format!("load canonical Plan Repair candidate failed: {error:?}")
                    })?;
            let amendment = snapshot
                .amendment
                .as_ref()
                .ok_or_else(|| "Plan Repair review manifest is missing".to_string())?;
            let amendment_id = request
                .amendment_id
                .as_deref()
                .ok_or_else(|| "Plan Repair review amendment id is missing".to_string())?;
            let expected_reason = match request.repair_target.kind {
                RepairTargetKind::CurrentWorkItem => PlanRevisionReason::RepairCurrentWorkItem,
                RepairTargetKind::UpstreamWorkItem => PlanRevisionReason::RepairUpstreamContract,
                RepairTargetKind::Subgraph => PlanRevisionReason::SubgraphReplan,
            };
            let expected_request_status = if snapshot.impact_scope_review.is_some() {
                PlanRepairRequestStatus::AwaitingConfirmation
            } else {
                PlanRepairRequestStatus::InProgress
            };
            if authoritative_request != *request
                || !candidate_request_matches_review_status(
                    &candidate_package.request,
                    request,
                    expected_request_status,
                )
                || candidate_package.minimum_manifest != *amendment
                || candidate_package.plan_projection_bundle != *projection
                || snapshot.validation.as_ref() != Some(&candidate_package.validation_report)
                || snapshot.impact.as_ref() != Some(&candidate_package.impact_report)
                || request.plan_id != lineage.id
                || lineage.active_revision_id.as_deref()
                    != Some(request.base_plan_revision_id.as_str())
                || lineage.active_amendment_id.as_deref() != Some(amendment_id)
                || amendment.repair_request_id != request.id
                || amendment.previous_plan_revision_id != request.base_plan_revision_id
                || amendment.new_plan_revision_id != revision.id
                || revision.supersedes.as_deref() != Some(request.base_plan_revision_id.as_str())
                || revision.reason != expected_reason
                || snapshot.projection.as_ref() != Some(projection)
            {
                return Err("Plan Repair review candidate provenance mismatch".to_string());
            }
            let contract_delta = amendment
                .contract_deltas
                .iter()
                .map(|delta| {
                    serde_json::to_string(delta)
                        .map_err(|error| format!("serialize Contract Delta failed: {error}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let impact = snapshot
                .impact
                .as_ref()
                .ok_or_else(|| "Plan Repair review impact analysis is missing".to_string())?;
            if string_set(&impact.unaffected) != string_set(&amendment.unaffected_units)
                || string_set(&impact.direct_revalidation)
                    != string_set(&amendment.revalidation_required_units)
                || string_set(&impact.direct_stale) != string_set(&amendment.stale_units)
            {
                return Err("Plan Repair review impact provenance mismatch".to_string());
            }
            let impact_analysis = vec![
                serde_json::to_string(impact)
                    .map_err(|error| format!("serialize Impact Analysis failed: {error}"))?,
            ];
            let mut repair_evidence = vec![format!(
                "request_id={}, amendment_id={}, base_plan_revision_id={}",
                request.id, amendment.id, request.base_plan_revision_id
            )];
            repair_evidence.extend(request.evidence.iter().map(|evidence| {
                format!(
                    "{}:{}:{}",
                    evidence.kind, evidence.source_ref, evidence.message
                )
            }));
            (
                contract_delta,
                impact_analysis,
                repair_evidence,
                snapshot.impact_scope_review.clone(),
                Some(candidate_package),
            )
        }
    };
    let stored_projection = store
        .get_plan_projection_bundle(&lineage, &projection.id)
        .map_err(|error| format!("load Plan projection failed: {error}"))?;
    if revision.plan_projection_bundle_id != projection.id || stored_projection != *projection {
        return Err("Plan Review projection artifact binding mismatch".to_string());
    }
    if revision.dependency_graph_revision_id != projection.dependency_graph_revision_id {
        return Err("Plan Review dependency graph binding mismatch".to_string());
    }
    if projection.human_group_projection.plan_id != lineage.id
        || projection.coder_group_context.plan_id != lineage.id
        || projection.reviewer_group_matrix.plan_id != lineage.id
    {
        return Err("Plan Review plan projection identity mismatch".to_string());
    }
    validate_plan_projection_hashes(projection)?;
    let dependency_contract_graph = store
        .get_dependency_graph_revision(&lineage, &projection.dependency_graph_revision_id)
        .map_err(|error| format!("load Dependency Contract Graph failed: {error}"))?;
    let validation_report = store
        .get_plan_validation_report(&lineage, &revision.validation_report_ref)
        .map_err(|error| format!("load Projection Validation Report failed: {error}"))?;
    if validation_report.plan_id != lineage.id
        || validation_report.plan_revision_id != revision.id
        || validation_report.plan_projection_bundle_id != projection.id
    {
        return Err("Plan Review validation artifact binding mismatch".to_string());
    }
    if let PlanReviewSource::PlanRepairCandidate(snapshot) = source
        && snapshot.validation.as_ref() != Some(&validation_report)
    {
        return Err("Plan Repair review validation provenance mismatch".to_string());
    }
    let projection_validation_report = validation_report.projection_validation.clone();
    let work_item_revisions = revision
        .work_item_bindings
        .iter()
        .map(|(logical_id, revision_id)| {
            store
                .get_work_item_revision(&lineage, logical_id, revision_id)
                .map_err(|error| format!("load Canonical Contract Candidate failed: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let expected_projection_refs = work_item_revisions
        .iter()
        .map(|revision| revision.work_item_projection_bundle_id.clone())
        .collect::<BTreeSet<_>>();
    let actual_projection_refs = projection
        .work_item_projection_bundle_refs
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if expected_projection_refs.len() != work_item_revisions.len()
        || actual_projection_refs.len() != projection.work_item_projection_bundle_refs.len()
        || expected_projection_refs != actual_projection_refs
    {
        return Err("Plan Review WorkItem projection reference set mismatch".to_string());
    }
    let work_item_projection_bundle_candidates = projection
        .work_item_projection_bundle_refs
        .iter()
        .map(|bundle_id| {
            store
                .get_work_item_projection_bundle(&lineage, bundle_id)
                .map_err(|error| format!("load WorkItem projection failed: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if candidate_package_artifact.as_ref().is_some_and(|artifact| {
        artifact.work_item_projection_bundles != work_item_projection_bundle_candidates
    }) {
        return Err("Plan Repair candidate WorkItem projections differ from artifact".to_string());
    }
    validate_work_item_projection_bindings(
        &work_item_revisions,
        &work_item_projection_bundle_candidates,
    )?;
    let canonical_contract_candidates = work_item_revisions
        .iter()
        .map(|revision| revision.canonical_contract.clone())
        .collect();
    let candidate_package_fingerprint = match source {
        PlanReviewSource::InitialActive => None,
        PlanReviewSource::PlanRepairCandidate(snapshot) => {
            let fingerprint = candidate_package_artifact
                .as_ref()
                .ok_or_else(|| "Plan Repair candidate package artifact is missing".to_string())?
                .candidate_package_fingerprint
                .clone();
            if snapshot
                .impact_scope_review
                .as_ref()
                .is_some_and(|proposal| proposal.candidate_package_fingerprint != fingerprint)
            {
                return Err("Plan Repair impact scope review fingerprint mismatch".to_string());
            }
            Some(fingerprint)
        }
    };
    let work_item_ids = revision
        .work_item_bindings
        .keys()
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");

    Ok(PlanReviewContext {
        story_design_traceability: projection.human_group_projection.source_refs.clone(),
        canonical_contract_candidates,
        dependency_contract_graph,
        plan_projection_bundle_candidate: projection.clone(),
        work_item_projection_bundle_candidates,
        projection_validation_report,
        contract_delta,
        impact_analysis: if impact_analysis.is_empty() {
            vec![format!("initial_full_set: {work_item_ids}")]
        } else {
            impact_analysis
        },
        repair_evidence,
        impact_scope_review,
        candidate_package_fingerprint,
    })
}

fn string_set(values: &[String]) -> BTreeSet<&str> {
    values.iter().map(String::as_str).collect()
}

fn validate_work_item_projection_bindings(
    revisions: &[WorkItemRevision],
    bundles: &[WorkItemProjectionBundle],
) -> Result<(), String> {
    let bundles_by_id = bundles
        .iter()
        .map(|bundle| (bundle.id.as_str(), bundle))
        .collect::<BTreeMap<_, _>>();
    for revision in revisions {
        let bundle = bundles_by_id
            .get(revision.work_item_projection_bundle_id.as_str())
            .ok_or_else(|| "Plan Review WorkItem projection bundle missing".to_string())?;
        let logical_id = revision.logical_work_item_id.as_str();
        let actual_contract_hash = canonical_contract_hash(&revision.canonical_contract)
            .map_err(|error| format!("hash Canonical Contract Candidate failed: {error}"))?;
        let projection_hashes = projection_hashes(&CompiledWorkItemProjections {
            human: bundle.human_projection.clone(),
            coder: bundle.coder_projection.clone(),
            reviewer: bundle.reviewer_projection.clone(),
        })
        .map_err(|error| format!("hash WorkItem projection failed: {error}"))?;
        if revision.canonical_contract_hash != actual_contract_hash
            || revision.canonical_contract.identity.logical_work_item_id != logical_id
            || bundle.work_item_revision_id != revision.id
            || bundle.canonical_contract_hash != revision.canonical_contract_hash
            || bundle.human_projection_hash != projection_hashes.human
            || bundle.coder_projection_hash != projection_hashes.coder
            || bundle.reviewer_projection_hash != projection_hashes.reviewer
            || bundle.human_projection.logical_work_item_id != logical_id
            || bundle.coder_projection.work_item_revision_id != revision.id
            || bundle.reviewer_projection.work_item_revision_id != revision.id
        {
            return Err(format!(
                "Plan Review WorkItem projection binding mismatch for `{logical_id}`"
            ));
        }
    }
    Ok(())
}

fn validate_plan_projection_hashes(projection: &PlanProjectionBundle) -> Result<(), String> {
    let hashes = plan_projection_hashes(&CompiledPlanProjections {
        human: projection.human_group_projection.clone(),
        coder: projection.coder_group_context.clone(),
        reviewer: projection.reviewer_group_matrix.clone(),
    })
    .map_err(|error| format!("hash Plan projection failed: {error}"))?;
    if projection.human_group_projection_hash != hashes.human
        || projection.coder_group_context_hash != hashes.coder
        || projection.reviewer_group_matrix_hash != hashes.reviewer
    {
        return Err("Plan Review plan projection payload hash mismatch".to_string());
    }
    Ok(())
}

/// REQ-TOP-04 场景 6（F-52 L2 前轮注入）+ D6（REQ-WSC-06「前轮 finding 分区
/// 注入」）：加载上一 ReviewerRun 持久化 verdict（与 F5-B 闸门同源的单候选
/// 加载形态），其非 advisory findings 逐条经 classify_finding 得到结构化身份
/// 后，以**内容身份**（`contract_field` 去 `plan_preflight.` 前缀 vs 当前候选
/// 机械报告 finding 的 `code`）求交、同身份去重并按「待办 / 已关闭」分区注入
/// ——「某前轮 finding 在本轮是否仍存在」由平台对当前候选机械报告复算得出，
/// 不由模型依据清单文本自行推断。机械报告不可得 ⇒ 如实降级
/// （`mechanical_report_available: false`，前轮 finding 既不进待办也不进
/// 已关闭，不 panic、不得当作「全部已解决」）。首轮（无前轮 verdict 或前轮
/// 无非 advisory finding）返回 None；prompt 字节预算由
/// `ensure_single_candidate_review_prompt_budget`（96KiB）兜底。
pub(crate) fn single_candidate_previous_round_findings_section(
    engine: &WorkspaceEngine,
    current_ir_ref: &str,
) -> Option<String> {
    let active_node_id = engine.active_node_id.as_ref()?;
    let lifecycle = engine.lifecycle_store.as_ref()?;
    let previous = engine
        .timeline_nodes
        .iter()
        .rev()
        .filter(|node| {
            matches!(
                node.node_type,
                crate::web::workspace_ws_types::TimelineNodeType::ReviewerRun
            )
        })
        .filter(|node| &node.node_id != active_node_id)
        .find_map(|node| {
            let detail = lifecycle
                .load_node_detail_for_issue_session(
                    &engine.session.project_id,
                    &engine.session.issue_id,
                    &engine.session.session_id,
                    &node.node_id,
                )
                .ok()?;
            let persisted = detail.verdict?;
            crate::product::workspace_engine::deserialize_historical_review_verdict(persisted).ok()
        })?;
    // REQ-WSC-14（作者/评审输入面一致性）：`previous` 是 durable 原始 verdict（未按
    // 已处置清单归一），此处 MUST 再按 owner 已处置清单过滤——否则同一 prompt 既说
    // 「这是上一轮未收敛 finding」又说「该事实已被 owner 排除」，自相矛盾。
    let excluded_keys = engine.session_human_disposition_keys();
    let previous_findings: Vec<_> = previous
        .findings
        .iter()
        .filter(|finding| !excluded_keys.contains(&finding.computed_finding_key()))
        .map(|finding| {
            crate::product::work_item_plan_policy::classify::classify_finding(
                previous.verdict.clone(),
                finding,
            )
        })
        .filter(|classified| {
            classified.class != crate::product::work_item_plan_policy::FindingClass::Advisory
        })
        .collect();
    if previous_findings.is_empty() {
        return None;
    }
    // D6：装载当前候选机械报告（Evaluate 阶段落盘，与评审输入同代）作为求交
    // 数据源；ref 缺失或装载失败 ⇒ 如实降级（None），不 panic、不静默当作空集。
    let current_codes = current_mechanical_codes(engine, lifecycle);
    let mut unresolved_findings = Vec::new();
    let mut resolved_findings = Vec::new();
    let mut duplicate_count = 0usize;
    let mut seen_identities = BTreeSet::new();
    for classified in &previous_findings {
        let Some(identity) = content_identity(classified.contract_field.as_deref()) else {
            // contract_field 缺失或无 plan_preflight. 前缀 ⇒ 不可机械判定：
            // 既不进待办也不进已关闭（不得当作已解决）。
            continue;
        };
        if !seen_identities.insert(identity.clone()) {
            // 同一内容身份只保留首条，其余折叠为计数（同一 preflight 码对
            // 多个 Work Item 各登记一次的重复噪声不再逐条呈现）。
            duplicate_count += 1;
            continue;
        }
        let Some(codes) = current_codes.as_ref() else {
            // 机械报告不可得 ⇒ 该 finding 既不进待办也不进已关闭。
            continue;
        };
        if codes.contains(&identity) {
            unresolved_findings.push(json!({
                "fingerprint": classified.fingerprint.0,
                "category": classified.category.map(|category| category.as_str()),
                "contract_field": classified.contract_field,
                "required_action": classified.required_action,
                "content_identity": identity,
            }));
        } else {
            resolved_findings.push(json!({
                "fingerprint": classified.fingerprint.0,
                "category": classified.category.map(|category| category.as_str()),
                "contract_field": classified.contract_field,
                "content_identity": identity,
                "resolved_by": current_ir_ref,
            }));
        }
    }
    let payload = json!({
        "note": "上一轮评审已登记的非 advisory finding 分区清单；待办=当前候选机械复算仍存在同内容身份，已关闭=当前候选机械复算已无该内容身份；duplicate_count=同身份被折叠的重复条目数",
        "unresolved_findings": unresolved_findings,
        "resolved_findings": resolved_findings,
        "duplicate_count": duplicate_count,
        "mechanical_report_available": current_codes.is_some(),
        "current_plan_candidate_ir_ref": current_ir_ref,
    });
    let serialized = serde_json::to_string_pretty(&payload).ok()?;
    Some(format!(
        "\n### Previous Review Round Findings\n{serialized}\n\
         [previous_round_repetition_discipline]\n\
         前轮 finding 在本轮是否仍存在，以平台给出的分区复算事实为准（unresolved_findings=仍存在的待办，resolved_findings=当前候选机械复算已确认关闭）：已关闭区条目不得再作为待办重复上报，也不得要求作者再次修改；该判定不得由你依据清单文本自行推断。若待办区某条 fingerprint 在本轮再次出现（同一问题未收敛），必须在该 finding 的 evidence 中显式标注 repeated 并说明为何自动修订未能关闭它；不得以改写措辞的方式隐式重复同一问题。\n"
    ))
}

/// D6（REQ-WSC-06「前轮 finding 状态求交按内容身份」）：前轮 finding 的
/// **内容身份** = `contract_field` 去掉 `plan_preflight.` 前缀后的机械码，
/// 与当前候选机械报告 finding 的 `code` 同域可比。`contract_field` 缺失或
/// 无该前缀 ⇒ None（不可机械判定：既不进待办也不进已关闭）。求交 MUST NOT
/// 用 `FindingFingerprint`——其派生含 `message`，而机械报告 message 与评审
/// message 不同，按指纹求交恒不相交。
pub(crate) fn content_identity(contract_field: Option<&str>) -> Option<String> {
    let code = contract_field?.strip_prefix("plan_preflight.")?;
    (!code.trim().is_empty()).then(|| code.to_string())
}

/// 装载当前候选机械报告的 finding `code` 集合（D6 求交数据源：与评审输入
/// 同代的 `session.mechanical_report_ref`）。ref 缺失或装载失败 ⇒ None
/// （如实降级；不得 panic、不得静默当作空集——空集意味着「全部已关闭」）。
fn current_mechanical_codes(
    engine: &WorkspaceEngine,
    lifecycle: &crate::product::lifecycle_store::LifecycleStore,
) -> Option<BTreeSet<String>> {
    let report_ref = engine.session.mechanical_report_ref.as_deref()?;
    let scope = crate::product::work_item_plan_source_store::SourceStoreScope {
        project_id: engine.session.project_id.clone(),
        issue_id: engine.session.issue_id.clone(),
        plan_id: engine.session.entity_id.clone(),
    };
    let record = crate::product::work_item_plan_source_store::WorkItemPlanSourceStore::new(
        lifecycle.app_paths(),
    )
    .get_mechanical_report(&scope, report_ref)
    .ok()?;
    Some(
        record
            .report
            .findings
            .iter()
            .map(|finding| finding.code.clone())
            .collect(),
    )
}

// 余量按完整多 WI plan 的增长预留；接近上限时先实测，再放宽至整百级。
// 实测记录：2026-09-07 pi×重 levels 单轮 66018B（64KiB 触顶→72KiB）；2026-09-08 pi×重 v4 多轮修订 81923B（72KiB 触顶→96KiB）。若第三次触顶应转 history 压缩调优而非继续放宽。
// 2026-09-07 3.6 矩阵 pi×重 levels 语料多轮修订真跑实测 66018B 触顶旧 64KiB，
// 按上述方针提额至 72KiB（留 ~11% 余量）。
pub(crate) const SINGLE_CANDIDATE_REVIEW_PROMPT_MAX_BYTES: usize = 96 * 1024;

pub(crate) fn ensure_single_candidate_review_prompt_budget(prompt: &str) -> Result<(), String> {
    if prompt.len() <= SINGLE_CANDIDATE_REVIEW_PROMPT_MAX_BYTES {
        Ok(())
    } else {
        Err(format!(
            "single-candidate reviewer prompt exceeds byte budget: actual={} max={}",
            prompt.len(),
            SINGLE_CANDIDATE_REVIEW_PROMPT_MAX_BYTES
        ))
    }
}

impl WorkspaceEngine {
    /// Task 2.4（REQ-ENV-10、REQ-PLN-01/PLN-07）：review builder 的
    /// cwd/target 分离合同（返回 `(target_worktree, session_cwd)`）。
    ///
    /// LC 分支（注入 logical gateway）：Task 2（REQ-PLN-11，设计 A 条目 2）
    /// lc 评审目标降级为 LC 根——target 与 cwd 均为 gateway 冻结的
    /// canonical authority root（经 `working_directory` 字段合同下发；不从
    /// target/worktree/进程 cwd 推导）；不再要求 `session.repository_path`
    /// 是成员 worktree（多仓评审范围以材料集合解析为准，见
    /// `assert_plan_review_material`）。legacy 直连面继续以 `working_dir`
    /// 表达 target——gateway 启动侧以它锚 `PolicyTarget`，spawn 前
    /// effective cwd 复验以 envelope 冻结值为准。
    ///
    /// 单仓分支（无 gateway）：原值不变——cwd 语义仍由 `working_dir` 承载
    ///（`repository_path`，缺省回退进程 cwd），`working_directory` 保持
    /// `None`（存量构造零行为变化）。
    pub(crate) fn review_launch_target_and_cwd(
        &self,
    ) -> Result<(std::path::PathBuf, Option<std::path::PathBuf>), String> {
        if let Some(gateway) = self.logical_provider_gateway() {
            let authority_root = gateway.authority_root().to_path_buf();
            return Ok((authority_root.clone(), Some(authority_root)));
        }
        // 单仓分支逐字不动。注意：lc 会话上 `session.repository_path` 是
        // 兼容字段，非 lc 范围权威（lc 评审范围 = 材料集合解析结果）。
        let working_dir = match &self.session.repository_path {
            Some(path) => path.clone(),
            None => std::env::current_dir()
                .map_err(|error| format!("working directory error: {error}"))?,
        };
        Ok((working_dir, None))
    }
}

/// REQ-WSC-13/14：把该会话 durable 的「owner 已处置事实清单」注入 reviewer
/// prompt。清单为空（owner 从未排除）→ None，输入面零变化。
///
/// 只喂**能拿到上轮上下文**的审查路径（单候选 Plan 复评、带
/// `latest_review_verdict` 的通用评审）；全新候选的 Outline/Draft/Batch/
/// Projection 路径不注入——那些轮次没有「上一轮处置」事实可对账。
pub(crate) fn human_dispositions_section(engine: &WorkspaceEngine) -> Option<String> {
    let lifecycle = engine.lifecycle_store.as_ref()?;
    let records = lifecycle
        .list_human_dispositions(&engine.session.session_id)
        .ok()?;
    render_human_dispositions_section(&records)
}

/// 纯渲染（单测入口）：把 durable 清单投影为 reviewer prompt 段落；空清单
/// → None（输入面零变化）。
pub(super) fn render_human_dispositions_section(
    records: &[crate::product::models::HumanDispositionRecord],
) -> Option<String> {
    if records.is_empty() {
        return None;
    }
    let mut rendered =
        String::from("\n[human_dispositions]\nowner 已处置、不得重复报为 actionable 的事实：\n");
    for record in records {
        let summary = record.message_summary.trim();
        let summary = if summary.is_empty() {
            "<无摘要>"
        } else {
            summary
        };
        rendered.push_str(&format!(
            "- {}：{}（owner 排除/已处置）\n",
            record.finding_key, summary
        ));
    }
    rendered.push_str(
        "上述事实 MUST NOT 再报为 actionable；如仍要提及，只能 disposition=advisory 且 required_action 不得要求作者执行。\n",
    );
    Some(rendered)
}

#[cfg(test)]
mod disposition_section_tests {
    use super::*;
    use crate::product::models::{HumanDispositionRecord, HumanFindingDispositionKind};

    fn record(key: &str, summary: &str) -> HumanDispositionRecord {
        HumanDispositionRecord {
            finding_key: key.to_string(),
            message_summary: summary.to_string(),
            disposition: HumanFindingDispositionKind::Excluded,
            recorded_at: "2026-10-30T00:00:00Z".to_string(),
        }
    }

    /// REQ-WSC-13：清单渲染逐条列 key + 摘要，并声明不得再报为 actionable。
    #[test]
    fn renders_dispositions_with_key_and_summary() {
        let rendered = render_human_dispositions_section(&[
            record("0123456789abcdef", "中文 locale 尚未配置"),
            record("fedcba9876543210", ""),
        ])
        .expect("non-empty ledger renders");

        assert!(rendered.contains("[human_dispositions]"), "{rendered}");
        assert!(rendered.contains("0123456789abcdef"), "{rendered}");
        assert!(rendered.contains("中文 locale 尚未配置"), "{rendered}");
        assert!(rendered.contains("<无摘要>"), "{rendered}");
        assert!(
            rendered.contains("MUST NOT 再报为 actionable"),
            "{rendered}"
        );
    }

    /// REQ-WSC-13：owner 从未排除 → 零注入（输入面逐字不变）。
    #[test]
    fn empty_ledger_injects_nothing() {
        assert!(render_human_dispositions_section(&[]).is_none());
    }
}
