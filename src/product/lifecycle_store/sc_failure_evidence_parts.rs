//! 严格证据 store 的事件校验与日志失败形状（同域私有子模块）。
//!
//! 从 sc_failure_evidence.rs 整块搬移：这两组函数同属"写前拒什么、日志坏了
//! 报什么"的校验职责；主模块已近行数上限，按职责搬移而非压行、删判定。
//! 私有子模块：`use super::*` 取同域私有项，不新增对外接口、不改 wire。

use super::*;

/// 事件准入：round 必须先 RoundBound；raw/source locator 必须绑定该 round。
impl LifecycleStore {
    pub(super) fn sc_failure_validate_event(
        &self,
        chain: &ScFailureChainIdentity,
        read: &ScFailureEvidenceRead,
        event: &ScFailureEvidenceEvent,
        step: &str,
    ) -> Result<(), ScFailureEvidenceError> {
        match event {
            ScFailureEvidenceEvent::RoundBound { round } => {
                if round.chain_id != chain.chain_id {
                    return Err(scope_mismatch_error(step, Some(round.round_id.clone()), "round 的 chain_id 与 chain 不一致"));
                }
                if round.workspace_session_id != chain.scope.session_id {
                    return Err(scope_mismatch_error(step, Some(round.round_id.clone()), "round 的 workspace_session_id 与 chain scope 不一致"));
                }
                if round.round_id.is_empty() || round.run_ref.is_empty() {
                    return Err(locator_invalid_error(step, Some(round.round_id.clone()), "round 身份不完整（round_id / run_ref）"));
                }
                Ok(())
            }
            ScFailureEvidenceEvent::FailureObserved {
                round_id, raw, source, ..
            } => {
                let bound = sc_failure_bound_round(read, round_id).ok_or_else(|| {
                    round_not_bound_error(step, Some(round_id.clone()), "该 round 尚未 RoundBound")
                })?;
                let expected = sc_failure_split_locator(
                    &bound.workspace_session_id,
                    bound.role_run_seq,
                );
                for (label, reference) in [("raw", raw), ("source", source)] {
                    if let ScEvidenceRef::Available { locator, .. } = reference
                        && locator != &expected
                    {
                        return Err(locator_invalid_error(step, Some(round_id.clone()), format!("{label} locator 与 round 绑定不符")));
                    }
                }
                Ok(())
            }
            ScFailureEvidenceEvent::PromptPrepared { evidence } => {
                sc_failure_prompt_round_bound(read, evidence, step)?;
                Ok(())
            }
            ScFailureEvidenceEvent::PromptUnavailable { evidence } => {
                sc_failure_prompt_round_bound(read, evidence, step)?;
                if matches!(evidence.prompt, ScEvidenceRef::Available { .. }) {
                    return Err(prompt_disposition_invalid_error(
                        step,
                        Some(evidence.failed_round_id.clone()),
                        "Unavailable 的教学提示必须带 Missing 真实原因，不得带 Available 正文",
                    ));
                }
                Ok(())
            }
            ScFailureEvidenceEvent::ChainTerminal { .. } => Ok(()),
            ScFailureEvidenceEvent::TeachingDecision {
                failed_round_id, ..
            } => {
                // 决策必须针对真实机械失败轮：日志里须已有该 round 的 FailureObserved
                // （FailureObserved 本身要求 round 已绑定，故"绑定 + 观察到失败"一并成立）。
                if failed_round_id.is_empty() {
                    return Err(locator_invalid_error(
                        step,
                        Some(failed_round_id.clone()),
                        "教学决定的 failed_round_id 不能为空",
                    ));
                }
                if !sc_failure_observed_round(read, failed_round_id) {
                    return Err(round_not_bound_error(
                        step,
                        Some(failed_round_id.clone()),
                        "教学决定必须针对已观察到的机械失败轮",
                    ));
                }
                Ok(())
            }
        }
    }
}

/// 教学提示的失败 round 必须已绑定（PromptPrepared / PromptUnavailable 共用）。
pub(super) fn sc_failure_prompt_round_bound(
    read: &ScFailureEvidenceRead,
    evidence: &ScTeachingPromptEvidence,
    step: &str,
) -> Result<(), ScFailureEvidenceError> {
    if sc_failure_bound_round(read, &evidence.failed_round_id).is_none() {
        return Err(round_not_bound_error(
            step,
            Some(evidence.failed_round_id.clone()),
            "教学提示的失败 round 尚未 RoundBound",
        ));
    }
    Ok(())
}

/// 该 round 是否已有 FailureObserved 日志条目（教学决定必须针对真实机械失败轮）。
fn sc_failure_observed_round(read: &ScFailureEvidenceRead, round_id: &str) -> bool {
    read.entries.iter().any(|entry| {
        matches!(
            &entry.event,
            ScFailureEvidenceEvent::FailureObserved { round_id: observed, .. }
                if observed.as_str() == round_id
        )
    })
}

/// 日志类错误的统一形状：带上已验证前缀（坏行/断号之后的行不被跳行吞下）与具体
/// missing 项——round 不可知时不伪 round（"unknown"），code/detail 沿用原故障。
///
/// `chain`：已校验的真实 chain → locator 给受控的内部相对 ref；`None`（坏首行、
/// 真实 chain 未知）→ locator 留 `None`，**不拿日志里的假 chain 拼 locator**。
pub(super) fn sc_failure_log_failure(
    code: &str,
    step: &str,
    chain: Option<&ScFailureChainIdentity>,
    entries: &[ScFailureEvidenceEntry],
    verified_bytes: &[u8],
    detail: String,
) -> ScFailureEvidenceError {
    let mut failure = error(code, step, None, detail.clone(), true);
    failure.verified_prefix = Some(Box::new(ScFailureEvidenceRead {
        evidence_through_entry_key: entries
            .last()
            .map(|entry| entry.entry_key.clone())
            .unwrap_or_default(),
        entries: entries.to_vec(),
        verified_prefix_bytes: verified_bytes.to_vec(),
        missing_evidence: vec![ScMissingEvidence {
            round_id: "unknown".to_string(),
            kind: "evidence_log".to_string(),
            code: code.to_string(),
            detail,
            locator: chain.and_then(|identity| sc_failure_evidence_log_ref(identity).ok()),
        }],
    }));
    failure
}
