use std::collections::{BTreeMap, BTreeSet};

use chrono::Utc;

use crate::product::coding_models::{
    CodingExecutionAttempt, CodingExecutionUnit, CodingExecutionUnitStatus, CodingUnitRun,
    CodingUnitRunStatus,
};
use crate::product::json_store::{ProductStoreError, read_json, write_json};
use crate::product::models::{AmendmentResumeMode, PlanAmendmentManifest};
use crate::product::work_item_projection::renderer_for;
use crate::product::work_item_revision_store::WorkItemRevisionStore;

use super::unit_run::{
    amendment_unit_run_id, same_materialization_identity, unique_unit, unique_unit_mut,
};

struct AmendmentRunContext {
    dependencies: BTreeMap<String, Vec<String>>,
    affected: BTreeSet<String>,
    stale: BTreeSet<String>,
    revalidation: BTreeSet<String>,
    replacement_targets: BTreeSet<String>,
    replacement_sources: BTreeSet<String>,
    plan: crate::product::models::WorkItemPlanLineage,
    revision: crate::product::models::WorkItemPlanRevision,
}

impl super::CodingAttemptStore {
    pub fn materialize_unit_runs_from_manifest(
        &self,
        attempt: &CodingExecutionAttempt,
        manifest: &PlanAmendmentManifest,
    ) -> Result<Vec<CodingUnitRun>, ProductStoreError> {
        let current = self.validate_attempt_lineage(attempt)?;
        let revision_store = WorkItemRevisionStore::new(self.paths());
        let context = self.amendment_run_context(&current, manifest, &revision_store, false)?;
        let coder_renderer = renderer_for(&current.provider_config_snapshot.author);
        let reviewer_provider = current
            .provider_config_snapshot
            .reviewer
            .as_ref()
            .unwrap_or(&current.provider_config_snapshot.author);
        let reviewer_renderer = renderer_for(reviewer_provider);
        let mut units =
            self.list_coding_units(&current.project_id, &current.issue_id, &current.id)?;

        for logical_id in &context.affected {
            let Some(unit) = unique_optional_unit(&units, logical_id)? else {
                if context.replacement_targets.contains(logical_id) {
                    continue;
                }
                return Err(identity_mismatch(
                    "coding_amendment_execution_unit",
                    logical_id,
                ));
            };
            let expected = expected_run(
                &current,
                manifest,
                &context,
                &revision_store,
                unit,
                &self.list_coding_unit_runs(&current, &unit.id)?,
                coder_renderer.renderer_version(),
                reviewer_renderer.renderer_version(),
                false,
            )?;
            if let Some((_, existing)) = self.find_unit_run_by_id(&current, &expected.id)? {
                validate_materialized_unit(unit, manifest, &context, false)?;
                if !same_materialization_identity(&existing, &expected) {
                    return Err(identity_mismatch("coding_amendment_unit_run", &expected.id));
                }
            }
        }

        for source_id in &context.replacement_sources {
            let unit = unique_unit_mut(&mut units, source_id)?;
            if unit.status != CodingExecutionUnitStatus::Superseded {
                unit.status = CodingExecutionUnitStatus::Superseded;
                unit.completed_at = Some(Utc::now().to_rfc3339());
                unit.updated_at = Utc::now().to_rfc3339();
                write_json(
                    &self.coding_unit_path(
                        &current.project_id,
                        &current.issue_id,
                        &current.id,
                        &unit.id,
                    ),
                    unit,
                )?;
            }
            for mut run in self
                .list_coding_unit_runs(&current, &unit.id)?
                .into_iter()
                .filter(|run| run.status.is_active())
            {
                run.status = CodingUnitRunStatus::Superseded;
                run.updated_at = Utc::now().to_rfc3339();
                write_json(
                    &self.coding_unit_run_path(
                        &current.project_id,
                        &current.issue_id,
                        &current.id,
                        &unit.id,
                        &run.id,
                    ),
                    &run,
                )?;
            }
        }

        let next_order_index = units
            .iter()
            .map(|unit| unit.order_index)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        for (offset, logical_id) in context.affected.iter().enumerate() {
            if units
                .iter()
                .all(|unit| &unit.logical_work_item_id != logical_id)
            {
                let revision_id = context
                    .revision
                    .work_item_bindings
                    .get(logical_id)
                    .ok_or_else(|| {
                        identity_mismatch("coding_amendment_execution_unit", logical_id)
                    })?;
                let created = self.create_coding_unit(super::CreateCodingExecutionUnitInput {
                    attempt_id: current.id.clone(),
                    project_id: current.project_id.clone(),
                    issue_id: current.issue_id.clone(),
                    plan_id: context.plan.id.clone(),
                    logical_work_item_id: logical_id.clone(),
                    work_item_revision_id: revision_id.clone(),
                    dependency_logical_work_item_ids: context
                        .dependencies
                        .get(logical_id)
                        .cloned()
                        .unwrap_or_default(),
                    order_index: next_order_index.saturating_add(offset as u32),
                    status: CodingExecutionUnitStatus::Pending,
                })?;
                units.push(created);
            }
        }

        let mut materialized = Vec::new();
        for logical_id in &context.affected {
            let unit = unique_unit_mut(&mut units, logical_id)?;
            let runs = self.list_coding_unit_runs(&current, &unit.id)?;
            let expected = expected_run(
                &current,
                manifest,
                &context,
                &revision_store,
                unit,
                &runs,
                coder_renderer.renderer_version(),
                reviewer_renderer.renderer_version(),
                false,
            )?;
            if let Some((_, existing)) = self.find_unit_run_by_id(&current, &expected.id)? {
                materialized.push(existing);
                continue;
            }

            materialize_unit(unit, manifest, &context);
            write_json(
                &self.coding_unit_path(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                    &unit.id,
                ),
                unit,
            )?;
            for mut run in runs.into_iter().filter(|run| run.status.is_active()) {
                run.status = CodingUnitRunStatus::Superseded;
                run.updated_at = Utc::now().to_rfc3339();
                write_json(
                    &self.coding_unit_run_path(
                        &current.project_id,
                        &current.issue_id,
                        &current.id,
                        &unit.id,
                        &run.id,
                    ),
                    &run,
                )?;
            }
            materialized.push(self.load_or_create_coding_unit_run(&current, &expected)?);
        }
        Ok(materialized)
    }

    pub fn validate_materialized_unit_runs_from_manifest(
        &self,
        attempt: &CodingExecutionAttempt,
        manifest: &PlanAmendmentManifest,
        resume_target_written: bool,
    ) -> Result<Vec<CodingUnitRun>, ProductStoreError> {
        let current = self.validate_attempt_lineage(attempt)?;
        let revision_store = WorkItemRevisionStore::new(self.paths());
        let context =
            self.amendment_run_context(&current, manifest, &revision_store, resume_target_written)?;
        let coder_renderer = renderer_for(&current.provider_config_snapshot.author);
        let reviewer_provider = current
            .provider_config_snapshot
            .reviewer
            .as_ref()
            .unwrap_or(&current.provider_config_snapshot.author);
        let reviewer_renderer = renderer_for(reviewer_provider);
        let units = self.list_coding_units(&current.project_id, &current.issue_id, &current.id)?;
        if resume_target_written {
            let target = unique_unit(&units, &manifest.resume_target.logical_work_item_id)?;
            if current.active_unit_id.as_deref() != Some(target.id.as_str())
                || current.current_work_item_id.as_deref()
                    != Some(target.logical_work_item_id.as_str())
            {
                return Err(identity_mismatch(
                    "coding_amendment_resume_target",
                    &manifest.resume_target.logical_work_item_id,
                ));
            }
        }
        for source_id in &context.replacement_sources {
            let source = unique_unit(&units, source_id)?;
            if source.status != CodingExecutionUnitStatus::Superseded
                || self
                    .list_coding_unit_runs(&current, &source.id)?
                    .iter()
                    .any(|run| run.status.is_active())
            {
                return Err(identity_mismatch(
                    "coding_amendment_replacement_source",
                    source_id,
                ));
            }
        }
        let mut validated = Vec::new();
        for logical_id in &context.affected {
            let unit = unique_unit(&units, logical_id)?;
            validate_materialized_unit(unit, manifest, &context, resume_target_written)?;
            let runs = self.list_coding_unit_runs(&current, &unit.id)?;
            let expected = expected_run(
                &current,
                manifest,
                &context,
                &revision_store,
                unit,
                &runs,
                coder_renderer.renderer_version(),
                reviewer_renderer.renderer_version(),
                resume_target_written,
            )?;
            let existing = self
                .find_unit_run_by_id(&current, &expected.id)?
                .map(|(_, run)| run)
                .ok_or_else(|| identity_mismatch("coding_amendment_unit_run", &expected.id))?;
            if !same_materialization_identity(&existing, &expected) {
                return Err(identity_mismatch("coding_amendment_unit_run", &expected.id));
            }
            validated.push(existing);
        }
        Ok(validated)
    }

    pub fn set_materialized_amendment_unit_run_status(
        &self,
        attempt: &CodingExecutionAttempt,
        manifest: &PlanAmendmentManifest,
        logical_work_item_id: &str,
        status: CodingUnitRunStatus,
    ) -> Result<CodingUnitRun, ProductStoreError> {
        let current = self.validate_attempt_lineage(attempt)?;
        let units = self.list_coding_units(&current.project_id, &current.issue_id, &current.id)?;
        let unit = unique_unit(&units, logical_work_item_id)?;
        let run_id = amendment_unit_run_id(&unit.id, &manifest.id);
        let path = self.coding_unit_run_path(
            &current.project_id,
            &current.issue_id,
            &current.id,
            &unit.id,
            &run_id,
        );
        let mut run: CodingUnitRun = read_json(&path)?;
        let revision_store = WorkItemRevisionStore::new(self.paths());
        let context = self.amendment_run_context(&current, manifest, &revision_store, false)?;
        let coder_renderer = renderer_for(&current.provider_config_snapshot.author);
        let reviewer_provider = current
            .provider_config_snapshot
            .reviewer
            .as_ref()
            .unwrap_or(&current.provider_config_snapshot.author);
        let reviewer_renderer = renderer_for(reviewer_provider);
        let mut expected = expected_run(
            &current,
            manifest,
            &context,
            &revision_store,
            unit,
            &self.list_coding_unit_runs(&current, &unit.id)?,
            coder_renderer.renderer_version(),
            reviewer_renderer.renderer_version(),
            false,
        )?;
        expected.status = run.status.clone();
        if !same_materialization_identity(&run, &expected) {
            return Err(identity_mismatch("coding_amendment_unit_run", &run_id));
        }
        if run.status == status {
            return Ok(run);
        }
        run.status = status;
        run.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &run)?;
        Ok(run)
    }

    fn amendment_run_context(
        &self,
        current: &CodingExecutionAttempt,
        manifest: &PlanAmendmentManifest,
        revision_store: &WorkItemRevisionStore,
        allow_released_lock: bool,
    ) -> Result<AmendmentRunContext, ProductStoreError> {
        let binding = self.get_plan_binding(current)?;
        if binding.bound_plan_revision_id != manifest.new_plan_revision_id
            || binding.applied_amendment_ids.last() != Some(&manifest.id)
        {
            return Err(identity_mismatch(
                "coding_amendment_unit_run_binding",
                &manifest.id,
            ));
        }
        let plan = revision_store.get_plan_lineage(
            &current.project_id,
            &current.issue_id,
            &binding.plan_id,
        )?;
        let amendment_matches = plan.active_amendment_id.as_deref() == Some(manifest.id.as_str())
            || (allow_released_lock && plan.active_amendment_id.is_none());
        if !amendment_matches
            || plan.active_revision_id.as_deref() != Some(manifest.new_plan_revision_id.as_str())
        {
            return Err(identity_mismatch(
                "coding_amendment_unit_run_binding",
                &manifest.id,
            ));
        }
        let revision = revision_store.get_plan_revision(
            &current.project_id,
            &current.issue_id,
            &plan.id,
            &manifest.new_plan_revision_id,
        )?;
        let graph = revision_store
            .get_dependency_graph_revision(&plan, &revision.dependency_graph_revision_id)?;
        let mut dependencies = revision
            .work_item_bindings
            .keys()
            .cloned()
            .map(|id| (id, Vec::new()))
            .collect::<BTreeMap<_, _>>();
        for edge in graph.edges {
            dependencies
                .get_mut(&edge.to)
                .ok_or_else(|| identity_mismatch("coding_amendment_dependency_graph", &edge.to))?
                .push(edge.from);
        }
        for values in dependencies.values_mut() {
            values.sort();
            values.dedup();
        }
        let revised = manifest
            .revised_work_items
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        let stale = manifest
            .stale_units
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let revalidation = manifest
            .revalidation_required_units
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let replacement_targets = manifest
            .replacement_units
            .values()
            .flatten()
            .cloned()
            .collect::<BTreeSet<_>>();
        let replacement_sources = manifest
            .replacement_units
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        let affected = revised
            .iter()
            .chain(stale.iter())
            .chain(revalidation.iter())
            .chain(replacement_targets.iter())
            .cloned()
            .collect::<BTreeSet<_>>();
        if !affected.is_disjoint(&replacement_sources) {
            return Err(identity_mismatch(
                "coding_amendment_replacement_source",
                &manifest.id,
            ));
        }
        Ok(AmendmentRunContext {
            dependencies,
            affected,
            stale,
            revalidation,
            replacement_targets,
            replacement_sources,
            plan,
            revision,
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn expected_run(
    current: &CodingExecutionAttempt,
    manifest: &PlanAmendmentManifest,
    context: &AmendmentRunContext,
    revision_store: &WorkItemRevisionStore,
    unit: &CodingExecutionUnit,
    runs: &[CodingUnitRun],
    coder_renderer_version: &str,
    reviewer_renderer_version: &str,
    resume_target_written: bool,
) -> Result<CodingUnitRun, ProductStoreError> {
    let logical_id = &unit.logical_work_item_id;
    let revision_id = context
        .revision
        .work_item_bindings
        .get(logical_id)
        .ok_or_else(|| identity_mismatch("coding_amendment_execution_unit", logical_id))?
        .clone();
    if let Some(replacement) = manifest.revised_work_items.get(logical_id)
        && replacement.next_revision_id != revision_id
    {
        return Err(identity_mismatch(
            "coding_amendment_execution_unit",
            logical_id,
        ));
    }
    let run_id = amendment_unit_run_id(&unit.id, &manifest.id);
    let predecessors = runs
        .iter()
        .filter(|run| run.id != run_id)
        .collect::<Vec<_>>();
    let execution_no = predecessors
        .iter()
        .map(|run| run.execution_no)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let plan_repair_count = predecessors
        .iter()
        .map(|run| run.plan_repair_count)
        .max()
        .unwrap_or(0);
    let revision =
        revision_store.get_work_item_revision(&context.plan, logical_id, &revision_id)?;
    let bundle = revision_store
        .get_work_item_projection_bundle(&context.plan, &revision.work_item_projection_bundle_id)?;
    let mut status = initial_run_status(context, logical_id);
    if resume_target_written && logical_id == &manifest.resume_target.logical_work_item_id {
        status = resume_run_status(&manifest.resume_target.mode);
    }
    Ok(CodingUnitRun {
        id: run_id,
        unit_id: unit.id.clone(),
        execution_no,
        work_item_revision_id: revision_id,
        resolved_handoff_revision_ids: Vec::new(),
        canonical_contract_hash: revision.canonical_contract_hash,
        projection_bundle_id: bundle.id,
        projection_compiler_version: bundle.compiler_version,
        coder_provider_renderer_version: coder_renderer_version.to_string(),
        reviewer_provider_renderer_version: reviewer_renderer_version.to_string(),
        internal_reviewer_provider_renderer_version: None,
        coder_projection_hash: bundle.coder_projection_hash,
        reviewer_projection_hash: bundle.reviewer_projection_hash,
        coder_execution_context_hash: None,
        reviewer_execution_context_hash: None,
        internal_reviewer_execution_context_hash: None,
        status,
        unit_rework_count: 0,
        verification_retry_count: 0,
        operational_retry_count: 0,
        plan_repair_count,
        start_commit: current.head_commit.clone(),
        completion_commit: None,
        created_at: String::new(),
        updated_at: String::new(),
    })
}

fn validate_materialized_unit(
    unit: &CodingExecutionUnit,
    manifest: &PlanAmendmentManifest,
    context: &AmendmentRunContext,
    resume_target_written: bool,
) -> Result<(), ProductStoreError> {
    let logical_id = &unit.logical_work_item_id;
    let expected_revision = context
        .revision
        .work_item_bindings
        .get(logical_id)
        .ok_or_else(|| identity_mismatch("coding_amendment_execution_unit", logical_id))?;
    let mut expected_status = CodingExecutionUnitStatus::Pending;
    let mut expected_started = None;
    let mut expected_summary = format!("Plan Amendment {} materialized", manifest.id);
    if resume_target_written && logical_id == &manifest.resume_target.logical_work_item_id {
        expected_status = resume_unit_status(&manifest.resume_target.mode);
        expected_summary = format!("Resume after Plan Amendment {}", manifest.id);
        if expected_status == CodingExecutionUnitStatus::Running {
            expected_started = unit.started_at.as_ref();
        }
    }
    if &unit.work_item_revision_id != expected_revision
        || unit.dependency_logical_work_item_ids
            != context
                .dependencies
                .get(logical_id)
                .cloned()
                .unwrap_or_default()
        || unit.status != expected_status
        || (expected_status != CodingExecutionUnitStatus::Running && unit.started_at.is_some())
        || (expected_status == CodingExecutionUnitStatus::Running && expected_started.is_none())
        || unit.completed_at.is_some()
        || unit.summary.as_deref() != Some(expected_summary.as_str())
    {
        return Err(identity_mismatch(
            "coding_amendment_execution_unit",
            logical_id,
        ));
    }
    Ok(())
}

fn materialize_unit(
    unit: &mut CodingExecutionUnit,
    manifest: &PlanAmendmentManifest,
    context: &AmendmentRunContext,
) {
    unit.work_item_revision_id =
        context.revision.work_item_bindings[&unit.logical_work_item_id].clone();
    unit.dependency_logical_work_item_ids = context
        .dependencies
        .get(&unit.logical_work_item_id)
        .cloned()
        .unwrap_or_default();
    unit.status = CodingExecutionUnitStatus::Pending;
    unit.started_at = None;
    unit.completed_at = None;
    unit.summary = Some(format!("Plan Amendment {} materialized", manifest.id));
    unit.updated_at = Utc::now().to_rfc3339();
}

fn initial_run_status(context: &AmendmentRunContext, logical_id: &str) -> CodingUnitRunStatus {
    if context.stale.contains(logical_id) {
        CodingUnitRunStatus::Stale
    } else if context.revalidation.contains(logical_id) {
        CodingUnitRunStatus::NeedsRevalidation
    } else {
        CodingUnitRunStatus::AwaitingAmendment
    }
}

fn resume_unit_status(mode: &AmendmentResumeMode) -> CodingExecutionUnitStatus {
    match mode {
        AmendmentResumeMode::Reexecute => CodingExecutionUnitStatus::Running,
        AmendmentResumeMode::Revalidate => CodingExecutionUnitStatus::NeedsRevalidation,
        AmendmentResumeMode::AwaitHandoff => CodingExecutionUnitStatus::AwaitingAmendment,
    }
}

fn resume_run_status(mode: &AmendmentResumeMode) -> CodingUnitRunStatus {
    match mode {
        AmendmentResumeMode::Reexecute => CodingUnitRunStatus::Running,
        AmendmentResumeMode::Revalidate => CodingUnitRunStatus::NeedsRevalidation,
        AmendmentResumeMode::AwaitHandoff => CodingUnitRunStatus::AwaitingAmendment,
    }
}

fn unique_optional_unit<'a>(
    units: &'a [CodingExecutionUnit],
    logical_work_item_id: &str,
) -> Result<Option<&'a CodingExecutionUnit>, ProductStoreError> {
    let matches = units
        .iter()
        .filter(|unit| unit.logical_work_item_id == logical_work_item_id)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [unit] => Ok(Some(*unit)),
        [] => Ok(None),
        _ => Err(ProductStoreError::Ambiguous {
            kind: "coding_execution_unit",
            id: logical_work_item_id.to_string(),
        }),
    }
}

fn identity_mismatch(kind: &'static str, id: &str) -> ProductStoreError {
    ProductStoreError::IdentityMismatch {
        kind,
        id: id.to_string(),
    }
}
