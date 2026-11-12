//! SC 生成教学重驱的耐久授权窗口 store CAS（REQ-WSC-28B/28C store 面，
//! bound-sc-generation-teaching Task 3）：会话排它锁 + 整记录 CAS 下的
//! 建立/批准/claim/真实启动回填/Unknown 迁移/关闭。全部方法遵循
//! `reserve_single_candidate_provider_start` 先例（`workspace_single_candidate.rs`）：
//! 锁内重读 stored、`stored != *expected` 即 `Conflict`、单次 `write_json`
//! 落整记录（同事务内的多字段变更原子完成；拒绝/幂等路径不写盘——历史
//! 记录不倒填，空窗口序列化缺省不写键）。

use chrono::Utc;

use super::LifecycleStore;
use crate::product::coding_attempt_store::locking::with_exclusive_lock;
use crate::product::json_store::{ProductStoreError, read_json, write_json};
use crate::product::models::{SingleCandidatePhase, WorkspaceSessionRecord};
use crate::product::planning_failure::StructuralAuthorizationKind;
use crate::product::work_item_plan_policy::{
    PolicyDiagnostic, ProviderStartLedgerEntry, SC_TEACHING_APPROVAL_RULE_VERSION,
    ScGenerationTeachingWindow, ScTeachingApprovalRejection, ScTeachingAttemptLifecycle,
    ScTeachingAttemptOutcome, ScTeachingAttemptState, ScTeachingWindowCloseReason,
    ScTeachingWindowRequest, ScTeachingWindowStatus, evaluate_sc_teaching_approval,
    next_sc_author_launch_ordinal, non_teaching_provider_start_count,
    sc_teaching_frozen_input_digest,
};

/// 未批准窗口的 durable 诊断码（同事务留痕，message 含拒绝枚举名）。
const SC_TEACHING_NOT_APPROVED_CODE: &str = "sc_teaching_window_not_approved";

/// 冻结输入漂移的 durable 诊断码（Task 6 handler 面：零额外调用的留痕）。
const SC_TEACHING_FROZEN_INPUT_DRIFT_CODE: &str = "sc_teaching_frozen_input_drift";

/// establish 的结果：新建（含四项合取批准判定与拒绝留痕）或既有
/// （末尾窗口 Active ⇒ A/B 接力与重放不得重建，窗口字段零变化）。
#[derive(Debug, Clone, PartialEq)]
pub enum ScTeachingEstablishOutcome {
    /// 新建窗口（无论批准与否都建立：未批准窗口 max=1 且同事务写诊断留痕）。
    Established {
        record: WorkspaceSessionRecord,
        approved: bool,
        rejection: Option<ScTeachingApprovalRejection>,
    },
    /// 已有 Active 窗口（重放/接力共用同一 durable 窗口）。
    Existing {
        record: WorkspaceSessionRecord,
        window_generation_seq: u32,
    },
}

/// claim 的结果：新授权 / 同 attempt 幂等重放 / 可判定拒绝（tasks 1.3）。
#[derive(Debug, Clone, PartialEq)]
pub enum ScTeachingClaimOutcome {
    /// 新 attempt 已 push（含同事务落盘的 ledger 占位条目）。
    Granted(WorkspaceSessionRecord),
    /// 末 attempt 已 Claimed/LaunchUnknown ⇒ 同 attempt_key 幂等返回，不追加。
    Replay(WorkspaceSessionRecord),
    /// 可判定拒绝（不写盘、不倒填任何字段）。
    Denied(ScTeachingClaimRejection),
}

/// claim 的可判定拒绝原因（tasks 1.3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScTeachingClaimRejection {
    /// 记录无任何窗口（历史旧记录零授权）。
    NoWindow,
    /// 窗口建立时未被批准（max=1 且 attempt1 已固化）。
    NotApproved,
    /// 窗口已关闭。
    WindowClosed,
    /// 机会已耗尽：max_attempts 与已消费数（含原调用）。
    Exhausted { max_attempts: u8, consumed: u8 },
}

/// 建立时前缀 `single_candidate_author:{id}:` 的最后一个条目键（原授权引用；
/// 无条目 ⇒ None）。与 `evaluate_sc_teaching_approval` 内部口径一致。
fn last_sc_author_launch_key(record: &WorkspaceSessionRecord) -> Option<String> {
    let prefix = format!("single_candidate_author:{}:", record.id);
    record
        .provider_start_ledger
        .iter()
        .rev()
        .find(|entry| entry.provider_start_idempotency_key.starts_with(&prefix))
        .map(|entry| entry.provider_start_idempotency_key.clone())
}

impl LifecycleStore {
    /// 教学窗口族共用的锁内 CAS 前奏（先例 `reserve_single_candidate_provider_start`）：
    /// 实体准入 → 排它锁 → 重读 stored → 整记录比对（expected 漂移即
    /// `Conflict`）。会话路径交给 body，由各方法在**单次** `write_json` 内
    /// 完成全部字段变更（同事务原子）；拒绝/幂等路径直接返回、不写盘。
    fn with_sc_teaching_session<T>(
        &self,
        expected: &WorkspaceSessionRecord,
        body: impl FnOnce(&std::path::Path, &mut WorkspaceSessionRecord) -> Result<T, ProductStoreError>,
    ) -> Result<T, ProductStoreError> {
        let admission_scope = self.entity_scope_for_session(expected)?;
        self.with_optional_entity_live_write(admission_scope.as_ref(), move || {
            let session_path = self.find_workspace_session_path(&expected.id)?;
            with_exclusive_lock(&session_path, || {
                let mut stored: WorkspaceSessionRecord = read_json(&session_path)?;
                if stored != *expected {
                    return Err(ProductStoreError::Conflict {
                        kind: "workspace_session",
                        id: expected.id.clone(),
                    });
                }
                body(&session_path, &mut stored)
            })
        })
    }

    /// 建立窗口（锁内 CAS expected）：末尾窗口 Active ⇒ `Existing`（A/B 接力
    /// 与重放不得重建）；否则 `evaluate_sc_teaching_approval` 判定 approved
    /// （四项合取），生成窗口（approved ⇒ max=2；否则 max=1 且同事务写一条
    /// PolicyDiagnostic：code `sc_teaching_window_not_approved`，message 含
    /// 拒绝枚举名——显式系统故障留痕），attempts[0] = attempt1（原调用，
    /// 固化 `Terminal{FailedTerminal}`，launch_ledger_key = 建立时前缀最后
    /// 一个条目键），attempt 不含占位新条目（REQ-WSC-28B，design D1/D2）。
    pub fn establish_sc_teaching_window(
        &self,
        expected: &WorkspaceSessionRecord,
        request: &ScTeachingWindowRequest,
    ) -> Result<ScTeachingEstablishOutcome, ProductStoreError> {
        self.with_sc_teaching_session(expected, |session_path, stored| {
            if let Some(last) = stored.sc_generation_teaching_windows.last()
                && matches!(last.status, ScTeachingWindowStatus::Active)
            {
                return Ok(ScTeachingEstablishOutcome::Existing {
                    record: stored.clone(),
                    window_generation_seq: last.generation_seq,
                });
            }
            let (approved, rejection) = match evaluate_sc_teaching_approval(stored, request) {
                Ok(_) => (true, None),
                Err(rejection) => (false, Some(rejection)),
            };
            // 原授权引用：批准与拒绝**同一条路径**——按同口径回查前缀最后一个条目键
            // （`last_sc_author_launch_key`，与 evaluate 的判据① 查找规则逐字相同）；
            // 判据①失败可能无条目 ⇒ 空串显式记录。（evaluate 的 Ok 载荷在此**不**取用。）
            let approval_checked_against =
                last_sc_author_launch_key(stored).unwrap_or_default();
            // 本链 base = 上一已关闭窗口的关闭快照（gen0 无前窗 ⇒ 0）。
            let chain_non_teaching_base = stored
                .sc_generation_teaching_windows
                .iter()
                .rev()
                .find(|window| matches!(window.status, ScTeachingWindowStatus::Closed { .. }))
                .and_then(|window| window.non_teaching_total_at_close)
                .unwrap_or(0);
            let generation_seq = stored.sc_generation_teaching_windows.len() as u32;
            let original_launch_key = last_sc_author_launch_key(stored);
            let window = ScGenerationTeachingWindow {
                generation_seq,
                authorization_kind: StructuralAuthorizationKind::GenerationRun,
                original_session_id: stored.id.clone(),
                original_run_phase: SingleCandidatePhase::Generate,
                established_at: Utc::now().to_rfc3339(),
                request: request.clone(),
                frozen_input_digest: sc_teaching_frozen_input_digest(request),
                frozen_execution_policy_digest: None,
                repair_once_approved: approved,
                approval_rule_version: SC_TEACHING_APPROVAL_RULE_VERSION.to_string(),
                approval_checked_against,
                max_attempts: u8::from(approved) + 1,
                attempts: vec![ScTeachingAttemptState {
                    attempt_no: 1,
                    attempt_key: ScGenerationTeachingWindow::attempt_key(
                        &stored.id,
                        generation_seq,
                        1,
                    ),
                    launch_ledger_key: original_launch_key,
                    lifecycle: ScTeachingAttemptLifecycle::Terminal {
                        outcome: ScTeachingAttemptOutcome::FailedTerminal,
                    },
                }],
                status: ScTeachingWindowStatus::Active,
                chain_non_teaching_base,
                non_teaching_total_at_close: None,
            };
            if let Some(rejection) = rejection {
                stored.policy_diagnostics.push(PolicyDiagnostic {
                    code: SC_TEACHING_NOT_APPROVED_CODE.to_string(),
                    message: format!(
                        "SC 教学窗口未批准：{rejection:?}（规则版本 {SC_TEACHING_APPROVAL_RULE_VERSION}，会话 {}）",
                        stored.id
                    ),
                    field: None,
                });
            }
            stored.sc_generation_teaching_windows.push(window);
            stored.updated_at = Utc::now().to_rfc3339();
            write_json(session_path, stored)?;
            Ok(ScTeachingEstablishOutcome::Established {
                record: stored.clone(),
                approved,
                rejection,
            })
        })
    }

    /// claim（锁内 CAS expected）⇒ 定位 generation_seq 窗口 ⇒ Closed/未批准/
    /// 超限 ⇒ `Denied`（可判定原因）；末 attempt 已 Claimed/LaunchUnknown ⇒
    /// `Replay`（同 attempt_key 幂等，不追加）；否则 push attempt{no =
    /// attempts.len()+1, Claimed} 并 push ledger 占位条目
    /// {key: single_candidate_author:{id}:{next_sc_author_launch_ordinal},
    /// started: false, provider: None, started_at: None,
    /// provider_attempt_key: Some(attempt_key)}——原子 claim + 占位**一次**
    /// 事务完成（REQ-WSC-28C，tasks 1.3/2.2）。
    pub fn claim_sc_teaching_attempt(
        &self,
        expected: &WorkspaceSessionRecord,
        generation_seq: u32,
    ) -> Result<ScTeachingClaimOutcome, ProductStoreError> {
        self.with_sc_teaching_session(expected, |session_path, stored| {
            let Some(window) = stored
                .sc_generation_teaching_windows
                .iter()
                .find(|window| window.generation_seq == generation_seq)
            else {
                return Ok(ScTeachingClaimOutcome::Denied(
                    ScTeachingClaimRejection::NoWindow,
                ));
            };
            if matches!(window.status, ScTeachingWindowStatus::Closed { .. }) {
                return Ok(ScTeachingClaimOutcome::Denied(
                    ScTeachingClaimRejection::WindowClosed,
                ));
            }
            if !window.repair_once_approved {
                return Ok(ScTeachingClaimOutcome::Denied(
                    ScTeachingClaimRejection::NotApproved,
                ));
            }
            if matches!(
                window.attempts.last().map(|attempt| &attempt.lifecycle),
                Some(
                    ScTeachingAttemptLifecycle::Claimed | ScTeachingAttemptLifecycle::LaunchUnknown
                )
            ) {
                return Ok(ScTeachingClaimOutcome::Replay(stored.clone()));
            }
            let consumed = window.attempts.len() as u8;
            if consumed >= window.max_attempts {
                return Ok(ScTeachingClaimOutcome::Denied(
                    ScTeachingClaimRejection::Exhausted {
                        max_attempts: window.max_attempts,
                        consumed,
                    },
                ));
            }
            let attempt_no = consumed + 1;
            let attempt_key =
                ScGenerationTeachingWindow::attempt_key(&stored.id, generation_seq, attempt_no);
            let launch_ledger_key = format!(
                "single_candidate_author:{}:{}",
                stored.id,
                next_sc_author_launch_ordinal(stored)
            );
            let window = stored
                .sc_generation_teaching_windows
                .iter_mut()
                .find(|window| window.generation_seq == generation_seq)
                .expect("window located above");
            window.attempts.push(ScTeachingAttemptState {
                attempt_no,
                attempt_key: attempt_key.clone(),
                launch_ledger_key: Some(launch_ledger_key.clone()),
                lifecycle: ScTeachingAttemptLifecycle::Claimed,
            });
            stored.provider_start_ledger.push(ProviderStartLedgerEntry {
                provider_start_idempotency_key: launch_ledger_key,
                started: false,
                provider: None,
                started_at: None,
                provider_attempt_key: Some(attempt_key),
            });
            stored.updated_at = Utc::now().to_rfc3339();
            write_json(session_path, stored)?;
            Ok(ScTeachingClaimOutcome::Granted(stored.clone()))
        })
    }

    /// 回填（锁内 CAS expected）⇒ 定位窗口与 attempt ⇒ 窗口必须 Active
    /// （已关 ⇒ 可判定拒绝，不静默补全——Task 3 评审 Minor-1 加固）⇒ 仅接受
    /// Claimed/LaunchUnknown（ActualStarted 且证据相同 ⇒ 幂等返回 stored；
    /// 证据不同 ⇒ `Err(Conflict)`；Terminal ⇒ `Err(InvalidRecord)`，不接受
    /// 自报）⇒ 按 launch_ledger_key 定位占位条目且 provider_attempt_key 匹配，
    /// **原位**补全 started=true + provider + started_at（绝不另插第二条）⇒
    /// attempt → ActualStarted（REQ-WSC-28C，tasks 2.2）。
    pub fn observe_sc_teaching_provider_start(
        &self,
        expected: &WorkspaceSessionRecord,
        generation_seq: u32,
        attempt_no: u8,
        provider: &str,
        started_at: String,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        self.with_sc_teaching_session(expected, |session_path, stored| {
            let invalid = |reason: &str| ProductStoreError::InvalidRecord {
                kind: "sc_teaching_observe",
                reason: reason.to_string(),
            };
            let Some(window) = stored
                .sc_generation_teaching_windows
                .iter()
                .find(|window| window.generation_seq == generation_seq)
            else {
                return Err(invalid("teaching window for the generation not found"));
            };
            if matches!(window.status, ScTeachingWindowStatus::Closed { .. }) {
                return Err(invalid(
                    "teaching window is closed; launch observation is refused",
                ));
            }
            let Some(attempt) = window
                .attempts
                .iter()
                .find(|attempt| attempt.attempt_no == attempt_no)
            else {
                return Err(invalid("teaching attempt not claimed at this number"));
            };
            let launch_ledger_key = attempt.launch_ledger_key.clone();
            let attempt_key = attempt.attempt_key.clone();
            let matches_placeholder = |entry: &ProviderStartLedgerEntry| {
                Some(entry.provider_start_idempotency_key.as_str()) == launch_ledger_key.as_deref()
                    && entry.provider_attempt_key.as_deref() == Some(attempt_key.as_str())
            };
            match attempt.lifecycle {
                ScTeachingAttemptLifecycle::Claimed | ScTeachingAttemptLifecycle::LaunchUnknown => {
                }
                ScTeachingAttemptLifecycle::ActualStarted => {
                    // 幂等：证据相同 ⇒ 原样返回（不写盘）；不同 ⇒ Conflict。
                    let same_evidence = stored.provider_start_ledger.iter().any(|entry| {
                        matches_placeholder(entry)
                            && entry.provider.as_deref() == Some(provider)
                            && entry.started_at.as_deref() == Some(started_at.as_str())
                    });
                    if same_evidence {
                        return Ok(stored.clone());
                    }
                    return Err(ProductStoreError::Conflict {
                        kind: "sc_teaching_provider_start",
                        id: attempt_key,
                    });
                }
                ScTeachingAttemptLifecycle::Terminal { .. } => {
                    return Err(invalid("terminal teaching attempt cannot be re-observed"));
                }
            }
            let entry_index = stored
                .provider_start_ledger
                .iter()
                .position(matches_placeholder)
                .ok_or_else(|| invalid("placeholder ledger entry for the attempt is missing"))?;
            // 原位补全（索引定位后改写，绝不 push 第二条）。
            let entry = &mut stored.provider_start_ledger[entry_index];
            entry.started = true;
            entry.provider = Some(provider.to_string());
            entry.started_at = Some(started_at);
            let window = stored
                .sc_generation_teaching_windows
                .iter_mut()
                .find(|window| window.generation_seq == generation_seq)
                .expect("window located above");
            let attempt = window
                .attempts
                .iter_mut()
                .find(|attempt| attempt.attempt_no == attempt_no)
                .expect("attempt located above");
            attempt.lifecycle = ScTeachingAttemptLifecycle::ActualStarted;
            stored.updated_at = Utc::now().to_rfc3339();
            write_json(session_path, stored)?;
            Ok(stored.clone())
        })
    }

    /// 迁移 Unknown（锁内 CAS）：attempt Claimed → LaunchUnknown（崩溃/回填
    /// 失败收口用）；已 LaunchUnknown ⇒ 幂等返回不写盘；ActualStarted/
    /// Terminal ⇒ 可判定拒绝（design D2 fail-closed）。
    pub fn mark_sc_teaching_attempt_unknown(
        &self,
        expected: &WorkspaceSessionRecord,
        generation_seq: u32,
        attempt_no: u8,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        self.with_sc_teaching_session(expected, |session_path, stored| {
            let invalid = |reason: &str| ProductStoreError::InvalidRecord {
                kind: "sc_teaching_mark_unknown",
                reason: reason.to_string(),
            };
            let Some(window) = stored
                .sc_generation_teaching_windows
                .iter()
                .find(|window| window.generation_seq == generation_seq)
            else {
                return Err(invalid("teaching window for the generation not found"));
            };
            let Some(attempt) = window
                .attempts
                .iter()
                .find(|attempt| attempt.attempt_no == attempt_no)
            else {
                return Err(invalid("teaching attempt not claimed at this number"));
            };
            match attempt.lifecycle {
                ScTeachingAttemptLifecycle::Claimed => {}
                ScTeachingAttemptLifecycle::LaunchUnknown => return Ok(stored.clone()),
                ScTeachingAttemptLifecycle::ActualStarted
                | ScTeachingAttemptLifecycle::Terminal { .. } => {
                    return Err(invalid("started or terminal attempt cannot go unknown"));
                }
            }
            let window = stored
                .sc_generation_teaching_windows
                .iter_mut()
                .find(|window| window.generation_seq == generation_seq)
                .expect("window located above");
            let attempt = window
                .attempts
                .iter_mut()
                .find(|attempt| attempt.attempt_no == attempt_no)
                .expect("attempt located above");
            attempt.lifecycle = ScTeachingAttemptLifecycle::LaunchUnknown;
            stored.updated_at = Utc::now().to_rfc3339();
            write_json(session_path, stored)?;
            Ok(stored.clone())
        })
    }
    /// 漂移留痕（锁内 CAS）：同事务追加一条 PolicyDiagnostic（code
    /// `sc_teaching_frozen_input_drift`）——冻结输入漂移是显式系统故障面，
    /// 「零额外调用」的决策必须可审计（REQ-WSC-28E，Task 6 handler 面）。
    pub fn record_sc_teaching_frozen_input_drift(
        &self,
        expected: &WorkspaceSessionRecord,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        self.with_sc_teaching_session(expected, |session_path, stored| {
            stored.policy_diagnostics.push(PolicyDiagnostic {
                code: SC_TEACHING_FROZEN_INPUT_DRIFT_CODE.to_string(),
                message: format!(
                    "SC 教学窗口冻结输入漂移：当前失败事实与窗口冻结摘要不一致\
                     （会话 {}，规则版本 {SC_TEACHING_APPROVAL_RULE_VERSION}）",
                    stored.id
                ),
                field: None,
            });
            stored.updated_at = Utc::now().to_rfc3339();
            write_json(session_path, stored)?;
            Ok(stored.clone())
        })
    }

    /// 关闭（锁内 CAS）：attempt → Terminal{outcome}；窗口 → Closed{reason}
    /// （对齐映射 Delivered→Delivered、FailedTerminal→Exhausted、Stopped→
    /// Stopped）；关窗前设防（Task 3 评审 Minor-1 加固）：存在**晚于**当前
    /// attempt 的在飞（Claimed/LaunchUnknown）后继 attempt ⇒ 可判定拒绝
    /// （防关窗后后继启动证据无处落账）。写 non_teaching_total_at_close
    /// 快照（供下一条链做 base，教学启动不占非教学计数）；**不删除**窗口
    /// （保留关闭与消耗事实，REQ-SG-05/REQ-WSC-28B）。同 outcome 且已关闭 ⇒
    /// 幂等返回不写盘；不同 outcome 的终态 ⇒ 可判定拒绝。
    pub fn close_sc_teaching_window(
        &self,
        expected: &WorkspaceSessionRecord,
        generation_seq: u32,
        attempt_no: u8,
        outcome: ScTeachingAttemptOutcome,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        self.with_sc_teaching_session(expected, |session_path, stored| {
            let invalid = |reason: &str| ProductStoreError::InvalidRecord {
                kind: "sc_teaching_close",
                reason: reason.to_string(),
            };
            let Some(window) = stored
                .sc_generation_teaching_windows
                .iter()
                .find(|window| window.generation_seq == generation_seq)
            else {
                return Err(invalid("teaching window for the generation not found"));
            };
            let Some(attempt) = window
                .attempts
                .iter()
                .find(|attempt| attempt.attempt_no == attempt_no)
            else {
                return Err(invalid("teaching attempt not claimed at this number"));
            };
            if let ScTeachingAttemptLifecycle::Terminal { outcome: settled } = attempt.lifecycle {
                if settled != outcome {
                    return Err(invalid(
                        "teaching attempt already terminal with a different outcome",
                    ));
                }
                if matches!(window.status, ScTeachingWindowStatus::Closed { .. }) {
                    return Ok(stored.clone());
                }
            }
            if window.attempts.iter().any(|later| {
                later.attempt_no > attempt_no
                    && matches!(
                        later.lifecycle,
                        ScTeachingAttemptLifecycle::Claimed | ScTeachingAttemptLifecycle::LaunchUnknown
                    )
            }) {
                return Err(invalid(
                    "teaching window has a later in-flight attempt; close the current attempt first",
                ));
            }
            // 先取快照（&Record）再进入可变改写，避免整记录借用冲突。
            let non_teaching_total = non_teaching_provider_start_count(stored);
            let reason = match outcome {
                ScTeachingAttemptOutcome::Delivered => ScTeachingWindowCloseReason::Delivered,
                ScTeachingAttemptOutcome::FailedTerminal => ScTeachingWindowCloseReason::Exhausted,
                ScTeachingAttemptOutcome::Stopped => ScTeachingWindowCloseReason::Stopped,
            };
            let window = stored
                .sc_generation_teaching_windows
                .iter_mut()
                .find(|window| window.generation_seq == generation_seq)
                .expect("window located above");
            let attempt = window
                .attempts
                .iter_mut()
                .find(|attempt| attempt.attempt_no == attempt_no)
                .expect("attempt located above");
            attempt.lifecycle = ScTeachingAttemptLifecycle::Terminal { outcome };
            window.status = ScTeachingWindowStatus::Closed { reason };
            window.non_teaching_total_at_close = Some(non_teaching_total);
            stored.updated_at = Utc::now().to_rfc3339();
            write_json(session_path, stored)?;
            Ok(stored.clone())
        })
    }
}

#[cfg(test)]
#[path = "workspace_sc_teaching_window_tests.rs"]
mod tests;
