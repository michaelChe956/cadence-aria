use std::collections::{BTreeMap, BTreeSet};

use chrono::Utc;

use crate::product::coding_models::{
    CodingExecutionAttempt, CodingExecutionUnit, CodingProviderRole, CodingUnitRun,
    CodingUnitRunStatus,
};
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::PlanAmendmentManifest;
use crate::product::work_item_projection::{RenderedExecutionContext, renderer_for};
use crate::product::work_item_revision_store::WorkItemRevisionStore;

use super::locking::with_exclusive_lock;

impl super::CodingAttemptStore {
    pub fn materialize_unit_runs_from_manifest(
        &self,
        attempt: &CodingExecutionAttempt,
        manifest: &PlanAmendmentManifest,
    ) -> Result<Vec<CodingUnitRun>, ProductStoreError> {
        let current = self.validate_attempt_lineage(attempt)?;
        let binding = self.get_plan_binding(&current)?;
        if binding.bound_plan_revision_id != manifest.new_plan_revision_id
            || binding.applied_amendment_ids.last() != Some(&manifest.id)
        {
            return Err(identity_mismatch(
                "coding_amendment_unit_run_binding",
                &manifest.id,
            ));
        }
        let revision_store = WorkItemRevisionStore::new(self.paths());
        let plan = revision_store.get_plan_lineage(
            &current.project_id,
            &current.issue_id,
            &binding.plan_id,
        )?;
        if plan.active_amendment_id.as_deref() != Some(manifest.id.as_str())
            || plan.active_revision_id.as_deref() != Some(manifest.new_plan_revision_id.as_str())
        {
            return Err(identity_mismatch(
                "coding_amendment_unit_run_binding",
                &manifest.id,
            ));
        }
        let plan_revision = revision_store.get_plan_revision(
            &current.project_id,
            &current.issue_id,
            &plan.id,
            &manifest.new_plan_revision_id,
        )?;
        let graph = revision_store
            .get_dependency_graph_revision(&plan, &plan_revision.dependency_graph_revision_id)?;
        let mut dependencies = plan_revision
            .work_item_bindings
            .keys()
            .cloned()
            .map(|id| (id, Vec::new()))
            .collect::<BTreeMap<_, _>>();
        for edge in graph.edges {
            let target = dependencies
                .get_mut(&edge.to)
                .ok_or_else(|| identity_mismatch("coding_amendment_dependency_graph", &edge.to))?;
            target.push(edge.from);
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
        let affected = revised
            .iter()
            .chain(stale.iter())
            .chain(revalidation.iter())
            .chain(replacement_targets.iter())
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut units =
            self.list_coding_units(&current.project_id, &current.issue_id, &current.id)?;
        for superseded_id in manifest.replacement_units.keys() {
            let unit = unique_unit_mut(&mut units, superseded_id)?;
            unit.status = crate::product::coding_models::CodingExecutionUnitStatus::Superseded;
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
        let next_order_index = units
            .iter()
            .map(|unit| unit.order_index)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        for (offset, logical_id) in affected.iter().enumerate() {
            if units
                .iter()
                .all(|unit| &unit.logical_work_item_id != logical_id)
            {
                if !replacement_targets.contains(logical_id) {
                    return Err(identity_mismatch(
                        "coding_amendment_execution_unit",
                        logical_id,
                    ));
                }
                let revision_id = plan_revision
                    .work_item_bindings
                    .get(logical_id)
                    .ok_or_else(|| {
                        identity_mismatch("coding_amendment_execution_unit", logical_id)
                    })?;
                let created = self.create_coding_unit(super::CreateCodingExecutionUnitInput {
                    attempt_id: current.id.clone(),
                    project_id: current.project_id.clone(),
                    issue_id: current.issue_id.clone(),
                    plan_id: plan.id.clone(),
                    logical_work_item_id: logical_id.clone(),
                    work_item_revision_id: revision_id.clone(),
                    dependency_logical_work_item_ids: dependencies
                        .get(logical_id)
                        .cloned()
                        .unwrap_or_default(),
                    order_index: next_order_index.saturating_add(offset as u32),
                    status: crate::product::coding_models::CodingExecutionUnitStatus::Pending,
                })?;
                units.push(created);
            }
        }

        let coder_renderer = renderer_for(&current.provider_config_snapshot.author);
        let reviewer_provider = current
            .provider_config_snapshot
            .reviewer
            .as_ref()
            .unwrap_or(&current.provider_config_snapshot.author);
        let reviewer_renderer = renderer_for(reviewer_provider);
        let mut materialized = Vec::new();
        for logical_id in affected {
            let unit = unique_unit_mut(&mut units, &logical_id)?;
            let revision_id = plan_revision
                .work_item_bindings
                .get(&logical_id)
                .ok_or_else(|| identity_mismatch("coding_amendment_execution_unit", &logical_id))?
                .clone();
            if let Some(replacement) = manifest.revised_work_items.get(&logical_id)
                && replacement.next_revision_id != revision_id
            {
                return Err(identity_mismatch(
                    "coding_amendment_execution_unit",
                    &logical_id,
                ));
            }
            unit.work_item_revision_id = revision_id.clone();
            unit.dependency_logical_work_item_ids =
                dependencies.get(&logical_id).cloned().unwrap_or_default();
            unit.status = crate::product::coding_models::CodingExecutionUnitStatus::Pending;
            unit.started_at = None;
            unit.completed_at = None;
            unit.summary = Some(format!("Plan Amendment {} materialized", manifest.id));
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

            let run_id = amendment_unit_run_id(&unit.id, &manifest.id);
            let mut runs = self.list_coding_unit_runs(&current, &unit.id)?;
            if let Some(existing) = runs.iter().find(|run| run.id == run_id) {
                materialized.push(existing.clone());
                continue;
            }
            for run in runs.iter_mut().filter(|run| run.status.is_active()) {
                let path = self.coding_unit_run_path(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                    &unit.id,
                    &run.id,
                );
                run.status = CodingUnitRunStatus::Superseded;
                run.updated_at = Utc::now().to_rfc3339();
                write_json(&path, run)?;
            }
            let execution_no = runs
                .iter()
                .map(|run| run.execution_no)
                .max()
                .unwrap_or(0)
                .saturating_add(1);
            let plan_repair_count = runs
                .iter()
                .map(|run| run.plan_repair_count)
                .max()
                .unwrap_or(0);
            let revision =
                revision_store.get_work_item_revision(&plan, &logical_id, &revision_id)?;
            let bundle = revision_store
                .get_work_item_projection_bundle(&plan, &revision.work_item_projection_bundle_id)?;
            let status = if stale.contains(&logical_id) {
                CodingUnitRunStatus::Stale
            } else if revalidation.contains(&logical_id) {
                CodingUnitRunStatus::NeedsRevalidation
            } else {
                CodingUnitRunStatus::AwaitingAmendment
            };
            let requested = CodingUnitRun {
                id: run_id,
                unit_id: unit.id.clone(),
                execution_no,
                work_item_revision_id: revision_id,
                resolved_handoff_revision_ids: Vec::new(),
                canonical_contract_hash: revision.canonical_contract_hash,
                projection_bundle_id: bundle.id,
                projection_compiler_version: bundle.compiler_version,
                coder_provider_renderer_version: coder_renderer.renderer_version().to_string(),
                reviewer_provider_renderer_version: reviewer_renderer
                    .renderer_version()
                    .to_string(),
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
            };
            materialized.push(self.load_or_create_coding_unit_run(&current, &requested)?);
        }
        Ok(materialized)
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
        if run.id != run_id || run.work_item_revision_id != unit.work_item_revision_id {
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

    pub fn create_coding_unit_run(
        &self,
        attempt: &CodingExecutionAttempt,
        run: &CodingUnitRun,
    ) -> Result<(), ProductStoreError> {
        self.validate_attempt_lineage(attempt)?;
        validate_unit_run(run)?;
        let unit = self.authoritative_unit(attempt, &run.unit_id)?;
        if run.work_item_revision_id != unit.work_item_revision_id {
            return Err(identity_mismatch("coding_unit_run", &run.id));
        }
        let lock_target = self
            .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .join("unit-runs-index.json");
        with_exclusive_lock(&lock_target, || {
            if let Some((_, existing)) = self.find_unit_run_by_id(attempt, &run.id)? {
                if existing == *run {
                    return Ok(());
                }
                return Err(identity_mismatch("coding_unit_run", &run.id));
            }
            if self
                .list_coding_unit_runs(attempt, &run.unit_id)?
                .iter()
                .any(|existing| existing.execution_no == run.execution_no)
            {
                return Err(identity_mismatch("coding_unit_run_execution_no", &run.id));
            }
            write_json(
                &self.coding_unit_run_path(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    &run.unit_id,
                    &run.id,
                ),
                run,
            )
        })
    }

    pub fn load_or_create_coding_unit_run(
        &self,
        attempt: &CodingExecutionAttempt,
        requested: &CodingUnitRun,
    ) -> Result<CodingUnitRun, ProductStoreError> {
        self.validate_attempt_lineage(attempt)?;
        validate_unit_run(requested)?;
        let unit = self.authoritative_unit(attempt, &requested.unit_id)?;
        if requested.work_item_revision_id != unit.work_item_revision_id {
            return Err(identity_mismatch("coding_unit_run", &requested.id));
        }
        let lock_target = self
            .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .join("unit-runs-index.json");
        with_exclusive_lock(&lock_target, || {
            let runs = self.list_coding_unit_runs(attempt, &requested.unit_id)?;
            if let Some(existing) = runs
                .into_iter()
                .find(|run| run.execution_no == requested.execution_no)
            {
                if same_materialization_identity(&existing, requested) {
                    return Ok(existing);
                }
                return Err(identity_mismatch("coding_unit_run", &requested.id));
            }
            if self.find_unit_run_by_id(attempt, &requested.id)?.is_some() {
                return Err(identity_mismatch("coding_unit_run", &requested.id));
            }
            let now = Utc::now().to_rfc3339();
            let mut persisted = requested.clone();
            persisted.created_at = now.clone();
            persisted.updated_at = now;
            write_json(
                &self.coding_unit_run_path(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    &persisted.unit_id,
                    &persisted.id,
                ),
                &persisted,
            )?;
            Ok(persisted)
        })
    }

    pub fn list_coding_unit_runs(
        &self,
        attempt: &CodingExecutionAttempt,
        unit_id: &str,
    ) -> Result<Vec<CodingUnitRun>, ProductStoreError> {
        self.validate_attempt_lineage(attempt)?;
        let unit = self.authoritative_unit(attempt, unit_id)?;
        let mut runs: Vec<CodingUnitRun> = super::list_json_records(&self.coding_unit_runs_root(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            unit_id,
        ))?;
        for run in &runs {
            validate_unit_run(run)?;
            if run.unit_id != unit.id {
                return Err(identity_mismatch("coding_unit_run", &run.id));
            }
        }
        runs.sort_by(|left, right| {
            left.execution_no
                .cmp(&right.execution_no)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(runs)
    }

    pub fn list_unit_runs_by_logical_id(
        &self,
        attempt: &CodingExecutionAttempt,
        logical_work_item_id: &str,
    ) -> Result<Vec<CodingUnitRun>, ProductStoreError> {
        self.validate_attempt_lineage(attempt)?;
        validate_relative_id(logical_work_item_id)?;
        let mut units = self
            .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .into_iter()
            .filter(|unit| unit.logical_work_item_id == logical_work_item_id);
        let unit = units.next().ok_or_else(|| ProductStoreError::NotFound {
            kind: "coding_execution_unit",
            id: logical_work_item_id.to_string(),
        })?;
        if units.next().is_some() {
            return Err(ProductStoreError::Ambiguous {
                kind: "coding_execution_unit",
                id: logical_work_item_id.to_string(),
            });
        }
        self.list_coding_unit_runs(attempt, &unit.id)
    }

    pub fn get_active_unit_run(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingUnitRun, ProductStoreError> {
        let stored_attempt = self.validate_attempt_lineage(attempt)?;
        let unit_id = stored_attempt.active_unit_id.as_deref().ok_or_else(|| {
            ProductStoreError::NotFound {
                kind: "coding_unit_run",
                id: attempt.id.clone(),
            }
        })?;
        let unit = self.authoritative_unit(attempt, unit_id)?;
        if !unit.status.is_active() {
            return Err(identity_mismatch("coding_execution_unit", &unit.id));
        }
        let mut active = self
            .list_coding_unit_runs(attempt, unit_id)?
            .into_iter()
            .filter(|run| run.status.is_active());
        let run = active.next().ok_or_else(|| ProductStoreError::NotFound {
            kind: "coding_unit_run",
            id: unit_id.to_string(),
        })?;
        if active.next().is_some() {
            return Err(ProductStoreError::Ambiguous {
                kind: "coding_unit_run",
                id: unit_id.to_string(),
            });
        }
        Ok(run)
    }

    pub fn bind_unit_run_execution_context(
        &self,
        attempt: &CodingExecutionAttempt,
        unit_run_id: &str,
        role: CodingProviderRole,
        rendered: &RenderedExecutionContext,
    ) -> Result<CodingUnitRun, ProductStoreError> {
        self.validate_attempt_lineage(attempt)?;
        validate_relative_id(unit_run_id)?;
        let (path, found) = self
            .find_unit_run_by_id(attempt, unit_run_id)?
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "coding_unit_run",
                id: unit_run_id.to_string(),
            })?;
        self.authoritative_unit(attempt, &found.unit_id)?;
        with_exclusive_lock(&path, || {
            let mut run: CodingUnitRun = read_json(&path)?;
            if run.id != unit_run_id || run.unit_id != found.unit_id {
                return Err(identity_mismatch("coding_unit_run", unit_run_id));
            }
            match role {
                CodingProviderRole::Coder => {
                    if let Some(existing_hash) = run.coder_execution_context_hash.as_deref() {
                        if run.coder_provider_renderer_version == rendered.renderer_version
                            && existing_hash == rendered.content_hash
                        {
                            return Ok(run);
                        }
                        return Err(identity_mismatch(
                            "coding_unit_run_execution_context",
                            unit_run_id,
                        ));
                    }
                    run.coder_provider_renderer_version = rendered.renderer_version.clone();
                    run.coder_execution_context_hash = Some(rendered.content_hash.clone());
                }
                CodingProviderRole::CodeReviewer => {
                    if let Some(existing_hash) = run.reviewer_execution_context_hash.as_deref() {
                        if run.reviewer_provider_renderer_version == rendered.renderer_version
                            && existing_hash == rendered.content_hash
                        {
                            return Ok(run);
                        }
                        return Err(identity_mismatch(
                            "coding_unit_run_execution_context",
                            unit_run_id,
                        ));
                    }
                    run.reviewer_provider_renderer_version = rendered.renderer_version.clone();
                    run.reviewer_execution_context_hash = Some(rendered.content_hash.clone());
                }
                CodingProviderRole::InternalReviewer => {
                    if let Some(existing_hash) =
                        run.internal_reviewer_execution_context_hash.as_deref()
                    {
                        if run.internal_reviewer_provider_renderer_version.as_deref()
                            == Some(rendered.renderer_version.as_str())
                            && existing_hash == rendered.content_hash
                        {
                            return Ok(run);
                        }
                        return Err(identity_mismatch(
                            "coding_unit_run_execution_context",
                            unit_run_id,
                        ));
                    }
                    if run.internal_reviewer_provider_renderer_version.is_some() {
                        return Err(identity_mismatch(
                            "coding_unit_run_execution_context",
                            unit_run_id,
                        ));
                    }
                    run.internal_reviewer_provider_renderer_version =
                        Some(rendered.renderer_version.clone());
                    run.internal_reviewer_execution_context_hash =
                        Some(rendered.content_hash.clone());
                }
                CodingProviderRole::Tester => {
                    return Err(identity_mismatch(
                        "coding_unit_run_execution_context_role",
                        unit_run_id,
                    ));
                }
            }
            run.updated_at = Utc::now().to_rfc3339();
            write_json(&path, &run)?;
            Ok(run)
        })
    }

    pub fn complete_coding_unit_run(
        &self,
        attempt: &CodingExecutionAttempt,
        unit_run_id: &str,
        completion_commit: &str,
    ) -> Result<CodingUnitRun, ProductStoreError> {
        self.validate_attempt_lineage(attempt)?;
        validate_relative_id(unit_run_id)?;
        validate_relative_id(completion_commit)?;
        let (path, found) = self
            .find_unit_run_by_id(attempt, unit_run_id)?
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "coding_unit_run",
                id: unit_run_id.to_string(),
            })?;
        self.authoritative_unit(attempt, &found.unit_id)?;
        with_exclusive_lock(&path, || {
            let mut run: CodingUnitRun = read_json(&path)?;
            if run.id != unit_run_id || run.unit_id != found.unit_id {
                return Err(identity_mismatch("coding_unit_run", unit_run_id));
            }
            if run.status == CodingUnitRunStatus::Completed {
                if run.completion_commit.as_deref() == Some(completion_commit) {
                    return Ok(run);
                }
                return Err(identity_mismatch("coding_unit_run", unit_run_id));
            }
            if run.status != CodingUnitRunStatus::Running || run.completion_commit.is_some() {
                return Err(identity_mismatch("coding_unit_run", unit_run_id));
            }
            run.status = CodingUnitRunStatus::Completed;
            run.completion_commit = Some(completion_commit.to_string());
            run.updated_at = Utc::now().to_rfc3339();
            write_json(&path, &run)?;
            Ok(run)
        })
    }

    fn authoritative_unit(
        &self,
        attempt: &CodingExecutionAttempt,
        unit_id: &str,
    ) -> Result<CodingExecutionUnit, ProductStoreError> {
        validate_relative_id(unit_id)?;
        let path =
            self.coding_unit_path(&attempt.project_id, &attempt.issue_id, &attempt.id, unit_id);
        if !super::path_is_regular_file(&path)? {
            return Err(ProductStoreError::NotFound {
                kind: "coding_execution_unit",
                id: unit_id.to_string(),
            });
        }
        let unit: CodingExecutionUnit = read_json(&path)?;
        if unit.id != unit_id
            || unit.attempt_id != attempt.id
            || unit.project_id != attempt.project_id
            || unit.issue_id != attempt.issue_id
            || unit.plan_id != attempt.work_item_group_id.as_deref().unwrap_or_default()
        {
            return Err(identity_mismatch("coding_execution_unit", unit_id));
        }
        Ok(unit)
    }

    fn find_unit_run_by_id(
        &self,
        attempt: &CodingExecutionAttempt,
        unit_run_id: &str,
    ) -> Result<Option<(std::path::PathBuf, CodingUnitRun)>, ProductStoreError> {
        let mut found = None;
        for unit in self.list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)? {
            let path = self.coding_unit_run_path(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &unit.id,
                unit_run_id,
            );
            if !super::path_is_regular_file(&path)? {
                continue;
            }
            let run: CodingUnitRun = read_json(&path)?;
            if run.id != unit_run_id || run.unit_id != unit.id {
                return Err(identity_mismatch("coding_unit_run", unit_run_id));
            }
            if found.is_some() {
                return Err(ProductStoreError::Ambiguous {
                    kind: "coding_unit_run",
                    id: unit_run_id.to_string(),
                });
            }
            found = Some((path, run));
        }
        Ok(found)
    }
}

fn validate_unit_run(run: &CodingUnitRun) -> Result<(), ProductStoreError> {
    for id in [
        run.id.as_str(),
        run.unit_id.as_str(),
        run.work_item_revision_id.as_str(),
        run.projection_bundle_id.as_str(),
    ] {
        validate_relative_id(id)?;
    }
    for handoff_id in &run.resolved_handoff_revision_ids {
        validate_relative_id(handoff_id)?;
    }
    if run.execution_no == 0
        || [
            run.canonical_contract_hash.as_str(),
            run.projection_compiler_version.as_str(),
            run.coder_provider_renderer_version.as_str(),
            run.reviewer_provider_renderer_version.as_str(),
            run.coder_projection_hash.as_str(),
            run.reviewer_projection_hash.as_str(),
        ]
        .into_iter()
        .any(str::is_empty)
        || run
            .internal_reviewer_provider_renderer_version
            .as_deref()
            .is_some_and(str::is_empty)
        || run.internal_reviewer_provider_renderer_version.is_some()
            != run.internal_reviewer_execution_context_hash.is_some()
    {
        return Err(identity_mismatch("coding_unit_run", &run.id));
    }
    Ok(())
}

fn same_materialization_identity(left: &CodingUnitRun, right: &CodingUnitRun) -> bool {
    left.unit_id == right.unit_id
        && left.execution_no == right.execution_no
        && left.work_item_revision_id == right.work_item_revision_id
        && left.resolved_handoff_revision_ids == right.resolved_handoff_revision_ids
        && left.canonical_contract_hash == right.canonical_contract_hash
        && left.projection_bundle_id == right.projection_bundle_id
        && left.projection_compiler_version == right.projection_compiler_version
        && left.coder_provider_renderer_version == right.coder_provider_renderer_version
        && left.reviewer_provider_renderer_version == right.reviewer_provider_renderer_version
        && left.internal_reviewer_provider_renderer_version
            == right.internal_reviewer_provider_renderer_version
        && left.coder_projection_hash == right.coder_projection_hash
        && left.reviewer_projection_hash == right.reviewer_projection_hash
        && left.start_commit == right.start_commit
}

fn unique_unit<'a>(
    units: &'a [crate::product::coding_models::CodingExecutionUnit],
    logical_work_item_id: &str,
) -> Result<&'a crate::product::coding_models::CodingExecutionUnit, ProductStoreError> {
    let mut matches = units
        .iter()
        .filter(|unit| unit.logical_work_item_id == logical_work_item_id);
    let unit = matches.next().ok_or_else(|| ProductStoreError::NotFound {
        kind: "coding_execution_unit",
        id: logical_work_item_id.to_string(),
    })?;
    if matches.next().is_some() {
        return Err(ProductStoreError::Ambiguous {
            kind: "coding_execution_unit",
            id: logical_work_item_id.to_string(),
        });
    }
    Ok(unit)
}

fn unique_unit_mut<'a>(
    units: &'a mut [crate::product::coding_models::CodingExecutionUnit],
    logical_work_item_id: &str,
) -> Result<&'a mut crate::product::coding_models::CodingExecutionUnit, ProductStoreError> {
    let indexes = units
        .iter()
        .enumerate()
        .filter_map(|(index, unit)| {
            (unit.logical_work_item_id == logical_work_item_id).then_some(index)
        })
        .collect::<Vec<_>>();
    match indexes.as_slice() {
        [index] => Ok(&mut units[*index]),
        [] => Err(ProductStoreError::NotFound {
            kind: "coding_execution_unit",
            id: logical_work_item_id.to_string(),
        }),
        _ => Err(ProductStoreError::Ambiguous {
            kind: "coding_execution_unit",
            id: logical_work_item_id.to_string(),
        }),
    }
}

fn amendment_unit_run_id(unit_id: &str, amendment_id: &str) -> String {
    format!("coding_unit_run_{unit_id}_{amendment_id}")
}

fn identity_mismatch(kind: &'static str, id: &str) -> ProductStoreError {
    ProductStoreError::IdentityMismatch {
        kind,
        id: id.to_string(),
    }
}
