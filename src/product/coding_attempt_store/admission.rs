use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(test)]
use std::collections::HashSet;
#[cfg(test)]
use std::sync::{Mutex, OnceLock};

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_models::{CodingAttemptStatus, CodingExecutionAttempt};
use crate::product::json_store::{ProductStoreError, read_json, write_json};
use crate::product::logical_codebase::snapshot_validator::validate_snapshot_fields;
use crate::product::logical_codebase::{AggregatePolicyArtifactStore, RepositoryRouting};

use super::CodingAttemptStore;
use super::locking::with_exclusive_lock;

/// 可持久化的 admission 票据。它绑定一次批准时的 attempt 版本和快照摘要，且只能消费一次。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdmissionTicketRecord {
    pub attempt_id: String,
    pub attempt_version: u64,
    pub snapshot_digest: String,
    pub approved_at: String,
    pub expires_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consumed_at: Option<String>,
}

const ADMISSION_TICKET_TTL: Duration = Duration::minutes(5);

pub(crate) const TARGET_SNAPSHOT_MISSING_FOR_LOGICAL: &str = "target_snapshot_missing_for_logical";
pub(crate) const TARGET_SNAPSHOT_IDENTITY_DRIFTED: &str = "target_snapshot_identity_drifted";
pub(crate) const MIXED_TARGET_GROUP_REJECTED: &str = "mixed_target_group_rejected";
const TARGET_SNAPSHOT_POLICY_DRIFTED: &str = "target_snapshot_policy_drifted";
const ADMISSION_TICKET_INVALID: &str = "admission_ticket_invalid";
const ADMISSION_TICKET_EXPIRED: &str = "admission_ticket_expired";
const ADMISSION_TICKET_CONSUMED: &str = "admission_ticket_consumed";
const ATTEMPT_AWAITING_MANUAL_RECOVERY: &str = "attempt_awaiting_manual_recovery";

/// Admission API failure code. The public boundary uses a stable string while its variants keep
/// internal failures distinguishable for callers and tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StableCode {
    TargetSnapshotMissingForLogical,
    TargetSnapshotIdentityDrifted,
    TargetSnapshotPolicyDrifted,
    MixedTargetGroupRejected,
    AdmissionTicketInvalid,
    AdmissionTicketExpired,
    AdmissionTicketConsumed,
    AttemptAwaitingManualRecovery,
    AttemptTerminalTransitionInvalid,
    StoreFailure,
}

impl StableCode {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::TargetSnapshotMissingForLogical => TARGET_SNAPSHOT_MISSING_FOR_LOGICAL,
            Self::TargetSnapshotIdentityDrifted => TARGET_SNAPSHOT_IDENTITY_DRIFTED,
            Self::TargetSnapshotPolicyDrifted => TARGET_SNAPSHOT_POLICY_DRIFTED,
            Self::MixedTargetGroupRejected => MIXED_TARGET_GROUP_REJECTED,
            Self::AdmissionTicketInvalid => ADMISSION_TICKET_INVALID,
            Self::AdmissionTicketExpired => ADMISSION_TICKET_EXPIRED,
            Self::AdmissionTicketConsumed => ADMISSION_TICKET_CONSUMED,
            Self::AttemptAwaitingManualRecovery => ATTEMPT_AWAITING_MANUAL_RECOVERY,
            Self::AttemptTerminalTransitionInvalid => "attempt_terminal_transition_invalid",
            Self::StoreFailure => "admission_store_failure",
        }
    }
}

impl std::fmt::Display for StableCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::error::Error for StableCode {}

#[allow(dead_code)]
/// Resolves and persists an admission ticket for the globally unique attempt id.
pub(crate) fn admit_attempt_for_execution(
    paths: &ProductAppPaths,
    attempt_id: &str,
) -> Result<AdmissionTicketRecord, StableCode> {
    CodingAttemptStore::new(paths.clone()).admit_attempt_for_execution(attempt_id)
}

#[allow(dead_code)]
/// Consumes a persisted admission ticket and atomically transitions the attempt to Running.
pub(crate) fn transition_to_executable(
    paths: &ProductAppPaths,
    attempt_id: &str,
    ticket: &AdmissionTicketRecord,
) -> Result<(), StableCode> {
    CodingAttemptStore::new(paths.clone()).transition_to_executable(attempt_id, ticket)
}

#[allow(dead_code)]
/// Persists the fail-closed manual-recovery state and its stable reason code.
pub(crate) fn transition_to_awaiting_manual_recovery(
    paths: &ProductAppPaths,
    attempt_id: &str,
    reason_stable_code: &str,
) -> Result<(), StableCode> {
    CodingAttemptStore::new(paths.clone())
        .transition_to_awaiting_manual_recovery(attempt_id, reason_stable_code)
}

#[allow(dead_code)]
/// Performs a terminal status transition under the attempt lock. Only terminal targets are
/// accepted; version advancement invalidates any admission ticket issued before termination.
pub(crate) fn transition_to_terminal(
    paths: &ProductAppPaths,
    attempt_id: &str,
    status: CodingAttemptStatus,
) -> Result<(), StableCode> {
    CodingAttemptStore::new(paths.clone()).transition_to_terminal(attempt_id, status)
}

impl CodingAttemptStore {
    /// 在 attempt 级文件锁内重新校验路由、快照身份和 policy，再生成持久化 ticket。
    ///
    /// `Legacy + None` 保留原来的单仓路径；任何 logical 状态中的缺失或漂移均 fail-closed。
    pub(crate) fn admit_attempt_for_execution(
        &self,
        attempt_id: &str,
    ) -> Result<AdmissionTicketRecord, StableCode> {
        let attempt = self
            .get_attempt_by_id(attempt_id)
            .map_err(admission_store_code)?;
        let path = self.attempt_path(&attempt.project_id, &attempt.issue_id, &attempt.id);
        with_exclusive_lock(&path, || {
            self.admit_attempt_for_execution_locked(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
            )
        })
        .map_err(admission_store_code)
    }

    /// 消费此前持久化的 ticket，并在同一个 attempt 级文件锁内完成 version CAS、状态和
    /// ticket consumed_at 的写入。任何失败均不写半状态。
    pub(crate) fn transition_to_executable(
        &self,
        attempt_id: &str,
        ticket: &AdmissionTicketRecord,
    ) -> Result<(), StableCode> {
        if ticket.attempt_id != attempt_id {
            return Err(StableCode::AdmissionTicketInvalid);
        }
        let attempt = self
            .get_attempt_by_id(attempt_id)
            .map_err(admission_store_code)?;
        let path = self.attempt_path(&attempt.project_id, &attempt.issue_id, &attempt.id);
        with_exclusive_lock(&path, || {
            self.transition_to_executable_locked(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                ticket,
            )
        })
        .map_err(admission_store_code)
    }

    /// 受控地将 attempt 置于 abort-only 的人工恢复状态，并持久化机器可读原因。
    pub(crate) fn transition_to_awaiting_manual_recovery(
        &self,
        attempt_id: &str,
        reason_stable_code: &str,
    ) -> Result<(), StableCode> {
        let attempt = self
            .get_attempt_by_id(attempt_id)
            .map_err(admission_store_code)?;
        let path = self.attempt_path(&attempt.project_id, &attempt.issue_id, &attempt.id);
        with_exclusive_lock(&path, || {
            let mut attempt =
                self.get_attempt(&attempt.project_id, &attempt.issue_id, attempt_id)?;
            if attempt.status == CodingAttemptStatus::AwaitingManualRecovery {
                return Ok(());
            }
            if !attempt.status.is_active() {
                return Err(ProductStoreError::Io(format!(
                    "{ATTEMPT_AWAITING_MANUAL_RECOVERY}: {:?}",
                    attempt.status
                )));
            }
            if attempt.status == CodingAttemptStatus::Running {
                attempt.admission_ticket_consumed_at = None;
            }
            attempt.status = CodingAttemptStatus::AwaitingManualRecovery;
            attempt.version += 1;
            attempt.manual_recovery_reason = Some(reason_stable_code.to_string());
            attempt.updated_at = Utc::now().to_rfc3339();
            self.save_coding_attempt_with_status(&attempt)
        })
        .map_err(admission_store_code)
    }

    /// 在 attempt 锁内执行终态状态转换。仅接受终态目标；version 推进会失效任何在
    /// 终止前签发的 admission ticket。
    ///
    /// 注意：当前仅测试使用（自由函数版本 `#[allow(dead_code)]`，无生产调用方）。
    /// 若未来接线到生产，必须保持「离开 Running 同步清 marker」的会话语义——
    /// 此处已预置清理逻辑，接线时无需再补。
    fn transition_to_terminal(
        &self,
        attempt_id: &str,
        status: CodingAttemptStatus,
    ) -> Result<(), StableCode> {
        if !matches!(
            status,
            CodingAttemptStatus::Completed
                | CodingAttemptStatus::Failed
                | CodingAttemptStatus::Aborted
        ) {
            return Err(StableCode::AttemptTerminalTransitionInvalid);
        }
        let attempt = self
            .get_attempt_by_id(attempt_id)
            .map_err(admission_store_code)?;
        let path = self.attempt_path(&attempt.project_id, &attempt.issue_id, &attempt.id);
        with_exclusive_lock(&path, || {
            let mut attempt =
                self.get_attempt(&attempt.project_id, &attempt.issue_id, attempt_id)?;
            if !super::attempt::valid_status_transition(&attempt.status, &status) {
                return Err(ProductStoreError::Io(
                    "attempt_terminal_transition_invalid".to_string(),
                ));
            }
            let now = Utc::now().to_rfc3339();
            // 离开 Running 的终态转换在同一次锁内结束当前 admission 会话（与
            // update_attempt_status / update_group_terminal_status_locked /
            // transition_to_awaiting_manual_recovery 同款会话语义）。
            if attempt.status == CodingAttemptStatus::Running {
                attempt.admission_ticket_consumed_at = None;
            }
            attempt.status = status;
            attempt.version += 1;
            attempt.updated_at = now.clone();
            attempt.completed_at = Some(now);
            self.save_coding_attempt_with_status(&attempt)
        })
        .map_err(admission_store_code)
    }

    /// 将 attempt 置于 Running 的唯一生产组合入口：先在 attempt 锁内重新校验
    /// 路由/快照/policy 并签发 ticket，再于锁内消费 ticket 完成 CAS 转换，
    /// 最后重载并返回权威 record。
    ///
    /// Created/WaitingForHuman/Blocked/ApplyingPlanAmendment/AmendmentApplyFailed
    /// 进入 Running 必须经此入口（或等价的 admit+transition 序列）；调用方不得用
    /// `update_attempt_status` 直接写 Running。
    pub(crate) fn admit_and_transition_attempt_to_executable(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        let attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
        let path = self.attempt_path(&attempt.project_id, &attempt.issue_id, &attempt.id);
        // 直接调用 locked 版本以保留原始 ProductStoreError（稳定码不经过 StableCode 往返）；
        // 两段锁顺序获取、不嵌套，无重入死锁风险。
        let ticket = with_exclusive_lock(&path, || {
            self.admit_attempt_for_execution_locked(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
            )
        })?;
        with_exclusive_lock(&path, || {
            self.transition_to_executable_locked(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &ticket,
            )
        })?;
        self.get_attempt(project_id, issue_id, attempt_id)
    }

    fn admit_attempt_for_execution_locked(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<AdmissionTicketRecord, ProductStoreError> {
        let attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
        if attempt.status == CodingAttemptStatus::AwaitingManualRecovery {
            return Err(ProductStoreError::Io(
                ATTEMPT_AWAITING_MANUAL_RECOVERY.to_string(),
            ));
        }
        if !super::attempt::valid_executable_admission_transition(&attempt.status) {
            return Err(ProductStoreError::Io(format!(
                "invalid_coding_attempt_status_transition: {:?} -> {:?}",
                attempt.status,
                CodingAttemptStatus::Running
            )));
        }

        let routing =
            RepositoryRouting::load_for_issue(&self.paths, &attempt.project_id, &attempt.issue_id)?;
        let snapshot_digest = match (routing, attempt.target_snapshot.as_ref()) {
            (RepositoryRouting::Legacy { .. }, None) => legacy_snapshot_digest(&attempt),
            (RepositoryRouting::Logical { .. }, None) => {
                return Err(ProductStoreError::Io(
                    TARGET_SNAPSHOT_MISSING_FOR_LOGICAL.to_string(),
                ));
            }
            (RepositoryRouting::Logical { .. }, Some(snapshot)) => {
                validate_snapshot_fields(&self.paths, &attempt).map_err(|_| {
                    ProductStoreError::Io(TARGET_SNAPSHOT_IDENTITY_DRIFTED.to_string())
                })?;
                let policy = AggregatePolicyArtifactStore::new(self.paths.clone())
                    .get(&attempt.project_id)?
                    .ok_or_else(|| {
                        ProductStoreError::Io(TARGET_SNAPSHOT_POLICY_DRIFTED.to_string())
                    })?;
                if policy.digest != snapshot.policy_digest {
                    return Err(ProductStoreError::Io(
                        TARGET_SNAPSHOT_POLICY_DRIFTED.to_string(),
                    ));
                }
                snapshot_digest(snapshot)
            }
            (RepositoryRouting::Legacy { .. }, Some(_))
            | (RepositoryRouting::FailClosed { .. }, _) => {
                return Err(ProductStoreError::Io(
                    TARGET_SNAPSHOT_IDENTITY_DRIFTED.to_string(),
                ));
            }
        };

        let now = Utc::now();
        let ticket = AdmissionTicketRecord {
            attempt_id: attempt.id.clone(),
            attempt_version: attempt.version,
            snapshot_digest,
            approved_at: now.to_rfc3339(),
            expires_at: (now + ADMISSION_TICKET_TTL).to_rfc3339(),
            consumed_at: None,
        };
        write_json(
            &self.admission_ticket_path(&attempt.project_id, &attempt.issue_id, &attempt.id),
            &ticket,
        )?;
        Ok(ticket)
    }

    fn transition_to_executable_locked(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        ticket: &AdmissionTicketRecord,
    ) -> Result<(), ProductStoreError> {
        let mut attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
        if attempt.status == CodingAttemptStatus::AwaitingManualRecovery {
            return Err(ProductStoreError::Io(
                ATTEMPT_AWAITING_MANUAL_RECOVERY.to_string(),
            ));
        }
        let ticket_path =
            self.admission_ticket_path(&attempt.project_id, &attempt.issue_id, &attempt.id);
        if attempt.admission_ticket_consumed_at.is_some() {
            return Err(ProductStoreError::Io(ADMISSION_TICKET_CONSUMED.to_string()));
        }
        // 持久化凭证缺失（未签发或已成功消费后清理）时，出示的 ticket 一律按无效拒绝。
        let persisted: AdmissionTicketRecord = read_json(&ticket_path)
            .map_err(|_| ProductStoreError::Io(ADMISSION_TICKET_INVALID.to_string()))?;
        if persisted.attempt_id != ticket.attempt_id
            || persisted.attempt_id != attempt.id
            || persisted.attempt_version != ticket.attempt_version
            || persisted.snapshot_digest != ticket.snapshot_digest
            || persisted.approved_at != ticket.approved_at
            || persisted.expires_at != ticket.expires_at
        {
            return Err(ProductStoreError::Io(ADMISSION_TICKET_INVALID.to_string()));
        }
        if persisted.consumed_at.is_some() {
            return Err(ProductStoreError::Io(ADMISSION_TICKET_CONSUMED.to_string()));
        }
        if ticket_expired(&persisted)? {
            return Err(ProductStoreError::Io(ADMISSION_TICKET_EXPIRED.to_string()));
        }
        if persisted.attempt_version != attempt.version
            || attempt.status == CodingAttemptStatus::Running
            || !super::attempt::valid_executable_admission_transition(&attempt.status)
        {
            return Err(ProductStoreError::Io(ADMISSION_TICKET_INVALID.to_string()));
        }

        let now = Utc::now().to_rfc3339();
        attempt.status = CodingAttemptStatus::Running;
        attempt.version += 1;
        attempt.updated_at = now.clone();
        attempt.admission_ticket_consumed_at = Some(now);
        self.save_coding_attempt_with_status(&attempt)?;
        // The attempt record is authoritative. Cleanup is intentionally best-effort: a failed
        // ticket-file deletion cannot turn a successfully committed transition into an error.
        remove_admission_ticket_best_effort(&ticket_path);
        Ok(())
    }
}

#[cfg(test)]
static ADMISSION_TICKET_CLEANUP_FAILPOINTS: OnceLock<Mutex<HashSet<std::path::PathBuf>>> =
    OnceLock::new();

#[cfg(test)]
struct AdmissionTicketCleanupFailpointGuard {
    ticket_path: std::path::PathBuf,
}

#[cfg(test)]
fn admission_ticket_cleanup_failpoints() -> &'static Mutex<HashSet<std::path::PathBuf>> {
    ADMISSION_TICKET_CLEANUP_FAILPOINTS.get_or_init(|| Mutex::new(HashSet::new()))
}

#[cfg(test)]
fn register_admission_ticket_cleanup_failpoint(
    ticket_path: &std::path::Path,
) -> AdmissionTicketCleanupFailpointGuard {
    let ticket_path = std::fs::canonicalize(ticket_path)
        .expect("persisted admission ticket must be canonicalizable");
    assert!(
        admission_ticket_cleanup_failpoints()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(ticket_path.clone()),
        "admission ticket cleanup failpoint already registered for {}",
        ticket_path.display()
    );
    AdmissionTicketCleanupFailpointGuard { ticket_path }
}

#[cfg(test)]
fn admission_ticket_cleanup_is_failed(ticket_path: &std::path::Path) -> bool {
    let Ok(ticket_path) = std::fs::canonicalize(ticket_path) else {
        return false;
    };
    admission_ticket_cleanup_failpoints()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .contains(&ticket_path)
}

#[cfg(test)]
impl Drop for AdmissionTicketCleanupFailpointGuard {
    fn drop(&mut self) {
        admission_ticket_cleanup_failpoints()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&self.ticket_path);
    }
}

fn remove_admission_ticket_best_effort(ticket_path: &std::path::Path) {
    #[cfg(test)]
    if admission_ticket_cleanup_is_failed(ticket_path) {
        return;
    }
    let _ = std::fs::remove_file(ticket_path);
}

fn ticket_expired(ticket: &AdmissionTicketRecord) -> Result<bool, ProductStoreError> {
    let expires_at = DateTime::parse_from_rfc3339(&ticket.expires_at)
        .map_err(|_| ProductStoreError::Io(ADMISSION_TICKET_INVALID.to_string()))?;
    Ok(expires_at <= Utc::now())
}

fn snapshot_digest(snapshot: &crate::product::coding_models::AttemptTargetSnapshot) -> String {
    let encoded = serde_json::to_vec(snapshot).expect("AttemptTargetSnapshot must serialize");
    format!("sha256:{:x}", Sha256::digest(encoded))
}

fn legacy_snapshot_digest(attempt: &CodingExecutionAttempt) -> String {
    format!(
        "legacy:{}:{}:{}",
        attempt.project_id, attempt.issue_id, attempt.id
    )
}

fn admission_store_code(error: ProductStoreError) -> StableCode {
    match error {
        ProductStoreError::Io(message) if message == TARGET_SNAPSHOT_MISSING_FOR_LOGICAL => {
            StableCode::TargetSnapshotMissingForLogical
        }
        ProductStoreError::Io(message) if message == TARGET_SNAPSHOT_IDENTITY_DRIFTED => {
            StableCode::TargetSnapshotIdentityDrifted
        }
        ProductStoreError::Io(message) if message == TARGET_SNAPSHOT_POLICY_DRIFTED => {
            StableCode::TargetSnapshotPolicyDrifted
        }
        ProductStoreError::Io(message) if message == ADMISSION_TICKET_INVALID => {
            StableCode::AdmissionTicketInvalid
        }
        ProductStoreError::Io(message) if message == ADMISSION_TICKET_EXPIRED => {
            StableCode::AdmissionTicketExpired
        }
        ProductStoreError::Io(message) if message == ADMISSION_TICKET_CONSUMED => {
            StableCode::AdmissionTicketConsumed
        }
        ProductStoreError::Io(message) if message == ATTEMPT_AWAITING_MANUAL_RECOVERY => {
            StableCode::AttemptAwaitingManualRecovery
        }
        ProductStoreError::Io(message) if message == "attempt_terminal_transition_invalid" => {
            StableCode::AttemptTerminalTransitionInvalid
        }
        _ => StableCode::StoreFailure,
    }
}

#[cfg(test)]
mod tests {
    include!("admission_tests.rs");
}
