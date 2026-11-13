//! P25：SC 教学重驱 prompt 的审计面（compile/IR/结构三臂共用一份实现）。
//!
//! 本模块只做两件事，都是「事实落盘 + 严格读回」：
//! - `audit_sc_teaching_prompt`：把 facade 装配出的**最终全文**（超预算也
//!   逐字写入）落成不可变 prompt 证据，并按 locator 严格读回；读回正文
//!   必须逐字等于写入全文（摘要不符即拒）——审计不成立就不进入任何窗口
//!   判定，更不 claim。
//! - `record_sc_prompt_unavailable`：缺提示（失败源/渲染失败）的真实记录
//!   ——`put` 不接受 `Unavailable`（store 明确拒绝），故走 append 的
//!   `PromptUnavailable` + `Missing` 真实原因（code/detail/round），绝不
//!   伪造「已 put」。
//!
//! 全文只装配一次、只落一次、只读一次：本模块不重建正文、不截断、不裁剪。
use super::sc_teaching_prompt::{ScPreparedTeachingPrompt, ScTeachingPromptBudgetVerdict};
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::lifecycle_store::sc_failure_evidence::{
    ScFailureEvidenceError, ScFailureEvidenceEvent,
};
use crate::web::workspace_ws_types::{
    ScEvidenceRef, ScFailureChainIdentity, ScPromptDisposition, ScRoundIdentity,
    ScTeachingPromptEvidence,
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
    ScFailureEvidenceError {
        code: code.to_string(),
        step: STEP_AUDIT.to_string(),
        round_id: Some(round_id.to_string()),
        detail: detail.to_string(),
        publication_incomplete: false,
        verified_prefix: None,
    }
}
