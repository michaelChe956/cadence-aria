// P11：SC 失败修复包的**纯组包器**（Plan §2.5）。
//
// `build_sc_failure_repair_package(identity, evidence, terminal_input)` 把严格证据
// 读视图折叠成 `ScFailureRepairPackage`：零 I/O、零 store 调用、零会话查询，不写
// 事件、不发布产物（严格 publisher 在同目录 `sc_failure_artifacts.rs`）。
// `Durable` 取日志里的真实 `ChainTerminal`；`Local` 取调用方对真实已验证前缀的
// 观察（腐坏日志场景），local 绝不入日志、不计前缀字节。
//
// 本模块是 `sc_failure_artifacts.rs` 的私有子模块（`#[path]` 登记）：模块本身不
// crate 全开放，只有组包入口 `build_sc_failure_repair_package` 是 crate 可见（供
// handler 侧真实 consumer 经 `crate::product::workspace_engine::…` 取用）；摘要助手
// 与共享错误码保持 `pub(super)` 私有，不导出、不加转发函数。

use sha2::{Digest, Sha256};

use crate::product::lifecycle_store::sc_failure_evidence::{
    ScFailureEvidenceError, ScFailureEvidenceEvent, ScFailureEvidenceRead,
    sc_failure_evidence_log_ref,
};
use crate::product::lifecycle_store::sc_failure_publication::{ScLocalTerminal, ScTerminalInput};
use crate::web::workspace_ws_types::{
    ScEvidenceRef, ScFailureChainIdentity, ScFailureClass, ScFailureRepairPackage,
    ScFailureSourceEvidence, ScMissingEvidence, ScPackageCompleteness, ScRoundEvidence,
    ScRoundLaunchState, ScTeachingDecision, ScTeachingPromptEvidence,
};

const STEP: &str = "build_sc_failure_repair_package";
const CODE_TERMINAL_NOT_OBSERVED: &str = "sc_failure_terminal_not_observed";
const CODE_TEACHING_DECISION_MISSING: &str = "sc_failure_teaching_decision_missing";
const CODE_FAILURE_ROUND_MISSING: &str = "sc_failure_failure_round_missing";
const CODE_ROUND_EVIDENCE_INCONSISTENT: &str = "sc_failure_round_evidence_inconsistent";
const CODE_OUTPUT_NOT_DELIVERED: &str = "sc_failure_output_not_delivered";
const CODE_RAW_MISSING: &str = "sc_failure_raw_missing";
pub(super) const CODE_SCOPE_MISMATCH: &str = "sc_failure_scope_mismatch";
const CODE_THROUGH_MISMATCH: &str = "sc_failure_evidence_through_mismatch";
const CODE_PREFIX_MISSING: &str = "sc_failure_evidence_prefix_missing";
const CODE_ROUND_ORDER_INVALID: &str = "sc_failure_round_order_invalid";
const CODE_ROUND_NOT_BOUND: &str = "sc_failure_round_not_bound";
const CODE_LAUNCH_STATE_UNKNOWN: &str = "sc_failure_launch_state_unknown";
const CODE_LOCAL_TERMINAL_INVALID: &str = "sc_failure_local_terminal_invalid";
pub(super) const CODE_PACKAGE_INVALID: &str = "sc_failure_package_invalid";
pub(super) const CODE_PACKAGE_SERIALIZE: &str = "sc_failure_package_serialize";

/// 一轮机械失败在日志里的原始事实（借用事件字段，不做任何补齐/默认）。
struct FailureFacts<'a> {
    failure_class: &'a Option<ScFailureClass>,
    blocking_reasons: &'a Vec<String>,
    repair_instructions: &'a Vec<String>,
    raw: &'a ScEvidenceRef,
    source: &'a ScEvidenceRef,
    existing_failure_id: &'a Option<String>,
    attempt_key: &'a Option<String>,
}

/// 终局事实视图（owned）：`Durable` 来自日志里的真实 `ChainTerminal`，
/// `Local` 来自调用方对真实已验证前缀的观察（local 绝不入日志、不计前缀字节）。
struct TerminalFacts {
    entry_key: String,
    last_failure_round_id: String,
    last_physical_round_id: Option<String>,
    terminal_class: String,
    terminal_reason: String,
    launch_observations: Vec<(String, ScRoundLaunchState)>,
    completeness: ScPackageCompleteness,
    missing_evidence: Vec<ScMissingEvidence>,
}

/// 纯组包：确定性、无副作用。非终局读视图 / 缺已验证前缀 / 失败轮缺显式教学
/// 决定 / 终局声明的失败轮没有 `FailureObserved` 或没有 `RoundBound` 事实，
/// 一律 `Err`，不产半成品包、不伪造 digest 与 locator。
pub(crate) fn build_sc_failure_repair_package(
    identity: &ScFailureChainIdentity,
    evidence: &ScFailureEvidenceRead,
    terminal_input: ScTerminalInput<'_>,
) -> Result<ScFailureRepairPackage, ScFailureEvidenceError> {
    check_chain_scope(identity, evidence)?;
    if evidence.verified_prefix_bytes.is_empty() {
        return Err(error(
            CODE_PREFIX_MISSING,
            None,
            "读视图没有已验证前缀字节，不能伪造 evidence_digest",
        ));
    }
    let terminal = match terminal_input {
        // Durable：行为与今天逐字一致（无终局事实即 Err；游标必须是终局条目）。
        ScTerminalInput::Durable => {
            let facts = terminal_facts(evidence)?;
            if facts.entry_key != evidence.evidence_through_entry_key {
                return Err(error(
                    CODE_THROUGH_MISMATCH,
                    None,
                    format!(
                        "读视图游标 {} 不是终局条目 {}，不能把非终局事实当包边界",
                        evidence.evidence_through_entry_key, facts.entry_key
                    ),
                ));
            }
            facts
        }
        // Local：日志只部分可读时的真实不完整包——through key 取已验证前缀末条真实
        // key，终端字段取调用方观察；rounds/prompts 只来自真实条目，local 不入条目。
        ScTerminalInput::Local(local) => local_terminal_facts(evidence, local)?,
    };
    // Plan §2.3/§3.2：`Unknown` 只表达“观察到但状态未知”，不写 ChainTerminal
    // （等待／对账不伪造终局）。组包器兜底：终局观察里出现 `Unknown` 即拒绝，
    // 绝不让它进包，也不把它降级成 NotStarted/Started。
    if let Some((round_id, _)) = terminal
        .launch_observations
        .iter()
        .find(|(_, state)| matches!(state, ScRoundLaunchState::Unknown))
    {
        return Err(error(
            CODE_LAUNCH_STATE_UNKNOWN,
            Some(round_id.clone()),
            format!("终局观察把轮 {round_id} 记为 Unknown：Unknown 不伪造终局，不能组包"),
        ));
    }

    let mut rounds: Vec<ScRoundEvidence> = Vec::new();
    let mut teaching_prompts: Vec<ScTeachingPromptEvidence> = Vec::new();
    for entry in &evidence.entries {
        let ScFailureEvidenceEvent::RoundBound { round } = &entry.event else {
            continue;
        };
        let failure = failure_for(evidence, round.round_id.as_str());
        let prompts = prompts_for(evidence, round.round_id.as_str());
        let observed_decision = teaching_decision_for(evidence, round.round_id.as_str());
        if failure.is_some() && observed_decision.is_none() {
            return Err(error(
                CODE_TEACHING_DECISION_MISSING,
                Some(round.round_id.clone()),
                format!(
                    "失败轮 {} 缺少显式教学决定事件，组包器不猜默认值",
                    round.round_id
                ),
            ));
        }
        // `NotRequested` 只覆盖“该轮没有机械失败、也没有触发任何 prompt/attempt”
        // 的真实非机械目标；挂着教学提示却没有 `FailureObserved` 属证据自相矛盾
        // （`attempt_key` 只存在于 `FailureObserved` 内，无失败轮不可能带 attempt）。
        if failure.is_none() && !prompts.is_empty() {
            return Err(error(
                CODE_ROUND_EVIDENCE_INCONSISTENT,
                Some(round.round_id.clone()),
                format!(
                    "轮 {} 没有任何 FailureObserved 事实却挂着教学提示，证据自相矛盾",
                    round.round_id
                ),
            ));
        }
        let launch_state = resolved_launch_state(
            evidence,
            &terminal.launch_observations,
            round.round_id.as_str(),
        )?;
        // 观察说“未启动”却又有真实 FailureObserved：证据自相矛盾，不掩盖。
        if failure.is_some() && launch_state != ScRoundLaunchState::Started {
            return Err(error(
                CODE_ROUND_EVIDENCE_INCONSISTENT,
                Some(round.round_id.clone()),
                format!(
                    "轮 {} 有 FailureObserved 交付但启动观察是 {:?}，证据自相矛盾",
                    round.round_id, launch_state
                ),
            ));
        }
        let (failure_class, blocking_reasons, repair_instructions, raw, source) = match &failure {
            Some(facts) => (
                facts.failure_class.clone(),
                facts.blocking_reasons.clone(),
                facts.repair_instructions.clone(),
                facts.raw.clone(),
                facts.source.clone(),
            ),
            None => {
                let undelivered = undelivered_ref(
                    &launch_state,
                    terminal.terminal_reason.as_str(),
                    round.round_id.as_str(),
                );
                (None, Vec::new(), Vec::new(), undelivered.clone(), undelivered)
            }
        };
        let round_evidence = ScRoundEvidence {
            identity: round.clone(),
            launch_state,
            failure_class,
            blocking_reasons,
            repair_instructions,
            raw,
            source,
            existing_failure_id: failure
                .as_ref()
                .and_then(|facts| facts.existing_failure_id.clone()),
            attempt_key: failure.as_ref().and_then(|facts| facts.attempt_key.clone()),
            prompts: prompts.clone(),
            teaching_decision: observed_decision.unwrap_or(ScTeachingDecision::NotRequested),
        };
        teaching_prompts.extend(prompts);
        rounds.push(round_evidence);
    }
    rounds.sort_by_key(|round| round.identity.round_ordinal);
    if rounds
        .windows(2)
        .any(|pair| pair[0].identity.round_ordinal == pair[1].identity.round_ordinal)
    {
        return Err(error(
            CODE_ROUND_ORDER_INVALID,
            None,
            "同一 round_ordinal 出现多个 RoundBound，无法确定轮次顺序",
        ));
    }
    let bound_round_ids: Vec<&str> = rounds
        .iter()
        .map(|round| round.identity.round_id.as_str())
        .collect();
    if !bound_round_ids.contains(&terminal.last_failure_round_id.as_str()) {
        return Err(error(
            CODE_ROUND_NOT_BOUND,
            Some(terminal.last_failure_round_id.clone()),
            format!(
                "终局声明的失败轮 {} 没有 RoundBound 事实",
                terminal.last_failure_round_id
            ),
        ));
    }
    if let Some(physical_round_id) = &terminal.last_physical_round_id {
        if !bound_round_ids.contains(&physical_round_id.as_str()) {
            return Err(error(
                CODE_ROUND_NOT_BOUND,
                Some(physical_round_id.clone()),
                format!("终局声明的物理轮 {physical_round_id} 没有 RoundBound 事实"),
            ));
        }
    }

    // last 物理轮 raw 与 last 失败轮 source 是两条不同来源：物理轮没有交付时留
    // 显式 missing（真实 code + 真实 terminal cause），绝不拿失败源顶替 raw。
    // 但 last physical 必须是真的 `Started`：显式 `NotStarted`（或观察缺失且无
    // 交付）说明终局选错了物理轮，直接 reject，不靠 Missing 掩盖矛盾。
    let last_provider_output = match &terminal.last_physical_round_id {
        Some(round_id) => {
            let state = resolved_launch_state(
                evidence,
                &terminal.launch_observations,
                round_id.as_str(),
            )?;
            if state != ScRoundLaunchState::Started {
                return Err(error(
                    CODE_ROUND_EVIDENCE_INCONSISTENT,
                    Some(round_id.clone()),
                    format!(
                        "终局把物理轮 {round_id} 记为 {state:?}：last physical 必须是真实 Started，不能选择未启动轮"
                    ),
                ));
            }
            match failure_for(evidence, round_id.as_str()) {
                Some(facts) => facts.raw.clone(),
                None => undelivered_ref(&state, terminal.terminal_reason.as_str(), round_id.as_str()),
            }
        }
        None => ScEvidenceRef::Missing {
            code: CODE_RAW_MISSING.into(),
            detail: format!(
                "{}（终局未观察到任何已启动的物理轮次）",
                terminal.terminal_reason
            ),
            round_id: terminal.last_failure_round_id.clone(),
        },
    };

    let last_fact = failure_for(evidence, terminal.last_failure_round_id.as_str()).ok_or_else(|| {
        error(
            CODE_FAILURE_ROUND_MISSING,
            Some(terminal.last_failure_round_id.clone()),
            format!(
                "终局声明的失败轮 {} 没有任何 FailureObserved 事实，不能伪造失败源",
                terminal.last_failure_round_id
            ),
        )
    })?;

    let mut package = ScFailureRepairPackage {
        schema_version: 1,
        package_id: String::new(),
        chain_id: identity.chain_id.clone(),
        session_id: identity.scope.session_id.clone(),
        project_id: identity.scope.project_id.clone(),
        issue_id: identity.scope.issue_id.clone(),
        plan_id: identity.scope.plan_id.clone(),
        terminal_class: terminal.terminal_class.clone(),
        terminal_reason: terminal.terminal_reason.clone(),
        completeness: terminal.completeness.clone(),
        rounds,
        last_provider_output,
        last_failure_source: ScFailureSourceEvidence {
            source_round_id: terminal.last_failure_round_id.clone(),
            source: last_fact.source.clone(),
        },
        teaching_prompts,
        evidence_log_ref: sc_failure_evidence_log_ref(identity)?,
        evidence_digest: sha256_hex(&evidence.verified_prefix_bytes),
        evidence_through_entry_key: evidence.evidence_through_entry_key.clone(),
        missing_evidence: terminal.missing_evidence.clone(),
    };
    // 包自身 id 的确定性配方：清空 `package_id` 后按 struct 固定字段序紧凑序列化
    // → SHA-256。覆盖全部内容字段（含终局措辞与轮次事实），不含写入时间/自身 id。
    let canonical = serde_json::to_vec(&package).map_err(|failure| {
        error(
            CODE_PACKAGE_SERIALIZE,
            None,
            format!("包规范序列化失败: {failure}"),
        )
    })?;
    package.package_id = sha256_hex(&canonical);
    package.validate().map_err(|detail| {
        error(
            CODE_PACKAGE_INVALID,
            None,
            format!("包自校验失败（§2.2 不变量）: {detail}"),
        )
    })?;
    Ok(package)
}

/// Local 终局：只能是真实不完整包（Incomplete + 结构化 missing）；through key 取
/// 已验证前缀末条真实 key，不伪造 entry / digest / through key。
fn local_terminal_facts(
    evidence: &ScFailureEvidenceRead,
    local: &ScLocalTerminal,
) -> Result<TerminalFacts, ScFailureEvidenceError> {
    if local.completeness != ScPackageCompleteness::Incomplete {
        return Err(error(
            CODE_LOCAL_TERMINAL_INVALID,
            None,
            "Local 终局只能是 Incomplete：真实缺证据不得包装成完整包",
        ));
    }
    if local.missing_evidence.is_empty() {
        return Err(error(
            CODE_LOCAL_TERMINAL_INVALID,
            None,
            "Local 终局必须带结构化 missing_evidence，不得空手成包",
        ));
    }
    Ok(TerminalFacts {
        entry_key: evidence.evidence_through_entry_key.clone(),
        last_failure_round_id: local.last_failure_round_id.clone(),
        last_physical_round_id: local.last_physical_round_id.clone(),
        terminal_class: local.terminal_class.clone(),
        terminal_reason: local.terminal_reason.clone(),
        launch_observations: local.launch_observations.clone(),
        completeness: local.completeness.clone(),
        missing_evidence: local.missing_evidence.clone(),
    })
}

/// 读视图必须整体属于组包身份：任一条目的链/作用域不一致即拒绝。
fn check_chain_scope(
    identity: &ScFailureChainIdentity,
    evidence: &ScFailureEvidenceRead,
) -> Result<(), ScFailureEvidenceError> {
    for entry in &evidence.entries {
        if &entry.chain != identity {
            return Err(error(
                CODE_SCOPE_MISMATCH,
                None,
                format!("条目 {} 的链/作用域与组包身份不一致", entry.entry_key),
            ));
        }
    }
    Ok(())
}

fn terminal_facts(evidence: &ScFailureEvidenceRead) -> Result<TerminalFacts, ScFailureEvidenceError> {
    evidence
        .entries
        .iter()
        .rev()
        .find_map(|entry| match &entry.event {
            ScFailureEvidenceEvent::ChainTerminal {
                last_failure_round_id,
                last_physical_round_id,
                terminal_class,
                terminal_reason,
                launch_observations,
                completeness,
                missing_evidence,
            } => Some(TerminalFacts {
                entry_key: entry.entry_key.clone(),
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
        .ok_or_else(|| {
            error(
                CODE_TERMINAL_NOT_OBSERVED,
                None,
                "证据读视图里没有 ChainTerminal 事实，不能凭空补终局",
            )
        })
}

fn failure_for<'a>(
    evidence: &'a ScFailureEvidenceRead,
    round_id: &str,
) -> Option<FailureFacts<'a>> {
    evidence
        .entries
        .iter()
        .rev()
        .find_map(|entry| match &entry.event {
            ScFailureEvidenceEvent::FailureObserved {
                round_id: observed,
                failure_class,
                blocking_reasons,
                repair_instructions,
                raw,
                source,
                existing_failure_id,
                attempt_key,
            } if observed == round_id => Some(FailureFacts {
                failure_class,
                blocking_reasons,
                repair_instructions,
                raw,
                source,
                existing_failure_id,
                attempt_key,
            }),
            _ => None,
        })
}

fn teaching_decision_for(
    evidence: &ScFailureEvidenceRead,
    failed_round_id: &str,
) -> Option<ScTeachingDecision> {
    evidence
        .entries
        .iter()
        .rev()
        .find_map(|entry| match &entry.event {
            ScFailureEvidenceEvent::TeachingDecision {
                failed_round_id: target,
                decision,
            } if target == failed_round_id => Some(decision.clone()),
            _ => None,
        })
}

fn prompts_for(
    evidence: &ScFailureEvidenceRead,
    failed_round_id: &str,
) -> Vec<ScTeachingPromptEvidence> {
    evidence
        .entries
        .iter()
        .filter_map(|entry| match &entry.event {
            ScFailureEvidenceEvent::PromptPrepared { evidence: prompt }
            | ScFailureEvidenceEvent::PromptUnavailable { evidence: prompt }
                if prompt.failed_round_id == failed_round_id =>
            {
                Some(prompt.clone())
            }
            _ => None,
        })
        .collect()
}

/// 解析一轮的真实启动状态。只认终局里的**显式观察**；观察缺失时，仅当该轮已有
/// 真实 `FailureObserved` 交付（有交付即已启动）才可推 `Started`，其余一律
/// `Err sc_failure_launch_state_unknown`——缺证据既不是“未启动”，也不能当
/// `Unknown` 混进包。
fn resolved_launch_state(
    evidence: &ScFailureEvidenceRead,
    observations: &[(String, ScRoundLaunchState)],
    round_id: &str,
) -> Result<ScRoundLaunchState, ScFailureEvidenceError> {
    match observations
        .iter()
        .find(|(observed_id, _)| observed_id == round_id)
        .map(|(_, state)| state.clone())
    {
        Some(state) => Ok(state),
        None if failure_for(evidence, round_id).is_some() => Ok(ScRoundLaunchState::Started),
        None => Err(error(
            CODE_LAUNCH_STATE_UNKNOWN,
            Some(round_id.to_string()),
            format!("终局没有轮 {round_id} 的启动观察，且该轮没有任何交付事实，启动状态未知，不能入包"),
        )),
    }
}

/// 无 `FailureObserved` 的轮如何表达“没有 raw/source”：只有真实 `Started` 才用
/// `sc_failure_output_not_delivered`（真实 terminal cause + 真实该轮）；
/// 显式 `NotStarted` 用 `sc_failure_raw_missing`——未启动就是“没有产生输出”，
/// 不能伪造启动事实。`Unknown` 已在终局入口拒绝，此臂仅穷尽（防御性）。
fn undelivered_ref(
    state: &ScRoundLaunchState,
    terminal_reason: &str,
    round_id: &str,
) -> ScEvidenceRef {
    let (code, wording) = match state {
        ScRoundLaunchState::Started => (
            CODE_OUTPUT_NOT_DELIVERED,
            format!("{terminal_reason}（{round_id} 已启动但未交付 provider 输出原文）"),
        ),
        ScRoundLaunchState::NotStarted => (
            CODE_RAW_MISSING,
            format!("{terminal_reason}（{round_id} 未启动，没有 provider 输出原文）"),
        ),
        ScRoundLaunchState::Unknown => (
            CODE_RAW_MISSING,
            format!("{terminal_reason}（{round_id} 启动状态未知，没有 provider 输出原文）"),
        ),
    };
    ScEvidenceRef::Missing {
        code: code.into(),
        detail: wording,
        round_id: round_id.into(),
    }
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn error(code: &str, round_id: Option<String>, detail: impl Into<String>) -> ScFailureEvidenceError {
    ScFailureEvidenceError {
        code: code.into(),
        step: STEP.into(),
        round_id,
        detail: detail.into(),
        publication_incomplete: false,
        verified_prefix: None,
    }
}

#[cfg(test)]
#[path = "tests/sc_failure_artifacts.rs"]
mod tests;
