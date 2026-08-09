//! Durable store for `AggregateInitializationOperation`.
//!
//! Records live at
//! `.aria/projects/{project}/logical-codebase/aggregate-initializations/{operation_id}.json`
//! and are byte-stable. The store enforces the five-step state machine, rejects
//! jumps/reorders, and requires an output checkpoint before a step can be marked
//! completed.

use std::path::PathBuf;

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};

use super::aggregate_initialization::{
    AGGREGATE_INITIALIZATION_OPERATION_KIND, AggregateCancellationRecord,
    AggregateInitializationErrorRecord, AggregateInitializationOperation,
    AggregateInitializationOperationStatus, AggregateInitializationStepKind,
    AggregateInitializationStepRecord, AggregateInitializationStepStatus,
};

#[derive(Debug, Clone)]
pub struct AggregateInitializationOperationStore {
    paths: ProductAppPaths,
}

impl AggregateInitializationOperationStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    /// Create the operation idempotently. A retried create with the same
    /// idempotency identity (project, key, manifest revision, policy digest,
    /// profile/evidence digest) returns the existing record; any drift yields
    /// a conflict.
    pub fn create_idempotent(
        &self,
        operation: AggregateInitializationOperation,
    ) -> Result<AggregateInitializationOperation, ProductStoreError> {
        validate_initial_operation(&operation)?;
        let path = self.operation_path(&operation.project_id, &operation.operation_id)?;
        if path.exists() {
            let existing: AggregateInitializationOperation = read_json(&path)?;
            ensure_identity(&existing, &operation.project_id, &operation.operation_id)?;
            validate_record_shape(&existing)?;
            if existing.idempotency_identity() == operation.idempotency_identity()
                && existing == operation
            {
                return Ok(existing);
            }
            return Err(conflict(&operation.operation_id));
        }
        write_json(&path, &operation)?;
        Ok(operation)
    }

    pub fn get(
        &self,
        project_id: &str,
        operation_id: &str,
    ) -> Result<AggregateInitializationOperation, ProductStoreError> {
        let path = self.operation_path(project_id, operation_id)?;
        if !path.exists() {
            return Err(not_found(operation_id));
        }
        let operation: AggregateInitializationOperation = read_json(&path)?;
        ensure_identity(&operation, project_id, operation_id)?;
        validate_record_shape(&operation)?;
        Ok(operation)
    }

    pub fn mark_running(
        &self,
        project_id: &str,
        operation_id: &str,
        updated_at: String,
    ) -> Result<AggregateInitializationOperation, ProductStoreError> {
        self.update(project_id, operation_id, |operation| {
            if operation.status != AggregateInitializationOperationStatus::Created {
                return Err(identity_mismatch(operation_id));
            }
            operation.status = AggregateInitializationOperationStatus::Running;
            operation.updated_at = updated_at;
            Ok(())
        })
    }

    /// Start a step. Requires the operation to be `Running`, all preceding
    /// steps to be `Completed`, and the target step to be `Pending`. The
    /// `input_digest` is captured on the step record so a replayed checkpoint
    /// can be matched.
    pub fn mark_step_running(
        &self,
        project_id: &str,
        operation_id: &str,
        step_id: AggregateInitializationStepKind,
        input_digest: String,
        updated_at: String,
    ) -> Result<AggregateInitializationOperation, ProductStoreError> {
        self.update(project_id, operation_id, |operation| {
            if operation.status != AggregateInitializationOperationStatus::Running {
                return Err(identity_mismatch(operation_id));
            }
            let step_index = step_id.index();
            if operation.steps[..step_index]
                .iter()
                .any(|step| step.status != AggregateInitializationStepStatus::Completed)
            {
                return Err(identity_mismatch(operation_id));
            }
            let step = operation
                .steps
                .get(step_index)
                .ok_or_else(|| identity_mismatch(operation_id))?;
            if step.status == AggregateInitializationStepStatus::Running {
                return Ok(());
            }
            if step.status != AggregateInitializationStepStatus::Pending {
                return Err(identity_mismatch(operation_id));
            }
            if operation.current_step.is_some() {
                return Err(identity_mismatch(operation_id));
            }

            let step = operation
                .steps
                .get_mut(step_index)
                .ok_or_else(|| identity_mismatch(operation_id))?;
            step.status = AggregateInitializationStepStatus::Running;
            step.started_at = Some(updated_at.clone());
            step.completed_at = None;
            step.input_digest = Some(input_digest);
            operation.current_step = Some(step_id);
            operation.updated_at = updated_at;
            Ok(())
        })
    }

    /// Capture a step's output artifact reference before the step is completed.
    /// Required before `mark_step_completed` for every step.
    pub fn checkpoint_step_output(
        &self,
        project_id: &str,
        operation_id: &str,
        step_id: AggregateInitializationStepKind,
        output_artifact_ref: String,
        updated_at: String,
    ) -> Result<AggregateInitializationOperation, ProductStoreError> {
        self.update(project_id, operation_id, |operation| {
            if operation.status != AggregateInitializationOperationStatus::Running
                || operation.current_step != Some(step_id)
            {
                return Err(identity_mismatch(operation_id));
            }
            let step_index = step_id.index();
            let step = operation
                .steps
                .get_mut(step_index)
                .ok_or_else(|| identity_mismatch(operation_id))?;
            if step.status != AggregateInitializationStepStatus::Running
                || step.output_artifact_ref.is_some()
            {
                return Err(identity_mismatch(operation_id));
            }
            step.output_artifact_ref = Some(output_artifact_ref);
            operation.updated_at = updated_at;
            Ok(())
        })
    }

    pub fn mark_step_completed(
        &self,
        project_id: &str,
        operation_id: &str,
        step_id: AggregateInitializationStepKind,
        updated_at: String,
    ) -> Result<AggregateInitializationOperation, ProductStoreError> {
        self.update(project_id, operation_id, |operation| {
            if operation.status != AggregateInitializationOperationStatus::Running
                || operation.current_step != Some(step_id)
            {
                return Err(identity_mismatch(operation_id));
            }
            let step_index = step_id.index();
            let step = operation
                .steps
                .get_mut(step_index)
                .ok_or_else(|| identity_mismatch(operation_id))?;
            if step.status != AggregateInitializationStepStatus::Running
                || step.output_artifact_ref.is_none()
            {
                return Err(identity_mismatch(operation_id));
            }
            step.status = AggregateInitializationStepStatus::Completed;
            step.completed_at = Some(updated_at.clone());
            operation.current_step = None;
            operation.updated_at = updated_at;
            Ok(())
        })
    }

    /// Mark the operation completed once every step is `Completed`. Requires
    /// the operation to be `Running` with no active step, failure or
    /// cancellation.
    pub fn finish_completed(
        &self,
        project_id: &str,
        operation_id: &str,
        completed_at: String,
    ) -> Result<AggregateInitializationOperation, ProductStoreError> {
        self.update(project_id, operation_id, |operation| {
            if operation.status != AggregateInitializationOperationStatus::Running
                || operation.current_step.is_some()
                || operation.failed_step.is_some()
                || operation.cancellation.is_some()
                || operation.error.is_some()
                || operation.completed_at.is_some()
                || !operation.steps.iter().all(is_completed_step)
            {
                return Err(identity_mismatch(operation_id));
            }
            operation.status = AggregateInitializationOperationStatus::Completed;
            operation.updated_at = completed_at.clone();
            operation.completed_at = Some(completed_at);
            Ok(())
        })
    }

    pub fn finish_failed(
        &self,
        project_id: &str,
        operation_id: &str,
        failed_step: Option<AggregateInitializationStepKind>,
        error: AggregateInitializationErrorRecord,
        completed_at: String,
    ) -> Result<AggregateInitializationOperation, ProductStoreError> {
        self.update(project_id, operation_id, |operation| {
            if operation.status != AggregateInitializationOperationStatus::Running {
                return Err(identity_mismatch(operation_id));
            }
            if let Some(step_id) = failed_step {
                if operation.current_step != Some(step_id) {
                    return Err(identity_mismatch(operation_id));
                }
                let step_index = step_id.index();
                let step = operation
                    .steps
                    .get_mut(step_index)
                    .ok_or_else(|| identity_mismatch(operation_id))?;
                if step.status != AggregateInitializationStepStatus::Running {
                    return Err(identity_mismatch(operation_id));
                }
                step.status = AggregateInitializationStepStatus::Failed;
                step.completed_at = Some(completed_at.clone());
            } else if operation.current_step.is_some() {
                return Err(identity_mismatch(operation_id));
            }

            operation.status = AggregateInitializationOperationStatus::Failed;
            operation.failed_step = failed_step;
            operation.current_step = None;
            operation.cancellation = None;
            operation.error = Some(error);
            operation.updated_at = completed_at.clone();
            operation.completed_at = Some(completed_at);
            Ok(())
        })
    }

    pub fn cancel(
        &self,
        project_id: &str,
        operation_id: &str,
        cancellation: AggregateCancellationRecord,
        updated_at: String,
    ) -> Result<AggregateInitializationOperation, ProductStoreError> {
        let operation = self.update(project_id, operation_id, |operation| {
            if !matches!(
                operation.status,
                AggregateInitializationOperationStatus::Created
                    | AggregateInitializationOperationStatus::Running
            ) {
                return Err(identity_mismatch(operation_id));
            }
            if operation.cancellation.is_some() {
                return Err(identity_mismatch(operation_id));
            }
            if let Some(step_id) = operation.current_step {
                let step_index = step_id.index();
                let step = operation
                    .steps
                    .get_mut(step_index)
                    .ok_or_else(|| identity_mismatch(operation_id))?;
                if step.status == AggregateInitializationStepStatus::Running {
                    step.status = AggregateInitializationStepStatus::Failed;
                    step.completed_at = Some(updated_at.clone());
                    operation.failed_step = Some(step_id);
                }
            }
            operation.status = AggregateInitializationOperationStatus::Cancelled;
            operation.current_step = None;
            operation.cancellation = Some(cancellation);
            operation.updated_at = updated_at.clone();
            operation.completed_at = Some(updated_at);
            Ok(())
        })?;

        // Persisted cancellation is the source of truth. Drop any
        // operation-owned staging so a later explicit resume starts clean;
        // already-atomic-published digests are never guessed-rolled-back.
        let staging_root = self.staging_path(project_id, operation_id)?;
        if staging_root.exists() {
            std::fs::remove_dir_all(&staging_root).map_err(|error| {
                ProductStoreError::Io(format!(
                    "cancel could not delete staging {}: {error}",
                    staging_root.display()
                ))
            })?;
        }
        Ok(operation)
    }

    pub fn recover_interrupted(
        &self,
        project_id: &str,
        operation_id: &str,
        completed_at: String,
    ) -> Result<AggregateInitializationOperation, ProductStoreError> {
        let operation = self.update(project_id, operation_id, |operation| {
            if matches!(
                operation.status,
                AggregateInitializationOperationStatus::Completed
                    | AggregateInitializationOperationStatus::Failed
                    | AggregateInitializationOperationStatus::Cancelled
            ) {
                return Ok(());
            }

            let failed_step = operation
                .steps
                .iter_mut()
                .find(|step| step.status == AggregateInitializationStepStatus::Running)
                .map(|step| {
                    step.status = AggregateInitializationStepStatus::Failed;
                    step.completed_at = Some(completed_at.clone());
                    step.step_id
                });
            operation.status = AggregateInitializationOperationStatus::Failed;
            operation.failed_step = failed_step;
            operation.current_step = None;
            operation.cancellation = None;
            operation.error = Some(AggregateInitializationErrorRecord::interrupted());
            operation.updated_at = completed_at.clone();
            operation.completed_at = Some(completed_at);
            Ok(())
        })?;

        // Recovery never auto-restarts the provider. Any operation-owned
        // staging left behind by the interrupted step is deleted so a later
        // explicit resume starts from a clean slate; the persisted record
        // above is the only source of truth. A missing staging directory is
        // not an error (the step may not have written any yet).
        let staging_root = self.staging_path(project_id, operation_id)?;
        if staging_root.exists() {
            std::fs::remove_dir_all(&staging_root).map_err(|error| {
                ProductStoreError::Io(format!(
                    "recover_interrupted could not delete staging {}: {error}",
                    staging_root.display()
                ))
            })?;
        }
        Ok(operation)
    }

    fn operation_path(
        &self,
        project_id: &str,
        operation_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(operation_id)?;
        Ok(self
            .paths
            .aggregate_initializations_root(project_id)
            .join(format!("{operation_id}.json")))
    }

    /// The operation-owned staging directory used by an in-flight step to
    /// buffer partial output before it is checkpointed. It lives next to the
    /// operation record under the aggregate-initializations root and is
    /// deleted on cancel/recover so a later explicit resume starts clean.
    fn staging_path(
        &self,
        project_id: &str,
        operation_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(operation_id)?;
        Ok(self
            .paths
            .aggregate_initializations_root(project_id)
            .join(operation_id)
            .join("staging"))
    }

    fn update(
        &self,
        project_id: &str,
        operation_id: &str,
        update: impl FnOnce(&mut AggregateInitializationOperation) -> Result<(), ProductStoreError>,
    ) -> Result<AggregateInitializationOperation, ProductStoreError> {
        let path = self.operation_path(project_id, operation_id)?;
        if !path.exists() {
            return Err(not_found(operation_id));
        }
        let mut operation: AggregateInitializationOperation = read_json(&path)?;
        ensure_identity(&operation, project_id, operation_id)?;
        validate_record_shape(&operation)?;
        update(&mut operation)?;
        validate_record_shape(&operation)?;
        write_json(&path, &operation)?;
        Ok(operation)
    }
}

fn validate_initial_operation(
    operation: &AggregateInitializationOperation,
) -> Result<(), ProductStoreError> {
    validate_relative_id(&operation.project_id)?;
    validate_relative_id(&operation.operation_id)?;
    if operation.operation_kind != AGGREGATE_INITIALIZATION_OPERATION_KIND
        || operation.status != AggregateInitializationOperationStatus::Created
        || operation.steps.len() != AggregateInitializationStepKind::V1.len()
        || operation
            .steps
            .iter()
            .zip(AggregateInitializationStepKind::V1)
            .any(|(step, expected)| {
                step.step_id != expected
                    || step.status != AggregateInitializationStepStatus::Pending
                    || step.started_at.is_some()
                    || step.completed_at.is_some()
                    || step.input_digest.is_some()
                    || step.output_artifact_ref.is_some()
            })
        || operation.current_step.is_some()
        || operation.failed_step.is_some()
        || !operation.member_projections.is_empty()
        || operation.cancellation.is_some()
        || operation.error.is_some()
        || operation.completed_at.is_some()
    {
        return Err(identity_mismatch(&operation.operation_id));
    }
    Ok(())
}

fn ensure_identity(
    operation: &AggregateInitializationOperation,
    project_id: &str,
    operation_id: &str,
) -> Result<(), ProductStoreError> {
    if operation.project_id != project_id || operation.operation_id != operation_id {
        return Err(identity_mismatch(operation_id));
    }
    Ok(())
}

fn validate_record_shape(
    operation: &AggregateInitializationOperation,
) -> Result<(), ProductStoreError> {
    if !has_supported_step_layout(operation) || !valid_operation_state(operation) {
        return Err(identity_mismatch(&operation.operation_id));
    }
    Ok(())
}

fn has_supported_step_layout(operation: &AggregateInitializationOperation) -> bool {
    operation.steps.len() == AggregateInitializationStepKind::V1.len()
        && operation
            .steps
            .iter()
            .zip(AggregateInitializationStepKind::V1)
            .all(|(step, expected)| step.step_id == expected)
}

fn valid_operation_state(operation: &AggregateInitializationOperation) -> bool {
    match operation.status {
        AggregateInitializationOperationStatus::Created => {
            operation.steps.iter().all(is_pending_step)
                && operation.current_step.is_none()
                && operation.failed_step.is_none()
                && operation.cancellation.is_none()
                && operation.error.is_none()
                && operation.completed_at.is_none()
        }
        AggregateInitializationOperationStatus::Running => {
            valid_running_steps(operation)
                && operation.failed_step.is_none()
                && operation.cancellation.is_none()
                && operation.error.is_none()
                && operation.completed_at.is_none()
        }
        AggregateInitializationOperationStatus::Completed => {
            operation.steps.iter().all(is_completed_step)
                && operation.current_step.is_none()
                && operation.failed_step.is_none()
                && operation.cancellation.is_none()
                && operation.error.is_none()
                && operation.completed_at.is_some()
        }
        AggregateInitializationOperationStatus::Failed => {
            valid_terminal_failed_state(operation) && operation.cancellation.is_none()
        }
        AggregateInitializationOperationStatus::Cancelled => {
            operation.cancellation.is_some() && operation.error.is_none()
        }
    }
}

fn valid_running_steps(operation: &AggregateInitializationOperation) -> bool {
    match operation
        .steps
        .iter()
        .position(|step| step.status == AggregateInitializationStepStatus::Running)
    {
        Some(running_index) => {
            operation.steps[..running_index]
                .iter()
                .all(is_completed_step)
                && is_running_step(&operation.steps[running_index])
                && operation.steps[running_index + 1..]
                    .iter()
                    .all(is_pending_step)
                && operation.current_step == Some(operation.steps[running_index].step_id)
        }
        None => completed_prefix_pending_suffix(operation) && operation.current_step.is_none(),
    }
}

fn valid_terminal_failed_state(operation: &AggregateInitializationOperation) -> bool {
    let has_terminal = operation.error.is_some() && operation.completed_at.is_some();
    match operation.failed_step {
        Some(failed_step) => {
            let failed_index = failed_step.index();
            let Some(failed_record) = operation.steps.get(failed_index) else {
                return false;
            };
            operation.steps[..failed_index]
                .iter()
                .all(is_completed_step)
                && is_failed_step(failed_record)
                && operation.steps[failed_index + 1..]
                    .iter()
                    .all(is_pending_step)
                && operation.current_step.is_none()
                && has_terminal
        }
        None => {
            operation.current_step.is_none()
                && has_terminal
                && (operation.steps.iter().all(is_completed_step)
                    || completed_prefix_pending_suffix(operation))
        }
    }
}

fn completed_prefix_pending_suffix(operation: &AggregateInitializationOperation) -> bool {
    let mut pending_seen = false;
    for step in &operation.steps {
        if is_completed_step(step) && !pending_seen {
            continue;
        }
        if is_pending_step(step) {
            pending_seen = true;
            continue;
        }
        return false;
    }
    true
}

fn is_pending_step(step: &AggregateInitializationStepRecord) -> bool {
    step.status == AggregateInitializationStepStatus::Pending
        && step.started_at.is_none()
        && step.completed_at.is_none()
        && step.input_digest.is_none()
        && step.output_artifact_ref.is_none()
}

fn is_running_step(step: &AggregateInitializationStepRecord) -> bool {
    step.status == AggregateInitializationStepStatus::Running
        && step.started_at.is_some()
        && step.completed_at.is_none()
        && step.input_digest.is_some()
}

fn is_completed_step(step: &AggregateInitializationStepRecord) -> bool {
    step.status == AggregateInitializationStepStatus::Completed
        && step.started_at.is_some()
        && step.completed_at.is_some()
        && step.output_artifact_ref.is_some()
}

fn is_failed_step(step: &AggregateInitializationStepRecord) -> bool {
    step.status == AggregateInitializationStepStatus::Failed
        && step.started_at.is_some()
        && step.completed_at.is_some()
}

fn identity_mismatch(id: &str) -> ProductStoreError {
    ProductStoreError::IdentityMismatch {
        kind: AGGREGATE_INITIALIZATION_OPERATION_KIND,
        id: id.to_string(),
    }
}

fn conflict(operation_id: &str) -> ProductStoreError {
    ProductStoreError::Conflict {
        kind: AGGREGATE_INITIALIZATION_OPERATION_KIND,
        id: operation_id.to_string(),
    }
}

fn not_found(operation_id: &str) -> ProductStoreError {
    ProductStoreError::NotFound {
        kind: AGGREGATE_INITIALIZATION_OPERATION_KIND,
        id: operation_id.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::json_store::ProductStoreError;
    use crate::product::logical_codebase::aggregate_initialization::{
        AggregateInitializationIdempotencyIdentity, AggregateInitializationOperationInput,
        AggregateInitializationStepKind,
    };
    use crate::product::repository_store::RepositoryInitializationOperationStore;

    const CREATED_AT: &str = "2026-08-09T00:00:00Z";
    const RUNNING_AT: &str = "2026-08-09T00:00:01Z";
    const STEP_AT: &str = "2026-08-09T00:00:10Z";

    struct AggregateInitFixture {
        _temp: tempfile::TempDir,
        paths: ProductAppPaths,
        store: AggregateInitializationOperationStore,
        provider_turns: std::sync::Mutex<u32>,
        operation_id: String,
    }

    impl AggregateInitFixture {
        fn store(&self) -> &AggregateInitializationOperationStore {
            &self.store
        }

        fn now(&self) -> String {
            STEP_AT.to_string()
        }

        /// A counter that the coordinator would increment on every provider
        /// turn. The store's recovery path must never touch a provider, so
        /// `turn_count()` staying at 0 after `recover_interrupted` proves no
        /// auto-restart happened.
        fn provider(&self) -> ProviderTurnProbe<'_> {
            ProviderTurnProbe {
                turns: &self.provider_turns,
            }
        }

        /// The operation-owned staging directory, mirroring the private
        /// `AggregateInitializationOperationStore::staging_path` layout so the
        /// test can seed and assert against the exact path the store cleans.
        fn staging_root(&self) -> PathBuf {
            self.paths
                .aggregate_initializations_root("project_0001")
                .join(&self.operation_id)
                .join("staging")
        }

        /// Write a partial staging artifact as an interrupted provider turn
        /// would, then leave the operation persisted as `Running` at `step`.
        fn persist_running_at(&self, step: AggregateInitializationStepKind) {
            let operation = self
                .store()
                .create_idempotent(self.new_operation("0001"))
                .unwrap();
            self.store()
                .mark_running("project_0001", &operation.operation_id, RUNNING_AT.into())
                .unwrap();
            for preceding in AggregateInitializationStepKind::V1 {
                if preceding == step {
                    break;
                }
                self.run_step(&operation.operation_id, preceding);
            }
            self.store()
                .mark_step_running(
                    "project_0001",
                    &operation.operation_id,
                    step,
                    format!(
                        "aggregate-init:project_0001:{}:{}:input",
                        operation.operation_id,
                        step.as_str()
                    ),
                    STEP_AT.to_string(),
                )
                .unwrap();
        }

        /// Seed a partial staging file inside the operation-owned staging
        /// directory, simulating an interrupted provider turn that wrote
        /// partial output before being killed.
        fn write_staging(&self, relative: &str) {
            let path = self.staging_root().join(relative);
            std::fs::create_dir_all(path.parent().expect("staging relative path has parent"))
                .unwrap();
            std::fs::write(&path, b"partial").unwrap();
        }

        fn new_operation(&self, idempotency_key: &str) -> AggregateInitializationOperation {
            AggregateInitializationOperation::new(
                format!("aggregate_initialization_{idempotency_key}"),
                "project_0001".to_string(),
                self.input(idempotency_key),
                CREATED_AT.to_string(),
            )
        }

        fn input(&self, idempotency_key: &str) -> AggregateInitializationOperationInput {
            AggregateInitializationOperationInput {
                idempotency_key: idempotency_key.to_string(),
                manifest_revision: 1,
                policy_digest: "sha256:policy".to_string(),
                profile_evidence_digest: Some("sha256:profile".to_string()),
                provider_context_root: self._temp.path().join("aggregate-root"),
                provider: "claude_code".to_string(),
            }
        }

        fn run_step(
            &self,
            operation_id: &str,
            step: AggregateInitializationStepKind,
        ) -> AggregateInitializationOperation {
            self.store
                .mark_step_running(
                    "project_0001",
                    operation_id,
                    step,
                    format!(
                        "aggregate-init:project_0001:{operation_id}:{}:input",
                        step.as_str()
                    ),
                    STEP_AT.to_string(),
                )
                .unwrap();
            self.store
                .checkpoint_step_output(
                    "project_0001",
                    operation_id,
                    step,
                    format!(
                        "aggregate-initializations/{operation_id}/{}.json",
                        step.as_str()
                    ),
                    STEP_AT.to_string(),
                )
                .unwrap();
            self.store
                .mark_step_completed("project_0001", operation_id, step, STEP_AT.to_string())
                .unwrap()
        }

        /// Try to read the aggregate operation record through the legacy
        /// single-repository operation store; it must reject the aggregate
        /// layout rather than silently accept it.
        fn repository_operation_store_rejects_aggregate_layout(
            &self,
            operation: &AggregateInitializationOperation,
        ) -> bool {
            let legacy = RepositoryInitializationOperationStore::new(self.paths.clone());
            matches!(
                legacy.get("project_0001", &operation.operation_id),
                Err(ProductStoreError::NotFound { .. })
                    | Err(ProductStoreError::IdentityMismatch { .. })
            )
        }
    }

    /// Lightweight probe over the fixture's provider-turn counter. The store's
    /// recovery path never invokes a provider; `turn_count()` staying at 0 is
    /// the proof that `recover_interrupted` does not auto-restart.
    struct ProviderTurnProbe<'a> {
        turns: &'a std::sync::Mutex<u32>,
    }

    impl ProviderTurnProbe<'_> {
        fn turn_count(&self) -> u32 {
            *self
                .turns
                .lock()
                .expect("provider turn probe mutex poisoned")
        }
    }

    fn aggregate_init_fixture() -> AggregateInitFixture {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        let store = AggregateInitializationOperationStore::new(paths.clone());
        AggregateInitFixture {
            _temp: temp,
            paths,
            store,
            provider_turns: std::sync::Mutex::new(0),
            operation_id: "aggregate_initialization_0001".to_string(),
        }
    }

    #[test]
    fn aggregate_layout_is_exactly_five_steps_and_rejects_jump_or_reorder() {
        let fixture = aggregate_init_fixture();
        let operation = fixture
            .store()
            .create_idempotent(fixture.new_operation("request-a"))
            .unwrap();
        assert_eq!(
            operation
                .steps
                .iter()
                .map(|step| step.step_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "machine_skills",
                "aggregate_preflight",
                "pre_check",
                "rule_and_mcp_config",
                "openspec_and_examples",
            ]
        );
        fixture
            .store()
            .mark_running("project_0001", &operation.operation_id, RUNNING_AT.into())
            .unwrap();
        // No preceding step has run, so jumping to RuleAndMcpConfig must be rejected.
        assert!(
            fixture
                .store()
                .mark_step_running(
                    "project_0001",
                    &operation.operation_id,
                    AggregateInitializationStepKind::RuleAndMcpConfig,
                    "key".to_string(),
                    STEP_AT.into(),
                )
                .is_err()
        );
    }

    #[test]
    fn aggregate_operation_is_separate_from_six_step_repository_operation_and_idempotent() {
        let fixture = aggregate_init_fixture();
        let first = fixture
            .store()
            .create_idempotent(fixture.new_operation("request-a"))
            .unwrap();
        let retry = fixture
            .store()
            .create_idempotent(fixture.new_operation("request-a"))
            .unwrap();

        assert_eq!(first.operation_id, retry.operation_id);
        assert_eq!(first.steps.len(), 5);
        assert_eq!(first.operation_kind, "aggregate_initialization");
        assert!(fixture.repository_operation_store_rejects_aggregate_layout(&first));
    }

    #[test]
    fn create_idempotent_returns_conflict_when_idempotency_identity_drifts() {
        let fixture = aggregate_init_fixture();
        let first = fixture
            .store()
            .create_idempotent(fixture.new_operation("request-a"))
            .unwrap();

        // Same operation id but a different idempotency key -> conflict.
        let mut drifted = fixture.new_operation("request-a");
        drifted.input.idempotency_key = "request-b".to_string();
        let error = fixture.store().create_idempotent(drifted).unwrap_err();
        assert!(matches!(error, ProductStoreError::Conflict { .. }));

        // Same idempotency key but drifted manifest revision -> conflict.
        let mut drifted_manifest = fixture.new_operation("request-a");
        drifted_manifest.input.manifest_revision = first.input.manifest_revision + 1;
        assert!(matches!(
            fixture.store().create_idempotent(drifted_manifest),
            Err(ProductStoreError::Conflict { .. })
        ));
    }

    #[test]
    fn five_steps_must_run_in_order_with_checkpoint_before_completion() {
        let fixture = aggregate_init_fixture();
        let operation = fixture
            .store()
            .create_idempotent(fixture.new_operation("request-a"))
            .unwrap();
        fixture
            .store()
            .mark_running("project_0001", &operation.operation_id, RUNNING_AT.into())
            .unwrap();

        // Cannot complete a step without first checkpointing its output.
        let started = fixture
            .store()
            .mark_step_running(
                "project_0001",
                &operation.operation_id,
                AggregateInitializationStepKind::MachineSkills,
                "digest".to_string(),
                STEP_AT.into(),
            )
            .unwrap();
        assert!(matches!(
            fixture.store().mark_step_completed(
                "project_0001",
                &operation.operation_id,
                AggregateInitializationStepKind::MachineSkills,
                STEP_AT.into(),
            ),
            Err(ProductStoreError::IdentityMismatch { .. })
        ));
        assert_eq!(
            started.current_step,
            Some(AggregateInitializationStepKind::MachineSkills)
        );

        // Cannot jump ahead to a later step while an earlier one is pending.
        assert!(matches!(
            fixture.store().mark_step_running(
                "project_0001",
                &operation.operation_id,
                AggregateInitializationStepKind::OpenspecAndExamples,
                "digest".to_string(),
                STEP_AT.into(),
            ),
            Err(ProductStoreError::IdentityMismatch { .. })
        ));

        // Running all five in order with checkpoints succeeds.
        let mut last = started;
        for step in AggregateInitializationStepKind::V1 {
            last = fixture.run_step(&operation.operation_id, step);
        }
        assert!(last.current_step.is_none());
        assert!(
            last.steps
                .iter()
                .all(|step| step.output_artifact_ref.is_some())
        );
    }

    #[test]
    fn cancel_marks_running_step_failed_and_records_cancellation() {
        let fixture = aggregate_init_fixture();
        let operation = fixture
            .store()
            .create_idempotent(fixture.new_operation("request-a"))
            .unwrap();
        fixture
            .store()
            .mark_running("project_0001", &operation.operation_id, RUNNING_AT.into())
            .unwrap();
        // Run the two preceding deterministic steps, then start PreCheck.
        fixture.run_step(
            &operation.operation_id,
            AggregateInitializationStepKind::MachineSkills,
        );
        fixture.run_step(
            &operation.operation_id,
            AggregateInitializationStepKind::AggregatePreflight,
        );
        fixture
            .store()
            .mark_step_running(
                "project_0001",
                &operation.operation_id,
                AggregateInitializationStepKind::PreCheck,
                "digest".to_string(),
                STEP_AT.into(),
            )
            .unwrap();

        let cancelled = fixture
            .store()
            .cancel(
                "project_0001",
                &operation.operation_id,
                AggregateCancellationRecord {
                    reason_code: "user_cancelled".to_string(),
                    cancelled_at: STEP_AT.to_string(),
                    detail: None,
                },
                STEP_AT.into(),
            )
            .unwrap();
        assert_eq!(
            cancelled.status,
            AggregateInitializationOperationStatus::Cancelled
        );
        assert_eq!(
            cancelled.failed_step,
            Some(AggregateInitializationStepKind::PreCheck)
        );
        assert!(cancelled.cancellation.is_some());
        assert!(cancelled.completed_at.is_some());
    }

    #[test]
    fn recover_interrupted_fails_running_step_and_records_interrupt_error() {
        let fixture = aggregate_init_fixture();
        let operation = fixture
            .store()
            .create_idempotent(fixture.new_operation("request-a"))
            .unwrap();
        fixture
            .store()
            .mark_running("project_0001", &operation.operation_id, RUNNING_AT.into())
            .unwrap();
        fixture.run_step(
            &operation.operation_id,
            AggregateInitializationStepKind::MachineSkills,
        );
        fixture
            .store()
            .mark_step_running(
                "project_0001",
                &operation.operation_id,
                AggregateInitializationStepKind::AggregatePreflight,
                "digest".to_string(),
                STEP_AT.into(),
            )
            .unwrap();

        let recovered = fixture
            .store()
            .recover_interrupted("project_0001", &operation.operation_id, STEP_AT.into())
            .unwrap();
        assert_eq!(
            recovered.status,
            AggregateInitializationOperationStatus::Failed
        );
        assert_eq!(
            recovered.failed_step,
            Some(AggregateInitializationStepKind::AggregatePreflight)
        );
        assert_eq!(
            recovered.error.as_ref().unwrap().reason_code,
            "aggregate_initialization_interrupted"
        );
    }

    #[test]
    fn idempotency_identity_groups_relevant_fields() {
        let fixture = aggregate_init_fixture();
        let operation = fixture
            .store()
            .create_idempotent(fixture.new_operation("request-a"))
            .unwrap();
        let identity = operation.idempotency_identity();
        assert_eq!(
            identity,
            AggregateInitializationIdempotencyIdentity {
                project_id: "project_0001".to_string(),
                idempotency_key: "request-a".to_string(),
                manifest_revision: 1,
                policy_digest: "sha256:policy".to_string(),
                profile_evidence_digest: Some("sha256:profile".to_string()),
            }
        );
    }

    #[test]
    fn interrupted_provider_step_cleans_staging_and_marks_failed_without_auto_restart() {
        let fixture = aggregate_init_fixture();
        fixture.persist_running_at(AggregateInitializationStepKind::RuleAndMcpConfig);
        fixture.write_staging("rule-and-mcp/partial.json");
        assert!(fixture.staging_root().exists());

        let operation = fixture
            .store()
            .recover_interrupted(
                "project_0001",
                "aggregate_initialization_0001",
                fixture.now(),
            )
            .unwrap();

        assert_eq!(
            operation.status,
            AggregateInitializationOperationStatus::Failed
        );
        assert_eq!(
            operation.failed_step,
            Some(AggregateInitializationStepKind::RuleAndMcpConfig)
        );
        assert!(!fixture.staging_root().exists());
        assert_eq!(fixture.provider().turn_count(), 0);
    }
}
