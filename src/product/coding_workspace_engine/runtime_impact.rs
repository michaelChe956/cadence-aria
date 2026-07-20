use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::product::coding_models::{CodingExecutionAttempt, CodingUnitRunStatus};
use crate::product::models::{ContractDeltaKind, HandoffRevision, PlanAmendmentManifest};
use crate::product::plan_repair::{
    ContractCapabilityAssociation, ContractDelta, ContractImpactAnalyzer, ContractImpactReport,
    PlanExecutionState, UnitExecutionSnapshot,
};
use crate::product::work_item_contract::{
    ContractCompatibilityPolicy, DependencyContractEdge, DependencyContractGraph,
    RequiredDependencyContract,
};
use crate::product::work_item_revision_store::WorkItemRevisionStore;

use super::runtime_handoff_authority::AuthoritativeHandoffTransition;
use super::{CodingWorkspaceEngine, CodingWorkspaceEngineError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandoffDeltaKind {
    Unchanged,
    CompatibleExtension,
    BreakingChange,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeHandoffImpactResult {
    pub resumed_units: Vec<String>,
    pub revalidation_units: Vec<String>,
    pub newly_stale_units: Vec<String>,
    pub conditional_units_released: Vec<String>,
    pub propagation_stopped_at: Option<String>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct RuntimeHandoffImpactPropagator;

struct RuntimeHandoffState<'a> {
    execution: &'a PlanExecutionState,
    latest_statuses: &'a BTreeMap<String, CodingUnitRunStatus>,
    authoritative_handoffs: &'a BTreeMap<String, HandoffRevision>,
}

#[derive(Serialize)]
struct HandoffContractHashInput<'a> {
    provided_contracts: &'a [String],
    provided_capabilities: &'a BTreeMap<String, Vec<String>>,
}

pub(super) fn stable_handoff_contract_hash(
    provided_contracts: &[String],
    provided_capabilities: &BTreeMap<String, Vec<String>>,
) -> Result<String, CodingWorkspaceEngineError> {
    let mut provided_contracts = provided_contracts.to_vec();
    provided_contracts.sort();
    provided_contracts.dedup();
    let provided_capabilities = provided_capabilities
        .iter()
        .map(|(contract_id, capabilities)| {
            let mut capabilities = capabilities.clone();
            capabilities.sort();
            capabilities.dedup();
            (contract_id.clone(), capabilities)
        })
        .collect::<BTreeMap<_, _>>();
    let bytes = serde_json::to_vec(&HandoffContractHashInput {
        provided_contracts: &provided_contracts,
        provided_capabilities: &provided_capabilities,
    })
    .map_err(|error| {
        CodingWorkspaceEngineError::ProviderStream(format!("handoff_contract_hash_failed: {error}"))
    })?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

pub fn compare_handoff_revisions(
    previous: Option<&HandoffRevision>,
    next: &HandoffRevision,
) -> HandoffDeltaKind {
    let Some(previous) = previous else {
        return HandoffDeltaKind::CompatibleExtension;
    };
    if previous.contract_hash == next.contract_hash {
        return HandoffDeltaKind::Unchanged;
    }

    let previous_contracts = previous.provided_contracts.iter().collect::<BTreeSet<_>>();
    let next_contracts = next.provided_contracts.iter().collect::<BTreeSet<_>>();
    let capabilities_are_extended =
        previous
            .provided_capabilities
            .iter()
            .all(|(contract_id, capabilities)| {
                let Some(next_capabilities) = next.provided_capabilities.get(contract_id) else {
                    return false;
                };
                let previous_capabilities = capabilities.iter().collect::<BTreeSet<_>>();
                let next_capabilities = next_capabilities.iter().collect::<BTreeSet<_>>();
                previous_capabilities.is_subset(&next_capabilities)
            });

    if previous_contracts.is_subset(&next_contracts) && capabilities_are_extended {
        HandoffDeltaKind::CompatibleExtension
    } else {
        HandoffDeltaKind::BreakingChange
    }
}

impl RuntimeHandoffImpactPropagator {
    pub fn apply_completed_handoff(
        &self,
        _attempt: &CodingExecutionAttempt,
        next_handoff: &HandoffRevision,
        manifest: &PlanAmendmentManifest,
        graph: &DependencyContractGraph,
    ) -> Result<RuntimeHandoffImpactResult, CodingWorkspaceEngineError> {
        let execution = PlanExecutionState {
            units: graph
                .contracts
                .keys()
                .map(|logical_id| {
                    (
                        logical_id.clone(),
                        UnitExecutionSnapshot {
                            logical_work_item_id: logical_id.clone(),
                            work_item_revision_id: String::new(),
                            completed_handoff_revision_id: None,
                            has_started: false,
                            has_completed: false,
                        },
                    )
                })
                .collect(),
        };
        let latest_statuses = BTreeMap::new();
        let authoritative_handoffs = BTreeMap::new();
        self.apply_with_runtime_state(
            None,
            next_handoff,
            manifest,
            graph,
            &RuntimeHandoffState {
                execution: &execution,
                latest_statuses: &latest_statuses,
                authoritative_handoffs: &authoritative_handoffs,
            },
        )
    }

    fn apply_with_runtime_state(
        &self,
        previous: Option<&HandoffRevision>,
        next: &HandoffRevision,
        manifest: &PlanAmendmentManifest,
        graph: &DependencyContractGraph,
        runtime: &RuntimeHandoffState<'_>,
    ) -> Result<RuntimeHandoffImpactResult, CodingWorkspaceEngineError> {
        let delta_kind = compare_handoff_revisions(previous, next);
        if delta_kind == HandoffDeltaKind::Unchanged {
            let mut resumed_units = direct_consumers(graph, &next.logical_work_item_id)
                .filter(|edge| {
                    incoming_edges_are_satisfied(
                        graph,
                        &edge.to,
                        next,
                        runtime.authoritative_handoffs,
                    )
                })
                .map(|edge| edge.to.clone())
                .filter(|logical_id| {
                    runtime.latest_statuses.get(logical_id)
                        == Some(&CodingUnitRunStatus::AwaitingAmendment)
                })
                .collect::<Vec<_>>();
            resumed_units.sort();
            resumed_units.dedup();
            return Ok(RuntimeHandoffImpactResult {
                resumed_units,
                propagation_stopped_at: Some(next.logical_work_item_id.clone()),
                ..RuntimeHandoffImpactResult::default()
            });
        }

        let delta = runtime_contract_delta(previous, next, delta_kind.clone());
        let mut report = ContractImpactAnalyzer
            .analyze_static(graph, &delta, runtime.execution)
            .map_err(|error| {
                CodingWorkspaceEngineError::ProviderStream(format!(
                    "runtime_handoff_impact_analysis_failed: {error:?}"
                ))
            })?;
        if delta_kind == HandoffDeltaKind::BreakingChange {
            report.direct_stale.extend(
                direct_consumers(graph, &next.logical_work_item_id).map(|edge| edge.to.clone()),
            );
            report.direct_stale.sort();
            report.direct_stale.dedup();
        }
        Ok(classify_runtime_impact(
            next,
            manifest,
            &report,
            runtime.latest_statuses,
            delta_kind,
            graph,
        ))
    }
}

impl CodingWorkspaceEngine {
    pub async fn apply_completed_handoff(
        &self,
        attempt: &CodingExecutionAttempt,
        next_handoff: &HandoffRevision,
    ) -> Result<RuntimeHandoffImpactResult, CodingWorkspaceEngineError> {
        let current = self.store.validate_attempt_lineage(attempt)?;
        let binding = self.store.get_plan_binding(&current)?;
        let Some(_) = binding.applied_amendment_ids.last() else {
            return Ok(RuntimeHandoffImpactResult::default());
        };
        let revision_store = WorkItemRevisionStore::new(self.store.paths());
        let lineage = revision_store.get_plan_lineage(
            &current.project_id,
            &current.issue_id,
            &binding.plan_id,
        )?;
        let persisted = revision_store.get_handoff_revision(
            &lineage,
            &next_handoff.logical_work_item_id,
            &next_handoff.id,
        )?;
        if persisted != *next_handoff {
            return Err(CodingWorkspaceEngineError::ProviderStream(format!(
                "runtime_handoff_revision_mismatch: {}",
                next_handoff.id
            )));
        }
        let next_run = self.authoritative_handoff_run(&current, next_handoff)?;
        let previous = self.authoritative_previous_handoff_for_run(
            &current,
            &lineage,
            next_handoff,
            &next_run,
        )?;
        let transition =
            self.authoritative_handoff_transition(&current, previous, next_handoff.clone())?;
        self.apply_authoritative_handoff_transition(&current, transition)
            .await
    }

    pub(super) async fn apply_authoritative_handoff_transition(
        &self,
        attempt: &CodingExecutionAttempt,
        transition: AuthoritativeHandoffTransition,
    ) -> Result<RuntimeHandoffImpactResult, CodingWorkspaceEngineError> {
        let current = self.store.validate_attempt_lineage(attempt)?;
        let binding = self.store.get_plan_binding(&current)?;
        let Some(amendment_id) = binding.applied_amendment_ids.last() else {
            return Ok(RuntimeHandoffImpactResult::default());
        };
        let revision_store = WorkItemRevisionStore::new(self.store.paths());
        let lineage = revision_store.get_plan_lineage(
            &current.project_id,
            &current.issue_id,
            &binding.plan_id,
        )?;
        for handoff in transition
            .previous
            .iter()
            .chain(std::iter::once(&transition.next))
        {
            let persisted = revision_store.get_handoff_revision(
                &lineage,
                &handoff.logical_work_item_id,
                &handoff.id,
            )?;
            if persisted != *handoff {
                return Err(CodingWorkspaceEngineError::ProviderStream(format!(
                    "runtime_handoff_revision_mismatch: {}",
                    handoff.id
                )));
            }
        }
        let transition =
            self.authoritative_handoff_transition(&current, transition.previous, transition.next)?;
        let manifest = revision_store.get_amendment_manifest(&lineage, amendment_id)?;
        if binding.bound_plan_revision_id != manifest.new_plan_revision_id {
            return Err(CodingWorkspaceEngineError::ProviderStream(format!(
                "runtime_handoff_plan_binding_mismatch: {}",
                current.id
            )));
        }
        let plan_revision = revision_store.get_plan_revision(
            &current.project_id,
            &current.issue_id,
            &lineage.id,
            &binding.bound_plan_revision_id,
        )?;
        let graph_revision = revision_store
            .get_dependency_graph_revision(&lineage, &plan_revision.dependency_graph_revision_id)?;
        let contracts = plan_revision
            .work_item_bindings
            .iter()
            .map(|(logical_id, revision_id)| {
                revision_store
                    .get_work_item_revision(&lineage, logical_id, revision_id)
                    .map(|revision| (logical_id.clone(), revision.canonical_contract))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let graph = DependencyContractGraph {
            contracts,
            edges: graph_revision.edges,
        };
        let authoritative_handoffs = self.runtime_authoritative_handoffs(&current, &lineage)?;
        let (execution, latest_statuses) = self.runtime_handoff_execution_state(&current)?;
        let result = RuntimeHandoffImpactPropagator.apply_with_runtime_state(
            transition.previous.as_ref(),
            &transition.next,
            &manifest,
            &graph,
            &RuntimeHandoffState {
                execution: &execution,
                latest_statuses: &latest_statuses,
                authoritative_handoffs: &authoritative_handoffs,
            },
        )?;
        self.persist_runtime_handoff_impact(
            &current,
            &manifest,
            &transition.next,
            &graph,
            &authoritative_handoffs,
            &result,
        )?;
        Ok(result)
    }

    fn runtime_authoritative_handoffs(
        &self,
        attempt: &CodingExecutionAttempt,
        lineage: &crate::product::models::WorkItemPlanLineage,
    ) -> Result<BTreeMap<String, HandoffRevision>, CodingWorkspaceEngineError> {
        let revision_store = WorkItemRevisionStore::new(self.store.paths());
        let mut handoffs = BTreeMap::new();
        for unit in
            self.store
                .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)?
        {
            let Some(handoff_id) = unit.latest_handoff_revision_id.as_deref() else {
                continue;
            };
            let handoff = revision_store.get_handoff_revision(
                lineage,
                &unit.logical_work_item_id,
                handoff_id,
            )?;
            self.authoritative_handoff_run(attempt, &handoff)?;
            if handoffs
                .insert(unit.logical_work_item_id.clone(), handoff)
                .is_some()
            {
                return Err(CodingWorkspaceEngineError::ProviderStream(format!(
                    "runtime_handoff_unit_ambiguous: {}",
                    unit.logical_work_item_id
                )));
            }
        }
        Ok(handoffs)
    }

    fn runtime_handoff_execution_state(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<
        (PlanExecutionState, BTreeMap<String, CodingUnitRunStatus>),
        CodingWorkspaceEngineError,
    > {
        let mut snapshots = BTreeMap::new();
        let mut statuses = BTreeMap::new();
        for unit in
            self.store
                .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)?
        {
            let latest = self
                .store
                .list_coding_unit_runs(attempt, &unit.id)?
                .into_iter()
                .max_by_key(|run| run.execution_no);
            if let Some(run) = latest.as_ref() {
                statuses.insert(unit.logical_work_item_id.clone(), run.status.clone());
            }
            snapshots.insert(
                unit.logical_work_item_id.clone(),
                UnitExecutionSnapshot {
                    logical_work_item_id: unit.logical_work_item_id,
                    work_item_revision_id: unit.work_item_revision_id,
                    completed_handoff_revision_id: unit.latest_handoff_revision_id,
                    has_started: latest.as_ref().is_some_and(|run| {
                        !matches!(
                            run.status,
                            CodingUnitRunStatus::Pending | CodingUnitRunStatus::AwaitingAmendment
                        )
                    }),
                    has_completed: latest
                        .as_ref()
                        .is_some_and(|run| run.status == CodingUnitRunStatus::Completed),
                },
            );
        }
        Ok((PlanExecutionState { units: snapshots }, statuses))
    }

    fn persist_runtime_handoff_impact(
        &self,
        attempt: &CodingExecutionAttempt,
        manifest: &PlanAmendmentManifest,
        next_handoff: &HandoffRevision,
        graph: &DependencyContractGraph,
        authoritative_handoffs: &BTreeMap<String, HandoffRevision>,
        result: &RuntimeHandoffImpactResult,
    ) -> Result<(), CodingWorkspaceEngineError> {
        for (logical_ids, status) in [
            (&result.resumed_units, CodingUnitRunStatus::Pending),
            (
                &result.revalidation_units,
                CodingUnitRunStatus::NeedsRevalidation,
            ),
            (&result.newly_stale_units, CodingUnitRunStatus::Stale),
            (
                &result.conditional_units_released,
                CodingUnitRunStatus::Pending,
            ),
        ] {
            for logical_id in logical_ids {
                let resolved_handoff_revision_ids = self.runtime_resolved_handoff_revision_ids(
                    attempt,
                    logical_id,
                    next_handoff,
                    graph,
                    authoritative_handoffs,
                    status != CodingUnitRunStatus::Stale,
                )?;
                self.store.resolve_runtime_handoff_unit_run(
                    attempt,
                    &manifest.id,
                    logical_id,
                    &resolved_handoff_revision_ids,
                    status.clone(),
                )?;
            }
        }
        Ok(())
    }

    fn runtime_resolved_handoff_revision_ids(
        &self,
        attempt: &CodingExecutionAttempt,
        logical_work_item_id: &str,
        next_handoff: &HandoffRevision,
        graph: &DependencyContractGraph,
        authoritative_handoffs: &BTreeMap<String, HandoffRevision>,
        require_capabilities: bool,
    ) -> Result<Vec<String>, CodingWorkspaceEngineError> {
        let units =
            self.store
                .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        unique_runtime_unit(&units, logical_work_item_id)?;
        let mut resolved = graph
            .edges
            .iter()
            .filter(|edge| edge.to == logical_work_item_id)
            .map(|edge| {
                let handoff = if edge.from == next_handoff.logical_work_item_id {
                    next_handoff
                } else {
                    authoritative_handoffs.get(&edge.from).ok_or_else(|| {
                        CodingWorkspaceEngineError::WorkItemHandoffMissing(edge.from.clone())
                    })?
                };
                if require_capabilities && !edge_requirements_are_satisfied(edge, handoff) {
                    return Err(CodingWorkspaceEngineError::ProviderStream(format!(
                        "runtime_handoff_dependency_unsatisfied: {}->{}",
                        edge.from, edge.to
                    )));
                }
                Ok(handoff.id.clone())
            })
            .collect::<Result<Vec<_>, _>>()?;
        resolved.sort();
        resolved.dedup();
        Ok(resolved)
    }
}

fn classify_runtime_impact(
    next: &HandoffRevision,
    manifest: &PlanAmendmentManifest,
    report: &ContractImpactReport,
    latest_statuses: &BTreeMap<String, CodingUnitRunStatus>,
    delta_kind: HandoffDeltaKind,
    graph: &DependencyContractGraph,
) -> RuntimeHandoffImpactResult {
    let explicit_revalidation = manifest
        .revalidation_required_units
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut direct = report
        .direct_revalidation
        .iter()
        .chain(report.direct_stale.iter())
        .cloned()
        .collect::<BTreeSet<_>>();
    if delta_kind == HandoffDeltaKind::CompatibleExtension {
        direct.extend(
            explicit_revalidation
                .iter()
                .filter(|logical_id| {
                    graph.edges.iter().any(|edge| {
                        edge.from == next.logical_work_item_id && &edge.to == *logical_id
                    })
                })
                .cloned(),
        );
    }
    let mut result = RuntimeHandoffImpactResult::default();
    match delta_kind {
        HandoffDeltaKind::Unchanged => unreachable!("unchanged returns before impact analysis"),
        HandoffDeltaKind::CompatibleExtension => {
            for logical_id in direct {
                if explicit_revalidation.contains(&logical_id) {
                    result.revalidation_units.push(logical_id);
                } else if matches!(
                    latest_statuses.get(&logical_id),
                    Some(CodingUnitRunStatus::AwaitingAmendment | CodingUnitRunStatus::Pending)
                ) {
                    result.resumed_units.push(logical_id);
                } else {
                    result.conditional_units_released.push(logical_id);
                }
            }
        }
        HandoffDeltaKind::BreakingChange => {
            result.newly_stale_units.extend(direct);
        }
    }
    sort_runtime_result(&mut result);
    if result.resumed_units.is_empty()
        && result.revalidation_units.is_empty()
        && result.newly_stale_units.is_empty()
        && result.conditional_units_released.is_empty()
    {
        result.propagation_stopped_at = Some(next.logical_work_item_id.clone());
    }
    result
}

fn runtime_contract_delta(
    previous: Option<&HandoffRevision>,
    next: &HandoffRevision,
    kind: HandoffDeltaKind,
) -> ContractDelta {
    let previous_contracts = previous
        .map(|handoff| handoff.provided_contracts.iter().cloned().collect())
        .unwrap_or_default();
    let next_contracts = next
        .provided_contracts
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let previous_associations = handoff_capability_associations(previous);
    let next_associations = handoff_capability_associations(Some(next));
    let previous_capabilities = previous_associations
        .iter()
        .map(|association| association.capability.clone())
        .collect::<BTreeSet<_>>();
    let next_capabilities = next_associations
        .iter()
        .map(|association| association.capability.clone())
        .collect::<BTreeSet<_>>();
    ContractDelta {
        logical_work_item_id: next.logical_work_item_id.clone(),
        previous_revision_id: previous
            .map(|handoff| handoff.work_item_revision_id.clone())
            .unwrap_or_default(),
        next_revision_id: next.work_item_revision_id.clone(),
        kind: match kind {
            HandoffDeltaKind::Unchanged => ContractDeltaKind::InformativeOnly,
            HandoffDeltaKind::CompatibleExtension => ContractDeltaKind::CompatibleContractExtension,
            HandoffDeltaKind::BreakingChange => ContractDeltaKind::BreakingContractChange,
        },
        added_contracts: next_contracts
            .difference(&previous_contracts)
            .cloned()
            .collect(),
        removed_contracts: previous_contracts
            .difference(&next_contracts)
            .cloned()
            .collect(),
        added_capabilities: next_capabilities
            .difference(&previous_capabilities)
            .cloned()
            .collect(),
        removed_capabilities: previous_capabilities
            .difference(&next_capabilities)
            .cloned()
            .collect(),
        changed_capabilities: Vec::new(),
        added_capability_associations: next_associations
            .difference(&previous_associations)
            .cloned()
            .collect(),
        removed_capability_associations: previous_associations
            .difference(&next_associations)
            .cloned()
            .collect(),
        acceptance_changed: false,
        verification_changed: false,
        write_policy_changed: false,
    }
}

fn handoff_capability_associations(
    handoff: Option<&HandoffRevision>,
) -> BTreeSet<ContractCapabilityAssociation> {
    handoff
        .into_iter()
        .flat_map(|handoff| &handoff.provided_capabilities)
        .flat_map(|(contract_id, capabilities)| {
            capabilities
                .iter()
                .map(move |capability| ContractCapabilityAssociation {
                    contract_id: contract_id.clone(),
                    capability: capability.clone(),
                })
        })
        .collect()
}

fn direct_consumers<'a>(
    graph: &'a DependencyContractGraph,
    provider: &'a str,
) -> impl Iterator<Item = &'a DependencyContractEdge> {
    graph.edges.iter().filter(move |edge| edge.from == provider)
}

fn edge_requirements_are_satisfied(
    edge: &DependencyContractEdge,
    handoff: &HandoffRevision,
) -> bool {
    edge.required_contracts
        .iter()
        .all(|required| requirement_is_satisfied(required, handoff))
}

fn incoming_edges_are_satisfied(
    graph: &DependencyContractGraph,
    consumer: &str,
    next: &HandoffRevision,
    authoritative_handoffs: &BTreeMap<String, HandoffRevision>,
) -> bool {
    graph
        .edges
        .iter()
        .filter(|edge| edge.to == consumer)
        .all(|edge| {
            let handoff = if edge.from == next.logical_work_item_id {
                next
            } else if let Some(handoff) = authoritative_handoffs.get(&edge.from) {
                handoff
            } else {
                return false;
            };
            edge_requirements_are_satisfied(edge, handoff)
        })
}

fn requirement_is_satisfied(
    required: &RequiredDependencyContract,
    handoff: &HandoffRevision,
) -> bool {
    let Some(provided) = handoff.provided_capabilities.get(&required.contract_id) else {
        return false;
    };
    let provided = provided.iter().collect::<BTreeSet<_>>();
    let required_capabilities = required
        .required_capabilities
        .iter()
        .collect::<BTreeSet<_>>();
    match required.compatibility_policy {
        ContractCompatibilityPolicy::RequireAll => required_capabilities.is_subset(&provided),
        ContractCompatibilityPolicy::RequireAny => {
            required_capabilities.is_empty() || !required_capabilities.is_disjoint(&provided)
        }
    }
}

fn unique_runtime_unit<'a>(
    units: &'a [crate::product::coding_models::CodingExecutionUnit],
    logical_id: &str,
) -> Result<&'a crate::product::coding_models::CodingExecutionUnit, CodingWorkspaceEngineError> {
    let mut matches = units
        .iter()
        .filter(|unit| unit.logical_work_item_id == logical_id);
    let unit = matches.next().ok_or_else(|| {
        CodingWorkspaceEngineError::ProviderStream(format!(
            "runtime_handoff_unit_missing: {logical_id}"
        ))
    })?;
    if matches.next().is_some() {
        return Err(CodingWorkspaceEngineError::ProviderStream(format!(
            "runtime_handoff_unit_ambiguous: {logical_id}"
        )));
    }
    Ok(unit)
}

fn sort_runtime_result(result: &mut RuntimeHandoffImpactResult) {
    for values in [
        &mut result.resumed_units,
        &mut result.revalidation_units,
        &mut result.newly_stale_units,
        &mut result.conditional_units_released,
    ] {
        values.sort();
        values.dedup();
    }
}
