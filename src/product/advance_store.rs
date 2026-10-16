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
        for id in artifacts
            .advance_ids
            .iter()
            .chain(artifacts.retry_command_ids.iter())
        {
            validate_relative_id(id)?;
        }
        let (project_id, issue_id, plan_id) = (
            scope.project_id.as_str(),
            scope.issue_id.as_str(),
            scope.entity_id.as_str(),
        );
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
            start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
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
            .persist_advance_record_if_absent(
                &input,
                "revision_0001",
                crate::product::coding_models::CodingStartRunPolicy::Manual,
            )
            .unwrap();
        let replay = store
            .persist_advance_record_if_absent(
                &input,
                "revision_changed",
                crate::product::coding_models::CodingStartRunPolicy::Manual,
            )
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
            .persist_advance_record_if_absent(
                &first_input,
                "revision_0001",
                crate::product::coding_models::CodingStartRunPolicy::Manual,
            )
            .unwrap();
        assert_eq!(
            store
                .persist_advance_record_if_absent(
                    &second_input,
                    "revision_0002",
                    crate::product::coding_models::CodingStartRunPolicy::Manual,
                )
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

        assert!(
            store
                .advance_is_ready_for_attempt(
                    "project_0001",
                    "issue_0001",
                    "plan_0001",
                    "attempt_0001"
                )
                .unwrap()
        );
        assert!(
            !store
                .advance_is_ready_for_attempt(
                    "project_0001",
                    "issue_0001",
                    "plan_0001",
                    "attempt_other"
                )
                .unwrap()
        );
        assert!(
            !store
                .advance_is_ready_for_attempt(
                    "project_0001",
                    "issue_0001",
                    "plan_missing",
                    "attempt_0001"
                )
                .unwrap()
        );
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

        assert!(
            store
                .advance_is_ready_for_attempt(
                    "project_0001",
                    "issue_0001",
                    "plan_0001",
                    "attempt_first"
                )
                .unwrap()
        );
        assert!(
            store
                .advance_is_ready_for_attempt(
                    "project_0001",
                    "issue_0001",
                    "plan_0001",
                    "attempt_second"
                )
                .unwrap()
        );
        // 双不匹配 fail-closed（守卫零变化红线）。
        assert!(
            !store
                .advance_is_ready_for_attempt(
                    "project_0001",
                    "issue_0001",
                    "plan_0001",
                    "attempt_unbound"
                )
                .unwrap()
        );
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
        assert!(
            !store
                .advance_is_ready_for_attempt(
                    "project_0001",
                    "issue_0001",
                    "plan_0001",
                    "attempt_0001"
                )
                .unwrap()
        );
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

    // ─── 级联删除 Task D1.2c：advance record/journal/retry 闭包 ───

    use crate::product::entity_deletion_store::{
        DeletionEntityKind, DeletionPhase, EntityDeletionManifest, EntityDeletionStore, EntityScope,
    };
    use crate::product::entity_operation_store::EntityOperationStore;
    use crate::product::models::automation::OperationState;

    fn cascade_store() -> (TempDir, AdvanceStore) {
        let root = TempDir::new().unwrap();
        let store = AdvanceStore::new(ProductAppPaths::new(root.path().join(".aria")));
        (root, store)
    }

    fn cascade_record(
        id: &str,
        command_id: &str,
        plan_id: &str,
        attempt_id: Option<&str>,
        status: AdvanceStatus,
    ) -> AdvanceRecord {
        AdvanceRecord {
            id: id.into(),
            command_id: command_id.into(),
            project_id: "project_0001".into(),
            issue_id: "issue_0001".into(),
            plan_id: plan_id.into(),
            plan_revision_id: "revision_0001".into(),
            attempt_id: attempt_id.map(Into::into),
            start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
            target_attempts: Vec::new(),
            status,
            workspace_entry: attempt_id.map(|attempt| format!("/workspaces/{attempt}")),
            error: None,
            created_at: "2026-10-13T00:00:00Z".into(),
            updated_at: "2026-10-13T00:00:01Z".into(),
        }
    }

    fn cascade_journal(
        advance_id: &str,
        plan_id: &str,
        attempt_id: &str,
    ) -> AdvanceInitializationJournal {
        AdvanceInitializationJournal {
            advance_id: advance_id.into(),
            plan_id: plan_id.into(),
            attempt_id: attempt_id.into(),
            target_attempt_ids: Vec::new(),
            phase: AdvanceInitializationPhase::JournalPrepared,
            error: None,
            created_at: "2026-10-13T00:00:00Z".into(),
            updated_at: "2026-10-13T00:00:01Z".into(),
        }
    }

    fn cascade_retry(command_id: &str, advance_id: &str) -> RetryInitializationRecord {
        RetryInitializationRecord {
            retry_id: format!("advance_retry_{command_id}"),
            command_id: command_id.into(),
            advance_id: advance_id.into(),
            attempt_id: "attempt_0001".into(),
            state: OperationState::Accepted,
            checkpoint: AdvanceInitializationPhase::JournalPrepared,
            created_at: "2026-10-13T00:00:00Z".into(),
            updated_at: "2026-10-13T00:00:01Z".into(),
        }
    }

    fn seed_retry(store: &AdvanceStore, command_id: &str, advance_id: &str) {
        write_json(
            &store
                .retry_path("project_0001", "issue_0001", command_id)
                .unwrap(),
            &cascade_retry(command_id, advance_id),
        )
        .unwrap();
    }

    #[test]
    fn cascade_d1_2c_advance_owner_purge_two_plans_accurate_attribution() {
        // REQ-GRP-01「Plan 所属 advance 整条内容随删除清除」：同 issue 两 Plan 的
        // record/journal/retry 按真实 plan_id/advance_id 归属，只清目标 Plan，
        // 不删他 Plan 对象、不整删 advance-records/advance-retries 根。
        let (_root, store) = cascade_store();
        let (p, i) = ("project_0001", "issue_0001");

        let first = cascade_record(
            "advance_command_0001",
            "command_0001",
            "plan_0001",
            Some("attempt_0001"),
            AdvanceStatus::Ready,
        );
        store.put_record(&first).unwrap();
        // 存量同 plan 多 record（legacy）：put_record 的 plan 唯一性检查会拒，
        // 直接落盘模拟历史数据——list 必须全 issue 枚举而非仅首条。
        let legacy = cascade_record(
            "advance_command_0002",
            "command_0002",
            "plan_0001",
            None,
            AdvanceStatus::Aborted,
        );
        write_json(
            &store.path_for(p, i, "advance_command_0002").unwrap(),
            &legacy,
        )
        .unwrap();
        let other = cascade_record(
            "advance_command_0003",
            "command_0003",
            "plan_0002",
            Some("attempt_0002"),
            AdvanceStatus::Ready,
        );
        store.put_record(&other).unwrap();
        store
            .save_advance_initialization(
                &first,
                &cascade_journal("advance_command_0001", "plan_0001", "attempt_0001"),
            )
            .unwrap();
        store
            .save_advance_initialization(
                &other,
                &cascade_journal("advance_command_0003", "plan_0002", "attempt_0002"),
            )
            .unwrap();
        seed_retry(&store, "retry_cmd_0001", "advance_command_0001");
        seed_retry(&store, "retry_cmd_0002", "advance_command_0003");

        let target = store.list_plan_artifacts(p, i, "plan_0001").unwrap();
        assert_eq!(
            target.advance_ids,
            vec!["advance_command_0001", "advance_command_0002"]
        );
        assert_eq!(target.retry_command_ids, vec!["retry_cmd_0001"]);
        let bystander = store.list_plan_artifacts(p, i, "plan_0002").unwrap();
        assert_eq!(bystander.advance_ids, vec!["advance_command_0003"]);
        assert_eq!(bystander.retry_command_ids, vec!["retry_cmd_0002"]);

        let scope = EntityScope::new(p, i, DeletionEntityKind::Plan, "plan_0001");
        let operations = EntityOperationStore::new(store.app_paths());
        let deletions = EntityDeletionStore::new(store.app_paths());
        let lease = operations.try_acquire_delete(&scope).unwrap();
        assert_eq!(
            deletions.begin(&lease).unwrap().phase,
            DeletionPhase::Purging
        );
        // purging 阶段仍可枚举（§0.5：purging/deleted scope 允许 list 查询）。
        assert_eq!(
            store.list_plan_artifacts(p, i, "plan_0001").unwrap(),
            target
        );
        // pending manifest 捕获 advance/retry 纯 ID 集（重入事实）。
        let mut manifest = EntityDeletionManifest::empty(scope.clone());
        manifest.advance_ids = target.advance_ids.clone();
        manifest.advance_retry_command_ids = target.retry_command_ids.clone();
        deletions.save_pending_manifest(&lease, &manifest).unwrap();
        assert_eq!(
            deletions
                .load_pending_manifest(&lease)
                .unwrap()
                .unwrap()
                .advance_ids,
            vec!["advance_command_0001", "advance_command_0002"]
        );

        // 清理序 retry→journal→record；missing 幂等（同 lease 二次 purge）。
        store.purge_plan_artifacts(&lease, &target).unwrap();
        store.purge_plan_artifacts(&lease, &target).unwrap();

        assert!(
            store
                .get_advance_for_plan(p, i, "plan_0001")
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .get_advance_by_command_id(p, i, "command_0001")
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .get_advance_by_command_id(p, i, "command_0002")
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .get_retry_initialization(p, i, "retry_cmd_0001")
                .unwrap()
                .is_none()
        );
        assert!(
            !store
                .root(p, i)
                .unwrap()
                .join("plan_0001.initialization.json")
                .is_file()
        );
        // 他 Plan 原样保留。
        assert_eq!(
            store
                .get_advance_for_plan(p, i, "plan_0002")
                .unwrap()
                .unwrap()
                .id,
            "advance_command_0003"
        );
        assert!(
            store
                .get_retry_initialization(p, i, "retry_cmd_0002")
                .unwrap()
                .is_some()
        );
        assert!(
            store
                .root(p, i)
                .unwrap()
                .join("plan_0002.initialization.json")
                .is_file()
        );
        // 不整删 advance-records/advance-retries 根。
        assert!(store.root(p, i).unwrap().is_dir());
        assert!(store.retries_root(p, i).unwrap().is_dir());

        // 真实协调序：清理成功后先移除 pending manifest 再 complete（D1.0 契约）。
        deletions.remove_pending_manifest(&lease).unwrap();
        assert_eq!(
            deletions.complete(&lease).unwrap().phase,
            DeletionPhase::Deleted
        );
        // Ready 准入：删除事实优先；未删 Plan 原精确 join 不变。
        assert!(
            !store
                .advance_is_ready_for_attempt(p, i, "plan_0001", "attempt_0001")
                .unwrap()
        );
        assert!(
            store
                .advance_is_ready_for_attempt(p, i, "plan_0002", "attempt_0002")
                .unwrap()
        );
    }

    #[test]
    fn cascade_d1_2c_deleted_plan_rejects_replay_ready_and_late_writes() {
        // REQ-GRP-01「删除终止 advance 重放与 Ready 放行」：已删除身份的同/异
        // command 重放、retry 与迟到 owner 写依据删除事实优先拒绝，不建目录；
        // 旧 Ready 残留查询 false；未删 Plan 原幂等合同由既有测试守护。
        let (_root, store) = cascade_store();
        let (p, i) = ("project_0001", "issue_0001");
        let record = cascade_record(
            "advance_command_0001",
            "command_0001",
            "plan_0001",
            Some("attempt_0001"),
            AdvanceStatus::Ready,
        );
        store.put_record(&record).unwrap();
        let journal = cascade_journal("advance_command_0001", "plan_0001", "attempt_0001");
        store
            .save_advance_initialization(&record, &journal)
            .unwrap();
        seed_retry(&store, "retry_cmd_0001", "advance_command_0001");

        let scope = EntityScope::new(p, i, DeletionEntityKind::Plan, "plan_0001");
        let operations = EntityOperationStore::new(store.app_paths());
        let deletions = EntityDeletionStore::new(store.app_paths());
        let lease = operations.try_acquire_delete(&scope).unwrap();
        deletions.begin(&lease).unwrap();
        let artifacts = store.list_plan_artifacts(p, i, "plan_0001").unwrap();
        store.purge_plan_artifacts(&lease, &artifacts).unwrap();
        assert_eq!(
            deletions.complete(&lease).unwrap().phase,
            DeletionPhase::Deleted
        );
        drop(lease);

        // 模拟完成后全局清理：advance 两根整体移除（根/摘要均缺）。
        std::fs::remove_dir_all(store.root(p, i).unwrap()).unwrap();
        std::fs::remove_dir_all(store.retries_root(p, i).unwrap()).unwrap();

        fn assert_deleting<T>(result: Result<T, ProductStoreError>) {
            assert!(matches!(
                result,
                Err(ProductStoreError::Conflict { kind, .. }) if kind == "entity_deleting"
            ));
        }
        // 同 command_id 重放 → 拒绝（不复活旧 record）。
        let replay = AdvanceInput {
            command_id: "command_0001".into(),
            project_id: p.into(),
            issue_id: i.into(),
            plan_id: "plan_0001".into(),
        };
        assert_deleting(store.persist_advance_record_if_absent(
            &replay,
            "revision_0001",
            crate::product::coding_models::CodingStartRunPolicy::Manual,
        ));
        // 不同 command_id 新 advance → 同样拒绝。
        let replay_fresh = AdvanceInput {
            command_id: "command_0009".into(),
            ..replay.clone()
        };
        assert_deleting(store.persist_advance_record_if_absent(
            &replay_fresh,
            "revision_0001",
            crate::product::coding_models::CodingStartRunPolicy::Manual,
        ));
        // 迟到 put/update/journal/reopen 全部拒绝且不重建目录。
        let late = cascade_record(
            "advance_command_0009",
            "command_0009",
            "plan_0001",
            None,
            AdvanceStatus::Initializing,
        );
        assert_deleting(store.put_record(&late));
        let mut updated = record.clone();
        updated.status = AdvanceStatus::Running;
        assert_deleting(store.update_record(&updated));
        assert_deleting(store.save_advance_initialization(&record, &journal));
        assert_deleting(store.load_or_prepare_advance_initialization_for_attempts(
            &record,
            "attempt_0001",
            &[],
        ));
        assert_deleting(store.advance_initialization_phase(
            &record,
            &journal,
            AdvanceInitializationPhase::AttemptPersisted,
        ));
        assert_deleting(store.reopen_failed_record_for_retry(&record));
        // retry 更新经 advance_id 反查 plan：record 已不存在 → fail-closed 拒绝。
        assert!(
            store
                .update_retry_initialization_state(
                    p,
                    i,
                    &cascade_retry("retry_cmd_0001", "advance_command_0001"),
                    OperationState::NeedsHuman,
                )
                .is_err()
        );
        assert!(!store.root(p, i).unwrap().is_dir());
        assert!(!store.retries_root(p, i).unwrap().is_dir());

        // 旧 Ready 残留查询：删除事实优先于残留 record 内容。
        write_json(
            &store.path_for(p, i, "advance_command_0001").unwrap(),
            &record,
        )
        .unwrap();
        assert!(
            !store
                .advance_is_ready_for_attempt(p, i, "plan_0001", "attempt_0001")
                .unwrap()
        );
    }

    #[test]
    fn cascade_d1_2c_list_reports_orphan_journal_without_plan_root() {
        // §0.5 list-by-owner：孤儿 initialization journal 按 {plan_id} 文件名归属
        // 仍可枚举（不依赖 Plan 根/pending），并按真实归属清掉；根缺失=已清。
        let (_root, store) = cascade_store();
        let (p, i) = ("project_0001", "issue_0001");
        let other = cascade_record(
            "advance_command_0003",
            "command_0003",
            "plan_0002",
            Some("attempt_0002"),
            AdvanceStatus::Ready,
        );
        store.put_record(&other).unwrap();
        // plan_0001 无 record，仅有孤儿 journal（模拟清理中断残留）。
        let journal_path = store
            .root(p, i)
            .unwrap()
            .join("plan_0001.initialization.json");
        write_json(
            &journal_path,
            &cascade_journal("advance_command_0001", "plan_0001", "attempt_0001"),
        )
        .unwrap();

        let artifacts = store.list_plan_artifacts(p, i, "plan_0001").unwrap();
        assert_eq!(artifacts.advance_ids, vec!["advance_command_0001"]);
        assert!(artifacts.retry_command_ids.is_empty());

        let scope = EntityScope::new(p, i, DeletionEntityKind::Plan, "plan_0001");
        let operations = EntityOperationStore::new(store.app_paths());
        let deletions = EntityDeletionStore::new(store.app_paths());
        let lease = operations.try_acquire_delete(&scope).unwrap();
        deletions.begin(&lease).unwrap();
        store.purge_plan_artifacts(&lease, &artifacts).unwrap();
        assert!(!journal_path.is_file());
        // 根与他 Plan 对象不动。
        assert!(store.root(p, i).unwrap().is_dir());
        assert!(
            store
                .get_advance_for_plan(p, i, "plan_0002")
                .unwrap()
                .is_some()
        );
        deletions.complete(&lease).unwrap();
        assert!(
            store
                .list_plan_artifacts(p, i, "plan_0001")
                .unwrap()
                .advance_ids
                .is_empty()
        );
        // 根整体缺失=已清：空集、不报错、不建目录。
        std::fs::remove_dir_all(store.root(p, i).unwrap()).unwrap();
        assert_eq!(
            store.list_plan_artifacts(p, i, "plan_0001").unwrap(),
            AdvancePlanArtifacts::default()
        );
        assert!(!store.root(p, i).unwrap().is_dir());
    }

    #[test]
    fn cascade_d1_2c_list_fails_closed_on_corrupt_or_misjoined_artifacts() {
        // 坏 JSON/错 join 一律报错，不当空集（fail-closed）。
        let (p, i) = ("project_0001", "issue_0001");
        // a) record 坏 JSON。
        {
            let (_root, store) = cascade_store();
            let root = store.root(p, i).unwrap();
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(root.join("advance_broken.json"), b"{ not json").unwrap();
            assert!(store.list_plan_artifacts(p, i, "plan_0001").is_err());
        }
        // b) journal 内容 plan_id 与文件名键不符。
        {
            let (_root, store) = cascade_store();
            write_json(
                &store
                    .root(p, i)
                    .unwrap()
                    .join("plan_0001.initialization.json"),
                &cascade_journal("advance_command_0001", "plan_0002", "attempt_0001"),
            )
            .unwrap();
            assert!(matches!(
                store.list_plan_artifacts(p, i, "plan_0001"),
                Err(ProductStoreError::IdentityMismatch { .. })
            ));
        }
        // c) retry 文件名与内容 command_id 不符（错 join）。
        {
            let (_root, store) = cascade_store();
            write_json(
                &store
                    .retries_root(p, i)
                    .unwrap()
                    .join("retry_cmd_0001.json"),
                &cascade_retry("retry_cmd_other", "advance_command_0001"),
            )
            .unwrap();
            assert!(matches!(
                store.list_plan_artifacts(p, i, "plan_0001"),
                Err(ProductStoreError::IdentityMismatch { .. })
            ));
        }
        // d) journal.advance_id 指向他 Plan 现存 record（错 join）。
        {
            let (_root, store) = cascade_store();
            let other = cascade_record(
                "advance_command_0003",
                "command_0003",
                "plan_0002",
                Some("attempt_0002"),
                AdvanceStatus::Ready,
            );
            store.put_record(&other).unwrap();
            write_json(
                &store
                    .root(p, i)
                    .unwrap()
                    .join("plan_0001.initialization.json"),
                &cascade_journal("advance_command_0003", "plan_0001", "attempt_0001"),
            )
            .unwrap();
            assert!(matches!(
                store.list_plan_artifacts(p, i, "plan_0001"),
                Err(ProductStoreError::IdentityMismatch { .. })
            ));
        }
    }

    #[test]
    fn cascade_d1_2c_persist_window_deletion_never_yields_stale_ok_or_identity_reclass() {
        // P3 修复轮（REQ-GRP-01 删除身份优先）：「预检后/put 前恰 begin 删除」
        // 微秒窗口内，persist 不得把陈旧 record 当幂等 Ok 命中，也不得把
        // put_record 的 entity_deleting 改判为 advance_record_identity。
        persist_deletion_window(/* seed_ours= */ true);
        persist_deletion_window(/* seed_ours= */ false);
    }

    fn persist_deletion_window(seed_ours: bool) {
        use std::sync::{Arc, Barrier};
        use std::time::Duration;

        use crate::product::coding_attempt_store::locking::{
            ExclusiveFileLock, register_lock_attempt_hook,
        };
        use crate::product::entity_operation_store::EntityOperationError;

        let (_root, store) = cascade_store();
        let (p, i) = ("project_0001", "issue_0001");
        if seed_ours {
            let ours = cascade_record(
                "advance_command_0001",
                "command_0001",
                "plan_0001",
                Some("attempt_0001"),
                AdvanceStatus::Ready,
            );
            store.put_record(&ours).unwrap();
        } else {
            // 异 plan record 使根存在：重放读取根锁但不命中，put 走新建路径。
            let other = cascade_record(
                "advance_command_0003",
                "command_0003",
                "plan_0002",
                Some("attempt_0002"),
                AdvanceStatus::Ready,
            );
            store.put_record(&other).unwrap();
        }

        let root = store.root(p, i).unwrap();
        let held = ExclusiveFileLock::acquire(&root).expect("hold advance root lock");
        // 持锁后注册 hook：主线程自身的 acquire 不触发通知。
        let (_hook, attempts) = register_lock_attempt_hook(&root);

        let worker_store = store.clone();
        let input = AdvanceInput {
            command_id: "command_0001".into(),
            project_id: p.into(),
            issue_id: i.into(),
            plan_id: "plan_0001".into(),
        };
        let started = Arc::new(Barrier::new(2));
        let worker_started = Arc::clone(&started);
        let worker = std::thread::spawn(move || {
            worker_started.wait();
            worker_store.persist_advance_record_if_absent(
                &input,
                "revision_0001",
                crate::product::coding_models::CodingStartRunPolicy::Manual,
            )
        });
        started.wait();

        // worker 停在重放读的根锁获取上：预检已过、put 未至的注入窗口。
        attempts
            .recv_timeout(Duration::from_secs(5))
            .expect("persist worker must contend on the advance root lock");

        let scope = EntityScope::new(p, i, DeletionEntityKind::Plan, "plan_0001");
        let operations = EntityOperationStore::new(store.app_paths());
        let deletions = EntityDeletionStore::new(store.app_paths());
        let deletion_began = match operations.try_acquire_delete(&scope) {
            Ok(lease) => {
                assert_eq!(
                    deletions.begin(&lease).unwrap().phase,
                    DeletionPhase::Purging
                );
                drop(lease);
                true
            }
            Err(EntityOperationError::Busy) => false,
            Err(other) => panic!("unexpected delete admission: {other:?}"),
        };

        drop(held);
        let result = worker.join().expect("persist worker thread");

        if deletion_began {
            // 窗口内删除事实已立：不得陈旧 Ok（seed_ours=true 旧路径）、不得
            // 改判 identity（seed_ours=false 旧路径），必须上抛 entity_deleting。
            assert!(
                matches!(
                    result,
                    Err(ProductStoreError::Conflict { kind, .. }) if kind == "entity_deleting"
                ),
                "persist must surface entity_deleting after deletion began, got {result:?}"
            );
        } else {
            // 准入跨全流程持有：persist 活着完成、删除被推迟（无墓碑）。
            let record = result.expect("live persist must complete while holding admission");
            assert_eq!(record.plan_id, "plan_0001");
            assert!(deletions.get(&scope).unwrap().is_none());
        }
    }

    // ─── H2b（设计 A 节，整组删除与重启）：旧 Ready 指向已死全集识别 + 零成员清后新轮绑定 ───

    fn h2b_ready_record(
        id: &str,
        command_id: &str,
        plan_id: &str,
        targets: &[(&str, &str)],
    ) -> AdvanceRecord {
        AdvanceRecord {
            id: id.into(),
            command_id: command_id.into(),
            project_id: "project_0001".into(),
            issue_id: "issue_0001".into(),
            plan_id: plan_id.into(),
            plan_revision_id: "revision_0001".into(),
            attempt_id: targets.first().map(|(_, attempt)| (*attempt).into()),
            start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
            target_attempts: targets
                .iter()
                .map(|(target, attempt)| target_binding(target, attempt))
                .collect(),
            status: AdvanceStatus::Ready,
            workspace_entry: None,
            error: None,
            created_at: "2026-10-14T00:00:00Z".into(),
            updated_at: "2026-10-14T00:00:01Z".into(),
        }
    }

    #[test]
    fn h2b_incident_ready_plus_ready_journal_all_dead_is_all_targets_dead() {
        // 事故形态：旧 Ready record 与外层 journal（Ready 相位）指向同一四个
        // 已删 attempts，现存集为空 → AllTargetsDead，交零成员闭包完整重置。
        let (_root, store) = cascade_store();
        let (p, i) = ("project_0001", "issue_0001");
        let dead = [
            ("repo_a", "attempt_0001"),
            ("repo_b", "attempt_0002"),
            ("repo_c", "attempt_0003"),
            ("repo_d", "attempt_0004"),
        ];
        let record = h2b_ready_record("advance_command_0001", "command_0001", "plan_0001", &dead);
        store.put_record(&record).unwrap();
        let mut journal = cascade_journal("advance_command_0001", "plan_0001", "attempt_0001");
        journal.target_attempt_ids = dead
            .iter()
            .map(|(_, attempt)| (*attempt).to_string())
            .collect();
        journal.phase = AdvanceInitializationPhase::Ready;
        store
            .save_advance_initialization(&record, &journal)
            .unwrap();

        let facts = store.plan_execution_references(p, i, "plan_0001").unwrap();
        assert_eq!(facts.records, vec![record.clone()]);
        assert_eq!(facts.initialization_journal.as_ref(), Some(&journal));
        assert_eq!(
            facts.ready_bound_attempt_ids(),
            vec![
                "attempt_0001".to_string(),
                "attempt_0002".to_string(),
                "attempt_0003".to_string(),
                "attempt_0004".to_string(),
            ]
        );
        assert_eq!(
            facts.journal_bound_attempt_ids(),
            vec![
                "attempt_0001".to_string(),
                "attempt_0002".to_string(),
                "attempt_0003".to_string(),
                "attempt_0004".to_string(),
            ]
        );
        assert_eq!(
            facts.classify_stale_references(&[]),
            AdvanceStaleReferenceVerdict::AllTargetsDead {
                dead_attempt_ids: vec![
                    "attempt_0001".into(),
                    "attempt_0002".into(),
                    "attempt_0003".into(),
                    "attempt_0004".into(),
                ],
            }
        );
    }

    #[test]
    fn h2b_partially_dead_members_keep_partial_verdict() {
        // 删 2/4：PartiallyDead——不缩水全集、不自动清，走清剩余出口。
        let (_root, store) = cascade_store();
        let (p, i) = ("project_0001", "issue_0001");
        let targets = [
            ("repo_a", "attempt_0001"),
            ("repo_b", "attempt_0002"),
            ("repo_c", "attempt_0003"),
            ("repo_d", "attempt_0004"),
        ];
        let record = h2b_ready_record(
            "advance_command_0001",
            "command_0001",
            "plan_0001",
            &targets,
        );
        store.put_record(&record).unwrap();

        let facts = store.plan_execution_references(p, i, "plan_0001").unwrap();
        assert_eq!(
            facts.classify_stale_references(&["attempt_0001".into(), "attempt_0002".into()]),
            AdvanceStaleReferenceVerdict::PartiallyDead {
                dead_attempt_ids: vec!["attempt_0003".into(), "attempt_0004".into()],
                alive_attempt_ids: vec!["attempt_0001".into(), "attempt_0002".into()],
            }
        );
    }

    #[test]
    fn h2b_prefix_journal_below_attempt_persisted_is_not_a_broken_reference() {
        // 设计 A 节 line 74：未越 AttemptPersisted 的 journal 绑定是初始化前缀
        //（attempt 尚未物化属预期缺失）——Initializing record + Prepared
        // journal 指向缺失 attempts 不判断裂/重置，Intact 续原协议。
        let (_root, store) = cascade_store();
        let (p, i) = ("project_0001", "issue_0001");
        let mut record = h2b_ready_record("advance_command_0001", "command_0001", "plan_0001", &[]);
        record.status = AdvanceStatus::Initializing;
        store.put_record(&record).unwrap();
        let mut journal = cascade_journal("advance_command_0001", "plan_0001", "attempt_0001");
        journal.target_attempt_ids = vec!["attempt_0001".into(), "attempt_0002".into()];
        journal.phase = AdvanceInitializationPhase::JournalPrepared;
        store
            .save_advance_initialization(&record, &journal)
            .unwrap();

        let facts = store.plan_execution_references(p, i, "plan_0001").unwrap();
        assert_eq!(
            facts.classify_stale_references(&[]),
            AdvanceStaleReferenceVerdict::Intact
        );
    }

    #[test]
    fn h2b_journal_past_attempt_persisted_alone_qualifies_as_dead_full_set() {
        // 旧 Ready 缺位的中断变体：record 非 Ready，但外层 journal 已越
        // AttemptPersisted 且指向缺失 attempts → 同样构成断裂全集。
        let (_root, store) = cascade_store();
        let (p, i) = ("project_0001", "issue_0001");
        let mut record = h2b_ready_record(
            "advance_command_0001",
            "command_0001",
            "plan_0001",
            &[("repo_a", "attempt_0001")],
        );
        record.status = AdvanceStatus::Aborted;
        store.put_record(&record).unwrap();
        let mut journal = cascade_journal("advance_command_0001", "plan_0001", "attempt_0001");
        journal.target_attempt_ids = vec!["attempt_0001".into(), "attempt_0002".into()];
        journal.phase = AdvanceInitializationPhase::AttemptPersisted;
        store
            .save_advance_initialization(&record, &journal)
            .unwrap();

        let facts = store.plan_execution_references(p, i, "plan_0001").unwrap();
        assert_eq!(
            facts.classify_stale_references(&[]),
            AdvanceStaleReferenceVerdict::AllTargetsDead {
                dead_attempt_ids: vec!["attempt_0001".into(), "attempt_0002".into()],
            }
        );
    }

    #[test]
    fn h2b_verdicts_for_alive_references_and_empty_plans() {
        // 全存活 → Intact；无 record 无 journal → NoReferences；仅非 Ready
        // records → Intact（在途初始化，不判重置）。
        let (_root, store) = cascade_store();
        let (p, i) = ("project_0001", "issue_0001");
        assert_eq!(
            store
                .plan_execution_references(p, i, "plan_0001")
                .unwrap()
                .classify_stale_references(&[]),
            AdvanceStaleReferenceVerdict::NoReferences
        );

        let mut initializing =
            h2b_ready_record("advance_command_0001", "command_0001", "plan_0001", &[]);
        initializing.status = AdvanceStatus::Initializing;
        store.put_record(&initializing).unwrap();
        assert_eq!(
            store
                .plan_execution_references(p, i, "plan_0001")
                .unwrap()
                .classify_stale_references(&[]),
            AdvanceStaleReferenceVerdict::Intact
        );

        let ready = h2b_ready_record(
            "advance_command_0002",
            "command_0002",
            "plan_0002",
            &[("repo_a", "attempt_0001")],
        );
        store.put_record(&ready).unwrap();
        assert_eq!(
            store
                .plan_execution_references(p, i, "plan_0002")
                .unwrap()
                .classify_stale_references(&["attempt_0001".into()]),
            AdvanceStaleReferenceVerdict::Intact
        );
    }

    #[test]
    fn h2b_zero_member_purge_then_new_round_binds_new_command() {
        // 零成员闭包 advance 半边（复用 D1.2c 闭包，不立墓碑）：list → Plan
        // 独占 lease → purge（retry→journal→record，幂等）→ 新 command 正常
        // 绑定新轮新 attempts——「正常 advance 绑定新轮」，旧 ID 不复活。
        let (_root, store) = cascade_store();
        let (p, i) = ("project_0001", "issue_0001");
        let stale = h2b_ready_record(
            "advance_command_0001",
            "command_0001",
            "plan_0001",
            &[("repo_a", "attempt_0001"), ("repo_b", "attempt_0002")],
        );
        store.put_record(&stale).unwrap();
        let mut journal = cascade_journal("advance_command_0001", "plan_0001", "attempt_0001");
        journal.target_attempt_ids = vec!["attempt_0001".into(), "attempt_0002".into()];
        journal.phase = AdvanceInitializationPhase::Ready;
        store.save_advance_initialization(&stale, &journal).unwrap();
        seed_retry(&store, "retry_cmd_0001", "advance_command_0001");

        let artifacts = store.list_plan_artifacts(p, i, "plan_0001").unwrap();
        assert_eq!(artifacts.advance_ids, vec!["advance_command_0001"]);
        assert_eq!(artifacts.retry_command_ids, vec!["retry_cmd_0001"]);
        let scope = EntityScope::new(p, i, DeletionEntityKind::Plan, "plan_0001");
        let operations = EntityOperationStore::new(store.app_paths());
        let lease = operations.try_acquire_delete(&scope).unwrap();
        // Coding 重置不调用 begin/complete——不给保留的 Plan 立永久墓碑
        //（设计 A 节注意），租约本身即安全清理窗口。
        store.purge_plan_artifacts(&lease, &artifacts).unwrap();
        store.purge_plan_artifacts(&lease, &artifacts).unwrap();
        drop(lease);

        assert!(
            store
                .get_advance_for_plan(p, i, "plan_0001")
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .get_retry_initialization(p, i, "retry_cmd_0001")
                .unwrap()
                .is_none()
        );
        assert!(
            !store
                .root(p, i)
                .unwrap()
                .join("plan_0001.initialization.json")
                .is_file()
        );

        // 新轮身份：新 command/新 attempt IDs，正常 Ready 放行。
        let fresh = h2b_ready_record(
            "advance_command_0002",
            "command_0002",
            "plan_0001",
            &[("repo_a", "attempt_1001"), ("repo_b", "attempt_1002")],
        );
        assert_eq!(store.put_record(&fresh).unwrap(), fresh);
        assert!(
            store
                .advance_is_ready_for_attempt(p, i, "plan_0001", "attempt_1001")
                .unwrap()
        );
        assert!(
            !store
                .advance_is_ready_for_attempt(p, i, "plan_0001", "attempt_0001")
                .unwrap()
        );
    }
}
