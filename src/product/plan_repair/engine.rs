use std::collections::{BTreeMap, BTreeSet};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::product::models::{
    AmendmentResumeMode, AmendmentResumeTarget, DependencyGraphChange, DependencyGraphChangeKind,
    DependencyGraphRevision, PlanAmendmentManifest, PlanRepairCandidatePackageArtifact,
    PlanRevisionReason, PlanValidationReportArtifact, VerificationPlanRevision,
    WorkItemDraftRevision, WorkItemPlanLineage, WorkItemPlanRevision, WorkItemProjectionBundle,
    WorkItemRevision, WorkItemRevisionReplacement,
};
use crate::product::work_item_contract::{
    DependencyContractEdge, DependencyContractGraph, build_dependency_contract_graph,
    validate_dependency_contract_graph,
};
use crate::product::work_item_projection::{
    CompiledWorkItemProjections, PlanProjectionCompileInput, PlanProjectionCompiler,
    WorkItemProjectionCompiler,
};
use crate::product::work_item_revision_store::{
    PlanAmendmentPublicationIds, WorkItemRevisionStore,
};
use crate::product::workspace_engine::{
    CompiledWorkItemRevision, compile_plan_projection_bundle, compile_work_item_revision,
};

use super::{
    ContractDelta, ContractImpactAnalyzer, ContractImpactReport, ImpactExplanationPath,
    PlanExecutionState, PlanRepairError, PlanRepairRequest, RepairTargetKind,
    UnitExecutionSnapshot, build_plan_repair_candidate_package,
    canonical_work_item_projection_bundles, compute_contract_delta,
};

mod publication;

#[cfg(test)]
pub(crate) use publication::final_plan_amendment_manifest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedPlanAmendment {
    pub base_plan_revision_id: String,
    pub publication_ids: PlanAmendmentPublicationIds,
    pub next_plan_revision: WorkItemPlanRevision,
    pub draft_revisions: Vec<WorkItemDraftRevision>,
    pub revised_work_items: Vec<WorkItemRevision>,
    pub verification_plan_revisions: Vec<VerificationPlanRevision>,
    pub work_item_projection_bundles: Vec<WorkItemProjectionBundle>,
    pub dependency_graph_revision: DependencyGraphRevision,
    pub plan_projection_bundle: crate::product::models::PlanProjectionBundle,
    pub validation_report: PlanValidationReportArtifact,
    pub contract_deltas: Vec<ContractDelta>,
    pub impact_report: ContractImpactReport,
    pub manifest: PlanAmendmentManifest,
    pub candidate_package: PlanRepairCandidatePackageArtifact,
}

#[derive(Debug, Clone)]
pub struct PlanRepairEngine {
    pub(super) store: WorkItemRevisionStore,
    pub(super) plan: WorkItemPlanLineage,
    candidate_drafts: BTreeMap<String, WorkItemDraftRevision>,
    execution_state: Option<PlanExecutionState>,
    created_at: String,
}

impl PlanRepairEngine {
    pub fn new(store: WorkItemRevisionStore, plan: WorkItemPlanLineage) -> Self {
        Self {
            store,
            plan,
            candidate_drafts: BTreeMap::new(),
            execution_state: None,
            created_at: Utc::now().to_rfc3339(),
        }
    }

    pub fn with_candidate_drafts(mut self, drafts: Vec<WorkItemDraftRevision>) -> Self {
        self.candidate_drafts = drafts
            .into_iter()
            .map(|draft| (draft.logical_work_item_id.clone(), draft))
            .collect();
        self
    }

    pub fn with_execution_state(mut self, execution_state: PlanExecutionState) -> Self {
        self.execution_state = Some(execution_state);
        self
    }

    pub fn with_created_at(mut self, created_at: impl Into<String>) -> Self {
        self.created_at = created_at.into();
        self
    }

    pub fn replan_subgraph(
        &self,
        graph: &DependencyContractGraph,
        request: &super::SubgraphReplanRequest,
    ) -> Result<super::SubgraphReplanResult, PlanRepairError> {
        if request.plan_id != self.plan.id {
            return Err(invalid_target(
                "subgraph replan request does not belong to this plan lineage",
            ));
        }
        super::SubgraphReplanner::default().replan(graph, request)
    }

    pub fn prepare_amendment(
        &self,
        request: &PlanRepairRequest,
    ) -> Result<PreparedPlanAmendment, PlanRepairError> {
        let (plan, base_revision, amendment_id) = self.validate_prepare_identity(request)?;
        let reason = repair_revision_reason(request)?;
        let next_revision_no = base_revision.revision_no + 1;
        let target_ids = request
            .repair_target
            .logical_work_item_ids
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        if target_ids.len() != request.repair_target.logical_work_item_ids.len()
            || target_ids.len() != self.candidate_drafts.len()
            || self
                .candidate_drafts
                .keys()
                .any(|logical_id| !target_ids.contains(logical_id))
        {
            return Err(invalid_target(
                "repair target and candidate draft identities must match exactly",
            ));
        }
        let target_ids_ordered = target_ids.iter().cloned().collect::<Vec<_>>();
        let publication_ids = self
            .store
            .allocate_plan_amendment_publication_ids(
                &plan,
                &amendment_id,
                next_revision_no,
                &target_ids_ordered,
            )
            .map_err(PlanRepairError::Store)?;

        let expected_previous_ids = request
            .repair_target
            .work_item_revision_ids
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        if expected_previous_ids.len() != target_ids.len() {
            return Err(invalid_target(
                "repair target must bind one previous revision per logical work item",
            ));
        }

        let mut compiled_items = Vec::new();
        let mut revised_items = Vec::new();
        let mut draft_revisions = Vec::new();
        let mut verification_plan_revisions = Vec::new();
        let mut contract_deltas = Vec::new();
        let mut revised_work_items = BTreeMap::new();
        let mut superseded_revisions = Vec::new();
        let mut next_bindings = base_revision.work_item_bindings.clone();

        for (logical_id, previous_revision_id) in &base_revision.work_item_bindings {
            let previous = self
                .store
                .get_work_item_revision(&plan, logical_id, previous_revision_id)
                .map_err(PlanRepairError::Store)?;
            if target_ids.contains(logical_id) {
                if !expected_previous_ids.contains(previous_revision_id) {
                    return Err(invalid_target(
                        "repair target previous revision does not match the active plan binding",
                    ));
                }
                let draft = self
                    .candidate_drafts
                    .get(logical_id)
                    .expect("candidate identities were checked above");
                validate_candidate_draft(draft, request, &reason, logical_id)?;
                let ids = publication_ids
                    .work_items
                    .get(logical_id)
                    .expect("store allocated every revised logical work item");
                let compiled = compile_work_item_revision(
                    draft,
                    &WorkItemProjectionCompiler,
                    ids,
                    &self.created_at,
                )
                .map_err(|error| invalid_target(&error.to_string()))?;
                let delta = compute_contract_delta(
                    &previous.id,
                    &previous.canonical_contract,
                    &compiled.work_item_revision.id,
                    &compiled.work_item_revision.canonical_contract,
                );
                next_bindings.insert(logical_id.clone(), compiled.work_item_revision.id.clone());
                revised_work_items.insert(
                    logical_id.clone(),
                    WorkItemRevisionReplacement {
                        previous_revision_id: previous.id.clone(),
                        next_revision_id: compiled.work_item_revision.id.clone(),
                        delta_kind: delta.kind.clone(),
                    },
                );
                superseded_revisions.push(previous.id);
                draft_revisions.push(compiled.draft_revision.clone());
                revised_items.push(compiled.work_item_revision.clone());
                verification_plan_revisions.push(compiled.verification_plan_revision.clone());
                contract_deltas.push(delta);
                compiled_items.push(compiled);
            } else {
                compiled_items.push(self.load_compiled_item(&plan, &previous)?);
            }
        }

        if revised_items.len() != target_ids.len() {
            return Err(invalid_target(
                "repair target logical work item is missing from the active plan",
            ));
        }
        compiled_items.sort_by(|left, right| {
            left.work_item_revision
                .logical_work_item_id
                .cmp(&right.work_item_revision.logical_work_item_id)
        });
        revised_items
            .sort_by(|left, right| left.logical_work_item_id.cmp(&right.logical_work_item_id));
        draft_revisions
            .sort_by(|left, right| left.logical_work_item_id.cmp(&right.logical_work_item_id));
        verification_plan_revisions
            .sort_by(|left, right| left.logical_work_item_id.cmp(&right.logical_work_item_id));

        let contracts = compiled_items
            .iter()
            .map(|item| item.work_item_revision.canonical_contract.clone())
            .collect::<Vec<_>>();
        let dependency_graph = build_dependency_contract_graph(&contracts)
            .map_err(PlanRepairError::ContractValidation)?;
        let contract_validation = validate_dependency_contract_graph(&dependency_graph);
        if !contract_validation.is_valid() {
            return Err(PlanRepairError::ContractValidation(contract_validation));
        }

        let dependency_graph_revision_id = publication_ids.dependency_graph_revision_id.clone();
        let dependency_graph_revision = DependencyGraphRevision {
            id: dependency_graph_revision_id.clone(),
            plan_id: plan.id.clone(),
            edges: dependency_graph.edges.clone(),
            created_at: self.created_at.clone(),
        };
        let projections = compiled_items
            .iter()
            .map(|item| {
                (
                    item.work_item_revision.logical_work_item_id.clone(),
                    CompiledWorkItemProjections {
                        human: item.projection_bundle.human_projection.clone(),
                        coder: item.projection_bundle.coder_projection.clone(),
                        reviewer: item.projection_bundle.reviewer_projection.clone(),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let plan_revision_id = publication_ids.plan_revision_id.clone();
        let plan_projection_bundle_id = publication_ids.plan_projection_bundle_id.clone();
        let mut source_refs = plan.story_spec_refs.clone();
        source_refs.extend(plan.design_spec_refs.iter().cloned());
        let plan_projection_bundle = compile_plan_projection_bundle(
            &plan_revision_id,
            &dependency_graph_revision_id,
            &plan_projection_bundle_id,
            &self.created_at,
            PlanProjectionCompileInput {
                plan_id: &plan.id,
                goal: "Publish a validated Plan Repair candidate",
                split_reason: &request.reason_code,
                source_refs,
                dependency_graph: &dependency_graph,
                work_item_projections: &projections,
                expected_work_item_revision_ids: next_bindings.clone(),
            },
            &PlanProjectionCompiler,
            &compiled_items,
        )
        .map_err(|error| invalid_target(&error.to_string()))?;
        let work_item_projection_bundles = canonical_work_item_projection_bundles(
            &plan_projection_bundle,
            &compiled_items
                .iter()
                .map(|item| item.projection_bundle.clone())
                .collect::<Vec<_>>(),
        )?;
        let projection_validation =
            crate::product::work_item_projection::ProjectionValidationReport {
                findings: Vec::new(),
            };
        let validation_report_id = publication_ids.validation_report_id.clone();
        let validation_report = PlanValidationReportArtifact {
            id: validation_report_id.clone(),
            plan_id: plan.id.clone(),
            plan_revision_id: plan_revision_id.clone(),
            plan_projection_bundle_id: plan_projection_bundle.id.clone(),
            contract_validation,
            projection_validation,
            created_at: self.created_at.clone(),
        };
        let next_plan_revision = WorkItemPlanRevision {
            id: plan_revision_id.clone(),
            plan_id: plan.id.clone(),
            revision_no: next_revision_no,
            supersedes: Some(base_revision.id.clone()),
            reason,
            work_item_bindings: next_bindings,
            dependency_graph_revision_id,
            validation_report_ref: validation_report_id,
            plan_projection_bundle_id: plan_projection_bundle.id.clone(),
            created_at: self.created_at.clone(),
        };
        let base_graph = self
            .store
            .get_dependency_graph_revision(&plan, &base_revision.dependency_graph_revision_id)
            .map_err(PlanRepairError::Store)?;
        let dependency_graph_changes =
            dependency_graph_changes(&base_graph.edges, &dependency_graph.edges);
        let execution = self
            .execution_state
            .clone()
            .unwrap_or_else(|| default_execution_state(&base_revision));
        let impact_report = aggregate_plan_repair_impact(
            &dependency_graph,
            &contract_deltas,
            &execution,
            revised_work_items.keys(),
        )?;
        let resume_target = resume_target(&impact_report, revised_work_items.keys());
        let manifest = PlanAmendmentManifest {
            id: amendment_id,
            repair_request_id: request.id.clone(),
            previous_plan_revision_id: base_revision.id.clone(),
            new_plan_revision_id: plan_revision_id,
            revised_work_items,
            superseded_revisions,
            dependency_graph_changes,
            contract_deltas: contract_deltas.clone(),
            unaffected_units: impact_report.unaffected.clone(),
            revalidation_required_units: impact_report.direct_revalidation.clone(),
            stale_units: impact_report.direct_stale.clone(),
            replacement_units: BTreeMap::new(),
            resume_target,
            created_at: self.created_at.clone(),
        };
        let candidate_package = build_plan_repair_candidate_package(
            &plan,
            request,
            &manifest,
            &plan_projection_bundle,
            &work_item_projection_bundles,
            &validation_report,
            &impact_report,
        )?;

        Ok(PreparedPlanAmendment {
            base_plan_revision_id: base_revision.id,
            publication_ids,
            next_plan_revision,
            draft_revisions,
            revised_work_items: revised_items,
            verification_plan_revisions,
            work_item_projection_bundles,
            dependency_graph_revision,
            plan_projection_bundle,
            validation_report,
            contract_deltas,
            impact_report,
            manifest,
            candidate_package,
        })
    }

    pub fn persist_candidate(
        &self,
        prepared: &PreparedPlanAmendment,
    ) -> Result<(), PlanRepairError> {
        let plan = self
            .store
            .get_plan_lineage(&self.plan.project_id, &self.plan.issue_id, &self.plan.id)
            .map_err(PlanRepairError::Store)?;
        if plan.active_revision_id.as_deref() != Some(prepared.base_plan_revision_id.as_str())
            || plan.active_amendment_id.as_deref() != Some(prepared.manifest.id.as_str())
            || prepared.next_plan_revision.supersedes.as_deref()
                != Some(prepared.base_plan_revision_id.as_str())
            || prepared.next_plan_revision.id != prepared.manifest.new_plan_revision_id
            || prepared.validation_report.plan_revision_id != prepared.next_plan_revision.id
            || prepared.validation_report.plan_projection_bundle_id
                != prepared.plan_projection_bundle.id
        {
            return Err(PlanRepairError::AmendmentConflict {
                expected: format!(
                    "{}@{}",
                    prepared.base_plan_revision_id, prepared.manifest.id
                ),
                actual: format!(
                    "{}@{}",
                    plan.active_revision_id.unwrap_or_default(),
                    plan.active_amendment_id.unwrap_or_default()
                ),
            });
        }
        let canonical_package = build_plan_repair_candidate_package(
            &plan,
            &prepared.candidate_package.request,
            &prepared.manifest,
            &prepared.plan_projection_bundle,
            &prepared.work_item_projection_bundles,
            &prepared.validation_report,
            &prepared.impact_report,
        )?;
        if canonical_package != prepared.candidate_package {
            return Err(invalid_target(
                "prepared candidate package differs from canonical package",
            ));
        }
        for draft in &prepared.draft_revisions {
            self.store
                .put_draft_revision(&plan, draft)
                .map_err(PlanRepairError::Store)?;
        }
        for verification in &prepared.verification_plan_revisions {
            self.store
                .put_verification_plan_revision(&plan, verification)
                .map_err(PlanRepairError::Store)?;
        }
        for bundle in &prepared.work_item_projection_bundles {
            self.store
                .put_work_item_projection_bundle(&plan, bundle)
                .map_err(PlanRepairError::Store)?;
        }
        for revision in &prepared.revised_work_items {
            self.store
                .put_work_item_revision(&plan, revision)
                .map_err(PlanRepairError::Store)?;
        }
        self.store
            .put_dependency_graph_revision(&plan, &prepared.dependency_graph_revision)
            .map_err(PlanRepairError::Store)?;
        self.store
            .put_plan_projection_bundle(&plan, &prepared.plan_projection_bundle)
            .map_err(PlanRepairError::Store)?;
        self.store
            .put_plan_validation_report(&plan, &prepared.validation_report)
            .map_err(PlanRepairError::Store)?;
        self.store
            .put_plan_revision(&plan, &prepared.next_plan_revision)
            .map_err(PlanRepairError::Store)?;
        self.store
            .put_plan_repair_candidate_package(&plan, &prepared.candidate_package)
            .map_err(PlanRepairError::Store)
    }

    fn validate_prepare_identity(
        &self,
        request: &PlanRepairRequest,
    ) -> Result<(WorkItemPlanLineage, WorkItemPlanRevision, String), PlanRepairError> {
        let plan = self
            .store
            .get_plan_lineage(&self.plan.project_id, &self.plan.issue_id, &self.plan.id)
            .map_err(PlanRepairError::Store)?;
        let amendment_id = request
            .amendment_id
            .clone()
            .ok_or_else(|| invalid_target("plan repair request has no amendment id"))?;
        if request.plan_id != plan.id
            || plan.active_revision_id.as_deref() != Some(request.base_plan_revision_id.as_str())
            || plan.active_amendment_id.as_deref() != Some(amendment_id.as_str())
        {
            return Err(PlanRepairError::AmendmentConflict {
                expected: format!("{}@{}", request.base_plan_revision_id, amendment_id),
                actual: format!(
                    "{}@{}",
                    plan.active_revision_id.clone().unwrap_or_default(),
                    plan.active_amendment_id.clone().unwrap_or_default()
                ),
            });
        }
        let stored_request = self
            .store
            .get_repair_request(&plan, &request.id)
            .map_err(PlanRepairError::Store)?;
        if stored_request != *request {
            return Err(invalid_target(
                "prepare requires the authoritative persisted repair request",
            ));
        }
        let base_revision = self
            .store
            .get_plan_revision(
                &plan.project_id,
                &plan.issue_id,
                &plan.id,
                &request.base_plan_revision_id,
            )
            .map_err(PlanRepairError::Store)?;
        Ok((plan, base_revision, amendment_id))
    }

    fn load_compiled_item(
        &self,
        plan: &WorkItemPlanLineage,
        revision: &WorkItemRevision,
    ) -> Result<CompiledWorkItemRevision, PlanRepairError> {
        Ok(CompiledWorkItemRevision {
            draft_revision: self
                .store
                .get_draft_revision(plan, &revision.source_draft_revision_id)
                .map_err(PlanRepairError::Store)?,
            work_item_revision: revision.clone(),
            verification_plan_revision: self
                .store
                .get_verification_plan_revision(plan, &revision.verification_plan_revision_id)
                .map_err(PlanRepairError::Store)?,
            projection_bundle: self
                .store
                .get_work_item_projection_bundle(plan, &revision.work_item_projection_bundle_id)
                .map_err(PlanRepairError::Store)?,
        })
    }
}

fn repair_revision_reason(
    request: &PlanRepairRequest,
) -> Result<PlanRevisionReason, PlanRepairError> {
    match request.repair_target.kind {
        RepairTargetKind::CurrentWorkItem => Ok(PlanRevisionReason::RepairCurrentWorkItem),
        RepairTargetKind::UpstreamWorkItem => Ok(PlanRevisionReason::RepairUpstreamContract),
        RepairTargetKind::Subgraph => Ok(PlanRevisionReason::SubgraphReplan),
    }
}

fn validate_candidate_draft(
    draft: &WorkItemDraftRevision,
    request: &PlanRepairRequest,
    reason: &PlanRevisionReason,
    logical_id: &str,
) -> Result<(), PlanRepairError> {
    if draft.logical_work_item_id != logical_id
        || draft
            .canonical_contract_candidate
            .identity
            .logical_work_item_id
            != logical_id
        || draft.revision_reason != *reason
        || draft.trigger_repair_request_id.as_deref() != Some(request.id.as_str())
    {
        return Err(invalid_target(
            "candidate draft is not bound to the repair request and target",
        ));
    }
    Ok(())
}

fn dependency_graph_changes(
    previous: &[DependencyContractEdge],
    next: &[DependencyContractEdge],
) -> Vec<DependencyGraphChange> {
    let previous = previous
        .iter()
        .map(|edge| ((edge.from.clone(), edge.to.clone()), edge))
        .collect::<BTreeMap<_, _>>();
    let next = next
        .iter()
        .map(|edge| ((edge.from.clone(), edge.to.clone()), edge))
        .collect::<BTreeMap<_, _>>();
    let keys = previous
        .keys()
        .chain(next.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    keys.into_iter()
        .filter_map(|key| match (previous.get(&key), next.get(&key)) {
            (Some(previous), Some(next)) if previous != next => Some(DependencyGraphChange {
                kind: DependencyGraphChangeKind::EdgeReplaced,
                previous: Some((*previous).clone()),
                next: Some((*next).clone()),
            }),
            (Some(previous), None) => Some(DependencyGraphChange {
                kind: DependencyGraphChangeKind::EdgeRemoved,
                previous: Some((*previous).clone()),
                next: None,
            }),
            (None, Some(next)) => Some(DependencyGraphChange {
                kind: DependencyGraphChangeKind::EdgeAdded,
                previous: None,
                next: Some((*next).clone()),
            }),
            _ => None,
        })
        .collect()
}

fn default_execution_state(revision: &WorkItemPlanRevision) -> PlanExecutionState {
    PlanExecutionState {
        units: revision
            .work_item_bindings
            .iter()
            .map(|(logical_id, revision_id)| {
                (
                    logical_id.clone(),
                    UnitExecutionSnapshot {
                        logical_work_item_id: logical_id.clone(),
                        work_item_revision_id: revision_id.clone(),
                        completed_handoff_revision_id: None,
                        has_started: false,
                        has_completed: false,
                    },
                )
            })
            .collect(),
    }
}

pub(crate) fn aggregate_plan_repair_impact<'a>(
    graph: &DependencyContractGraph,
    deltas: &[ContractDelta],
    execution: &PlanExecutionState,
    revised_ids: impl Iterator<Item = &'a String>,
) -> Result<ContractImpactReport, PlanRepairError> {
    let mut revalidation = BTreeSet::new();
    let mut stale = BTreeSet::new();
    let mut conditional = BTreeSet::new();
    let mut paths = Vec::<ImpactExplanationPath>::new();
    for delta in deltas {
        let report = ContractImpactAnalyzer.analyze_static(graph, delta, execution)?;
        revalidation.extend(report.direct_revalidation);
        stale.extend(report.direct_stale);
        conditional.extend(report.conditional_downstream);
        paths.extend(report.explanation_paths);
    }
    revalidation.retain(|unit| !stale.contains(unit));
    let revised = revised_ids.cloned().collect::<BTreeSet<_>>();
    let unaffected = graph
        .contracts
        .keys()
        .filter(|id| {
            !revised.contains(*id)
                && !revalidation.contains(*id)
                && !stale.contains(*id)
                && !conditional.contains(*id)
        })
        .cloned()
        .collect();
    paths.sort_by(|left, right| {
        (
            &left.from,
            &left.to,
            &left.contract_id,
            &left.capability_refs,
        )
            .cmp(&(
                &right.from,
                &right.to,
                &right.contract_id,
                &right.capability_refs,
            ))
    });
    paths.dedup();
    Ok(ContractImpactReport {
        unaffected,
        direct_revalidation: revalidation.into_iter().collect(),
        direct_stale: stale.into_iter().collect(),
        conditional_downstream: conditional.into_iter().collect(),
        explanation_paths: paths,
    })
}

fn resume_target<'a>(
    impact: &ContractImpactReport,
    mut revised_ids: impl Iterator<Item = &'a String>,
) -> AmendmentResumeTarget {
    if let Some(logical_id) = impact.direct_stale.first() {
        return AmendmentResumeTarget {
            logical_work_item_id: logical_id.clone(),
            mode: AmendmentResumeMode::Reexecute,
        };
    }
    if let Some(logical_id) = impact.direct_revalidation.first() {
        return AmendmentResumeTarget {
            logical_work_item_id: logical_id.clone(),
            mode: AmendmentResumeMode::Revalidate,
        };
    }
    AmendmentResumeTarget {
        logical_work_item_id: revised_ids.next().cloned().unwrap_or_default(),
        mode: AmendmentResumeMode::AwaitHandoff,
    }
}

fn invalid_target(message: &str) -> PlanRepairError {
    PlanRepairError::InvalidRepairTarget(message.to_string())
}
