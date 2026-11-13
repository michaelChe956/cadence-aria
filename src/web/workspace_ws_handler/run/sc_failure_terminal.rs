//! P10：SC 失败链的终局（ChainTerminal）接线——真实 chain 历史 → 包输入。
//!
//! 唯一事实来源是已落盘的证据日志（RoundBound / FailureObserved）；启动状态
//! 只认实参观察或该轮真实 FailureObserved，缺证据一律 Unknown（绝不默认
//! NotStarted、绝不用 RoundBound 推 start）。
//!
//! 本模块是 `sc_failure_evidence` 的子模块：P19d 在此实现真实组包与发布
//! （`Durable`/`Prefix` 两条出口），调用点只取终局出口，不在本模块外重复
//! 终局 fold。
use super::evidence_store_failure;
use super::super::sc_teaching_audit::sc_failure_step_error;
use super::SingleCandidateProviderRunError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::lifecycle_store::sc_failure_evidence::{
    ScFailureEvidenceError, ScFailureEvidenceEvent, ScFailureEvidenceRead,
    sc_failure_evidence_log_ref,
};
use crate::product::lifecycle_store::sc_failure_publication::ScLocalTerminal;
use crate::product::work_item_plan_policy::ScTeachingAttemptRef;
use crate::product::workspace_engine::WorkspaceEngine;
use crate::web::workspace_ws_types::{
    ScFailureChainIdentity, ScMissingEvidence, ScPackageCompleteness, ScRoundIdentity,
    ScRoundLaunchState,
};

/// 终局类：三类机械失败 + 教学中断（与包 `terminal_class` 同一字面量）。
pub(in crate::web::workspace_ws_handler::run) const SC_TERMINAL_CLASS_COMPILE: &str = "compile";
pub(in crate::web::workspace_ws_handler::run) const SC_TERMINAL_CLASS_IR_PREFLIGHT: &str =
    "ir_preflight";
pub(in crate::web::workspace_ws_handler::run) const SC_TERMINAL_CLASS_STRUCTURE_GATE: &str =
    "structure_gate";
pub(in crate::web::workspace_ws_handler::run) const SC_TERMINAL_CLASS_TEACHING_INTERRUPTED:
    &str = "teaching_interrupted";

const SC_TERMINAL_STEP: &str = "finalize_sc_failure_chain";
const SC_TERMINAL_UNKNOWN_CODE: &str = "sc_failure_launch_observation_unknown";
const SC_TERMINAL_OUTPUT_MISSING_CODE: &str = "sc_failure_output_not_delivered";

/// 终局结果：`Durable` = 已 append 并严格读回；`Prefix` = 日志损坏/写读失败，
/// **未 append**，只带严格前缀与本地终局事实（P19d 以真实前缀组包）。
#[derive(Debug)]
pub(in crate::web::workspace_ws_handler::run) enum ScFailureTerminalOutcome {
    Durable(ScFailureEvidenceRead),
    Prefix {
        read: ScFailureEvidenceRead,
        terminal: ScLocalTerminal,
    },
}

/// 终局 fold（规则 2-7 的同一实现，`Durable` 与 `Prefix` 共用，不写第二份）：
/// 只投影 `read` 中的真实 entries 与调用方实参观察。
fn terminal_facts(
    read: &ScFailureEvidenceRead,
    terminal_class: &str,
    original_message: &str,
    launch_observations: &[(String, ScRoundLaunchState)],
) -> Result<ScLocalTerminal, ScFailureEvidenceError> {
    let mut rounds: Vec<&ScRoundIdentity> = Vec::new();
    let mut observed: Vec<&str> = Vec::new();
    for entry in &read.entries {
        match &entry.event {
            ScFailureEvidenceEvent::RoundBound { round } => rounds.push(round),
            ScFailureEvidenceEvent::FailureObserved { round_id, .. } => {
                observed.push(round_id.as_str());
            }
            ScFailureEvidenceEvent::PromptPrepared { .. }
            | ScFailureEvidenceEvent::PromptUnavailable { .. }
            | ScFailureEvidenceEvent::TeachingDecision { .. }
            | ScFailureEvidenceEvent::ChainTerminal { .. } => {}
        }
    }
    // 契约是「仅机械失败后调用」：无 FailureObserved 是编程错误守卫，不是数据路径。
    let last_failure_round_id = observed
        .last()
        .map(|round_id| (*round_id).to_string())
        .ok_or_else(|| {
            sc_failure_step_error(
                "sc_failure_round_not_bound",
                SC_TERMINAL_STEP,
                None,
                "finalize 调用前无机械失败观察",
            )
        })?;
    let mut states: Vec<(String, ScRoundLaunchState)> = Vec::new();
    let mut missing_evidence: Vec<ScMissingEvidence> = Vec::new();
    let mut last_physical_round_id: Option<String> = None;
    for round in &rounds {
        let round_id = round.round_id.as_str();
        // 实参观察优先；未命中但该轮已交付并产出机械失败 ⇒ Started；
        // 其余（只落 RoundBound、无观察）⇒ Unknown——不合成 NotStarted。
        let state = match launch_observations
            .iter()
            .find(|(observed_round, _)| observed_round.as_str() == round_id)
            .map(|(_, state)| state.clone())
        {
            Some(state) => state,
            None if observed.contains(&round_id) => ScRoundLaunchState::Started,
            None => ScRoundLaunchState::Unknown,
        };
        if matches!(state, ScRoundLaunchState::Unknown) {
            // Unknown 不写终局、不改等待：调用方据此先返回 typed Waiting。
            return Err(sc_failure_step_error(
                SC_TERMINAL_UNKNOWN_CODE,
                SC_TERMINAL_STEP,
                Some(round_id),
                "启动观察不可确认（无实际 launch 观察，且无该轮机械失败事实）",
            ));
        }
        if matches!(state, ScRoundLaunchState::Started) {
            last_physical_round_id = Some(round_id.to_string());
            if !observed.contains(&round_id) {
                // 真实 cause 由调用方给（P10 只投影，不捏造）。
                missing_evidence.push(ScMissingEvidence {
                    round_id: round_id.to_string(),
                    kind: "provider_output".to_string(),
                    code: SC_TERMINAL_OUTPUT_MISSING_CODE.to_string(),
                    detail: original_message.to_string(),
                    locator: None,
                });
            }
        }
        states.push((round_id.to_string(), state));
    }
    let completeness = if missing_evidence.is_empty() {
        ScPackageCompleteness::Complete
    } else {
        ScPackageCompleteness::Incomplete
    };
    Ok(ScLocalTerminal {
        last_failure_round_id,
        last_physical_round_id,
        terminal_class: terminal_class.to_string(),
        terminal_reason: original_message.to_string(),
        launch_observations: states,
        completeness,
        missing_evidence,
    })
}

/// 终局事件（durable 追加用）：字段与 `ScLocalTerminal` 一一对应，取所有权零克隆。
fn terminal_event(terminal: ScLocalTerminal) -> ScFailureEvidenceEvent {
    ScFailureEvidenceEvent::ChainTerminal {
        last_failure_round_id: terminal.last_failure_round_id,
        last_physical_round_id: terminal.last_physical_round_id,
        terminal_class: terminal.terminal_class,
        terminal_reason: terminal.terminal_reason,
        launch_observations: terminal.launch_observations,
        completeness: terminal.completeness,
        missing_evidence: terminal.missing_evidence,
    }
}

/// 组终局并落盘：先严格读（坏行 ⇒ 取已验证前缀组 `Prefix`，**不 append**）→
/// fold → append → 再严格读 ⇒ `Durable`。同槽异文冲突原样上浮（既有终局成立）。
#[allow(clippy::result_large_err)] // 运行收尾的唯一 typed 出口（同既有先例）。
pub(in crate::web::workspace_ws_handler::run) fn finalize_sc_failure_chain(
    lifecycle: &LifecycleStore,
    chain: &ScFailureChainIdentity,
    terminal_class: &str,
    original_message: &str,
    launch_observations: Vec<(String, ScRoundLaunchState)>,
) -> Result<ScFailureTerminalOutcome, ScFailureEvidenceError> {
    let read = match lifecycle.read_sc_failure_evidence(chain) {
        Ok(read) => read,
        Err(mut error) => {
            // 有严格前缀 ⇒ Prefix；无前缀（目录/session 不可读）⇒ 原错上浮，
            // 没有真实前缀可作包成员，绝不合成条目、绝不改坏日志。
            let Some(prefix) = error.verified_prefix.take() else {
                return Err(error);
            };
            let mut terminal =
                terminal_facts(&prefix, terminal_class, original_message, &launch_observations)?;
            // 坏日志路径恒 Incomplete：缺项沿用 store 原 code/detail，不 hardcode 新码。
            terminal.completeness = ScPackageCompleteness::Incomplete;
            let round_id = prefix
                .entries
                .iter()
                .rev()
                .find_map(|entry| match &entry.event {
                    ScFailureEvidenceEvent::RoundBound { round } => Some(round.round_id.clone()),
                    ScFailureEvidenceEvent::FailureObserved { round_id, .. } => {
                        Some(round_id.clone())
                    }
                    _ => None,
                })
                .unwrap_or_else(|| terminal.last_failure_round_id.clone());
            terminal.missing_evidence.push(ScMissingEvidence {
                round_id,
                kind: "evidence_log".to_string(),
                code: error.code,
                detail: error.detail,
                locator: Some(sc_failure_evidence_log_ref(chain)?),
            });
            return Ok(ScFailureTerminalOutcome::Prefix {
                read: *prefix,
                terminal,
            });
        }
    };
    let terminal = terminal_facts(&read, terminal_class, original_message, &launch_observations)?;
    lifecycle.append_sc_failure_evidence(chain, &terminal_event(terminal))?;
    let read = lifecycle.read_sc_failure_evidence(chain)?;
    Ok(ScFailureTerminalOutcome::Durable(read))
}

/// 唯一终局出口：`Durable`/`Prefix` ⇒ persist Failed + Message（原始失败原文；
/// P19d 在 `Ok` 分支消费 outcome 组包/发布，调用点不动）；`Unknown` ⇒ **先于
/// persist Failed** 返回 typed Waiting（既有 CAS 收口，重入复用同一 recovery_id）。
///
/// `AlreadyFinished` 不是 Waiting（上层同属 generic Failed）：这里不再用它伪装
/// 等待，CAS 失败即真实 store 错。
#[allow(clippy::result_large_err)] // 运行收尾的唯一 typed 出口（同既有先例）。
pub(in crate::web::workspace_ws_handler::run) async fn terminal_exit_error(
    engine: &mut WorkspaceEngine,
    lifecycle: &LifecycleStore,
    chain: &ScFailureChainIdentity,
    terminal_class: &str,
    launch_observations: Vec<(String, ScRoundLaunchState)>,
    original_message: String,
) -> SingleCandidateProviderRunError {
    match finalize_sc_failure_chain(
        lifecycle,
        chain,
        terminal_class,
        &original_message,
        launch_observations,
    ) {
        Ok(_terminal) => {
            engine.persist_single_candidate_terminal_phase(
                crate::product::models::SingleCandidatePhase::Failed,
            );
            SingleCandidateProviderRunError::Message(original_message)
        }
        Err(error) if error.code == SC_TERMINAL_UNKNOWN_CODE => {
            // 未知启动观察：先返回等待，零 Failed。
            let reason_code = error.code;
            match engine
                .persist_single_candidate_author_superseded_wait(reason_code.as_str())
                .await
            {
                Ok(recovery) => SingleCandidateProviderRunError::NativeSupersededWaiting {
                    reason_code,
                    superseded_native_session_id: recovery.superseded_native_session_id.clone(),
                    recovery_id: recovery.recovery_id,
                },
                Err(message) => SingleCandidateProviderRunError::Message(format!(
                    "persist single candidate author superseded wait failed: {message}"
                )),
            }
        }
        Err(error) => evidence_store_failure(engine, &error, &original_message),
    }
}

/// 教学中断轮的真实启动观察：attempt 键命中且为 actual launch（`started` 与
/// provider/started_at 齐备）⇒ `Started`；其余（无 attempt、缺条目、会话不可读）
/// ⇒ `Unknown`——不判 `NotStarted`（本片不合成该值）。
pub(in crate::web::workspace_ws_handler::run) fn teaching_launch_observations(
    lifecycle: &LifecycleStore,
    session_id: &str,
    round: &ScRoundIdentity,
    attempt: Option<&ScTeachingAttemptRef>,
) -> Vec<(String, ScRoundLaunchState)> {
    let state = match attempt {
        None => ScRoundLaunchState::Unknown,
        Some(attempt) => match lifecycle.get_workspace_session(session_id) {
            Ok(record) => {
                let started = record.provider_start_ledger.iter().any(|entry| {
                    entry.provider_attempt_key.as_deref() == Some(attempt.attempt_key.as_str())
                        && entry.is_actual_launch_observation()
                });
                if started {
                    ScRoundLaunchState::Started
                } else {
                    ScRoundLaunchState::Unknown
                }
            }
            // 会话不可读：启动与否不可确认，不猜。
            Err(_) => ScRoundLaunchState::Unknown,
        },
    };
    vec![(round.round_id.clone(), state)]
}