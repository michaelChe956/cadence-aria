use crate::daemon::checkpoint::RuntimeSnapshot;
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeLeaseStatus {
    Acquired,
    Released,
    Waiting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorktreeLease {
    pub lease_id: String,
    pub worktree_path: String,
    pub base_ref: String,
    pub branch_name: String,
    pub status: WorktreeLeaseStatus,
    pub acquired_at: Option<String>,
    pub released_at: Option<String>,
    pub allowed_write_scope: Vec<String>,
    pub worktask_id: String,
    pub session_id: String,
    pub blocked_by_lease_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorktreeEvent {
    pub event_type: String,
    pub payload: Value,
}

#[derive(Debug, Clone)]
pub struct WorktreeLeaseManager {
    session_id: String,
    task_id: String,
    worktree_path: PathBuf,
    base_ref: String,
    leases: Vec<WorktreeLease>,
    events: Vec<WorktreeEvent>,
    snapshots: Vec<RuntimeSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorktreeError {
    #[error("worktree_scope_symlink_escape")]
    SymlinkEscape,
    #[error("worktree_scope_forbidden_path: {0}")]
    ForbiddenPath(String),
    #[error("worktree_scope_denied: {0}")]
    ScopeDenied(String),
    #[error("worktree_lease_not_found: {0}")]
    LeaseNotFound(String),
    #[error("worktree_io_error: {0}")]
    Io(String),
}

impl WorktreeLeaseManager {
    pub fn new(
        session_id: impl Into<String>,
        task_id: impl Into<String>,
        worktree_path: impl AsRef<Path>,
        base_ref: impl Into<String>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            task_id: task_id.into(),
            worktree_path: worktree_path.as_ref().to_path_buf(),
            base_ref: base_ref.into(),
            leases: Vec::new(),
            events: Vec::new(),
            snapshots: Vec::new(),
        }
    }

    pub fn recover(
        session_id: impl Into<String>,
        task_id: impl Into<String>,
        worktree_path: impl AsRef<Path>,
        base_ref: impl Into<String>,
        leases: Vec<WorktreeLease>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            task_id: task_id.into(),
            worktree_path: worktree_path.as_ref().to_path_buf(),
            base_ref: base_ref.into(),
            leases,
            events: Vec::new(),
            snapshots: Vec::new(),
        }
    }

    pub fn acquire(
        &mut self,
        worktask_id: impl Into<String>,
        branch_name: impl Into<String>,
        allowed_write_scope: Vec<String>,
    ) -> Result<WorktreeLease, WorktreeError> {
        fs::create_dir_all(&self.worktree_path)
            .map_err(|error| WorktreeError::Io(error.to_string()))?;

        let worktask_id = worktask_id.into();
        let lease_id = format!("lease_{}_{:04}", worktask_id, self.leases.len() + 1);
        let branch_name = branch_name.into();
        let blocked_by = self
            .leases
            .iter()
            .find(|lease| {
                lease.status == WorktreeLeaseStatus::Acquired
                    && scopes_may_overlap(
                        &lease.allowed_write_scope,
                        &allowed_write_scope,
                        platform_case_sensitive(),
                    )
            })
            .map(|lease| lease.lease_id.clone());
        let status = if blocked_by.is_some() {
            WorktreeLeaseStatus::Waiting
        } else {
            WorktreeLeaseStatus::Acquired
        };
        let now = now_iso();
        let lease = WorktreeLease {
            lease_id: lease_id.clone(),
            worktree_path: self.worktree_path.to_string_lossy().to_string(),
            base_ref: self.base_ref.clone(),
            branch_name,
            status: status.clone(),
            acquired_at: (status == WorktreeLeaseStatus::Acquired).then(|| now.clone()),
            released_at: None,
            allowed_write_scope,
            worktask_id,
            session_id: self.session_id.clone(),
            blocked_by_lease_id: blocked_by.clone(),
        };

        self.leases.push(lease.clone());
        if status == WorktreeLeaseStatus::Acquired {
            self.record_acquired_event(&lease);
            self.record_snapshot(&lease, None);
        } else {
            self.record_snapshot(&lease, Some("worktree_scope_conflict_waiting"));
        }
        Ok(lease)
    }

    pub fn release(&mut self, lease_id: &str) -> Result<(), WorktreeError> {
        let now = now_iso();
        let index = self
            .leases
            .iter()
            .position(|lease| lease.lease_id == lease_id)
            .ok_or_else(|| WorktreeError::LeaseNotFound(lease_id.to_string()))?;
        self.leases[index].status = WorktreeLeaseStatus::Released;
        self.leases[index].released_at = Some(now);
        let released = self.leases[index].clone();
        self.record_snapshot(&released, None);
        self.promote_waiting_leases();
        Ok(())
    }

    pub fn lease(&self, lease_id: &str) -> Option<&WorktreeLease> {
        self.leases.iter().find(|lease| lease.lease_id == lease_id)
    }

    pub fn leases(&self) -> &[WorktreeLease] {
        &self.leases
    }

    pub fn events(&self) -> &[WorktreeEvent] {
        &self.events
    }

    pub fn snapshots(&self) -> &[RuntimeSnapshot] {
        &self.snapshots
    }

    pub fn waiting_edges(&self) -> Vec<(String, String)> {
        self.leases
            .iter()
            .filter_map(|lease| {
                lease
                    .blocked_by_lease_id
                    .as_ref()
                    .map(|blocked_by| (blocked_by.clone(), lease.lease_id.clone()))
            })
            .collect()
    }

    fn promote_waiting_leases(&mut self) {
        let waiting_ids = self
            .leases
            .iter()
            .filter(|lease| lease.status == WorktreeLeaseStatus::Waiting)
            .map(|lease| lease.lease_id.clone())
            .collect::<Vec<_>>();
        for lease_id in waiting_ids {
            let Some(index) = self
                .leases
                .iter()
                .position(|lease| lease.lease_id == lease_id)
            else {
                continue;
            };
            if self.has_conflicting_acquired_lease(index) {
                continue;
            }
            self.leases[index].status = WorktreeLeaseStatus::Acquired;
            self.leases[index].blocked_by_lease_id = None;
            self.leases[index].acquired_at = Some(now_iso());
            let promoted = self.leases[index].clone();
            self.record_acquired_event(&promoted);
            self.record_snapshot(&promoted, None);
        }
    }

    fn has_conflicting_acquired_lease(&self, index: usize) -> bool {
        self.leases.iter().enumerate().any(|(other_index, other)| {
            other_index != index
                && other.status == WorktreeLeaseStatus::Acquired
                && scopes_may_overlap(
                    &other.allowed_write_scope,
                    &self.leases[index].allowed_write_scope,
                    platform_case_sensitive(),
                )
        })
    }

    fn record_acquired_event(&mut self, lease: &WorktreeLease) {
        self.events.push(WorktreeEvent {
            event_type: "worktree.lease_acquired".to_string(),
            payload: json!({
                "lease_id": lease.lease_id,
                "worktree_path": lease.worktree_path,
                "worktask_id": lease.worktask_id,
                "acquired_at": lease.acquired_at,
            }),
        });
    }

    fn record_snapshot(&mut self, lease: &WorktreeLease, reason: Option<&str>) {
        let mut snapshot = RuntimeSnapshot::minimal_for_test("N14");
        snapshot.session_id = self.session_id.clone();
        snapshot.task_id = self.task_id.clone();
        snapshot.phase = "execution".to_string();
        snapshot.timestamp = now_iso();
        snapshot.worktree_ref = Some(lease.lease_id.clone());
        snapshot.node_specific_fields = json!({
            "lease_id": lease.lease_id,
            "worktree_path": lease.worktree_path,
            "base_ref": lease.base_ref,
            "branch_name": lease.branch_name,
            "status": lease.status,
            "allowed_write_scope": lease.allowed_write_scope,
            "worktask_id": lease.worktask_id,
            "session_id": lease.session_id,
            "reason": reason,
        });
        self.snapshots.push(snapshot);
    }
}

pub fn validate_write_path(
    worktree_root: &Path,
    allowed_write_scope: &[String],
    candidate_path: &Path,
    case_sensitive: bool,
) -> Result<String, WorktreeError> {
    let root = worktree_root
        .canonicalize()
        .map_err(|error| WorktreeError::Io(error.to_string()))?;
    let candidate = if candidate_path.is_absolute() {
        candidate_path.to_path_buf()
    } else {
        worktree_root.join(candidate_path)
    };
    let canonical_candidate = canonicalize_existing_or_parent(&candidate)?;
    if !canonical_candidate.starts_with(&root) {
        return Err(WorktreeError::SymlinkEscape);
    }
    let relative = canonical_candidate
        .strip_prefix(&root)
        .map_err(|_| WorktreeError::SymlinkEscape)?;
    let relative = normalize_relative_path(relative, case_sensitive);

    if is_forbidden_runtime_path(&relative) {
        return Err(WorktreeError::ForbiddenPath(relative));
    }
    if allowed_write_scope.is_empty() {
        return Err(WorktreeError::ScopeDenied(relative));
    }
    if allowed_write_scope
        .iter()
        .any(|scope| scope_allows_path(scope, &relative, case_sensitive))
    {
        Ok(relative)
    } else {
        Err(WorktreeError::ScopeDenied(relative))
    }
}

pub fn scopes_may_overlap(a: &[String], b: &[String], case_sensitive: bool) -> bool {
    if a.is_empty() || b.is_empty() {
        return false;
    }
    a.iter().any(|left| {
        b.iter()
            .any(|right| scope_pair_may_overlap(left, right, case_sensitive))
    })
}

fn scope_pair_may_overlap(left: &str, right: &str, case_sensitive: bool) -> bool {
    let left = ScopePattern::parse(left, case_sensitive);
    let right = ScopePattern::parse(right, case_sensitive);
    if left.all || right.all {
        return true;
    }
    if left.base == right.base {
        return true;
    }
    if left.has_wildcard || right.has_wildcard || left.is_dir || right.is_dir {
        return left.base.starts_with(&right.base) || right.base.starts_with(&left.base);
    }
    false
}

pub(crate) fn scope_allows_path(scope: &str, relative_path: &str, case_sensitive: bool) -> bool {
    let pattern = ScopePattern::parse(scope, case_sensitive);
    let path = normalize_scope_text(relative_path, case_sensitive);
    if pattern.all {
        return true;
    }
    if pattern.has_wildcard || pattern.is_dir {
        return path.starts_with(&pattern.base);
    }
    path == pattern.base
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScopePattern {
    base: String,
    all: bool,
    is_dir: bool,
    has_wildcard: bool,
}

impl ScopePattern {
    fn parse(value: &str, case_sensitive: bool) -> Self {
        let normalized = normalize_scope_text(value, case_sensitive);
        if normalized == "*" {
            return Self {
                base: String::new(),
                all: true,
                is_dir: true,
                has_wildcard: true,
            };
        }
        let has_wildcard = normalized.contains('*');
        let is_dir = normalized.ends_with('/');
        let base = if has_wildcard {
            normalized.split('*').next().unwrap_or_default().to_string()
        } else {
            normalized.clone()
        };
        Self {
            base,
            all: false,
            is_dir,
            has_wildcard,
        }
    }
}

fn canonicalize_existing_or_parent(candidate: &Path) -> Result<PathBuf, WorktreeError> {
    if candidate.exists() {
        return candidate
            .canonicalize()
            .map_err(|error| WorktreeError::Io(error.to_string()));
    }
    let parent = candidate
        .parent()
        .ok_or_else(|| WorktreeError::Io("candidate path has no parent".to_string()))?;
    let parent = parent
        .canonicalize()
        .map_err(|error| WorktreeError::Io(error.to_string()))?;
    let Some(file_name) = candidate.file_name() else {
        return Ok(parent);
    };
    Ok(parent.join(file_name))
}

fn normalize_relative_path(path: &Path, case_sensitive: bool) -> String {
    normalize_scope_text(&path.to_string_lossy(), case_sensitive)
}

fn normalize_scope_text(value: &str, case_sensitive: bool) -> String {
    let mut normalized = value.replace('\\', "/");
    while let Some(stripped) = normalized.strip_prefix("./") {
        normalized = stripped.to_string();
    }
    while normalized.contains("//") {
        normalized = normalized.replace("//", "/");
    }
    if !case_sensitive {
        normalized = normalized.to_ascii_lowercase();
    }
    normalized
}

fn is_forbidden_runtime_path(relative: &str) -> bool {
    relative == ".git"
        || relative.starts_with(".git/")
        || relative == ".aria"
        || relative.starts_with(".aria/")
}

fn platform_case_sensitive() -> bool {
    !cfg!(windows)
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}
