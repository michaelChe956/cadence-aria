// P11：SC 失败修复包的**纯组包器**（Plan §2.5）+ 内联行为用例。
//
// `build_sc_failure_repair_package(identity, evidence)` 把严格证据读视图折叠成
// `ScFailureRepairPackage`：零 I/O、零 store 调用、零会话查询，不写事件、不发布
// 产物（严格 publisher 属 P13）。P13 可把下方 `mod tests` 原样迁至
// `engine/tests/sc_failure_artifacts.rs`，`use super::...` 两种落点都解析到本模块。
//
// 契约要点（controller 定案，Plan §2.5）：
// 1) `package_id`：清空自身 id 后按 struct 固定字段序 `serde_json::to_vec` 紧凑
//    序列化 → SHA-256；不含写入时间、不含自身 id，不引第三方 canonicalizer。
// 2) 未交付 raw 的 `detail` 必含真实 terminal cause，其余为可读补充文案。
// 3) 非失败轮仅当无 `FailureObserved` 且未触发该轮 prompt/attempt 时
//    `NotRequested`；有失败轮缺 decision、无失败轮却挂 prompt，一律 `Err`（不猜默认）。
// 4) 可见性：`pub(crate)` 纯 fn，等 P11 publisher（同模块、随后接）消费；消费者
//    落地前非测试构建的 dead_code 提示属 integration 未完，不加 `allow`/无用 export。

use sha2::{Digest, Sha256};

use crate::product::lifecycle_store::sc_failure_evidence::{
    sc_failure_evidence_log_ref, ScFailureEvidenceError, ScFailureEvidenceEvent,
    ScFailureEvidenceRead,
};
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
const CODE_SCOPE_MISMATCH: &str = "sc_failure_scope_mismatch";
const CODE_THROUGH_MISMATCH: &str = "sc_failure_evidence_through_mismatch";
const CODE_PREFIX_MISSING: &str = "sc_failure_evidence_prefix_missing";
const CODE_ROUND_ORDER_INVALID: &str = "sc_failure_round_order_invalid";
const CODE_ROUND_NOT_BOUND: &str = "sc_failure_round_not_bound";
const CODE_PACKAGE_INVALID: &str = "sc_failure_package_invalid";
const CODE_PACKAGE_SERIALIZE: &str = "sc_failure_package_serialize";

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

/// 终局事实的借用视图（取最近一条 `ChainTerminal`）。
struct TerminalFacts<'a> {
    entry_key: &'a String,
    last_failure_round_id: &'a String,
    last_physical_round_id: &'a Option<String>,
    terminal_class: &'a String,
    terminal_reason: &'a String,
    launch_observations: &'a Vec<(String, ScRoundLaunchState)>,
    completeness: &'a ScPackageCompleteness,
    missing_evidence: &'a Vec<ScMissingEvidence>,
}

/// 纯组包：确定性、无副作用。非终局读视图 / 缺已验证前缀 / 失败轮缺显式教学
/// 决定 / 终局声明的失败轮没有 `FailureObserved` 或没有 `RoundBound` 事实，
/// 一律 `Err`，不产半成品包、不伪造 digest 与 locator。
pub(crate) fn build_sc_failure_repair_package(
    identity: &ScFailureChainIdentity,
    evidence: &ScFailureEvidenceRead,
) -> Result<ScFailureRepairPackage, ScFailureEvidenceError> {
    check_chain_scope(identity, evidence)?;
    if evidence.verified_prefix_bytes.is_empty() {
        return Err(error(
            CODE_PREFIX_MISSING,
            None,
            "读视图没有已验证前缀字节，不能伪造 evidence_digest",
        ));
    }
    let terminal = terminal_facts(evidence)?;
    if terminal.entry_key.as_str() != evidence.evidence_through_entry_key.as_str() {
        return Err(error(
            CODE_THROUGH_MISMATCH,
            None,
            format!(
                "读视图游标 {} 不是终局条目 {}，不能把非终局事实当包边界",
                evidence.evidence_through_entry_key, terminal.entry_key
            ),
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
        let (failure_class, blocking_reasons, repair_instructions, raw, source) = match &failure {
            Some(facts) => (
                facts.failure_class.clone(),
                facts.blocking_reasons.clone(),
                facts.repair_instructions.clone(),
                facts.raw.clone(),
                facts.source.clone(),
            ),
            None => (
                None,
                Vec::new(),
                Vec::new(),
                not_delivered(terminal.terminal_reason, round.round_id.as_str()),
                not_delivered(terminal.terminal_reason, round.round_id.as_str()),
            ),
        };
        let round_evidence = ScRoundEvidence {
            identity: round.clone(),
            launch_state: round_launch_state(terminal.launch_observations, round.round_id.as_str()),
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
    let last_provider_output = match &terminal.last_physical_round_id {
        Some(round_id) => match failure_for(evidence, round_id.as_str()) {
            Some(facts) => facts.raw.clone(),
            None => not_delivered(terminal.terminal_reason, round_id.as_str()),
        },
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

fn terminal_facts(evidence: &ScFailureEvidenceRead) -> Result<TerminalFacts<'_>, ScFailureEvidenceError> {
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
                entry_key: &entry.entry_key,
                last_failure_round_id,
                last_physical_round_id,
                terminal_class,
                terminal_reason,
                launch_observations,
                completeness,
                missing_evidence,
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

fn round_launch_state(
    observations: &[(String, ScRoundLaunchState)],
    round_id: &str,
) -> ScRoundLaunchState {
    observations
        .iter()
        .find(|(observed_id, _)| observed_id == round_id)
        .map(|(_, state)| state.clone())
        .unwrap_or(ScRoundLaunchState::Unknown)
}

fn not_delivered(terminal_reason: &str, round_id: &str) -> ScEvidenceRef {
    ScEvidenceRef::Missing {
        code: CODE_OUTPUT_NOT_DELIVERED.into(),
        detail: format!("{terminal_reason}（{round_id} 已启动但未交付 provider 输出原文）"),
        round_id: round_id.into(),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
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
mod tests {
    use super::build_sc_failure_repair_package;
    use crate::product::lifecycle_store::sc_failure_evidence::{
        sc_failure_evidence_log_ref, ScFailureEvidenceEntry, ScFailureEvidenceEvent,
        ScFailureEvidenceRead,
    };
    use crate::product::models::ProviderName;
    use crate::web::workspace_ws_types::{
        ScEvidenceRef, ScFailureChainIdentity, ScFailureClass, ScFailureScope, ScMissingEvidence,
        ScPackageCompleteness, ScPromptDisposition, ScRoundIdentity, ScRoundLaunchState,
        ScTeachingDecision, ScTeachingPromptEvidence,
    };
    use sha2::{Digest, Sha256};

    const SESSION: &str = "ws-sc-p11";
    const CHAIN: &str = "chain-v1-p11";
    const RAW1: &str = "provider raw 1：{\"plan\":\"候选一\"}\n第二行 tab\t quotes \"x\"";
    const SOURCE1: &str = "normalized author delivery source 1（去掉 preamble 后）";
    const IR_REASON: &str = "IR preflight rejected: dependency cycle（保留原机械失败措辞）";
    const INTERRUPTED_REASON: &str = "教学中断：第二轮 provider 调用被 operational 错误终止";

    fn sha_hex(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new(); hasher.update(bytes);
        hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn available(locator: &str, body: &str) -> ScEvidenceRef {
        ScEvidenceRef::Available { locator: locator.into(), sha256: sha_hex(body.as_bytes()), bytes: body.as_bytes().len() as u64 }
    }

    fn split_locator(seq: u64) -> String {
        format!("sc-split-output/v1/{SESSION}/{seq}")
    }

    fn chain() -> ScFailureChainIdentity {
        let scope = ScFailureScope { session_id: SESSION.into(), project_id: "proj-p11".into(), issue_id: "issue-p11".into(), plan_id: "plan-p11".into() };
        ScFailureChainIdentity { scope, chain_id: CHAIN.into(), business_key: "biz-p11".into(), original_node_id: "node-original".into(), original_role_run_seq: 10 }
    }

    fn round(ordinal: u32, node: &str, seq: u64) -> ScRoundIdentity {
        let round_id = format!("round-{ordinal}");
        ScRoundIdentity { chain_id: CHAIN.into(), round_id, node_id: node.into(), round_ordinal: ordinal, provider: ProviderName::Fake, workspace_session_id: SESSION.into(), role_run_seq: seq, run_ref: format!("single-candidate/runs/{seq}") }
    }

    fn entry(seq: u64, key: String, chain: &ScFailureChainIdentity, event: ScFailureEvidenceEvent) -> ScFailureEvidenceEntry {
        ScFailureEvidenceEntry { schema_version: 1, entry_seq: seq, entry_key: key, chain: chain.clone(), event }
    }

    fn read(entries: Vec<ScFailureEvidenceEntry>, prefix: Vec<u8>) -> ScFailureEvidenceRead {
        ScFailureEvidenceRead { entries, verified_prefix_bytes: prefix, evidence_through_entry_key: "terminal".into(), missing_evidence: Vec::new() }
    }

    fn bound(seq: u64, chain: &ScFailureChainIdentity, r: &ScRoundIdentity) -> ScFailureEvidenceEntry {
        entry(seq, format!("round/{}", r.round_id), chain, ScFailureEvidenceEvent::RoundBound { round: r.clone() })
    }

    fn failure(seq: u64, chain: &ScFailureChainIdentity, r: &ScRoundIdentity, class: ScFailureClass, reasons: Vec<String>, instructions: Vec<String>, raw: ScEvidenceRef, source: ScEvidenceRef) -> ScFailureEvidenceEntry {
        entry(seq, format!("failureobserved/{}", r.round_id), chain, ScFailureEvidenceEvent::FailureObserved {
            round_id: r.round_id.clone(), failure_class: Some(class), blocking_reasons: reasons, repair_instructions: instructions, raw, source, existing_failure_id: None, attempt_key: None,
        })
    }

    fn decision(seq: u64, chain: &ScFailureChainIdentity, failed: &ScRoundIdentity, decision: ScTeachingDecision) -> ScFailureEvidenceEntry {
        entry(seq, format!("teachingdecision/{}", failed.round_id), chain, ScFailureEvidenceEvent::TeachingDecision { failed_round_id: failed.round_id.clone(), decision })
    }

    fn sent_prompt(r: &ScRoundIdentity, target_ordinal: u32) -> ScTeachingPromptEvidence {
        ScTeachingPromptEvidence {
            failed_round_id: r.round_id.clone(), target_round_ordinal: target_ordinal, builder_id: "sc-teach-compile-v1".into(),
            prompt: available("sc-teaching-prompt/v1/ws-sc-p11/chain-v1-p11/2/p", "教学提示全文"), disposition: ScPromptDisposition::Sent,
        }
    }

    fn terminal(seq: u64, chain: &ScFailureChainIdentity, last_failure: &ScRoundIdentity, last_physical: &ScRoundIdentity, class: &str, reason: &str, launches: Vec<(String, ScRoundLaunchState)>, completeness: ScPackageCompleteness, missing: Vec<ScMissingEvidence>) -> ScFailureEvidenceEntry {
        entry(seq, "terminal".into(), chain, ScFailureEvidenceEvent::ChainTerminal {
            last_failure_round_id: last_failure.round_id.clone(), last_physical_round_id: Some(last_physical.round_id.clone()), terminal_class: class.into(), terminal_reason: reason.into(), launch_observations: launches, completeness, missing_evidence: missing,
        })
    }

    fn started(r: &ScRoundIdentity) -> (String, ScRoundLaunchState) {
        (r.round_id.clone(), ScRoundLaunchState::Started)
    }

    fn complete_prefix() -> Vec<u8> {
        "{\"entry_seq\":2,\"kind\":\"failure_observed\",\"reasons\":[\"E0308:3: mismatched types\",\"E0425:9: not found\"]}\n{\"entry_seq\":8,\"kind\":\"chain_terminal\",\"terminal_class\":\"ir_preflight\"}\n中文尾部\n".as_bytes().to_vec()
    }

    fn interrupted_prefix() -> Vec<u8> {
        "{\"entry_seq\":2,\"kind\":\"failure_observed\",\"reasons\":[\"E0308:3: mismatched types\"]}\n{\"entry_seq\":6,\"kind\":\"chain_terminal\",\"terminal_class\":\"teaching_interrupted\"}\n中断前缀\n".as_bytes().to_vec()
    }

    fn one_round_prefix() -> Vec<u8> {
        "{\"entry_seq\":1,\"kind\":\"round_bound\"}\n{\"entry_seq\":2,\"kind\":\"failure_observed\"}\n{\"entry_seq\":4,\"kind\":\"chain_terminal\"}\n".as_bytes().to_vec()
    }

    fn one_round_read(reason: &str, prefix: Vec<u8>) -> ScFailureEvidenceRead {
        let chain = chain();
        let r = round(1, "node-compile", 11);
        let locator = split_locator(11);
        read(vec![
            bound(1, &chain, &r),
            failure(2, &chain, &r, ScFailureClass::Compile, vec![reason.to_string()], vec!["fix the compile error".into()], available(&locator, RAW1), available(&locator, SOURCE1)),
            decision(3, &chain, &r, ScTeachingDecision::NotRequested),
            terminal(4, &chain, &r, &r, "compile", reason, vec![started(&r)], ScPackageCompleteness::Complete, Vec::new()),
        ], prefix)
    }

    #[test]
    fn complete_terminal_builds_ordered_rounds_and_last_physical_raw() {
        let chain = chain();
        let (r1, r2) = (round(1, "node-compile", 11), round(2, "node-ir", 12));
        let raw1 = available(&split_locator(11), RAW1);
        let src1 = available(&split_locator(11), SOURCE1);
        let raw2 = available(&split_locator(12), "IR provider 原始输出二");
        let src2 = available(&split_locator(12), "IR 归一化 source 2");
        let reasons1 = vec!["E0308:3: mismatched types".to_string(), "E0425:9: not found".to_string()];
        let instructions1 = vec!["fix the borrow".to_string(), "rename the binding".to_string()];
        let reasons2 = vec!["ir_preflight: dependency cycle".to_string()];
        let prompt = sent_prompt(&r1, 2);
        let entries = vec![
            bound(1, &chain, &r1),
            failure(2, &chain, &r1, ScFailureClass::Compile, reasons1.clone(), instructions1.clone(), raw1.clone(), src1.clone()),
            entry(3, format!("prompt/{}/2/sent", r1.round_id), &chain, ScFailureEvidenceEvent::PromptPrepared { evidence: prompt.clone() }),
            decision(4, &chain, &r1, ScTeachingDecision::Granted),
            bound(5, &chain, &r2),
            failure(6, &chain, &r2, ScFailureClass::IrPreflight, reasons2.clone(), Vec::new(), raw2.clone(), src2.clone()),
            decision(7, &chain, &r2, ScTeachingDecision::NotRequested),
            terminal(8, &chain, &r2, &r2, "ir_preflight", IR_REASON, vec![started(&r1), started(&r2)], ScPackageCompleteness::Complete, Vec::new()),
        ];
        let prefix = complete_prefix();
        let package = build_sc_failure_repair_package(&chain, &read(entries, prefix.clone())).expect("complete terminal evidence must build a package");

        assert_eq!(package.schema_version, 1);
        assert_eq!((package.chain_id.as_str(), package.session_id.as_str(), package.project_id.as_str(), package.issue_id.as_str(), package.plan_id.as_str()), (CHAIN, SESSION, "proj-p11", "issue-p11", "plan-p11"));
        assert_eq!(package.terminal_class, "ir_preflight");
        assert_eq!(package.terminal_reason, IR_REASON);
        assert_eq!(package.completeness, ScPackageCompleteness::Complete);
        assert!(package.missing_evidence.is_empty());
        assert_eq!(package.rounds.len(), 2);
        assert_eq!(package.rounds[0].identity, r1);
        assert_eq!(package.rounds[1].identity, r2);
        assert_eq!(package.rounds[0].launch_state, ScRoundLaunchState::Started);
        assert_eq!(package.rounds[1].launch_state, ScRoundLaunchState::Started);
        assert_eq!(package.rounds[0].failure_class, Some(ScFailureClass::Compile));
        assert_eq!(package.rounds[0].blocking_reasons, reasons1);
        assert_eq!(package.rounds[0].repair_instructions, instructions1);
        assert_eq!(package.rounds[0].raw, raw1);
        assert_eq!(package.rounds[0].source, src1);
        assert_eq!(package.rounds[0].teaching_decision, ScTeachingDecision::Granted);
        assert_eq!(package.rounds[0].prompts, vec![prompt.clone()]);
        assert_eq!(package.rounds[1].failure_class, Some(ScFailureClass::IrPreflight));
        assert_eq!(package.rounds[1].blocking_reasons, reasons2);
        assert_eq!(package.rounds[1].raw, raw2);
        assert_eq!(package.rounds[1].source, src2);
        assert_eq!(package.rounds[1].teaching_decision, ScTeachingDecision::NotRequested);
        assert!(package.rounds[1].prompts.is_empty());
        assert_eq!(package.last_provider_output, raw2);
        assert_eq!(package.last_failure_source.source_round_id, r2.round_id);
        assert_eq!(package.last_failure_source.source, src2);
        assert_eq!(package.teaching_prompts, vec![prompt]);
        assert_eq!(package.evidence_through_entry_key, "terminal");
        assert_eq!(package.evidence_digest, sha_hex(&prefix));
        assert_eq!(package.evidence_log_ref, sc_failure_evidence_log_ref(&chain).expect("strict log ref"));
        assert!(package.validate().is_ok(), "{:?}", package.validate());
    }

    #[test]
    fn interrupted_second_started_keeps_round_one_as_failure_source() {
        let chain = chain();
        let (r1, r2) = (round(1, "node-compile", 11), round(2, "node-compile", 12));
        let raw1 = available(&split_locator(11), RAW1);
        let src1 = available(&split_locator(11), SOURCE1);
        let prompt = sent_prompt(&r1, 2);
        let gap = ScMissingEvidence { round_id: r2.round_id.clone(), kind: "raw".into(), code: "sc_failure_output_not_delivered".into(), detail: "第二轮已启动但未交付 raw 即被中断".into(), locator: None };
        let entries = vec![
            bound(1, &chain, &r1),
            failure(2, &chain, &r1, ScFailureClass::Compile, vec!["E0308:3: mismatched types".into()], vec!["fix the compile error".into()], raw1.clone(), src1.clone()),
            entry(3, format!("prompt/{}/2/sent", r1.round_id), &chain, ScFailureEvidenceEvent::PromptPrepared { evidence: prompt.clone() }),
            decision(4, &chain, &r1, ScTeachingDecision::Granted),
            bound(5, &chain, &r2),
            terminal(6, &chain, &r1, &r2, "teaching_interrupted", INTERRUPTED_REASON, vec![started(&r1), started(&r2)], ScPackageCompleteness::Incomplete, vec![gap.clone()]),
        ];
        let prefix = interrupted_prefix();
        let package = build_sc_failure_repair_package(&chain, &read(entries, prefix.clone())).expect("interrupted chain with observed failure must build a package");

        assert_eq!(package.terminal_class, "teaching_interrupted");
        assert_eq!(package.terminal_reason, INTERRUPTED_REASON);
        assert_eq!(package.completeness, ScPackageCompleteness::Incomplete);
        assert_eq!(package.missing_evidence, vec![gap]);
        assert_eq!(package.rounds.len(), 2);
        assert_eq!(package.rounds[0].raw, raw1);
        assert_eq!(package.rounds[0].teaching_decision, ScTeachingDecision::Granted);
        assert_eq!(package.rounds[0].prompts, vec![prompt.clone()]);
        assert_eq!(package.rounds[1].identity, r2);
        assert_eq!(package.rounds[1].launch_state, ScRoundLaunchState::Started);
        assert!(package.rounds[1].failure_class.is_none());
        match &package.rounds[1].raw {
            ScEvidenceRef::Missing { code, detail, round_id } => {
                assert_eq!(code, "sc_failure_output_not_delivered");
                assert_eq!(round_id.as_str(), r2.round_id.as_str());
                assert!(!detail.is_empty());
            }
            other => panic!("started round without delivery must stay explicitly missing: {other:?}"),
        }
        match &package.last_provider_output {
            ScEvidenceRef::Missing { code, .. } => assert_eq!(code, "sc_failure_output_not_delivered"),
            other => panic!("last physical round has no delivery, so its raw must be an explicit missing ref: {other:?}"),
        }
        assert_eq!(package.last_failure_source.source_round_id, r1.round_id);
        assert_eq!(package.last_failure_source.source, src1);
        assert_eq!(package.teaching_prompts, vec![prompt]);
        assert_eq!(package.evidence_digest, sha_hex(&prefix));
        assert_ne!(package.evidence_digest, "0".repeat(64));
        assert!(package.validate().is_ok(), "{:?}", package.validate());
    }

    #[test]
    fn equal_terminal_content_replays_same_package_id_and_reason_change_flips_it() {
        let chain = chain();
        let first = build_sc_failure_repair_package(&chain, &one_round_read("E0308:3: mismatched types", one_round_prefix())).expect("build");
        let replay = build_sc_failure_repair_package(&chain, &one_round_read("E0308:3: mismatched types", one_round_prefix())).expect("replay from independently constructed equal evidence");
        assert!(!first.package_id.is_empty(), "package id must be a real derived id, never empty placeholder");
        assert_eq!(first, replay, "same terminal content must replay an identical package");

        let changed = build_sc_failure_repair_package(&chain, &one_round_read("E0308:3: different wording", one_round_prefix())).expect("changed reason");
        assert_ne!(first.package_id, changed.package_id);
        assert_eq!(first.evidence_digest, changed.evidence_digest, "evidence digest covers the log prefix bytes, not event wording");

        let other_prefix = b"other rounds.jsonl prefix bytes".to_vec();
        let rehashed = build_sc_failure_repair_package(&chain, &one_round_read("E0308:3: mismatched types", other_prefix.clone())).expect("other prefix");
        assert_eq!(rehashed.evidence_digest, sha_hex(&other_prefix));
        assert_ne!(first.package_id, rehashed.package_id);
    }

    #[test]
    fn non_terminal_read_is_rejected_instead_of_fabricating_terminal() {
        let mut evidence = one_round_read("E0308:3: mismatched types", one_round_prefix());
        evidence.entries.pop();
        let error = build_sc_failure_repair_package(&chain(), &evidence).expect_err("non-terminal evidence must not build a terminal package");
        assert!(error.code.starts_with("sc_failure_"), "unexpected code: {}", error.code);
        assert!(!error.step.is_empty());
        assert!(!error.detail.is_empty());
    }

    #[test]
    fn missing_teaching_decision_is_rejected_instead_of_defaulted() {
        let mut evidence = one_round_read("E0308:3: mismatched types", one_round_prefix());
        evidence.entries.retain(|item| !matches!(&item.event, ScFailureEvidenceEvent::TeachingDecision { .. }));
        let error = build_sc_failure_repair_package(&chain(), &evidence).expect_err("missing decision on a failed round must not be defaulted");
        assert_eq!(error.code, "sc_failure_teaching_decision_missing");
        assert!(!error.step.is_empty());
    }
}
