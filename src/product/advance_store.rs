use std::path::PathBuf;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::locking::with_exclusive_lock;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdvanceStatus {
    Initializing,
    Ready,
    Running,
    AwaitingPlanAmendment,
    Completed,
    Failed,
    Aborted,
}

/// REQ-ADV-01/02（OQ1 定案，multi-repo-group-coding WP2）：advance record 的
/// per-target attempt 集绑定条目。additive-only：存量 record 无此字段（serde
/// default 空集），单值 `attempt_id` 承载保持不变。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvanceTargetAttemptBinding {
    pub target_repository_id: String,
    pub attempt_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvanceRecord {
    pub id: String,
    pub command_id: String,
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub plan_revision_id: String,
    pub attempt_id: Option<String>,
    /// REQ-ADV-01/02（OQ1）：多 target 拆分的 per-target attempt 集绑定。
    /// additive（serde default 空集=存量零迁移；空集不序列化）；单 target 场景
    /// 单值 `attempt_id` 承载语义不变。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target_attempts: Vec<AdvanceTargetAttemptBinding>,
    pub status: AdvanceStatus,
    pub workspace_entry: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvanceInput {
    pub command_id: String,
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdvanceInitializationPhase {
    /// The durable AdvanceRecord itself represents this checkpoint; no separate journal phase is
    /// written until JournalPrepared has an attempt identity.
    RecordPersisted,
    JournalPrepared,
    AttemptPersisted,
    WorktreeBound,
    PlanBindingSaved,
    UnitsMaterialized,
    Ready,
}

impl AdvanceInitializationPhase {
    fn order(self) -> u8 {
        match self {
            Self::RecordPersisted => 0,
            Self::JournalPrepared => 1,
            Self::AttemptPersisted => 2,
            Self::WorktreeBound => 3,
            Self::PlanBindingSaved => 4,
            Self::UnitsMaterialized => 5,
            Self::Ready => 6,
        }
    }

    pub(crate) fn order_for_engine(self) -> u8 {
        self.order()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvanceInitializationJournal {
    pub advance_id: String,
    pub plan_id: String,
    pub attempt_id: String,
    /// REQ-ADV-01/02（OQ1）：多 target 拆分的 attempt 引用集（journal 侧仅存
    /// attempt_id，target 归属由增殖审计面承载——两结构用途不同不混用）。
    /// additive：`attempt_id` 恒为拓扑序首个（兼容旧消费面非空假设）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target_attempt_ids: Vec<String>,
    pub phase: AdvanceInitializationPhase,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdvanceOutcome {
    Completed {
        record: AdvanceRecord,
        attempt_id: String,
        workspace_entry: String,
        /// REQ-MTG-03（OQ1）：多 target 拆分时的全集绑定（单 target 空）。
        target_attempts: Vec<AdvanceTargetAttemptBinding>,
    },
    Replayed {
        record: AdvanceRecord,
    },
    Rejected {
        record: Option<AdvanceRecord>,
        code: String,
        reason: String,
    },
}

#[derive(Debug, Clone)]
pub struct AdvanceStore {
    app_paths: ProductAppPaths,
}

impl AdvanceStore {
    pub fn new(app_paths: ProductAppPaths) -> Self {
        Self { app_paths }
    }

    pub fn app_paths(&self) -> ProductAppPaths {
        self.app_paths.clone()
    }

    fn root(&self, project_id: &str, issue_id: &str) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        Ok(self
            .app_paths
            .issue_root(project_id, issue_id)
            .join("advance-records"))
    }

    fn path_for(
        &self,
        project_id: &str,
        issue_id: &str,
        id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(id)?;
        Ok(self.root(project_id, issue_id)?.join(format!("{id}.json")))
    }

    fn with_existing_root<T>(
        &self,
        project_id: &str,
        issue_id: &str,
        operation: impl FnOnce() -> Result<T, ProductStoreError>,
    ) -> Result<T, ProductStoreError> {
        let root = self.root(project_id, issue_id)?;
        if !root.is_dir() {
            return operation();
        }
        with_exclusive_lock(&root, operation)
    }

    fn records(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<AdvanceRecord>, ProductStoreError> {
        let root = self.root(project_id, issue_id)?;
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "read {}: {error}",
                    root.display()
                )));
            }
        };
        let mut records = Vec::new();
        for entry in entries {
            let path = entry
                .map_err(|error| {
                    ProductStoreError::Io(format!("read advance record entry: {error}"))
                })?
                .path();
            if path.extension().and_then(|value| value.to_str()) != Some("json")
                || path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|name| name.ends_with(".initialization.json"))
            {
                continue;
            }
            let record: AdvanceRecord = read_json(&path)?;
            if record.project_id != project_id || record.issue_id != issue_id {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "advance_record",
                    id: record.id,
                });
            }
            records.push(record);
        }
        records.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(records)
    }

    pub fn get_advance_by_command_id(
        &self,
        project_id: &str,
        issue_id: &str,
        command_id: &str,
    ) -> Result<Option<AdvanceRecord>, ProductStoreError> {
        validate_relative_id(command_id)?;
        self.with_existing_root(project_id, issue_id, || {
            Ok(self
                .records(project_id, issue_id)?
                .into_iter()
                .find(|record| record.command_id == command_id))
        })
    }

    pub fn get_advance_for_plan(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<Option<AdvanceRecord>, ProductStoreError> {
        validate_relative_id(plan_id)?;
        self.with_existing_root(project_id, issue_id, || {
            Ok(self
                .records(project_id, issue_id)?
                .into_iter()
                .find(|record| record.plan_id == plan_id))
        })
    }

    /// Returns whether the durable advance completed the exact attempt bound to
    /// the plan. A missing record, a non-ready status, or an attempt mismatch is
    /// fail-closed so an SC attempt cannot bypass the advance gate.
    ///
    /// REQ-ADV-01/02（OQ1，WP2 additive 扩展）：判定=「`attempt_id` 精确匹配 OR
    /// 属于 `target_attempts` 集」——多 target 拆分下每 target-attempt 各自
    /// per-attempt 放行；缺记录/未 Ready/双不匹配一律 false（fail-closed 语义
    /// 零变化，不引入 plan 级聚合 Ready）。
    pub fn advance_is_ready_for_attempt(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        attempt_id: &str,
    ) -> Result<bool, ProductStoreError> {
        Ok(self
            .get_advance_for_plan(project_id, issue_id, plan_id)?
            .is_some_and(|record| {
                record.status == AdvanceStatus::Ready
                    && (record.attempt_id.as_deref() == Some(attempt_id)
                        || record
                            .target_attempts
                            .iter()
                            .any(|binding| binding.attempt_id == attempt_id))
            }))
    }

    /// Test/next-stage orchestration hook. Task 5.1 deliberately never calls this
    /// on a first request; Task 5.2 owns the first durable record write.
    pub fn put_record(
        &self,
        record: &AdvanceRecord,
    ) -> Result<AdvanceRecord, ProductStoreError> {
        validate_relative_id(&record.id)?;
        validate_relative_id(&record.command_id)?;
        validate_relative_id(&record.project_id)?;
        validate_relative_id(&record.issue_id)?;
        validate_relative_id(&record.plan_id)?;
        validate_relative_id(&record.plan_revision_id)?;
        let path = self.path_for(&record.project_id, &record.issue_id, &record.id)?;
        let root = self.root(&record.project_id, &record.issue_id)?;
        with_exclusive_lock(&root, || {
            if path.exists() {
                let existing: AdvanceRecord = read_json(&path)?;
                validate_advance_record_identity(&existing)?;
                if existing == *record {
                    return Ok(existing);
                }
                return Err(ProductStoreError::Conflict {
                    kind: "advance_record",
                    id: record.id.clone(),
                });
            }
            if self
                .records(&record.project_id, &record.issue_id)?
                .into_iter()
                .any(|existing| {
                    existing.command_id == record.command_id || existing.plan_id == record.plan_id
                })
            {
                return Err(ProductStoreError::Conflict {
                    kind: "advance_record_identity",
                    id: record.id.clone(),
                });
            }
            write_json(&path, record)?;
            Ok(record.clone())
        })
    }

    /// Persist the first durable anchor for a valid advance. Existing command/plan
    /// records are returned unchanged so callers can safely resume without allocating
    /// another timestamp or identifier.
    pub fn persist_advance_record_if_absent(
        &self,
        input: &AdvanceInput,
        plan_revision_id: &str,
    ) -> Result<AdvanceRecord, ProductStoreError> {
        for id in [
            input.command_id.as_str(),
            input.project_id.as_str(),
            input.issue_id.as_str(),
            input.plan_id.as_str(),
            plan_revision_id,
        ] {
            validate_relative_id(id)?;
        }
        if let Some(record) =
            self.get_advance_by_command_id(&input.project_id, &input.issue_id, &input.command_id)?
        {
            return Ok(record);
        }
        if let Some(record) =
            self.get_advance_for_plan(&input.project_id, &input.issue_id, &input.plan_id)?
        {
            return Ok(record);
        }
        let record = self.now_record(input, plan_revision_id.to_string());
        match self.put_record(&record) {
            Ok(record) => Ok(record),
            Err(ProductStoreError::Conflict { .. }) => self
                .get_advance_by_command_id(&input.project_id, &input.issue_id, &input.command_id)?
                .or_else(|| {
                    self.get_advance_for_plan(&input.project_id, &input.issue_id, &input.plan_id)
                        .ok()
                        .flatten()
                })
                .ok_or(ProductStoreError::Conflict {
                    kind: "advance_record_identity",
                    id: record.id,
                }),
            Err(error) => Err(error),
        }
    }

    /// Update only mutable state while enforcing the immutable advance identity.
    pub fn update_record(
        &self,
        expected: &AdvanceRecord,
    ) -> Result<AdvanceRecord, ProductStoreError> {
        let path = self.path_for(&expected.project_id, &expected.issue_id, &expected.id)?;
        let root = self.root(&expected.project_id, &expected.issue_id)?;
        with_exclusive_lock(&root, || {
            let current: AdvanceRecord = read_json(&path)?;
            if current.project_id != expected.project_id
                || current.issue_id != expected.issue_id
                || current.plan_id != expected.plan_id
                || current.command_id != expected.command_id
                || current.plan_revision_id != expected.plan_revision_id
            {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "advance_record",
                    id: expected.id.clone(),
                });
            }
            write_json(&path, expected)?;
            Ok(expected.clone())
        })
    }

    pub fn get_advance_initialization(
        &self,
        record: &AdvanceRecord,
    ) -> Result<Option<AdvanceInitializationJournal>, ProductStoreError> {
        validate_advance_record_identity(record)?;
        let path = self.initialization_path(record)?;
        if !path.is_file() {
            return Ok(None);
        }
        let journal: AdvanceInitializationJournal = read_json(&path)?;
        validate_advance_initialization_journal(&journal, record)?;
        Ok(Some(journal))
    }
    #[cfg(test)]
    pub(crate) fn put_advance_initialization_if_absent(
        &self,
        record: &AdvanceRecord,
        attempt_id: &str,
    ) -> Result<AdvanceInitializationJournal, ProductStoreError> {
        validate_advance_record_identity(record)?;
        validate_relative_id(attempt_id)?;
        let path = self.initialization_path(record)?;
        let root = self.root(&record.project_id, &record.issue_id)?;
        with_exclusive_lock(&root, || {
            if path.is_file() {
                let journal: AdvanceInitializationJournal = read_json(&path)?;
                validate_advance_initialization_journal(&journal, record)?;
                if journal.attempt_id != attempt_id {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "advance_initialization_journal_attempt",
                        id: record.id.clone(),
                    });
                }
                return Ok(journal);
            }
            let now = Utc::now().to_rfc3339();
            let journal = AdvanceInitializationJournal {
                advance_id: record.id.clone(),
                plan_id: record.plan_id.clone(),
                attempt_id: attempt_id.to_string(),
                target_attempt_ids: Vec::new(),
                phase: AdvanceInitializationPhase::JournalPrepared,
                error: None,
                created_at: now.clone(),
                updated_at: now,
            };
            write_json(&path, &journal)?;
            Ok(journal)
        })
    }

    pub fn mark_advance_initialization_error(
        &self,
        record: &AdvanceRecord,
        journal: &AdvanceInitializationJournal,
        error: &str,
    ) -> Result<(AdvanceRecord, AdvanceInitializationJournal), ProductStoreError> {
        validate_advance_record_identity(record)?;
        validate_advance_initialization_journal(journal, record)?;
        let mut failed_record = record.clone();
        failed_record.status = AdvanceStatus::Failed;
        failed_record.error = Some(error.to_string());
        failed_record.updated_at = Utc::now().to_rfc3339();
        let mut failed_journal = journal.clone();
        failed_journal.error = Some(error.to_string());
        failed_journal.updated_at = Utc::now().to_rfc3339();
        self.update_record(&failed_record)?;
        self.save_advance_initialization(&failed_record, &failed_journal)?;
        Ok((failed_record, failed_journal))
    }

    pub fn save_advance_initialization(
        &self,
        record: &AdvanceRecord,
        journal: &AdvanceInitializationJournal,
    ) -> Result<AdvanceInitializationJournal, ProductStoreError> {
        validate_advance_record_identity(record)?;
        validate_advance_initialization_journal(journal, record)?;
        let path = self.initialization_path(record)?;
        write_json(&path, journal)?;
        Ok(journal.clone())
    }
    /// The group journal is the durable source for this phase; the outer journal only records the
    /// corresponding checkpoint after the group-side write succeeds.
    pub fn load_or_prepare_advance_initialization(
        &self,
        record: &AdvanceRecord,
        group: &crate::product::coding_attempt_store::CodingGroupInitializationJournal,
    ) -> Result<AdvanceInitializationJournal, ProductStoreError> {
        self.load_or_prepare_advance_initialization_for_attempts(
            record,
            &group.attempt.id,
            std::slice::from_ref(&group.attempt.id),
        )
    }

    /// REQ-ADV-01/02（WP2 多 target 拆分）：外层 advance journal 的集绑定形态。
    /// `first_attempt_id` 恒为拓扑序首个（兼容旧消费面非空假设），
    /// `target_attempt_ids` 为全集（集合一致性比对，顺序不敏感）。
    pub fn load_or_prepare_advance_initialization_for_attempts(
        &self,
        record: &AdvanceRecord,
        first_attempt_id: &str,
        target_attempt_ids: &[String],
    ) -> Result<AdvanceInitializationJournal, ProductStoreError> {
        validate_advance_record_identity(record)?;
        validate_relative_id(first_attempt_id)?;
        for attempt_id in target_attempt_ids {
            validate_relative_id(attempt_id)?;
        }
        let path = self.initialization_path(record)?;
        if path.is_file() {
            let journal: AdvanceInitializationJournal = read_json(&path)?;
            validate_advance_initialization_journal(&journal, record)?;
            if journal.attempt_id != first_attempt_id {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "advance_initialization_journal_attempt",
                    id: record.id.clone(),
                });
            }
            let mut persisted = journal.target_attempt_ids.clone();
            let mut expected = target_attempt_ids.to_vec();
            persisted.sort_unstable();
            expected.sort_unstable();
            if persisted != expected {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "advance_initialization_journal_attempt",
                    id: record.id.clone(),
                });
            }
            return Ok(journal);
        }
        let now = Utc::now().to_rfc3339();
        let journal = AdvanceInitializationJournal {
            advance_id: record.id.clone(),
            plan_id: record.plan_id.clone(),
            attempt_id: first_attempt_id.to_string(),
            target_attempt_ids: target_attempt_ids.to_vec(),
            phase: AdvanceInitializationPhase::JournalPrepared,
            error: None,
            created_at: now.clone(),
            updated_at: now,
        };
        write_json(&path, &journal)?;
        Ok(journal)
    }

    pub fn advance_initialization_phase(
        &self,
        record: &AdvanceRecord,
        expected: &AdvanceInitializationJournal,
        next: AdvanceInitializationPhase,
    ) -> Result<AdvanceInitializationJournal, ProductStoreError> {
        validate_advance_record_identity(record)?;
        validate_advance_initialization_journal(expected, record)?;
        let path = self.initialization_path(record)?;
        let root = self.root(&record.project_id, &record.issue_id)?;
        with_exclusive_lock(&root, || {
            let mut current: AdvanceInitializationJournal = read_json(&path)?;
            validate_advance_initialization_journal(&current, record)?;
            if current != *expected {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "advance_initialization_journal",
                    id: record.id.clone(),
                });
            }
            if current.phase == next {
                return Ok(current);
            }
            if next.order() != current.phase.order() + 1 {
                return Err(ProductStoreError::Conflict {
                    kind: "advance_initialization_phase",
                    id: record.id.clone(),
                });
            }
            current.phase = next;
            current.error = None;
            current.updated_at = Utc::now().to_rfc3339();
            write_json(&path, &current)?;
            Ok(current)
        })
    }

    fn initialization_path(&self, record: &AdvanceRecord) -> Result<PathBuf, ProductStoreError> {
        Ok(self
            .root(&record.project_id, &record.issue_id)?
            .join(format!("{}.initialization.json", record.plan_id)))
    }

    pub fn now_record(&self, input: &AdvanceInput, plan_revision_id: String) -> AdvanceRecord {
        let now = Utc::now().to_rfc3339();
        AdvanceRecord {
            id: format!("advance_{}", input.command_id),
            command_id: input.command_id.clone(),
            project_id: input.project_id.clone(),
            issue_id: input.issue_id.clone(),
            plan_id: input.plan_id.clone(),
            plan_revision_id,
            attempt_id: None,
            target_attempts: Vec::new(),
            status: AdvanceStatus::Initializing,
            workspace_entry: None,
            error: None,
            created_at: now.clone(),
            updated_at: now,
        }
    }
}

pub use AdvanceRecord as Record;

fn validate_advance_record_identity(record: &AdvanceRecord) -> Result<(), ProductStoreError> {
    for id in [
        record.id.as_str(),
        record.command_id.as_str(),
        record.project_id.as_str(),
        record.issue_id.as_str(),
        record.plan_id.as_str(),
        record.plan_revision_id.as_str(),
    ] {
        validate_relative_id(id)?;
    }
    Ok(())
}

fn validate_advance_initialization_journal(
    journal: &AdvanceInitializationJournal,
    record: &AdvanceRecord,
) -> Result<(), ProductStoreError> {
    for id in [
        journal.advance_id.as_str(),
        journal.plan_id.as_str(),
        journal.attempt_id.as_str(),
    ] {
        validate_relative_id(id)?;
    }
    if journal.advance_id != record.id || journal.plan_id != record.plan_id {
        return Err(ProductStoreError::IdentityMismatch {
            kind: "advance_initialization_journal",
            id: record.id.clone(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn record() -> AdvanceRecord {
        AdvanceRecord {
            id: "advance_0001".into(),
            command_id: "command_0001".into(),
            project_id: "project_0001".into(),
            issue_id: "issue_0001".into(),
            plan_id: "plan_0001".into(),
            plan_revision_id: "revision_0001".into(),
            attempt_id: Some("attempt_0001".into()),
            target_attempts: Vec::new(),
            status: AdvanceStatus::Ready,
            workspace_entry: Some("/workspaces/attempt_0001".into()),
            error: None,
            created_at: "2026-08-31T00:00:00Z".into(),
            updated_at: "2026-08-31T00:00:01Z".into(),
        }
    }

    #[test]
    fn advance_record_store_supports_both_idempotency_indexes() {
        let root = TempDir::new().unwrap();
        let store = AdvanceStore::new(ProductAppPaths::new(root.path().join(".aria")));
        let value = record();
        assert_eq!(store.put_record(&value).unwrap(), value);
        assert_eq!(
            store
                .get_advance_by_command_id("project_0001", "issue_0001", "command_0001")
                .unwrap(),
            Some(value.clone())
        );
        assert_eq!(
            store
                .get_advance_for_plan("project_0001", "issue_0001", "plan_0001")
                .unwrap(),
            Some(value)
        );
    }

    #[test]
    fn persist_advance_record_if_absent_is_idempotent_and_binds_revision() {
        let root = TempDir::new().unwrap();
        let store = AdvanceStore::new(ProductAppPaths::new(root.path().join(".aria")));
        let input = AdvanceInput {
            command_id: "command_0001".into(),
            project_id: "project_0001".into(),
            issue_id: "issue_0001".into(),
            plan_id: "plan_0001".into(),
        };
        let first = store
            .persist_advance_record_if_absent(&input, "revision_0001")
            .unwrap();
        let replay = store
            .persist_advance_record_if_absent(&input, "revision_changed")
            .unwrap();
        assert_eq!(first, replay);
        assert_eq!(replay.plan_revision_id, "revision_0001");
        assert_eq!(replay.status, AdvanceStatus::Initializing);
    }

    #[test]
    fn persist_advance_record_if_absent_replays_by_plan_before_new_command() {
        let root = TempDir::new().unwrap();
        let store = AdvanceStore::new(ProductAppPaths::new(root.path().join(".aria")));
        let first_input = AdvanceInput {
            command_id: "command_0001".into(),
            project_id: "project_0001".into(),
            issue_id: "issue_0001".into(),
            plan_id: "plan_0001".into(),
        };
        let second_input = AdvanceInput {
            command_id: "command_0002".into(),
            ..first_input.clone()
        };
        let first = store
            .persist_advance_record_if_absent(&first_input, "revision_0001")
            .unwrap();
        assert_eq!(
            store
                .persist_advance_record_if_absent(&second_input, "revision_0002")
                .unwrap(),
            first
        );
    }

    #[test]
    fn advance_initialization_journal_rejects_identity_drift() {
        let record = record();
        let journal = AdvanceInitializationJournal {
            advance_id: "advance_other".into(),
            plan_id: record.plan_id.clone(),
            attempt_id: "attempt_0001".into(),
            target_attempt_ids: Vec::new(),
            phase: AdvanceInitializationPhase::JournalPrepared,
            error: None,
            created_at: record.created_at.clone(),
            updated_at: record.updated_at.clone(),
        };
        assert!(matches!(
            validate_advance_initialization_journal(&journal, &record),
            Err(ProductStoreError::IdentityMismatch { .. })
        ));
    }

    fn target_binding(target: &str, attempt: &str) -> AdvanceTargetAttemptBinding {
        AdvanceTargetAttemptBinding {
            target_repository_id: target.to_string(),
            attempt_id: attempt.to_string(),
        }
    }

    #[test]
    fn advance_ready_judgment_keeps_exact_match_semantics_for_single_target() {
        // REQ-ADV-02（约束 5 单 target 零变化）：单值 attempt_id 精确匹配语义逐字节
        // 保持——空 target_attempts 集下仅精确匹配放行，其余一律 fail-closed。
        let root = TempDir::new().unwrap();
        let store = AdvanceStore::new(ProductAppPaths::new(root.path().join(".aria")));
        let mut value = record();
        value.target_attempts = Vec::new();
        store.put_record(&value).unwrap();

        assert!(store
            .advance_is_ready_for_attempt("project_0001", "issue_0001", "plan_0001", "attempt_0001")
            .unwrap());
        assert!(!store
            .advance_is_ready_for_attempt("project_0001", "issue_0001", "plan_0001", "attempt_other")
            .unwrap());
        assert!(!store
            .advance_is_ready_for_attempt("project_0001", "issue_0001", "plan_missing", "attempt_0001")
            .unwrap());
    }

    #[test]
    fn advance_ready_judgment_accepts_target_attempt_set_membership() {
        // REQ-ADV-01/02（OQ1）：多 target record 的判定=精确匹配 OR 集合包含——
        // 每 target-attempt 各自 per-attempt 放行。
        let root = TempDir::new().unwrap();
        let store = AdvanceStore::new(ProductAppPaths::new(root.path().join(".aria")));
        let mut value = record();
        value.attempt_id = Some("attempt_first".into());
        value.target_attempts = vec![
            target_binding("11111111-1111-1111-1111-111111111111", "attempt_first"),
            target_binding("22222222-2222-2222-2222-222222222222", "attempt_second"),
        ];
        store.put_record(&value).unwrap();

        assert!(store
            .advance_is_ready_for_attempt("project_0001", "issue_0001", "plan_0001", "attempt_first")
            .unwrap());
        assert!(store
            .advance_is_ready_for_attempt("project_0001", "issue_0001", "plan_0001", "attempt_second")
            .unwrap());
        // 双不匹配 fail-closed（守卫零变化红线）。
        assert!(!store
            .advance_is_ready_for_attempt("project_0001", "issue_0001", "plan_0001", "attempt_unbound")
            .unwrap());
    }

    #[test]
    fn advance_ready_judgment_fails_closed_when_not_ready_or_unbound() {
        let root = TempDir::new().unwrap();
        let store = AdvanceStore::new(ProductAppPaths::new(root.path().join(".aria")));
        let mut value = record();
        value.status = AdvanceStatus::Initializing;
        value.target_attempts = vec![target_binding(
            "11111111-1111-1111-1111-111111111111",
            "attempt_0001",
        )];
        store.put_record(&value).unwrap();

        // 未 Ready 一律拒绝（缺记录由上一测试覆盖）。
        assert!(!store
            .advance_is_ready_for_attempt("project_0001", "issue_0001", "plan_0001", "attempt_0001")
            .unwrap());
    }

    #[test]
    fn legacy_advance_record_json_deserializes_without_target_attempts() {
        // REQ-MTG-03 scenario「绑定关系 additive 扩展兼容存量」：本 change 前形态
        // JSON（无 target_attempts 键）反序列化 → 空集，判定路径与语义零变化。
        let legacy_json = serde_json::json!({
            "id": "advance_0001",
            "command_id": "command_0001",
            "project_id": "project_0001",
            "issue_id": "issue_0001",
            "plan_id": "plan_0001",
            "plan_revision_id": "revision_0001",
            "attempt_id": "attempt_0001",
            "status": "ready",
            "workspace_entry": "/workspaces/attempt_0001",
            "error": null,
            "created_at": "2026-08-31T00:00:00Z",
            "updated_at": "2026-08-31T00:00:01Z"
        });
        let value: AdvanceRecord = serde_json::from_value(legacy_json).unwrap();
        assert_eq!(value.attempt_id.as_deref(), Some("attempt_0001"));
        assert!(value.target_attempts.is_empty());
        // 序列化往返：空集不落键（additive-only wire）。
        let serialized = serde_json::to_value(&value).unwrap();
        assert!(serialized.get("target_attempts").is_none());
    }

    #[test]
    fn advance_initialization_journal_legacy_json_defaults_target_attempt_ids() {
        let legacy_json = serde_json::json!({
            "advance_id": "advance_0001",
            "plan_id": "plan_0001",
            "attempt_id": "attempt_0001",
            "phase": "journal_prepared",
            "error": null,
            "created_at": "2026-08-31T00:00:00Z",
            "updated_at": "2026-08-31T00:00:01Z"
        });
        let journal: AdvanceInitializationJournal = serde_json::from_value(legacy_json).unwrap();
        assert_eq!(journal.attempt_id, "attempt_0001");
        assert!(journal.target_attempt_ids.is_empty());
    }
}
