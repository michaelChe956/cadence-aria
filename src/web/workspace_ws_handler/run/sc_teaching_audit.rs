//! P25：SC 教学重驱的审计面（compile/IR/结构三臂共用一份实现）。
//!
//! 本模块承载两件同族的事，都是「事实落盘 + 严格读回」：
//! - 教学判定的写读（`observe_sc_teaching_decision`）：判定追加 → 严格读回
//!   → 读回行逐字核对 → 轮证据投影；缺判定即 Err，绝不默认 `NotRequested`。
//! - 教学 prompt 的审计（`audit_sc_teaching_prompt`）：把 facade 装配出的
//!   **最终全文**（超预算也逐字写入）落成不可变 prompt 证据，并按 locator
//!   严格读回；读回正文必须逐字等于写入全文（摘要不符即拒）——审计不成立
//!   就不进入任何窗口判定，更不 claim。
//! - 缺提示的真实记录（`record_sc_prompt_unavailable`）：`put` 不接受
//!   `Unavailable`（store 明确拒绝），故走 append 的 `PromptUnavailable` +
//!   `Missing` 真实原因（code/detail/round），绝不伪造「已 put」。
//!
//! 全文只装配一次、只落一次、只读一次：本模块不重建正文、不截断、不裁剪。
use super::SingleCandidateProviderRunError;
use super::sc_teaching_prompt::{ScPreparedTeachingPrompt, ScTeachingPromptBudgetVerdict};
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::lifecycle_store::sc_failure_evidence::{
    ScFailureEvidenceEntry, ScFailureEvidenceError, ScFailureEvidenceEvent, ScFailureEvidenceRead,
};
use crate::product::workspace_engine::WorkspaceEngine;
use crate::web::workspace_ws_types::{
    ScEvidenceRef, ScFailureChainIdentity, ScFailureClass, ScPromptDisposition, ScRoundEvidence,
    ScRoundIdentity, ScRoundLaunchState, ScTeachingDecision, ScTeachingPromptEvidence,
};

/// 审计面的稳定步名（错误 step 面）。
const STEP_AUDIT: &str = "audit_sc_teaching_prompt";

/// 教学轮的目标序号：失败轮的下一序号（教学轮是新物理 run，与既有
/// `begin_sc_teaching_round` 的序号推进同源）。
pub(super) fn sc_teaching_target_ordinal(round: &ScRoundIdentity) -> u32 {
    round.round_ordinal.saturating_add(1)
}

/// 完整 prompt 的审计落盘 + 严格读回（唯一实现，三臂共用）。
///
/// 处置由预算裁决决定：预算内 ⇒ `Prepared`；超限 ⇒ `RejectedBudget`
/// （**不是** `Sent`——未发送的 prompt 永不冒充已发送）。
#[allow(clippy::result_large_err)] // 证据面错误沿既有 typed 出口上浮。
pub(super) fn audit_sc_teaching_prompt(
    lifecycle: &LifecycleStore,
    chain: &ScFailureChainIdentity,
    failed_round_id: &str,
    target_round_ordinal: u32,
    prepared: &ScPreparedTeachingPrompt,
) -> Result<ScTeachingPromptEvidence, ScFailureEvidenceError> {
    let disposition = match prepared.budget_verdict {
        ScTeachingPromptBudgetVerdict::Allowed => ScPromptDisposition::Prepared,
        ScTeachingPromptBudgetVerdict::RejectedBudget { .. } => ScPromptDisposition::RejectedBudget,
    };
    let evidence = lifecycle.put_sc_teaching_prompt(
        chain,
        failed_round_id,
        target_round_ordinal,
        &prepared.builder_id,
        &prepared.text,
        disposition,
    )?;
    let ScEvidenceRef::Available { locator, .. } = &evidence.prompt else {
        return Err(audit_error(
            "sc_failure_prompt_ref_missing",
            failed_round_id,
            "教学提示写入未产出可用 locator",
        ));
    };
    // 严格读回：正文必须逐字等于写入全文（store 已核 locator 摘要；此处再
    // 核正文本身——审计不成立就不进入窗口判定）。
    let body = lifecycle.read_sc_teaching_prompt(chain, locator)?;
    if body != prepared.text {
        return Err(audit_error(
            "sc_failure_prompt_digest_mismatch",
            failed_round_id,
            "教学提示严格读回正文与写入全文不一致",
        ));
    }
    Ok(evidence)
}

/// 缺提示的真实记录（append `PromptUnavailable` + `Missing` 真实原因）。
///
/// `put` 的 `Unavailable` 处置被 store 明确拒绝，故缺提示只能以事件形式
/// 追加；追加后严格读回核对（不拿内存副本冒充 durable 行）。
#[allow(clippy::result_large_err)] // 证据面错误沿既有 typed 出口上浮。
pub(super) fn record_sc_prompt_unavailable(
    lifecycle: &LifecycleStore,
    chain: &ScFailureChainIdentity,
    failed_round_id: &str,
    target_round_ordinal: u32,
    builder_id: &str,
    code: &str,
    detail: &str,
) -> Result<(), ScFailureEvidenceError> {
    let evidence = ScTeachingPromptEvidence {
        failed_round_id: failed_round_id.to_string(),
        target_round_ordinal,
        builder_id: builder_id.to_string(),
        prompt: ScEvidenceRef::Missing {
            code: code.to_string(),
            detail: detail.to_string(),
            round_id: failed_round_id.to_string(),
        },
        disposition: ScPromptDisposition::Unavailable,
    };
    let event = ScFailureEvidenceEvent::PromptUnavailable { evidence };
    lifecycle.append_sc_failure_evidence(chain, &event)?;
    let read = lifecycle.read_sc_failure_evidence(chain)?;
    if !read.entries.iter().any(|entry| entry.event == event) {
        return Err(audit_error(
            "sc_failure_prompt_unavailable_missing",
            failed_round_id,
            "缺提示记录未出现在严格读回中",
        ));
    }
    Ok(())
}

/// 审计面稳定错误的统一构造（字段全 pub，与既有 `sc_failure_step_error`
/// 同形；不新增码、不吞错）。
fn audit_error(code: &str, round_id: &str, detail: &str) -> ScFailureEvidenceError {
    sc_failure_step_error(code, STEP_AUDIT, Some(round_id), detail)
}

/// 一轮失败的事实集合：全部来自本轮真实实参（不做内存 last-valid 回退）。
#[derive(Debug)]
pub(super) struct ScFailureFacts {
    pub failure_class: ScFailureClass,
    pub blocking_reasons: Vec<String>,
    pub repair_instructions: Vec<String>,
    pub raw: ScEvidenceRef,
    pub source: ScEvidenceRef,
    pub existing_failure_id: Option<String>,
    pub attempt_key: Option<String>,
}

/// 观察结果：只到「本轮 FailureObserved 已真实写读」；判定形成前不产出轮证据，
/// 也不携带任何默认 `teaching_decision`。
#[derive(Debug)]
pub(super) struct ScObservedFailure {
    pub facts: ScFailureFacts,
    pub round: ScRoundIdentity,
    /// 严格读回的本轮 FailureObserved 行（持久化证明，不是内存副本）。
    pub entry: ScFailureEvidenceEntry,
}

/// TeachingDecision 追加 → 严格读回 → 仅在此时投影完整轮证据（缺 decision
/// 即 Err，绝不默认 `NotRequested`）。
pub(super) fn observe_sc_teaching_decision(
    lifecycle: &LifecycleStore,
    chain: &ScFailureChainIdentity,
    observed: &ScObservedFailure,
    decision: ScTeachingDecision,
) -> Result<ScRoundEvidence, ScFailureEvidenceError> {
    const STEP: &str = "observe_sc_teaching_decision";
    let failed_round_id = observed.round.round_id.clone();
    let event = ScFailureEvidenceEvent::TeachingDecision {
        failed_round_id: failed_round_id.clone(),
        decision,
    };
    lifecycle.append_sc_failure_evidence(chain, &event)?;
    let read = lifecycle.read_sc_failure_evidence(chain)?;
    let key = teaching_decision_key(&failed_round_id);
    let entry = read
        .entries
        .iter()
        .find(|entry| entry.entry_key == key)
        .ok_or_else(|| {
            sc_failure_step_error(
                "sc_failure_teaching_decision_missing",
                STEP,
                Some(failed_round_id.as_str()),
                "本轮教学判定未出现在严格读回中",
            )
        })?;
    if entry.event != event {
        return Err(sc_failure_step_error(
            "sc_failure_entry_conflict",
            STEP,
            Some(failed_round_id.as_str()),
            "本轮教学判定严格读回与写入正文不一致",
        ));
    }
    project_sc_round_evidence(&read, observed)
}

/// 完整轮证据投影：只在本轮 FailureObserved 与显式 decision 都已 durable
/// 之后调用；缺任一项都 Err（不猜默认值）。launch_state 取「该轮确已交付
/// 输出并产出机械失败」的事实。
fn project_sc_round_evidence(
    read: &ScFailureEvidenceRead,
    observed: &ScObservedFailure,
) -> Result<ScRoundEvidence, ScFailureEvidenceError> {
    const STEP: &str = "project_sc_round_evidence";
    let round_id = observed.round.round_id.clone();
    let observed_key = failure_observed_key(&round_id);
    let entry = read
        .entries
        .iter()
        .find(|entry| entry.entry_key == observed_key)
        .ok_or_else(|| {
            sc_failure_step_error(
                "sc_failure_observed_readback_missing",
                STEP,
                Some(round_id.as_str()),
                "轮证据投影缺少本轮 FailureObserved 行",
            )
        })?;
    // 读回行必须就是观察行（它在 `observe_sc_failure` 中已逐字校验过写入事件），
    // 否则投影不成立——不拿内存事实冒充 durable 行。
    if entry.event != observed.entry.event {
        return Err(sc_failure_step_error(
            "sc_failure_entry_conflict",
            STEP,
            Some(round_id.as_str()),
            "本轮 FailureObserved 读回行与观察行不一致",
        ));
    }
    let prompts = read
        .entries
        .iter()
        .filter_map(|entry| match &entry.event {
            ScFailureEvidenceEvent::PromptPrepared { evidence }
            | ScFailureEvidenceEvent::PromptUnavailable { evidence }
                if evidence.failed_round_id == round_id =>
            {
                Some(evidence.clone())
            }
            _ => None,
        })
        .collect::<Vec<ScTeachingPromptEvidence>>();
    let decision = read
        .entries
        .iter()
        .rev()
        .find_map(|entry| match &entry.event {
            ScFailureEvidenceEvent::TeachingDecision {
                failed_round_id,
                decision,
            } if *failed_round_id == round_id => Some(decision.clone()),
            _ => None,
        })
        .ok_or_else(|| {
            sc_failure_step_error(
                "sc_failure_teaching_decision_missing",
                STEP,
                Some(round_id.as_str()),
                "轮证据投影缺少显式教学判定，绝不写默认值",
            )
        })?;
    Ok(ScRoundEvidence {
        identity: observed.round.clone(),
        launch_state: ScRoundLaunchState::Started,
        failure_class: Some(observed.facts.failure_class.clone()),
        blocking_reasons: observed.facts.blocking_reasons.clone(),
        repair_instructions: observed.facts.repair_instructions.clone(),
        raw: observed.facts.raw.clone(),
        source: observed.facts.source.clone(),
        existing_failure_id: observed.facts.existing_failure_id.clone(),
        attempt_key: observed.facts.attempt_key.clone(),
        prompts,
        teaching_decision: decision,
    })
}

/// 教学链门结果的终局收口（三臂共用一份）：`Ok` 原样返回；`Err` ⇒ 交终局
/// 处理器组包（失败源/预算/审计失败时判定与缺提示记录已 durable——不只丢
/// 一条 Message 丢掉终局）。错误原因取既有 `Message` 载荷逐字（原始失败
/// 原因不丢），其余变体按 Debug 呈现（该错误类型无 Display）。
pub(super) async fn sc_teaching_terminal_grant<T>(
    engine: &mut WorkspaceEngine,
    lifecycle: &LifecycleStore,
    chain: &ScFailureChainIdentity,
    terminal_class: &str,
    outcome: Result<T, SingleCandidateProviderRunError>,
) -> Result<T, SingleCandidateProviderRunError> {
    match outcome {
        Ok(value) => Ok(value),
        Err(error) => {
            let reason = match &error {
                SingleCandidateProviderRunError::Message(message) => message.clone(),
                other => format!("{other:?}"),
            };
            let terminal = super::sc_failure_evidence::sc_failure_terminal::terminal_exit_error(
                engine,
                lifecycle,
                chain,
                terminal_class,
                Vec::new(),
                reason,
            )
            .await;
            Err(terminal)
        }
    }
}

/// FailureObserved 行的逻辑槽位键。
pub(super) fn failure_observed_key(round_id: &str) -> String {
    format!("failureobserved/{round_id}")
}

/// 教学判定行的逻辑槽位键。
pub(super) fn teaching_decision_key(round_id: &str) -> String {
    format!("teachingdecision/{round_id}")
}

/// 审计面稳定错误的构造（与既有 `sc_failure_step_error` 同形；字段全 pub，
/// 不新增码、不吞错）。
pub(super) fn sc_failure_step_error(
    code: &str,
    step: &str,
    round_id: Option<&str>,
    detail: impl Into<String>,
) -> ScFailureEvidenceError {
    ScFailureEvidenceError {
        code: code.to_string(),
        step: step.to_string(),
        round_id: round_id.map(str::to_string),
        detail: detail.into(),
        publication_incomplete: false,
        verified_prefix: None,
    }
}
