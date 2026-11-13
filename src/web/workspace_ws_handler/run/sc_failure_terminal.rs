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
use super::super::sc_teaching_source::{ScTeachingSourceInput, resolve_sc_teaching_source};
use super::SingleCandidateProviderRunError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::lifecycle_store::sc_failure_evidence::{
    ScFailureEvidenceError, ScFailureEvidenceEvent, ScFailureEvidenceRead,
    sc_failure_evidence_log_ref,
};
use crate::product::lifecycle_store::sc_failure_publication::{ScLocalTerminal, ScTerminalInput};
use crate::product::work_item_plan_policy::ScTeachingAttemptRef;
use crate::product::workspace_engine::WorkspaceEngine;
use crate::product::workspace_engine::build_sc_failure_repair_package;
use crate::web::workspace_ws_types::{
    ScEvidenceRef, ScFailureChainIdentity, ScMissingEvidence, ScPackageCompleteness,
    ScRoundIdentity, ScRoundLaunchState,
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
const SC_TERMINAL_PHASE_UNCONFIRMED_CODE: &str = "sc_failure_terminal_phase_unconfirmed";

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
        Ok(outcome) => {
            terminal_outcome_error(engine, lifecycle, chain, outcome, original_message).await
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

/// 交包前的 source 真实核验：每个失败轮都用 P20 输入（该轮真实
/// class/raw/source）读回失败源；读回失败**只**追加一条 `source` 缺项（同 bound
/// round），由 builder 按 `(round, kind)` 把 `Available` 投影为 `Missing`——
/// 不改 verified entries、不改证据字节摘要、不猜 launch 状态、不反填 raw。
fn source_missing_evidence(
    lifecycle: &LifecycleStore,
    chain: &ScFailureChainIdentity,
    read: &ScFailureEvidenceRead,
) -> Vec<ScMissingEvidence> {
    let mut missing = Vec::new();
    for entry in &read.entries {
        let ScFailureEvidenceEvent::FailureObserved {
            round_id,
            failure_class: Some(class),
            raw,
            source,
            ..
        } = &entry.event
        else {
            continue;
        };
        // 只有 `Available` 才需要核验；显式 `Missing` 是当时的真实事实。
        if !matches!(source, ScEvidenceRef::Available { .. }) {
            continue;
        }
        let Some(round) = read.entries.iter().find_map(|entry| match &entry.event {
            ScFailureEvidenceEvent::RoundBound { round } if &round.round_id == round_id => {
                Some(round)
            }
            _ => None,
        }) else {
            continue;
        };
        let input = ScTeachingSourceInput {
            identity: round,
            failure_class: class,
            raw,
            source,
        };
        if let Err(error) = resolve_sc_teaching_source(lifecycle, &chain.scope, &input) {
            missing.push(ScMissingEvidence {
                round_id: round_id.clone(),
                kind: "source".to_string(),
                code: error.code,
                detail: error.detail,
                locator: None,
            });
        }
    }
    missing
}

/// 从真实 durable `ChainTerminal` 行取同一终端事实（source 核验失败时改用
/// `Local`）：launch 观察来自该实际事件，绝不按包自身推断。
fn durable_terminal_facts(read: &ScFailureEvidenceRead) -> Option<ScLocalTerminal> {
    read.entries.iter().rev().find_map(|entry| match &entry.event {
        ScFailureEvidenceEvent::ChainTerminal {
            last_failure_round_id,
            last_physical_round_id,
            terminal_class,
            terminal_reason,
            launch_observations,
            completeness,
            missing_evidence,
        } => Some(ScLocalTerminal {
            last_failure_round_id: last_failure_round_id.clone(),
            last_physical_round_id: last_physical_round_id.clone(),
            terminal_class: terminal_class.clone(),
            terminal_reason: terminal_reason.clone(),
            launch_observations: launch_observations.clone(),
            completeness: completeness.clone(),
            missing_evidence: missing_evidence.clone(),
        }),
        _ => None,
    })
}

/// 终局已组（`Durable`/`Prefix`）后的唯一收口：真实 source 核验 → 真实组包 →
/// 严格发布 → 失败相位 durable 回读确认 → 才回原始失败原文。
///
/// - `Unknown` 到不了这里（`finalize` 已先返回 typed Waiting，零 publish）；
/// - 证据视图逐字使用：`Durable` 走 `ScTerminalInput::Durable`，`Prefix` 走
///   `ScTerminalInput::Local`；source 读回失败才转 `Local` + `Incomplete`（真实
///   日志里 `Available` 仍是当时事实，只是终局投影如实标缺）；
/// - 发布成功 ≠ candidate 成功：这里只闭合真实失败终局的发布与相位；
/// - 发布失败文本带 store `code`/`detail`/`publication_incomplete`，不吞原错。
async fn terminal_outcome_error(
    engine: &mut WorkspaceEngine,
    lifecycle: &LifecycleStore,
    chain: &ScFailureChainIdentity,
    outcome: ScFailureTerminalOutcome,
    original_message: String,
) -> SingleCandidateProviderRunError {
    let (read, prefix_local) = match outcome {
        ScFailureTerminalOutcome::Durable(read) => (read, None),
        ScFailureTerminalOutcome::Prefix { read, terminal } => (read, Some(terminal)),
    };
    let missing_sources = source_missing_evidence(lifecycle, chain, &read);
    let local = match prefix_local {
        // `Prefix` 已经带真实前缀终端事实：缺项追加即可。
        Some(mut facts) => {
            if !missing_sources.is_empty() {
                facts.completeness = ScPackageCompleteness::Incomplete;
                facts.missing_evidence.extend(missing_sources);
            }
            Some(facts)
        }
        // 无 source 读回失败：保持 `Durable`（不无谓改写成 Local、不白克隆终局行）。
        None if missing_sources.is_empty() => None,
        // source 读回失败：用真实 durable 终局行转 `Local` + `Incomplete`。
        None => durable_terminal_facts(&read).map(|mut facts| {
            facts.completeness = ScPackageCompleteness::Incomplete;
            facts.missing_evidence.extend(missing_sources);
            facts
        }),
    };
    let terminal_input = match &local {
        Some(facts) => ScTerminalInput::Local(facts),
        None => ScTerminalInput::Durable,
    };
    let published = match build_sc_failure_repair_package(chain, &read, terminal_input) {
        Ok(package) => engine.publish_sc_failure_repair_package(package).await,
        Err(error) => Err(error),
    };
    engine.persist_single_candidate_terminal_phase(
        crate::product::models::SingleCandidatePhase::Failed,
    );
    // 既有 void persist + 真实读回：失败码从这里准确可见，不新增 checked shim。
    let confirmed = lifecycle
        .get_workspace_session(&chain.scope.session_id)
        .map(|record| {
            matches!(
                record.single_candidate_phase,
                Some(crate::product::models::SingleCandidatePhase::Failed)
            )
        });
    match (published, confirmed) {
        (Ok(_artifact_ref), Ok(true)) => {
            SingleCandidateProviderRunError::Message(original_message)
        }
        (Ok(_artifact_ref), Ok(false)) => SingleCandidateProviderRunError::Message(format!(
            "{SC_TERMINAL_PHASE_UNCONFIRMED_CODE}: 失败相位回读不是 Failed；\
             原始失败原因：{original_message}"
        )),
        (Ok(_artifact_ref), Err(error)) => SingleCandidateProviderRunError::Message(format!(
            "{SC_TERMINAL_PHASE_UNCONFIRMED_CODE}: 失败相位回读失败（{error:?}）；\
             原始失败原因：{original_message}"
        )),
        (Err(error), _) => SingleCandidateProviderRunError::Message(format!(
            "SC 失败终局发布失败：{}: {}（publication_incomplete={}）；\
             原始失败原因：{original_message}",
            error.code, error.detail, error.publication_incomplete
        )),
    }
}