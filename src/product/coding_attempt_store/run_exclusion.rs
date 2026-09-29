//! C2 Task 2（REQ-CRO-02）：最小编码互斥、接管判别与 attempt 命令账本。
//!
//! 复用 C1（`enrollment-recovery-surface`）已落地的租约三态判定与命令账本
//! `payload_digest` 幂等模式：admission 临界区内串接「命令账本 → 租约三态 →
//! 既有 `ensure_provider_run_allowed` 校验」。不引入 durable owner／fence／
//! incarnation，不自动抢占任何无法证明死亡的租约。

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::product::coding_models::CodingExecutionAttempt;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::automation::{LeaseDecision, LeaseDisposition, OperationState};

use super::locking::with_exclusive_lock;

/// C2 Task 2：attempt 命令账本条目（attempt-scoped journal；同 C1
/// `EnrollmentCommandResult` 的幂等语义）。同 command 同 payload 重放首次
/// durable 结果，同 command 异 payload fail-closed（请刷新）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingAttemptCommandRecord {
    pub command_id: String,
    pub payload_digest: String,
    pub state: OperationState,
    pub recorded_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
struct CodingAttemptCommandLedger {
    #[serde(default)]
    records: Vec<CodingAttemptCommandRecord>,
}

/// C2 Task 2：admission 互斥判定。三态证据全部来自 C1
/// `classify_worktree_lease`（不新建判定）：
/// - `Allowed`：无租约事实（legacy/未启锁）、自持活跃租约的合法续跑，或
///   接管清出后的继续请求——经既有 admission 校验后放行。
/// - `AlreadyRunning`：活跃租约由其他持有者持有（ActiveWait）——拒绝并
///   通知"已在运行／请等待"，不启动 provider。
/// - `TakeoverRequired`：租约已死（终态持有者／锁已释放）——呈现"确认
///   接管"（复用 C1 `confirm_takeover`），未确认不继续。
/// - `LeaseUnknown`：证据缺失或读失败——停等，绝不抢占。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodingRunExclusionDecision {
    Allowed(CodingExecutionAttempt),
    AlreadyRunning { lease: LeaseDecision },
    TakeoverRequired { lease: LeaseDecision },
    LeaseUnknown { lease: LeaseDecision },
}

impl super::CodingAttemptStore {
    fn attempt_command_ledger_path(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<std::path::PathBuf, ProductStoreError> {
        validate_relative_id(attempt_id)?;
        Ok(self
            .attempt_dir(project_id, issue_id, attempt_id)
            .join("command-ledger.json"))
    }

    /// C2 Task 2：attempt 命令账本读取（只读，不改变任何事实）。
    pub fn find_attempt_command_result(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        command_id: &str,
    ) -> Result<Option<CodingAttemptCommandRecord>, ProductStoreError> {
        let path = self.attempt_command_ledger_path(project_id, issue_id, attempt_id)?;
        with_exclusive_lock(&path, || {
            if !path.is_file() {
                return Ok(None);
            }
            let ledger: CodingAttemptCommandLedger = read_json(&path)?;
            Ok(ledger
                .records
                .into_iter()
                .find(|record| record.command_id == command_id))
        })
    }

    /// C2 Task 12：列出 attempt 命令账本全部条目（只读；投影据此识别
    /// admission 停等事实，如 NeedsHuman 的双 kick／restart 拒绝记录）。
    pub fn list_attempt_command_records(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<CodingAttemptCommandRecord>, ProductStoreError> {
        let path = self.attempt_command_ledger_path(project_id, issue_id, attempt_id)?;
        with_exclusive_lock(&path, || {
            if !path.is_file() {
                return Ok(Vec::new());
            }
            Ok(read_json::<CodingAttemptCommandLedger>(&path)?.records)
        })
    }

    /// C2 Task 2：追加 attempt 命令账本（同 C1 幂等语义）：同 command 同
    /// payload 返回既有条目（不重复记录），同 command 异 payload Conflict。
    pub fn append_attempt_command_result(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        record: &CodingAttemptCommandRecord,
    ) -> Result<CodingAttemptCommandRecord, ProductStoreError> {
        validate_relative_id(&record.command_id)?;
        let path = self.attempt_command_ledger_path(project_id, issue_id, attempt_id)?;
        with_exclusive_lock(&path, || {
            let mut ledger: CodingAttemptCommandLedger = if path.is_file() {
                read_json(&path)?
            } else {
                CodingAttemptCommandLedger::default()
            };
            if let Some(existing) = ledger
                .records
                .iter()
                .find(|entry| entry.command_id == record.command_id)
            {
                if existing.payload_digest == record.payload_digest {
                    return Ok(existing.clone());
                }
                return Err(ProductStoreError::Conflict {
                    kind: "coding_attempt_command_ledger",
                    id: record.command_id.clone(),
                });
            }
            ledger.records.push(record.clone());
            write_json(&path, &ledger)?;
            Ok(record.clone())
        })
    }

    /// C2 Task 2：admission 互斥入口（在既有 admission 临界区语义内串接：
    /// 命令账本 → `classify_worktree_lease` 三态 → 既有
    /// `ensure_provider_run_allowed` 校验）。停等结论会先落账本（首次
    /// durable 结果），重放返回同一结论；不在此启动 provider、不清出任何
    /// 租约——接管确认只走既有 `confirm_takeover`。
    pub fn admit_coding_run_exclusive(
        &self,
        attempt: &CodingExecutionAttempt,
        command_id: &str,
        payload_digest: &str,
    ) -> Result<CodingRunExclusionDecision, ProductStoreError> {
        validate_relative_id(command_id)?;
        let ledger_path =
            self.attempt_command_ledger_path(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        with_exclusive_lock(&ledger_path, || {
            // ① 命令账本：同 command 同 payload 重放首次 durable 结果；
            //    异 payload fail-closed。
            let mut ledger: CodingAttemptCommandLedger = if ledger_path.is_file() {
                read_json(&ledger_path)?
            } else {
                CodingAttemptCommandLedger::default()
            };
            if let Some(existing_index) = ledger
                .records
                .iter()
                .position(|entry| entry.command_id == command_id)
            {
                if ledger.records[existing_index].payload_digest != payload_digest {
                    return Err(ProductStoreError::Conflict {
                        kind: "coding_attempt_command_ledger",
                        id: command_id.to_string(),
                    });
                }
                if ledger.records[existing_index].state != OperationState::Accepted {
                    // 首次 durable 结果是停等：以当前租约证据重放同类停等；
                    // 冲突已解除（如经 confirm_takeover 清出）时按当前请求
                    // 继续——既有校验通过后推进同 command 的账本相位为
                    // Accepted（同 command 同 payload）。
                    let lease =
                        self.classify_worktree_lease(&attempt.project_id, &attempt.issue_id);
                    if let Some(exclusion) = self.lease_exclusion_option(&lease, attempt) {
                        return Ok(exclusion);
                    }
                    let authoritative = self.ensure_provider_run_allowed(attempt)?;
                    ledger.records[existing_index].state = OperationState::Accepted;
                    ledger.records[existing_index].recorded_at = Utc::now().to_rfc3339();
                    write_json(&ledger_path, &ledger)?;
                    return Ok(CodingRunExclusionDecision::Allowed(authoritative));
                }
                // 首次结果 Accepted：重放放行（provider 启动去重由调用侧
                // 临界区收口，账本不重复驱动副作用）。
                return Ok(CodingRunExclusionDecision::Allowed(
                    self.ensure_provider_run_allowed(attempt)?,
                ));
            }

            // ② 租约三态（复用 C1 判定）：活跃他人 → AlreadyRunning；
            //    死亡 → TakeoverRequired；未知 → LeaseUnknown。无租约事实
            //    与自持活跃租约不拦截（合法续跑）。
            let lease = self.classify_worktree_lease(&attempt.project_id, &attempt.issue_id);
            if let Some(exclusion) = self.lease_exclusion_option(&lease, attempt) {
                let record = CodingAttemptCommandRecord {
                    command_id: command_id.to_string(),
                    payload_digest: payload_digest.to_string(),
                    state: OperationState::NeedsHuman,
                    recorded_at: Utc::now().to_rfc3339(),
                };
                ledger.records.push(record);
                write_json(&ledger_path, &ledger)?;
                return Ok(exclusion);
            }

            // ③ 既有 admission 校验（错误透传，不落 Accepted）。
            let authoritative = self.ensure_provider_run_allowed(attempt)?;

            // ④ 放行事实落账本（Accepted）。
            let record = CodingAttemptCommandRecord {
                command_id: command_id.to_string(),
                payload_digest: payload_digest.to_string(),
                state: OperationState::Accepted,
                recorded_at: Utc::now().to_rfc3339(),
            };
            ledger.records.push(record);
            write_json(&ledger_path, &ledger)?;
            Ok(CodingRunExclusionDecision::Allowed(authoritative))
        })
    }

    /// 互斥只拦「跨持有者冲突」：活跃他人 → AlreadyRunning；死亡他人仍
    /// 占锁 → TakeoverRequired（确认接管只走 C1 `confirm_takeover`）；
    /// 未知且非无事实 → LeaseUnknown。自持活跃租约（阶段续跑）、自持
    /// 死亡租约、锁已释放（空 owner）与无租约事实（未启锁的 legacy
    /// attempt）不拦截——维持既有链路行为零回归。
    fn lease_exclusion_option(
        &self,
        lease: &LeaseDecision,
        attempt: &CodingExecutionAttempt,
    ) -> Option<CodingRunExclusionDecision> {
        let foreign_holder = !lease.lease_id.is_empty() && lease.lease_id != attempt.id;
        match lease.disposition {
            LeaseDisposition::ActiveWait if foreign_holder => {
                Some(CodingRunExclusionDecision::AlreadyRunning {
                    lease: lease.clone(),
                })
            }
            LeaseDisposition::DeadNeedsTakeover if foreign_holder => {
                Some(CodingRunExclusionDecision::TakeoverRequired {
                    lease: lease.clone(),
                })
            }
            LeaseDisposition::UnknownNeedsHuman => {
                // 无租约事实（未启锁的 legacy attempt）不拦截既有链路；
                // 其余未知证据（瞬态 owner／读取失败）一律停等。
                if lease
                    .evidence
                    .iter()
                    .any(|fact| fact.contains("worktree record not found"))
                {
                    None
                } else {
                    Some(CodingRunExclusionDecision::LeaseUnknown {
                        lease: lease.clone(),
                    })
                }
            }
            _ => None,
        }
    }

    // ─── C1 Task 6（REQ-WIGA-03）：租约三态判定（store 级实现；引擎侧
    // `classify_worktree_lease` 委托此处，单一实现）───

    /// 租约三态判定（只读）。只从现有 durable 证据分类：
    /// `IssueSharedWorktree.current_lock_owner_id`/`current_active_work_item_id`
    /// 与 owner attempt 的 status；issue 维老路径优先，多仓按仓维确定性
    /// 顺序检查。活跃→`ActiveWait`（等待，不抢占）；owner 是终态 attempt
    /// 或锁已明确释放→`DeadNeedsTakeover`（需用户确认才接管）；owner 未
    /// 绑定 attempt（`*_worktree_lease_*` 瞬态）、证据缺失或读失败→
    /// `UnknownNeedsHuman`（绝不抢占）。不写任何文件、不启动 provider。
    pub fn classify_worktree_lease(&self, project_id: &str, issue_id: &str) -> LeaseDecision {
        use crate::product::lifecycle_store::LifecycleStore;
        use crate::product::models::IssueSharedWorktree;

        let lifecycle = LifecycleStore::new(self.paths());
        match lifecycle.get_issue_shared_worktree(project_id, issue_id) {
            Ok(Some(record)) => self.classify_worktree_record(&record),
            Ok(None) => {
                let repo_ids = match lifecycle.list_repo_shared_worktrees(project_id, issue_id) {
                    Ok(ids) => ids,
                    Err(error) => {
                        return LeaseDecision {
                            disposition: LeaseDisposition::UnknownNeedsHuman,
                            lease_id: String::new(),
                            last_activity_at: None,
                            evidence: vec![format!("list repo worktrees failed: {error}")],
                        };
                    }
                };
                let mut free_records = 0usize;
                for repository_id in repo_ids {
                    match lifecycle.get_repo_shared_worktree(project_id, issue_id, repository_id) {
                        Ok(Some(record)) => {
                            if record.current_lock_owner_id.is_some() {
                                return self.classify_worktree_record(&record);
                            }
                            free_records += 1;
                        }
                        Ok(None) => {}
                        Err(error) => {
                            return LeaseDecision {
                                disposition: LeaseDisposition::UnknownNeedsHuman,
                                lease_id: String::new(),
                                last_activity_at: None,
                                evidence: vec![format!(
                                    "repo worktree {} read failed: {error}",
                                    repository_id.0
                                )],
                            };
                        }
                    }
                }
                if free_records > 0 {
                    return LeaseDecision {
                        disposition: LeaseDisposition::DeadNeedsTakeover,
                        lease_id: String::new(),
                        last_activity_at: None,
                        evidence: vec![
                            "worktree lock explicitly released".to_string(),
                            "no repo worktree holds an owner".to_string(),
                        ],
                    };
                }
                LeaseDecision {
                    disposition: LeaseDisposition::UnknownNeedsHuman,
                    lease_id: String::new(),
                    last_activity_at: None,
                    evidence: vec!["worktree record not found for issue".to_string()],
                }
            }
            Err(error) => LeaseDecision {
                disposition: LeaseDisposition::UnknownNeedsHuman,
                lease_id: String::new(),
                last_activity_at: None,
                evidence: vec![format!("issue worktree read failed: {error}")],
            },
        }
    }

    /// 单条 worktree 记录的三态分类：owner 缺失按证据一致性分流（无
    /// active item＝已释放；有 active item＝证据不一致→Unknown）；owner
    /// 在场时按 owner attempt 的 status 判活跃/死亡，owner 不是本 issue
    /// 的 attempt（瞬态 lease 前缀或漂移）一律 Unknown（fail-closed）。
    fn classify_worktree_record(
        &self,
        record: &crate::product::models::IssueSharedWorktree,
    ) -> LeaseDecision {
        let unknown = |messages: Vec<String>| LeaseDecision {
            disposition: LeaseDisposition::UnknownNeedsHuman,
            lease_id: record.current_lock_owner_id.clone().unwrap_or_default(),
            last_activity_at: Some(record.updated_at.clone()),
            evidence: messages,
        };
        match (&record.current_lock_owner_id, &record.current_active_work_item_id) {
            (None, None) => LeaseDecision {
                disposition: LeaseDisposition::DeadNeedsTakeover,
                lease_id: String::new(),
                last_activity_at: Some(record.updated_at.clone()),
                evidence: vec![
                    "worktree lock explicitly released".to_string(),
                    format!(
                        "last completed item: {:?}",
                        record.last_completed_work_item_id
                    ),
                ],
            },
            (None, Some(active)) => unknown(vec![
                "lock owner missing while active work item present".to_string(),
                format!("active work item: {active}"),
            ]),
            (Some(owner), active) => {
                let mut evidence = vec![
                    format!("worktree lock owner: {owner}"),
                    format!("active work item: {active:?}"),
                ];
                match self.get_attempt(&record.project_id, &record.issue_id, owner) {
                    Ok(attempt) if attempt.status.is_active() => {
                        evidence.push(format!(
                            "owner attempt {} is active ({:?})",
                            attempt.id, attempt.status
                        ));
                        LeaseDecision {
                            disposition: LeaseDisposition::ActiveWait,
                            lease_id: owner.clone(),
                            last_activity_at: Some(record.updated_at.clone()),
                            evidence,
                        }
                    }
                    Ok(attempt) => {
                        evidence.push(format!(
                            "owner attempt {} is terminal ({:?})",
                            attempt.id, attempt.status
                        ));
                        LeaseDecision {
                            disposition: LeaseDisposition::DeadNeedsTakeover,
                            lease_id: owner.clone(),
                            last_activity_at: Some(record.updated_at.clone()),
                            evidence,
                        }
                    }
                    Err(ProductStoreError::NotFound { .. }) => {
                        evidence.push(format!(
                            "owner {owner} does not resolve to an attempt of this issue; \
                             liveness cannot be proven"
                        ));
                        unknown(evidence)
                    }
                    Err(error) => {
                        evidence.push(format!("owner attempt read failed: {error}"));
                        unknown(evidence)
                    }
                }
            }
        }
    }
}
