use chrono::Utc;
use sha2::{Digest, Sha256};

use crate::product::coding_models::{
    CodingExecutionAttempt, CodingExecutionUnitStatus, CodingUnitRun, CodingUnitRunStatus,
};
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::work_item_projection::renderer_for;
use crate::product::work_item_revision_store::WorkItemRevisionStore;

use super::locking::with_exclusive_lock;
use super::unit_run::amendment_unit_run_id;

enum PlaceholderRunResolution {
    Resolved(CodingUnitRun),
    AdvancedReplay(CodingUnitRun),
    DifferentTuple,
}

impl super::CodingAttemptStore {
    pub fn start_pending_coding_unit_run(
        &self,
        attempt: &CodingExecutionAttempt,
        unit_id: &str,
    ) -> Result<bool, ProductStoreError> {
        let current = self.validate_attempt_lineage(attempt)?;
        validate_relative_id(unit_id)?;
        let unit = self.authoritative_unit(&current, unit_id)?;
        let Some(latest) = self
            .list_coding_unit_runs(&current, &unit.id)?
            .into_iter()
            .max_by_key(|run| run.execution_no)
        else {
            return Ok(true);
        };
        if latest.status == CodingUnitRunStatus::Running {
            return Ok(true);
        }
        if latest.status != CodingUnitRunStatus::Pending || latest.completion_commit.is_some() {
            return Ok(false);
        }
        let path = self
            .find_unit_run_by_id(&current, &latest.id)?
            .map(|(path, _)| path)
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "coding_unit_run",
                id: latest.id.clone(),
            })?;
        with_exclusive_lock(&path, || {
            let mut stored: CodingUnitRun = read_json(&path)?;
            if stored != latest {
                return Err(identity_mismatch(&latest.id));
            }
            stored.status = CodingUnitRunStatus::Running;
            stored.updated_at = Utc::now().to_rfc3339();
            write_json(&path, &stored)?;
            Ok(true)
        })
    }

    pub fn resolve_runtime_handoff_unit_run(
        &self,
        attempt: &CodingExecutionAttempt,
        amendment_id: &str,
        logical_work_item_id: &str,
        resolved_handoff_revision_ids: &[String],
        status: CodingUnitRunStatus,
    ) -> Result<CodingUnitRun, ProductStoreError> {
        let current = self.validate_attempt_lineage(attempt)?;
        validate_runtime_resolution(
            amendment_id,
            logical_work_item_id,
            resolved_handoff_revision_ids,
            &status,
        )?;
        let mut unit = self.unique_runtime_handoff_unit(&current, logical_work_item_id)?;
        let revision_store = WorkItemRevisionStore::new(self.paths());
        let plan = revision_store.get_plan_lineage(
            &current.project_id,
            &current.issue_id,
            &unit.plan_id,
        )?;
        let revision = revision_store.get_work_item_revision(
            &plan,
            logical_work_item_id,
            &unit.work_item_revision_id,
        )?;
        let bundle = revision_store
            .get_work_item_projection_bundle(&plan, &revision.work_item_projection_bundle_id)?;
        let providers = self.get_role_provider_config_snapshot(
            &current.project_id,
            &current.issue_id,
            &current.id,
        )?;
        let runs = self.list_coding_unit_runs(&current, &unit.id)?;
        let placeholder_id = amendment_unit_run_id(&unit.id, amendment_id);
        let placeholder_resolution =
            if let Some((path, _)) = self.find_unit_run_by_id(&current, &placeholder_id)? {
                Some(with_exclusive_lock(&path, || {
                    resolve_placeholder_run(&path, &unit, resolved_handoff_revision_ids, &status)
                })?)
            } else {
                None
            };
        let update_unit;
        let resolved = match placeholder_resolution {
            Some(PlaceholderRunResolution::Resolved(run)) => {
                update_unit = true;
                run
            }
            Some(PlaceholderRunResolution::AdvancedReplay(run)) => return Ok(run),
            placeholder_resolution => {
                update_unit = placeholder_resolution.is_none();
                let run_id = runtime_handoff_unit_run_id(
                    &current.id,
                    amendment_id,
                    logical_work_item_id,
                    &unit.work_item_revision_id,
                    resolved_handoff_revision_ids,
                )?;
                if let Some((_, existing)) = self.find_unit_run_by_id(&current, &run_id)? {
                    if existing.unit_id != unit.id
                        || existing.work_item_revision_id != unit.work_item_revision_id
                        || existing.resolved_handoff_revision_ids != resolved_handoff_revision_ids
                    {
                        return Err(identity_mismatch(&run_id));
                    }
                    return Ok(existing);
                }
                let run = CodingUnitRun {
                    id: run_id,
                    unit_id: unit.id.clone(),
                    execution_no: runs
                        .iter()
                        .map(|run| run.execution_no)
                        .max()
                        .unwrap_or(0)
                        .saturating_add(1),
                    work_item_revision_id: revision.id.clone(),
                    resolved_handoff_revision_ids: resolved_handoff_revision_ids.to_vec(),
                    canonical_contract_hash: revision.canonical_contract_hash,
                    projection_bundle_id: bundle.id,
                    projection_compiler_version: bundle.compiler_version,
                    coder_provider_renderer_version: renderer_for(&providers.coder)
                        .renderer_version()
                        .to_string(),
                    reviewer_provider_renderer_version: renderer_for(&providers.code_reviewer)
                        .renderer_version()
                        .to_string(),
                    internal_reviewer_provider_renderer_version: None,
                    coder_projection_hash: bundle.coder_projection_hash,
                    reviewer_projection_hash: bundle.reviewer_projection_hash,
                    coder_execution_context_hash: None,
                    reviewer_execution_context_hash: None,
                    internal_reviewer_execution_context_hash: None,
                    status: status.clone(),
                    unit_rework_count: 0,
                    verification_retry_count: 0,
                    operational_retry_count: 0,
                    plan_repair_count: runs
                        .iter()
                        .map(|run| run.plan_repair_count)
                        .max()
                        .unwrap_or(0),
                    start_commit: current.head_commit.clone(),
                    completion_commit: None,
                    created_at: String::new(),
                    updated_at: String::new(),
                };
                self.load_or_create_coding_unit_run(&current, &run)?
            }
        };
        if update_unit {
            unit.status = unit_status(&status);
            unit.started_at = None;
            unit.completed_at = None;
            unit.summary = Some(format!(
                "Runtime Handoff {} resolved after Plan Amendment {}",
                resolved_handoff_revision_ids.join(","),
                amendment_id
            ));
            unit.updated_at = Utc::now().to_rfc3339();
            write_json(
                &self.coding_unit_path(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                    &unit.id,
                ),
                &unit,
            )?;
        }
        Ok(resolved)
    }

    fn unique_runtime_handoff_unit(
        &self,
        attempt: &CodingExecutionAttempt,
        logical_work_item_id: &str,
    ) -> Result<crate::product::coding_models::CodingExecutionUnit, ProductStoreError> {
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
        Ok(unit)
    }
}

fn validate_runtime_resolution(
    amendment_id: &str,
    logical_work_item_id: &str,
    resolved_handoff_revision_ids: &[String],
    status: &CodingUnitRunStatus,
) -> Result<(), ProductStoreError> {
    validate_relative_id(amendment_id)?;
    validate_relative_id(logical_work_item_id)?;
    if resolved_handoff_revision_ids.is_empty()
        || resolved_handoff_revision_ids
            .iter()
            .any(|id| validate_relative_id(id).is_err())
        || !resolved_handoff_revision_ids
            .windows(2)
            .all(|window| window[0] < window[1])
        || !matches!(
            status,
            CodingUnitRunStatus::Pending
                | CodingUnitRunStatus::NeedsRevalidation
                | CodingUnitRunStatus::Stale
        )
    {
        return Err(identity_mismatch(logical_work_item_id));
    }
    Ok(())
}

fn resolve_placeholder_run(
    path: &std::path::Path,
    unit: &crate::product::coding_models::CodingExecutionUnit,
    resolved_handoff_revision_ids: &[String],
    status: &CodingUnitRunStatus,
) -> Result<PlaceholderRunResolution, ProductStoreError> {
    let mut run: CodingUnitRun = read_json(path)?;
    if run.unit_id != unit.id || run.work_item_revision_id != unit.work_item_revision_id {
        return Err(identity_mismatch(&run.id));
    }
    if run.resolved_handoff_revision_ids == resolved_handoff_revision_ids {
        if matches!(run.status, CodingUnitRunStatus::Running) && run.completion_commit.is_none()
            || matches!(run.status, CodingUnitRunStatus::Completed)
                && run.completion_commit.is_some()
        {
            return Ok(PlaceholderRunResolution::AdvancedReplay(run));
        }
        if &run.status == status && run.completion_commit.is_none() {
            return Ok(PlaceholderRunResolution::Resolved(run));
        }
        return Err(identity_mismatch(&run.id));
    }
    if !run.resolved_handoff_revision_ids.is_empty() {
        return Ok(PlaceholderRunResolution::DifferentTuple);
    }
    if !matches!(
        run.status,
        CodingUnitRunStatus::AwaitingAmendment
            | CodingUnitRunStatus::NeedsRevalidation
            | CodingUnitRunStatus::Pending
            | CodingUnitRunStatus::Stale
    ) || run.completion_commit.is_some()
        || (run.status == CodingUnitRunStatus::Stale && status != &CodingUnitRunStatus::Stale)
    {
        return Err(identity_mismatch(&run.id));
    }
    run.resolved_handoff_revision_ids = resolved_handoff_revision_ids.to_vec();
    run.status = status.clone();
    run.updated_at = Utc::now().to_rfc3339();
    write_json(path, &run)?;
    Ok(PlaceholderRunResolution::Resolved(run))
}

fn unit_status(status: &CodingUnitRunStatus) -> CodingExecutionUnitStatus {
    match status {
        CodingUnitRunStatus::Pending => CodingExecutionUnitStatus::Pending,
        CodingUnitRunStatus::NeedsRevalidation => CodingExecutionUnitStatus::NeedsRevalidation,
        CodingUnitRunStatus::Stale => CodingExecutionUnitStatus::Stale,
        _ => unreachable!("runtime handoff status was validated"),
    }
}

fn runtime_handoff_unit_run_id(
    attempt_id: &str,
    amendment_id: &str,
    logical_work_item_id: &str,
    work_item_revision_id: &str,
    resolved_handoff_revision_ids: &[String],
) -> Result<String, ProductStoreError> {
    let bytes = serde_json::to_vec(&(
        attempt_id,
        amendment_id,
        logical_work_item_id,
        work_item_revision_id,
        resolved_handoff_revision_ids,
    ))
    .map_err(|error| ProductStoreError::Io(error.to_string()))?;
    Ok(format!(
        "coding_unit_run_runtime_handoff_{}",
        hex::encode(Sha256::digest(bytes))
    ))
}

fn identity_mismatch(id: &str) -> ProductStoreError {
    ProductStoreError::IdentityMismatch {
        kind: "coding_runtime_handoff_unit_run",
        id: id.to_string(),
    }
}
