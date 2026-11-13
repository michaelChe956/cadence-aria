//! SC 生成段教学重驱的共用驱动臂与单槽窗口消费判定（REQ-SG-02/03、
//! REQ-WSC-18/25/28/28E）。
//!
//! Task 6 起「槽位」不再是进程内布尔量：四类可教学失败（parse 语法 /
//! 事实完整 typed lowering / 事实完整 typed IR 校验 / 确定性结构拒收）
//! 在重驱判定处统一消费**同一**耐久教学窗口（establish → 漂移核验 →
//! claim；Granted/Replay ⇒ 恰一次重驱，Denied ⇒ 停止该生成自动链）。
//! 本文件承载该窗口的消费判定辅助（四类共用，自 run/single_candidate.rs
//! 拆出以守其行数登记）与结构门拒收的教学重驱驱动臂——事件发射与
//! provider 驱动和 compile/IR 教学重驱同一形状（同 node 再驱一次
//! provider），**不新造第二个 provider attempt 通道**；重驱资格由调用方
//! 消费同一窗口的 claim 结果后经 `sc_teaching_attempt` 传入。

use super::gateway_start::PlanAuthorLaunch;
use super::sc_failure_transport::{ScAuthorRoundDelivery, ScBoundProviderRound};
use super::sc_teaching_source::{
    ScResolvedTeachingSource, ScTeachingSourceInput, verify_sc_teaching_request_sources,
};
use super::single_candidate::SingleCandidateProviderRunError;
use super::*;
use crate::product::lifecycle_store::sc_failure_evidence::ScFailureEvidenceError;
use crate::product::lifecycle_store::{ScTeachingClaimOutcome, ScTeachingEstablishOutcome};
use crate::product::models::{SingleCandidatePhase, WorkspaceSessionRecord};
use crate::product::planning_failure::PlanningFailureKind;
use crate::product::work_item_plan_policy::{
    ScTeachingAttemptLifecycle, ScTeachingAttemptOutcome, ScTeachingAttemptRef,
    ScTeachingWindowRequest, ScTeachingWindowStatus,
};
use crate::product::work_item_plan_source_store::{SourceStoreScope, WorkItemPlanSourceStore};
use crate::web::types::GenerateWorkItemsRequest;
use crate::web::workspace_ws_types::{
    ScEvidenceRef, ScFailureClass, ScFailureChainIdentity, ScFailureScope, ScPromptDisposition,
    ScRoundIdentity, ScTeachingDecision, ScTeachingPromptEvidence,
};

/// 结构门拒收的首轮失败描述（终态错误文案用；只述事实，不虚构原因）。
pub(super) fn structure_gate_first_round_failure() -> String {
    format!(
        "candidate structure gate rejected ({})",
        crate::product::workspace_engine::single_candidate_structure_failure::STRUCTURE_STABLE_CODE
    )
}

/// run 级冻结绑定事实（业务请求/材料/上下文 digest——整条链内恒定，
/// 四类失败的窗口请求共享，构成漂移核验的基准）。
pub(super) struct ScTeachingChainBinding {
    pub business_request_digest: String,
    pub material_digests: Vec<String>,
    pub context_digest: String,
}

/// 教学窗口请求/冻结摘要共用的 sha256 hex。
pub(super) fn sc_sha256(text: &str) -> String {
    use sha2::Digest as _;
    hex::encode(sha2::Sha256::digest(text.as_bytes()))
}

/// run 级冻结绑定的构造：业务请求取 serde 投影、材料 = story/design
/// 上下文块逐块摘要、context = 语言规则正文摘要（三者链内恒定）。
pub(super) fn sc_teaching_chain_binding(
    request: &GenerateWorkItemsRequest,
    story_blocks: &[String],
    design_blocks: &[String],
    language_rules: &str,
) -> ScTeachingChainBinding {
    let business_request = serde_json::to_string(request)
        .unwrap_or_else(|error| format!("unserializable-request:{error}"));
    ScTeachingChainBinding {
        business_request_digest: sc_sha256(&business_request),
        material_digests: story_blocks
            .iter()
            .chain(design_blocks.iter())
            .map(|block| sc_sha256(block))
            .collect(),
        context_digest: sc_sha256(language_rules),
    }
}

/// 允许修改闭包（本 change 冻结为常量描述：四类失败共用的教学修复边界）。
const SC_TEACHING_ALLOWED_EDIT_CLOSURE: &str =
    "仅允许修复本次失败事实指出的 work-item-plan markdown source 缺陷";

/// typed 事实的稳定码投影（IR/结构类共用；资格判定不经此字段——WSC-09
/// 资格由 teaching_verdict 先行给出，`Unknown{code}` 在窗口内仅作审计
/// 留痕，不参与结构门 L1 资格）。
pub(super) fn stable_code_kind(code: &str) -> PlanningFailureKind {
    PlanningFailureKind::Unknown {
        code: code.to_string(),
    }
}

/// compile 域 typed 事实的稳定码投影（parse 码表 / lowering / IR 诊断码）。
pub(super) fn compile_typed_kinds(
    diagnostics: &[crate::product::work_item_plan_compiler::CompilerDiagnostic],
) -> Vec<PlanningFailureKind> {
    diagnostics
        .iter()
        .map(|diagnostic| stable_code_kind(&diagnostic.code))
        .collect()
}

/// 各失败类的窗口请求构造：失败源引用/摘要取**真实解析出的失败源**
/// （`ScResolvedTeachingSource`：split locator 或 canonical revision ref），
/// prompt 引用/摘要取**已落盘并严格读回**的 prompt 证据（真实 locator +
/// 已核正文摘要）。构造器名只作审计留痕，**绝不进 `prompt_ref`**。
///
/// prompt 证据无可用 locator（历史/缺提示记录）⇒ 显式 Err，不伪造引用。
#[allow(clippy::result_large_err)] // 证据面错误沿既有 typed 出口上浮。
pub(super) fn sc_teaching_failure_request(
    binding: &ScTeachingChainBinding,
    source: &ScResolvedTeachingSource,
    failure_typed_kinds: Vec<PlanningFailureKind>,
    prompt: &ScTeachingPromptEvidence,
    instruction_digest: String,
) -> Result<ScTeachingWindowRequest, ScFailureEvidenceError> {
    let ScEvidenceRef::Available {
        locator: prompt_ref,
        sha256: prompt_digest,
        ..
    } = &prompt.prompt
    else {
        return Err(ScFailureEvidenceError {
            code: "sc_failure_prompt_ref_missing".to_string(),
            step: "sc_teaching_failure_request".to_string(),
            round_id: Some(prompt.failed_round_id.clone()),
            detail: "教学提示证据无可用 locator，无法构造窗口请求".to_string(),
            publication_incomplete: false,
            verified_prefix: None,
        });
    };
    Ok(ScTeachingWindowRequest {
        // 判据②：author 走 start_work_item_plan_author 且 Task 4 回填已接线。
        provider_path_admissible: true,
        // 失败源引用 = 本轮真实失败源（与证据面同源；缺失 ⇒ 无引用可填）。
        failure_source_ref: Some(source.failure_source_ref.clone()),
        failure_source_sha256: source.sha256.clone(),
        failure_typed_kinds,
        business_request_digest: binding.business_request_digest.clone(),
        material_digests: binding.material_digests.clone(),
        context_digest: binding.context_digest.clone(),
        allowed_edit_closure: SC_TEACHING_ALLOWED_EDIT_CLOSURE.to_string(),
        instruction_digest,
        prompt_ref: prompt_ref.clone(),
        prompt_digest: prompt_digest.clone(),
    })
}

/// 启动前复验用的冻结事实：失败源输入（本轮真实 raw/source 引用——零回退
/// latest/last-good、不以摘要当身份）+ 已核验的窗口请求 + 已落盘读回的
/// prompt 证据。
#[derive(Debug)]
pub(super) struct ScTeachingStartVerify {
    /// 失败轮身份（复验时的轮绑定）。
    pub identity: ScRoundIdentity,
    /// 失败类别（来源类别只由它决定，绝不从 locator 反猜）。
    pub failure_class: ScFailureClass,
    /// 本轮 raw 引用（split 原始正文）。
    pub raw: ScEvidenceRef,
    /// 本轮失败源引用（split locator / canonical revision ref）。
    pub source: ScEvidenceRef,
    /// 已核验的窗口请求（引用/摘要冻结面）。
    pub request: ScTeachingWindowRequest,
    /// 已落盘并严格读回的 prompt 证据。
    pub prompt_evidence: ScTeachingPromptEvidence,
}

impl ScTeachingStartVerify {
    /// 复验用的借用输入（与 resolver 的冻结契约同形：只四个引用，无
    /// `ScRoundEvidence`、无默认教学决定）。
    fn source_input(&self) -> ScTeachingSourceInput<'_> {
        ScTeachingSourceInput {
            identity: &self.identity,
            failure_class: &self.failure_class,
            raw: &self.raw,
            source: &self.source,
        }
    }
}

/// 启动前复验（硬前置 ⇒ 零额外启动）：失败源引用/摘要必须与真实失败源
/// 一致（P20 复验：读真实 source，不取最新、不回退 last-good），且教学
/// 提示正文必须严格读回、摘要与窗口请求一致。任一不符 ⇒ 零启动（不 claim、
/// 不发 prompt 事件、不起 provider）。
#[allow(clippy::result_large_err)] // 运行收尾的唯一 typed 出口（同既有先例）。
pub(super) fn verify_sc_teaching_start(
    lifecycle: &LifecycleStore,
    chain: &ScFailureChainIdentity,
    verify: &ScTeachingStartVerify,
) -> Result<(), SingleCandidateProviderRunError> {
    verify_sc_teaching_request_sources(
        lifecycle,
        &chain.scope,
        &verify.source_input(),
        &verify.request,
    )
    .map_err(|error| {
        SingleCandidateProviderRunError::Message(format!(
            "SC 教学重驱启动前复验失败（失败源不可用）：{error}"
        ))
    })?;
    let body = lifecycle
        .read_sc_teaching_prompt(chain, &verify.request.prompt_ref)
        .map_err(|error| {
            SingleCandidateProviderRunError::Message(format!(
                "SC 教学重驱启动前复验失败（教学提示正文不可读）：{error}"
            ))
        })?;
    if sc_sha256(&body) != verify.request.prompt_digest {
        return Err(SingleCandidateProviderRunError::Message(
            "SC 教学重驱启动前复验失败：教学提示正文摘要与窗口请求不一致".to_string(),
        ));
    }
    Ok(())
}

/// 实际发送后的处置记账：只有**真的发出**才记 `Sent`（预算拒绝/仅装配
/// 永不冒充已发送）。正文与已落盘全文逐字相同（不可变正文守卫），失败
/// 只留告警——发送事实不因记账失败而被改写。
pub(super) fn record_sc_teaching_prompt_sent(
    lifecycle: &LifecycleStore,
    chain: &ScFailureChainIdentity,
    verify: &ScTeachingStartVerify,
    text: &str,
) {
    let evidence = &verify.prompt_evidence;
    let outcome = lifecycle.put_sc_teaching_prompt(
        chain,
        &evidence.failed_round_id,
        evidence.target_round_ordinal,
        &evidence.builder_id,
        text,
        ScPromptDisposition::Sent,
    );
    if let Err(error) = outcome {
        tracing::warn!(
            session_id = %chain.scope.session_id,
            error = ?error,
            "record sc teaching prompt sent failed (send stands)"
        );
    }
}

/// 结构臂失败源的真实摘要面：只读该轮 sealed 的 canonical revision（不取
/// 最新、不回退 last-good）——事实面的 source 摘要/字节必须是该 revision
/// 的真值（split locator 冒充 ⇒ 拒绝）。
#[allow(clippy::result_large_err)] // 证据面错误沿既有 typed 出口上浮。
pub(super) fn structure_source_digest(
    lifecycle: &LifecycleStore,
    scope: &ScFailureScope,
    locator: &str,
) -> Result<(String, u64), ScFailureEvidenceError> {
    let store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
    let record = store
        .get_source_revision(
            &SourceStoreScope {
                project_id: scope.project_id.clone(),
                issue_id: scope.issue_id.clone(),
                plan_id: scope.plan_id.clone(),
            },
            locator,
        )
        .map_err(|failure| ScFailureEvidenceError {
            code: "sc_failure_source_unavailable".to_string(),
            step: "structure_source_digest".to_string(),
            round_id: None,
            detail: format!("读取 source revision 失败: {} ({:?})", failure.code(), failure),
            publication_incomplete: false,
            verified_prefix: None,
        })?;
    Ok((record.source_revision_hash, record.source.len() as u64))
}

/// 单槽收口的教学窗口消费判定结果（四类失败共用）。
pub(super) enum ScTeachingWindowDecision {
    /// Granted/Replay ⇒ 以该 attempt 执行恰一次重驱（原调用在内合计两次）。
    Drive { attempt: ScTeachingAttemptRef },
    /// 可判定拒绝（未批准/耗尽/已关闭）：已尽力 close(FailedTerminal)，
    /// 调用方停止该生成自动链并按既有失败面回显真实原因。
    Denied,
    /// 冻结输入漂移：`sc_teaching_frozen_input_drift` 诊断已落 durable，
    /// 零额外 provider 调用，调用方按既有失败面收口。
    Drift,
    /// 窗口执法失败（store 错误）：fail-closed 上浮（不无账启动）。
    StoreError(String),
}

/// 四类失败在重驱判定处共用的窗口消费（REQ-WSC-28/28E）：
/// establish（首次可教学失败建窗；末尾 Active ⇒ Existing，A/B 接力与
/// 重放不重建）→ 结构性拒绝先行（未批准/耗尽/已关闭 ⇒ 关窗
/// FailedTerminal——第二次交付失败即关窗、无第三次启动，且不消费 claim
/// 故漂移核验无需进行）→ 对当前失败事实 verify_frozen_inputs（漂移 ⇒
/// 零额外调用 + durable 诊断）→ claim（Granted/Replay ⇒ Drive）。
pub(super) fn consume_sc_teaching_window(
    lifecycle: &LifecycleStore,
    session_id: &str,
    request: &ScTeachingWindowRequest,
) -> ScTeachingWindowDecision {
    let record = match lifecycle.get_workspace_session(session_id) {
        Ok(record) => record,
        Err(error) => {
            return ScTeachingWindowDecision::StoreError(format!(
                "load session for sc teaching window failed: {error:?}"
            ));
        }
    };
    let record = match lifecycle.establish_sc_teaching_window(&record, request) {
        Ok(ScTeachingEstablishOutcome::Established { record, .. })
        | Ok(ScTeachingEstablishOutcome::Existing { record, .. }) => record,
        Err(error) => {
            return ScTeachingWindowDecision::StoreError(format!(
                "establish sc teaching window failed: {error:?}"
            ));
        }
    };
    let Some(window) = record.sc_generation_teaching_windows.last() else {
        return ScTeachingWindowDecision::StoreError(
            "teaching window missing after establish".to_string(),
        );
    };
    let generation_seq = window.generation_seq;
    let attempt_no = window
        .attempts
        .last()
        .map_or(1, |attempt| attempt.attempt_no);
    let in_flight = matches!(
        window.attempts.last().map(|attempt| &attempt.lifecycle),
        Some(ScTeachingAttemptLifecycle::Claimed | ScTeachingAttemptLifecycle::LaunchUnknown)
    );
    if !in_flight && window.claimable_attempt_no().is_none() {
        return deny_and_close(lifecycle, &record, generation_seq, attempt_no);
    }
    if window.verify_frozen_inputs(request).is_err() {
        if let Err(error) = lifecycle.record_sc_teaching_frozen_input_drift(&record) {
            tracing::warn!(
                session_id,
                error = ?error,
                "record sc teaching frozen input drift failed (drift decision stands)"
            );
        }
        return ScTeachingWindowDecision::Drift;
    }
    match lifecycle.claim_sc_teaching_attempt(&record, generation_seq) {
        Ok(ScTeachingClaimOutcome::Granted(saved)) | Ok(ScTeachingClaimOutcome::Replay(saved)) => {
            let attempt = saved
                .sc_generation_teaching_windows
                .last()
                .and_then(|window| window.attempts.last())
                .expect("claimed teaching window keeps its attempt");
            ScTeachingWindowDecision::Drive {
                attempt: ScTeachingAttemptRef {
                    generation_seq,
                    attempt_no: attempt.attempt_no,
                    attempt_key: attempt.attempt_key.clone(),
                },
            }
        }
        Ok(ScTeachingClaimOutcome::Denied(_)) => {
            deny_and_close(lifecycle, &record, generation_seq, attempt_no)
        }
        Err(error) => ScTeachingWindowDecision::StoreError(format!(
            "claim sc teaching attempt failed: {error:?}"
        )),
    }
}

/// 可判定拒绝的关窗收口（尽力而为）：close(FailedTerminal) 失败只留告警、
/// 不改变拒绝结论——durable 原状由恢复侧按 Unknown/关闭语义处理。
fn deny_and_close(
    lifecycle: &LifecycleStore,
    record: &WorkspaceSessionRecord,
    generation_seq: u32,
    attempt_no: u8,
) -> ScTeachingWindowDecision {
    if let Err(error) = lifecycle.close_sc_teaching_window(
        record,
        generation_seq,
        attempt_no,
        ScTeachingAttemptOutcome::FailedTerminal,
    ) {
        tracing::warn!(
            session_id = %record.id,
            generation_seq,
            attempt_no,
            error = ?error,
            "close denied sc teaching window failed (denial stands)"
        );
    }
    ScTeachingWindowDecision::Denied
}

/// 重驱交付成功（Candidate）后的关窗（Delivered）——尽力而为：无窗口/
/// 已关窗/冲突只留告警，不阻断交付链（窗口保留关闭与消耗事实，不删除；
/// 关窗快照供下一条链做判据③ 的 base）。
pub(super) fn close_sc_teaching_delivered(lifecycle: &LifecycleStore, session_id: &str) {
    let Ok(record) = lifecycle.get_workspace_session(session_id) else {
        return;
    };
    let Some(window) = record.sc_generation_teaching_windows.last() else {
        return;
    };
    if !matches!(window.status, ScTeachingWindowStatus::Active) {
        return;
    }
    let generation_seq = window.generation_seq;
    let attempt_no = window
        .attempts
        .last()
        .map_or(1, |attempt| attempt.attempt_no);
    if let Err(error) = lifecycle.close_sc_teaching_window(
        &record,
        generation_seq,
        attempt_no,
        ScTeachingAttemptOutcome::Delivered,
    ) {
        tracing::warn!(
            session_id,
            generation_seq,
            attempt_no,
            error = ?error,
            "close delivered sc teaching window failed (delivery stands)"
        );
    }
}

/// 教学重驱共用的轮绑定：沿用原轮链身份（同一 scope/business_key/原节点），
/// 教学轮是新物理 provider run（新 handle + role_run_seq）。
/// 返回 **typed 启动分类**（Ready/Completed/Waiting）：只有链身份/存储级失败才
/// 持久化终态 Failed 并以 Message 上浮（fail-closed、零启动）——Waiting 不得被
/// 当成启动失败，Completed 不得重新 claim。
#[allow(clippy::result_large_err)] // 运行收尾的唯一 typed 出口（同本文件先例）。
pub(super) fn begin_sc_teaching_reredrive_run(
    lifecycle: &LifecycleStore,
    engine: &mut WorkspaceEngine,
    original: &ScBoundProviderRound,
    previous: Option<&(ScEvidenceRef, ScRoundIdentity)>,
    node_id: &str,
    author_provider: &ProviderName,
) -> Result<super::sc_failure_transport::ScRoundEntry, SingleCandidateProviderRunError> {
    match super::sc_failure_transport::begin_sc_teaching_round(
        lifecycle,
        original,
        previous,
        node_id,
        author_provider,
    ) {
        // Ready/Completed/Waiting 都是**已成立**的分类，交调用方按类收口：Waiting
        // 走真实 CAS 等待（不落 Failed/A4），Completed 复用 durable raw（不再
        // claim、不再发提示词）。只有链身份/存储级失败才落 Failed。
        Ok(entry) => Ok(entry),
        Err(failure) => {
            engine.persist_single_candidate_terminal_phase(SingleCandidatePhase::Failed);
            Err(SingleCandidateProviderRunError::Message(format!("{failure}")))
        }
    }
}

/// 重驱判定的收口包装（compile/IR/结构三类失败共用）：把既有窗口消费结果
/// 一一映射为**真实判定对**——Drive/Replay ⇒ `(Granted, Some(attempt))`；
/// Denied ⇒ `(Denied, None)`（`deny_and_close` 已先执行）；Drift ⇒
/// `(Drift, None)`（durable 漂移诊断已先写）；StoreError ⇒ 不形成判定，
/// 持久化终态 Failed 后以 Message 上浮（fail-closed，不无账启动）。
/// 调用方必须在返回后立刻把判定 durable 写读，再启动 provider。
#[allow(clippy::result_large_err)] // 运行收尾的唯一 typed 出口（同本文件先例）。
pub(super) fn authorize_sc_teaching_reredrive(
    lifecycle: &LifecycleStore,
    engine: &mut WorkspaceEngine,
    request: &ScTeachingWindowRequest,
) -> Result<(ScTeachingDecision, Option<ScTeachingAttemptRef>), SingleCandidateProviderRunError> {
    match consume_sc_teaching_window(lifecycle, &engine.session().session_id, request) {
        ScTeachingWindowDecision::Drive { attempt } => {
            Ok((ScTeachingDecision::Granted, Some(attempt)))
        }
        ScTeachingWindowDecision::Denied => Ok((ScTeachingDecision::Denied, None)),
        ScTeachingWindowDecision::Drift => Ok((ScTeachingDecision::Drift, None)),
        ScTeachingWindowDecision::StoreError(message) => {
            engine.persist_single_candidate_terminal_phase(SingleCandidatePhase::Failed);
            Err(SingleCandidateProviderRunError::Message(message))
        }
    }
}

/// compile/IR 两类教学重驱的共用驱动臂（自 run/single_candidate.rs 抽出
/// 以守其行数登记）：发可观测 prompt 事件 → 严格绑定教学物理轮 → 复用
/// `drive_single_candidate_reredrive` 同一 provider 通道执行恰一次重驱，
/// 返回归一化 source、严格读回的 raw 引用与轮身份（调用方回编译循环）。
#[allow(clippy::too_many_arguments)]
#[allow(clippy::result_large_err)] // 同上：该枚举是运行收尾的唯一 typed 出口。
pub(super) async fn drive_compile_teaching_reredrive(
    engine: &mut WorkspaceEngine,
    lifecycle: &LifecycleStore,
    launch: &PlanAuthorLaunch,
    provider_for_run: &Arc<dyn StreamingProviderAdapter>,
    run_cancel: &CancellationToken,
    command_rx: &mut mpsc::Receiver<ProviderCommand>,
    node_id: &str,
    author_provider: &ProviderName,
    event_key: &str,
    event_title: &str,
    event_detail: &str,
    reredrive_prompt: &str,
    repository_path: &std::path::Path,
    attempt: &ScTeachingAttemptRef,
    // P25：启动前复验用的冻结事实（失败源引用面 + 已核验的窗口请求 +
    // 已落盘读回的 prompt 证据）——启动前必核，零额外启动。
    verify: &ScTeachingStartVerify,
    // P08：教学轮沿用原轮链身份；当前轮（raw + 轮身份）推进教学轮 ordinal。
    original: &ScBoundProviderRound,
    previous: Option<&(ScEvidenceRef, ScRoundIdentity)>,
) -> Result<ScAuthorRoundDelivery, SingleCandidateProviderRunError> {
    // 启动前复验（硬前置）：失败源引用/摘要与 prompt 正文摘要都必须与冻结
    // 请求一致；任一不符 ⇒ 零启动（不发 prompt 事件、不起 provider）。
    verify_sc_teaching_start(lifecycle, &original.chain, verify)?;
    // P08 replay 安全：先分类再发事件/启动——Ready 才真实启动；Completed 复用
    // durable raw（不重发「已发送提示词」、不记 `Sent`）；Waiting 走真实 CAS 等待
    // （不当启动失败、不落 Failed/A4）。分类是 Ok 值，只有链/存储失败才 Err。
    let entry = begin_sc_teaching_reredrive_run(
        lifecycle,
        engine,
        original,
        previous,
        node_id,
        author_provider,
    )?;
    if entry.is_ready() {
        engine
            .emit_execution_event(
                ProviderExecutionEvent {
                    event_id: format!("{node_id}_prompt_{event_key}"),
                    kind: ProviderExecutionEventKind::Output,
                    status: ProviderExecutionEventStatus::Started,
                    title: event_title.to_string(),
                    detail: Some(event_detail.to_string()),
                    command: None,
                    cwd: None,
                    output: Some(reredrive_prompt.to_string()),
                    exit_code: None,
                },
                Some(node_id.to_string()),
                Some(author_provider.clone()),
            )
            .await;
    }
    let (delivery, launched) = super::sc_failure_round_entry::drive_sc_teaching_entry(
        engine,
        lifecycle,
        launch,
        Arc::clone(provider_for_run),
        run_cancel,
        command_rx,
        node_id,
        author_provider,
        reredrive_prompt,
        repository_path,
        entry,
        Some(attempt),
    )
    .await?;
    // 真的发出之后才记 `Sent`（预算拒绝/仅装配/已完成重放永不冒充已发送）。
    if launched {
        record_sc_teaching_prompt_sent(lifecycle, &original.chain, verify, reredrive_prompt);
    }
    Ok(delivery)
}

/// 结构门拒收教学重驱：发一条可观测的 prompt 事件（与 compile/IR 重驱的
/// 事件同形），随后复用 `drive_single_candidate_reredrive` 同一 provider
/// 通道再驱一次作者，返回归一化 source、raw 引用与轮身份——调用方回编译
/// 循环重走 compile → IR 预校验 → 候选结构门；`sc_teaching_attempt` 为调用方
/// 消费同一教学窗口的 claim 结果（结构门类不再有独立槽位或布尔授权）。
#[allow(clippy::too_many_arguments)]
#[allow(clippy::result_large_err)] // 同上：该枚举是运行收尾的唯一 typed 出口。
pub(super) async fn drive_structure_gate_reredrive(
    engine: &mut WorkspaceEngine,
    lifecycle: &LifecycleStore,
    launch: &PlanAuthorLaunch,
    provider_for_run: Arc<dyn StreamingProviderAdapter>,
    run_cancel: &CancellationToken,
    command_rx: &mut mpsc::Receiver<ProviderCommand>,
    node_id: &str,
    author_provider: &ProviderName,
    repair_prompt: &str,
    repository_path: &std::path::Path,
    sc_teaching_attempt: Option<&ScTeachingAttemptRef>,
    // P25：启动前复验用的冻结事实（失败源引用面 + 已核验的窗口请求 +
    // 已落盘读回的 prompt 证据）——启动前必核，零额外启动。
    verify: &ScTeachingStartVerify,
    // P08：教学轮沿用原轮链身份；当前轮（raw + 轮身份）推进教学轮 ordinal。
    original: &ScBoundProviderRound,
    previous: Option<&(ScEvidenceRef, ScRoundIdentity)>,
) -> Result<ScAuthorRoundDelivery, SingleCandidateProviderRunError> {
    // 启动前复验（硬前置）：失败源引用/摘要与 prompt 正文摘要都必须与冻结
    // 请求一致；任一不符 ⇒ 零启动（不发 prompt 事件、不起 provider）。
    verify_sc_teaching_start(lifecycle, &original.chain, verify)?;
    // P08 replay 安全：同 compile/IR 臂——先分类再发事件/启动。
    let entry = begin_sc_teaching_reredrive_run(
        lifecycle,
        engine,
        original,
        previous,
        node_id,
        author_provider,
    )?;
    if entry.is_ready() {
        engine
            .emit_execution_event(
                ProviderExecutionEvent {
                    event_id: format!("{node_id}_prompt_structure_reredrive"),
                    kind: ProviderExecutionEventKind::Output,
                    status: ProviderExecutionEventStatus::Started,
                    title: "SC 结构门拒收教学重驱提示词".to_string(),
                    detail: Some(
                        "候选结构门拒收（final_compile_structure_rejected）的教学重驱\
                         （环在哪 + 两个合法修法各带后果 + 保护项 + 完整修订版输出指令），\
                         同一教学窗口的恰一次机会（与 compile/IR 类共用窗口）"
                            .to_string(),
                    ),
                    command: None,
                    cwd: None,
                    output: Some(repair_prompt.to_string()),
                    exit_code: None,
                },
                Some(node_id.to_string()),
                Some(author_provider.clone()),
            )
            .await;
    }
    let (delivery, launched) = super::sc_failure_round_entry::drive_sc_teaching_entry(
        engine,
        lifecycle,
        launch,
        provider_for_run,
        run_cancel,
        command_rx,
        node_id,
        author_provider,
        repair_prompt,
        repository_path,
        entry,
        sc_teaching_attempt,
    )
    .await?;
    // 真的发出之后才记 `Sent`（预算拒绝/仅装配/已完成重放永不冒充已发送）。
    if launched {
        record_sc_teaching_prompt_sent(lifecycle, &original.chain, verify, repair_prompt);
    }
    Ok(delivery)
}

/// Task 8（tasks 5.1/5.2，REQ-WSC-25/28D；评审 Important-1 修订）：generate
/// 段失败流水行的阶段判定——lowering fault 在场 ⇒ `TypedLowering`；否则
/// ir_fault 在场 ⇒ `TypedIrValidation`；否则仅当诊断集非空且**全部**命中
/// parse 码表才记 `Parse`；码表外（unknown/operational 类——不可归因到
/// parse/lowering/IR 任一子阶段，含空诊断集）记 `Generate`（生成尝试
/// 整体失败）——五段区分在 durable 数据上穷尽，`Parse` 不再吞并码表外类。
fn generate_segment_failure_stage(
    diagnostics: &[crate::product::work_item_plan_compiler::CompilerDiagnostic],
) -> crate::product::lifecycle_store::candidate_attempts::CandidateAttemptStage {
    use crate::product::lifecycle_store::candidate_attempts::CandidateAttemptStage;
    let parse_table_hit =
        |diagnostic: &crate::product::work_item_plan_compiler::CompilerDiagnostic| {
            super::single_candidate::single_candidate_compile_preflight::is_teachable_parse_failure(
                &diagnostic.code,
            )
        };
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.lowering_fault.is_some())
    {
        return CandidateAttemptStage::TypedLowering;
    }
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.ir_fault.is_some())
    {
        return CandidateAttemptStage::TypedIrValidation;
    }
    if !diagnostics.is_empty() && diagnostics.iter().all(parse_table_hit) {
        return CandidateAttemptStage::Parse;
    }
    CandidateAttemptStage::Generate
}

/// Task 8（5.1/5.2，REQ-WSC-25/28D；评审 Important-2 修订）：generate 段失败
/// 流水行写入（non-blocking 义务逐字不变——写失败沿用既有降级码
/// `candidate_attempt_log_write_failed` 记 policy diagnostics，不阻断生成链）。
/// 关联单源 durable 解析（`generate_segment_provider_attempt`）：行所属窗口
/// attempt ⇒ `Started`；解析不出窗口 attempt ⇒ **None（不可归因）**——该形态
/// 下交付 1 已物理发生（verdict 非 Eligible ⇒ grant 块先于建窗 break 即写行），
/// 既不可伪造关联、也不可伪造「无启动」的否定事实，与 engine 侧同形态
/// （写点先于建窗的 `engine_failure_provider_attempt`）编码一致。source
/// 引用恒 None + 行内带缺失原因（generate 段失败产物不落 durable source，
/// 不提升 current refs）。幂等键由构造器按尝试身份派生；失败身份掺入世代
/// 业务键，不同世代（re-arm）同内容失败不互吃幂等键。
///
/// A1（REQ-WSC-32）：parse 语法类诊断的完整五字段审计文本——**不进
/// prompt、不拼入被哈希的 `detail`**（`failure_id`/attempt 身份逐字不变），
/// 落流水行 `source_missing_reason` 的显式标记分区、原缺失说明保留在前。
/// 教学面预算内被省略条目的完整事实由此可回取（runner 侧接线见 C2）。
fn parse_fact_audit_partition(
    diagnostics: &[crate::product::work_item_plan_compiler::CompilerDiagnostic],
) -> Option<String> {
    use super::single_candidate::single_candidate_compile_preflight::is_teachable_parse_failure;

    let parse_diagnostics: Vec<_> = diagnostics
        .iter()
        .filter(|diagnostic| is_teachable_parse_failure(&diagnostic.code))
        .collect();
    let mut partition = format!(
        "[sc_parse_fact_audit total={} 五字段完整,不进 prompt]",
        parse_diagnostics.len()
    );
    for diagnostic in &parse_diagnostics {
        partition.push_str(&format!(
            "\n{}|{}|{}|{}|{}",
            diagnostic.code,
            diagnostic.line,
            diagnostic.field,
            diagnostic.message,
            diagnostic.repair_example
        ));
    }
    (!parse_diagnostics.is_empty()).then_some(partition)
}

fn record_generate_segment_failure(
    engine: &mut WorkspaceEngine,
    lifecycle: &LifecycleStore,
    stage: crate::product::lifecycle_store::candidate_attempts::CandidateAttemptStage,
    code: &str,
    kinds: Vec<PlanningFailureKind>,
    detail: &str,
    // A1：parse 五字段审计分区（仅进 source_missing_reason，不进被哈希 detail）。
    parse_fact_audit: Option<String>,
) {
    use crate::product::lifecycle_store::candidate_attempts::{
        CandidateAttemptFailure, candidate_attempt_record_from_generate_segment,
        generate_segment_provider_attempt,
    };
    let session_id = engine.session().session_id.clone();
    let record = lifecycle.get_workspace_session(&session_id).ok();
    let provider_attempt = record.as_ref().and_then(generate_segment_provider_attempt);
    let series = record
        .as_ref()
        .and_then(crate::product::workspace_engine::current_sc_author_business_key)
        .unwrap_or_else(|| "no-launch".to_string());
    let failure = CandidateAttemptFailure {
        failure_id: sc_sha256(&format!("{series}/{stage:?}/{code}/{detail}")),
        code: code.to_string(),
        typed_kinds: kinds,
        cycle_fingerprint: None,
    };
    let attempt = candidate_attempt_record_from_generate_segment(
        stage,
        Some(failure),
        None,
        Some(match parse_fact_audit {
            Some(audit) => format!("{detail}\n{audit}"),
            None => detail.to_string(),
        }),
        provider_attempt,
    );
    if let Err(error) = lifecycle.append_candidate_attempt(&session_id, &attempt) {
        engine.record_single_candidate_compile_failure(
            lifecycle,
            "candidate_attempt_log_write_failed",
            &format!("append generate segment attempt row failed: {error}"),
        );
    }
}

/// Task 8（5.1）：converge 失败（parse / typed lowering 类诊断集）的流水行
/// 写点——稳定码取首条诊断码（编译器确定性输出序），typed 种类沿用窗口
/// 请求同款投影（`compile_typed_kinds`）。
pub(super) fn record_generate_segment_compile_failure(
    engine: &mut WorkspaceEngine,
    lifecycle: &LifecycleStore,
    diagnostics: &[crate::product::work_item_plan_compiler::CompilerDiagnostic],
    detail: &str,
) {
    let code = diagnostics
        .first()
        .map(|diagnostic| diagnostic.code.clone())
        .unwrap_or_else(|| "converge_failed".to_string());
    record_generate_segment_failure(
        engine,
        lifecycle,
        generate_segment_failure_stage(diagnostics),
        &code,
        compile_typed_kinds(diagnostics),
        detail,
        parse_fact_audit_partition(diagnostics),
    );
}

/// Task 8（5.1）：typed IR 校验失败（preflight 判定面）的流水行写点——
/// 阶段恒 `TypedIrValidation`，稳定码与窗口请求同款
/// （`stable_code_kind("typed_ir_validation")`）。
pub(super) fn record_generate_segment_ir_validation_failure(
    engine: &mut WorkspaceEngine,
    lifecycle: &LifecycleStore,
    detail: &str,
) {
    record_generate_segment_failure(
        engine,
        lifecycle,
        crate::product::lifecycle_store::candidate_attempts::CandidateAttemptStage::TypedIrValidation,
        "typed_ir_validation",
        vec![stable_code_kind("typed_ir_validation")],
        detail,
        None,
    );
}

/// 评审定向（Important-1/Minor-5）：阶段分类的 durable 穷尽与新写点吞错面。
#[cfg(test)]
mod task8_review_fixes {
    use super::*;

    fn diagnostic(code: &str) -> crate::product::work_item_plan_compiler::CompilerDiagnostic {
        crate::product::work_item_plan_compiler::CompilerDiagnostic {
            code: code.to_string(),
            line: 1,
            field: "fixture".to_string(),
            message: format!("{code}: fixture"),
            repair_example: "fixture".to_string(),
            kind: None,
            lowering_fault: None,
            ir_fault: None,
        }
    }

    /// Important-1：五段区分穷尽——parse 码表命中 ⇒ `Parse`；码表外
    /// （unknown/operational 类）与空集 ⇒ `Generate`；lowering/ir fault
    /// 仍各自优先（管线序）。
    #[test]
    fn generate_segment_failure_stage_is_exhaustive_in_durable_data() {
        use crate::product::lifecycle_store::candidate_attempts::CandidateAttemptStage;
        assert!(matches!(
            generate_segment_failure_stage(&[diagnostic("missing_section")]),
            CandidateAttemptStage::Parse
        ));
        assert!(matches!(
            generate_segment_failure_stage(&[diagnostic("intent_undeclared")]),
            CandidateAttemptStage::Generate
        ));
        assert!(matches!(
            generate_segment_failure_stage(&[
                diagnostic("missing_section"),
                diagnostic("operational_x")
            ]),
            CandidateAttemptStage::Generate
        ));
        assert!(matches!(
            generate_segment_failure_stage(&[]),
            CandidateAttemptStage::Generate
        ));
    }

    /// Minor-5：新 handler 写点的 non-blocking 降级——占路径注入（root 与
    /// 非 root 行为一致）后触发 converge 失败写点：吞错（不 panic/不上浮）、
    /// 记既有降级码 `candidate_attempt_log_write_failed`、不阻断生成（会话
    /// 状态零变化）。
    #[test]
    fn generate_segment_attempt_log_write_failure_degrades_without_blocking() {
        let tmp = tempfile::TempDir::new().expect("fixture tempdir");
        let lifecycle = LifecycleStore::new(crate::product::app_paths::ProductAppPaths::new(
            tmp.path().join(".aria"),
        ));
        let record = lifecycle
            .create_workspace_session(
                crate::product::lifecycle_store::CreateWorkspaceSessionInput {
                    project_id: "project_0001".to_string(),
                    issue_id: "issue_0001".to_string(),
                    entity_id: "work_item_plan_0001".to_string(),
                    workspace_type: crate::product::models::WorkspaceType::WorkItemPlan,
                    author_provider: crate::product::models::ProviderName::Codex,
                    reviewer_provider: Some(crate::product::models::ProviderName::ClaudeCode),
                    review_rounds: 2,
                    superpowers_enabled: true,
                    openspec_enabled: true,
                    work_item_plan_options: None,
                },
            )
            .expect("fixture session");
        let status_before = record.status.clone();
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
        let mut engine = crate::product::workspace_engine::WorkspaceEngine::new_persistent(
            std::sync::Arc::new(crate::product::checkpoint_store::CheckpointStore::new(
                tmp.path().join("checkpoints"),
            )),
            lifecycle.clone(),
            event_tx,
            crate::product::workspace_engine::WorkspaceSession::from_record(record),
        );
        let ledger_path = lifecycle
            .app_paths()
            .issue_lifecycle_root(&engine.session().project_id, &engine.session().issue_id)
            .join(
                crate::product::lifecycle_store::candidate_attempts::CANDIDATE_ATTEMPTS_FILE_NAME,
            );
        std::fs::create_dir_all(&ledger_path).expect("occupy ledger path with a directory");
        record_generate_segment_compile_failure(
            &mut engine,
            &lifecycle,
            &[diagnostic("missing_section")],
            "missing_section: ### Goal 缺失",
        );
        let persisted = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("reload session after degraded write");
        assert!(
            persisted
                .policy_diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "candidate_attempt_log_write_failed"),
            "新写点流水写失败必须降级为可观测静默（记入运行日志）：{:?}",
            persisted.policy_diagnostics
        );
        assert_eq!(
            persisted.status, status_before,
            "新写点流水写失败不得阻断生成（会话状态零变化）"
        );
    }
}

/// A1（REQ-WSC-32）：parse 事实审计与身份中性——完整五字段落
/// `source_missing_reason` 的标记分区（不进 prompt、不进被哈希的 detail），
/// `failure_id` 对同一诊断集逐字不变；非 parse 类失败保持改前形态。
#[cfg(test)]
mod a1_parse_fact_audit {
    use super::*;
    use crate::product::work_item_plan_compiler::CompilerDiagnostic;

    fn parse_diagnostic(field: &str, line: usize) -> CompilerDiagnostic {
        CompilerDiagnostic {
            code: "missing_section".to_string(),
            line,
            field: field.to_string(),
            message: "结构化 section 缺少必需字段。".to_string(),
            repair_example: format!("- {field}: value"),
            kind: None,
            lowering_fault: None,
            ir_fault: None,
        }
    }

    fn last_attempt_row(
        lifecycle: &LifecycleStore,
        session_id: &str,
    ) -> crate::product::lifecycle_store::candidate_attempts::CandidateAttemptRecord {
        lifecycle
            .list_candidate_attempts(session_id)
            .expect("读回流水行")
            .last()
            .expect("一行失败流水")
            .clone()
    }

    /// 临时目录 + store + engine（事件 rx 以 forget 保活，通道不得先关）。
    fn fixture() -> (
        tempfile::TempDir,
        LifecycleStore,
        crate::product::workspace_engine::WorkspaceEngine,
    ) {
        let tmp = tempfile::TempDir::new().expect("fixture tempdir");
        let lifecycle = LifecycleStore::new(crate::product::app_paths::ProductAppPaths::new(
            tmp.path().join(".aria"),
        ));
        let record = lifecycle
            .create_workspace_session(
                crate::product::lifecycle_store::CreateWorkspaceSessionInput {
                    project_id: "project_0001".to_string(),
                    issue_id: "issue_0001".to_string(),
                    entity_id: "work_item_plan_0001".to_string(),
                    workspace_type: crate::product::models::WorkspaceType::WorkItemPlan,
                    author_provider: crate::product::models::ProviderName::Codex,
                    reviewer_provider: Some(crate::product::models::ProviderName::ClaudeCode),
                    review_rounds: 2,
                    superpowers_enabled: true,
                    openspec_enabled: true,
                    work_item_plan_options: None,
                },
            )
            .expect("fixture session");
        let (event_tx, event_rx) = tokio::sync::mpsc::channel(8);
        std::mem::forget(event_rx); // 通道保活：rx 关闭会使引擎事件发送面失效
        let engine = WorkspaceEngine::new_persistent(
            std::sync::Arc::new(crate::product::checkpoint_store::CheckpointStore::new(
                tmp.path().join("checkpoints"),
            )),
            lifecycle.clone(),
            event_tx,
            crate::product::workspace_engine::WorkspaceSession::from_record(record),
        );
        (tmp, lifecycle, engine)
    }

    /// 完整五字段审计在标记分区在场、原缺失说明保留在前，且 `failure_id`
    /// 仍按 detail（不含审计文本）哈希——身份中性逐字可证。
    #[test]
    fn parse_fact_audit_lands_in_source_missing_reason_and_keeps_failure_identity() {
        let (_tmp, lifecycle, mut engine) = fixture();
        let detail = "missing_section:5:结构化 section 缺少必需字段。; missing_section:9:结构化 section 缺少必需字段。";
        record_generate_segment_compile_failure(
            &mut engine,
            &lifecycle,
            &[parse_diagnostic("Goal", 5), parse_diagnostic("role", 9)],
            detail,
        );
        let row = last_attempt_row(&lifecycle, &engine.session().session_id);
        let reason = row.source_missing_reason.as_deref().expect("缺失说明在场");
        assert!(
            reason.starts_with(detail),
            "原缺失说明必须保留在前: {reason}"
        );
        assert!(
            reason.contains("[sc_parse_fact_audit"),
            "审计分区必须显式标记: {reason}"
        );
        for (field, line) in [("Goal", 5), ("role", 9)] {
            assert!(
                reason.contains(&format!(
                    "missing_section|{line}|{field}|结构化 section 缺少必需字段。|- {field}: value"
                )),
                "同 message 不同 field 的五字段必须在审计面逐条可取回: {reason}"
            );
        }
        // 身份中性：failure_id 仍按 detail（不含审计）哈希，逐字不变。
        let series = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .ok()
            .and_then(|session| {
                crate::product::workspace_engine::current_sc_author_business_key(&session)
            })
            .unwrap_or_else(|| "no-launch".to_string());
        assert_eq!(
            row.failure.as_ref().expect("失败事实").failure_id,
            sc_sha256(&format!("{series}/Parse/missing_section/{detail}")),
            "审计文本不得进入被哈希的 detail"
        );
    }

    /// 非 parse 类失败不落审计分区——`source_missing_reason` 保持改前形态
    /// （仅 detail 本体）。
    #[test]
    fn compile_failure_without_parse_diagnostics_keeps_plain_missing_reason() {
        let (_tmp, lifecycle, mut engine) = fixture();
        let detail =
            "unknown_requirement_ref:30:task references unknown design requirement REQ-002";
        record_generate_segment_compile_failure(
            &mut engine,
            &lifecycle,
            &[CompilerDiagnostic {
                code: "unknown_requirement_ref".to_string(),
                line: 30,
                field: "Tasks.requirement_refs".to_string(),
                message: "task references unknown design requirement REQ-002".to_string(),
                repair_example: "- requirement_refs: REQ-001".to_string(),
                kind: None,
                lowering_fault: None,
                ir_fault: None,
            }],
            detail,
        );
        let row = last_attempt_row(&lifecycle, &engine.session().session_id);
        assert_eq!(
            row.source_missing_reason.as_deref(),
            Some(detail),
            "非 parse 类失败不落审计分区，保持改前形态"
        );
    }
}
