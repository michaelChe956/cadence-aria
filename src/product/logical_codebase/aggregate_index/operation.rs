use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde_json::Value;
use uuid::Uuid;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::locking::with_exact_exclusive_lock;
use crate::product::json_store::{ProductStoreError, validate_relative_id};
use crate::product::logical_codebase::{
    CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, LogicalCodebaseManifest,
    LogicalCodebaseStore, MemberStatus, RepositoryCheckoutRecord,
};

use super::{
    AggregateIndexError, AggregateIndexRecord, AggregateIndexSnapshotCollector,
    AggregateIndexStatus, AggregateIndexStore, CodeGraphCli, CodeGraphExcludeGenerator,
};

const REPRESENTATIVE_QUERY: &str = "crossRepoGreeting";
const EXCLUDED_QUERIES: [&str; 4] = [
    "SHOULD_NOT_INDEX_NONMEMBER",
    "SHOULD_NOT_INDEX_WORKTREE",
    "SHOULD_NOT_INDEX_ARIA",
    "SHOULD_NOT_INDEX_BUILD",
];

/// Evidence produced only after every CodeGraph scope assertion succeeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregateIndexAcceptance {
    pub member_files: BTreeMap<String, Vec<PathBuf>>,
    pub representative_query: Value,
    pub excluded_queries: BTreeMap<String, Vec<PathBuf>>,
}

impl AggregateIndexAcceptance {
    fn verify(
        cli: &CodeGraphCli,
        root: &Path,
        member_names: &[String],
    ) -> Result<Self, AggregateIndexError> {
        let files = cli.files(root)?;
        let member_files = verify_member_coverage(&files, member_names)?;
        let representative_query = cli.query_json(root, REPRESENTATIVE_QUERY)?;
        verify_cross_member_hit(&representative_query, member_names)?;

        let mut excluded_queries = BTreeMap::new();
        for query in EXCLUDED_QUERIES {
            let result = cli.query_json(root, query)?;
            let offending_paths = result_paths(&result);
            if !is_empty_query_result(&result) {
                return Err(exclusion_failed(query, offending_paths));
            }
            excluded_queries.insert(query.to_string(), offending_paths);
        }

        Ok(Self {
            member_files,
            representative_query,
            excluded_queries,
        })
    }

    fn soft_warning(&self) -> Option<String> {
        None
    }
}

/// Orchestrates immutable configuration publication, CodeGraph initialization,
/// scope verification, and durable active-record publication.
pub struct AggregateIndexOperation {
    logical: LogicalCodebaseStore,
    store: AggregateIndexStore,
    excludes: CodeGraphExcludeGenerator,
    cli: CodeGraphCli,
    snapshots: AggregateIndexSnapshotCollector,
}

impl AggregateIndexOperation {
    pub fn new(
        paths: ProductAppPaths,
        cli: CodeGraphCli,
        excludes: CodeGraphExcludeGenerator,
    ) -> Self {
        Self::with_snapshot_dependencies(
            LogicalCodebaseStore::new(paths.clone()),
            AggregateIndexStore::new(paths.clone()),
            cli,
            excludes,
            AggregateIndexSnapshotCollector::for_paths(paths),
        )
    }

    pub fn with_dependencies(
        logical: LogicalCodebaseStore,
        store: AggregateIndexStore,
        cli: CodeGraphCli,
        excludes: CodeGraphExcludeGenerator,
    ) -> Self {
        Self::with_snapshot_dependencies(
            logical.clone(),
            store,
            cli,
            excludes,
            AggregateIndexSnapshotCollector::with_dependencies(
                logical,
                std::sync::Arc::new(
                    crate::cross_cutting::bounded_command_runner::TokioBoundedCommandRunner,
                ),
            ),
        )
    }

    pub fn with_snapshot_dependencies(
        logical: LogicalCodebaseStore,
        store: AggregateIndexStore,
        cli: CodeGraphCli,
        excludes: CodeGraphExcludeGenerator,
        snapshots: AggregateIndexSnapshotCollector,
    ) -> Self {
        Self {
            logical,
            store,
            excludes,
            cli,
            snapshots,
        }
    }

    /// Clones the logical-codebase store handle for sibling services such as
    /// the freshness service that need independent read access.
    pub fn logical_clone(&self) -> LogicalCodebaseStore {
        self.logical.clone()
    }

    /// Clones the aggregate-index store handle.
    pub fn store_clone(&self) -> AggregateIndexStore {
        self.store.clone()
    }

    /// Clones the snapshot collector handle.
    pub fn snapshots_clone(&self) -> AggregateIndexSnapshotCollector {
        self.snapshots.clone()
    }

    pub fn build(
        &self,
        project_id: &str,
        expected_membership_revision: u64,
    ) -> Result<AggregateIndexRecord, AggregateIndexError> {
        validate_relative_id(project_id)?;
        let manifest = self
            .logical
            .load_manifest(project_id)?
            .ok_or_else(|| missing_manifest(project_id))?;
        if manifest.membership_revision != expected_membership_revision {
            return Err(AggregateIndexError::Failed {
                code: "aggregate_index_membership_revision_mismatch",
                message: format!(
                    "project {project_id} membership revision is {}, expected {expected_membership_revision}",
                    manifest.membership_revision
                ),
            });
        }

        self.with_single_writer(project_id, || {
            self.apply_index(project_id, &manifest, IndexApplicationMode::Initialize)
        })
    }

    /// Rebuilds the aggregate index under the single-writer lock, preserving the
    /// last-known-good generation on failure.
    ///
    /// On success the freshly verified record supersedes the prior active record
    /// via [`AggregateIndexStore::replace_active`]. On failure the newly-created
    /// generation is marked [`AggregateIndexStatus::Stale`] with the failure
    /// reason, while the before-snapshot generation remains the readable active
    /// pointer. The active pointer is never switched on the failure path.
    pub fn rebuild(&self, project_id: &str) -> Result<AggregateIndexRecord, AggregateIndexError> {
        self.with_single_writer(project_id, || {
            let manifest = self
                .logical
                .load_manifest(project_id)?
                .ok_or_else(|| missing_manifest(project_id))?;
            match self.apply_index(project_id, &manifest, IndexApplicationMode::Rebuild) {
                Ok(next) => Ok(next),
                Err(error) => Err(error),
            }
        })
    }

    /// Serializes `operation` against concurrent rebuild/sync writers for the
    /// project by holding an exclusive flock on the project-scoped aggregate
    /// index lock file. Half-written indexes and racing active-pointer flips
    /// are thereby impossible: only one writer mutates the index generation at a
    /// time.
    ///
    /// The original [`AggregateIndexError`] from `operation` is preserved
    /// verbatim (including its `&'static str` code); only lock-acquisition
    /// failures are mapped to a distinct `aggregate_index_lock_error` code.
    pub fn with_single_writer<T>(
        &self,
        project_id: &str,
        operation: impl FnOnce() -> Result<T, AggregateIndexError>,
    ) -> Result<T, AggregateIndexError> {
        validate_relative_id(project_id)?;
        let path = self.store.lock_path(project_id)?;
        // Capture the domain error produced inside the locked section so its
        // `&'static str` code survives the lock boundary; lock-acquisition
        // errors fall through to a distinct code below.
        let mut captured: Option<AggregateIndexError> = None;
        let result = with_exact_exclusive_lock(&path, || match operation() {
            Ok(value) => Ok(value),
            Err(error) => {
                captured = Some(error.clone());
                Err(ProductStoreError::Io(error.to_string()))
            }
        });
        match result {
            Ok(value) => Ok(value),
            Err(_) => {
                if let Some(error) = captured {
                    Err(error)
                } else {
                    Err(AggregateIndexError::Failed {
                        code: "aggregate_index_lock_error",
                        message: format!(
                            "acquire aggregate-index single-writer lock for {project_id} failed"
                        ),
                    })
                }
            }
        }
    }

    /// Refreshes an existing active index after freshness detected a drift.
    ///
    /// The caller (freshness service) supplies the previously-active record so
    /// that on success we supersede it with a freshly verified record. On any
    /// failure the newly-created generation is marked stale with the error and
    /// the original error is propagated; the before-snapshot active generation
    /// is never silently replaced.
    pub fn sync_and_verify(
        &self,
        project_id: &str,
        _prior: AggregateIndexRecord,
    ) -> Result<AggregateIndexRecord, AggregateIndexError> {
        validate_relative_id(project_id)?;
        self.with_single_writer(project_id, || {
            let manifest = self
                .logical
                .load_manifest(project_id)?
                .ok_or_else(|| missing_manifest(project_id))?;

            match self.apply_index(project_id, &manifest, IndexApplicationMode::Sync) {
                Ok(record) => Ok(record),
                Err(error) => Err(error),
            }
        })
    }

    /// Shared body that regenerates the CodeGraph configuration, executes the
    /// index command (`init` for fresh builds, `sync` for incremental refresh),
    /// re-runs member-coverage and negative-query acceptance, captures fresh
    /// member snapshots, and publishes a new active record superseding any prior.
    fn apply_index(
        &self,
        project_id: &str,
        manifest: &LogicalCodebaseManifest,
        mode: IndexApplicationMode,
    ) -> Result<AggregateIndexRecord, AggregateIndexError> {
        let members = self.logical.list_members(project_id)?;
        let checkouts = self.logical.list_checkouts(project_id)?;
        let included = included_main_checkouts(manifest, &members, &checkouts)?;
        let before = self.snapshots.capture_included(project_id, manifest)?;
        let member_names = included
            .iter()
            .map(|(_, checkout)| checkout_root_name(&manifest.provider_context_root, checkout))
            .collect::<Result<Vec<_>, _>>()?;

        let now = Utc::now().to_rfc3339();
        let building = AggregateIndexRecord::building(
            new_index_id(),
            project_id.to_string(),
            manifest.membership_revision,
            before,
            now,
        );
        let building = self.store.create(project_id, building)?;
        let index_id = building.aggregate_index_id.clone();

        let command_result = (|| {
            self.cli.verify_v1_5_0()?;
            let config = self.excludes.generate(manifest, &members, &checkouts)?;
            let config_digest = self
                .excludes
                .write_atomically(&manifest.provider_context_root, &config)?;
            match mode {
                IndexApplicationMode::Initialize => {
                    self.cli.init(&manifest.provider_context_root)?;
                }
                IndexApplicationMode::Sync => {
                    self.cli.sync(&manifest.provider_context_root)?;
                }
                IndexApplicationMode::Rebuild => {
                    self.cli.init(&manifest.provider_context_root)?;
                }
            }
            Ok::<_, AggregateIndexError>(config_digest)
        })();

        let after_result = self.snapshots.capture_included(project_id, manifest);
        let after = match after_result {
            Ok(after) => after,
            Err(error) => {
                return self.fail_building(project_id, &index_id, mode, error, Vec::new());
            }
        };
        let config_digest = match command_result {
            Ok(config_digest) => config_digest,
            Err(error) => return self.fail_building(project_id, &index_id, mode, error, after),
        };
        if snapshots_differ(&building.member_snapshots, &after) {
            return self.fail_building(
                project_id,
                &index_id,
                mode,
                AggregateIndexError::Failed {
                    code: "aggregate_index_member_drifted",
                    message: "member checkout changed while aggregate index was building".into(),
                },
                after,
            );
        }

        let acceptance = match AggregateIndexAcceptance::verify(
            &self.cli,
            &manifest.provider_context_root,
            &member_names,
        ) {
            Ok(acceptance) => acceptance,
            Err(error) => return self.fail_building(project_id, &index_id, mode, error, after),
        };

        let now = Utc::now().to_rfc3339();
        let mut record = building;
        record.observed_after_member_snapshots = after;
        record.status = AggregateIndexStatus::Active;
        record.codegraph_root = manifest.provider_context_root.clone();
        record.config_digest = config_digest;
        record.warning = acceptance.soft_warning();
        record.updated_at = now;
        self.store.replace_active(project_id, record)
    }

    fn fail_building(
        &self,
        project_id: &str,
        index_id: &str,
        mode: IndexApplicationMode,
        error: AggregateIndexError,
        after: Vec<super::AggregateIndexMemberSnapshot>,
    ) -> Result<AggregateIndexRecord, AggregateIndexError> {
        let status = match mode {
            IndexApplicationMode::Initialize => AggregateIndexStatus::Failed,
            IndexApplicationMode::Sync | IndexApplicationMode::Rebuild => {
                AggregateIndexStatus::Stale
            }
        };
        self.store
            .mark_status(project_id, index_id, status, Some(error.to_string()))?;
        if !after.is_empty() {
            let mut record = self.store.get(project_id, index_id)?.ok_or_else(|| {
                AggregateIndexError::Failed {
                    code: "aggregate_index_not_found",
                    message: format!("aggregate index {index_id} was not found"),
                }
            })?;
            record.observed_after_member_snapshots = after;
            record.warning = Some(error.to_string());
            self.store.update_record(project_id, record)?;
        }
        Err(error)
    }
}

fn snapshots_differ(
    before: &[super::AggregateIndexMemberSnapshot],
    after: &[super::AggregateIndexMemberSnapshot],
) -> bool {
    before.len() != after.len()
        || before.iter().zip(after).any(|(before, after)| {
            before.logical_repository_id != after.logical_repository_id
                || before.checkout_id != after.checkout_id
                || before.revision != after.revision
                || before.dirty != after.dirty
                || before.included != after.included
        })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum IndexApplicationMode {
    Initialize,
    Sync,
    Rebuild,
}

fn included_main_checkouts<'a>(
    manifest: &LogicalCodebaseManifest,
    members: &'a [CodebaseMemberRecord],
    checkouts: &'a [RepositoryCheckoutRecord],
) -> Result<Vec<(&'a CodebaseMemberRecord, &'a RepositoryCheckoutRecord)>, AggregateIndexError> {
    let members_by_id = members
        .iter()
        .map(|member| (member.logical_repository_id, member))
        .collect::<BTreeMap<_, _>>();
    let mut included = Vec::with_capacity(manifest.member_ids.len());
    let mut seen_members = BTreeSet::new();

    for member_id in &manifest.member_ids {
        if !seen_members.insert(*member_id) {
            return Err(AggregateIndexError::Failed {
                code: "aggregate_index_member_invalid",
                message: format!("manifest repeats member {}", member_id.0),
            });
        }
        let member =
            members_by_id
                .get(member_id)
                .copied()
                .ok_or_else(|| AggregateIndexError::Failed {
                    code: "aggregate_index_member_invalid",
                    message: format!("manifest member {} has no authority record", member_id.0),
                })?;
        if member.status != MemberStatus::Active {
            return Err(AggregateIndexError::Failed {
                code: "aggregate_index_member_invalid",
                message: format!("manifest member {} is not active", member_id.0),
            });
        }
        let main_checkouts = checkouts
            .iter()
            .filter(|checkout| {
                checkout.logical_repository_id == *member_id && checkout.kind == CheckoutKind::Main
            })
            .collect::<Vec<_>>();
        let [checkout] = main_checkouts.as_slice() else {
            return Err(AggregateIndexError::Failed {
                code: "aggregate_index_member_invalid",
                message: format!(
                    "manifest member {} must have exactly one main checkout, found {}",
                    member_id.0,
                    main_checkouts.len()
                ),
            });
        };
        if !member.checkout_ids.contains(&checkout.checkout_id)
            || member.physical_repository_id != checkout.physical_repository_id
            || checkout.availability != CheckoutAvailability::Available
        {
            return Err(AggregateIndexError::Failed {
                code: "aggregate_index_member_invalid",
                message: format!(
                    "main checkout {} is not an available checkout of member {}",
                    checkout.checkout_id.0, member_id.0
                ),
            });
        }
        included.push((member, *checkout));
    }
    Ok(included)
}

fn checkout_root_name(
    root: &Path,
    checkout: &RepositoryCheckoutRecord,
) -> Result<String, AggregateIndexError> {
    let root = std::fs::canonicalize(root).map_err(|error| layout_error(root, error))?;
    let checkout_path = std::fs::canonicalize(&checkout.canonical_path)
        .map_err(|error| layout_error(&checkout.canonical_path, error))?;
    let name = checkout_path
        .strip_prefix(&root)
        .ok()
        .and_then(|relative| {
            let mut components = relative.components();
            let first = components.next()?;
            components
                .next()
                .is_none()
                .then(|| first.as_os_str().to_str())?
        })
        .ok_or_else(|| AggregateIndexError::Failed {
            code: "aggregate_index_layout_unsupported",
            message: format!(
                "main checkout {} is not a direct child of aggregate root {}",
                checkout_path.display(),
                root.display()
            ),
        })?;
    validate_relative_id(name)?;
    Ok(name.to_string())
}

fn layout_error(path: &Path, error: std::io::Error) -> AggregateIndexError {
    AggregateIndexError::Failed {
        code: "aggregate_index_layout_unsupported",
        message: format!("cannot canonicalize {}: {error}", path.display()),
    }
}

fn verify_member_coverage(
    files: &[PathBuf],
    member_names: &[String],
) -> Result<BTreeMap<String, Vec<PathBuf>>, AggregateIndexError> {
    let mut result = BTreeMap::new();
    for member_name in member_names {
        validate_relative_id(member_name)?;
        let prefix = Path::new(member_name);
        let covered = files
            .iter()
            .filter(|file| file.starts_with(prefix))
            .cloned()
            .collect::<Vec<_>>();
        if covered.is_empty() {
            return Err(AggregateIndexError::Failed {
                code: "aggregate_index_member_coverage_failed",
                message: format!(
                    "codegraph files has no indexed file under included member {member_name}"
                ),
            });
        }
        result.insert(member_name.clone(), covered);
    }
    Ok(result)
}

fn verify_cross_member_hit(
    result: &Value,
    member_names: &[String],
) -> Result<(), AggregateIndexError> {
    let paths = result_paths(result);
    let hit_members = paths
        .iter()
        .filter_map(|path| first_path_component(path))
        .filter(|member| member_names.iter().any(|name| name == member))
        .collect::<BTreeSet<_>>();
    if hit_members.len() < 2 {
        return Err(AggregateIndexError::Failed {
            code: "aggregate_index_cross_member_query_failed",
            message: format!(
                "representative query {REPRESENTATIVE_QUERY} must hit two included members; paths: {}",
                format_paths(&paths)
            ),
        });
    }
    Ok(())
}

fn is_empty_query_result(value: &Value) -> bool {
    matches!(value, Value::Array(values) if values.is_empty())
}

fn exclusion_failed(query: &str, paths: Vec<PathBuf>) -> AggregateIndexError {
    AggregateIndexError::Failed {
        code: "aggregate_index_exclusion_failed",
        message: format!(
            "excluded unique symbol {query} was indexed at: {}",
            format_paths(&paths)
        ),
    }
}

fn result_paths(value: &Value) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    collect_result_paths(value, &mut paths);
    paths.sort();
    paths.dedup();
    paths
}

fn collect_result_paths(value: &Value, paths: &mut Vec<PathBuf>) {
    match value {
        Value::Array(values) => {
            for value in values {
                collect_result_paths(value, paths);
            }
        }
        Value::Object(object) => {
            for key in ["file", "path", "filePath"] {
                if let Some(Value::String(path)) = object.get(key) {
                    paths.push(PathBuf::from(path));
                }
            }
            for value in object.values() {
                collect_result_paths(value, paths);
            }
        }
        _ => {}
    }
}

fn first_path_component(path: &Path) -> Option<&str> {
    path.components().next()?.as_os_str().to_str()
}

fn format_paths(paths: &[PathBuf]) -> String {
    if paths.is_empty() {
        "<no paths returned>".to_string()
    } else {
        paths
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn missing_manifest(project_id: &str) -> AggregateIndexError {
    AggregateIndexError::Failed {
        code: "aggregate_index_manifest_missing",
        message: format!("logical-codebase manifest is missing for project {project_id}"),
    }
}

fn new_index_id() -> String {
    format!("aggregate_index_{}", Uuid::new_v4())
}

#[cfg(test)]
include!("operation_tests.inc.rs");
