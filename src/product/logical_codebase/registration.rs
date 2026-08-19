use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(test)]
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::locking::with_exact_exclusive_lock;
use crate::product::id::repo_hash_for_path;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::logical_codebase::{
    CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, IdentityRegistryEntry,
    IdentityRegistryState, IdentityRegistryStore, LogicalCodebaseFeature, LogicalCodebaseStore,
    LogicalRepositoryId, MemberStatus, RepositoryCheckoutId, RepositoryCheckoutRecord,
    RepositorySourceIdentity, RepositoryType,
};
use crate::product::repository_store::{canonicalize_repo_path, resolve_repository_source};

/// Canonical, non-Git common parent that has passed aggregate-root admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalAggregateRoot {
    pub canonical_path: PathBuf,
}

/// The caller-owned preflight manifest. An empty `paths` list requests
/// recursive child Git-directory discovery below the already admitted
/// aggregate root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistrationPreflightInput {
    pub project_id: String,
    pub aggregate_root: CanonicalAggregateRoot,
    pub paths: Vec<PathBuf>,
}

/// A category assigned to one submitted or discovered registration candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationCandidateState {
    Eligible,
    NonGit,
    Duplicate,
    Nested,
    /// Retained in the public classification vocabulary. A dirty repository
    /// remains registrable and is reported as [`Self::NeedsAttention`].
    Dirty,
    Missing,
    OutsideRoot,
    NeedsAttention,
}

/// The complete read-only observation made for one registration candidate.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RegistrationCandidate {
    pub submitted_path: PathBuf,
    pub canonical_path: Option<PathBuf>,
    pub git_root: Option<PathBuf>,
    pub source_identity: Option<RepositorySourceIdentity>,
    pub state: RegistrationCandidateState,
    pub reason: String,
    pub preflight_revision: String,
}

impl RegistrationCandidate {
    fn missing(submitted_path: PathBuf) -> Self {
        Self::new(
            submitted_path,
            None,
            None,
            None,
            RegistrationCandidateState::Missing,
            "path_missing",
            None,
            None,
        )
    }

    fn outside_root(submitted_path: PathBuf, canonical_path: PathBuf) -> Self {
        Self::new(
            submitted_path,
            Some(canonical_path),
            None,
            None,
            RegistrationCandidateState::OutsideRoot,
            "outside_aggregate_root",
            None,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new(
        submitted_path: PathBuf,
        canonical_path: Option<PathBuf>,
        git_root: Option<PathBuf>,
        source_identity: Option<RepositorySourceIdentity>,
        state: RegistrationCandidateState,
        reason: impl Into<String>,
        head: Option<&str>,
        status: Option<&str>,
    ) -> Self {
        let preflight_revision = preflight_revision(
            canonical_path.as_deref(),
            git_root.as_deref(),
            source_identity.as_ref(),
            head,
            status,
        );
        Self {
            submitted_path,
            canonical_path,
            git_root,
            source_identity,
            state,
            reason: reason.into(),
            preflight_revision,
        }
    }
}

/// A complete, independently classified registration preflight result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistrationPreflightResult {
    pub project_id: String,
    pub aggregate_root: CanonicalAggregateRoot,
    pub candidates: Vec<RegistrationCandidate>,
}

impl RegistrationPreflightResult {
    pub fn count(&self, state: RegistrationCandidateState) -> usize {
        self.candidates
            .iter()
            .filter(|candidate| candidate.state == state)
            .count()
    }
}

/// The persisted lifecycle of a confirmed batch registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationBatchStatus {
    Queued,
    Running,
    PartialFailed,
    Completed,
    Cancelled,
}

/// The persisted lifecycle of an individual confirmed candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationItemStatus {
    Pending,
    Skipped,
    Completed,
    Failed,
    NeedsAttention,
}

/// One candidate frozen from an explicitly user-confirmed preflight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrationBatchItem {
    pub source_digest: String,
    pub submitted_path: PathBuf,
    pub canonical_path: PathBuf,
    pub git_root: PathBuf,
    pub source_identity: RepositorySourceIdentity,
    pub preflight_revision: String,
    pub alias: String,
    pub role: String,
    pub repo_type: RepositoryType,
    pub tech_stack: Vec<String>,
    pub status: RegistrationItemStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    #[serde(default)]
    pub retry_count: u32,
}

/// A durable receipt for a confirmed batch registration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrationBatchRecord {
    pub id: String,
    pub project_id: String,
    pub idempotency_key: String,
    pub aggregate_root: PathBuf,
    pub status: RegistrationBatchStatus,
    pub items: Vec<RegistrationBatchItem>,
    #[serde(default)]
    pub retry_count: u32,
    pub created_at: String,
    pub updated_at: String,
}

/// Caller-owned confirmation of a preflight. `include_needs_attention` is an
/// explicit user acknowledgement for dirty checkouts; all other non-eligible
/// candidates are retained as skipped audit entries and are never attached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmedRegistrationBatchInput {
    pub project_id: String,
    pub aggregate_root: CanonicalAggregateRoot,
    pub candidates: Vec<RegistrationCandidate>,
    pub include_needs_attention: bool,
}

impl ConfirmedRegistrationBatchInput {
    pub fn from_preflight(
        preflight: &RegistrationPreflightResult,
        include_needs_attention: bool,
    ) -> Self {
        Self {
            project_id: preflight.project_id.clone(),
            aggregate_root: preflight.aggregate_root.clone(),
            candidates: preflight.candidates.clone(),
            include_needs_attention,
        }
    }
}

/// Stores batch records below the project logical-codebase root. Every
/// mutation is serialized on the project-scoped lock so simultaneous creates,
/// resumes and cancels cannot race a record transition.
/// A deterministic admission failure for the aggregate root.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct AggregateRootPreflightError {
    code: &'static str,
    message: String,
}

impl AggregateRootPreflightError {
    pub fn code(&self) -> &'static str {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

#[cfg(test)]
impl AggregateRootPreflightError {
    pub(crate) fn new_for_test(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Validates the filesystem ownership and containment invariants for an
/// aggregate root before any member discovery or registration is performed.
#[derive(Debug, Clone)]
pub struct AttachOnlyRegistrationInput {
    pub project_id: String,
    pub alias: String,
    pub role: String,
    pub canonical_path: PathBuf,
    pub repo_type: RepositoryType,
    pub tech_stack: Vec<String>,
    pub idempotency_key: String,
}

/// Coordinates an attach-only registration without entering the single-
/// repository initialization chain.
#[derive(Debug, Clone)]
pub struct LogicalCodebaseRegistrationCoordinator {
    paths: ProductAppPaths,
    lc_id: Option<String>,
    feature: LogicalCodebaseFeature,
    #[cfg(test)]
    failure_after_completed_items: Arc<AtomicUsize>,
}

include!("registration_batch.inc.rs");
include!("registration_preflight.inc.rs");
include!("registration_coordinator.inc.rs");
include!("registration_git_snapshot.inc.rs");
include!("registration_profile_detector.inc.rs");
include!("registration_tests.inc.rs");
