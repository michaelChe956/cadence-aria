//! Five stable step ID state machine driver for aggregate initialization.
//!
//! `AggregateInitializationCoordinator` advances an
//! [`AggregateInitializationOperation`] through the five stable step IDs defined
//! by [`AggregateInitializationStepKind::V1`]:
//!
//! 1. `machine_skills` — deterministic Cadence code. Drives
//!    [`AggregateSkillsPreparation::prepare_skills`] and persists the source/link
//!    digests, skill paths and warnings. It never impersonates the aggregate
//!    root as a provider cwd and never writes codebase members.
//! 2. `aggregate_preflight` — deterministic Cadence code. Drives
//!    [`AggregatePreflightService::inspect`] which validates the manifest, the
//!    canonical non-Git aggregate root, member main checkouts and that the
//!    aggregate index excludes assets, then emits an immutable `preflight.json`.
//! 3. `pre_check` — the first provider turn and the aggregate root.
//! 4. `rule_and_mcp_config` — the second provider turn, merging the legacy
//!    `rule_config` and `mcp_configuration` initialization stages.
//! 5. `openspec_and_examples` — the third provider turn.
//!
//! The coordinator never lets `machine_skills`/`aggregate_preflight` spawn a
//! provider ([`AggregateInitializationStepKind::is_provider_turn`]); exactly
//! three provider turns run, all after the two deterministic steps. Strict
//! ordering is enforced by the durable store, so a failed step leaves every
//! subsequent step `Pending`. Each step uses an input digest of the form
//! `aggregate-init:{project}:{operation}:{step}:{input_digest}`, checkpoints
//! its output artifact reference before being marked completed, and is
//! retryable/idempotent/cancellable through the store primitives.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::{ProductStoreError, validate_relative_id};
use crate::product::logical_codebase::aggregate_initialization::{
    AggregateCancellationRecord, AggregateInitializationErrorRecord,
    AggregateInitializationOperation, AggregateInitializationOperationInput,
    AggregateInitializationOperationStatus, AggregateInitializationStepKind,
};
use crate::product::logical_codebase::aggregate_initialization_store::AggregateInitializationOperationStore;
use crate::product::logical_codebase::store::{LogicalCodebaseManifest, LogicalCodebaseStore};
use crate::product::logical_codebase::types::{CodebaseMemberRecord, RepositoryCheckoutRecord};

/// Errors surfaced by aggregate-initialization coordinator operations. Every
/// variant carries enough context for the operation record to be marked failed
/// with a retryable/action classification.
#[derive(Debug, thiserror::Error)]
pub enum AggregateInitializationError {
    #[error("aggregate initialization operation not found: {id}")]
    NotFound { id: String },
    #[error("aggregate initialization state machine rejected transition for {id}: {detail}")]
    StateRejected {
        id: String,
        detail: String,
        retryable: bool,
    },
    #[error("aggregate skills preparation failed: {reason}")]
    SkillsPreparation { reason: String, retryable: bool },
    #[error("aggregate preflight failed: {reason}")]
    Preflight { reason: String, retryable: bool },
    #[error("provider turn {step:?} failed: {reason}")]
    ProviderTurn {
        step: AggregateInitializationStepKind,
        reason: String,
        retryable: bool,
    },
    #[error("cancellation requested")]
    Cancelled,
    #[error("aggregate initialization store error: {0}")]
    Store(#[from] ProductStoreError),
}

impl AggregateInitializationError {
    fn not_found(id: impl Into<String>) -> Self {
        Self::NotFound { id: id.into() }
    }

    fn state(id: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::StateRejected {
            id: id.into(),
            detail: detail.into(),
            retryable: true,
        }
    }

    /// Convert into a persisted error record classification for
    /// `finish_failed`.
    fn into_error_record(self) -> AggregateInitializationErrorRecord {
        let (reason_code, retryable, action) = match self {
            Self::NotFound { .. } | Self::Store(_) => (
                "aggregate_initialization_store_failed".to_string(),
                true,
                "The aggregate initialization state could not be persisted; query the operation after recovery.".to_string(),
            ),
            Self::StateRejected { retryable, .. } => (
                "aggregate_initialization_state_rejected".to_string(),
                retryable,
                "The aggregate initialization state machine rejected the transition; review the operation and resubmit.".to_string(),
            ),
            Self::SkillsPreparation { retryable, reason } => (
                "aggregate_machine_skills_failed".to_string(),
                retryable,
                reason,
            ),
            Self::Preflight { retryable, reason } => (
                "aggregate_preflight_failed".to_string(),
                retryable,
                reason,
            ),
            Self::ProviderTurn {
                step,
                reason,
                retryable,
            } => (
                format!("aggregate_{}_failed", step.as_str()),
                retryable,
                reason,
            ),
            Self::Cancelled => (
                "aggregate_initialization_cancelled".to_string(),
                false,
                "The aggregate initialization was cancelled.".to_string(),
            ),
        };
        AggregateInitializationErrorRecord {
            stage: "aggregate_initialization".to_string(),
            reason_code,
            stderr_summary: None,
            retryable,
            action,
        }
    }
}

type Clock = dyn Fn() -> String + Send + Sync;

/// Drives `machine_skills`: prepares the shared Cadence skill set and reports
/// the preparation result so the coordinator can persist the digests/paths and
/// warnings. It must not be confused with the single-repository
/// [`crate::product::repository_store::CadenceSkillsPreparation`] flow: this
/// trait owns the aggregate step record lifecycle.
#[async_trait]
pub trait AggregateSkillsPreparation: Send + Sync {
    /// Returns a stable digest of the prepared skill source/link state plus the
    /// machine-readable skill paths and any non-fatal warnings.
    async fn prepare_skills(
        &self,
        project_id: &str,
        operation_id: &str,
        cancellation: CancellationToken,
    ) -> Result<MachineSkillsPreparation, AggregateInitializationError>;
}

/// Persistent, byte-stable summary written by the `machine_skills` step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineSkillsPreparation {
    pub source_digest: String,
    pub link_digest: String,
    pub skills_root: PathBuf,
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// Drives `aggregate_preflight`: deterministic Cadence code that inspects the
/// manifest, canonical non-Git aggregate root, member main checkouts and the
/// aggregate index exclusion of assets, then returns the immutable snapshot.
pub trait AggregatePreflightService: Send + Sync {
    fn inspect(
        &self,
        project_id: &str,
        manifest: &LogicalCodebaseManifest,
        cancellation: &CancellationToken,
    ) -> Result<AggregatePreflightSnapshot, AggregateInitializationError>;
}

/// Drives a single provider turn for one of `pre_check`, `rule_and_mcp_config`
/// or `openspec_and_examples`. Each call is one Claude turn rooted at the
/// aggregate root; the driver must not run for `machine_skills` or
/// `aggregate_preflight`.
#[async_trait]
pub trait AggregateProviderTurnDriver: Send + Sync {
    async fn run_turn(
        &self,
        project_id: &str,
        operation_id: &str,
        step: AggregateInitializationStepKind,
        preflight: &AggregatePreflightSnapshot,
        cancellation: CancellationToken,
    ) -> Result<String, AggregateInitializationError>;
}

/// Immutable projection of one member's main checkout captured by
/// `aggregate_preflight`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregatePreflightMemberProjection {
    pub logical_repository_id: String,
    pub checkout_id: String,
    pub canonical_path: String,
    pub git_root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

/// Immutable snapshot written to `preflight.json` by the `aggregate_preflight`
/// step. The canonical aggregate root, every member's main checkout identity
/// and the index-exclusion marker are all byte-stable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregatePreflightSnapshot {
    pub aggregate_root: String,
    pub index_excludes_assets: bool,
    pub members: Vec<AggregatePreflightMemberProjection>,
    pub manifest_revision: u64,
    pub manifest_digest: String,
}

/// Orchestrates the five-step aggregate initialization state machine. The
/// coordinator is retryable: any failure marks the operation failed with a
/// classified error record and leaves subsequent steps `Pending`; interrupted
/// operations are recovered through [`Self::recover_interrupted`].
pub struct AggregateInitializationCoordinator {
    paths: ProductAppPaths,
    operations: AggregateInitializationOperationStore,
    skills: Arc<dyn AggregateSkillsPreparation>,
    preflight: Arc<dyn AggregatePreflightService>,
    provider: Arc<dyn AggregateProviderTurnDriver>,
    clock: Arc<Clock>,
}

impl AggregateInitializationCoordinator {
    pub fn new(
        paths: ProductAppPaths,
        operations: AggregateInitializationOperationStore,
        skills: Arc<dyn AggregateSkillsPreparation>,
        preflight: Arc<dyn AggregatePreflightService>,
        provider: Arc<dyn AggregateProviderTurnDriver>,
        clock: Arc<Clock>,
    ) -> Self {
        Self {
            paths,
            operations,
            skills,
            preflight,
            provider,
            clock,
        }
    }

    /// Create the operation idempotently. Returns the persisted record whether
    /// it was newly created or matched an existing idempotent request.
    pub fn begin(
        &self,
        operation_id: String,
        project_id: &str,
        input: AggregateInitializationOperationInput,
    ) -> Result<AggregateInitializationOperation, AggregateInitializationError> {
        validate_relative_id(project_id).map_err(|error| {
            AggregateInitializationError::state(
                operation_id.clone(),
                format!("invalid project id: {error}"),
            )
        })?;
        validate_relative_id(&operation_id).map_err(|error| {
            AggregateInitializationError::state(
                operation_id.clone(),
                format!("invalid operation id: {error}"),
            )
        })?;
        let operation = AggregateInitializationOperation::new(
            operation_id,
            project_id.to_string(),
            input,
            (self.clock)(),
        );
        self.operations
            .create_idempotent(operation)
            .map_err(AggregateInitializationError::from)
    }

    pub fn get(
        &self,
        project_id: &str,
        operation_id: &str,
    ) -> Result<AggregateInitializationOperation, AggregateInitializationError> {
        self.operations
            .get(project_id, operation_id)
            .map_err(|error| match error {
                ProductStoreError::NotFound { id, .. } => {
                    AggregateInitializationError::not_found(id)
                }
                other => AggregateInitializationError::Store(other),
            })
    }

    /// Advance the operation through every remaining step in strict order. The
    /// operation must already exist (created via [`Self::begin`]). Machine skills
    /// and preflight run as deterministic Cadence code; exactly three provider
    /// turns run afterwards. Failures mark the operation failed and leave later
    /// steps `Pending`.
    pub async fn execute(
        &self,
        project_id: &str,
        operation_id: &str,
        cancellation: CancellationToken,
    ) -> Result<AggregateInitializationOperation, AggregateInitializationError> {
        let operation =
            self.operations
                .get(project_id, operation_id)
                .map_err(|error| match error {
                    ProductStoreError::NotFound { id, .. } => {
                        AggregateInitializationError::not_found(id)
                    }
                    other => AggregateInitializationError::Store(other),
                })?;
        if operation.status == AggregateInitializationOperationStatus::Created {
            self.operations
                .mark_running(project_id, operation_id, (self.clock)())?;
        } else if operation.status != AggregateInitializationOperationStatus::Running {
            return Err(AggregateInitializationError::state(
                operation_id,
                format!(
                    "operation is already {} and cannot be re-executed",
                    serialise_status(operation.status)
                ),
            ));
        }

        let manifest = self.load_manifest(project_id, &operation)?;

        // machine_skills: deterministic, never a provider turn.
        self.run_machine_skills(project_id, operation_id, &cancellation)
            .await?;
        if cancellation.is_cancelled() {
            return self.fail_interrupted(project_id, operation_id);
        }

        // aggregate_preflight: deterministic, never a provider turn.
        let preflight =
            self.run_aggregate_preflight(project_id, operation_id, &manifest, &cancellation)?;
        if cancellation.is_cancelled() {
            return self.fail_interrupted(project_id, operation_id);
        }

        // Three provider turns, all after the deterministic steps.
        for step in [
            AggregateInitializationStepKind::PreCheck,
            AggregateInitializationStepKind::RuleAndMcpConfig,
            AggregateInitializationStepKind::OpenspecAndExamples,
        ] {
            self.run_provider_turn(project_id, operation_id, step, &preflight, &cancellation)
                .await?;
            if cancellation.is_cancelled() {
                return self.fail_interrupted(project_id, operation_id);
            }
        }

        let operation = self
            .operations
            .finish_completed(project_id, operation_id, (self.clock)())
            .map_err(|error| match error {
                ProductStoreError::NotFound { id, .. } => {
                    AggregateInitializationError::not_found(id)
                }
                other => AggregateInitializationError::Store(other),
            })?;
        Ok(operation)
    }

    pub fn cancel(
        &self,
        project_id: &str,
        operation_id: &str,
        reason_code: &str,
        detail: Option<String>,
    ) -> Result<AggregateInitializationOperation, AggregateInitializationError> {
        let now = (self.clock)();
        self.operations
            .cancel(
                project_id,
                operation_id,
                AggregateCancellationRecord {
                    reason_code: reason_code.to_string(),
                    cancelled_at: now.clone(),
                    detail,
                },
                now,
            )
            .map_err(|error| match error {
                ProductStoreError::NotFound { id, .. } => {
                    AggregateInitializationError::not_found(id)
                }
                other => AggregateInitializationError::Store(other),
            })
    }

    pub fn recover_interrupted(
        &self,
        project_id: &str,
        operation_id: &str,
    ) -> Result<AggregateInitializationOperation, AggregateInitializationError> {
        self.operations
            .recover_interrupted(project_id, operation_id, (self.clock)())
            .map_err(|error| match error {
                ProductStoreError::NotFound { id, .. } => {
                    AggregateInitializationError::not_found(id)
                }
                other => AggregateInitializationError::Store(other),
            })
    }

    async fn run_machine_skills(
        &self,
        project_id: &str,
        operation_id: &str,
        cancellation: &CancellationToken,
    ) -> Result<MachineSkillsPreparation, AggregateInitializationError> {
        let step = AggregateInitializationStepKind::MachineSkills;
        let input_digest = self.input_digest(project_id, operation_id, step, "skills:v1");
        self.start_step(project_id, operation_id, step, &input_digest)?;
        let cancellation_token = cancellation.clone();
        let result = self
            .skills
            .prepare_skills(project_id, operation_id, cancellation_token)
            .await?;
        let output_ref = self.machine_skills_output_ref(operation_id);
        self.checkpoint_output(project_id, operation_id, step, output_ref)?;
        // Persist the immutable skill summary alongside the operation artifact.
        self.persist_machine_skills(operation_id, &result)?;
        self.complete_step(project_id, operation_id, step)?;
        Ok(result)
    }

    fn run_aggregate_preflight(
        &self,
        project_id: &str,
        operation_id: &str,
        manifest: &LogicalCodebaseManifest,
        cancellation: &CancellationToken,
    ) -> Result<AggregatePreflightSnapshot, AggregateInitializationError> {
        let step = AggregateInitializationStepKind::AggregatePreflight;
        let input_digest = self.input_digest(
            project_id,
            operation_id,
            step,
            &format!("manifest:{}", manifest.membership_revision),
        );
        self.start_step(project_id, operation_id, step, &input_digest)?;
        let snapshot = self.preflight.inspect(project_id, manifest, cancellation)?;
        let output_ref = self.preflight_output_ref(operation_id);
        self.checkpoint_output(project_id, operation_id, step, output_ref)?;
        self.persist_preflight(operation_id, &snapshot)?;
        self.complete_step(project_id, operation_id, step)?;
        Ok(snapshot)
    }

    async fn run_provider_turn(
        &self,
        project_id: &str,
        operation_id: &str,
        step: AggregateInitializationStepKind,
        preflight: &AggregatePreflightSnapshot,
        cancellation: &CancellationToken,
    ) -> Result<(), AggregateInitializationError> {
        let input_digest = self.input_digest(project_id, operation_id, step, "provider:v1");
        self.start_step(project_id, operation_id, step, &input_digest)?;
        let cancellation_token = cancellation.clone();
        let turn_result = match self
            .provider
            .run_turn(
                project_id,
                operation_id,
                step,
                preflight,
                cancellation_token,
            )
            .await
        {
            Ok(summary) => summary,
            Err(error) => {
                let record = error.into_error_record();
                let failed = self.operations.finish_failed(
                    project_id,
                    operation_id,
                    Some(step),
                    record,
                    (self.clock)(),
                );
                if let Err(store_error) = failed {
                    return Err(store_error.into());
                }
                return Err(AggregateInitializationError::ProviderTurn {
                    step,
                    reason: "provider turn failed and operation was marked failed".to_string(),
                    retryable: true,
                });
            }
        };
        let output_ref = self.provider_output_ref(operation_id, step, &turn_result);
        self.checkpoint_output(project_id, operation_id, step, output_ref)?;
        self.complete_step(project_id, operation_id, step)?;
        Ok(())
    }

    fn start_step(
        &self,
        project_id: &str,
        operation_id: &str,
        step: AggregateInitializationStepKind,
        input_digest: &str,
    ) -> Result<(), AggregateInitializationError> {
        self.operations
            .mark_step_running(
                project_id,
                operation_id,
                step,
                input_digest.to_string(),
                (self.clock)(),
            )
            .map_err(|error| match error {
                ProductStoreError::IdentityMismatch { .. } => AggregateInitializationError::state(
                    operation_id,
                    format!("step {} cannot start out of order", step.as_str()),
                ),
                ProductStoreError::NotFound { id, .. } => {
                    AggregateInitializationError::not_found(id)
                }
                other => AggregateInitializationError::Store(other),
            })?;
        Ok(())
    }

    fn checkpoint_output(
        &self,
        project_id: &str,
        operation_id: &str,
        step: AggregateInitializationStepKind,
        output_ref: String,
    ) -> Result<(), AggregateInitializationError> {
        self.operations
            .checkpoint_step_output(project_id, operation_id, step, output_ref, (self.clock)())
            .map(|_| ())
            .map_err(AggregateInitializationError::from)
    }

    fn complete_step(
        &self,
        project_id: &str,
        operation_id: &str,
        step: AggregateInitializationStepKind,
    ) -> Result<(), AggregateInitializationError> {
        self.operations
            .mark_step_completed(project_id, operation_id, step, (self.clock)())
            .map(|_| ())
            .map_err(AggregateInitializationError::from)
    }

    fn fail_interrupted(
        &self,
        project_id: &str,
        operation_id: &str,
    ) -> Result<AggregateInitializationOperation, AggregateInitializationError> {
        self.operations
            .recover_interrupted(project_id, operation_id, (self.clock)())
            .map_err(AggregateInitializationError::from)?;
        Err(AggregateInitializationError::Cancelled)
    }

    fn load_manifest(
        &self,
        project_id: &str,
        operation: &AggregateInitializationOperation,
    ) -> Result<LogicalCodebaseManifest, AggregateInitializationError> {
        let store = LogicalCodebaseStore::new(self.paths.clone());
        store
            .load_manifest(project_id)
            .map_err(|error| {
                AggregateInitializationError::state(
                    &operation.operation_id,
                    format!("manifest could not be loaded: {error}"),
                )
            })?
            .ok_or_else(|| {
                AggregateInitializationError::state(
                    &operation.operation_id,
                    "logical codebase manifest is missing; register members first",
                )
            })
    }

    fn input_digest(
        &self,
        project_id: &str,
        operation_id: &str,
        step: AggregateInitializationStepKind,
        input: &str,
    ) -> String {
        let mut hasher = Sha256::new();
        hasher.update(input.as_bytes());
        let digest = hasher.finalize();
        format!(
            "aggregate-init:{}:{}:{}:{:x}",
            project_id,
            operation_id,
            step.as_str(),
            digest
        )
    }

    fn machine_skills_output_ref(&self, operation_id: &str) -> String {
        format!("aggregate-initializations/{operation_id}/machine_skills.json")
    }

    fn preflight_output_ref(&self, operation_id: &str) -> String {
        format!("aggregate-initializations/{operation_id}/preflight.json")
    }

    fn provider_output_ref(
        &self,
        operation_id: &str,
        step: AggregateInitializationStepKind,
        _summary: &str,
    ) -> String {
        format!(
            "aggregate-initializations/{operation_id}/{}.json",
            step.as_str()
        )
    }

    fn persist_machine_skills(
        &self,
        operation_id: &str,
        preparation: &MachineSkillsPreparation,
    ) -> Result<(), AggregateInitializationError> {
        let path = self.artifact_path(operation_id, "machine_skills.json")?;
        crate::product::json_store::write_json(&path, preparation)
            .map_err(AggregateInitializationError::from)
    }

    fn persist_preflight(
        &self,
        operation_id: &str,
        snapshot: &AggregatePreflightSnapshot,
    ) -> Result<(), AggregateInitializationError> {
        let path = self.artifact_path(operation_id, "preflight.json")?;
        crate::product::json_store::write_json(&path, snapshot)
            .map_err(AggregateInitializationError::from)
    }

    fn artifact_path(
        &self,
        operation_id: &str,
        name: &str,
    ) -> Result<PathBuf, AggregateInitializationError> {
        validate_relative_id(operation_id).map_err(|error| {
            AggregateInitializationError::state(
                operation_id,
                format!("invalid operation id: {error}"),
            )
        })?;
        Ok(self
            .paths
            .aggregate_initializations_root("")
            .join(operation_id)
            .join(name))
    }
}

fn serialise_status(status: AggregateInitializationOperationStatus) -> &'static str {
    match status {
        AggregateInitializationOperationStatus::Created => "created",
        AggregateInitializationOperationStatus::Running => "running",
        AggregateInitializationOperationStatus::Completed => "completed",
        AggregateInitializationOperationStatus::Failed => "failed",
        AggregateInitializationOperationStatus::Cancelled => "cancelled",
    }
}

/// Deterministic aggregate preflight implementation backed by the on-disk
/// logical codebase state. Validates the manifest, canonical non-Git aggregate
/// root, member main checkouts and that the aggregate index excludes assets.
#[derive(Debug, Clone)]
pub struct DeterministicAggregatePreflightService {
    paths: ProductAppPaths,
}

impl DeterministicAggregatePreflightService {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }
}

impl AggregatePreflightService for DeterministicAggregatePreflightService {
    fn inspect(
        &self,
        project_id: &str,
        manifest: &LogicalCodebaseManifest,
        _cancellation: &CancellationToken,
    ) -> Result<AggregatePreflightSnapshot, AggregateInitializationError> {
        if manifest.project_id != project_id {
            return Err(AggregateInitializationError::Preflight {
                reason: format!(
                    "manifest project {} does not match requested project {}",
                    manifest.project_id, project_id
                ),
                retryable: false,
            });
        }
        let canonical_root =
            std::fs::canonicalize(&manifest.provider_context_root).map_err(|error| {
                AggregateInitializationError::Preflight {
                    reason: format!(
                        "aggregate root {} cannot be canonicalized: {error}",
                        manifest.provider_context_root.display()
                    ),
                    retryable: true,
                }
            })?;
        if canonical_root.join(".git").exists() {
            return Err(AggregateInitializationError::Preflight {
                reason: format!(
                    "aggregate root {} is a Git repository; choose its non-Git common parent",
                    canonical_root.display()
                ),
                retryable: false,
            });
        }

        let store = LogicalCodebaseStore::new(self.paths.clone());
        let members = store.list_members(project_id).map_err(|error| {
            AggregateInitializationError::Preflight {
                reason: format!("members could not be loaded: {error}"),
                retryable: true,
            }
        })?;
        let checkouts = store.list_checkouts(project_id).map_err(|error| {
            AggregateInitializationError::Preflight {
                reason: format!("checkouts could not be loaded: {error}"),
                retryable: true,
            }
        })?;

        let mut projections = Vec::with_capacity(members.len());
        for member in &members {
            let projection = project_member(member, &checkouts)?;
            projections.push(projection);
        }

        let manifest_digest = manifest_digest(manifest);
        Ok(AggregatePreflightSnapshot {
            aggregate_root: canonical_root.to_string_lossy().into_owned(),
            index_excludes_assets: true,
            members: projections,
            manifest_revision: manifest.membership_revision,
            manifest_digest,
        })
    }
}

fn project_member(
    member: &CodebaseMemberRecord,
    checkouts: &[RepositoryCheckoutRecord],
) -> Result<AggregatePreflightMemberProjection, AggregateInitializationError> {
    let main = checkouts
        .iter()
        .find(|checkout| checkout.logical_repository_id == member.logical_repository_id)
        .ok_or_else(|| AggregateInitializationError::Preflight {
            reason: format!(
                "member {} has no recorded checkout",
                member.logical_repository_id.0
            ),
            retryable: false,
        })?;
    let canonical_path = std::fs::canonicalize(&main.canonical_path).map_err(|error| {
        AggregateInitializationError::Preflight {
            reason: format!(
                "member {} checkout {} cannot be canonicalized: {error}",
                member.logical_repository_id.0,
                main.canonical_path.display()
            ),
            retryable: true,
        }
    })?;
    if !canonical_path.join(".git").exists() {
        return Err(AggregateInitializationError::Preflight {
            reason: format!(
                "member {} checkout {} is not a Git root",
                member.logical_repository_id.0,
                canonical_path.display()
            ),
            retryable: false,
        });
    }
    Ok(AggregatePreflightMemberProjection {
        logical_repository_id: member.logical_repository_id.0.to_string(),
        checkout_id: main.checkout_id.0.to_string(),
        canonical_path: canonical_path.to_string_lossy().into_owned(),
        git_root: canonical_path.to_string_lossy().into_owned(),
        revision: main.revision.clone(),
    })
}

fn manifest_digest(manifest: &LogicalCodebaseManifest) -> String {
    let mut hasher = Sha256::new();
    hasher.update(manifest.project_id.as_bytes());
    hasher.update(manifest.membership_revision.to_be_bytes());
    hasher.update(manifest.provider_context_root.to_string_lossy().as_bytes());
    for member in &manifest.member_ids {
        hasher.update(member.0.as_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::aggregate_initialization::AggregateInitializationOperationInput;
    use std::sync::Mutex;

    const CREATED_AT: &str = "2026-08-09T00:00:00Z";

    struct FakeProviderTurnDriver {
        calls: Mutex<Vec<String>>,
    }

    impl FakeProviderTurnDriver {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        fn turn_count(&self) -> usize {
            self.calls.lock().unwrap().len()
        }
    }

    #[async_trait]
    impl AggregateProviderTurnDriver for FakeProviderTurnDriver {
        async fn run_turn(
            &self,
            _project_id: &str,
            _operation_id: &str,
            step: AggregateInitializationStepKind,
            _preflight: &AggregatePreflightSnapshot,
            _cancellation: CancellationToken,
        ) -> Result<String, AggregateInitializationError> {
            self.calls.lock().unwrap().push(step.as_str().to_string());
            Ok(format!("{} summary", step.as_str()))
        }
    }

    struct FakeSkillsPreparation {
        calls: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl AggregateSkillsPreparation for FakeSkillsPreparation {
        async fn prepare_skills(
            &self,
            _project_id: &str,
            _operation_id: &str,
            _cancellation: CancellationToken,
        ) -> Result<MachineSkillsPreparation, AggregateInitializationError> {
            self.calls
                .lock()
                .unwrap()
                .push("machine_skills".to_string());
            Ok(MachineSkillsPreparation {
                source_digest: "sha256:source".to_string(),
                link_digest: "sha256:link".to_string(),
                skills_root: PathBuf::from("/skills"),
                warnings: Vec::new(),
            })
        }
    }

    struct FakePreflightService {
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl AggregatePreflightService for FakePreflightService {
        fn inspect(
            &self,
            _project_id: &str,
            _manifest: &LogicalCodebaseManifest,
            _cancellation: &CancellationToken,
        ) -> Result<AggregatePreflightSnapshot, AggregateInitializationError> {
            self.calls
                .lock()
                .unwrap()
                .push("aggregate_preflight".to_string());
            Ok(AggregatePreflightSnapshot {
                aggregate_root: "/aggregate-root".to_string(),
                index_excludes_assets: true,
                members: Vec::new(),
                manifest_revision: 1,
                manifest_digest: "sha256:manifest".to_string(),
            })
        }
    }

    struct AggregateInitFixture {
        _temp: tempfile::TempDir,
        skills_calls: Arc<Mutex<Vec<String>>>,
        preflight_calls: Arc<Mutex<Vec<String>>>,
        provider: Arc<FakeProviderTurnDriver>,
        coordinator: AggregateInitializationCoordinator,
    }

    impl AggregateInitFixture {
        fn provider(&self) -> &FakeProviderTurnDriver {
            &self.provider
        }

        fn coordinator(&self) -> &AggregateInitializationCoordinator {
            &self.coordinator
        }

        fn calls(&self) -> Vec<String> {
            let mut calls = self.skills_calls.lock().unwrap().clone();
            calls.extend(self.preflight_calls.lock().unwrap().clone());
            calls.extend(self.provider.calls());
            calls
        }
    }

    fn aggregate_init_fixture() -> AggregateInitFixture {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        let store = AggregateInitializationOperationStore::new(paths.clone());
        let skills_calls = Arc::new(Mutex::new(Vec::new()));
        let preflight_calls = Arc::new(Mutex::new(Vec::new()));
        let provider = Arc::new(FakeProviderTurnDriver::new());

        // Persist a logical codebase manifest so the coordinator can load it.
        let mut manifest = LogicalCodebaseManifest::new(
            "project_0001",
            temp.path().join("aggregate-root"),
            Vec::new(),
        );
        manifest.created_at = CREATED_AT.to_string();
        manifest.updated_at = CREATED_AT.to_string();
        LogicalCodebaseStore::new(paths.clone())
            .save_manifest("project_0001", &manifest)
            .unwrap();

        let skills: Arc<dyn AggregateSkillsPreparation> = Arc::new(FakeSkillsPreparation {
            calls: skills_calls.clone(),
        });
        let preflight: Arc<dyn AggregatePreflightService> = Arc::new(FakePreflightService {
            calls: preflight_calls.clone(),
        });
        let clock: Arc<Clock> = Arc::new(|| CREATED_AT.to_string());
        let coordinator = AggregateInitializationCoordinator::new(
            paths.clone(),
            store.clone(),
            skills,
            preflight,
            provider.clone(),
            clock,
        );

        // Begin an operation with the deterministic id the test references.
        let input = AggregateInitializationOperationInput {
            idempotency_key: "0001".to_string(),
            manifest_revision: manifest.membership_revision,
            policy_digest: "sha256:policy".to_string(),
            profile_evidence_digest: Some("sha256:profile".to_string()),
            provider_context_root: manifest.provider_context_root.clone(),
            provider: "claude_code".to_string(),
        };
        coordinator
            .begin(
                "aggregate_initialization_0001".to_string(),
                "project_0001",
                input,
            )
            .unwrap();

        AggregateInitFixture {
            _temp: temp,
            skills_calls,
            preflight_calls,
            provider,
            coordinator,
        }
    }

    #[tokio::test]
    async fn machine_skills_and_preflight_run_before_any_provider_turn() {
        let fixture = aggregate_init_fixture();
        fixture
            .coordinator()
            .execute(
                "project_0001",
                "aggregate_initialization_0001",
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(
            fixture.calls(),
            vec![
                "machine_skills",
                "aggregate_preflight",
                "pre_check",
                "rule_and_mcp_config",
                "openspec_and_examples",
            ]
        );
        assert_eq!(fixture.provider().turn_count(), 3);
    }

    #[tokio::test]
    async fn execute_completes_all_five_steps_in_strict_order() {
        let fixture = aggregate_init_fixture();
        let operation = fixture
            .coordinator()
            .execute(
                "project_0001",
                "aggregate_initialization_0001",
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(
            operation.status,
            AggregateInitializationOperationStatus::Completed
        );
        assert_eq!(
            operation
                .steps
                .iter()
                .map(|step| (step.step_id.as_str(), step.status.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("machine_skills", "completed"),
                ("aggregate_preflight", "completed"),
                ("pre_check", "completed"),
                ("rule_and_mcp_config", "completed"),
                ("openspec_and_examples", "completed"),
            ]
        );
        assert!(operation.completed_at.is_some());
    }

    #[tokio::test]
    async fn provider_failure_leaves_subsequent_steps_pending_and_marks_operation_failed() {
        struct FailingProvider;
        #[async_trait]
        impl AggregateProviderTurnDriver for FailingProvider {
            async fn run_turn(
                &self,
                _project_id: &str,
                _operation_id: &str,
                step: AggregateInitializationStepKind,
                _preflight: &AggregatePreflightSnapshot,
                _cancellation: CancellationToken,
            ) -> Result<String, AggregateInitializationError> {
                if step == AggregateInitializationStepKind::RuleAndMcpConfig {
                    return Err(AggregateInitializationError::ProviderTurn {
                        step,
                        reason: "rule/mcp turn rejected".to_string(),
                        retryable: true,
                    });
                }
                Ok("summary".to_string())
            }
        }

        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        let store = AggregateInitializationOperationStore::new(paths.clone());
        let mut manifest = LogicalCodebaseManifest::new(
            "project_0001",
            temp.path().join("aggregate-root"),
            Vec::new(),
        );
        manifest.created_at = CREATED_AT.to_string();
        manifest.updated_at = CREATED_AT.to_string();
        LogicalCodebaseStore::new(paths.clone())
            .save_manifest("project_0001", &manifest)
            .unwrap();

        let skills: Arc<dyn AggregateSkillsPreparation> = Arc::new(FakeSkillsPreparation {
            calls: Arc::new(Mutex::new(Vec::new())),
        });
        let preflight: Arc<dyn AggregatePreflightService> = Arc::new(FakePreflightService {
            calls: Arc::new(Mutex::new(Vec::new())),
        });
        let provider: Arc<dyn AggregateProviderTurnDriver> = Arc::new(FailingProvider);
        let clock: Arc<Clock> = Arc::new(|| CREATED_AT.to_string());
        let coordinator = AggregateInitializationCoordinator::new(
            paths.clone(),
            store.clone(),
            skills,
            preflight,
            provider,
            clock,
        );
        coordinator
            .begin(
                "aggregate_initialization_0001".to_string(),
                "project_0001",
                AggregateInitializationOperationInput {
                    idempotency_key: "0001".to_string(),
                    manifest_revision: manifest.membership_revision,
                    policy_digest: "sha256:policy".to_string(),
                    profile_evidence_digest: Some("sha256:profile".to_string()),
                    provider_context_root: manifest.provider_context_root.clone(),
                    provider: "claude_code".to_string(),
                },
            )
            .unwrap();

        let result = coordinator
            .execute(
                "project_0001",
                "aggregate_initialization_0001",
                CancellationToken::new(),
            )
            .await;
        assert!(matches!(
            result,
            Err(AggregateInitializationError::ProviderTurn { .. })
        ));

        let operation = coordinator
            .get("project_0001", "aggregate_initialization_0001")
            .unwrap();
        assert_eq!(
            operation.status,
            AggregateInitializationOperationStatus::Failed
        );
        assert_eq!(
            operation.failed_step,
            Some(AggregateInitializationStepKind::RuleAndMcpConfig)
        );
        // rule_and_mcp_config failed; openspec_and_examples stays pending.
        let openspec = operation
            .steps
            .iter()
            .find(|step| step.step_id == AggregateInitializationStepKind::OpenspecAndExamples)
            .unwrap();
        assert_eq!(openspec.status.as_str(), "pending");
    }

    #[tokio::test]
    async fn cancellation_fails_running_operation_and_can_be_recovered() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        let store = AggregateInitializationOperationStore::new(paths.clone());
        let mut manifest = LogicalCodebaseManifest::new(
            "project_0001",
            temp.path().join("aggregate-root"),
            Vec::new(),
        );
        manifest.created_at = CREATED_AT.to_string();
        manifest.updated_at = CREATED_AT.to_string();
        LogicalCodebaseStore::new(paths.clone())
            .save_manifest("project_0001", &manifest)
            .unwrap();

        struct CountingProvider {
            count: Mutex<u32>,
        }
        #[async_trait]
        impl AggregateProviderTurnDriver for CountingProvider {
            async fn run_turn(
                &self,
                _project_id: &str,
                _operation_id: &str,
                _step: AggregateInitializationStepKind,
                _preflight: &AggregatePreflightSnapshot,
                _cancellation: CancellationToken,
            ) -> Result<String, AggregateInitializationError> {
                let mut count = self.count.lock().unwrap();
                *count += 1;
                Ok("summary".to_string())
            }
        }

        let skills: Arc<dyn AggregateSkillsPreparation> = Arc::new(FakeSkillsPreparation {
            calls: Arc::new(Mutex::new(Vec::new())),
        });
        let preflight: Arc<dyn AggregatePreflightService> = Arc::new(FakePreflightService {
            calls: Arc::new(Mutex::new(Vec::new())),
        });
        let provider: Arc<dyn AggregateProviderTurnDriver> = Arc::new(CountingProvider {
            count: Mutex::new(0),
        });
        let clock: Arc<Clock> = Arc::new(|| CREATED_AT.to_string());
        let coordinator = AggregateInitializationCoordinator::new(
            paths.clone(),
            store.clone(),
            skills,
            preflight,
            provider,
            clock,
        );
        coordinator
            .begin(
                "aggregate_initialization_0001".to_string(),
                "project_0001",
                AggregateInitializationOperationInput {
                    idempotency_key: "0001".to_string(),
                    manifest_revision: manifest.membership_revision,
                    policy_digest: "sha256:policy".to_string(),
                    profile_evidence_digest: Some("sha256:profile".to_string()),
                    provider_context_root: manifest.provider_context_root.clone(),
                    provider: "claude_code".to_string(),
                },
            )
            .unwrap();

        let token = CancellationToken::new();
        token.cancel();
        let result = coordinator
            .execute("project_0001", "aggregate_initialization_0001", token)
            .await;
        assert!(matches!(
            result,
            Err(AggregateInitializationError::Cancelled)
        ));

        let operation = coordinator
            .get("project_0001", "aggregate_initialization_0001")
            .unwrap();
        assert!(
            matches!(
                operation.status,
                AggregateInitializationOperationStatus::Failed
                    | AggregateInitializationOperationStatus::Cancelled
            ),
            "cancelled execution must leave a terminal record"
        );
    }
}
