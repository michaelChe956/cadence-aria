//! REQ-MTG-05（WP5 增殖审计）：按 target 分流的 target-attempt 创建 durable 审计面。
//!
//! 只记事实、不承载调度（spec「审计不承载编排」scenario）：本模块仅提供幂等
//! 写入与 per-`(plan,target)` 检索，无任何状态转换/启动/排队/依赖判定调用；
//! 恢复/重放/二期（跨 attempt 依赖门）立项证据只读消费。
//!
//! 落点 `issue_lifecycle_root/{project}/{issue}/split-audit/{attempt_id}.json`
//! （json_store durable 先例，与 `coding-attempts` 目录族同根）；一 target-attempt
//! 一条（id=attempt_id ⇒ per-`(plan,target)` 唯一⇒审计唯一）。

use serde::{Deserialize, Serialize};

use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};

/// 分流触发入口：advance 命令键（`command_id` 从 `AdvanceInput` 透传）或建组
/// 入口。一期接线仅 advance 面（engine 分流循环）；`group_creation` 为记录
/// 词汇表的既有入口形态（REQ-MTG-05「触发入口」定案）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitAuditTriggerKind {
    Advance,
    GroupCreation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SplitAuditTrigger {
    pub kind: SplitAuditTriggerKind,
    pub command_id: Option<String>,
}

/// 增殖审计记录：plan/target/attempt 身份+分流依据（authoritative binding
/// revision 双键）+触发入口+创建时间。首写定档：身份一致命中不重写
/// （`created_at` 与首写 `trigger.command_id` 保留——恢复/重放不漂移）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SplitAuditRecord {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub target_repository_id: String,
    pub attempt_id: String,
    pub bound_plan_revision_id: String,
    pub dependency_graph_revision_id: String,
    pub trigger: SplitAuditTrigger,
    pub created_at: String,
}

impl super::CodingAttemptStore {
    fn split_audit_path(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<std::path::PathBuf, ProductStoreError> {
        for id in [project_id, issue_id, attempt_id] {
            validate_relative_id(id)?;
        }
        Ok(self
            .paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("split-audit")
            .join(format!("{attempt_id}.json")))
    }

    /// 身份一致性（幂等命中判据）：分流事实字段全等。`trigger.command_id` 与
    /// `created_at` 不参与——恢复/重放可能携带异 command_id（首写记录是权威）。
    fn split_audit_identity_matches(
        existing: &SplitAuditRecord,
        record: &SplitAuditRecord,
    ) -> bool {
        existing.id == record.id
            && existing.project_id == record.project_id
            && existing.issue_id == record.issue_id
            && existing.plan_id == record.plan_id
            && existing.target_repository_id == record.target_repository_id
            && existing.attempt_id == record.attempt_id
            && existing.bound_plan_revision_id == record.bound_plan_revision_id
            && existing.dependency_graph_revision_id == record.dependency_graph_revision_id
            && existing.trigger.kind == record.trigger.kind
    }

    /// 幂等落审计：文件在场且身份一致 → Ok（不重写）；身份冲突 → fail-closed。
    pub fn record_split_audit(&self, record: &SplitAuditRecord) -> Result<(), ProductStoreError> {
        if record.id != record.attempt_id {
            return Err(ProductStoreError::InvalidRecord {
                kind: "split_audit_record",
                reason: "id must equal attempt_id".to_string(),
            });
        }
        if record.created_at.trim().is_empty() {
            return Err(ProductStoreError::InvalidRecord {
                kind: "split_audit_record",
                reason: "created_at must be present".to_string(),
            });
        }
        let path =
            self.split_audit_path(&record.project_id, &record.issue_id, &record.attempt_id)?;
        if super::path_is_regular_file(&path)? {
            let existing: SplitAuditRecord = read_json(&path)?;
            if Self::split_audit_identity_matches(&existing, record) {
                return Ok(());
            }
            return Err(ProductStoreError::Conflict {
                kind: "split_audit_record",
                id: record.attempt_id.clone(),
            });
        }
        write_json(&path, record)
    }

    /// per-`(plan,target)` 检索（消解增殖后「取最早」歧义，REQ-MTG-05）：
    /// plan 内全部 target-attempt 审计，按 target 升序确定性返回；同 issue 下
    /// 异 plan 记录过滤不串扰；身份漂移 fail-closed。
    pub fn get_split_audits_for_plan(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<Vec<SplitAuditRecord>, ProductStoreError> {
        for id in [project_id, issue_id, plan_id] {
            validate_relative_id(id)?;
        }
        let root = self
            .paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("split-audit");
        let mut records: Vec<SplitAuditRecord> =
            super::list_json_records::<SplitAuditRecord>(&root)?
                .into_iter()
                .filter(|record| record.plan_id == plan_id)
                .collect();
        for record in &records {
            if record.project_id != project_id
                || record.issue_id != issue_id
                || record.id != record.attempt_id
            {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "split_audit_record",
                    id: plan_id.to_string(),
                });
            }
        }
        records.sort_by(|left, right| {
            left.target_repository_id
                .cmp(&right.target_repository_id)
                .then_with(|| left.attempt_id.cmp(&right.attempt_id))
        });
        Ok(records)
    }
}
