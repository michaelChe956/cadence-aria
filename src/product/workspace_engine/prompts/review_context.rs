use std::collections::{BTreeMap, BTreeSet};

use crate::product::models::{
    DependencyGraphRevision, PlanProjectionBundle, PlanRepairImpactScopeReview,
    PlanRepairRequestStatus, PlanRepairSessionSnapshotDto, PlanRepairSessionStage,
    PlanRevisionReason, RepairTargetKind, WorkItemProjectionBundle, WorkItemRevision,
};
use crate::product::plan_repair::{
    candidate_request_matches_review_status, load_plan_repair_candidate_package,
};
use crate::product::work_item_contract::{CanonicalWorkItemContract, canonical_contract_hash};
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
