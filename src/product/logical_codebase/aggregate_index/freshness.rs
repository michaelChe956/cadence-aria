//! Freshness assessment, on-demand sync, scheduled staleness polling and a
//! budget-aware compact member inventory for the aggregate index.
//!
//! See Task 6 of the aggregate-index implementation plan. This module never
//! fabricates an updated index: when the on-disk Git evidence disagrees with the
//! published active record we report [`AggregateIndexStatus::Stale`] (or
//! [`AggregateIndexStatus::Degraded`] when the CodeGraph CLI itself is
//! unavailable), and an explicit `sync_if_stale` is required to publish a new
//! verified active record.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::product::json_store::validate_relative_id;
use crate::product::logical_codebase::{
    CodebaseMemberRecord, LogicalCodebaseManifest, LogicalCodebaseStore, LogicalRepositoryId,
    MemberStatus,
};

use super::{
    AggregateIndexError, AggregateIndexMemberSnapshot, AggregateIndexOperation,
    AggregateIndexRecord, AggregateIndexSnapshotCollector, AggregateIndexStatus,
    AggregateIndexStore,
};

/// Poll cadence for the scheduled staleness sweep. Active records are reassessed
/// at most once per interval; faster re-entry is a no-op.
pub const STALE_POLL_INTERVAL: Duration = Duration::from_secs(30);

/// Minimum quiet window after the most recent aggregate write before the index
/// is considered settled for an incremental refresh. Active writing is debounced
/// rather than racing the writer.
pub const ACTIVE_WRITE_DEBOUNCE: Duration = Duration::from_secs(2);

/// Soft budget for the compact inventory. Once exceeded we drop non-target
/// profile summaries (tech stack, tags, owner) to keep the rendering compact.
pub const COMPACT_INVENTORY_SOFT_BUDGET_BYTES: usize = 4 * 1024;

/// Hard budget for the compact inventory. Once exceeded only the target members
/// plus an `omitted_member_count` marker remain.
pub const COMPACT_INVENTORY_HARD_BUDGET_BYTES: usize = 8 * 1024;

/// Outcome of an [`AggregateIndexFreshnessService::assess`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregateIndexFreshness {
    pub status: AggregateIndexStatus,
    pub record: AggregateIndexRecord,
    /// Machine-readable reason for a non-active verdict, e.g.
    /// `membership_revision_changed` or `member_revision_or_dirty_changed`.
    /// Empty when the index is still active.
    pub reason: String,
}

impl AggregateIndexFreshness {
    pub fn active(record: AggregateIndexRecord) -> Self {
        Self {
            status: AggregateIndexStatus::Active,
            record,
            reason: String::new(),
        }
    }

    pub fn stale(record: AggregateIndexRecord, reason: &str) -> Self {
        Self {
            status: AggregateIndexStatus::Stale,
            record,
            reason: reason.to_string(),
        }
    }

    pub fn degraded(record: AggregateIndexRecord, reason: &str) -> Self {
        Self {
            status: AggregateIndexStatus::Degraded,
            record,
            reason: reason.to_string(),
        }
    }
}

/// Compares the published active record against the live Git evidence and, on
/// drift, refreshes it via the shared [`AggregateIndexOperation`].
pub struct AggregateIndexFreshnessService {
    logical: LogicalCodebaseStore,
    store: AggregateIndexStore,
    snapshots: AggregateIndexSnapshotCollector,
    operation: AggregateIndexOperation,
}

impl AggregateIndexFreshnessService {
    pub fn new(operation: AggregateIndexOperation) -> Self {
        Self {
            logical: operation.logical_clone(),
            store: operation.store_clone(),
            snapshots: operation.snapshots_clone(),
            operation,
        }
    }

    /// Returns a fresh assessment without mutating any durable state. The AI
    /// caller invokes this before reading the aggregate index so stale or
    /// degraded state is explicit rather than silently trusted.
    pub fn assess(&self, project_id: &str) -> Result<AggregateIndexFreshness, AggregateIndexError> {
        validate_relative_id(project_id)?;
        let record = self.store.active_required(project_id)?;
        let manifest = self
            .logical
            .load_manifest(project_id)?
            .ok_or_else(|| missing_manifest(project_id))?;
        if record.membership_revision != manifest.membership_revision {
            return Ok(AggregateIndexFreshness::stale(
                record,
                "membership_revision_changed",
            ));
        }
        let current = self.snapshots.capture_included(project_id, &manifest)?;
        if member_evidence_drifted(&record.member_snapshots, &current) {
            return Ok(AggregateIndexFreshness::stale(
                record,
                "member_revision_or_dirty_changed",
            ));
        }
        if record.status == AggregateIndexStatus::Degraded {
            let warning = record.warning.clone().unwrap_or_default();
            return Ok(AggregateIndexFreshness::degraded(record, &warning));
        }
        Ok(AggregateIndexFreshness::active(record))
    }

    /// Refreshes the active index only when freshness detects drift. Returns
    /// the readable record unchanged when it is active or degraded.
    pub fn sync_if_stale(
        &self,
        project_id: &str,
    ) -> Result<AggregateIndexRecord, AggregateIndexError> {
        validate_relative_id(project_id)?;
        let freshness = self.assess(project_id)?;
        if freshness.status != AggregateIndexStatus::Stale {
            return Ok(freshness.record);
        }
        self.operation.sync_and_verify(project_id, freshness.record)
    }

    /// Scheduled staleness sweep. Returns the project ids whose active index is
    /// due for reassessment (older than [`STALE_POLL_INTERVAL`]). Callers should
    /// invoke [`Self::sync_if_stale`] for each returned id; this method never
    /// performs a refresh itself so it is safe to call frequently.
    pub fn poll_due(&self, now: DateTime<Utc>) -> Result<Vec<String>, AggregateIndexError> {
        let manifests = self.logical.list_manifests()?;
        let mut due = Vec::new();
        for manifest in manifests {
            let Some(active) = self.store.active(&manifest.project_id)? else {
                continue;
            };
            let Some(updated_at) = parse_rfc3339(&active.updated_at) else {
                continue;
            };
            if now.signed_duration_since(updated_at).num_seconds()
                >= STALE_POLL_INTERVAL.as_secs() as i64
            {
                due.push(manifest.project_id);
            }
        }
        Ok(due)
    }
}

fn member_evidence_drifted(
    published: &[AggregateIndexMemberSnapshot],
    current: &[AggregateIndexMemberSnapshot],
) -> bool {
    if published.len() != current.len() {
        return true;
    }
    published
        .iter()
        .zip(current)
        .any(|(before, now)| now.revision != before.revision || now.dirty != before.dirty)
}

fn parse_rfc3339(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value).ok().map(DateTime::from)
}

/// Budget-aware compact rendering of the logical-codebase member inventory.
///
/// The default rendering lists each member with its logical id, alias,
/// aggregate-root-relative path, role and a short profile summary (tech stack,
/// tags, owner). When the soft budget (4 KiB) is exceeded non-target profile
/// summaries are trimmed first; when the hard budget (8 KiB) is exceeded only
/// the requested target members plus an `omitted_member_count` marker remain.
/// The hard budget is never exceeded.
pub struct CompactMemberInventory {
    logical: LogicalCodebaseStore,
}

impl CompactMemberInventory {
    pub fn new(logical: LogicalCodebaseStore) -> Self {
        Self { logical }
    }

    pub fn render(
        &self,
        project_id: &str,
        target_member_ids: &[LogicalRepositoryId],
    ) -> Result<String, AggregateIndexError> {
        validate_relative_id(project_id)?;
        let manifest = self
            .logical
            .load_manifest(project_id)?
            .ok_or_else(|| missing_manifest(project_id))?;
        let members = self.logical.list_members(project_id)?;
        let checkouts = self.logical.list_checkouts(project_id)?;

        let members_by_id = members
            .iter()
            .map(|member| (member.logical_repository_id, member))
            .collect::<BTreeMap<_, _>>();
        let paths_by_member =
            member_root_relative_paths(&manifest.provider_context_root, &checkouts);
        let targets: BTreeMap<LogicalRepositoryId, ()> = target_member_ids
            .iter()
            .copied()
            .map(|id| (id, ()))
            .collect();

        let full = self.render_full(&manifest, &members_by_id, &paths_by_member, &targets);
        if full.len() <= COMPACT_INVENTORY_SOFT_BUDGET_BYTES {
            return Ok(full);
        }

        // Past the soft budget, drop non-target profiles but keep every member.
        let trimmed =
            self.render_trimmed_profiles(&manifest, &members_by_id, &paths_by_member, &targets);
        if trimmed.len() <= COMPACT_INVENTORY_SOFT_BUDGET_BYTES {
            return Ok(trimmed);
        }

        // Still too large: keep only the requested targets and count the rest.
        let minimal = self.render_minimal(&manifest, &members_by_id, &paths_by_member, &targets);
        debug_assert!(minimal.len() <= COMPACT_INVENTORY_HARD_BUDGET_BYTES);
        Ok(minimal)
    }

    fn render_full(
        &self,
        manifest: &LogicalCodebaseManifest,
        members_by_id: &BTreeMap<LogicalRepositoryId, &CodebaseMemberRecord>,
        paths_by_member: &BTreeMap<LogicalRepositoryId, String>,
        targets: &BTreeMap<LogicalRepositoryId, ()>,
    ) -> String {
        let mut out = String::new();
        for member_id in &manifest.member_ids {
            let Some(member) = members_by_id.get(member_id) else {
                continue;
            };
            if member.status != MemberStatus::Active {
                continue;
            }
            write_member_line(
                &mut out,
                member,
                paths_by_member.get(member_id).map(String::as_str),
                true,
            );
            let _ = writeln!(out);
        }
        write_footer(&mut out, manifest.member_ids.len(), targets.len());
        out
    }

    fn render_trimmed_profiles(
        &self,
        manifest: &LogicalCodebaseManifest,
        members_by_id: &BTreeMap<LogicalRepositoryId, &CodebaseMemberRecord>,
        paths_by_member: &BTreeMap<LogicalRepositoryId, String>,
        targets: &BTreeMap<LogicalRepositoryId, ()>,
    ) -> String {
        let mut out = String::new();
        let mut omitted = 0usize;
        for member_id in &manifest.member_ids {
            let Some(member) = members_by_id.get(member_id) else {
                continue;
            };
            if member.status != MemberStatus::Active {
                continue;
            }
            // Targets always keep their profile; non-targets drop it past the soft budget.
            let keep_profile =
                out.len() <= COMPACT_INVENTORY_SOFT_BUDGET_BYTES || targets.contains_key(member_id);
            if keep_profile {
                write_member_line(
                    &mut out,
                    member,
                    paths_by_member.get(member_id).map(String::as_str),
                    true,
                );
            } else {
                write_member_line(
                    &mut out,
                    member,
                    paths_by_member.get(member_id).map(String::as_str),
                    false,
                );
                omitted += 1;
            }
            let _ = writeln!(out);
        }
        if omitted > 0 {
            let _ = writeln!(out, "trimmed_profile_count: {omitted}");
        }
        write_footer(&mut out, manifest.member_ids.len(), targets.len());
        out
    }

    fn render_minimal(
        &self,
        manifest: &LogicalCodebaseManifest,
        members_by_id: &BTreeMap<LogicalRepositoryId, &CodebaseMemberRecord>,
        paths_by_member: &BTreeMap<LogicalRepositoryId, String>,
        targets: &BTreeMap<LogicalRepositoryId, ()>,
    ) -> String {
        let mut out = String::new();
        let mut emitted = 0usize;
        for member_id in &manifest.member_ids {
            if !targets.contains_key(member_id) {
                continue;
            }
            let Some(member) = members_by_id.get(member_id) else {
                continue;
            };
            if member.status != MemberStatus::Active {
                continue;
            }
            write_member_line(
                &mut out,
                member,
                paths_by_member.get(member_id).map(String::as_str),
                true,
            );
            let _ = writeln!(out);
            emitted += 1;
        }
        let omitted = manifest.member_ids.len().saturating_sub(emitted);
        let _ = writeln!(out, "omitted_member_count: {omitted}");
        out
    }
}

fn member_root_relative_paths(
    root: &Path,
    checkouts: &[crate::product::logical_codebase::RepositoryCheckoutRecord],
) -> BTreeMap<LogicalRepositoryId, String> {
    let canonical_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut paths = BTreeMap::new();
    for checkout in checkouts {
        let relative = std::fs::canonicalize(&checkout.canonical_path)
            .ok()
            .and_then(|absolute| {
                absolute
                    .strip_prefix(&canonical_root)
                    .ok()
                    .map(|relative| relative.to_path_buf())
            })
            .map(|relative| relative.to_string_lossy().into_owned())
            .unwrap_or_else(|| checkout.canonical_path.to_string_lossy().into_owned());
        paths.insert(checkout.logical_repository_id, relative);
    }
    paths
}

fn write_member_line(
    out: &mut String,
    member: &CodebaseMemberRecord,
    path: Option<&str>,
    include_profile: bool,
) {
    let _ = write!(out, "- id: {}", member.logical_repository_id.0);
    let _ = write!(out, " alias: {}", member.alias);
    if let Some(path) = path {
        let _ = write!(out, " path: {path}");
    }
    let _ = write!(out, " role: {}", member.role);
    if include_profile {
        let profile = profile_summary(member);
        if !profile.is_empty() {
            let _ = write!(out, " profile: {profile}");
        }
    }
}

fn profile_summary(member: &CodebaseMemberRecord) -> String {
    let mut parts = Vec::new();
    if !member.tech_stack.is_empty() {
        parts.push(format!("tech_stack=[{}]", member.tech_stack.join(",")));
    }
    if !member.tags.is_empty() {
        parts.push(format!("tags=[{}]", member.tags.join(",")));
    }
    if let Some(owner) = &member.owner {
        parts.push(format!("owner={owner}"));
    }
    parts.join(" ")
}

fn write_footer(out: &mut String, total: usize, targets: usize) {
    let _ = writeln!(out, "member_count: {total}");
    let _ = writeln!(out, "target_member_count: {targets}");
}

fn missing_manifest(project_id: &str) -> AggregateIndexError {
    AggregateIndexError::Failed {
        code: "aggregate_index_manifest_missing",
        message: format!("logical-codebase manifest is missing for project {project_id}"),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use uuid::Uuid;

    use super::*;
    use crate::cross_cutting::bounded_command_runner::{
        BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
    };
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::aggregate_index::{
        AggregateIndexMemberSnapshot, AggregateIndexRecord, AggregateIndexStatus, CodeGraphCli,
        CodeGraphExcludeGenerator,
    };
    use crate::product::logical_codebase::{
        CheckoutAvailability, CheckoutKind, LogicalRepositoryId, RepositoryCheckoutId,
        RepositorySourceIdentity, RepositoryType,
    };

    #[test]
    fn changed_member_marks_active_index_stale_then_sync_replaces_snapshot() {
        let fixture = freshness_fixture();
        fixture.persist_active("api", &"a".repeat(40));
        fixture.git_head("api", &"b".repeat(40));

        let assessed = fixture.service().assess("project_0001").unwrap();
        assert_eq!(assessed.status, AggregateIndexStatus::Stale);
        assert_eq!(assessed.reason, "member_revision_or_dirty_changed");

        let synced = fixture.service().sync_if_stale("project_0001").unwrap();
        assert_eq!(synced.status, AggregateIndexStatus::Active);
        let api_snapshot = synced
            .member_snapshots
            .iter()
            .find(|snapshot| snapshot.revision == "b".repeat(40))
            .expect("refreshed snapshot carries the new api revision");
        assert_eq!(api_snapshot.revision, "b".repeat(40));
        let active = fixture.store.active("project_0001").unwrap().unwrap();
        assert!(
            active
                .member_snapshots
                .iter()
                .any(|snapshot| snapshot.revision == "b".repeat(40))
        );
    }

    #[test]
    fn assess_marks_stale_when_membership_revision_advances() {
        let fixture = freshness_fixture();
        fixture.persist_active("api", &"a".repeat(40));

        let mut manifest = fixture
            .logical
            .load_manifest("project_0001")
            .unwrap()
            .unwrap();
        manifest.membership_revision += 1;
        fixture
            .logical
            .save_manifest("project_0001", &manifest)
            .unwrap();

        let assessed = fixture.service().assess("project_0001").unwrap();
        assert_eq!(assessed.status, AggregateIndexStatus::Stale);
        assert_eq!(assessed.reason, "membership_revision_changed");
    }

    #[test]
    fn assess_reports_active_when_evidence_matches() {
        let fixture = freshness_fixture();
        fixture.persist_active("api", &"a".repeat(40));
        fixture.git_head("api", &"a".repeat(40));

        let assessed = fixture.service().assess("project_0001").unwrap();
        assert_eq!(assessed.status, AggregateIndexStatus::Active);
        assert!(assessed.reason.is_empty());
    }

    #[test]
    fn assess_preserves_degraded_last_known_good_instead_of_reporting_active() {
        let fixture = freshness_fixture();
        fixture.persist_active("api", &"a".repeat(40));
        let active = fixture.store.active("project_0001").unwrap().unwrap();
        fixture
            .store
            .mark_status(
                "project_0001",
                &active.aggregate_index_id,
                AggregateIndexStatus::Degraded,
                Some("sync failed".to_string()),
            )
            .unwrap();

        let result = fixture.service().assess("project_0001").unwrap();
        assert_eq!(result.status, AggregateIndexStatus::Degraded);
        assert_eq!(result.reason, "sync failed");
        assert_eq!(result.record.warning.as_deref(), Some("sync failed"));
    }

    #[test]
    fn sync_if_stale_preserves_degraded_last_known_good() {
        let fixture = freshness_fixture();
        fixture.persist_active("api", &"a".repeat(40));
        let active = fixture.store.active("project_0001").unwrap().unwrap();
        fixture
            .store
            .mark_status(
                "project_0001",
                &active.aggregate_index_id,
                AggregateIndexStatus::Degraded,
                Some("sync failed".to_string()),
            )
            .unwrap();

        let result = fixture.service().sync_if_stale("project_0001").unwrap();
        assert_eq!(result.status, AggregateIndexStatus::Degraded);
        assert_eq!(result.warning.as_deref(), Some("sync failed"));
    }

    #[test]
    fn sync_if_stale_is_a_noop_when_already_active() {
        let fixture = freshness_fixture();
        fixture.persist_active("api", &"a".repeat(40));
        fixture.git_head("api", &"a".repeat(40));

        let before = fixture
            .store
            .active("project_0001")
            .unwrap()
            .unwrap()
            .aggregate_index_id;
        let synced = fixture.service().sync_if_stale("project_0001").unwrap();
        assert_eq!(synced.aggregate_index_id, before);
        assert_eq!(synced.status, AggregateIndexStatus::Active);
    }

    #[test]
    fn poll_due_returns_projects_with_stale_active_record_only() {
        let fixture = freshness_fixture();
        fixture.persist_active("api", &"a".repeat(40));

        let now = Utc::now();
        let fresh = now - chrono::Duration::seconds(5);
        let stale = now - chrono::Duration::seconds(STALE_POLL_INTERVAL.as_secs() as i64 + 1);

        // Active record updated_at is "now"; not due.
        assert!(fixture.service().poll_due(now).unwrap().is_empty());

        // Force updated_at into the past and it becomes due.
        let mut active = fixture.store.active("project_0001").unwrap().unwrap();
        active.updated_at = stale.to_rfc3339();
        fixture.save_active(&active);
        assert_eq!(
            fixture.service().poll_due(now).unwrap(),
            vec!["project_0001"]
        );

        // Reset to recent and confirm it is skipped.
        active.updated_at = fresh.to_rfc3339();
        fixture.save_active(&active);
        assert!(fixture.service().poll_due(now).unwrap().is_empty());
    }

    #[test]
    fn compact_inventory_never_exceeds_hard_budget() {
        let rendered = inventory_fixture(50).render("project_0001", &[]).unwrap();
        assert!(
            rendered.len() <= COMPACT_INVENTORY_HARD_BUDGET_BYTES,
            "rendered inventory is {} bytes, hard budget is {}",
            rendered.len(),
            COMPACT_INVENTORY_HARD_BUDGET_BYTES
        );
        assert!(rendered.contains("omitted_member_count"));
    }

    #[test]
    fn compact_inventory_keeps_targets_when_others_are_omitted() {
        let fixture = inventory_fixture(50);
        let target = fixture.first_member_id();
        let rendered = fixture.render("project_0001", &[target]).unwrap();
        assert!(rendered.len() <= COMPACT_INVENTORY_HARD_BUDGET_BYTES);
        assert!(rendered.contains(&target.0.to_string()));
    }

    #[test]
    fn compact_inventory_small_inventory_lists_all_members() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let logical = LogicalCodebaseStore::new(paths.clone());
        let members = setup_two_member_manifest(&logical, temp.path());
        let inventory = CompactMemberInventory::new(logical);

        let rendered = inventory.render("project_0001", &[]).unwrap();
        assert!(rendered.len() <= COMPACT_INVENTORY_HARD_BUDGET_BYTES);
        for (_, alias) in &members {
            assert!(rendered.contains(alias), "missing alias {alias}");
        }
        assert!(!rendered.contains("omitted_member_count"));
    }

    struct FreshnessFixture {
        _temp: tempfile::TempDir,
        logical: LogicalCodebaseStore,
        store: AggregateIndexStore,
        runner: Arc<FreshnessRunner>,
        cli_executable: String,
    }

    impl FreshnessFixture {
        fn service(&self) -> AggregateIndexFreshnessService {
            let cli = CodeGraphCli::new(self.runner.clone(), self.cli_executable.clone());
            let operation = AggregateIndexOperation::with_snapshot_dependencies(
                self.logical.clone(),
                self.store.clone(),
                cli,
                CodeGraphExcludeGenerator,
                AggregateIndexSnapshotCollector::with_dependencies(
                    self.logical.clone(),
                    self.runner.clone(),
                ),
            );
            AggregateIndexFreshnessService::new(operation)
        }

        fn persist_active(&self, member_name: &str, revision: &str) {
            let manifest = self.logical.load_manifest("project_0001").unwrap().unwrap();
            let members = self.logical.list_members("project_0001").unwrap();
            let checkouts = self.logical.list_checkouts("project_0001").unwrap();
            let now = Utc::now().to_rfc3339();
            let mut snapshots = Vec::new();
            for member_id in &manifest.member_ids {
                let member = members
                    .iter()
                    .find(|candidate| candidate.logical_repository_id == *member_id)
                    .unwrap();
                let checkout = checkouts
                    .iter()
                    .find(|candidate| candidate.logical_repository_id == *member_id)
                    .unwrap();
                // The named member uses the caller-supplied revision; every other
                // member is recorded with its baseline `aaaa...` revision so that
                // re-assessment isolates the single changed member.
                let recorded_revision = if member.alias == member_name {
                    revision.to_string()
                } else {
                    "a".repeat(40)
                };
                snapshots.push(AggregateIndexMemberSnapshot::indexed(
                    *member_id,
                    checkout.checkout_id,
                    recorded_revision,
                    false,
                    now.clone(),
                ));
            }
            let mut record = AggregateIndexRecord::building(
                format!("aggregate_index_{}", Uuid::new_v4()),
                "project_0001".to_string(),
                manifest.membership_revision,
                snapshots,
                now.clone(),
            );
            record.status = AggregateIndexStatus::Active;
            record.codegraph_root = manifest.provider_context_root.clone();
            record.updated_at = now;
            self.store.replace_active("project_0001", record).unwrap();
        }

        fn git_head(&self, member_name: &str, revision: &str) {
            self.runner
                .state
                .lock()
                .unwrap()
                .heads
                .insert(member_name.to_string(), revision.to_string());
        }

        fn save_active(&self, record: &AggregateIndexRecord) {
            self.store
                .mark_status(
                    "project_0001",
                    &record.aggregate_index_id,
                    record.status,
                    record.warning.clone(),
                )
                .unwrap();
            // mark_status rewrites updated_at; overwrite the file with the desired timestamp.
            let mut current = self
                .store
                .get("project_0001", &record.aggregate_index_id)
                .unwrap()
                .unwrap();
            current.updated_at = record.updated_at.clone();
            self.store.replace_active("project_0001", current).unwrap();
        }
    }

    fn freshness_fixture() -> FreshnessFixture {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("aggregate");
        std::fs::create_dir_all(root.join("api/src")).unwrap();
        std::fs::create_dir_all(root.join("web/src")).unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let logical = LogicalCodebaseStore::new(paths.clone());
        let store = AggregateIndexStore::new(paths.clone());

        let (api_member, api_checkout) = member_with_checkout(&root, "api", "a".repeat(40));
        let (web_member, web_checkout) = member_with_checkout(&root, "web", "a".repeat(40));
        let mut manifest = LogicalCodebaseManifest::new(
            "project_0001",
            root,
            vec![
                api_member.logical_repository_id,
                web_member.logical_repository_id,
            ],
        );
        manifest.membership_revision = 1;
        logical.save_manifest("project_0001", &manifest).unwrap();
        logical.save_member("project_0001", &api_member).unwrap();
        logical
            .save_checkout("project_0001", &api_checkout)
            .unwrap();
        logical.save_member("project_0001", &web_member).unwrap();
        logical
            .save_checkout("project_0001", &web_checkout)
            .unwrap();

        let runner = Arc::new(FreshnessRunner::default());
        FreshnessFixture {
            _temp: temp,
            logical,
            store,
            runner,
            cli_executable: "codegraph".to_string(),
        }
    }

    struct InventoryFixture {
        _temp: tempfile::TempDir,
        logical: LogicalCodebaseStore,
        first_member_id: LogicalRepositoryId,
    }

    impl InventoryFixture {
        fn first_member_id(&self) -> LogicalRepositoryId {
            self.first_member_id
        }

        fn render(
            &self,
            project_id: &str,
            targets: &[LogicalRepositoryId],
        ) -> Result<String, AggregateIndexError> {
            CompactMemberInventory::new(self.logical.clone()).render(project_id, targets)
        }
    }

    fn inventory_fixture(count: usize) -> InventoryFixture {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("aggregate");
        std::fs::create_dir_all(&root).unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let logical = LogicalCodebaseStore::new(paths);

        let mut member_ids = Vec::with_capacity(count);
        let mut first_id = None;
        for index in 0..count {
            let name = format!("repo_{index:03}");
            std::fs::create_dir_all(root.join(&name)).unwrap();
            let (member, checkout) = verbose_member_with_checkout(&root, &name, index);
            if first_id.is_none() {
                first_id = Some(member.logical_repository_id);
            }
            member_ids.push(member.logical_repository_id);
            logical.save_member("project_0001", &member).unwrap();
            logical.save_checkout("project_0001", &checkout).unwrap();
        }
        let first_member_id = first_id.expect("at least one member");

        let manifest = LogicalCodebaseManifest::new("project_0001", root, member_ids);
        logical.save_manifest("project_0001", &manifest).unwrap();

        InventoryFixture {
            _temp: temp,
            logical,
            first_member_id,
        }
    }

    fn setup_two_member_manifest(
        logical: &LogicalCodebaseStore,
        root: &Path,
    ) -> Vec<(LogicalRepositoryId, String)> {
        let mut members = Vec::new();
        let mut member_ids = Vec::new();
        for name in ["api", "web"] {
            std::fs::create_dir_all(root.join(name)).unwrap();
            let (member, checkout) = member_with_checkout(root, name, "a".repeat(40));
            member_ids.push(member.logical_repository_id);
            members.push((member.logical_repository_id, member.alias.clone()));
            logical.save_member("project_0001", &member).unwrap();
            logical.save_checkout("project_0001", &checkout).unwrap();
        }
        let manifest = LogicalCodebaseManifest::new("project_0001", root.to_path_buf(), member_ids);
        logical.save_manifest("project_0001", &manifest).unwrap();
        members
    }

    fn member_with_checkout(
        root: &Path,
        name: &str,
        revision: String,
    ) -> (
        CodebaseMemberRecord,
        crate::product::logical_codebase::RepositoryCheckoutRecord,
    ) {
        let logical_repository_id = LogicalRepositoryId(Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        let canonical_path = root.join(name);
        let now = "2026-08-09T00:00:00Z".to_string();
        let source_identity = RepositorySourceIdentity {
            scheme: "test".to_string(),
            key_digest: format!("sha256:source-{name}"),
            canonical_git_dir: canonical_path.join(".git"),
            canonical_origin: None,
            first_seen_path_hash: format!("sha256:path-{name}"),
        };
        let member = CodebaseMemberRecord {
            logical_repository_id,
            physical_repository_id: format!("repository_{name}"),
            alias: name.to_string(),
            role: "repository".to_string(),
            ordinal: 1,
            source_identity: source_identity.clone(),
            repo_type: RepositoryType::Unknown,
            tech_stack: Vec::new(),
            owner: None,
            tags: Vec::new(),
            default_ref: None,
            checkout_ids: vec![checkout_id],
            status: MemberStatus::Active,
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        let checkout = crate::product::logical_codebase::RepositoryCheckoutRecord {
            checkout_id,
            logical_repository_id,
            physical_repository_id: member.physical_repository_id.clone(),
            kind: CheckoutKind::Main,
            canonical_path,
            checkout_path_hash: format!("sha256:checkout-{name}"),
            git_dir_identity: source_identity.git_dir_identity(),
            revision: Some(revision),
            availability: CheckoutAvailability::Available,
            observed_at: now.clone(),
            created_at: now.clone(),
            updated_at: now,
        };
        (member, checkout)
    }

    fn verbose_member_with_checkout(
        root: &Path,
        name: &str,
        index: usize,
    ) -> (
        CodebaseMemberRecord,
        crate::product::logical_codebase::RepositoryCheckoutRecord,
    ) {
        let logical_repository_id = LogicalRepositoryId(Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        let canonical_path = root.join(name);
        let now = "2026-08-09T00:00:00Z".to_string();
        let source_identity = RepositorySourceIdentity {
            scheme: "test".to_string(),
            key_digest: format!("sha256:source-{name}"),
            canonical_git_dir: canonical_path.join(".git"),
            canonical_origin: None,
            first_seen_path_hash: format!("sha256:path-{name}"),
        };
        let member = CodebaseMemberRecord {
            logical_repository_id,
            physical_repository_id: format!("repository_{name}"),
            alias: name.to_string(),
            role: "repository".to_string(),
            ordinal: index as u32,
            source_identity: source_identity.clone(),
            repo_type: if index.is_multiple_of(2) {
                RepositoryType::Backend
            } else {
                RepositoryType::Frontend
            },
            tech_stack: vec![
                "rust".to_string(),
                "tokio".to_string(),
                "axum".to_string(),
                "serde".to_string(),
                "sqlx".to_string(),
                "tower-http".to_string(),
                "tracing".to_string(),
            ],
            owner: Some(format!("team-platform-{index}")),
            tags: vec![
                format!("service-{index}"),
                "critical".to_string(),
                "aggregateroot".to_string(),
                "monorepo".to_string(),
            ],
            default_ref: Some("main".to_string()),
            checkout_ids: vec![checkout_id],
            status: MemberStatus::Active,
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        let checkout = crate::product::logical_codebase::RepositoryCheckoutRecord {
            checkout_id,
            logical_repository_id,
            physical_repository_id: member.physical_repository_id.clone(),
            kind: CheckoutKind::Main,
            canonical_path,
            checkout_path_hash: format!("sha256:checkout-{name}"),
            git_dir_identity: source_identity.git_dir_identity(),
            revision: Some("a".repeat(40)),
            availability: CheckoutAvailability::Available,
            observed_at: now.clone(),
            created_at: now.clone(),
            updated_at: now,
        };
        (member, checkout)
    }

    /// Test-only bounded runner that scripts both Git (`rev-parse`, `status`)
    /// and CodeGraph (`--version`, `init`, `sync`, `files`, `query`) commands.
    #[derive(Default)]
    struct FreshnessRunner {
        state: Mutex<FreshnessRunnerState>,
    }

    #[derive(Default)]
    struct FreshnessRunnerState {
        /// Per-member (`file_name()` of the checkout dir) scripted HEAD revision.
        /// Members not present default to `"a" * 40`.
        heads: BTreeMap<String, String>,
    }

    impl FreshnessRunner {
        fn member_head(&self, working_dir: &Path) -> String {
            let member = working_dir
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            self.state
                .lock()
                .unwrap()
                .heads
                .get(member)
                .cloned()
                .unwrap_or_else(|| "a".repeat(40))
        }
    }

    #[async_trait::async_trait]
    impl BoundedCommandRunner for FreshnessRunner {
        async fn run(
            &self,
            request: BoundedCommandRequest,
        ) -> Result<BoundedCommandResult, BoundedCommandError> {
            let stdout = match request.argv.as_slice() {
                [version] if version == "--version" => "1.5.0\n".to_string(),
                [rev_parse, head] if rev_parse == "rev-parse" && head == "HEAD" => {
                    self.member_head(&request.working_dir)
                }
                [status, porcelain] if status == "status" && porcelain == "--porcelain=v1" => {
                    String::new()
                }
                [init, dot] if init == "init" && dot == "." => "Indexed 1 file\n".to_string(),
                [sync, dot] if sync == "sync" && dot == "." => "Synced 1 file\n".to_string(),
                [files, json] if files == "files" && json == "--json" => {
                    // Return a file under every direct child of the aggregate
                    // root so member coverage and cross-member queries succeed.
                    let mut entries = Vec::new();
                    if let Ok(children) = std::fs::read_dir(&request.working_dir) {
                        for child in children.flatten() {
                            if !child.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                                continue;
                            }
                            let name = match child.file_name().to_str() {
                                Some(name) => name.to_string(),
                                None => continue,
                            };
                            if name.starts_with('.') {
                                continue;
                            }
                            entries.push(serde_json::json!({
                                "path": format!("{name}/src/lib.rs")
                            }));
                        }
                    }
                    serde_json::to_string(&entries).unwrap()
                }
                [query, symbol, json] if query == "query" && json == "--json" => {
                    if symbol == "crossRepoGreeting" {
                        serde_json::json!([
                            {"file": "api/src/lib.rs"},
                            {"file": "web/src/lib.rs"}
                        ])
                        .to_string()
                    } else {
                        serde_json::json!([]).to_string()
                    }
                }
                argv => panic!("unexpected argv in FreshnessRunner: {argv:?}"),
            };
            Ok(BoundedCommandResult {
                exit_code: Some(0),
                stdout,
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                duration_ms: 1,
            })
        }
    }
}
