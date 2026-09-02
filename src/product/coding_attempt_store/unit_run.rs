use chrono::Utc;

use crate::product::coding_models::{
    CodingExecutionAttempt, CodingExecutionUnit, CodingProviderRole, CodingUnitRun,
    CodingUnitRunStatus,
};
use crate::product::id::next_sequential_id;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::work_item_projection::RenderedExecutionContext;

use super::locking::with_exclusive_lock;

impl super::CodingAttemptStore {
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
                if same_initial_materialization_state(&existing, requested) {
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

    pub fn create_retry_coding_unit_run(
        &self,
        attempt: &CodingExecutionAttempt,
        unit_id: &str,
        prior_run_id: &str,
    ) -> Result<CodingUnitRun, ProductStoreError> {
        let current = self.validate_attempt_lineage(attempt)?;
        validate_relative_id(unit_id)?;
        validate_relative_id(prior_run_id)?;
        let unit = self.authoritative_unit(&current, unit_id)?;
        let lock_target = self
            .attempt_dir(&current.project_id, &current.issue_id, &current.id)
            .join("unit-runs-index.json");
        with_exclusive_lock(&lock_target, || {
            let runs = self.list_coding_unit_runs(&current, unit_id)?;
            let prior = runs
                .iter()
                .find(|run| run.id == prior_run_id)
                .ok_or_else(|| ProductStoreError::NotFound {
                    kind: "coding_unit_run",
                    id: prior_run_id.to_string(),
                })?;
            if prior.unit_id != unit.id
                || prior.status != CodingUnitRunStatus::Running
                || prior.completion_commit.is_some()
            {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "coding_unit_run_retry",
                    id: prior_run_id.to_string(),
                });
            }
            let prior_path = self.coding_unit_run_path(
                &current.project_id,
                &current.issue_id,
                &current.id,
                unit_id,
                prior_run_id,
            );
            let mut failed = prior.clone();
            failed.status = CodingUnitRunStatus::Failed;
            failed.updated_at = Utc::now().to_rfc3339();
            write_json(&prior_path, &failed)?;

            let now = Utc::now().to_rfc3339();
            let mut retry = prior.clone();
            retry.id = next_sequential_id("coding_unit_run", runs.len());
            retry.execution_no = prior.execution_no.saturating_add(1);
            retry.status = CodingUnitRunStatus::Running;
            retry.completion_commit = None;
            retry.operational_retry_count = prior.operational_retry_count.saturating_add(1);
            retry.created_at = now.clone();
            retry.updated_at = now;
            write_json(
                &self.coding_unit_run_path(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                    unit_id,
                    &retry.id,
                ),
                &retry,
            )?;
            Ok(retry)
        })
    }

    pub fn fail_coding_unit_run(
        &self,
        attempt: &CodingExecutionAttempt,
        unit_id: &str,
        run_id: &str,
    ) -> Result<CodingUnitRun, ProductStoreError> {
        let current = self.validate_attempt_lineage(attempt)?;
        validate_relative_id(unit_id)?;
        validate_relative_id(run_id)?;
        let (path, found) = self.find_unit_run_by_id(&current, run_id)?.ok_or_else(|| {
            ProductStoreError::NotFound {
                kind: "coding_unit_run",
                id: run_id.to_string(),
            }
        })?;
        if found.unit_id != unit_id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_unit_run",
                id: run_id.to_string(),
            });
        }
        with_exclusive_lock(&path, || {
            let mut run: CodingUnitRun = read_json(&path)?;
            if run.id != run_id || run.unit_id != unit_id {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "coding_unit_run",
                    id: run_id.to_string(),
                });
            }
            if run.status == CodingUnitRunStatus::Failed {
                return Ok(run);
            }
            if run.status != CodingUnitRunStatus::Running || run.completion_commit.is_some() {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "coding_unit_run",
                    id: run_id.to_string(),
                });
            }
            run.status = CodingUnitRunStatus::Failed;
            run.updated_at = Utc::now().to_rfc3339();
            write_json(&path, &run)?;
            Ok(run)
        })
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
            }
            run.updated_at = Utc::now().to_rfc3339();
            write_json(&path, &run)?;
            Ok(run)
        })
    }

    pub fn backfill_coding_unit_run_start_commit(
        &self,
        attempt: &CodingExecutionAttempt,
        unit_run_id: &str,
        start_commit: &str,
    ) -> Result<CodingUnitRun, ProductStoreError> {
        self.validate_attempt_lineage(attempt)?;
        validate_relative_id(unit_run_id)?;
        validate_relative_id(start_commit)?;
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
            if run.start_commit.is_some() {
                return Ok(run);
            }
            run.start_commit = Some(start_commit.to_string());
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
            if !matches!(
                run.status,
                CodingUnitRunStatus::Running | CodingUnitRunStatus::NeedsRevalidation
            ) || run.completion_commit.is_some()
            {
                return Err(identity_mismatch("coding_unit_run", unit_run_id));
            }
            run.status = CodingUnitRunStatus::Completed;
            run.completion_commit = Some(completion_commit.to_string());
            run.updated_at = Utc::now().to_rfc3339();
            write_json(&path, &run)?;
            Ok(run)
        })
    }

    pub(super) fn authoritative_unit(
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

    pub(super) fn find_unit_run_by_id(
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

pub(super) fn same_immutable_materialization_identity(
    left: &CodingUnitRun,
    right: &CodingUnitRun,
) -> bool {
    left.unit_id == right.unit_id
        && left.execution_no == right.execution_no
        && left.work_item_revision_id == right.work_item_revision_id
        && left.resolved_handoff_revision_ids == right.resolved_handoff_revision_ids
        && left.canonical_contract_hash == right.canonical_contract_hash
        && left.projection_bundle_id == right.projection_bundle_id
        && left.projection_compiler_version == right.projection_compiler_version
        && left.coder_provider_renderer_version == right.coder_provider_renderer_version
        && left.reviewer_provider_renderer_version == right.reviewer_provider_renderer_version
        && left.coder_projection_hash == right.coder_projection_hash
        && left.reviewer_projection_hash == right.reviewer_projection_hash
        && left.start_commit == right.start_commit
        && left
            .internal_reviewer_provider_renderer_version
            .as_deref()
            .is_none_or(|version| version == right.reviewer_provider_renderer_version)
}

pub(super) fn same_initial_materialization_state(
    left: &CodingUnitRun,
    right: &CodingUnitRun,
) -> bool {
    same_immutable_materialization_identity(left, right)
        && left.internal_reviewer_provider_renderer_version
            == right.internal_reviewer_provider_renderer_version
        && left.coder_execution_context_hash == right.coder_execution_context_hash
        && left.reviewer_execution_context_hash == right.reviewer_execution_context_hash
        && left.internal_reviewer_execution_context_hash
            == right.internal_reviewer_execution_context_hash
        && left.status == right.status
        && left.unit_rework_count == right.unit_rework_count
        && left.verification_retry_count == right.verification_retry_count
        && left.operational_retry_count == right.operational_retry_count
        && left.plan_repair_count == right.plan_repair_count
        && left.completion_commit == right.completion_commit
}

pub(super) fn valid_materialized_runtime_evolution(
    current: &CodingUnitRun,
    initial: &CodingUnitRun,
) -> bool {
    same_stable_materialization_identity(current, initial)
        && valid_bound_renderer_evolution(
            &current.coder_provider_renderer_version,
            &initial.coder_provider_renderer_version,
            current.coder_execution_context_hash.as_deref(),
        )
        && valid_bound_renderer_evolution(
            &current.reviewer_provider_renderer_version,
            &initial.reviewer_provider_renderer_version,
            current.reviewer_execution_context_hash.as_deref(),
        )
        && match (
            current
                .internal_reviewer_provider_renderer_version
                .as_deref(),
            current.internal_reviewer_execution_context_hash.as_deref(),
        ) {
            (None, None) => true,
            (Some(renderer), Some(hash)) => !renderer.is_empty() && !hash.is_empty(),
            _ => false,
        }
        && current.unit_rework_count >= initial.unit_rework_count
        && current.verification_retry_count >= initial.verification_retry_count
        && current.operational_retry_count >= initial.operational_retry_count
        && current.plan_repair_count >= initial.plan_repair_count
        && valid_runtime_status(&initial.status, &current.status)
        && current
            .coder_execution_context_hash
            .as_deref()
            .is_none_or(|hash| !hash.is_empty())
        && current
            .reviewer_execution_context_hash
            .as_deref()
            .is_none_or(|hash| !hash.is_empty())
        && current
            .internal_reviewer_execution_context_hash
            .as_deref()
            .is_none_or(|hash| !hash.is_empty())
        && (current.status != CodingUnitRunStatus::Completed || current.completion_commit.is_some())
}

fn same_stable_materialization_identity(left: &CodingUnitRun, right: &CodingUnitRun) -> bool {
    left.unit_id == right.unit_id
        && left.execution_no == right.execution_no
        && left.work_item_revision_id == right.work_item_revision_id
        && left.resolved_handoff_revision_ids == right.resolved_handoff_revision_ids
        && left.canonical_contract_hash == right.canonical_contract_hash
        && left.projection_bundle_id == right.projection_bundle_id
        && left.projection_compiler_version == right.projection_compiler_version
        && left.coder_projection_hash == right.coder_projection_hash
        && left.reviewer_projection_hash == right.reviewer_projection_hash
        && left.start_commit == right.start_commit
}

fn valid_bound_renderer_evolution(
    current_renderer: &str,
    initial_renderer: &str,
    execution_context_hash: Option<&str>,
) -> bool {
    if current_renderer.is_empty() {
        return false;
    }
    match execution_context_hash {
        Some(hash) => !hash.is_empty(),
        None => current_renderer == initial_renderer,
    }
}

fn valid_runtime_status(initial: &CodingUnitRunStatus, current: &CodingUnitRunStatus) -> bool {
    match initial {
        CodingUnitRunStatus::Running => !matches!(
            current,
            CodingUnitRunStatus::Pending
                | CodingUnitRunStatus::AwaitingAmendment
                | CodingUnitRunStatus::NeedsRevalidation
                | CodingUnitRunStatus::Stale
        ),
        CodingUnitRunStatus::NeedsRevalidation => !matches!(
            current,
            CodingUnitRunStatus::Pending
                | CodingUnitRunStatus::AwaitingAmendment
                | CodingUnitRunStatus::Stale
        ),
        CodingUnitRunStatus::AwaitingAmendment => !matches!(
            current,
            CodingUnitRunStatus::Pending
                | CodingUnitRunStatus::NeedsRevalidation
                | CodingUnitRunStatus::Stale
        ),
        CodingUnitRunStatus::Stale => matches!(
            current,
            CodingUnitRunStatus::Stale | CodingUnitRunStatus::Superseded
        ),
        _ => current == initial,
    }
}

pub(super) fn unique_unit<'a>(
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

pub(super) fn unique_unit_mut<'a>(
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

pub(super) fn amendment_unit_run_id(unit_id: &str, amendment_id: &str) -> String {
    format!("coding_unit_run_{unit_id}_{amendment_id}")
}

fn identity_mismatch(kind: &'static str, id: &str) -> ProductStoreError {
    ProductStoreError::IdentityMismatch {
        kind,
        id: id.to_string(),
    }
}
