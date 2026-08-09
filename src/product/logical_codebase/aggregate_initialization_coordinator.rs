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

// 以下各职责模块通过文本包含(`include!`)进入此文件,保持模块命名空间与公开 API 不变。
// 拆分仅为满足 large_file_guard 的 1200 行上限,不改变任何行为。

include!("coordinator_lifecycle.inc.rs");
include!("coordinator_provider_turn.inc.rs");
include!("coordinator_preflight.inc.rs");
include!("coordinator_profile.inc.rs");
include!("coordinator_tests.inc.rs");
