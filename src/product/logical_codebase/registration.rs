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
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::logical_codebase::{
    CodebaseMemberRecord, IdentityRegistryStore, LogicalCodebaseFeature, LogicalCodebaseStore,
    RepositorySourceIdentity, RepositoryType,
};
use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};

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
#[derive(Debug, Clone)]
pub struct RegistrationBatchStore {
    paths: ProductAppPaths,
}

impl RegistrationBatchStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    pub fn create_or_get(
        &self,
        batch: RegistrationBatchRecord,
    ) -> Result<RegistrationBatchRecord, ProductStoreError> {
        validate_batch_record(&batch)?;
        let project_id = batch.project_id.clone();
        self.with_project_lock(&project_id, || {
            let existing =
                self.find_by_idempotency_key_unlocked(&project_id, &batch.idempotency_key)?;
            match existing {
                Some(existing) => {
                    validate_batch_record(&existing)?;
                    if existing.aggregate_root != batch.aggregate_root
                        || existing.items != batch.items
                    {
                        return Err(ProductStoreError::Conflict {
                            kind: "registration_batch_idempotency_key_reused",
                            id: batch.idempotency_key.clone(),
                        });
                    }
                    Ok(existing)
                }
                None => {
                    let path = self.batch_path(&project_id, &batch.id)?;
                    if path.exists() {
                        return Err(ProductStoreError::Conflict {
                            kind: "registration_batch_id_collision",
                            id: batch.id.clone(),
                        });
                    }
                    write_json(&path, &batch)?;
                    Ok(batch)
                }
            }
        })
    }

    pub fn load(
        &self,
        project_id: &str,
        batch_id: &str,
    ) -> Result<RegistrationBatchRecord, ProductStoreError> {
        let path = self.batch_path(project_id, batch_id)?;
        if !path.exists() {
            return Err(ProductStoreError::NotFound {
                kind: "registration_batch",
                id: batch_id.to_string(),
            });
        }
        let batch: RegistrationBatchRecord = read_json(&path)?;
        validate_batch_record_for(&batch, project_id, batch_id)?;
        Ok(batch)
    }

    pub fn resume(
        &self,
        project_id: &str,
        batch_id: &str,
    ) -> Result<RegistrationBatchRecord, ProductStoreError> {
        self.load(project_id, batch_id)
    }

    pub fn cancel(
        &self,
        project_id: &str,
        batch_id: &str,
    ) -> Result<RegistrationBatchRecord, ProductStoreError> {
        self.with_batch_mutation(project_id, batch_id, |batch| {
            if batch.status != RegistrationBatchStatus::Completed {
                batch.status = RegistrationBatchStatus::Cancelled;
                batch.updated_at = Utc::now().to_rfc3339();
            }
            Ok(())
        })
        .map(|(batch, ())| batch)
    }

    fn with_batch_mutation<T>(
        &self,
        project_id: &str,
        batch_id: &str,
        mutation: impl FnOnce(&mut RegistrationBatchRecord) -> Result<T, ProductStoreError>,
    ) -> Result<(RegistrationBatchRecord, T), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(batch_id)?;
        self.with_project_lock(project_id, || {
            let mut batch = self.load(project_id, batch_id)?;
            let output = mutation(&mut batch)?;
            validate_batch_record(&batch)?;
            write_json(&self.batch_path(project_id, batch_id)?, &batch)?;
            Ok((batch, output))
        })
    }

    fn save_unlocked(&self, batch: &RegistrationBatchRecord) -> Result<(), ProductStoreError> {
        validate_batch_record(batch)?;
        write_json(&self.batch_path(&batch.project_id, &batch.id)?, batch)
    }

    fn with_project_lock<T>(
        &self,
        project_id: &str,
        operation: impl FnOnce() -> Result<T, ProductStoreError>,
    ) -> Result<T, ProductStoreError> {
        validate_relative_id(project_id)?;
        with_exact_exclusive_lock(
            &self.paths.registration_batches_lock_path(project_id),
            operation,
        )
    }

    fn find_by_idempotency_key_unlocked(
        &self,
        project_id: &str,
        idempotency_key: &str,
    ) -> Result<Option<RegistrationBatchRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(idempotency_key)?;
        let root = self.paths.registration_batches_root(project_id);
        let entries = match fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "read registration batches {}: {error}",
                    root.display()
                )));
            }
        };
        for entry in entries {
            let entry = entry.map_err(|error| {
                ProductStoreError::Io(format!("read registration batch entry: {error}"))
            })?;
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            let batch: RegistrationBatchRecord = read_json(&path)?;
            validate_batch_record(&batch)?;
            if batch.idempotency_key == idempotency_key {
                return Ok(Some(batch));
            }
        }
        Ok(None)
    }

    fn batch_path(&self, project_id: &str, batch_id: &str) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(batch_id)?;
        Ok(self
            .paths
            .registration_batches_root(project_id)
            .join(format!("{batch_id}.json")))
    }
}

fn validate_batch_record(batch: &RegistrationBatchRecord) -> Result<(), ProductStoreError> {
    validate_batch_record_for(batch, &batch.project_id, &batch.id)
}

fn validate_batch_record_for(
    batch: &RegistrationBatchRecord,
    project_id: &str,
    batch_id: &str,
) -> Result<(), ProductStoreError> {
    validate_relative_id(project_id)?;
    validate_relative_id(batch_id)?;
    validate_relative_id(&batch.project_id)?;
    validate_relative_id(&batch.id)?;
    validate_relative_id(&batch.idempotency_key)?;
    if batch.project_id != project_id || batch.id != batch_id || batch.items.is_empty() {
        return Err(ProductStoreError::IdentityMismatch {
            kind: "registration_batch",
            id: batch_id.to_string(),
        });
    }
    let mut source_digests = std::collections::BTreeSet::new();
    for item in &batch.items {
        if item.source_digest.is_empty()
            || !source_digests.insert(item.source_digest.clone())
            || item.canonical_path.as_os_str().is_empty()
            || item.git_root.as_os_str().is_empty()
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "registration_batch_item",
                id: batch_id.to_string(),
            });
        }
    }
    Ok(())
}

fn aggregate_batch_status(items: &[RegistrationBatchItem]) -> RegistrationBatchStatus {
    if items.iter().all(|item| {
        matches!(
            item.status,
            RegistrationItemStatus::Completed | RegistrationItemStatus::Skipped
        )
    }) {
        RegistrationBatchStatus::Completed
    } else {
        RegistrationBatchStatus::PartialFailed
    }
}

fn sha256_key(payload: impl AsRef<[u8]>) -> String {
    format!("sha256:{:x}", Sha256::digest(payload.as_ref()))
}

fn stable_alias(candidate: &RegistrationCandidate) -> String {
    candidate
        .canonical_path
        .as_deref()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("repository")
        .to_string()
}

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

/// Validates the filesystem ownership and containment invariants for an
/// aggregate root before any member discovery or registration is performed.
#[derive(Debug, Clone)]
pub struct AggregateRootPreflight {
    paths: ProductAppPaths,
}

impl AggregateRootPreflight {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    pub fn validate(
        &self,
        project_id: &str,
        root: &Path,
        candidate_paths: &[PathBuf],
    ) -> Result<CanonicalAggregateRoot, AggregateRootPreflightError> {
        validate_relative_id(project_id).map_err(|error| {
            preflight_error(
                "aggregate_root_invalid_project_id",
                format!(
                    "project ID {project_id:?} is not a safe relative identifier: {error}; use a project ID without path separators"
                ),
            )
        })?;

        let canonical_root = canonicalize_for_preflight(root, "aggregate_root_missing")?;
        if !canonical_root.is_dir() {
            return Err(preflight_error(
                "aggregate_root_missing",
                format!(
                    "aggregate root {} is not a directory; choose an existing common parent directory",
                    canonical_root.display()
                ),
            ));
        }
        if is_git_root(&canonical_root)? {
            return Err(preflight_error(
                "aggregate_root_is_git",
                format!(
                    "aggregate root {} is a Git repository; choose its non-Git common parent instead",
                    canonical_root.display()
                ),
            ));
        }

        for candidate in candidate_paths {
            self.validate_member_path(root, &canonical_root, candidate)?;
        }

        self.reject_owned_root_files(&canonical_root)?;
        self.reject_overlapping_manifest_root(project_id, &canonical_root)?;

        Ok(CanonicalAggregateRoot {
            canonical_path: canonical_root,
        })
    }

    fn validate_member_path(
        &self,
        supplied_root: &Path,
        canonical_root: &Path,
        candidate: &Path,
    ) -> Result<(), AggregateRootPreflightError> {
        let candidate_is_under_root =
            candidate.starts_with(supplied_root) || candidate.starts_with(canonical_root);
        let canonical_member = canonicalize_for_preflight(candidate, "member_path_missing")?;
        if canonical_member == canonical_root {
            return Err(preflight_error(
                "member_path_outside_root",
                format!(
                    "member path {} resolves to aggregate root {}; select a descendant member directory",
                    candidate.display(),
                    canonical_root.display()
                ),
            ));
        }
        if !candidate_is_under_root {
            return Err(preflight_error(
                "member_path_outside_root",
                format!(
                    "member path {} resolves to {} outside aggregate root {}; select a path below the aggregate root",
                    candidate.display(),
                    canonical_member.display(),
                    canonical_root.display()
                ),
            ));
        }
        if !canonical_member.starts_with(canonical_root) {
            return Err(preflight_error(
                "member_symlink_escape",
                format!(
                    "member path {} resolves to {} outside aggregate root {}; remove the escaping symlink or select an in-root member",
                    candidate.display(),
                    canonical_member.display(),
                    canonical_root.display()
                ),
            ));
        }
        if is_linked_worktree(&canonical_member)? {
            return Err(preflight_error(
                "nested_worktree",
                format!(
                    "member path {} resolves to linked worktree {}; select the main checkout instead",
                    candidate.display(),
                    canonical_member.display()
                ),
            ));
        }
        Ok(())
    }

    fn reject_owned_root_files(
        &self,
        canonical_root: &Path,
    ) -> Result<(), AggregateRootPreflightError> {
        for name in ["CLAUDE.md", "AGENTS.md", ".aria"] {
            let path = canonical_root.join(name);
            if path_exists_for_preflight(&path)? {
                return Err(preflight_error(
                    "aggregate_root_ownership_conflict",
                    format!(
                        "aggregate root {} already contains user-owned {}; move or merge it before aggregate initialization",
                        canonical_root.display(),
                        path.display()
                    ),
                ));
            }
        }
        Ok(())
    }

    fn reject_overlapping_manifest_root(
        &self,
        project_id: &str,
        canonical_root: &Path,
    ) -> Result<(), AggregateRootPreflightError> {
        for manifest in LogicalCodebaseStore::new(self.paths.clone())
            .list_manifests()
            .map_err(|error| {
                preflight_error(
                    "aggregate_root_overlap",
                    format!(
                        "could not inspect existing logical-codebase manifests while validating {}: {error}; resolve the state-store error before retrying",
                        canonical_root.display()
                    ),
                )
            })?
        {
            if manifest.project_id == project_id {
                continue;
            }
            let existing_root = canonicalize_for_preflight(
                &manifest.provider_context_root,
                "aggregate_root_overlap",
            )?;
            if paths_overlap(canonical_root, &existing_root) {
                return Err(preflight_error(
                    "aggregate_root_overlap",
                    format!(
                        "aggregate root {} overlaps logical codebase root {} owned by project {}; choose a disjoint common parent",
                        canonical_root.display(),
                        existing_root.display(),
                        manifest.project_id
                    ),
                ));
            }
        }
        Ok(())
    }
}

fn canonicalize_for_preflight(
    path: &Path,
    missing_code: &'static str,
) -> Result<PathBuf, AggregateRootPreflightError> {
    fs::canonicalize(path).map_err(|error| {
        preflight_error(
            missing_code,
            format!(
                "path {} cannot be canonicalized: {error}; choose an existing accessible path",
                path.display()
            ),
        )
    })
}

fn is_git_root(path: &Path) -> Result<bool, AggregateRootPreflightError> {
    let git_path = path.join(".git");
    let metadata = match fs::symlink_metadata(&git_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(preflight_error(
                "aggregate_root_is_git",
                format!(
                    "could not inspect Git metadata {}: {error}; fix access and retry",
                    git_path.display()
                ),
            ));
        }
    };
    Ok(metadata.file_type().is_dir()
        || metadata.file_type().is_file()
        || metadata.file_type().is_symlink())
}

fn is_linked_worktree(path: &Path) -> Result<bool, AggregateRootPreflightError> {
    let git_file = path.join(".git");
    let metadata = match fs::symlink_metadata(&git_file) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(preflight_error(
                "nested_worktree",
                format!(
                    "could not inspect worktree metadata {}: {error}; fix access and retry",
                    git_file.display()
                ),
            ));
        }
    };
    if !metadata.file_type().is_file() {
        return Ok(false);
    }

    let contents = fs::read_to_string(&git_file).map_err(|error| {
        preflight_error(
            "nested_worktree",
            format!(
                "could not read worktree metadata {}: {error}; select a main checkout instead",
                git_file.display()
            ),
        )
    })?;
    let Some(gitdir) = contents.strip_prefix("gitdir:") else {
        return Ok(false);
    };
    let gitdir = gitdir.trim();
    if gitdir.is_empty() {
        return Err(preflight_error(
            "nested_worktree",
            format!(
                "worktree metadata {} has an empty gitdir target; select a main checkout instead",
                git_file.display()
            ),
        ));
    }

    let gitdir_path = Path::new(gitdir);
    let gitdir_path = if gitdir_path.is_absolute() {
        gitdir_path.to_path_buf()
    } else {
        path.join(gitdir_path)
    };
    let canonical_gitdir = fs::canonicalize(&gitdir_path).map_err(|error| {
            preflight_error(
                "nested_worktree",
                format!(
                    "worktree metadata {} points to inaccessible gitdir {}: {error}; select a main checkout instead",
                    git_file.display(),
                    gitdir_path.display()
                ),
            )
        })?;
    if canonical_gitdir
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name == "worktrees")
    {
        return Ok(true);
    }
    Ok(false)
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

fn path_exists_for_preflight(path: &Path) -> Result<bool, AggregateRootPreflightError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(preflight_error(
            "aggregate_root_ownership_conflict",
            format!(
                "could not inspect existing root asset {}: {error}; fix access and retry",
                path.display()
            ),
        )),
    }
}

fn preflight_error(code: &'static str, message: impl Into<String>) -> AggregateRootPreflightError {
    AggregateRootPreflightError {
        code,
        message: message.into(),
    }
}

/// The caller-owned input for one attach-only logical codebase registration.
///
/// This command deliberately carries no initialization or Git-finalization
/// controls: it only attaches an already-existing checkout to product
/// authority records.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    repositories: RepositoryStore,
    feature: LogicalCodebaseFeature,
    #[cfg(test)]
    failure_after_completed_items: Arc<AtomicUsize>,
}

impl LogicalCodebaseRegistrationCoordinator {
    pub fn new(
        paths: ProductAppPaths,
        repositories: RepositoryStore,
        feature: LogicalCodebaseFeature,
    ) -> Self {
        Self {
            paths,
            repositories,
            feature,
            #[cfg(test)]
            failure_after_completed_items: Arc::new(AtomicUsize::new(usize::MAX)),
        }
    }

    /// Persists the caller-confirmed preflight snapshot without attaching any
    /// member. Call [`Self::resume_batch`] to perform the revalidation and
    /// attach work; this keeps confirmation and execution separately durable.
    pub fn submit_confirmed_batch(
        &self,
        input: ConfirmedRegistrationBatchInput,
    ) -> Result<RegistrationBatchRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        if !self.feature.is_enabled() {
            return Err(ProductStoreError::Conflict {
                kind: "logical_codebase_feature_disabled",
                id: input.project_id,
            });
        }
        if input.candidates.is_empty() {
            return Err(ProductStoreError::InvalidRecord {
                kind: "registration_batch",
                reason: "confirmed preflight must contain at least one candidate".to_string(),
            });
        }

        let items = input
            .candidates
            .iter()
            .filter_map(|candidate| {
                batch_item_from_candidate(candidate, input.include_needs_attention).transpose()
            })
            .collect::<Result<Vec<_>, _>>()?;
        if items.is_empty() {
            return Err(ProductStoreError::InvalidRecord {
                kind: "registration_batch",
                reason: "confirmed preflight contains no selected registrable candidates"
                    .to_string(),
            });
        }
        let mut source_digests = std::collections::BTreeSet::new();
        if items
            .iter()
            .any(|item| !source_digests.insert(item.source_digest.clone()))
        {
            return Err(ProductStoreError::Conflict {
                kind: "registration_batch_duplicate_source",
                id: input.project_id,
            });
        }

        let canonical_manifest_digest = canonical_manifest_digest(&input.aggregate_root, &items);
        let mut revisions = items
            .iter()
            .map(|item| item.preflight_revision.as_str())
            .collect::<Vec<_>>();
        revisions.sort_unstable();
        let idempotency_key =
            batch_idempotency_key(&input.project_id, &canonical_manifest_digest, &revisions);
        let id = format!("registration_batch_{}", Uuid::new_v4().simple());
        validate_relative_id(&id)?;
        let now = Utc::now().to_rfc3339();
        RegistrationBatchStore::new(self.paths.clone()).create_or_get(RegistrationBatchRecord {
            id,
            project_id: input.project_id,
            idempotency_key,
            aggregate_root: input.aggregate_root.canonical_path,
            status: RegistrationBatchStatus::Queued,
            items,
            retry_count: 0,
            created_at: now.clone(),
            updated_at: now,
        })
    }

    pub fn get_batch(
        &self,
        project_id: &str,
        batch_id: &str,
    ) -> Result<RegistrationBatchRecord, ProductStoreError> {
        RegistrationBatchStore::new(self.paths.clone()).load(project_id, batch_id)
    }

    pub fn cancel_batch(
        &self,
        project_id: &str,
        batch_id: &str,
    ) -> Result<RegistrationBatchRecord, ProductStoreError> {
        RegistrationBatchStore::new(self.paths.clone()).cancel(project_id, batch_id)
    }

    /// Revalidates every unfinished item immediately before registration. The
    /// only operations before `attach_member` are the same read-only probes as
    /// preflight; changed path, Git root, identity, HEAD or worktree state is
    /// made visible as `needs_attention` rather than being silently attached.
    pub fn resume_batch(
        &self,
        project_id: &str,
        batch_id: &str,
    ) -> Result<RegistrationBatchRecord, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(batch_id)?;
        let batches = RegistrationBatchStore::new(self.paths.clone());
        let (batch, ()) = batches.with_batch_mutation(project_id, batch_id, |batch| {
            if batch.status == RegistrationBatchStatus::Cancelled
                || batch.status == RegistrationBatchStatus::Completed
            {
                return Ok(());
            }
            batch.status = RegistrationBatchStatus::Running;
            batch.updated_at = Utc::now().to_rfc3339();
            batches.save_unlocked(batch)
        })?;
        if matches!(
            batch.status,
            RegistrationBatchStatus::Cancelled | RegistrationBatchStatus::Completed
        ) {
            return Ok(batch);
        }

        // `with_batch_mutation` intentionally releases the lock before I/O
        // that may traverse Git metadata. The running transition arbitrates
        // concurrent callers; a second resume observes Running and receives a
        // deterministic conflict below.
        #[cfg(test)]
        let mut interrupted_for_test = false;
        #[cfg(not(test))]
        let interrupted_for_test = false;
        for index in 0..batch.items.len() {
            let mut current = batches.load(project_id, batch_id)?;
            if current.status == RegistrationBatchStatus::Cancelled {
                return Ok(current);
            }
            if current.status != RegistrationBatchStatus::Running {
                return Err(ProductStoreError::Conflict {
                    kind: "registration_batch_not_running",
                    id: batch_id.to_string(),
                });
            }
            let item = &mut current.items[index];
            if matches!(
                item.status,
                RegistrationItemStatus::Completed
                    | RegistrationItemStatus::Skipped
                    | RegistrationItemStatus::NeedsAttention
            ) {
                continue;
            }

            if self.member_already_attached(project_id, item)? {
                item.status = RegistrationItemStatus::Completed;
                item.failure_reason = None;
                item.retry_count = item.retry_count.saturating_add(1);
                current.updated_at = Utc::now().to_rfc3339();
                batches.with_batch_mutation(project_id, batch_id, |stored| {
                    replace_batch_item(stored, item.clone())?;
                    stored.updated_at = current.updated_at.clone();
                    Ok(())
                })?;
                continue;
            }

            let revalidated =
                self.revalidate_batch_item(project_id, &current.aggregate_root, item)?;
            if revalidated.preflight_revision != item.preflight_revision {
                item.status = RegistrationItemStatus::NeedsAttention;
                item.failure_reason = Some("preflight_revision_changed".to_string());
                item.retry_count = item.retry_count.saturating_add(1);
                current.updated_at = Utc::now().to_rfc3339();
                batches.with_batch_mutation(project_id, batch_id, |stored| {
                    replace_batch_item(stored, item.clone())?;
                    stored.updated_at = current.updated_at.clone();
                    Ok(())
                })?;
                continue;
            }

            item.retry_count = item.retry_count.saturating_add(1);
            let profile = RepositoryProfileDetector::detect(&item.git_root)?;
            item.repo_type = profile.repo_type.clone();
            item.tech_stack = profile.tech_stack.clone();
            let item_key = batch_item_idempotency_key(batch_id, &item.source_digest);
            match self.attach_member(AttachOnlyRegistrationInput {
                project_id: project_id.to_string(),
                alias: item.alias.clone(),
                role: item.role.clone(),
                canonical_path: item.canonical_path.clone(),
                repo_type: profile.repo_type,
                tech_stack: profile.tech_stack,
                idempotency_key: item_key,
            }) {
                Ok(_) => {
                    item.status = RegistrationItemStatus::Completed;
                    item.failure_reason = None;
                }
                Err(error) => {
                    item.status = RegistrationItemStatus::Failed;
                    item.failure_reason = Some(batch_failure_reason(&error));
                }
            }
            current.updated_at = Utc::now().to_rfc3339();
            batches.with_batch_mutation(project_id, batch_id, |stored| {
                replace_batch_item(stored, item.clone())?;
                stored.updated_at = current.updated_at.clone();
                Ok(())
            })?;
            #[cfg(test)]
            if item.status == RegistrationItemStatus::Completed
                && self.should_interrupt_after_completed_item()
            {
                interrupted_for_test = true;
                break;
            }
        }

        let (completed, ()) = batches.with_batch_mutation(project_id, batch_id, |stored| {
            if stored.status != RegistrationBatchStatus::Cancelled {
                stored.status = if interrupted_for_test {
                    RegistrationBatchStatus::PartialFailed
                } else {
                    aggregate_batch_status(&stored.items)
                };
                stored.retry_count = stored.retry_count.saturating_add(1);
                stored.updated_at = Utc::now().to_rfc3339();
            }
            Ok(())
        })?;
        Ok(completed)
    }

    fn member_already_attached(
        &self,
        project_id: &str,
        item: &RegistrationBatchItem,
    ) -> Result<bool, ProductStoreError> {
        let registry = IdentityRegistryStore::new(self.paths.clone());
        let Some(entry) = registry.find_by_source(project_id, &item.source_identity)? else {
            return Ok(false);
        };
        if entry.state != crate::product::logical_codebase::IdentityRegistryState::Active {
            return Ok(false);
        }
        let authority = LogicalCodebaseStore::new(self.paths.clone());
        let member = authority
            .load_member(project_id, entry.logical_repository_id)?
            .ok_or_else(|| ProductStoreError::IdentityMismatch {
                kind: "registration_batch_member_recovery",
                id: item.source_digest.clone(),
            })?;
        if member.physical_repository_id != entry.physical_repository_id
            || member.source_identity != item.source_identity
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "registration_batch_member_recovery",
                id: item.source_digest.clone(),
            });
        }
        Ok(true)
    }

    fn revalidate_batch_item(
        &self,
        project_id: &str,
        aggregate_root: &Path,
        item: &RegistrationBatchItem,
    ) -> Result<RegistrationCandidate, ProductStoreError> {
        let canonical_path = fs::canonicalize(&item.submitted_path).map_err(|error| {
            ProductStoreError::Io(format!(
                "canonicalize registration batch item {}: {error}",
                item.submitted_path.display()
            ))
        })?;
        if !canonical_path.starts_with(aggregate_root) {
            return Err(ProductStoreError::Conflict {
                kind: "registration_batch_candidate_outside_root",
                id: item.source_digest.clone(),
            });
        }
        let (candidate, evidence) = self.classify_git_candidate(
            project_id,
            item.submitted_path.clone(),
            canonical_path,
            &[],
        )?;
        let Some(evidence) = evidence else {
            return Err(ProductStoreError::Conflict {
                kind: "registration_batch_candidate_not_git",
                id: item.source_digest.clone(),
            });
        };
        if candidate.canonical_path.as_deref() != Some(item.canonical_path.as_path())
            || candidate.git_root.as_deref() != Some(item.git_root.as_path())
            || candidate.source_identity.as_ref() != Some(&item.source_identity)
            || evidence.source_key_digest != item.source_identity.key_digest
        {
            return Err(ProductStoreError::Conflict {
                kind: "registration_batch_candidate_identity_changed",
                id: item.source_digest.clone(),
            });
        }
        Ok(candidate)
    }

    #[cfg(test)]
    fn should_interrupt_after_completed_item(&self) -> bool {
        self.failure_after_completed_items
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                if remaining == usize::MAX || remaining == 0 {
                    None
                } else {
                    Some(remaining.saturating_sub(1))
                }
            })
            .is_ok_and(|previous| previous == 1)
    }

    /// Reads a submitted manifest (or discovers child Git directories when the
    /// manifest is empty) and classifies every candidate independently.
    /// This method only invokes read-only Git probes and never changes a
    /// checkout, index, ref, config, or branch.
    pub fn preflight(
        &self,
        input: RegistrationPreflightInput,
    ) -> Result<RegistrationPreflightResult, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        let submitted_paths = if input.paths.is_empty() {
            discover_git_directories(&input.aggregate_root.canonical_path)?
        } else {
            input.paths
        };
        let mut candidates = Vec::with_capacity(submitted_paths.len());
        let mut seen = Vec::new();

        for submitted_path in submitted_paths {
            let (candidate, evidence) = match fs::canonicalize(&submitted_path) {
                Err(_) => (RegistrationCandidate::missing(submitted_path), None),
                Ok(canonical_path)
                    if !canonical_path.starts_with(&input.aggregate_root.canonical_path) =>
                {
                    (
                        RegistrationCandidate::outside_root(submitted_path, canonical_path),
                        None,
                    )
                }
                Ok(canonical_path) => self.classify_git_candidate(
                    &input.project_id,
                    submitted_path,
                    canonical_path,
                    &seen,
                )?,
            };
            if let Some(evidence) = evidence {
                seen.push(evidence);
            }
            candidates.push(candidate);
        }

        Ok(RegistrationPreflightResult {
            project_id: input.project_id,
            aggregate_root: input.aggregate_root,
            candidates,
        })
    }

    fn classify_git_candidate(
        &self,
        project_id: &str,
        submitted_path: PathBuf,
        canonical_path: PathBuf,
        seen: &[GitCandidateEvidence],
    ) -> Result<(RegistrationCandidate, Option<GitCandidateEvidence>), ProductStoreError> {
        if !canonical_path.is_dir() {
            return Ok((
                RegistrationCandidate::new(
                    submitted_path,
                    Some(canonical_path),
                    None,
                    None,
                    RegistrationCandidateState::NonGit,
                    "not_git_repository",
                    None,
                    None,
                ),
                None,
            ));
        }

        let Some(git_root) = git_probe(&canonical_path, &["rev-parse", "--show-toplevel"])? else {
            return Ok((
                RegistrationCandidate::new(
                    submitted_path,
                    Some(canonical_path),
                    None,
                    None,
                    RegistrationCandidateState::NonGit,
                    "not_git_repository",
                    None,
                    None,
                ),
                None,
            ));
        };
        let git_root = fs::canonicalize(git_root.trim()).map_err(|error| {
            ProductStoreError::Io(format!(
                "canonicalize Git root reported for {}: {error}",
                canonical_path.display()
            ))
        })?;
        let git_dir = git_probe(&canonical_path, &["rev-parse", "--git-dir"])?
            .ok_or_else(|| git_probe_inconsistent(&canonical_path, "git_dir"))?;
        let git_dir = PathBuf::from(git_dir.trim());
        let git_dir = if git_dir.is_absolute() {
            git_dir
        } else {
            canonical_path.join(git_dir)
        };
        let canonical_git_dir = fs::canonicalize(&git_dir).map_err(|error| {
            ProductStoreError::Io(format!(
                "canonicalize Git directory {} reported for {}: {error}",
                git_dir.display(),
                canonical_path.display()
            ))
        })?;
        let canonical_origin =
            git_probe(&canonical_path, &["config", "--get", "remote.origin.url"])?.and_then(
                |origin| {
                    let origin = origin.trim();
                    (!origin.is_empty()).then(|| origin.to_string())
                },
            );
        let status = git_probe(&canonical_path, &["status", "--porcelain"])?
            .ok_or_else(|| git_probe_inconsistent(&canonical_path, "status"))?;
        // An unborn repository is still a Git repository. Its absent HEAD is
        // represented by an empty component in the revision digest.
        let head = git_probe(&canonical_path, &["rev-parse", "HEAD"])?;
        let source_identity = RepositorySourceIdentity::from_git_parts(
            &canonical_path,
            canonical_git_dir.clone(),
            canonical_origin,
        );
        let evidence = GitCandidateEvidence {
            git_root: git_root.clone(),
            canonical_git_dir,
            source_key_digest: source_identity.key_digest.clone(),
        };

        let duplicate_reason = if seen.iter().any(|prior| {
            prior.canonical_git_dir == evidence.canonical_git_dir
                || prior.source_key_digest == evidence.source_key_digest
        }) {
            Some("duplicate_source_identity")
        } else if IdentityRegistryStore::new(self.paths.clone())
            .find_by_source(project_id, &source_identity)?
            .is_some()
        {
            Some("already_registered")
        } else {
            None
        };
        let nested = seen.iter().any(|prior| {
            git_root.starts_with(&prior.git_root) || prior.git_root.starts_with(&git_root)
        });

        let (state, reason) = if let Some(reason) = duplicate_reason {
            (RegistrationCandidateState::Duplicate, reason)
        } else if nested {
            (RegistrationCandidateState::Nested, "nested_repository")
        } else if !status.is_empty() {
            (RegistrationCandidateState::NeedsAttention, "dirty_worktree")
        } else {
            (RegistrationCandidateState::Eligible, "eligible")
        };

        Ok((
            RegistrationCandidate::new(
                submitted_path,
                Some(canonical_path),
                Some(git_root),
                Some(source_identity),
                state,
                reason,
                head.as_deref().map(str::trim),
                Some(&status),
            ),
            Some(evidence),
        ))
    }

    pub fn attach_member(
        &self,
        input: AttachOnlyRegistrationInput,
    ) -> Result<CodebaseMemberRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.idempotency_key)?;
        if !self.feature.is_enabled() {
            return Err(ProductStoreError::Conflict {
                kind: "logical_codebase_feature_disabled",
                id: input.project_id,
            });
        }

        let repository = self.repositories.create(CreateRepositoryInput {
            project_id: input.project_id.clone(),
            name: input.alias.clone(),
            path: input.canonical_path,
            default_policy_preset: None,
            default_provider_mode: None,
            idempotency_key: input.idempotency_key,
        })?;
        validate_relative_id(&repository.id)?;
        let logical_repository_id = repository.logical_repository_id.ok_or_else(|| {
            ProductStoreError::IdentityMismatch {
                kind: "repository_projection",
                id: repository.id.clone(),
            }
        })?;

        let store = LogicalCodebaseStore::new(self.paths.clone());
        let mut member = store
            .load_member(&input.project_id, logical_repository_id)?
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "logical_codebase_member",
                id: logical_repository_id.0.to_string(),
            })?;
        if member.physical_repository_id != repository.id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "logical_codebase_member",
                id: logical_repository_id.0.to_string(),
            });
        }

        member.alias = input.alias;
        member.role = input.role;
        member.repo_type = input.repo_type;
        member.tech_stack = input.tech_stack;
        member.updated_at = Utc::now().to_rfc3339();
        store.save_member(&input.project_id, &member)?;
        Ok(member)
    }
}

fn batch_item_from_candidate(
    candidate: &RegistrationCandidate,
    include_needs_attention: bool,
) -> Result<Option<RegistrationBatchItem>, ProductStoreError> {
    let selected = match candidate.state {
        RegistrationCandidateState::Eligible => true,
        RegistrationCandidateState::NeedsAttention => include_needs_attention,
        _ => false,
    };
    if !selected {
        return Ok(None);
    }
    let canonical_path =
        candidate
            .canonical_path
            .clone()
            .ok_or_else(|| ProductStoreError::InvalidRecord {
                kind: "confirmed_registration_candidate",
                reason: "selected candidate is missing a canonical path".to_string(),
            })?;
    let git_root = candidate
        .git_root
        .clone()
        .ok_or_else(|| ProductStoreError::InvalidRecord {
            kind: "confirmed_registration_candidate",
            reason: "selected candidate is missing a Git root".to_string(),
        })?;
    let source_identity =
        candidate
            .source_identity
            .clone()
            .ok_or_else(|| ProductStoreError::InvalidRecord {
                kind: "confirmed_registration_candidate",
                reason: "selected candidate is missing a source identity".to_string(),
            })?;
    Ok(Some(RegistrationBatchItem {
        source_digest: source_identity.key_digest.clone(),
        submitted_path: candidate.submitted_path.clone(),
        canonical_path,
        git_root,
        source_identity,
        preflight_revision: candidate.preflight_revision.clone(),
        alias: stable_alias(candidate),
        role: "repository".to_string(),
        repo_type: RepositoryType::Unknown,
        tech_stack: Vec::new(),
        status: RegistrationItemStatus::Pending,
        failure_reason: None,
        retry_count: 0,
    }))
}

fn canonical_manifest_digest(
    aggregate_root: &CanonicalAggregateRoot,
    items: &[RegistrationBatchItem],
) -> String {
    let mut sources = items
        .iter()
        .map(|item| item.source_digest.as_str())
        .collect::<Vec<_>>();
    sources.sort_unstable();
    sha256_key(format!(
        "{}\0{}",
        aggregate_root.canonical_path.to_string_lossy(),
        sources.join("\0")
    ))
}

fn batch_idempotency_key(
    project_id: &str,
    canonical_manifest_digest: &str,
    sorted_revisions: &[&str],
) -> String {
    sha256_key(format!(
        "{}\0{}\0{}",
        project_id,
        canonical_manifest_digest,
        sorted_revisions.join("\0")
    ))
}

fn batch_item_idempotency_key(batch_id: &str, source_digest: &str) -> String {
    format!("batch:{batch_id}:item:{source_digest}")
}

fn batch_failure_reason(error: &ProductStoreError) -> String {
    match error {
        ProductStoreError::Conflict { kind, .. }
        | ProductStoreError::NotFound { kind, .. }
        | ProductStoreError::Ambiguous { kind, .. }
        | ProductStoreError::IdentityMismatch { kind, .. }
        | ProductStoreError::InvalidRecord { kind, .. } => (*kind).to_string(),
        ProductStoreError::Io(_) => "product_store_io".to_string(),
        ProductStoreError::Json(_) => "product_store_json".to_string(),
        ProductStoreError::PathEscape(_) => "product_store_path_escape".to_string(),
    }
}

fn replace_batch_item(
    batch: &mut RegistrationBatchRecord,
    replacement: RegistrationBatchItem,
) -> Result<(), ProductStoreError> {
    let Some(item) = batch
        .items
        .iter_mut()
        .find(|item| item.source_digest == replacement.source_digest)
    else {
        return Err(ProductStoreError::IdentityMismatch {
            kind: "registration_batch_item",
            id: replacement.source_digest,
        });
    };
    *item = replacement;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryGitSnapshot {
    status_porcelain: Vec<u8>,
    head: Vec<u8>,
    refs: Vec<u8>,
    worktree_list: Vec<u8>,
    config: Vec<u8>,
    hooks: Vec<(PathBuf, Vec<u8>)>,
    index: Option<Vec<u8>>,
}

impl RepositoryGitSnapshot {
    pub fn capture(root: &Path) -> Result<Self, ProductStoreError> {
        Ok(Self {
            status_porcelain: git_stdout(root, &["status", "--porcelain"])?,
            head: git_stdout(root, &["rev-parse", "HEAD"])?,
            refs: git_stdout(root, &["for-each-ref", "--format=%(refname) %(objectname)"])?,
            worktree_list: git_stdout(root, &["worktree", "list", "--porcelain"])?,
            config: fs::read(root.join(".git/config")).unwrap_or_default(),
            hooks: read_tree_bytes(&root.join(".git/hooks"))?,
            index: fs::read(root.join(".git/index")).ok(),
        })
    }

    pub fn assert_unchanged(&self, after: &Self) -> Result<(), ProductStoreError> {
        if self == after {
            Ok(())
        } else {
            Err(ProductStoreError::IdentityMismatch {
                kind: "registration_git_side_effect",
                id: "git_snapshot_changed".into(),
            })
        }
    }
}

#[derive(Debug, Clone)]
struct GitCandidateEvidence {
    git_root: PathBuf,
    canonical_git_dir: PathBuf,
    source_key_digest: String,
}

fn discover_git_directories(root: &Path) -> Result<Vec<PathBuf>, ProductStoreError> {
    let mut directories = Vec::new();
    discover_git_directories_recursive(root, root, &mut directories)?;
    directories.sort();
    Ok(directories)
}

fn discover_git_directories_recursive(
    root: &Path,
    directory: &Path,
    directories: &mut Vec<PathBuf>,
) -> Result<(), ProductStoreError> {
    let entries = fs::read_dir(directory).map_err(|error| {
        ProductStoreError::Io(format!(
            "read aggregate directory {}: {error}",
            directory.display()
        ))
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            ProductStoreError::Io(format!(
                "read aggregate directory entry {}: {error}",
                directory.display()
            ))
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| {
            ProductStoreError::Io(format!(
                "inspect aggregate entry {}: {error}",
                path.display()
            ))
        })?;
        if !file_type.is_dir() || path.file_name().is_some_and(|name| name == ".git") {
            continue;
        }
        if path == root {
            continue;
        }
        if path.join(".git").exists() {
            directories.push(path.clone());
        }
        discover_git_directories_recursive(root, &path, directories)?;
    }
    Ok(())
}

fn git_probe(
    repository_path: &Path,
    arguments: &[&str],
) -> Result<Option<String>, ProductStoreError> {
    let allowed = [
        ["status", "--porcelain"].as_slice(),
        ["rev-parse", "HEAD"].as_slice(),
        ["for-each-ref", "--format=%(refname) %(objectname)"].as_slice(),
        ["worktree", "list", "--porcelain"].as_slice(),
        ["rev-parse", "--show-toplevel"].as_slice(),
        ["rev-parse", "--git-dir"].as_slice(),
        ["config", "--get", "remote.origin.url"].as_slice(),
    ];
    if !allowed.iter().any(|candidate| *candidate == arguments) {
        return Err(ProductStoreError::InvalidRecord {
            kind: "registration_git_command",
            reason: format!("Git command is not allowed: {arguments:?}"),
        });
    }
    let output = Command::new("git")
        .current_dir(repository_path)
        .args(arguments)
        .output()
        .map_err(|error| {
            ProductStoreError::Io(format!("run git in {}: {error}", repository_path.display()))
        })?;
    if !output.status.success() {
        return Ok(None);
    }
    String::from_utf8(output.stdout)
        .map(Some)
        .map_err(|error| ProductStoreError::Io(format!("Git output was not UTF-8: {error}")))
}

fn git_stdout(root: &Path, arguments: &[&str]) -> Result<Vec<u8>, ProductStoreError> {
    let allowed = [
        ["status", "--porcelain"].as_slice(),
        ["rev-parse", "HEAD"].as_slice(),
        ["for-each-ref", "--format=%(refname) %(objectname)"].as_slice(),
        ["worktree", "list", "--porcelain"].as_slice(),
        ["rev-parse", "--show-toplevel"].as_slice(),
        ["rev-parse", "--git-dir"].as_slice(),
        ["config", "--get", "remote.origin.url"].as_slice(),
    ];
    if !allowed.iter().any(|candidate| *candidate == arguments) {
        return Err(ProductStoreError::InvalidRecord {
            kind: "registration_git_command",
            reason: format!("Git command is not allowed: {arguments:?}"),
        });
    }
    let output = Command::new("git")
        .current_dir(root)
        .args(arguments)
        .output()
        .map_err(|error| {
            ProductStoreError::Io(format!("run git in {}: {error}", root.display()))
        })?;
    if !output.status.success() {
        return Err(ProductStoreError::Io(format!(
            "git exited {:?} in {}: {}",
            output.status.code(),
            root.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}

fn read_tree_bytes(root: &Path) -> Result<Vec<(PathBuf, Vec<u8>)>, ProductStoreError> {
    fn visit(
        root: &Path,
        path: &Path,
        files: &mut Vec<(PathBuf, Vec<u8>)>,
    ) -> Result<(), ProductStoreError> {
        let entries = match fs::read_dir(path) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "read Git hooks {}: {error}",
                    path.display()
                )));
            }
        };
        let mut paths = entries
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| ProductStoreError::Io(format!("read Git hooks entry: {error}")))?;
        paths.sort();
        for child in paths {
            let metadata = fs::symlink_metadata(&child).map_err(|error| {
                ProductStoreError::Io(format!("inspect Git hooks {}: {error}", child.display()))
            })?;
            if metadata.is_dir() {
                visit(root, &child, files)?;
            } else if metadata.is_file() {
                let relative = child.strip_prefix(root).map_err(|error| {
                    ProductStoreError::Io(format!(
                        "relativize Git hooks {}: {error}",
                        child.display()
                    ))
                })?;
                files.push((
                    relative.to_path_buf(),
                    fs::read(&child).map_err(|error| {
                        ProductStoreError::Io(format!("read Git hook {}: {error}", child.display()))
                    })?,
                ));
            }
        }
        Ok(())
    }

    let mut files = Vec::new();
    if root.exists() {
        visit(root, root, &mut files)?;
    }
    Ok(files)
}

fn git_probe_inconsistent(repository_path: &Path, observation: &str) -> ProductStoreError {
    ProductStoreError::Io(format!(
        "Git probe for {} succeeded without {observation} in {}",
        repository_path.display(),
        repository_path.display()
    ))
}

fn preflight_revision(
    canonical_path: Option<&Path>,
    git_root: Option<&Path>,
    source_identity: Option<&RepositorySourceIdentity>,
    head: Option<&str>,
    status: Option<&str>,
) -> String {
    let status_digest = format!(
        "sha256:{:x}",
        Sha256::digest(status.unwrap_or_default().as_bytes())
    );
    let payload = format!(
        "{}\0{}\0{}\0{}\0{}",
        canonical_path
            .map(|path| path.to_string_lossy())
            .unwrap_or_default(),
        git_root
            .map(|path| path.to_string_lossy())
            .unwrap_or_default(),
        source_identity
            .map(|identity| identity.key_digest.as_str())
            .unwrap_or_default(),
        head.unwrap_or_default(),
        status_digest,
    );
    format!("sha256:{:x}", Sha256::digest(payload.as_bytes()))
}

/// The repository profile observed from deterministic, read-only filesystem signals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedRepositoryProfile {
    pub repo_type: RepositoryType,
    pub tech_stack: Vec<String>,
    pub initialization_commands: Vec<String>,
}

/// Detects member technology without invoking package managers, build tools, or
/// repository initialization commands.
pub struct RepositoryProfileDetector;

impl RepositoryProfileDetector {
    pub fn detect(root: &Path) -> Result<DetectedRepositoryProfile, ProductStoreError> {
        let package_json = root.join("package.json").is_file();
        let pnpm =
            root.join("pnpm-lock.yaml").is_file() || root.join("pnpm-workspace.yaml").is_file();
        let vite = ["vite.config.ts", "vite.config.js", "vite.config.mts"]
            .iter()
            .any(|name| root.join(name).is_file());

        if package_json {
            let mut tech_stack = vec!["package.json".to_string()];
            if pnpm {
                tech_stack.push("pnpm".to_string());
            }
            if vite {
                tech_stack.push("vite".to_string());
            }
            let repo_type = if vite {
                RepositoryType::Frontend
            } else {
                RepositoryType::Library
            };
            return Ok(DetectedRepositoryProfile {
                repo_type,
                tech_stack,
                initialization_commands: Vec::new(),
            });
        }

        Ok(DetectedRepositoryProfile {
            repo_type: RepositoryType::Unknown,
            tech_stack: Vec::new(),
            initialization_commands: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::{
        LogicalCodebaseFeature, LogicalCodebaseManifest, LogicalCodebaseStore,
    };
    use crate::product::project_store::{CreateProjectInput, ProjectStore};
    use crate::product::repository_store::RepositoryStore;

    #[test]
    fn frontend_and_lib_manifest_profiles_are_detected_without_java_initialization() {
        let fixture = tempfile::tempdir().unwrap();
        let web = fixture.path().join("web");
        let shared = fixture.path().join("shared");
        fs::create_dir_all(&web).unwrap();
        fs::create_dir_all(&shared).unwrap();
        fs::write(
            web.join("package.json"),
            r#"{"name":"web","scripts":{"test":"vitest"}}"#,
        )
        .unwrap();
        fs::write(web.join("pnpm-lock.yaml"), "lockfileVersion: '9.0'").unwrap();
        fs::write(web.join("vite.config.ts"), "export default {}").unwrap();
        fs::write(shared.join("package.json"), r#"{"name":"shared"}"#).unwrap();

        let frontend = RepositoryProfileDetector::detect(&web).unwrap();
        let library = RepositoryProfileDetector::detect(&shared).unwrap();
        let parsed: RepositoryType = serde_json::from_str("\"lib\"").unwrap();

        assert_eq!(frontend.repo_type, RepositoryType::Frontend);
        assert_eq!(frontend.tech_stack, vec!["package.json", "pnpm", "vite"]);
        assert!(frontend.initialization_commands.is_empty());
        assert_eq!(library.repo_type, RepositoryType::Library);
        assert!(library.initialization_commands.is_empty());
        assert_eq!(parsed, RepositoryType::Library);
    }

    struct AttachFixture {
        _root: tempfile::TempDir,
        paths: ProductAppPaths,
        coordinator: LogicalCodebaseRegistrationCoordinator,
        git_root: PathBuf,
        head_before: String,
        branch_before: String,
        status_before: String,
    }

    struct ScanFixture {
        _temp: tempfile::TempDir,
        paths: ProductAppPaths,
        root: CanonicalAggregateRoot,
        clean_git: PathBuf,
        non_git: PathBuf,
        nested_git: PathBuf,
        dirty_git: PathBuf,
        missing: PathBuf,
        outside: PathBuf,
        coordinator: LogicalCodebaseRegistrationCoordinator,
    }

    struct BatchFixture {
        _root: tempfile::TempDir,
        root: CanonicalAggregateRoot,
        first: PathBuf,
        second: PathBuf,
        coordinator: LogicalCodebaseRegistrationCoordinator,
    }

    struct FiftyRepositoryFixture {
        _root: tempfile::TempDir,
        root: CanonicalAggregateRoot,
        repositories: Vec<PathBuf>,
        coordinator: LogicalCodebaseRegistrationCoordinator,
    }

    impl FiftyRepositoryFixture {
        fn confirmed_preflight(&self) -> ConfirmedRegistrationBatchInput {
            ConfirmedRegistrationBatchInput::from_preflight(
                &self
                    .coordinator
                    .preflight(RegistrationPreflightInput {
                        project_id: "project_0001".to_string(),
                        aggregate_root: self.root.clone(),
                        paths: self.repositories.clone(),
                    })
                    .unwrap(),
                false,
            )
        }
    }

    impl BatchFixture {
        fn preflight(&self) -> RegistrationPreflightResult {
            self.coordinator
                .preflight(RegistrationPreflightInput {
                    project_id: "project_0001".to_string(),
                    aggregate_root: self.root.clone(),
                    paths: vec![self.first.clone(), self.second.clone()],
                })
                .unwrap()
        }

        fn fail_after_first_completed_item(&self) {
            self.coordinator
                .failure_after_completed_items
                .store(1, Ordering::SeqCst);
        }

        fn change_head_of_second_repository(&self) {
            fs::write(self.second.join("README.md"), "# Second changed\n").unwrap();
            git(&self.second, &["add", "README.md"]);
            git(&self.second, &["commit", "--quiet", "-m", "changed"]);
        }
    }

    #[test]
    fn registering_fifty_members_preserves_every_git_checkout_byte_for_byte() {
        let fixture = fifty_repository_fixture();
        let before: Vec<_> = fixture
            .repositories
            .iter()
            .map(|root| RepositoryGitSnapshot::capture(root).unwrap())
            .collect();
        let batch = fixture
            .coordinator
            .submit_confirmed_batch(fixture.confirmed_preflight())
            .unwrap();
        let completed = fixture
            .coordinator
            .resume_batch("project_0001", &batch.id)
            .unwrap();
        assert_eq!(completed.status, RegistrationBatchStatus::Completed);

        for (root, snapshot) in fixture.repositories.iter().zip(before.iter()) {
            snapshot
                .assert_unchanged(&RepositoryGitSnapshot::capture(root).unwrap())
                .unwrap();
            assert!(!root.join(".aria").exists());
            assert!(!root.join(".codegraph").exists());
        }
    }

    #[test]
    fn resumed_batch_revalidates_revision_skips_completed_item_and_marks_changed_item_for_reconfirmation()
     {
        let fixture = batch_fixture_with_two_git_repositories();
        let preflight = fixture.preflight();
        let batch = fixture
            .coordinator
            .submit_confirmed_batch(ConfirmedRegistrationBatchInput::from_preflight(
                &preflight, true,
            ))
            .unwrap();
        fixture.fail_after_first_completed_item();
        let interrupted = fixture
            .coordinator
            .resume_batch("project_0001", &batch.id)
            .unwrap();
        assert_eq!(interrupted.status, RegistrationBatchStatus::PartialFailed);

        fixture.change_head_of_second_repository();

        let resumed = fixture
            .coordinator
            .resume_batch("project_0001", &batch.id)
            .unwrap();
        let first = resumed
            .items
            .iter()
            .find(|item| item.canonical_path == fixture.first)
            .expect("first repository batch item");
        let second = resumed
            .items
            .iter()
            .find(|item| item.canonical_path == fixture.second)
            .expect("second repository batch item");
        assert_eq!(first.status, RegistrationItemStatus::Completed);
        assert_eq!(second.status, RegistrationItemStatus::NeedsAttention);
        assert_eq!(
            second.failure_reason.as_deref(),
            Some("preflight_revision_changed")
        );
    }

    #[test]
    fn preflight_groups_mixed_manifest_and_marks_dirty_repository_needs_attention() {
        let fixture = scan_fixture();
        let result = fixture
            .coordinator
            .preflight(RegistrationPreflightInput {
                project_id: "project_0001".into(),
                aggregate_root: fixture.root.clone(),
                paths: vec![
                    fixture.clean_git.clone(),
                    fixture.non_git.clone(),
                    fixture.clean_git.clone(),
                    fixture.nested_git.clone(),
                    fixture.dirty_git.clone(),
                    fixture.missing.clone(),
                    fixture.outside.clone(),
                ],
            })
            .unwrap();

        assert_eq!(result.count(RegistrationCandidateState::Eligible), 1);
        assert_eq!(result.count(RegistrationCandidateState::NonGit), 1);
        assert_eq!(result.count(RegistrationCandidateState::Duplicate), 1);
        assert_eq!(result.count(RegistrationCandidateState::Nested), 1);
        assert_eq!(result.count(RegistrationCandidateState::NeedsAttention), 1);
        assert_eq!(result.count(RegistrationCandidateState::Missing), 1);
        assert_eq!(result.count(RegistrationCandidateState::OutsideRoot), 1);
    }

    #[test]
    fn preflight_discovers_direct_child_repositories_and_registry_duplicates() {
        let fixture = scan_fixture();
        let initial = fixture
            .coordinator
            .preflight(RegistrationPreflightInput {
                project_id: "project_0001".into(),
                aggregate_root: fixture.root.clone(),
                paths: vec![],
            })
            .unwrap();
        assert_eq!(initial.count(RegistrationCandidateState::Eligible), 1);
        assert_eq!(initial.count(RegistrationCandidateState::NeedsAttention), 1);
        assert_eq!(initial.count(RegistrationCandidateState::Nested), 1);

        let clean = initial
            .candidates
            .iter()
            .find(|candidate| candidate.state == RegistrationCandidateState::Eligible)
            .unwrap();
        let source_identity = clean.source_identity.clone().unwrap();
        IdentityRegistryStore::new(fixture.paths.clone())
            .upsert_active(
                "project_0001",
                crate::product::logical_codebase::IdentityRegistryEntry::active(
                    source_identity,
                    crate::product::logical_codebase::LogicalRepositoryId(uuid::Uuid::nil()),
                    "repository_0001".into(),
                    crate::product::logical_codebase::RepositoryCheckoutId(uuid::Uuid::nil()),
                    "test-create".into(),
                ),
            )
            .unwrap();

        let duplicate = fixture
            .coordinator
            .preflight(RegistrationPreflightInput {
                project_id: "project_0001".into(),
                aggregate_root: fixture.root.clone(),
                paths: vec![fixture.clean_git.clone()],
            })
            .unwrap();
        assert_eq!(duplicate.count(RegistrationCandidateState::Duplicate), 1);
        assert_eq!(duplicate.candidates[0].reason, "already_registered");
    }

    #[test]
    fn preflight_revisions_change_with_head_and_worktree_status() {
        let fixture = scan_fixture();
        let first = fixture
            .coordinator
            .preflight(RegistrationPreflightInput {
                project_id: "project_0001".into(),
                aggregate_root: fixture.root.clone(),
                paths: vec![fixture.clean_git.clone()],
            })
            .unwrap();
        fs::write(fixture.clean_git.join("README.md"), "changed\n").unwrap();
        let dirty = fixture
            .coordinator
            .preflight(RegistrationPreflightInput {
                project_id: "project_0001".into(),
                aggregate_root: fixture.root.clone(),
                paths: vec![fixture.clean_git.clone()],
            })
            .unwrap();
        assert_eq!(
            dirty.candidates[0].state,
            RegistrationCandidateState::NeedsAttention
        );
        assert_ne!(
            first.candidates[0].preflight_revision,
            dirty.candidates[0].preflight_revision
        );

        git(&fixture.clean_git, &["add", "README.md"]);
        git(&fixture.clean_git, &["commit", "--quiet", "-m", "changed"]);
        let committed = fixture
            .coordinator
            .preflight(RegistrationPreflightInput {
                project_id: "project_0001".into(),
                aggregate_root: fixture.root.clone(),
                paths: vec![fixture.clean_git.clone()],
            })
            .unwrap();
        assert_eq!(
            committed.candidates[0].state,
            RegistrationCandidateState::Eligible
        );
        assert_ne!(
            dirty.candidates[0].preflight_revision,
            committed.candidates[0].preflight_revision
        );
    }

    #[test]
    fn attach_only_registration_creates_member_without_initialization_operation() {
        let fixture = attach_fixture();
        let member = fixture
            .coordinator
            .attach_member(AttachOnlyRegistrationInput {
                project_id: "project_0001".into(),
                alias: "api".into(),
                role: "service".into(),
                canonical_path: fixture.git_root.clone(),
                repo_type: RepositoryType::Backend,
                tech_stack: vec!["rust".into()],
                idempotency_key: "batch-1:item-api".into(),
            })
            .unwrap();

        assert_eq!(member.alias, "api");
        assert_eq!(member.role, "service");
        assert_eq!(member.repo_type, RepositoryType::Backend);
        assert_eq!(member.tech_stack, vec!["rust"]);
        assert!(
            !fixture
                .paths
                .repository_initializations_root("project_0001")
                .exists()
        );
        assert_eq!(
            git_output(&fixture.git_root, &["rev-parse", "HEAD"]),
            fixture.head_before
        );
        assert_eq!(
            git_output(&fixture.git_root, &["branch", "--show-current"]),
            fixture.branch_before
        );
        assert_eq!(
            git_output(&fixture.git_root, &["status", "--porcelain"]),
            fixture.status_before
        );
    }

    #[test]
    #[cfg(unix)]
    fn aggregate_root_preflight_rejects_git_root_symlink_escape_and_ownership_conflicts() {
        let fixture = aggregate_root_fixture();
        fixture.init_git_at_root();
        assert_eq!(
            fixture
                .preflight()
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&fixture.member),
                )
                .unwrap_err()
                .code(),
            "aggregate_root_is_git"
        );

        fixture.remove_root_git();
        fixture.create_symlink_member_to_outside_root();
        assert_eq!(
            fixture
                .preflight()
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&fixture.symlink_member),
                )
                .unwrap_err()
                .code(),
            "member_symlink_escape"
        );

        fixture.create_file("CLAUDE.md", "user instructions");
        assert_eq!(
            fixture
                .preflight()
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&fixture.member),
                )
                .unwrap_err()
                .code(),
            "aggregate_root_ownership_conflict"
        );
    }

    #[test]
    #[cfg(unix)]
    fn aggregate_root_preflight_rejects_members_outside_root_nested_worktrees_and_overlap() {
        let fixture = aggregate_root_fixture();
        let outside_member = fixture.root.parent().unwrap().join("outside-member");
        fs::create_dir(&outside_member).unwrap();
        assert_eq!(
            fixture
                .preflight()
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&outside_member),
                )
                .unwrap_err()
                .code(),
            "member_path_outside_root"
        );
        assert!(
            fixture
                .preflight()
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&fixture.root),
                )
                .is_err_and(|error| error.code() == "member_path_outside_root")
        );

        fixture.init_git_at_member();
        fixture.create_nested_worktree_at_member();
        assert_eq!(
            fixture
                .preflight()
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&fixture.nested_worktree_member),
                )
                .unwrap_err()
                .code(),
            "nested_worktree"
        );

        fixture.remove_nested_worktree();
        fixture.save_manifest_for("project_0002", fixture.root.parent().unwrap());
        assert_eq!(
            fixture
                .preflight()
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&fixture.member),
                )
                .unwrap_err()
                .code(),
            "aggregate_root_overlap"
        );
    }

    #[test]
    #[cfg(unix)]
    fn aggregate_root_preflight_accepts_non_git_root_with_direct_git_members() {
        let fixture = aggregate_root_fixture();
        fixture.init_git_at_member();

        assert_eq!(
            fixture
                .preflight()
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&fixture.member),
                )
                .unwrap()
                .canonical_path,
            fs::canonicalize(&fixture.root).unwrap()
        );
    }

    struct AggregateRootFixture {
        _temp: tempfile::TempDir,
        paths: ProductAppPaths,
        root: PathBuf,
        member: PathBuf,
        symlink_member: PathBuf,
        nested_worktree_member: PathBuf,
    }

    impl AggregateRootFixture {
        fn preflight(&self) -> AggregateRootPreflight {
            AggregateRootPreflight::new(self.paths.clone())
        }

        fn create_file(&self, relative_path: &str, contents: &str) {
            fs::write(self.root.join(relative_path), contents).unwrap();
        }

        fn init_git_at_root(&self) {
            git(&self.root, &["init", "--quiet"]);
        }

        fn remove_root_git(&self) {
            fs::remove_dir_all(self.root.join(".git")).unwrap();
        }

        fn init_git_at_member(&self) {
            git(&self.member, &["init", "--quiet"]);
            git(
                &self.member,
                &["config", "user.email", "aria@example.invalid"],
            );
            git(&self.member, &["config", "user.name", "Aria Test"]);
            fs::write(self.member.join("README.md"), "# Member\n").unwrap();
            git(&self.member, &["add", "README.md"]);
            git(&self.member, &["commit", "--quiet", "-m", "initial"]);
        }

        fn create_symlink_member_to_outside_root(&self) {
            let outside_member = self.root.parent().unwrap().join("outside-symlink-target");
            fs::create_dir(&outside_member).unwrap();
            symlink(&outside_member, &self.symlink_member).unwrap();
        }

        fn create_nested_worktree_at_member(&self) {
            git(
                &self.member,
                &[
                    "worktree",
                    "add",
                    "--detach",
                    self.nested_worktree_member.to_str().unwrap(),
                ],
            );
        }

        fn remove_nested_worktree(&self) {
            git(
                &self.member,
                &[
                    "worktree",
                    "remove",
                    "--force",
                    self.nested_worktree_member.to_str().unwrap(),
                ],
            );
        }

        fn save_manifest_for(&self, project_id: &str, root: &Path) {
            let manifest = LogicalCodebaseManifest::new(
                project_id,
                fs::canonicalize(root).unwrap(),
                Vec::new(),
            );
            LogicalCodebaseStore::new(self.paths.clone())
                .save_manifest(project_id, &manifest)
                .unwrap();
        }
    }

    fn aggregate_root_fixture() -> AggregateRootFixture {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join("aria-state"));
        let root = temp.path().join("aggregate-root");
        let member = root.join("member");
        let symlink_member = root.join("symlink-member");
        let nested_worktree_member = member.join("nested-worktree");
        fs::create_dir_all(&member).unwrap();
        for name in ["project", "project-other"] {
            ProjectStore::new(paths.clone())
                .create(CreateProjectInput {
                    name: name.to_string(),
                    description: None,
                })
                .unwrap();
        }

        AggregateRootFixture {
            _temp: temp,
            paths,
            root,
            member,
            symlink_member,
            nested_worktree_member,
        }
    }

    fn scan_fixture() -> ScanFixture {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join("aria-state"));
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "project".into(),
                description: None,
            })
            .unwrap();

        let root = temp.path().join("aggregate-root");
        let clean_git = root.join("clean-git");
        let non_git = root.join("non-git");
        let nested_git = clean_git.join("nested-git");
        let dirty_git = root.join("dirty-git");
        let missing = root.join("missing");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&non_git).unwrap();
        fs::create_dir_all(&outside).unwrap();
        init_git_repository(&clean_git);
        fs::write(clean_git.join(".git/info/exclude"), "nested-git/\n").unwrap();
        init_git_repository(&nested_git);
        init_git_repository(&dirty_git);
        fs::write(dirty_git.join("README.md"), "dirty\n").unwrap();

        let repositories = RepositoryStore::with_logical_codebase_feature(
            paths.clone(),
            LogicalCodebaseFeature::enabled(),
        );
        ScanFixture {
            _temp: temp,
            paths: paths.clone(),
            root: CanonicalAggregateRoot {
                canonical_path: fs::canonicalize(root).unwrap(),
            },
            clean_git,
            non_git,
            nested_git,
            dirty_git,
            missing,
            outside,
            coordinator: LogicalCodebaseRegistrationCoordinator::new(
                paths,
                repositories,
                LogicalCodebaseFeature::enabled(),
            ),
        }
    }

    fn fifty_repository_fixture() -> FiftyRepositoryFixture {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join("aria-state"));
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "project".to_string(),
                description: None,
            })
            .unwrap();
        let root_path = temp.path().join("aggregate-root");
        let repositories = (1..=50)
            .map(|index| {
                let path = root_path.join(format!("repo_{index:04}"));
                init_git_repository(&path);
                path
            })
            .collect::<Vec<_>>();
        let repository_store = RepositoryStore::with_logical_codebase_feature(
            paths.clone(),
            LogicalCodebaseFeature::enabled(),
        );
        FiftyRepositoryFixture {
            _root: temp,
            root: CanonicalAggregateRoot {
                canonical_path: fs::canonicalize(root_path).unwrap(),
            },
            repositories,
            coordinator: LogicalCodebaseRegistrationCoordinator::new(
                paths,
                repository_store,
                LogicalCodebaseFeature::enabled(),
            ),
        }
    }

    fn batch_fixture_with_two_git_repositories() -> BatchFixture {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join("aria-state"));
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "project".to_string(),
                description: None,
            })
            .unwrap();
        let root_path = temp.path().join("aggregate-root");
        let first = root_path.join("first");
        let second = root_path.join("second");
        init_git_repository(&first);
        init_git_repository(&second);
        let repositories = RepositoryStore::with_logical_codebase_feature(
            paths.clone(),
            LogicalCodebaseFeature::enabled(),
        );
        BatchFixture {
            _root: temp,
            root: CanonicalAggregateRoot {
                canonical_path: fs::canonicalize(&root_path).unwrap(),
            },
            first,
            second,
            coordinator: LogicalCodebaseRegistrationCoordinator::new(
                paths,
                repositories,
                LogicalCodebaseFeature::enabled(),
            ),
        }
    }

    fn init_git_repository(path: &Path) {
        fs::create_dir_all(path).unwrap();
        git(path, &["init", "--quiet"]);
        git(path, &["config", "user.email", "aria@example.invalid"]);
        git(path, &["config", "user.name", "Aria Test"]);
        fs::write(path.join("README.md"), "# Repository\n").unwrap();
        git(path, &["add", "README.md"]);
        git(path, &["commit", "--quiet", "-m", "initial"]);
    }

    fn attach_fixture() -> AttachFixture {
        let root = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(root.path());
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "project".into(),
                description: None,
            })
            .unwrap();

        let git_root = root.path().join("api");
        fs::create_dir_all(&git_root).unwrap();
        git(&git_root, &["init", "--quiet"]);
        git(&git_root, &["config", "user.email", "aria@example.invalid"]);
        git(&git_root, &["config", "user.name", "Aria Test"]);
        fs::write(git_root.join("README.md"), "# API\n").unwrap();
        git(&git_root, &["add", "README.md"]);
        git(&git_root, &["commit", "--quiet", "-m", "initial"]);

        let head_before = git_output(&git_root, &["rev-parse", "HEAD"]);
        let branch_before = git_output(&git_root, &["branch", "--show-current"]);
        let status_before = git_output(&git_root, &["status", "--porcelain"]);
        let repositories = RepositoryStore::with_logical_codebase_feature(
            paths.clone(),
            LogicalCodebaseFeature::enabled(),
        );

        AttachFixture {
            _root: root,
            paths: paths.clone(),
            coordinator: LogicalCodebaseRegistrationCoordinator::new(
                paths,
                repositories,
                LogicalCodebaseFeature::enabled(),
            ),
            git_root,
            head_before,
            branch_before,
            status_before,
        }
    }

    fn git(cwd: &Path, arguments: &[&str]) {
        let output = Command::new("git")
            .current_dir(cwd)
            .args(arguments)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_output(cwd: &Path, arguments: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(cwd)
            .args(arguments)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }
}
