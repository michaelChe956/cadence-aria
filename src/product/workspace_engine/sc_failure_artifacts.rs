// P11：SC 失败修复包的**纯组包器**（Plan §2.5）+ 内联行为用例。
//
// `build_sc_failure_repair_package(identity, evidence)` 把严格证据读视图折叠成
// `ScFailureRepairPackage`：零 I/O、零 store 调用、零会话查询，不写事件、不发布
// 产物（严格 publisher 属 P13）。行为用例已整体迁至
// `engine/tests/sc_failure_artifacts.rs`（`#[path]` 挂载，`use super::...` 仍解析到本模块）。
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

use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::sc_failure_evidence::{
    ScFailureEvidenceError, ScFailureEvidenceEvent, ScFailureEvidenceRead,
    sc_failure_evidence_log_ref, sc_failure_split_locator,
};
use crate::product::lifecycle_store::sc_failure_publication::{
    ScLocalTerminal, ScTerminalInput,
};
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::ArtifactRef;
use crate::product::work_item_plan_source_store::{SourceStoreScope, WorkItemPlanSourceStore};
use crate::web::workspace_ws_types::{
    ArtifactPayload, ArtifactVersion, ScArtifactPublishedReceipt, ScEvidenceRef,
    ScFailureChainIdentity, ScFailureClass, ScFailureRepairPackage, ScFailureScope,
    ScFailureSourceEvidence, ScMissingEvidence, ScPackageCompleteness, ScRoundEvidence,
    ScRoundLaunchState, ScTeachingDecision, ScTeachingPromptEvidence,
};

use super::{EngineEvent, WorkspaceEngine};

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
const CODE_LAUNCH_STATE_UNKNOWN: &str = "sc_failure_launch_state_unknown";
const CODE_LOCAL_TERMINAL_INVALID: &str = "sc_failure_local_terminal_invalid";
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

// ── P11 严格 publisher（Plan §2.5）────────────────────────────────────
//
// 顺序即持久屏障：真实证据核验 → 同锁同步写版本表 → 节点 artifact_ref 写入并读回
// → 发布回执写入并过发布屏障 → 广播 ArtifactUpdate。
// 不启动 provider、不调用旧 update_artifact、不写 no-op 引用：任何一步失败都不
// 广播成功帧。

const PUBLISH_STEP: &str = "publish_sc_failure_repair_package";
const CODE_LIFECYCLE_MISSING: &str = "sc_failure_lifecycle_missing";
const CODE_ACTIVE_NODE_MISSING: &str = "sc_failure_active_node_missing";
const CODE_PACKAGE_MISMATCH: &str = "sc_failure_package_mismatch";
const CODE_PACKAGE_ID_CONFLICT: &str = "sc_failure_package_id_conflict";
const CODE_EVIDENCE_BODY_MISMATCH: &str = "sc_failure_evidence_body_mismatch";
const CODE_EVIDENCE_INVALID: &str = "sc_failure_evidence_invalid";
const CODE_VERSION_WRITE_FAILED: &str = "sc_failure_version_write_failed";
const CODE_NODE_REF_FAILED: &str = "sc_failure_node_ref_failed";
const CODE_RECEIPT_FAILED: &str = "sc_failure_receipt_failed";
const CODE_BROADCAST_FAILED: &str = "sc_failure_broadcast_failed";
const REASON_PACKAGE_ID_CONFLICT: &str = "sc_failure_package_id_conflict";

impl WorkspaceEngine {
    /// 严格发布 SC 失败修复包（A4）。
    ///
    /// 1. 真实 lifecycle + durable SC 会话作用域 + 活动节点（无未知节点兜底）；
    /// 2. 包自校验 + 真实证据重建逐字段相等（调用方载荷不可信）；
    /// 3. 证据正文核验：raw 逐字读回、教学提示逐字读回、结构 source 走既有
    ///    source store 读回；编译/IR 的归一化 source 是**调用方前置**（P19d 用
    ///    P20 resolver 在交包前核过），product 不重算归一化；
    /// 4. 同锁同步写 artifact_versions.json：同包重放复用同一 version/created_at，
    ///    异文同 id 拒绝；新版本 max+1 且 is_current=false（不动普通 current）；
    /// 5. 节点 artifact_ref 写入 + 读回同版本；
    /// 6. 发布回执写入 + 发布屏障核验；
    /// 7. 广播 ArtifactUpdate（只在回执核验之后）。
    pub(crate) async fn publish_sc_failure_repair_package(
        &mut self,
        package: ScFailureRepairPackage,
    ) -> Result<ArtifactRef, ScFailureEvidenceError> {
        let store = self.lifecycle_store.clone().ok_or_else(|| {
            publish_error(
                CODE_LIFECYCLE_MISSING,
                PUBLISH_STEP,
                None,
                "engine 没有持久 lifecycle store，不能发布 SC 失败修复包",
                false,
            )
        })?;
        let node_id = self.active_node_id.clone().ok_or_else(|| {
            publish_error(
                CODE_ACTIVE_NODE_MISSING,
                PUBLISH_STEP,
                None,
                "没有活动节点，拒绝发布 SC 失败修复包（不做未知节点兜底）",
                false,
            )
        })?;
        if self.session.session_id != package.session_id
            || self.session.project_id != package.project_id
            || self.session.issue_id != package.issue_id
            || self.session.entity_id != package.plan_id
        {
            return Err(publish_error(
                CODE_SCOPE_MISMATCH,
                PUBLISH_STEP,
                None,
                "会话作用域与包作用域不一致",
                false,
            ));
        }
        package.validate().map_err(|detail| {
            publish_error(
                CODE_PACKAGE_INVALID,
                PUBLISH_STEP,
                None,
                format!("包自校验失败（§2.2 不变量）: {detail}"),
                false,
            )
        })?;

        let scope = ScFailureScope {
            session_id: package.session_id.clone(),
            project_id: package.project_id.clone(),
            issue_id: package.issue_id.clone(),
            plan_id: package.plan_id.clone(),
        };

        // 真实证据：strict read → 纯组包器重建 → 与调用方给的包逐字段相等。
        let (identity, rebuilt) = match store
            .read_sc_failure_chain_for_publication(&scope, &package.chain_id)
        {
            Ok((identity, evidence)) => {
                let rebuilt =
                    build_sc_failure_repair_package(&identity, &evidence, ScTerminalInput::Durable)?;
                (identity, rebuilt)
            }
            Err(failure) => {
                // 腐坏日志：只有真实已验证前缀能组真实 Incomplete 包；没有前缀就
                // 原样上浮，不伪造包、不伪造 durable 终局。
                let Some(prefix) = failure.verified_prefix.as_deref() else {
                    return Err(publish_error(
                        &failure.code,
                        PUBLISH_STEP,
                        failure.round_id.clone(),
                        format!("读取真实证据失败: {}", failure.detail),
                        failure.publication_incomplete,
                    ));
                };
                let prefix = prefix.clone();
                let identity = prefix_chain_identity(&prefix, &scope, &package.chain_id)?;
                let local = local_terminal_from_package(&package);
                let rebuilt =
                    build_sc_failure_repair_package(&identity, &prefix, ScTerminalInput::Local(&local))?;
                (identity, rebuilt)
            }
        };
        if rebuilt != package {
            return Err(publish_error(
                CODE_PACKAGE_MISMATCH,
                PUBLISH_STEP,
                None,
                "调用方给的包与真实证据重建结果不一致，拒绝发布",
                false,
            ));
        }
        verify_sc_failure_package_bodies(&store, &identity, &scope, &package)?;

        let digest = sc_failure_package_digest(&package)?;
        let session_id = package.session_id.clone();
        let author_provider = self.session.author_provider;
        let package_for_write = package.clone();
        let node_for_write = node_id.clone();

        // 同锁同步闭环：查包 → 复用或 max+1 → merged save → 字节回读核验。
        let (published_row, merged) = store
            .with_artifact_versions_write(&session_id, |ctx| {
                for entry in ctx.raw() {
                    let ArtifactPayload::ScFailureRepairPackage {
                        sc_failure_repair_package,
                    } = &entry.payload
                    else {
                        continue;
                    };
                    if sc_failure_repair_package.package_id != package_for_write.package_id {
                        continue;
                    }
                    let stored_digest =
                        sc_failure_package_digest(sc_failure_repair_package).map_err(|failure| {
                            ProductStoreError::InvalidRecord {
                                kind: "artifact_versions",
                                reason: failure.detail,
                            }
                        })?;
                    if stored_digest != digest {
                        return Err(ProductStoreError::InvalidRecord {
                            kind: "artifact_versions",
                            reason: REASON_PACKAGE_ID_CONFLICT.to_string(),
                        });
                    }
                    return Ok((entry.clone(), ctx.raw().to_vec()));
                }
                let next = ctx.next_version()?;
                let proposed_row = ArtifactVersion {
                    version: next,
                    payload: ArtifactPayload::ScFailureRepairPackage {
                        sc_failure_repair_package: Box::new(package_for_write.clone()),
                    },
                    generated_by: author_provider,
                    reviewed_by: None,
                    review_verdict: None,
                    confirmed_by: None,
                    is_current: false,
                    created_at: chrono::Utc::now().to_rfc3339(),
                    source_node_id: node_for_write.clone(),
                };
                let mut proposed = ctx.raw().to_vec();
                proposed.push(proposed_row);
                let merged = ctx.save(&proposed)?;
                let bytes = ctx.read_bytes()?;
                let expected = serde_json::to_vec_pretty(&merged).map_err(|failure| {
                    ProductStoreError::Json(format!("序列化合并表失败: {failure}"))
                })?;
                if bytes != expected {
                    return Err(ProductStoreError::Json(
                        "落盘字节与合并表序列化结果不一致".to_string(),
                    ));
                }
                let parsed: Vec<ArtifactVersion> = serde_json::from_slice(&bytes).map_err(|failure| {
                    ProductStoreError::Json(format!("回读解析失败: {failure}"))
                })?;
                if parsed != merged {
                    return Err(ProductStoreError::Json("回读版本表与合并表不一致".to_string()));
                }
                let row = merged
                    .iter()
                    .find(|entry| entry.version == next)
                    .cloned()
                    .ok_or_else(|| {
                        ProductStoreError::Json("合并表缺少新版本行".to_string())
                    })?;
                let ArtifactPayload::ScFailureRepairPackage {
                    sc_failure_repair_package,
                } = &row.payload
                else {
                    return Err(ProductStoreError::Json(
                        "新版本行不是 SC 失败修复包载荷".to_string(),
                    ));
                };
                if sc_failure_repair_package.as_ref() != &package_for_write {
                    return Err(ProductStoreError::Json(
                        "落盘后的包成员与发布包不一致".to_string(),
                    ));
                }
                Ok((row, merged))
            })
            .map_err(|failure| match failure {
                ProductStoreError::InvalidRecord { reason, .. }
                    if reason == REASON_PACKAGE_ID_CONFLICT =>
                {
                    publish_error(
                        CODE_PACKAGE_ID_CONFLICT,
                        PUBLISH_STEP,
                        None,
                        "同一 package_id 已有异文版本，拒绝覆盖",
                        false,
                    )
                }
                other => publish_error(
                    CODE_VERSION_WRITE_FAILED,
                    PUBLISH_STEP,
                    None,
                    format!("写入 artifact 版本失败: {other}"),
                    false,
                ),
            })?;

        // 节点 artifact_ref：写入后读回同一版本（无未核验兜底、不写 no-op 引用）。
        let artifact_ref = ArtifactRef {
            artifact_id: package.package_id.clone(),
            version: published_row.version,
        };
        self.persist_artifact_ref(&node_id, artifact_ref.clone())
            .await
            .map_err(|detail| {
                publish_error(
                    CODE_NODE_REF_FAILED,
                    PUBLISH_STEP,
                    None,
                    format!("节点 artifact_ref 落盘失败: {detail}"),
                    true,
                )
            })?;
        let detail = store
            .load_node_detail(&session_id, &node_id)
            .map_err(|failure| {
                publish_error(
                    CODE_NODE_REF_FAILED,
                    PUBLISH_STEP,
                    None,
                    format!("节点详情读回失败: {failure}"),
                    true,
                )
            })?;
        if detail.artifact_ref.as_ref() != Some(&artifact_ref) {
            return Err(publish_error(
                CODE_NODE_REF_FAILED,
                PUBLISH_STEP,
                None,
                "节点 artifact_ref 读回与写入不一致",
                true,
            ));
        }

        // 发布回执：写入 + 发布屏障核验（缺回执/摘要不符/节点指向别的版本都 Err）。
        let receipt = ScArtifactPublishedReceipt {
            schema_version: 1,
            package_id: package.package_id.clone(),
            chain_id: package.chain_id.clone(),
            node_id: node_id.clone(),
            package_digest: digest.clone(),
            version: published_row.version,
        };
        store
            .put_sc_artifact_published_receipt(&identity, &receipt)
            .map_err(|failure| {
                publish_error(
                    CODE_RECEIPT_FAILED,
                    PUBLISH_STEP,
                    failure.round_id.clone(),
                    format!("发布回执落盘失败: {}", failure.detail),
                    true,
                )
            })?;
        store
            .validate_sc_artifact_publication(&scope, &published_row)
            .map_err(|failure| {
                publish_error(
                    CODE_RECEIPT_FAILED,
                    PUBLISH_STEP,
                    failure.round_id.clone(),
                    format!("发布屏障核验失败: {}", failure.detail),
                    true,
                )
            })?;

        // 持久屏障齐了才同步引擎内存版本表并广播（普通 current 版本原样保留，
        // session.artifact 不动）。
        self.artifact_versions = merged;
        self.event_tx
            .send(EngineEvent::ArtifactUpdate {
                version: published_row.version,
                payload: ArtifactPayload::ScFailureRepairPackage {
                    sc_failure_repair_package: Box::new(package),
                },
            })
            .await
            .map_err(|failure| {
                publish_error(
                    CODE_BROADCAST_FAILED,
                    PUBLISH_STEP,
                    None,
                    format!("广播 ArtifactUpdate 失败: {failure}"),
                    false,
                )
            })?;
        Ok(artifact_ref)
    }
}

/// 已验证前缀的链身份：取首条**真实**条目的 chain（受控首行），并核作用域与
/// chain_id 与请求一致。
fn prefix_chain_identity(
    prefix: &ScFailureEvidenceRead,
    scope: &ScFailureScope,
    chain_id: &str,
) -> Result<ScFailureChainIdentity, ScFailureEvidenceError> {
    let Some(entry) = prefix.entries.first() else {
        return Err(publish_error(
            CODE_EVIDENCE_INVALID,
            PUBLISH_STEP,
            None,
            "已验证前缀里没有任何真实条目，无法确认链身份",
            true,
        ));
    };
    let identity = entry.chain.clone();
    if &identity.scope != scope || identity.chain_id != chain_id {
        return Err(publish_error(
            CODE_SCOPE_MISMATCH,
            PUBLISH_STEP,
            None,
            "已验证前缀的链身份与请求作用域/chain_id 不一致",
            true,
        ));
    }
    Ok(identity)
}

/// Local 终局事实：取包自身的终局字段与结构化 missing（不伪造 durable 终局）。
fn local_terminal_from_package(package: &ScFailureRepairPackage) -> ScLocalTerminal {
    let last_physical_round_id = package
        .rounds
        .iter()
        .find(|round| round.raw == package.last_provider_output)
        .map(|round| round.identity.round_id.clone());
    ScLocalTerminal {
        last_failure_round_id: package.last_failure_source.source_round_id.clone(),
        last_physical_round_id,
        terminal_class: package.terminal_class.clone(),
        terminal_reason: package.terminal_reason.clone(),
        launch_observations: package
            .rounds
            .iter()
            .map(|round| (round.identity.round_id.clone(), round.launch_state.clone()))
            .collect(),
        completeness: package.completeness.clone(),
        missing_evidence: package.missing_evidence.clone(),
    }
}

/// 证据正文核验：raw 走真实 split 输出读回，教学提示走提示正文读回；结构类
/// source 按 typed `failure_class` 走既有 source store 读回（不按字符串猜域）。
///
/// 编译/IR 的归一化 source 不在这里重算：product 不反向 import handler 的
/// resolver、也不复制归一化算法——它的正确性是调用方前置（P19d 交包前用 P20
/// resolver 核过）。任何读不出来的 source 都如实报错（`sc_failure_evidence_invalid`），
/// 绝不伪造 Available，也绝不悄悄改写载荷。
fn verify_sc_failure_package_bodies(
    store: &LifecycleStore,
    identity: &ScFailureChainIdentity,
    scope: &ScFailureScope,
    package: &ScFailureRepairPackage,
) -> Result<(), ScFailureEvidenceError> {
    for round in &package.rounds {
        if let ScEvidenceRef::Available {
            locator,
            sha256,
            bytes,
        } = &round.raw
        {
            let expected_locator = sc_failure_split_locator(
                &round.identity.workspace_session_id,
                round.identity.role_run_seq,
            );
            if locator != &expected_locator {
                return Err(publish_error(
                    CODE_EVIDENCE_BODY_MISMATCH,
                    PUBLISH_STEP,
                    Some(round.identity.round_id.clone()),
                    format!("raw locator {locator} 与该轮真实 split 输出定位符不一致"),
                    true,
                ));
            }
            let body = store.read_sc_split_output(scope, &round.identity)?;
            verify_sc_failure_body(
                locator,
                body.as_bytes(),
                sha256,
                *bytes,
                &round.identity.round_id,
            )?;
        }
        if let ScEvidenceRef::Available {
            locator,
            sha256,
            bytes,
        } = &round.source
            && round.failure_class == Some(ScFailureClass::StructureGate)
        {
            let source_store = WorkItemPlanSourceStore::new(store.app_paths().clone());
            let source_scope = SourceStoreScope {
                project_id: scope.project_id.clone(),
                issue_id: scope.issue_id.clone(),
                plan_id: scope.plan_id.clone(),
            };
            let record = source_store
                .get_source_revision(&source_scope, locator)
                .map_err(|failure| {
                    publish_error(
                        CODE_EVIDENCE_INVALID,
                        PUBLISH_STEP,
                        Some(round.identity.round_id.clone()),
                        format!("结构 source 读回失败: {failure}"),
                        true,
                    )
                })?;
            verify_sc_failure_body(
                locator,
                record.source.as_bytes(),
                sha256,
                *bytes,
                &round.identity.round_id,
            )?;
        }
    }
    for prompt in &package.teaching_prompts {
        if let ScEvidenceRef::Available {
            locator,
            sha256,
            bytes,
        } = &prompt.prompt
        {
            let body = store.read_sc_teaching_prompt(identity, locator)?;
            verify_sc_failure_body(
                locator,
                body.as_bytes(),
                sha256,
                *bytes,
                &prompt.failed_round_id,
            )?;
        }
    }
    Ok(())
}

fn verify_sc_failure_body(
    locator: &str,
    body: &[u8],
    expected_sha256: &str,
    expected_bytes: u64,
    round_id: &str,
) -> Result<(), ScFailureEvidenceError> {
    if sha256_hex(body) != expected_sha256 || body.len() as u64 != expected_bytes {
        return Err(publish_error(
            CODE_EVIDENCE_BODY_MISMATCH,
            PUBLISH_STEP,
            Some(round_id.to_string()),
            format!("证据正文与引用不符（locator={locator}）"),
            true,
        ));
    }
    Ok(())
}

/// 发布用摘要：整包紧凑序列化的 SHA-256（含 package_id，不含写入时间）。
fn sc_failure_package_digest(
    package: &ScFailureRepairPackage,
) -> Result<String, ScFailureEvidenceError> {
    let bytes = serde_json::to_vec(package).map_err(|failure| {
        publish_error(
            CODE_PACKAGE_SERIALIZE,
            PUBLISH_STEP,
            None,
            format!("包序列化失败: {failure}"),
            false,
        )
    })?;
    Ok(sha256_hex(&bytes))
}

/// publisher 专用错误构造（可带 publication_incomplete）。
fn publish_error(
    code: &str,
    step: &str,
    round_id: Option<String>,
    detail: impl Into<String>,
    publication_incomplete: bool,
) -> ScFailureEvidenceError {
    ScFailureEvidenceError {
        code: code.to_string(),
        step: step.to_string(),
        round_id,
        detail: detail.into(),
        publication_incomplete,
        verified_prefix: None,
    }
}

#[cfg(test)]
#[path = "tests/sc_failure_artifacts.rs"]
mod tests;

/// P11 严格 publisher 行为用例（真实 store / 真实 engine / 真实文件故障）。
#[cfg(test)]
#[path = "tests/sc_failure_publication.rs"]
mod publication_tests;
