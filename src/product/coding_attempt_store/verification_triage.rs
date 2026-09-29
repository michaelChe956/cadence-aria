//! C2 Task 8（REQ-CVT-03/04，#19／BYPASS-19）：attempt-scoped 验证处理记录。
//!
//! 独立验证处理持久面（目录结构照 quality-bypass-audits：attempt 目录下
//! 一记录一 JSON）：从 coder 输出门与 Code Review 验证不完整三门转入时
//! 创建唯一绑定记录；三类结论均需用户明确批准；finding 只标注不清空；
//! 审计（操作者／时间／理由）随记录落账。不硬塞 `QualityGateBypassAudit`，
//! 不进 `coding_gate_action_for_id` 动作枚举，不关闭或改写原门与 finding。

use std::path::PathBuf;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::product::coding_models::CodingExecutionAttempt;
use crate::product::id::next_sequential_id_in_directory;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};

use super::locking::with_exclusive_lock;

/// 验证处理状态：未决／已批准／已拒绝。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationTriageStatus {
    Pending,
    Approved,
    Rejected,
}

/// 三类结论（均需用户明确批准）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationTriageConclusion {
    /// 经既有 prepare_amendment／apply_plan_amendment 链发起计划修订。
    ApprovePlanRevision,
    /// 接受等价证据：需替代命令＋cwd＋结果，check 要求非零测试时另需
    /// 非零测试执行数量。
    AcceptEquivalentEvidence,
    /// 限域环境例外：只覆盖绑定 check 与 scope，记录理由／操作者／时间。
    GrantScopedEnvironmentException,
}

/// attempt-scoped 验证处理记录（唯一新增契约）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationTriageRecord {
    pub triage_id: String,
    pub attempt_id: String,
    pub finding_id: String,
    pub check_id: String,
    pub plan_revision_id: String,
    pub original_command: Option<String>,
    pub alternative_command: Option<String>,
    pub cwd: Option<String>,
    pub outcome: Option<String>,
    pub test_execution_count: Option<u64>,
    pub environment: Option<String>,
    pub scope: Vec<String>,
    pub expires_at: String,
    pub status: VerificationTriageStatus,
    pub conclusion: Option<VerificationTriageConclusion>,
    pub reason: Option<String>,
    pub decided_by: Option<String>,
    pub decided_at: Option<String>,
}

/// store 级转入输入：plan_revision／scope／expiry 由 engine 应用服务按
/// attempt 权威绑定派生后传入（store 不信任调用方自报的权威字段来源，
/// 但持久化以传入为准，供测试直接落账构造过期等场景）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnterVerificationTriageInput {
    pub attempt_id: String,
    pub finding_id: String,
    pub check_id: String,
    pub plan_revision_id: String,
    pub original_command: Option<String>,
    pub alternative_command: Option<String>,
    pub cwd: Option<String>,
    pub outcome: Option<String>,
    pub test_execution_count: Option<u64>,
    pub environment: Option<String>,
    pub scope: Vec<String>,
    pub expires_at: String,
}

/// 决定：批准（三类结论）或拒绝该验证处理记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationTriageDecision {
    Approve {
        conclusion: VerificationTriageConclusion,
        decided_by: String,
        reason: String,
    },
    Reject {
        decided_by: String,
        reason: String,
    },
}

impl super::CodingAttemptStore {
    fn verification_triage_root(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> PathBuf {
        self.attempt_dir(project_id, issue_id, attempt_id)
            .join("verification-triage")
    }

    /// 转入验证处理：同一 finding/check/plan revision 已有未决记录时返回
    /// 既有记录（不创建第二条）；已决记录不参与去重（拒绝后可重新转入）。
    pub fn enter_verification_triage(
        &self,
        attempt: &CodingExecutionAttempt,
        input: EnterVerificationTriageInput,
    ) -> Result<VerificationTriageRecord, ProductStoreError> {
        validate_relative_id(&input.attempt_id)?;
        validate_relative_id(&input.finding_id)?;
        validate_relative_id(&input.check_id)?;
        validate_relative_id(&input.plan_revision_id)?;
        self.validate_scoped_attempt_record(
            attempt,
            &input.attempt_id,
            "coding_verification_triage",
            &input.attempt_id,
        )?;
        if input.finding_id.trim().is_empty() || input.check_id.trim().is_empty() {
            return Err(ProductStoreError::InvalidRecord {
                kind: "coding_verification_triage",
                reason: "finding_and_check_ids_required".to_string(),
            });
        }
        let root =
            self.verification_triage_root(&attempt.project_id, &attempt.issue_id, &attempt.id);
        let lock_target = root.join("verification-triage-mutations");
        with_exclusive_lock(&lock_target, || {
            if let Some(existing) = list_verification_triage_records_locked(&root)?.into_iter().find(
                |record| {
                    record.status == VerificationTriageStatus::Pending
                        && record.finding_id == input.finding_id
                        && record.check_id == input.check_id
                        && record.plan_revision_id == input.plan_revision_id
                },
            ) {
                return Ok(existing);
            }
            std::fs::create_dir_all(&root).map_err(|error| {
                ProductStoreError::Io(format!("create {}: {error}", root.display()))
            })?;
            let id = next_sequential_id_in_directory("verification_triage", &root).map_err(
                |error| ProductStoreError::Io(format!("read {}: {error}", root.display())),
            )?;
            let record = VerificationTriageRecord {
                triage_id: id,
                attempt_id: attempt.id.clone(),
                finding_id: input.finding_id,
                check_id: input.check_id,
                plan_revision_id: input.plan_revision_id,
                original_command: input.original_command,
                alternative_command: input.alternative_command,
                cwd: input.cwd,
                outcome: input.outcome,
                test_execution_count: input.test_execution_count,
                environment: input.environment,
                scope: input.scope,
                expires_at: input.expires_at,
                status: VerificationTriageStatus::Pending,
                conclusion: None,
                reason: None,
                decided_by: None,
                decided_at: None,
            };
            write_json(&root.join(format!("{}.json", record.triage_id)), &record)?;
            Ok(record)
        })
    }

    pub fn list_verification_triage_records(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<VerificationTriageRecord>, ProductStoreError> {
        list_verification_triage_records_locked(
            &self.verification_triage_root(project_id, issue_id, attempt_id),
        )
    }

    pub fn get_verification_triage_record(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        triage_id: &str,
    ) -> Result<VerificationTriageRecord, ProductStoreError> {
        validate_relative_id(triage_id)?;
        let path = self
            .verification_triage_root(project_id, issue_id, attempt_id)
            .join(format!("{triage_id}.json"));
        if !path.is_file() {
            return Err(ProductStoreError::NotFound {
                kind: "coding_verification_triage",
                id: triage_id.to_string(),
            });
        }
        Ok(read_json(&path)?)
    }

    /// 落决定（审计字段随记录落账）：未决记录写入结论；同结论同操作者同
    /// 理由的重放幂等返回首次结果；已决异义 fail-closed。
    pub fn apply_verification_triage_decision(
        &self,
        attempt: &CodingExecutionAttempt,
        triage_id: &str,
        decision: &VerificationTriageDecision,
    ) -> Result<VerificationTriageRecord, ProductStoreError> {
        validate_relative_id(triage_id)?;
        self.validate_scoped_attempt_record(
            attempt,
            &attempt.id,
            "coding_verification_triage",
            triage_id,
        )?;
        let root =
            self.verification_triage_root(&attempt.project_id, &attempt.issue_id, &attempt.id);
        let lock_target = root.join("verification-triage-mutations");
        with_exclusive_lock(&lock_target, || {
            let path = root.join(format!("{triage_id}.json"));
            if !path.is_file() {
                return Err(ProductStoreError::NotFound {
                    kind: "coding_verification_triage",
                    id: triage_id.to_string(),
                });
            }
            let mut record: VerificationTriageRecord = read_json(&path)?;
            if record.attempt_id != attempt.id {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "coding_verification_triage",
                    id: triage_id.to_string(),
                });
            }
            let (status, conclusion, decided_by, reason) = match decision {
                VerificationTriageDecision::Approve {
                    conclusion,
                    decided_by,
                    reason,
                } => (
                    VerificationTriageStatus::Approved,
                    Some(*conclusion),
                    decided_by.clone(),
                    reason.clone(),
                ),
                VerificationTriageDecision::Reject { decided_by, reason } => {
                    (VerificationTriageStatus::Rejected, None, decided_by.clone(), reason.clone())
                }
            };
            if record.status != VerificationTriageStatus::Pending {
                if record.status == status
                    && record.conclusion == conclusion
                    && record.decided_by.as_deref() == Some(decided_by.as_str())
                    && record.reason.as_deref() == Some(reason.as_str())
                {
                    return Ok(record);
                }
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "coding_verification_triage_decision",
                    id: triage_id.to_string(),
                });
            }
            record.status = status;
            record.conclusion = conclusion;
            record.decided_by = Some(decided_by);
            record.reason = Some(reason);
            record.decided_at = Some(Utc::now().to_rfc3339());
            write_json(&path, &record)?;
            Ok(record)
        })
    }

    /// finding 标注：批准"等价证据"或"限域例外"后，原 finding 保留并标注
    /// "已由验证处理覆盖"（派生自记录，不改写 finding 本体）。计划修订结论
    /// 不标注（finding 走修订链处理）。
    pub fn verification_triage_annotation_for_finding(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        finding_id: &str,
    ) -> Result<Option<String>, ProductStoreError> {
        Ok(self
            .list_verification_triage_records(project_id, issue_id, attempt_id)?
            .into_iter()
            .find(|record| {
                record.finding_id == finding_id
                    && record.status == VerificationTriageStatus::Approved
                    && matches!(
                        record.conclusion,
                        Some(
                            VerificationTriageConclusion::AcceptEquivalentEvidence
                                | VerificationTriageConclusion::GrantScopedEnvironmentException
                        )
                    )
            })
            .map(|record| format!("已由验证处理覆盖（{}）", record.triage_id)))
    }
}

fn list_verification_triage_records_locked(
    root: &std::path::Path,
) -> Result<Vec<VerificationTriageRecord>, ProductStoreError> {
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut records = Vec::new();
    for path in super::json_file_paths(root)? {
        let record: VerificationTriageRecord = read_json(&path)?;
        records.push(record);
    }
    records.sort_by(|left, right| left.triage_id.cmp(&right.triage_id));
    Ok(records)
}
