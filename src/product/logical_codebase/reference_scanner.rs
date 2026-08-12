use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_models::{
    CodingAttemptPlanBinding, CodingAttemptScope, CodingExecutionAttempt, CodingExecutionUnit,
};
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id};
use crate::product::logical_codebase::{
    IdentityRegistryStore, LogicalCodebaseStore, LogicalRepositoryId,
};
use crate::product::models::{
    IssueRecord, IssueRuntimeBindingRecord, IssueSharedWorktree, LifecycleWorkItemRecord,
    RepositoryProfile, StorySpecRecord,
};

/// A durable record that currently prevents deleting a physical repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RepositoryReference {
    pub kind: String,
    pub record_id: String,
    pub path: String,
}

/// The complete, stable-order set of records that prevent deletion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RepositoryReferenceReport {
    pub blockers: Vec<RepositoryReference>,
}

impl RepositoryReferenceReport {
    pub fn is_empty(&self) -> bool {
        self.blockers.is_empty()
    }
}

/// Scans persisted authority and compatibility projections before a logical
/// repository can be tombstoned. The scan is deliberately conservative: an
/// invalid record is returned as an error rather than silently ignored.
#[derive(Debug, Clone)]
pub struct RepositoryReferenceScanner {
    paths: ProductAppPaths,
}

impl RepositoryReferenceScanner {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    pub fn scan(
        &self,
        project_id: &str,
        physical_repository_id: &str,
        logical_repository_id: LogicalRepositoryId,
    ) -> Result<RepositoryReferenceReport, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(physical_repository_id)?;

        let authority = LogicalCodebaseStore::new(self.paths.clone());
        let member = authority
            .load_member(project_id, logical_repository_id)?
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "logical_codebase_member",
                id: logical_repository_id.0.to_string(),
            })?;
        if member.physical_repository_id != physical_repository_id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "logical_codebase_member",
                id: logical_repository_id.0.to_string(),
            });
        }

        let checkout_ids = member
            .checkout_ids
            .iter()
            .map(|id| id.0.to_string())
            .collect::<BTreeSet<_>>();
        let identities = RepositoryIdentities {
            physical_repository_id,
            logical_repository_id,
            checkout_ids: &checkout_ids,
            source_values: member.source_identity_values(),
        };
        let mut blockers = Vec::new();

        self.scan_authority_records(project_id, &member, &identities, &mut blockers)?;
        for issue in self.issues(project_id)? {
            self.scan_issue(project_id, &issue, &identities, &mut blockers)?;
        }
        self.scan_generic_roots(
            project_id,
            &[
                (
                    "aggregate_index",
                    self.paths
                        .logical_codebase_root(project_id)
                        .join("aggregate-indexes"),
                ),
                (
                    "aggregate_index",
                    self.paths
                        .logical_codebase_root(project_id)
                        .join("aggregate-index"),
                ),
                (
                    "repository_initialization",
                    self.paths.repository_initializations_root(project_id),
                ),
                (
                    "operation",
                    self.paths.project_root(project_id).join("operations"),
                ),
                (
                    "operation",
                    self.paths
                        .project_root(project_id)
                        .join("running-operations"),
                ),
            ],
            &identities,
            &mut blockers,
        )?;

        blockers.sort_by(|left, right| {
            let kind_order =
                reference_kind_order(&left.kind).cmp(&reference_kind_order(&right.kind));
            kind_order.then_with(|| {
                (&left.kind, &left.record_id, &left.path).cmp(&(
                    &right.kind,
                    &right.record_id,
                    &right.path,
                ))
            })
        });
        blockers.dedup();
        Ok(RepositoryReferenceReport { blockers })
    }

    fn scan_authority_records(
        &self,
        project_id: &str,
        target_member: &crate::product::logical_codebase::CodebaseMemberRecord,
        identities: &RepositoryIdentities<'_>,
        blockers: &mut Vec<RepositoryReference>,
    ) -> Result<(), ProductStoreError> {
        let authority = LogicalCodebaseStore::new(self.paths.clone());
        for member in authority.list_members(project_id)? {
            validate_relative_id(&member.physical_repository_id)?;
            let target_is_self = member.logical_repository_id == identities.logical_repository_id
                && member.physical_repository_id == identities.physical_repository_id;
            if !target_is_self && identities.matches_member(&member) {
                self.push_reference(
                    blockers,
                    "logical_codebase_member",
                    &member.logical_repository_id.0.to_string(),
                    self.paths
                        .logical_codebase_root(project_id)
                        .join("members")
                        .join(format!("{}.json", member.logical_repository_id.0)),
                )?;
            }
        }
        for checkout in authority.list_checkouts(project_id)? {
            validate_relative_id(&checkout.physical_repository_id)?;
            let target_is_self = checkout.logical_repository_id == identities.logical_repository_id
                && checkout.physical_repository_id == identities.physical_repository_id
                && identities
                    .checkout_ids
                    .contains(&checkout.checkout_id.0.to_string());
            if !target_is_self && identities.matches_checkout(&checkout) {
                self.push_reference(
                    blockers,
                    "repository_checkout",
                    &checkout.checkout_id.0.to_string(),
                    self.paths
                        .logical_codebase_root(project_id)
                        .join("checkouts")
                        .join(format!("{}.json", checkout.checkout_id.0)),
                )?;
            }
        }

        let registry = IdentityRegistryStore::new(self.paths.clone());
        if let Some(entry) = registry.find_by_source(project_id, &target_member.source_identity)?
            && (entry.logical_repository_id != identities.logical_repository_id
                || entry.physical_repository_id != identities.physical_repository_id)
        {
            self.push_reference(
                blockers,
                "identity_registry",
                &entry.physical_repository_id,
                self.paths
                    .logical_codebase_root(project_id)
                    .join("identity-registry.json"),
            )?;
        }
        Ok(())
    }

    fn scan_issue(
        &self,
        project_id: &str,
        issue: &IssueRecord,
        identities: &RepositoryIdentities<'_>,
        blockers: &mut Vec<RepositoryReference>,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(&issue.id)?;
        if issue.project_id != project_id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "issue",
                id: issue.id.clone(),
            });
        }
        let issue_root = self.paths.issue_root(project_id, &issue.id);
        if issue
            .repo_id
            .as_deref()
            .is_some_and(|id| identities.matches_string(id))
        {
            self.push_reference(blockers, "issue", &issue.id, issue_root.join("issue.json"))?;
        }

        self.scan_typed_records::<IssueRuntimeBindingRecord, _, _>(
            "runtime_binding",
            &issue_root.join("bindings"),
            identities,
            blockers,
            |record, identities| {
                identities.matches_string(&record.repo_id)
                    || record
                        .logical_repository_id
                        .is_some_and(|id| id == identities.logical_repository_id)
                    || record
                        .checkout_id
                        .is_some_and(|id| identities.checkout_ids.contains(&id.0.to_string()))
            },
            |record| &record.id,
        )?;
        self.scan_typed_records::<StorySpecRecord, _, _>(
            "story_spec",
            &issue_root.join("story-specs"),
            identities,
            blockers,
            |record, identities| {
                identities.matches_string(&record.repository_id)
                    || record
                        .involved_repository_ids
                        .contains(&identities.logical_repository_id)
                    || record.focus_repository_id == Some(identities.logical_repository_id)
            },
            |record| &record.id,
        )?;
        self.scan_typed_records::<LifecycleWorkItemRecord, _, _>(
            "work_item",
            &issue_root.join("work-items"),
            identities,
            blockers,
            |record, identities| {
                identities.matches_string(&record.repository_id)
                    || record.target_repository_id == Some(identities.logical_repository_id)
            },
            |record| &record.id,
        )?;
        self.scan_shared_worktree(project_id, &issue.id, identities, blockers)?;
        self.scan_typed_records::<RepositoryProfile, _, _>(
            "repository_profile",
            &issue_root.join("repository-profiles"),
            identities,
            blockers,
            |record, identities| {
                identities.matches_string(&record.repository_id)
                    || record.logical_repository_id == Some(identities.logical_repository_id)
            },
            |record| &record.id,
        )?;
        self.scan_attempts(project_id, &issue.id, identities, blockers)?;
        self.scan_generic_roots(
            project_id,
            &[
                (
                    "codebase_selection",
                    issue_root.join("codebase-selection.json"),
                ),
                ("workspace_session", issue_root.join("workspace-sessions")),
                (
                    "workspace_session_link",
                    issue_root.join("workspace-session-links"),
                ),
            ],
            identities,
            blockers,
        )
    }

    fn scan_shared_worktree(
        &self,
        project_id: &str,
        issue_id: &str,
        identities: &RepositoryIdentities<'_>,
        blockers: &mut Vec<RepositoryReference>,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(issue_id)?;
        let path = self
            .paths
            .issue_root(project_id, issue_id)
            .join("issue-shared-worktree.json");
        if !path.exists() {
            return Ok(());
        }
        let worktree: IssueSharedWorktree = read_json(&path)?;
        validate_relative_id(&worktree.id)?;
        if identities.matches_string(&worktree.repository_id)
            || worktree.target_repository_id == Some(identities.logical_repository_id)
            || worktree
                .checkout_id
                .is_some_and(|id| identities.checkout_ids.contains(&id.0.to_string()))
        {
            self.push_reference(blockers, "issue_shared_worktree", &worktree.id, path)?;
        }
        Ok(())
    }

    fn scan_attempts(
        &self,
        project_id: &str,
        issue_id: &str,
        identities: &RepositoryIdentities<'_>,
        blockers: &mut Vec<RepositoryReference>,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(issue_id)?;
        let root = self
            .paths
            .issue_root(project_id, issue_id)
            .join("coding-attempts");
        for path in self.json_paths(&root, false)? {
            let attempt: CodingExecutionAttempt = read_json(&path)?;
            self.validate_attempt(&attempt, project_id, issue_id, &path)?;
            if self.attempt_references_repository(&attempt, identities)? {
                self.push_reference(blockers, "coding_attempt", &attempt.id, path)?;
            }
        }
        Ok(())
    }

    fn attempt_references_repository(
        &self,
        attempt: &CodingExecutionAttempt,
        identities: &RepositoryIdentities<'_>,
    ) -> Result<bool, ProductStoreError> {
        if let Some(snapshot) = &attempt.target_snapshot {
            validate_relative_id(&snapshot.physical_repository_id)?;
            return Ok(
                snapshot.logical_repository_id == identities.logical_repository_id
                    || identities
                        .checkout_ids
                        .contains(&snapshot.checkout_id.0.to_string())
                    || snapshot.physical_repository_id == identities.physical_repository_id
                    || identities
                        .source_values
                        .contains(&snapshot.git_dir_identity),
            );
        }

        let mut targets = BTreeSet::new();
        let current_work_item = match attempt.scope {
            CodingAttemptScope::WorkItem => attempt
                .current_work_item_id
                .as_deref()
                .unwrap_or(&attempt.work_item_id),
            CodingAttemptScope::WorkItemGroup => attempt
                .current_work_item_id
                .as_deref()
                .ok_or_else(|| ProductStoreError::InvalidRecord {
                    kind: "coding_attempt",
                    reason: format!("group attempt {} has no current_work_item_id", attempt.id),
                })?,
        };
        targets.insert(self.work_item_repository_id(attempt, current_work_item)?);
        if attempt.scope == CodingAttemptScope::WorkItemGroup {
            let attempt_root = self
                .paths
                .issue_root(&attempt.project_id, &attempt.issue_id)
                .join("coding-attempts")
                .join(&attempt.id);
            for unit_path in self.json_paths(&attempt_root.join("units"), true)? {
                let unit: CodingExecutionUnit = read_json(&unit_path)?;
                self.validate_group_unit(attempt, &unit)?;
                targets.insert(self.work_item_repository_id(attempt, &unit.logical_work_item_id)?);
            }
            self.validate_group_plan_binding(attempt, &attempt_root.join("plan-binding.json"))?;
            self.validate_group_initialization(attempt)?;
        }
        Ok(targets.contains(identities.physical_repository_id))
    }

    fn validate_group_plan_binding(
        &self,
        attempt: &CodingExecutionAttempt,
        path: &Path,
    ) -> Result<(), ProductStoreError> {
        if !path.exists() {
            return Ok(());
        }
        let binding: CodingAttemptPlanBinding = read_json(path)?;
        let group_id = attempt.work_item_group_id.as_deref().ok_or_else(|| {
            ProductStoreError::InvalidRecord {
                kind: "coding_attempt_plan_binding",
                reason: format!("group attempt {} has no plan id", attempt.id),
            }
        })?;
        validate_relative_id(group_id)?;
        if binding.attempt_id != attempt.id || binding.plan_id != group_id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_attempt_plan_binding",
                id: attempt.id.clone(),
            });
        }
        Ok(())
    }

    fn validate_group_initialization(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(), ProductStoreError> {
        let Some(plan_id) = attempt.work_item_group_id.as_deref() else {
            return Err(ProductStoreError::InvalidRecord {
                kind: "coding_group_initialization",
                reason: format!("group attempt {} has no plan id", attempt.id),
            });
        };
        validate_relative_id(plan_id)?;
        let path = self
            .paths
            .issue_root(&attempt.project_id, &attempt.issue_id)
            .join("coding-attempts/group-initializations")
            .join(format!("{plan_id}.json"));
        if !path.exists() {
            return Ok(());
        }
        let value: Value = read_json(&path)?;
        let initialized_attempt_id = value
            .pointer("/attempt/id")
            .and_then(Value::as_str)
            .ok_or_else(|| ProductStoreError::InvalidRecord {
                kind: "coding_group_initialization",
                reason: format!("group initialization {plan_id} lacks attempt id"),
            })?;
        let initialized_work_item_id = value
            .pointer("/attempt/current_work_item_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ProductStoreError::InvalidRecord {
                kind: "coding_group_initialization",
                reason: format!("group initialization {plan_id} lacks current work item"),
            })?;
        validate_relative_id(initialized_attempt_id)?;
        validate_relative_id(initialized_work_item_id)?;
        if initialized_attempt_id != attempt.id
            || attempt.current_work_item_id.as_deref() != Some(initialized_work_item_id)
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_group_initialization",
                id: attempt.id.clone(),
            });
        }
        Ok(())
    }

    fn validate_group_unit(
        &self,
        attempt: &CodingExecutionAttempt,
        unit: &CodingExecutionUnit,
    ) -> Result<(), ProductStoreError> {
        for id in [
            &unit.id,
            &unit.attempt_id,
            &unit.project_id,
            &unit.issue_id,
            &unit.logical_work_item_id,
        ] {
            validate_relative_id(id)?;
        }
        if unit.attempt_id != attempt.id
            || unit.project_id != attempt.project_id
            || unit.issue_id != attempt.issue_id
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_execution_unit",
                id: unit.id.clone(),
            });
        }
        Ok(())
    }

    fn work_item_repository_id(
        &self,
        attempt: &CodingExecutionAttempt,
        work_item_id: &str,
    ) -> Result<String, ProductStoreError> {
        validate_relative_id(work_item_id)?;
        let path = self
            .paths
            .issue_root(&attempt.project_id, &attempt.issue_id)
            .join("work-items")
            .join(format!("{work_item_id}.json"));
        let work_item: LifecycleWorkItemRecord = read_json(&path)?;
        if work_item.id != work_item_id
            || work_item.project_id != attempt.project_id
            || work_item.issue_id != attempt.issue_id
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "work_item",
                id: work_item_id.to_string(),
            });
        }
        validate_relative_id(&work_item.repository_id)?;
        Ok(work_item.repository_id)
    }

    fn validate_attempt(
        &self,
        attempt: &CodingExecutionAttempt,
        project_id: &str,
        issue_id: &str,
        path: &Path,
    ) -> Result<(), ProductStoreError> {
        for id in [
            &attempt.id,
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.work_item_id,
        ] {
            validate_relative_id(id)?;
        }
        let file_id = file_record_id(path)?;
        if attempt.id != file_id || attempt.project_id != project_id || attempt.issue_id != issue_id
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_attempt",
                id: attempt.id.clone(),
            });
        }
        Ok(())
    }

    fn scan_typed_records<T, F, I>(
        &self,
        kind: &str,
        root: &Path,
        identities: &RepositoryIdentities<'_>,
        blockers: &mut Vec<RepositoryReference>,
        references: F,
        id: I,
    ) -> Result<(), ProductStoreError>
    where
        T: serde::de::DeserializeOwned,
        F: Fn(&T, &RepositoryIdentities<'_>) -> bool,
        I: Fn(&T) -> &str,
    {
        for path in self.json_paths(root, true)? {
            let record: T = read_json(&path)?;
            let record_id = id(&record);
            validate_relative_id(record_id)?;
            if references(&record, identities) {
                self.push_reference(blockers, kind, record_id, path)?;
            }
        }
        Ok(())
    }

    fn scan_generic_roots(
        &self,
        project_id: &str,
        roots: &[(&str, PathBuf)],
        identities: &RepositoryIdentities<'_>,
        blockers: &mut Vec<RepositoryReference>,
    ) -> Result<(), ProductStoreError> {
        for (kind, root) in roots {
            let paths = if root.extension().and_then(|value| value.to_str()) == Some("json") {
                if root.exists() {
                    vec![root.clone()]
                } else {
                    Vec::new()
                }
            } else {
                self.json_paths(root, true)?
            };
            for path in paths {
                let value: Value = read_json(&path)?;
                if value_contains_identity(&value, identities) {
                    let record_id = value
                        .get("id")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| {
                            file_record_id(&path).unwrap_or_else(|_| "record".to_string())
                        });
                    validate_relative_id(&record_id)?;
                    self.push_reference(blockers, kind, &record_id, path)?;
                }
            }
        }
        // Ensure the supplied project id has passed validation even for an empty scan.
        validate_relative_id(project_id)
    }

    fn issues(&self, project_id: &str) -> Result<Vec<IssueRecord>, ProductStoreError> {
        let root = self.paths.project_root(project_id).join("issues");
        let mut issues = Vec::new();
        for directory in child_directories(&root)? {
            let issue_id = directory_name(&directory)?;
            let path = directory.join("issue.json");
            if !path.exists() {
                continue;
            }
            let issue: IssueRecord = read_json(&path)?;
            if issue.id != issue_id {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "issue",
                    id: issue_id,
                });
            }
            issues.push(issue);
        }
        issues.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(issues)
    }

    fn json_paths(&self, root: &Path, recursive: bool) -> Result<Vec<PathBuf>, ProductStoreError> {
        let mut paths = Vec::new();
        collect_json_paths(root, recursive, &mut paths)?;
        paths.sort();
        Ok(paths)
    }

    fn push_reference(
        &self,
        blockers: &mut Vec<RepositoryReference>,
        kind: &str,
        record_id: &str,
        path: PathBuf,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(record_id)?;
        let relative = path
            .strip_prefix(self.paths.root())
            .map_err(|_| ProductStoreError::PathEscape(path.display().to_string()))?;
        blockers.push(RepositoryReference {
            kind: kind.to_string(),
            record_id: record_id.to_string(),
            path: relative.to_string_lossy().replace('\\', "/"),
        });
        Ok(())
    }
}

struct RepositoryIdentities<'a> {
    physical_repository_id: &'a str,
    logical_repository_id: LogicalRepositoryId,
    checkout_ids: &'a BTreeSet<String>,
    source_values: BTreeSet<String>,
}

impl RepositoryIdentities<'_> {
    fn matches_string(&self, value: &str) -> bool {
        value == self.physical_repository_id
            || value == self.logical_repository_id.0.to_string()
            || self.checkout_ids.contains(value)
            || self.source_values.contains(value)
    }

    fn matches_member(
        &self,
        member: &crate::product::logical_codebase::CodebaseMemberRecord,
    ) -> bool {
        self.matches_string(&member.physical_repository_id)
            || member.logical_repository_id == self.logical_repository_id
            || member
                .checkout_ids
                .iter()
                .any(|id| self.checkout_ids.contains(&id.0.to_string()))
            || member
                .source_identity_values()
                .iter()
                .any(|value| self.source_values.contains(value))
    }

    fn matches_checkout(
        &self,
        checkout: &crate::product::logical_codebase::RepositoryCheckoutRecord,
    ) -> bool {
        self.matches_string(&checkout.physical_repository_id)
            || checkout.logical_repository_id == self.logical_repository_id
            || self
                .checkout_ids
                .contains(&checkout.checkout_id.0.to_string())
            || self.source_values.contains(&checkout.git_dir_identity)
    }
}

trait SourceIdentityValues {
    fn source_identity_values(&self) -> BTreeSet<String>;
}

impl SourceIdentityValues for crate::product::logical_codebase::CodebaseMemberRecord {
    fn source_identity_values(&self) -> BTreeSet<String> {
        let mut values = BTreeSet::from([
            self.source_identity.key_digest.clone(),
            self.source_identity
                .canonical_git_dir
                .to_string_lossy()
                .to_string(),
            self.source_identity.git_dir_identity(),
        ]);
        if let Some(origin) = &self.source_identity.canonical_origin {
            values.insert(origin.clone());
        }
        values
    }
}

fn reference_kind_order(kind: &str) -> u8 {
    match kind {
        "issue" => 0,
        "runtime_binding" => 1,
        "story_spec" => 2,
        "work_item" => 3,
        "issue_shared_worktree" => 4,
        "repository_profile" => 5,
        "coding_attempt" => 6,
        _ => 7,
    }
}
fn value_contains_identity(value: &Value, identities: &RepositoryIdentities<'_>) -> bool {
    match value {
        Value::String(value) => identities.matches_string(value),
        Value::Array(values) => values
            .iter()
            .any(|value| value_contains_identity(value, identities)),
        Value::Object(values) => values
            .values()
            .any(|value| value_contains_identity(value, identities)),
        _ => false,
    }
}

fn child_directories(root: &Path) -> Result<Vec<PathBuf>, ProductStoreError> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(ProductStoreError::Io(format!(
                "read {}: {error}",
                root.display()
            )));
        }
    };
    let mut directories = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            ProductStoreError::Io(format!("read {} entry: {error}", root.display()))
        })?;
        if entry
            .file_type()
            .map_err(|error| {
                ProductStoreError::Io(format!("stat {}: {error}", entry.path().display()))
            })?
            .is_dir()
        {
            let path = entry.path();
            validate_relative_id(&directory_name(&path)?)?;
            directories.push(path);
        }
    }
    directories.sort();
    Ok(directories)
}

fn collect_json_paths(
    root: &Path,
    recursive: bool,
    paths: &mut Vec<PathBuf>,
) -> Result<(), ProductStoreError> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(ProductStoreError::Io(format!(
                "read {}: {error}",
                root.display()
            )));
        }
    };
    for entry in entries {
        let entry = entry.map_err(|error| {
            ProductStoreError::Io(format!("read {} entry: {error}", root.display()))
        })?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| ProductStoreError::Io(format!("stat {}: {error}", path.display())))?;
        if file_type.is_dir() {
            validate_relative_id(&directory_name(&path)?)?;
            if recursive {
                collect_json_paths(&path, true, paths)?;
            }
            continue;
        }
        if !file_type.is_file() || path.extension().and_then(|value| value.to_str()) != Some("json")
        {
            continue;
        }
        validate_relative_id(&file_record_id(&path)?)?;
        paths.push(path);
    }
    Ok(())
}

fn directory_name(path: &Path) -> Result<String, ProductStoreError> {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(str::to_string)
        .ok_or_else(|| ProductStoreError::PathEscape(path.display().to_string()))
}

fn file_record_id(path: &Path) -> Result<String, ProductStoreError> {
    path.file_stem()
        .and_then(|value| value.to_str())
        .map(str::to_string)
        .ok_or_else(|| ProductStoreError::PathEscape(path.display().to_string()))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::process::Command;

    use super::*;
    use crate::product::coding_models::{
        AttemptTargetSnapshot, CodingAttemptScope, CodingAttemptStatus, CodingExecutionAttempt,
        CodingExecutionStage,
    };
    use crate::product::logical_codebase::{
        CheckoutAvailability, IdentityRegistryState, LogicalCodebaseFeature, MemberStatus,
    };
    use crate::product::models::ProviderName;
    use crate::product::models::{IssuePhase, IssueRecord, IssueStatus};
    use crate::product::project_store::{CreateProjectInput, ProjectStore};
    use crate::product::repository_store::{
        CreateRepositoryInput, DeleteRepositoryCommand, RepositoryStore,
    };
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;

    struct DeletionFixture {
        _root: tempfile::TempDir,
        paths: ProductAppPaths,
        store: RepositoryStore,
        repository: crate::product::models::RepositoryRecord,
        logical_id: LogicalRepositoryId,
        checkout_id: crate::product::logical_codebase::RepositoryCheckoutId,
    }

    impl DeletionFixture {
        fn write_issue_reference(&self) {
            let now = "2026-08-08T00:00:00Z".to_string();
            let issue = IssueRecord {
                id: "issue_0001".to_string(),
                project_id: "project_0001".to_string(),
                repo_id: Some(self.repository.id.clone()),
                title: "referenced".to_string(),
                description: None,
                change_id: "referenced".to_string(),
                phase: IssuePhase::Clarification,
                status: IssueStatus::Draft,
                active_binding_id: None,
                created_at: now.clone(),
                updated_at: now,
            };
            crate::product::json_store::write_json(
                &self
                    .paths
                    .issue_root("project_0001", "issue_0001")
                    .join("issue.json"),
                &issue,
            )
            .unwrap();
        }

        fn write_attempt_with_target_snapshot(&self) {
            let now = "2026-08-08T00:00:00Z".to_string();
            let attempt = CodingExecutionAttempt {
                id: "coding_attempt_0001".to_string(),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                work_item_id: "work_item_0001".to_string(),
                attempt_no: 1,
                scope: CodingAttemptScope::WorkItem,
                status: CodingAttemptStatus::Completed,
                version: 0,
                manual_recovery_reason: None,
                stage: CodingExecutionStage::FinalConfirm,
                base_branch: "main".to_string(),
                branch_name: "aria/attempt".to_string(),
                worktree_path: None,
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: ProviderName::Fake,
                    reviewer: None,
                    review_rounds: 0,
                    permission_modes: Default::default(),
                },
                rework_count: 0,
                max_auto_rework: 0,
                work_item_group_id: None,
                current_work_item_id: Some("work_item_0001".to_string()),
                active_unit_id: None,
                head_commit: None,
                pushed_remote: None,
                review_request_id: None,
                provider_conversations: Vec::new(),
                created_at: now.clone(),
                updated_at: now.clone(),
                target_snapshot: Some(AttemptTargetSnapshot {
                    logical_repository_id: self.logical_id,
                    checkout_id: self.checkout_id,
                    physical_repository_id: self.repository.id.clone(),
                    canonical_path: self.repository.path.clone(),
                    git_dir_identity: "git-dir".to_string(),
                    revision: None,
                    policy_digest: String::new(),
                    membership_revision: 1,
                    captured_at: now.clone(),
                    capture_source: "test".to_string(),
                }),
                completed_at: Some(now),
            };
            crate::product::json_store::write_json(
                &self
                    .paths
                    .issue_root("project_0001", "issue_0001")
                    .join("coding-attempts/coding_attempt_0001.json"),
                &attempt,
            )
            .unwrap();
        }

        fn read_repos_json_bytes(&self) -> Vec<u8> {
            fs::read(self.paths.project_root("project_0001").join("repos.json")).unwrap()
        }

        fn scanner(&self) -> RepositoryReferenceScanner {
            RepositoryReferenceScanner::new(self.paths.clone())
        }
    }

    fn deletion_fixture_with_repository() -> DeletionFixture {
        let root = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(root.path());
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "project".to_string(),
                description: None,
            })
            .unwrap();
        let git_root = root.path().join("api");
        fs::create_dir_all(&git_root).unwrap();
        assert!(
            Command::new("git")
                .args(["init", "--quiet"])
                .current_dir(&git_root)
                .status()
                .unwrap()
                .success()
        );
        let store = RepositoryStore::with_logical_codebase_feature(
            paths.clone(),
            LogicalCodebaseFeature::enabled(),
        );
        let repository = store
            .create(CreateRepositoryInput {
                project_id: "project_0001".to_string(),
                name: "api".to_string(),
                path: git_root,
                default_policy_preset: None,
                default_provider_mode: None,
                idempotency_key: "register-api-1".to_string(),
            })
            .unwrap();
        DeletionFixture {
            _root: root,
            paths,
            store,
            logical_id: repository.logical_repository_id.unwrap(),
            checkout_id: repository.primary_checkout_id.unwrap(),
            repository,
        }
    }

    #[test]
    fn disabled_feature_delete_returns_legacy_receipt_and_replays_by_operation_id() {
        let root = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(root.path());
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "project".to_string(),
                description: None,
            })
            .unwrap();
        let repository_path = root.path().join("legacy-api");
        fs::create_dir_all(&repository_path).unwrap();
        let store = RepositoryStore::new(paths);
        let repository = store
            .create(CreateRepositoryInput {
                project_id: "project_0001".to_string(),
                name: "legacy-api".to_string(),
                path: repository_path,
                default_policy_preset: None,
                default_provider_mode: None,
                idempotency_key: "ignored-by-legacy-create".to_string(),
            })
            .unwrap();
        let command = DeleteRepositoryCommand {
            operation_id: "delete-legacy-1".to_string(),
            expected_updated_at: None,
            allow_tombstone_reactivation: false,
        };

        let receipt = store
            .delete("project_0001", &repository.id, command.clone())
            .unwrap();
        assert!(receipt.legacy_delete);
        assert_eq!(receipt.logical_repository_id, None);
        assert_eq!(receipt.checkout_id, None);
        assert_eq!(receipt.tombstone_operation_id, None);
        assert!(store.list("project_0001").unwrap().is_empty());
        assert_eq!(
            store
                .delete("project_0001", &repository.id, command)
                .unwrap(),
            receipt
        );
    }

    #[test]
    fn delete_without_references_tombstones_authority_and_replays_receipt() {
        let fixture = deletion_fixture_with_repository();
        let command = DeleteRepositoryCommand {
            operation_id: "delete-api-1".into(),
            expected_updated_at: None,
            allow_tombstone_reactivation: false,
        };

        let receipt = fixture
            .store
            .delete("project_0001", &fixture.repository.id, command.clone())
            .unwrap();
        assert_eq!(receipt.physical_repository_id, fixture.repository.id);
        assert_eq!(receipt.logical_repository_id, Some(fixture.logical_id));
        assert_eq!(receipt.checkout_id, Some(fixture.checkout_id));
        assert_eq!(
            receipt.tombstone_operation_id.as_deref(),
            Some("delete-api-1")
        );
        assert!(!receipt.legacy_delete);
        assert!(
            !fixture
                .read_repos_json_bytes()
                .windows(fixture.repository.id.len())
                .any(|bytes| bytes == fixture.repository.id.as_bytes())
        );

        let authority = LogicalCodebaseStore::new(fixture.paths.clone());
        let member = authority
            .load_member("project_0001", fixture.logical_id)
            .unwrap()
            .unwrap();
        assert_eq!(member.status, MemberStatus::Tombstoned);
        assert_eq!(
            authority
                .load_checkout("project_0001", fixture.checkout_id)
                .unwrap()
                .unwrap()
                .availability,
            CheckoutAvailability::Unresolved
        );
        assert_eq!(
            IdentityRegistryStore::new(fixture.paths.clone())
                .find_by_source("project_0001", &member.source_identity)
                .unwrap()
                .unwrap()
                .state,
            IdentityRegistryState::Tombstoned
        );
        assert_eq!(
            fixture
                .store
                .delete("project_0001", &fixture.repository.id, command)
                .unwrap(),
            receipt
        );
    }

    #[test]
    fn delete_rejects_legacy_and_logical_references_without_mutating_repos_json() {
        let fixture = deletion_fixture_with_repository();
        fixture.write_issue_reference();
        fixture.write_attempt_with_target_snapshot();
        let before = fixture.read_repos_json_bytes();

        let error = fixture
            .store
            .delete(
                "project_0001",
                &fixture.repository.id,
                DeleteRepositoryCommand {
                    operation_id: "delete-api-1".into(),
                    expected_updated_at: None,
                    allow_tombstone_reactivation: false,
                },
            )
            .unwrap_err();

        assert!(matches!(
            error,
            ProductStoreError::Conflict {
                kind: "repository_references",
                ..
            }
        ));
        assert_eq!(fixture.read_repos_json_bytes(), before);
        let report = fixture
            .scanner()
            .scan("project_0001", &fixture.repository.id, fixture.logical_id)
            .unwrap();
        assert_eq!(
            report
                .blockers
                .iter()
                .map(|item| item.kind.as_str())
                .collect::<Vec<_>>(),
            vec!["issue", "coding_attempt"]
        );
    }
}
