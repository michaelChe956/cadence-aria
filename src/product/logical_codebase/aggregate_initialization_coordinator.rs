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
    AggregateInitializationOperationStatus, AggregateInitializationProfile,
    AggregateInitializationStepKind, RepositoryTypeEvidence,
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

/// Detects the repository type for one member checkout root without invoking
/// package managers, build tools, package scripts, `pnpm install`, Node or
/// Java. The detector only reads the member main checkout root (no recursion,
/// no symlink following outside the root).
pub trait RepositoryTypeDetector: Send + Sync {
    fn detect(
        &self,
        checkout_root: &std::path::Path,
        logical_repository_id: &str,
    ) -> Result<RepositoryTypeEvidence, AggregateInitializationError>;
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
    detector: Arc<dyn RepositoryTypeDetector>,
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
        Self::with_detector(
            paths,
            operations,
            skills,
            preflight,
            provider,
            Arc::new(DeterministicRepositoryTypeDetector::new()),
            clock,
        )
    }

    /// Construct a coordinator with an explicit repository-type detector,
    /// allowing tests and future integrations to override profile detection
    /// while keeping the five stable step IDs unchanged.
    pub fn with_detector(
        paths: ProductAppPaths,
        operations: AggregateInitializationOperationStore,
        skills: Arc<dyn AggregateSkillsPreparation>,
        preflight: Arc<dyn AggregatePreflightService>,
        provider: Arc<dyn AggregateProviderTurnDriver>,
        detector: Arc<dyn RepositoryTypeDetector>,
        clock: Arc<Clock>,
    ) -> Self {
        Self {
            paths,
            operations,
            skills,
            preflight,
            provider,
            detector,
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

    /// Resolve the aggregate initialization profile from read-only member
    /// checkout signals. The detector only reads each member's main checkout
    /// root; it never recurses, follows symlinks outside the root, executes
    /// package scripts, runs `pnpm install`, Node or Java. The five stable
    /// step IDs are unaffected — only the template/precheck selection changes.
    pub fn preflight_profile(
        &self,
        project_id: &str,
    ) -> Result<AggregateInitializationProfile, AggregateInitializationError> {
        validate_relative_id(project_id).map_err(|error| {
            AggregateInitializationError::state(project_id, format!("invalid project id: {error}"))
        })?;
        let _manifest = LogicalCodebaseStore::new(self.paths.clone())
            .load_manifest(project_id)
            .map_err(|error| AggregateInitializationError::Preflight {
                reason: format!("manifest could not be loaded: {error}"),
                retryable: true,
            })?
            .ok_or_else(|| AggregateInitializationError::Preflight {
                reason: "logical codebase manifest is missing; register members first".to_string(),
                retryable: false,
            })?;
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
        let mut evidence = Vec::with_capacity(members.len());
        for member in &members {
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
            let detected = self.detector.detect(
                &main.canonical_path,
                &member.logical_repository_id.0.to_string(),
            )?;
            evidence.push(detected);
        }
        resolve_aggregate_profile(&evidence)
    }

    /// Profile-specific preflight command templates for the resolved profile.
    /// Frontend pnpm/Vite never includes Maven/Gradle commands.
    pub fn preflight_commands(
        &self,
        project_id: &str,
    ) -> Result<Vec<String>, AggregateInitializationError> {
        let profile = self.preflight_profile(project_id)?;
        Ok(profile_preflight_commands(profile))
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

/// 默认托管配置 artifact 引用,经 gateway envelope 的 config_digest 复验。
const AGGREGATE_CONFIG_ARTIFACT_REF: &str = "sha256:aggregate-initialization-managed-config";

/// 聚合根在 envelope target 中使用的稳定逻辑标识。聚合 provider turn 的 cwd
/// 是 canonical non-Git aggregate root(聚合根本身,非任何成员 checkout);此处用
/// 一个 `aggregate-root` 占位标识表示「整个聚合根」,而非单个成员仓库。
const AGGREGATE_ROOT_LOGICAL_REPO: &str = "aggregate-root";
const AGGREGATE_ROOT_CHECKOUT: &str = "aggregate-root";

/// Task 16:gateway-backed provider turn 驱动。
///
/// 三个 provider turn(`pre_check`/`rule_and_mcp_config`/`openspec_and_examples`)
/// 经 [`LogicalCodebaseProviderGateway::start_streaming`] 启动(feature gate):
/// 当注入此驱动作为 coordinator 的 `AggregateProviderTurnDriver` 时,每个 turn
/// 都会在共享的 [`GatewayRunAudit`] 累加一次 `stream_launches()` 记录,使「聚合
/// provider turn 唯一经 gateway 启动」成为可审计事实而非仅靠代码审查。
///
/// 聚合根是 canonical non-Git aggregate root(聚合初始化 envelope 配置);三个
/// turn 的 cwd 均为该根,配置来自托管配置 artifact。该驱动绝不依赖单仓持久化
/// 层、单仓注册协调器或单仓 git 终结点,故聚合模式不会进入成员仓 git 调用图。
/// 该隔离契约由 `aggregate_coordinator_isolation` 测试在编译期锁定。
pub struct GatewayBackedAggregateProviderTurnDriver {
    gateway: Arc<crate::product::logical_codebase::LogicalCodebaseProviderGateway>,
    provider: crate::product::logical_codebase::ProviderRef,
}

impl GatewayBackedAggregateProviderTurnDriver {
    /// 用 Claude Code dialect 与给定 capability snapshot ref 构造驱动。聚合
    /// 初始化当前固定使用 Claude Code 作为唯一逻辑 provider(Codex 在
    /// `danger-full-access` 下被 gateway 路由级阻断)。
    pub fn claude_code(
        gateway: Arc<crate::product::logical_codebase::LogicalCodebaseProviderGateway>,
        capability_snapshot_ref: impl Into<String>,
    ) -> Self {
        Self {
            gateway,
            provider: crate::product::logical_codebase::ProviderRef::claude_code(
                capability_snapshot_ref,
            ),
        }
    }

    /// 把单个 provider turn 组装成经 gateway 启动的 streaming 请求。read-only
    /// planning action(聚合根不写成员仓),target 锚定聚合根本身。
    fn launch_request(
        &self,
        project_id: &str,
        aggregate_root: &std::path::Path,
    ) -> crate::product::logical_codebase::SessionLaunchRequest {
        use crate::product::logical_codebase::{PolicyTarget, SessionPolicyAction};
        let target = PolicyTarget::checkout(
            AGGREGATE_ROOT_LOGICAL_REPO,
            AGGREGATE_ROOT_CHECKOUT,
            aggregate_root.to_path_buf(),
        );
        crate::product::logical_codebase::SessionLaunchRequest {
            project_id: project_id.to_string(),
            provider: self.provider.clone(),
            action: SessionPolicyAction::PlanningReadOnly,
            target,
            readable_roots: vec![aggregate_root.to_path_buf()],
            writable_roots: Vec::new(),
            config_artifact_ref: AGGREGATE_CONFIG_ARTIFACT_REF.to_string(),
        }
    }

    fn streaming_input(
        &self,
        step: AggregateInitializationStepKind,
        aggregate_root: &std::path::Path,
    ) -> crate::cross_cutting::streaming_provider::StreamingProviderInput {
        use crate::cross_cutting::streaming_provider::{
            ProviderPermissionMode, StreamingProviderInput,
        };
        use crate::protocol::contracts::{AdapterRole, ProviderType};
        StreamingProviderInput {
            provider_type: ProviderType::ClaudeCode,
            role: AdapterRole::Executor,
            prompt: format!("aggregate initialization turn: {}", step.as_str()),
            working_dir: aggregate_root.to_path_buf(),
            workspace_session_id: None,
            resume_provider_session_id: None,
            permission_mode: ProviderPermissionMode::Auto,
            structured_output_contract: None,
            env_vars: std::collections::BTreeMap::new(),
            timeout_secs: 1,
        }
    }
}

#[async_trait]
impl AggregateProviderTurnDriver for GatewayBackedAggregateProviderTurnDriver {
    async fn run_turn(
        &self,
        project_id: &str,
        _operation_id: &str,
        step: AggregateInitializationStepKind,
        preflight: &AggregatePreflightSnapshot,
        cancellation: CancellationToken,
    ) -> Result<String, AggregateInitializationError> {
        use crate::cross_cutting::session_launch::ValidatedStreamingProviderInput;
        let aggregate_root = std::path::PathBuf::from(&preflight.aggregate_root);
        let request = self.launch_request(project_id, &aggregate_root);
        let validated = self.gateway.validate(request).map_err(|error| {
            AggregateInitializationError::ProviderTurn {
                step,
                reason: format!("gateway validate failed: {error}"),
                retryable: true,
            }
        })?;
        let input = self.streaming_input(step, &aggregate_root);
        let launch = ValidatedStreamingProviderInput::new(input, validated);
        self.gateway
            .start_streaming(launch, cancellation)
            .await
            .map_err(|error| AggregateInitializationError::ProviderTurn {
                step,
                reason: format!("gateway start_streaming failed: {error}"),
                retryable: true,
            })?;
        Ok(format!("{} via gateway", step.as_str()))
    }
}

/// Task 16:聚合 asset 发布器。三个 provider turn 产出的聚合 artifact 只允许发布到
/// `.aria/aggregate/**`,禁止任何成员仓路径,使「聚合模式不进成员仓 git」成为可
/// 验证契约。发布的相对路径以正斜杠分隔;`published_paths()` 返回发布顺序供审计。
#[derive(Debug, Default)]
pub struct AggregateAssetPublisher {
    published: std::sync::Mutex<Vec<String>>,
}

impl AggregateAssetPublisher {
    pub fn new() -> Self {
        Self::default()
    }

    /// 发布一个聚合 asset 的相对路径。只允许 `.aria/aggregate/**`;其余路径
    /// (成员仓、父目录逃逸、绝对路径)一律 fail-closed。
    pub fn publish(
        &self,
        operation_id: &str,
        relative_path: &str,
    ) -> Result<(), AggregateInitializationError> {
        validate_relative_id(operation_id).map_err(|error| {
            AggregateInitializationError::state(
                operation_id,
                format!("invalid operation id: {error}"),
            )
        })?;
        if !Self::is_aggregate_asset_path(relative_path) {
            return Err(AggregateInitializationError::state(
                operation_id,
                format!("aggregate asset publisher rejects non-aggregate path: {relative_path}"),
            ));
        }
        self.published
            .lock()
            .expect("aggregate asset publisher mutex poisoned")
            .push(relative_path.to_string());
        Ok(())
    }

    /// 已发布的聚合 asset 相对路径,按发布顺序。
    pub fn published_paths(&self) -> Vec<String> {
        self.published
            .lock()
            .expect("aggregate asset publisher mutex poisoned")
            .clone()
    }

    /// 判定相对路径是否落在 `.aria/aggregate/**` 内。必须以 `.aria/aggregate/` 起
    /// 头,禁止空、禁止 `..` 段、禁止绝对路径前缀。
    fn is_aggregate_asset_path(relative_path: &str) -> bool {
        if relative_path.is_empty() {
            return false;
        }
        let normalized = std::path::Path::new(relative_path);
        if normalized.is_absolute() {
            return false;
        }
        if normalized.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::RootDir
            )
        }) {
            return false;
        }
        let mut components = normalized.components();
        matches!(components.next(), Some(std::path::Component::Normal(a)) if a == ".aria")
            && matches!(components.next(), Some(std::path::Component::Normal(b)) if b == "aggregate")
            && components.next().is_some()
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

/// Deterministic repository-type detector backed by
/// [`crate::product::logical_codebase::RepositoryProfileDetector`]. Only reads
/// the member main checkout root: it never recurses, follows symlinks outside
/// the root, executes package scripts, runs `pnpm install`, Node or Java. The
/// evidence digest makes the observation byte-stable for the preflight record.
#[derive(Debug, Clone)]
pub struct DeterministicRepositoryTypeDetector;

impl DeterministicRepositoryTypeDetector {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DeterministicRepositoryTypeDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl RepositoryTypeDetector for DeterministicRepositoryTypeDetector {
    fn detect(
        &self,
        checkout_root: &std::path::Path,
        logical_repository_id: &str,
    ) -> Result<RepositoryTypeEvidence, AggregateInitializationError> {
        let profile =
            crate::product::logical_codebase::RepositoryProfileDetector::detect(checkout_root)
                .map_err(|error| AggregateInitializationError::Preflight {
                    reason: format!(
                        "repository type detection failed for {}: {error}",
                        checkout_root.display()
                    ),
                    retryable: true,
                })?;
        let profile_digest = evidence_digest(logical_repository_id, &profile.tech_stack);
        Ok(RepositoryTypeEvidence {
            logical_repository_id: logical_repository_id.to_string(),
            repo_type: profile.repo_type,
            tech_stack: profile.tech_stack,
            profile_digest: Some(profile_digest),
        })
    }
}

fn evidence_digest(logical_repository_id: &str, tech_stack: &[String]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(logical_repository_id.as_bytes());
    hasher.update(tech_stack.join(",").as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

/// Resolve the aggregate initialization profile from the per-member evidence.
///
/// - All members `Backend` (Java/Maven/Gradle) → `JavaBackend`.
/// - All members `Frontend` (pnpm/Vite) → `FrontendPnpmVite`.
/// - A mix of both (or `Mixed`) → `Mixed`.
/// - Any `Unknown` member, or a profile that cannot be classified within the
///   requested scope, fails preflight closed.
pub fn resolve_aggregate_profile(
    evidence: &[RepositoryTypeEvidence],
) -> Result<AggregateInitializationProfile, AggregateInitializationError> {
    use crate::product::logical_codebase::types::RepositoryType;
    if evidence.is_empty() {
        return Err(AggregateInitializationError::Preflight {
            reason: "aggregate profile cannot be resolved without any members".to_string(),
            retryable: false,
        });
    }
    let mut any_backend = false;
    let mut any_frontend = false;
    for item in evidence {
        match item.repo_type {
            RepositoryType::Backend => any_backend = true,
            RepositoryType::Frontend => any_frontend = true,
            RepositoryType::Mixed => {
                any_backend = true;
                any_frontend = true;
            }
            RepositoryType::Library => {
                // A pure library member does not by itself force a profile; it
                // stays neutral and lets the backend/frontend members decide.
            }
            RepositoryType::Unknown => {
                return Err(AggregateInitializationError::Preflight {
                    reason: format!(
                        "member {} has an unknown repository type; profile cannot be classified",
                        item.logical_repository_id
                    ),
                    retryable: false,
                });
            }
        }
    }
    Ok(match (any_backend, any_frontend) {
        (true, false) => AggregateInitializationProfile::JavaBackend,
        (false, true) => AggregateInitializationProfile::FrontendPnpmVite,
        (false, false) => AggregateInitializationProfile::FrontendPnpmVite,
        (true, true) => AggregateInitializationProfile::Mixed,
    })
}

/// Profile-specific preflight command templates. Frontend pnpm/Vite never
/// includes Maven/Gradle commands; the Java/Mixed templates carry the Java
/// build commands. The five stable step IDs are unaffected.
pub fn profile_preflight_commands(profile: AggregateInitializationProfile) -> Vec<String> {
    match profile {
        AggregateInitializationProfile::FrontendPnpmVite => vec![
            "pnpm --version".to_string(),
            "pnpm exec vite --version".to_string(),
        ],
        AggregateInitializationProfile::JavaBackend => vec![
            "mvn -v".to_string(),
            "git rev-parse --show-toplevel".to_string(),
        ],
        AggregateInitializationProfile::Mixed => vec![
            "pnpm --version".to_string(),
            "mvn -v".to_string(),
            "git rev-parse --show-toplevel".to_string(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate;
    use crate::cross_cutting::provider_registry::ProviderRegistry;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::aggregate_initialization::AggregateInitializationOperationInput;
    use crate::product::logical_codebase::provider_gateway::ResumeEvidenceState;
    use crate::product::logical_codebase::types::LogicalRepositoryId;
    use crate::product::logical_codebase::{
        CodebaseMemberRecord, GatewayRunAudit, LogicalCodebaseProviderGateway,
        LogicalCodebaseStore, PolicyTarget, PolicyTargetResolver, ProviderCapability,
        ProviderCapabilitySource, ProviderDialect, ProviderGatewayError, ProviderRef,
        ProviderRefType, RepositoryCheckoutId, RepositoryCheckoutRecord, RepositorySourceIdentity,
        SessionLaunchRequest, SessionPolicyAction,
    };
    use crate::product::models::ProviderName;
    use std::path::Path;
    use std::sync::Mutex;
    use uuid::Uuid;

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

    // ---- Task 16: provider step 经 gateway 启动 + GitFinalize 调用图切断 ----

    /// 测试用 capability source:固定 Claude Code capability,version 与 resume
    /// 能力可调,用于 gateway 复验。聚合 provider turn 固定 Claude Code(Codex
    /// danger-full-access 被 gateway 路由级阻断)。
    struct StaticCapabilitySource {
        version: Mutex<String>,
        resume: Mutex<ResumeEvidenceState>,
    }

    impl StaticCapabilitySource {
        fn new(version: &str) -> Self {
            Self {
                version: Mutex::new(version.to_string()),
                resume: Mutex::new(ResumeEvidenceState::Confirmed),
            }
        }
    }

    impl ProviderCapabilitySource for StaticCapabilitySource {
        fn require_supported(
            &self,
            provider: &ProviderRef,
            _action: SessionPolicyAction,
        ) -> Result<ProviderCapability, ProviderGatewayError> {
            Ok(ProviderCapability {
                provider_type: provider.provider_type,
                version: self.version.lock().unwrap().clone(),
                adapter_dialect: match provider.provider_type {
                    ProviderRefType::ClaudeCode => ProviderDialect::ClaudeCodeCliV1,
                    ProviderRefType::Codex => ProviderDialect::CodexCliV1,
                },
                capability_snapshot_ref: provider.capability_snapshot_ref.clone(),
                resume_evidence: *self.resume.lock().unwrap(),
            })
        }
    }

    /// 测试用 target resolver:直接返回请求中的 target(聚合根路径已由 fixture
    /// 真实创建)。spawn 前 canonical 复验由 gateway 内部完成。
    struct PassthroughTargetResolver;

    impl PolicyTargetResolver for PassthroughTargetResolver {
        fn resolve_and_revalidate(
            &self,
            request: &SessionLaunchRequest,
        ) -> Result<PolicyTarget, ProviderGatewayError> {
            Ok(request.target.clone())
        }
    }

    /// 测试用 streaming adapter:记录 start 调用次数并立即完成会话。
    struct CountingStreamingAdapter {
        start_count: std::sync::atomic::AtomicUsize,
    }

    impl CountingStreamingAdapter {
        fn new() -> Self {
            Self {
                start_count: std::sync::atomic::AtomicUsize::new(0),
            }
        }

        fn start_count(&self) -> usize {
            self.start_count.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl crate::cross_cutting::streaming_provider::StreamingProviderAdapter
        for CountingStreamingAdapter
    {
        async fn start(
            &self,
            _input: crate::cross_cutting::streaming_provider::StreamingProviderInput,
            _cancel: CancellationToken,
        ) -> Result<
            crate::cross_cutting::streaming_provider::ProviderSession,
            crate::cross_cutting::provider_adapter::ProviderAdapterError,
        > {
            self.start_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let (_event_tx, events) = tokio::sync::mpsc::channel(1);
            let (commands, _command_rx) = tokio::sync::mpsc::channel(1);
            Ok(crate::cross_cutting::streaming_provider::ProviderSession { events, commands })
        }
    }

    struct StubSyncAdapter;

    impl crate::cross_cutting::provider_adapter::ProviderAdapter for StubSyncAdapter {
        fn run(
            &self,
            _input: &crate::protocol::contracts::AdapterInput,
        ) -> Result<
            crate::protocol::contracts::AdapterOutput,
            crate::cross_cutting::provider_adapter::ProviderAdapterError,
        > {
            use crate::protocol::contracts::TimeoutStatus;
            Ok(crate::protocol::contracts::AdapterOutput {
                exit_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
                structured_output: None,
                files_modified: Vec::new(),
                duration_ms: 0,
                timeout_status: TimeoutStatus::NotTimedOut,
            })
        }
    }

    fn always_available_gate() -> Arc<ProviderAvailabilityGate> {
        use crate::cross_cutting::provider_availability_gate::ProviderHealthSource;
        use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
        use chrono::Utc;

        struct AlwaysHealthy(Arc<ProviderHealthSnapshot>);
        impl ProviderHealthSource for AlwaysHealthy {
            fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
                self.0.clone()
            }
            fn degraded(&self) -> bool {
                false
            }
        }

        let checked_at = Utc::now();
        let snapshot = Arc::new(ProviderHealthSnapshot {
            schema_version: 1,
            generation: 1,
            checked_at,
            providers: [ProviderName::ClaudeCode, ProviderName::Codex]
                .into_iter()
                .map(|provider| ProviderHealthEntry {
                    provider,
                    command: "stub".to_string(),
                    available: true,
                    version: Some("1.0".to_string()),
                    reason_code: None,
                    reason: None,
                    checked_at,
                })
                .collect(),
        });
        Arc::new(ProviderAvailabilityGate::new(Arc::new(AlwaysHealthy(
            snapshot,
        ))))
    }

    /// gateway-backed 聚合初始化 fixture:把 `GatewayBackedAggregateProviderTurnDriver`
    /// 注入 coordinator,使三个 provider turn 唯一经 gateway 启动。共享
    /// `GatewayRunAudit` 与 `CountingStreamingAdapter` 供断言。
    struct GatewayAggregateFixture {
        _temp: tempfile::TempDir,
        audit: Arc<GatewayRunAudit>,
        streaming_adapter: Arc<CountingStreamingAdapter>,
        coordinator: AggregateInitializationCoordinator,
    }

    impl GatewayAggregateFixture {
        fn coordinator(&self) -> &AggregateInitializationCoordinator {
            &self.coordinator
        }

        fn gateway_audit(&self) -> Arc<GatewayRunAudit> {
            self.audit.clone()
        }

        fn streaming_start_count(&self) -> usize {
            self.streaming_adapter.start_count()
        }
    }

    fn gateway_aggregate_fixture() -> GatewayAggregateFixture {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        let store = AggregateInitializationOperationStore::new(paths.clone());

        // 聚合根是真实目录(aggregate_preflight + gateway spawn 前 canonicalize 需要它存在且非 git)。
        let aggregate_root = temp.path().join("aggregate-root");
        std::fs::create_dir_all(&aggregate_root).unwrap();

        let manifest =
            LogicalCodebaseManifest::new("project_0001", aggregate_root.clone(), Vec::new());
        LogicalCodebaseStore::new(paths.clone())
            .save_manifest("project_0001", &manifest)
            .unwrap();

        // 安装 bootstrap policy(gateway validate 需要 policy artifact)。
        crate::product::logical_codebase::provider_gateway::ensure_bootstrap_policy(
            &paths, &manifest,
        )
        .unwrap();

        let audit = Arc::new(GatewayRunAudit::new());
        let streaming_adapter = Arc::new(CountingStreamingAdapter::new());
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderName::ClaudeCode, streaming_adapter.clone());
        let gateway = Arc::new(LogicalCodebaseProviderGateway::with_audit(
            crate::product::logical_codebase::AggregatePolicyArtifactStore::new(paths.clone()),
            Arc::new(StaticCapabilitySource::new("1.4.0")),
            Arc::new(PassthroughTargetResolver),
            Arc::new(registry),
            Arc::new(StubSyncAdapter),
            always_available_gate(),
            audit.clone(),
        ));

        let skills: Arc<dyn AggregateSkillsPreparation> = Arc::new(FakeSkillsPreparation {
            calls: Arc::new(Mutex::new(Vec::new())),
        });
        // 用真实 `DeterministicAggregatePreflightService` 以产出可 canonicalize 的聚合根快照,
        // 使 gateway spawn 前 cwd 复验能通过。
        let preflight: Arc<dyn AggregatePreflightService> =
            Arc::new(DeterministicAggregatePreflightService::new(paths.clone()));
        let provider: Arc<dyn AggregateProviderTurnDriver> = Arc::new(
            GatewayBackedAggregateProviderTurnDriver::claude_code(gateway, "cap_claude_code_1_4_0"),
        );
        let clock: Arc<Clock> = Arc::new(|| CREATED_AT.to_string());
        let coordinator = AggregateInitializationCoordinator::new(
            paths.clone(),
            store.clone(),
            skills,
            preflight,
            provider,
            clock,
        );

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

        GatewayAggregateFixture {
            _temp: temp,
            audit,
            streaming_adapter,
            coordinator,
        }
    }

    #[tokio::test]
    async fn provider_steps_use_gateway_to_launch_three_streaming_turns() {
        let fixture = gateway_aggregate_fixture();
        fixture
            .coordinator()
            .execute(
                "project_0001",
                "aggregate_initialization_0001",
                CancellationToken::new(),
            )
            .await
            .unwrap();

        // 三个 provider turn(pre_check/rule_and_mcp_config/openspec_and_examples)
        // 唯一经 gateway 启动,故 stream_launches()==3。machine_skills 与
        // aggregate_preflight 是确定性 Cadence 代码,不产生 gateway 启动。
        assert_eq!(fixture.gateway_audit().stream_launches(), 3);
        assert_eq!(fixture.streaming_start_count(), 3);
        assert!(fixture.gateway_audit().all_have_policy_digest());
    }

    #[test]
    fn aggregate_coordinator_isolation_locked_against_single_repository_persistence_and_git_finalize()
     {
        // 隔离回归门:coordinator 生产代码(非测试、非 doc comment)不得引用
        // 单仓持久化层或单仓 git 终结点,保证聚合模式不进入成员仓 git 调用图。
        // 本测试排除 `#[cfg(test)]` 模块与 doc comment 行后再扫描,避免自指。
        let source = include_str!("aggregate_initialization_coordinator.rs");
        let forbidden = [
            "RepositoryPersistence",
            "git_finalize",
            "RepositoryRegistrationCoordinator",
        ];
        let mut in_test_module = false;
        for line in source.lines() {
            if line.trim_start().starts_with("#[cfg(test)]") {
                in_test_module = true;
            }
            if in_test_module {
                continue;
            }
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            for token in forbidden {
                assert!(
                    !line.contains(token),
                    "aggregate coordinator production code must not reference {token}: {line}"
                );
            }
        }
    }

    #[test]
    fn aggregate_asset_publisher_only_accepts_aria_aggregate_paths() {
        let publisher = AggregateAssetPublisher::new();
        publisher
            .publish("aggregate_initialization_0001", ".aria/aggregate/CLAUDE.md")
            .unwrap();
        publisher
            .publish("aggregate_initialization_0001", ".aria/aggregate/mcp.json")
            .unwrap();
        publisher
            .publish(
                "aggregate_initialization_0001",
                ".aria/aggregate/openspec-examples.json",
            )
            .unwrap();
        assert_eq!(
            publisher.published_paths(),
            vec![
                ".aria/aggregate/CLAUDE.md",
                ".aria/aggregate/mcp.json",
                ".aria/aggregate/openspec-examples.json",
            ]
        );
    }

    #[test]
    fn aggregate_asset_publisher_rejects_member_repository_and_escape_paths() {
        let publisher = AggregateAssetPublisher::new();
        // 成员仓路径:fail-closed。
        assert!(
            publisher
                .publish(
                    "aggregate_initialization_0001",
                    "members/repo_0001/CLAUDE.md"
                )
                .is_err()
        );
        // 父目录逃逸:fail-closed。
        assert!(
            publisher
                .publish(
                    "aggregate_initialization_0001",
                    ".aria/aggregate/../../../etc/passwd"
                )
                .is_err()
        );
        // 绝对路径:fail-closed。
        assert!(
            publisher
                .publish("aggregate_initialization_0001", "/etc/passwd")
                .is_err()
        );
        // 仅 `.aria/aggregate` 目录本身(无子项)不算 asset:fail-closed。
        assert!(
            publisher
                .publish("aggregate_initialization_0001", ".aria/aggregate")
                .is_err()
        );
        // 非 aggregate 子树:fail-closed。
        assert!(
            publisher
                .publish("aggregate_initialization_0001", ".aria/other/config.json")
                .is_err()
        );
        assert!(publisher.published_paths().is_empty());
    }

    // ---- Task 17: profile 预检与 frontend pnpm/Vite 选择 ----

    /// 为 profile 预检构建一个真实的 logical codebase coordinator fixture:
    /// 在 temp 目录创建 aggregate root、member main checkout 目录,并持久化
    /// manifest + member + checkout。`member_root(name)` 返回该 member 的
    /// main checkout 根,使测试可以在里面写 package.json / vite.config.ts。
    struct ProfileFixture {
        _temp: tempfile::TempDir,
        _paths: ProductAppPaths,
        member_roots: std::collections::HashMap<String, PathBuf>,
        coordinator: AggregateInitializationCoordinator,
    }

    impl ProfileFixture {
        /// 返回指定别名 member 的 main checkout 根,供测试写入 profile 信号。
        fn member_root(&self, alias: &str) -> &Path {
            self.member_roots
                .get(alias)
                .unwrap_or_else(|| panic!("unknown member alias {alias}"))
        }

        fn preflight_profile(
            &self,
        ) -> Result<AggregateInitializationProfile, AggregateInitializationError> {
            self.coordinator.preflight_profile("project_0001")
        }

        fn preflight_commands(&self) -> Vec<String> {
            self.coordinator.preflight_commands("project_0001").unwrap()
        }
    }

    fn profile_fixture(member_aliases: &[&str]) -> ProfileFixture {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        let store = AggregateInitializationOperationStore::new(paths.clone());

        let aggregate_root = temp.path().join("aggregate-root");
        std::fs::create_dir_all(&aggregate_root).unwrap();

        let mut member_roots = std::collections::HashMap::new();
        let mut member_ids = Vec::new();
        let lc_store = LogicalCodebaseStore::new(paths.clone());
        for (ordinal, alias) in member_aliases.iter().enumerate() {
            let member_dir = aggregate_root.join(alias);
            std::fs::create_dir_all(&member_dir).unwrap();
            let member_id = LogicalRepositoryId(Uuid::new_v4());
            let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
            member_ids.push(member_id);
            member_roots.insert((*alias).to_string(), member_dir.clone());
            let now = CREATED_AT.to_string();
            let member = CodebaseMemberRecord {
                logical_repository_id: member_id,
                physical_repository_id: format!("repository_{alias}"),
                alias: (*alias).to_string(),
                role: "service".to_string(),
                ordinal: ordinal as u32,
                source_identity: RepositorySourceIdentity::from_git_parts(
                    &member_dir,
                    member_dir.join(".git"),
                    Some(format!("ssh://git@example.test/acme/{alias}.git")),
                ),
                repo_type: Default::default(),
                tech_stack: Vec::new(),
                owner: None,
                tags: Vec::new(),
                default_ref: None,
                checkout_ids: vec![checkout_id],
                status: Default::default(),
                created_at: now.clone(),
                updated_at: now,
            };
            lc_store.save_member("project_0001", &member).unwrap();
            let now = CREATED_AT.to_string();
            let checkout = RepositoryCheckoutRecord {
                checkout_id,
                logical_repository_id: member_id,
                physical_repository_id: format!("repository_{alias}"),
                kind: crate::product::logical_codebase::CheckoutKind::Main,
                canonical_path: member_dir.clone(),
                checkout_path_hash: "sha256:checkout".to_string(),
                git_dir_identity: "sha256:git-dir".to_string(),
                revision: Some("abc123".to_string()),
                availability: crate::product::logical_codebase::CheckoutAvailability::Available,
                observed_at: now.clone(),
                created_at: now.clone(),
                updated_at: now,
            };
            lc_store.save_checkout("project_0001", &checkout).unwrap();
        }

        let manifest = LogicalCodebaseManifest::new("project_0001", aggregate_root, member_ids);
        lc_store.save_manifest("project_0001", &manifest).unwrap();

        let skills_calls = Arc::new(Mutex::new(Vec::new()));
        let preflight_calls = Arc::new(Mutex::new(Vec::new()));
        let skills: Arc<dyn AggregateSkillsPreparation> = Arc::new(FakeSkillsPreparation {
            calls: skills_calls,
        });
        let preflight: Arc<dyn AggregatePreflightService> = Arc::new(FakePreflightService {
            calls: preflight_calls,
        });
        let provider = Arc::new(FakeProviderTurnDriver::new());
        let clock: Arc<Clock> = Arc::new(|| CREATED_AT.to_string());
        let coordinator = AggregateInitializationCoordinator::new(
            paths.clone(),
            store,
            skills,
            preflight,
            provider,
            clock,
        );

        ProfileFixture {
            _temp: temp,
            _paths: paths.clone(),
            member_roots,
            coordinator,
        }
    }

    #[test]
    fn frontend_pnpm_vite_profile_changes_templates_not_stable_step_layout() {
        let fixture = profile_fixture(&["web"]);
        std::fs::write(
            fixture.member_root("web").join("package.json"),
            r#"{"packageManager":"pnpm@9","devDependencies":{"vite":"1"}}"#,
        )
        .unwrap();
        std::fs::write(
            fixture.member_root("web").join("pnpm-lock.yaml"),
            "lockfileVersion: '9.0'",
        )
        .unwrap();
        std::fs::write(
            fixture.member_root("web").join("vite.config.ts"),
            "export default {}",
        )
        .unwrap();

        let profile = fixture.preflight_profile().unwrap();
        assert_eq!(profile, AggregateInitializationProfile::FrontendPnpmVite);
        assert_eq!(AggregateInitializationStepKind::V1.len(), 5);
        assert!(
            !fixture
                .preflight_commands()
                .iter()
                .any(|command| command.contains("mvn") || command.contains("gradle"))
        );
    }

    #[test]
    fn java_backend_profile_resolves_when_all_members_are_backend() {
        let fixture = profile_fixture(&["api"]);
        std::fs::write(
            fixture.member_root("api").join("pom.xml"),
            "<project><modelVersion>4.0.0</modelVersion></project>",
        )
        .unwrap();

        let profile = fixture.preflight_profile().unwrap();
        assert_eq!(profile, AggregateInitializationProfile::JavaBackend);
        assert!(
            fixture
                .preflight_commands()
                .iter()
                .any(|command| command.contains("mvn"))
        );
    }

    #[test]
    fn mixed_profile_resolves_when_backend_and_frontend_members_coexist() {
        let fixture = profile_fixture(&["api", "web"]);
        std::fs::write(
            fixture.member_root("api").join("pom.xml"),
            "<project><modelVersion>4.0.0</modelVersion></project>",
        )
        .unwrap();
        std::fs::write(
            fixture.member_root("web").join("package.json"),
            r#"{"packageManager":"pnpm@9"}"#,
        )
        .unwrap();
        std::fs::write(
            fixture.member_root("web").join("pnpm-lock.yaml"),
            "lockfileVersion: '9.0'",
        )
        .unwrap();
        std::fs::write(
            fixture.member_root("web").join("vite.config.ts"),
            "export default {}",
        )
        .unwrap();

        let profile = fixture.preflight_profile().unwrap();
        assert_eq!(profile, AggregateInitializationProfile::Mixed);
    }

    #[test]
    fn unknown_profile_fails_preflight_closed() {
        let fixture = profile_fixture(&["stray"]);
        // No recognizable signals -> detect returns Unknown -> preflight fails closed.
        let error = fixture.preflight_profile().unwrap_err();
        assert!(matches!(
            error,
            AggregateInitializationError::Preflight { .. }
        ));
    }

    #[test]
    fn profile_preflight_commands_are_profile_specific_and_keep_five_step_layout() {
        // Frontend pnpm/Vite precheck never includes Maven/Gradle commands.
        let frontend = profile_preflight_commands(AggregateInitializationProfile::FrontendPnpmVite);
        assert!(frontend.iter().any(|command| command.contains("pnpm")));
        assert!(
            !frontend
                .iter()
                .any(|command| command.contains("mvn") || command.contains("gradle"))
        );

        // Java backend includes Maven.
        let java = profile_preflight_commands(AggregateInitializationProfile::JavaBackend);
        assert!(java.iter().any(|command| command.contains("mvn")));

        // Mixed composes both namespaced command sets.
        let mixed = profile_preflight_commands(AggregateInitializationProfile::Mixed);
        assert!(mixed.iter().any(|command| command.contains("mvn")));
        assert!(mixed.iter().any(|command| command.contains("pnpm")));

        // The five stable step IDs never change regardless of profile.
        assert_eq!(
            AggregateInitializationStepKind::V1.len(),
            5,
            "profile selection must not change the stable step layout"
        );
    }
}
