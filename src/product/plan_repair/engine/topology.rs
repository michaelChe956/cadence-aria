use std::collections::{BTreeMap, BTreeSet};

use crate::product::models::{
    AmendmentResumeMode, AmendmentResumeTarget, LogicalWorkItem, PlanAmendmentManifest,
    PlanRevisionReason, PlanValidationReportArtifact, WorkItemRevisionReplacement,
};
use crate::product::work_item_contract::{
    build_dependency_contract_graph, validate_dependency_contract_graph,
};
use crate::product::work_item_projection::{
    CompiledWorkItemProjections, PlanProjectionCompileInput, PlanProjectionCompiler,
    WorkItemProjectionCompiler,
};
use crate::product::workspace_engine::{
    compile_plan_projection_bundle, compile_work_item_revision,
};

use super::{
    PlanRepairEngine, PreparedPlanAmendment, dependency_graph_changes, invalid_target,
    validate_candidate_draft,
};
use crate::product::plan_repair::{
    ContractImpactReport, PlanRepairError, PlanRepairRequest, RepairTargetKind,
    SubgraphReplanReadiness, build_plan_repair_candidate_package,
    canonical_work_item_projection_bundles, compute_contract_delta,
};

impl PlanRepairEngine {
    pub(super) fn prepare_topology_amendment(
        &self,
        request: &PlanRepairRequest,
    ) -> Result<PreparedPlanAmendment, PlanRepairError> {
        let (plan, base_revision, amendment_id) = self.validate_prepare_identity(request)?;
        if request.repair_target.kind != RepairTargetKind::Subgraph {
            return Err(invalid_target(
                "topology amendment requires a subgraph repair target",
            ));
        }
        let subgraph_request = self
            .subgraph_replan_request
            .as_ref()
            .ok_or_else(|| invalid_target("subgraph amendment requires a typed replan request"))?;
        if subgraph_request.repair_request_id != request.id
            || subgraph_request.plan_id != request.plan_id
            || subgraph_request.base_plan_revision_id != request.base_plan_revision_id
        {
            return Err(invalid_target(
                "subgraph replan request is not bound to the prepared amendment",
            ));
        }
        let subgraph_replan = self.replan_subgraph(subgraph_request)?;
        if subgraph_replan.readiness != SubgraphReplanReadiness::PublicationReady
            || subgraph_replan.dependency_graph_revision.is_none()
        {
            return Err(invalid_target("subgraph replan is not publication ready"));
        }
        let affected = subgraph_replan
            .affected_logical_work_items
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let targets = request
            .repair_target
            .logical_work_item_ids
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        if affected != targets || targets.len() != request.repair_target.logical_work_item_ids.len()
        {
            return Err(invalid_target(
                "subgraph affected scope must match the persisted repair target",
            ));
        }
        let replacement_ids = subgraph_replan
            .replacement_mapping
            .values()
            .flatten()
            .cloned()
            .collect::<BTreeSet<_>>();
        if replacement_ids.len() != self.candidate_drafts.len()
            || self
                .candidate_drafts
                .keys()
                .any(|id| !replacement_ids.contains(id))
        {
            return Err(invalid_target(
                "subgraph replacements and candidate draft identities must match exactly",
            ));
        }

        let previous_revision_ids = request
            .repair_target
            .work_item_revision_ids
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        if previous_revision_ids.len() != targets.len() {
            return Err(invalid_target(
                "subgraph repair target must bind one active revision per affected work item",
            ));
        }
        let replacement_ids_ordered = replacement_ids.iter().cloned().collect::<Vec<_>>();
        let publication_ids = self
            .store
            .allocate_plan_amendment_publication_ids(
                &plan,
                &amendment_id,
                base_revision.revision_no + 1,
                &replacement_ids_ordered,
            )
            .map_err(PlanRepairError::Store)?;

        let mut previous_by_logical = BTreeMap::new();
        for old_id in &affected {
            let previous_id = base_revision
                .work_item_bindings
                .get(old_id)
                .ok_or_else(|| {
                    invalid_target("subgraph affected work item is absent from active bindings")
                })?;
            if !previous_revision_ids.contains(previous_id) {
                return Err(invalid_target(
                    "subgraph previous revision does not match active plan binding",
                ));
            }
            let previous = self
                .store
                .get_work_item_revision(&plan, old_id, previous_id)
                .map_err(PlanRepairError::Store)?;
            previous_by_logical.insert(old_id.clone(), previous);
        }

        let mut compiled_items = base_revision
            .work_item_bindings
            .iter()
            .filter(|(logical_id, _)| !affected.contains(*logical_id))
            .map(|(logical_id, revision_id)| {
                self.store
                    .get_work_item_revision(&plan, logical_id, revision_id)
                    .map_err(PlanRepairError::Store)
                    .and_then(|revision| self.load_compiled_item(&plan, &revision))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut compiled_replacements = BTreeMap::new();
        let mut logical_work_items = Vec::new();
        let mut draft_revisions = Vec::new();
        let mut revised_items = Vec::new();
        let mut verification_plan_revisions = Vec::new();
        for logical_id in &replacement_ids_ordered {
            let draft = self
                .candidate_drafts
                .get(logical_id)
                .expect("replacement identities were validated");
            validate_candidate_draft(
                draft,
                request,
                &PlanRevisionReason::SubgraphReplan,
                logical_id,
            )?;
            let logical = match base_revision.work_item_bindings.get(logical_id) {
                Some(previous_revision_id) => {
                    let previous = self
                        .store
                        .get_work_item_revision(&plan, logical_id, previous_revision_id)
                        .map_err(PlanRepairError::Store)?;
                    if draft.supersedes.as_deref()
                        != Some(previous.source_draft_revision_id.as_str())
                    {
                        return Err(invalid_target(
                            "existing replacement draft does not supersede its active draft",
                        ));
                    }
                    self.store
                        .get_logical_work_item(&plan, logical_id)
                        .map_err(PlanRepairError::Store)?
                }
                None => {
                    if draft.revision_no != 1 || draft.supersedes.is_some() {
                        return Err(invalid_target(
                            "new replacement logical work item must start at draft revision one",
                        ));
                    }
                    LogicalWorkItem {
                        id: logical_id.clone(),
                        plan_id: plan.id.clone(),
                        title: draft.canonical_contract_candidate.identity.title.clone(),
                        active_revision_id: None,
                        created_at: self.created_at.clone(),
                        updated_at: self.created_at.clone(),
                    }
                }
            };
            let ids = publication_ids
                .work_items
                .get(logical_id)
                .expect("store allocated every replacement identity");
            let compiled = compile_work_item_revision(
                draft,
                &WorkItemProjectionCompiler,
                ids,
                &self.created_at,
            )
            .map_err(|error| invalid_target(&error.to_string()))?;
            logical_work_items.push(logical);
            draft_revisions.push(compiled.draft_revision.clone());
            revised_items.push(compiled.work_item_revision.clone());
            verification_plan_revisions.push(compiled.verification_plan_revision.clone());
            compiled_replacements.insert(logical_id.clone(), compiled.clone());
            compiled_items.push(compiled);
        }
        compiled_items.sort_by(|left, right| {
            left.work_item_revision
                .logical_work_item_id
                .cmp(&right.work_item_revision.logical_work_item_id)
        });
        logical_work_items.sort_by(|left, right| left.id.cmp(&right.id));
        draft_revisions
            .sort_by(|left, right| left.logical_work_item_id.cmp(&right.logical_work_item_id));
        revised_items
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
        let dependency_graph_revision = subgraph_replan
            .dependency_graph_revision
            .clone()
            .expect("publication readiness was checked");
        if dependency_graph_revision.id != publication_ids.dependency_graph_revision_id
            || dependency_graph_revision.edges != dependency_graph.edges
        {
            return Err(invalid_target(
                "prepared subgraph graph differs from the authoritative typed graph",
            ));
        }

        let mut next_bindings = base_revision.work_item_bindings.clone();
        for logical_id in &affected {
            next_bindings.remove(logical_id);
        }
        for compiled in compiled_replacements.values() {
            next_bindings.insert(
                compiled.work_item_revision.logical_work_item_id.clone(),
                compiled.work_item_revision.id.clone(),
            );
        }
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
        let mut source_refs = plan.story_spec_refs.clone();
        source_refs.extend(plan.design_spec_refs.iter().cloned());
        let plan_projection_bundle = compile_plan_projection_bundle(
            &plan_revision_id,
            &dependency_graph_revision.id,
            &publication_ids.plan_projection_bundle_id,
            &self.created_at,
            PlanProjectionCompileInput {
                plan_id: &plan.id,
                goal: "Publish a validated Subgraph Plan Repair candidate",
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
        let validation_report = PlanValidationReportArtifact {
            id: publication_ids.validation_report_id.clone(),
            plan_id: plan.id.clone(),
            plan_revision_id: plan_revision_id.clone(),
            plan_projection_bundle_id: plan_projection_bundle.id.clone(),
            contract_validation,
            projection_validation:
                crate::product::work_item_projection::ProjectionValidationReport {
                    findings: Vec::new(),
                },
            created_at: self.created_at.clone(),
        };
        let next_plan_revision = crate::product::models::WorkItemPlanRevision {
            id: plan_revision_id.clone(),
            plan_id: plan.id.clone(),
            revision_no: base_revision.revision_no + 1,
            supersedes: Some(base_revision.id.clone()),
            reason: PlanRevisionReason::SubgraphReplan,
            work_item_bindings: next_bindings,
            dependency_graph_revision_id: dependency_graph_revision.id.clone(),
            validation_report_ref: validation_report.id.clone(),
            plan_projection_bundle_id: plan_projection_bundle.id.clone(),
            created_at: self.created_at.clone(),
        };

        let mut contract_deltas = Vec::new();
        let mut revised_work_items = BTreeMap::new();
        let mut superseded_revisions = Vec::new();
        for old_id in &affected {
            let previous = previous_by_logical
                .get(old_id)
                .expect("affected previous revision was loaded");
            superseded_revisions.push(previous.id.clone());
            let replacements = subgraph_replan
                .replacement_mapping
                .get(old_id)
                .expect("complete mapping covers every affected identity");
            for replacement_id in replacements {
                let next = &compiled_replacements
                    .get(replacement_id)
                    .expect("replacement was compiled")
                    .work_item_revision;
                contract_deltas.push(compute_contract_delta(
                    &previous.id,
                    &previous.canonical_contract,
                    &next.id,
                    &next.canonical_contract,
                ));
            }
            if let [replacement_id] = replacements.as_slice() {
                let next = &compiled_replacements
                    .get(replacement_id)
                    .expect("replacement was compiled")
                    .work_item_revision;
                let kind = contract_deltas
                    .iter()
                    .rev()
                    .find(|delta| delta.previous_revision_id == previous.id)
                    .expect("single replacement delta exists")
                    .kind
                    .clone();
                revised_work_items.insert(
                    old_id.clone(),
                    WorkItemRevisionReplacement {
                        previous_revision_id: previous.id.clone(),
                        next_revision_id: next.id.clone(),
                        delta_kind: kind,
                    },
                );
            }
        }
        superseded_revisions.sort();
        contract_deltas.sort_by(|left, right| {
            (&left.logical_work_item_id, &left.next_revision_id)
                .cmp(&(&right.logical_work_item_id, &right.next_revision_id))
        });
        let replacement_scope = replacement_ids.iter().cloned().collect::<Vec<_>>();
        let impact_report = ContractImpactReport {
            unaffected: dependency_graph
                .contracts
                .keys()
                .filter(|id| !replacement_ids.contains(*id))
                .cloned()
                .collect(),
            direct_revalidation: Vec::new(),
            direct_stale: replacement_scope.clone(),
            conditional_downstream: Vec::new(),
            explanation_paths: Vec::new(),
        };
        let resume_target = AmendmentResumeTarget {
            logical_work_item_id: replacement_scope
                .first()
                .cloned()
                .ok_or_else(|| invalid_target("subgraph replacement scope is empty"))?,
            mode: AmendmentResumeMode::Reexecute,
        };
        let base_graph = self
            .store
            .get_dependency_graph_revision(&plan, &base_revision.dependency_graph_revision_id)
            .map_err(PlanRepairError::Store)?;
        let manifest = PlanAmendmentManifest {
            id: amendment_id,
            repair_request_id: request.id.clone(),
            previous_plan_revision_id: base_revision.id.clone(),
            new_plan_revision_id: next_plan_revision.id.clone(),
            revised_work_items,
            superseded_revisions,
            dependency_graph_changes: dependency_graph_changes(
                &base_graph.edges,
                &dependency_graph_revision.edges,
            ),
            contract_deltas: contract_deltas.clone(),
            unaffected_units: impact_report.unaffected.clone(),
            revalidation_required_units: impact_report.direct_revalidation.clone(),
            stale_units: impact_report.direct_stale.clone(),
            replacement_units: subgraph_replan.replacement_mapping.clone(),
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
            logical_work_items,
            dependency_graph_revision,
            plan_projection_bundle,
            validation_report,
            contract_deltas,
            impact_report,
            manifest,
            candidate_package,
            subgraph_replan: Some(subgraph_replan),
        })
    }

    pub(super) fn validate_prepared_subgraph(
        &self,
        prepared: &PreparedPlanAmendment,
    ) -> Result<(), PlanRepairError> {
        if prepared.next_plan_revision.reason != PlanRevisionReason::SubgraphReplan {
            if prepared.subgraph_replan.is_some() {
                return Err(invalid_target(
                    "non-subgraph amendment cannot carry subgraph publication state",
                ));
            }
            return Ok(());
        }
        let result = prepared
            .subgraph_replan
            .as_ref()
            .ok_or_else(|| invalid_target("subgraph amendment is missing publication readiness"))?;
        if result.readiness != SubgraphReplanReadiness::PublicationReady
            || result.dependency_graph_revision.as_ref()
                != Some(&prepared.dependency_graph_revision)
            || result.base_plan_revision_id != prepared.base_plan_revision_id
            || result.replacement_mapping != prepared.manifest.replacement_units
            || !prepared.validation_report.contract_validation.is_valid()
        {
            return Err(invalid_target(
                "subgraph amendment is not publication ready",
            ));
        }
        Ok(())
    }
}
