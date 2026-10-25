use std::path::PathBuf;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::locking::with_exclusive_lock;
use crate::product::entity_deletion_store::{DeletionEntityKind, EntityDeletionStore, EntityScope};
use crate::product::entity_operation_store::{EntityDeletionLease, EntityOperationStore};
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::automation::{RetryInitializationRecord, RetryInitializationRequest};
use chrono::Utc;
use serde::{Deserialize, Serialize};

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
    /// 批 2 R1（REQ-WIGA-02/03）：首次 persist 时一次写定的首启策略冻结。
    /// additive：旧 record 缺字段反序列化读 `Manual`（旧 Manual 不升格）；
    /// update/replay 不许改（`update_record` 身份断言参与比较）。
    #[serde(default)]
    pub start_run_policy: crate::product::coding_models::CodingStartRunPolicy,
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
/// plan Task D1.2c（§0.2/§0.5）：Plan 拥有的 advance 闭包纯 ID 集合——
/// `advance_ids`=该 plan 全部 record ID（含孤儿 initialization journal 的归属），
/// `retry_command_ids`=按 advance_id join 选中的独立 retry command ID。只真实
/// ID、无内容，供 `EntityDeletionManifest` 的 `advance_ids`/
/// `advance_retry_command_ids` 与 `purge_plan_artifacts` 消费。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AdvancePlanArtifacts {
    pub advance_ids: Vec<String>,
    pub retry_command_ids: Vec<String>,
}
/// H2b（设计 A 节，整组删除与重启）：零成员/最后成员重置闭包与开始入口的
/// advance 侧只读交叉核对事实。「全删」判据不是 issue 无 attempts：要按
/// (project,issue,plan) 枚举真实 group attempts，并与 run、所有该 plan
/// advance records、外层 journal 交叉核对——本结构承载 advance 侧引用，
/// attempt 存活集由调用方枚举后经 `classify_stale_references` 判定。
/// fail-closed：坏 JSON/身份错配/IO 错误原样上抛，不当空集。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvancePlanExecutionReferences {
    /// 该 plan 全部 advance records（created_at/id 升序；含 legacy 同 plan
    /// 多 record 形态，不只首条）。
    pub records: Vec<AdvanceRecord>,
    /// 外层 initialization journal（`{plan_id}.initialization.json`）；缺失
    /// 为 `None`。
    pub initialization_journal: Option<AdvanceInitializationJournal>,
}

/// 「旧 Ready 指向已死全集」识别结果（设计 A 节状态裁定表/B 节错误族）：
/// `AllTargetsDead` → 交零成员闭包完整重置后新建；`PartiallyDead` →
/// Invalidated＋清剩余出口（不缩水全集、不收养）；`Intact` → 正常重放或
/// 初始化前缀续做；`NoReferences` → 无 advance 执行引用（闭包只清其余
/// 对象）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdvanceStaleReferenceVerdict {
    NoReferences,
    Intact,
    /// 被引用全集皆缺（旧 Ready 或已越 AttemptPersisted 的外层 journal 指向
    /// 缺失 attempts）。
    AllTargetsDead {
        dead_attempt_ids: Vec<String>,
    },
    /// 仅部分被引用 attempt 缺失——剩余成员在，不得自动清。
    PartiallyDead {
        dead_attempt_ids: Vec<String>,
        alive_attempt_ids: Vec<String>,
    },
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

impl AdvancePlanExecutionReferences {
    /// 旧 Ready record 绑定的 attempt 全集（单值 `attempt_id` ∪
    /// `target_attempts`，升序去重）。非 Ready record 不构成「旧 Ready
    /// 指向」事实。
    pub fn ready_bound_attempt_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .records
            .iter()
            .filter(|record| record.status == AdvanceStatus::Ready)
            .flat_map(|record| {
                let mut bound: Vec<String> = record
                    .target_attempts
                    .iter()
                    .map(|binding| binding.attempt_id.clone())
                    .collect();
                bound.extend(record.attempt_id.iter().cloned());
                bound
            })
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }

    /// 外层 journal 绑定的 attempt 全集（`attempt_id` ∪ `target_attempt_ids`，
    /// 升序去重）；无 journal 为空集。
    pub fn journal_bound_attempt_ids(&self) -> Vec<String> {
        let Some(journal) = &self.initialization_journal else {
            return Vec::new();
        };
        let mut ids = journal.target_attempt_ids.clone();
        ids.push(journal.attempt_id.clone());
        ids.sort();
        ids.dedup();
        ids
    }

    /// 按调用方枚举的真实 attempt 存活集分类旧执行引用。断裂引用集 =
    /// Ready 绑定 ∪（外层 journal 已越 AttemptPersisted 时的 journal 绑定）
    /// ——journal 未越 AttemptPersisted 的绑定是初始化前缀（attempt 尚未
    /// 物化属预期缺失），不计断裂（设计 A 节：「不能仅因前缀判零成员重置，
    /// 正常初始化前缀继续原协议」）。判定只做集合差，不读 attempt 文件
    ///（权威存活集归闭包/调用方，不复制第二套扫描）。
    pub fn classify_stale_references(
        &self,
        existing_attempt_ids: &[String],
    ) -> AdvanceStaleReferenceVerdict {
        if self.records.is_empty() && self.initialization_journal.is_none() {
            return AdvanceStaleReferenceVerdict::NoReferences;
        }
        let mut referenced = self.ready_bound_attempt_ids();
        let journal_beyond_attempt_persisted =
            self.initialization_journal.as_ref().is_some_and(|journal| {
                journal.phase.order_for_engine()
                    >= AdvanceInitializationPhase::AttemptPersisted.order_for_engine()
            });
        if journal_beyond_attempt_persisted {
            referenced.extend(self.journal_bound_attempt_ids());
        }
        referenced.sort();
        referenced.dedup();
        if referenced.is_empty() {
            return AdvanceStaleReferenceVerdict::Intact;
        }
        let existing: std::collections::BTreeSet<String> =
            existing_attempt_ids.iter().cloned().collect();
        let mut dead_attempt_ids = Vec::new();
        let mut alive_attempt_ids = Vec::new();
        for attempt_id in referenced {
            if existing.contains(&attempt_id) {
                alive_attempt_ids.push(attempt_id);
            } else {
                dead_attempt_ids.push(attempt_id);
            }
        }
        if dead_attempt_ids.is_empty() {
            AdvanceStaleReferenceVerdict::Intact
        } else if alive_attempt_ids.is_empty() {
            AdvanceStaleReferenceVerdict::AllTargetsDead { dead_attempt_ids }
        } else {
            AdvanceStaleReferenceVerdict::PartiallyDead {
                dead_attempt_ids,
                alive_attempt_ids,
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
        // plan Task D1.2c：删除事实优先——purging/deleted 一律 Ok(false)；
        // 未删 Plan 保持原精确 join 语义零变化（REQ-GRP-01 scenario
        // 「删除终止 advance 重放与 Ready 放行」）。
        let scope = Self::plan_scope(project_id, issue_id, plan_id)?;
        if EntityDeletionStore::new(self.app_paths.clone())
            .get(&scope)?
            .is_some()
        {
            return Ok(false);
        }
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
    pub fn put_record(&self, record: &AdvanceRecord) -> Result<AdvanceRecord, ProductStoreError> {
        validate_relative_id(&record.id)?;
        validate_relative_id(&record.command_id)?;
        validate_relative_id(&record.project_id)?;
        validate_relative_id(&record.issue_id)?;
        validate_relative_id(&record.plan_id)?;
        validate_relative_id(&record.plan_revision_id)?;
        let path = self.path_for(&record.project_id, &record.issue_id, &record.id)?;
        let root = self.root(&record.project_id, &record.issue_id)?;
        self.with_plan_live_write(
            &record.project_id,
            &record.issue_id,
            &record.plan_id,
            || {
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
                            existing.command_id == record.command_id
                                || existing.plan_id == record.plan_id
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
            },
        )
    }

    /// Persist the first durable anchor for a valid advance. Existing command/plan
    /// records are returned unchanged so callers can safely resume without allocating
    /// another timestamp or identifier.
    ///
    /// 批 2 R1（REQ-WIGA-02/03）：`start_run_policy` 显式接策略参数（不留旧
    /// 签名 shim）——首次 persist 连策略一次写定；既存 record 原样返回
    /// （冻结值以 record 为准，调用方互证不符即停，不覆盖旧 Manual）。
    pub fn persist_advance_record_if_absent(
        &self,
        input: &AdvanceInput,
        plan_revision_id: &str,
        start_run_policy: crate::product::coding_models::CodingStartRunPolicy,
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
        // plan Task D1.2c＋P3 修复轮：准入（scope try-shared）跨「重放→put」
        // 全流程持有——删除独占无法在预检与 put 之间插入，purging/deleted
        // 事实在入口即拒；put_record 的 entity_deleting 因此不可能落入下方
        // 身份兜底被改判（REQ-GRP-01 删除身份优先）。
        self.with_plan_live_write(&input.project_id, &input.issue_id, &input.plan_id, || {
            if let Some(record) = self.get_advance_by_command_id(
                &input.project_id,
                &input.issue_id,
                &input.command_id,
            )? {
                return Ok(record);
            }
            if let Some(record) =
                self.get_advance_for_plan(&input.project_id, &input.issue_id, &input.plan_id)?
            {
                return Ok(record);
            }
            let record = self.now_record(input, plan_revision_id.to_string(), start_run_policy);
            match self.put_record(&record) {
                Ok(record) => Ok(record),
                Err(ProductStoreError::Conflict { .. }) => self
                    .get_advance_by_command_id(
                        &input.project_id,
                        &input.issue_id,
                        &input.command_id,
                    )?
                    .or_else(|| {
                        self.get_advance_for_plan(
                            &input.project_id,
                            &input.issue_id,
                            &input.plan_id,
                        )
                        .ok()
                        .flatten()
                    })
                    .ok_or(ProductStoreError::Conflict {
                        kind: "advance_record_identity",
                        id: record.id,
                    }),
                Err(error) => Err(error),
            }
        })
    }

    /// Update only mutable state while enforcing the immutable advance identity.
    pub fn update_record(
        &self,
        expected: &AdvanceRecord,
    ) -> Result<AdvanceRecord, ProductStoreError> {
        let path = self.path_for(&expected.project_id, &expected.issue_id, &expected.id)?;
        let root = self.root(&expected.project_id, &expected.issue_id)?;
        self.with_plan_live_write(
            &expected.project_id,
            &expected.issue_id,
            &expected.plan_id,
            || {
                with_exclusive_lock(&root, || {
                    let current: AdvanceRecord = read_json(&path)?;
                    if current.project_id != expected.project_id
                        || current.issue_id != expected.issue_id
                        || current.plan_id != expected.plan_id
                        || current.command_id != expected.command_id
                        || current.plan_revision_id != expected.plan_revision_id
                        // 批 2 R1：start_run_policy 首次 persist 冻结，update/
                        // replay 不许改（身份断言 fail-closed）。
                        || current.start_run_policy != expected.start_run_policy
                    {
                        return Err(ProductStoreError::IdentityMismatch {
                            kind: "advance_record",
                            id: expected.id.clone(),
                        });
                    }
                    write_json(&path, expected)?;
                    Ok(expected.clone())
                })
            },
        )
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
        self.with_plan_live_write(
            &record.project_id,
            &record.issue_id,
            &record.plan_id,
            || {
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
            },
        )
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
        self.with_plan_live_write(
            &record.project_id,
            &record.issue_id,
            &record.plan_id,
            || {
                write_json(&path, journal)?;
                Ok(journal.clone())
            },
        )
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
        self.with_plan_live_write(
            &record.project_id,
            &record.issue_id,
            &record.plan_id,
            || {
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
            },
        )
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
        self.with_plan_live_write(
            &record.project_id,
            &record.issue_id,
            &record.plan_id,
            || {
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
            },
        )
    }

    fn initialization_path(&self, record: &AdvanceRecord) -> Result<PathBuf, ProductStoreError> {
        Ok(self
            .root(&record.project_id, &record.issue_id)?
            .join(format!("{}.initialization.json", record.plan_id)))
    }

    pub fn now_record(
        &self,
        input: &AdvanceInput,
        plan_revision_id: String,
        start_run_policy: crate::product::coding_models::CodingStartRunPolicy,
    ) -> AdvanceRecord {
        let now = Utc::now().to_rfc3339();
        AdvanceRecord {
            id: format!("advance_{}", input.command_id),
            command_id: input.command_id.clone(),
            project_id: input.project_id.clone(),
            issue_id: input.issue_id.clone(),
            plan_id: input.plan_id.clone(),
            plan_revision_id,
            attempt_id: None,
            start_run_policy,
            target_attempts: Vec::new(),
            status: AdvanceStatus::Initializing,
            workspace_entry: None,
            error: None,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    // ─── C1 Task 7（REQ-ADV-C1-RETRY）：显式 retry-initialization ───

    fn retries_root(&self, project_id: &str, issue_id: &str) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        Ok(self
            .app_paths
            .issue_root(project_id, issue_id)
            .join("advance-retries"))
    }

    fn retry_path(
        &self,
        project_id: &str,
        issue_id: &str,
        command_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(command_id)?;
        Ok(self
            .retries_root(project_id, issue_id)?
            .join(format!("{command_id}.json")))
    }

    /// 读取独立 durable retry 记录（`advance-retries/{command_id}.json`）。
    pub fn get_retry_initialization(
        &self,
        project_id: &str,
        issue_id: &str,
        command_id: &str,
    ) -> Result<Option<RetryInitializationRecord>, ProductStoreError> {
        let path = self.retry_path(project_id, issue_id, command_id)?;
        if !path.is_file() {
            return Ok(None);
        }
        let record: RetryInitializationRecord = read_json(&path)?;
        if record.command_id != command_id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "advance_retry_initialization",
                id: command_id.to_string(),
            });
        }
        Ok(Some(record))
    }

    /// 追加独立 retry 事实：原 Failed record/journal 的失败审计不动；同
    /// `command_id` 已存在时幂等返回既有记录（不重写）。`state` 由调用方
    /// 决定（NeedsHuman＝等待未知副作用确认；Accepted＝已进入安全续做）。
    pub fn create_retry_initialization(
        &self,
        record: &AdvanceRecord,
        request: &RetryInitializationRequest,
        state: crate::product::models::automation::OperationState,
    ) -> Result<RetryInitializationRecord, ProductStoreError> {
        validate_relative_id(&request.command_id)?;
        self.with_plan_live_write(
            &record.project_id,
            &record.issue_id,
            &record.plan_id,
            || {
                let journal = self.get_advance_initialization(record)?.ok_or_else(|| {
                    ProductStoreError::NotFound {
                        kind: "advance_initialization_journal",
                        id: record.id.clone(),
                    }
                })?;
                if journal.attempt_id != request.expected_attempt_id {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "advance_retry_attempt",
                        id: request.expected_attempt_id.clone(),
                    });
                }
                let path =
                    self.retry_path(&record.project_id, &record.issue_id, &request.command_id)?;
                let root = self.retries_root(&record.project_id, &record.issue_id)?;
                with_exclusive_lock(&root, || {
                    if path.is_file() {
                        let existing: RetryInitializationRecord = read_json(&path)?;
                        if existing.command_id == request.command_id {
                            return Ok(existing);
                        }
                        return Err(ProductStoreError::Conflict {
                            kind: "advance_retry_initialization",
                            id: request.command_id.clone(),
                        });
                    }
                    let now = Utc::now().to_rfc3339();
                    let retry = RetryInitializationRecord {
                        retry_id: format!("advance_retry_{}", request.command_id),
                        command_id: request.command_id.clone(),
                        advance_id: record.id.clone(),
                        attempt_id: journal.attempt_id.clone(),
                        state,
                        checkpoint: journal.phase,
                        created_at: now.clone(),
                        updated_at: now,
                    };
                    write_json(&path, &retry)?;
                    Ok(retry)
                })
            },
        )
    }

    /// 更新 retry 记录状态（续做失败→NeedsHuman 等）；identity 不可变。
    pub fn update_retry_initialization_state(
        &self,
        project_id: &str,
        issue_id: &str,
        retry: &RetryInitializationRecord,
        state: crate::product::models::automation::OperationState,
    ) -> Result<RetryInitializationRecord, ProductStoreError> {
        let scope = self.plan_scope_for_advance(project_id, issue_id, &retry.advance_id)?;
        let path = self.retry_path(project_id, issue_id, &retry.command_id)?;
        let root = self.retries_root(project_id, issue_id)?;
        self.operation_store().with_live_write(&scope, || {
            with_exclusive_lock(&root, || {
                let mut current: RetryInitializationRecord = read_json(&path)?;
                if current.retry_id != retry.retry_id
                    || current.advance_id != retry.advance_id
                    || current.attempt_id != retry.attempt_id
                    || current.command_id != retry.command_id
                {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "advance_retry_initialization",
                        id: retry.retry_id.clone(),
                    });
                }
                current.state = state;
                current.updated_at = Utc::now().to_rfc3339();
                write_json(&path, &current)?;
                Ok(current)
            })
        })
    }

    /// CAS 重开 Failed record 供显式 retry 的安全续做：仅当当前 status
    /// 仍为 Failed 时置回 Initializing（created_at/error 失败审计保留），
    /// 使既有 checkpoint continuation 以同一 command/attempt 续进。
    pub fn reopen_failed_record_for_retry(
        &self,
        record: &AdvanceRecord,
    ) -> Result<AdvanceRecord, ProductStoreError> {
        validate_advance_record_identity(record)?;
        let path = self.path_for(&record.project_id, &record.issue_id, &record.id)?;
        let root = self.root(&record.project_id, &record.issue_id)?;
        self.with_plan_live_write(
            &record.project_id,
            &record.issue_id,
            &record.plan_id,
            || {
                with_exclusive_lock(&root, || {
                    let mut current: AdvanceRecord = read_json(&path)?;
                    validate_advance_record_identity(&current)?;
                    if current.id != record.id {
                        return Err(ProductStoreError::IdentityMismatch {
                            kind: "advance_record",
                            id: record.id.clone(),
                        });
                    }
                    if current.status != AdvanceStatus::Failed {
                        return Err(ProductStoreError::Conflict {
                            kind: "advance_retry_reopen",
                            id: format!("{}: status is {:?}", current.id, current.status),
                        });
                    }
                    current.status = AdvanceStatus::Initializing;
                    current.updated_at = Utc::now().to_rfc3339();
                    write_json(&path, &current)?;
                    Ok(current)
                })
            },
        )
    }

    // ─── 级联删除 Task D1.2c：Plan 拥有的 advance 闭包 list/purge ───

    fn operation_store(&self) -> EntityOperationStore {
        EntityOperationStore::new(self.app_paths.clone())
    }

    fn plan_scope(
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<EntityScope, ProductStoreError> {
        let scope = EntityScope::new(project_id, issue_id, DeletionEntityKind::Plan, plan_id);
        scope.validate()?;
        Ok(scope)
    }

    /// plan §0.2 owner 写准入：先取 scope try-shared（争用/墓碑一律立即
    /// `Conflict{entity_deleting}`，零阻塞等待），再进原 advance 根锁——
    /// 锁序恒为 scope → 对象锁，未删 Plan 原语义/CAS 零变化。
    fn with_plan_live_write<T>(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        operation: impl FnOnce() -> Result<T, ProductStoreError>,
    ) -> Result<T, ProductStoreError> {
        let scope = Self::plan_scope(project_id, issue_id, plan_id)?;
        self.operation_store().with_live_write(&scope, operation)
    }

    /// retry 更新无 plan_id：经 advance_id 反查所属 plan scope；record 不存在
    /// 时 fail-closed（root 缺不授权补造，plan §0.2「无 plan_id 不能猜」）。
    fn plan_scope_for_advance(
        &self,
        project_id: &str,
        issue_id: &str,
        advance_id: &str,
    ) -> Result<EntityScope, ProductStoreError> {
        let record = self
            .records(project_id, issue_id)?
            .into_iter()
            .find(|record| record.id == advance_id)
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "advance_record",
                id: advance_id.to_string(),
            })?;
        Self::plan_scope(&record.project_id, &record.issue_id, &record.plan_id)
    }

    fn with_existing_retries_root<T>(
        &self,
        project_id: &str,
        issue_id: &str,
        operation: impl FnOnce() -> Result<T, ProductStoreError>,
    ) -> Result<T, ProductStoreError> {
        let root = self.retries_root(project_id, issue_id)?;
        if !root.is_dir() {
            return operation();
        }
        with_exclusive_lock(&root, operation)
    }

    /// 枚举 issue 全部独立 retry 事实；文件名键与内容 command_id 不符=错 join
    /// 报错，坏 JSON 报错（fail-closed，不当空集）。
    fn retry_records(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<RetryInitializationRecord>, ProductStoreError> {
        let root = self.retries_root(project_id, issue_id)?;
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Vec::new());
            }
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
                    ProductStoreError::Io(format!("read advance retry entry: {error}"))
                })?
                .path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let record: RetryInitializationRecord = read_json(&path)?;
            let file_stem = path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_string();
            if file_stem != record.command_id {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "advance_retry_initialization",
                    id: file_stem,
                });
            }
            records.push(record);
        }
        Ok(records)
    }

    /// §0.5 list-by-owner：按 `plan_id` 全 issue 枚举 record（非仅
    /// `get_advance_for_plan` 首条），核 `{plan_id}.initialization.json` 的
    /// plan/advance 身份；retry 无 plan_id，一律按 advance_id join 选定
    /// record IDs，不猜。坏 JSON/错 join 报错（fail-closed），不依赖 Plan 根
    /// 或 pending manifest 存在，purging/deleted scope 亦可枚举残留。
    pub fn list_plan_artifacts(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<AdvancePlanArtifacts, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(plan_id)?;
        let journal_path = self
            .root(project_id, issue_id)?
            .join(format!("{plan_id}.initialization.json"));
        let mut advance_ids: Vec<String> = Vec::new();
        self.with_existing_root(project_id, issue_id, || {
            let records = self.records(project_id, issue_id)?;
            if journal_path.is_file() {
                let journal: AdvanceInitializationJournal = read_json(&journal_path)?;
                if journal.plan_id != plan_id {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "advance_initialization_journal",
                        id: plan_id.to_string(),
                    });
                }
                if records
                    .iter()
                    .any(|record| record.id == journal.advance_id && record.plan_id != plan_id)
                {
                    // journal 绑定到同 issue 他 Plan 的现存 record：错 join。
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "advance_initialization_journal_join",
                        id: journal.advance_id,
                    });
                }
                advance_ids.push(journal.advance_id.clone());
            }
            advance_ids.extend(
                records
                    .iter()
                    .filter(|record| record.plan_id == plan_id)
                    .map(|record| record.id.clone()),
            );
            advance_ids.sort();
            advance_ids.dedup();
            Ok(())
        })?;
        let mut retry_command_ids: Vec<String> = Vec::new();
        self.with_existing_retries_root(project_id, issue_id, || {
            for retry in self.retry_records(project_id, issue_id)? {
                if advance_ids.contains(&retry.advance_id) {
                    retry_command_ids.push(retry.command_id);
                }
            }
            retry_command_ids.sort();
            retry_command_ids.dedup();
            Ok(())
        })?;
        Ok(AdvancePlanArtifacts {
            advance_ids,
            retry_command_ids,
        })
    }

    /// 删除闭包清理（REQ-DEL-06/07、REQ-GRP-01）：仅独占 delete lease 调用；
    /// 清理序 retry → initialization journal → record；missing 幂等；不整删
    /// `advance-records`/`advance-retries` 根及其公共锁；内容身份不符即报错，
    /// 不误删他 Plan 对象。
    pub fn purge_plan_artifacts(
        &self,
        lease: &EntityDeletionLease,
        artifacts: &AdvancePlanArtifacts,
    ) -> Result<(), ProductStoreError> {
        let scope = lease.scope();
        scope.validate()?;
        if scope.kind != DeletionEntityKind::Plan {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "advance_purge_scope",
                id: scope.entity_id.clone(),
            });
        }
        self.purge_plan_artifacts_for_generation(
            scope.project_id.as_str(),
            scope.issue_id.as_str(),
            scope.entity_id.as_str(),
            artifacts,
        )
    }

    /// plan 代际取代的**原地演进**路径：按 plan 身份清 advance 产物
    /// （retry → initialization journal → record）。
    ///
    /// 与 [`Self::purge_plan_artifacts`] 的差别是**不消费删除独占**——原地
    /// 演进的 plan 根保留（不产生墓碑），但旧代 advance 产物必须清：残留的
    /// record 会被 advance 按 plan 回放（`Replayed`），新代永远进不了 coding。
    pub fn purge_plan_artifacts_for_generation(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        artifacts: &AdvancePlanArtifacts,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(plan_id)?;
        for id in artifacts
            .advance_ids
            .iter()
            .chain(artifacts.retry_command_ids.iter())
        {
            validate_relative_id(id)?;
        }
        // ① retry（advance-retries 根短独占）。
        self.with_existing_retries_root(project_id, issue_id, || {
            for command_id in &artifacts.retry_command_ids {
                let path = self.retry_path(project_id, issue_id, command_id)?;
                if !path.is_file() {
                    continue;
                }
                let existing: RetryInitializationRecord = read_json(&path)?;
                if existing.command_id != *command_id
                    || !artifacts.advance_ids.contains(&existing.advance_id)
                {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "advance_retry_initialization",
                        id: command_id.clone(),
                    });
                }
                std::fs::remove_file(&path).map_err(|error| {
                    ProductStoreError::Io(format!("remove {}: {error}", path.display()))
                })?;
            }
            Ok(())
        })?;
        // ② initialization journal → ③ record（advance-records 根短独占；
        //    根目录与公共锁不删）。
        self.with_existing_root(project_id, issue_id, || {
            let journal_path = self
                .root(project_id, issue_id)?
                .join(format!("{plan_id}.initialization.json"));
            if journal_path.is_file() {
                let journal: AdvanceInitializationJournal = read_json(&journal_path)?;
                if journal.plan_id != plan_id
                    || !artifacts.advance_ids.contains(&journal.advance_id)
                {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "advance_initialization_journal",
                        id: plan_id.to_string(),
                    });
                }
                std::fs::remove_file(&journal_path).map_err(|error| {
                    ProductStoreError::Io(format!("remove {}: {error}", journal_path.display()))
                })?;
            }
            for advance_id in &artifacts.advance_ids {
                let path = self.path_for(project_id, issue_id, advance_id)?;
                if !path.is_file() {
                    continue;
                }
                let existing: AdvanceRecord = read_json(&path)?;
                if existing.id != *advance_id || existing.plan_id != plan_id {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "advance_record",
                        id: advance_id.clone(),
                    });
                }
                std::fs::remove_file(&path).map_err(|error| {
                    ProductStoreError::Io(format!("remove {}: {error}", path.display()))
                })?;
            }
            Ok(())
        })
    }

    // ─── H2b（整组删除与重启）：旧执行引用只读交叉核对事实 ───

    /// 读取该 plan 的 advance 执行引用事实（全部 records + 外层
    /// initialization journal），供零成员/最后成员重置闭包与开始入口交叉
    /// 核对（设计 A 节「全删判据」）。purging/deleted scope 亦可枚举残留
    ///（同 `list_plan_artifacts` §0.5 语义）；journal 归属错配/错 join
    /// fail-closed，不当 `None`。
    pub fn plan_execution_references(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<AdvancePlanExecutionReferences, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(plan_id)?;
        let journal_path = self
            .root(project_id, issue_id)?
            .join(format!("{plan_id}.initialization.json"));
        self.with_existing_root(project_id, issue_id, || {
            let all_records = self.records(project_id, issue_id)?;
            let initialization_journal = if journal_path.is_file() {
                let journal: AdvanceInitializationJournal = read_json(&journal_path)?;
                if journal.plan_id != plan_id {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "advance_initialization_journal",
                        id: plan_id.to_string(),
                    });
                }
                if all_records
                    .iter()
                    .any(|record| record.id == journal.advance_id && record.plan_id != plan_id)
                {
                    // journal 绑定到同 issue 他 Plan 的现存 record：错 join。
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "advance_initialization_journal_join",
                        id: journal.advance_id.clone(),
                    });
                }
                Some(journal)
            } else {
                None
            };
            Ok(AdvancePlanExecutionReferences {
                records: all_records
                    .into_iter()
                    .filter(|record| record.plan_id == plan_id)
                    .collect(),
                initialization_journal,
            })
        })
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

include!("advance_store_tests.inc.rs");
