use chrono::Utc;

use crate::product::coding_models::{
    CodingExecutionAttempt, CodingExecutionUnit, CodingProviderRole, CodingUnitRun,
};
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
                    run.coder_provider_renderer_version = rendered.renderer_version.clone();
                    run.coder_execution_context_hash = Some(rendered.content_hash.clone());
                }
                CodingProviderRole::CodeReviewer => {
                    run.reviewer_provider_renderer_version = rendered.renderer_version.clone();
                    run.reviewer_execution_context_hash = Some(rendered.content_hash.clone());
                }
                CodingProviderRole::Tester | CodingProviderRole::InternalReviewer => {
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
    {
        return Err(identity_mismatch("coding_unit_run", &run.id));
    }
    Ok(())
}

fn identity_mismatch(kind: &'static str, id: &str) -> ProductStoreError {
    ProductStoreError::IdentityMismatch {
        kind,
        id: id.to_string(),
    }
}
