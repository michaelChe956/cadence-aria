//! P09：SC 失败证据 blocking reasons 三通道纯拼装（compile / IR / structure）。
//!
//! 本模块只做**事实投影**：不解析 message/code 文本、不重建 typed payload、
//! 不改写来源集合、不授予任何资格。教学资格只由
//! `single_candidate_compile_preflight::PreflightVerdict::to_teaching_verdict`
//! 判定——本模块产出 facts，资格由调用侧既有判据决定。
//!
//! 运行时接线（P09）：在既有资格/窗口判定之前把本轮完整事实写成 durable
//! `FailureObserved` 并严格读回，在 claim 之后、provider 启动之前把真实判定
//! （`TeachingDecision`）写读一次；顺序与既有 establish/漂移/claim 一字不改。
use super::single_candidate_compile_preflight::{PreflightVerdict, format_compile_failure_reasons};
use crate::product::planning_failure::{PlanningFailure, PlanningFailureKind, PlanningFailureSet};
use crate::product::work_item_plan_compiler::{
    CompilerDiagnostic, IrFault, IrFaultKind, IrFaultLocator,
};

use super::sc_failure_transport::ScBoundProviderRound;
use super::sc_teaching_audit::{
    ScFailureFacts, ScObservedFailure, audit_sc_teaching_prompt, failure_observed_key,
    observe_sc_teaching_decision, record_sc_prompt_unavailable, sc_failure_step_error,
    sc_teaching_target_ordinal,
};
use super::sc_teaching_prompt::{
    ScPreparedTeachingPrompt, ScTeachingPromptBudgetVerdict, prepare_compile_teaching_prompt,
    prepare_ir_teaching_prompt, prepare_structure_teaching_prompt,
};
use super::sc_teaching_source::{
    ScResolvedTeachingSource, ScTeachingSourceInput, resolve_sc_teaching_source,
    verify_sc_teaching_request_sources,
};
use super::single_candidate_compile_preflight::{TeachingVerdict, teaching_verdict};
use super::single_candidate_structure_reredrive::{
    ScTeachingChainBinding, ScTeachingStartVerify, authorize_sc_teaching_reredrive,
    compile_typed_kinds, sc_sha256, sc_teaching_failure_request, stable_code_kind,
    structure_source_digest,
};
use super::{ScEvidenceRef, ScRoundIdentity, SingleCandidateProviderRunError};
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::lifecycle_store::sc_failure_evidence::{
    ScFailureEvidenceError, ScFailureEvidenceEvent,
};
use crate::product::work_item_plan_policy::ScTeachingAttemptRef;
use crate::product::workspace_engine::PlanningCompileFailure;
use crate::product::workspace_engine::ScTeachingPromptError;
use crate::product::workspace_engine::WorkspaceEngine;
use crate::product::workspace_engine::single_candidate_structure_failure::{
    STRUCTURE_STABLE_CODE, legacy_author_structure_repair_body,
};
use crate::web::workspace_ws_types::{ScFailureChainIdentity, ScFailureClass, ScTeachingDecision};

/// 三臂的审计用构造器名（**不进 `prompt_ref`**——与 P22 facade 的 builder
/// id 同名同值，供既有 prompt 证据对照）。
const COMPILE_BUILDER_ID: &str = "build_typed_compile_reredrive_prompt";
const IR_BUILDER_ID: &str = "build_preflight_ir_reredrive_prompt";
const STRUCTURE_BUILDER_ID: &str = "author_structure_repair_prompt";

/// 教学 prompt 超预算的稳定码（真实未请求消费窗口，不是假 Denied）。
const SC_TEACHING_PROMPT_BUDGET_EXCEEDED: &str = "sc_teaching_prompt_budget_exceeded";

/// compile 失败面：逐条 `code:line:message`（复用既有单源形态；顺序即
/// diagnostics 顺序，不去重、不排序）后**全量**追加修复指令行——不设呈现
/// 上限、不折叠计数，第 N 条指令逐字在场（呈现截断仍由
/// `compile_failure_surface` 单独负责，本通道不截断）。
pub(super) fn compile_blocking_reasons(
    diagnostics: &[CompilerDiagnostic],
    instructions: &[String],
) -> Vec<String> {
    let mut reasons = format_compile_failure_reasons(diagnostics);
    reasons.extend(instructions.iter().cloned());
    reasons
}

/// IR 预检失败面：operational 在前、呈现原文在后（与
/// `PreflightVerdict::presentation_reasons` 同序）。两文本面皆空且 typed
/// 事实非空时，才回落到 typed 事实的稳定渲染——**有事实 ≠ 有资格**：
/// 不可定位的 typed 事实仍由 `to_teaching_verdict` 判 `ir_fault_not_locatable`。
pub(super) fn ir_blocking_reasons(preflight: &PreflightVerdict) -> Vec<String> {
    let mut reasons = preflight.operational_failures.clone();
    reasons.extend(preflight.failure_reasons.iter().cloned());
    if reasons.is_empty() && !preflight.ir_faults.is_empty() {
        reasons = preflight
            .ir_faults
            .iter()
            .map(render_ir_fault_fact)
            .collect();
    }
    reasons
}

/// 结构失败面：按 producer 顺序逐条渲染（精确重复条目重复输出，不去重、
/// 不排序），只投影 typed 事实，集合本体零改写。
pub(super) fn structure_blocking_reasons(set: &PlanningFailureSet) -> Vec<String> {
    set.failures.iter().map(render_structure_failure).collect()
}

/// typed IR 事实的单行渲染：保留 work item 身份、定位身份、出错引用、
/// 合法集合（原序）、期望形态；行号**仅在 `source_line` 在场时**写出，
/// 缺行不伪造 0/占位。渲染确定：同一事实两次渲染逐字相等。
fn render_ir_fault_fact(fault: &IrFault) -> String {
    let locator = match &fault.locator {
        IrFaultLocator::Task { task_id } => format!("task {task_id}"),
        IrFaultLocator::Handoff => "handoff".to_string(),
        IrFaultLocator::Criterion { criterion_id } => format!("criterion {criterion_id}"),
    };
    let mut fact = format!(
        "ir_fault {} work_item={} locator={} offending_ref={} legal_refs=[{}]",
        ir_fault_kind_code(&fault.kind),
        fault.work_item_id,
        locator,
        fault.offending_ref,
        fault.legal_refs.join("、"),
    );
    if let Some(line) = fault.source_line {
        fact.push_str(&format!(" source_line={line}"));
    }
    fact.push_str(&format!(" expected_shape={}", fault.expected_shape));
    fact
}

/// 四类 IR 故障的稳定码名（与 producer 写入的 diagnostic code 同名；枚举
/// 变体 → 码名的显式映射，不从 message 反解析）。既有同名单源在
/// `workspace_engine::prompts::sc_compile_teaching`（模块私有），P09 三文件
/// 闭合内不得改它，故此处显式映射同名字符串。
fn ir_fault_kind_code(kind: &IrFaultKind) -> &'static str {
    match kind {
        IrFaultKind::UnknownDoneWhenRef => "unknown_done_when_ref",
        IrFaultKind::UnknownReviewerCheckRef => "unknown_reviewer_check_ref",
        IrFaultKind::AcceptanceCriterionWithoutReviewerCheck => {
            "acceptance_criterion_without_reviewer_check"
        }
        IrFaultKind::UnknownRequirementRef => "unknown_requirement_ref",
    }
}

/// 结构失败的单行渲染：稳定码 + origin/stage + 诊断引用 + 受影响 ID
/// （producer 原序，重复保留）+ details 每对 key/value（BTreeMap 键序）。
/// 不解析 `Operational { stable_code, cause }` 的 cause 文本、不从 prose 造
/// id、不重封集合——渲染是来源集合的纯函数。
fn render_structure_failure(failure: &PlanningFailure) -> String {
    let mut line = format!(
        "planning_failure {} origin={:?} stage={:?} diagnostic_ref={}",
        failure.kind.stable_code(),
        failure.origin,
        failure.stage,
        failure.diagnostic_ref,
    );
    if !failure.evidence.affected_ids.is_empty() {
        line.push_str(&format!(
            " affected_ids=[{}]",
            failure.evidence.affected_ids.join("、")
        ));
    }
    if !failure.evidence.details.is_empty() {
        let details = failure
            .evidence
            .details
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join(", ");
        line.push_str(&format!(" details=[{details}]"));
    }
    line
}

// P09：运行时接线（FailureObserved → 真实判定 → 原窗口顺序不变）。
//
// 每失败轮恰一次的顺序契约：完整本轮事实 → FailureObserved 追加+严格读回 →
// 既有资格/窗口（establish → 漂移核验 → claim，一字不改）→ 真实判定追加+
// 严格读回 → 调用方在 grant 后启动重驱 provider。判定只形成一次：store
// 写读错误 ⇒ 不写任何 decision，fail-closed 上浮。

/// 已形成的重驱授予：返回时判定已 durable 记录（append + 严格读回）。
///
/// `reredrive_prompt` 是**已落盘并严格读回**的最终全文（全文只装配一次、
/// 只落一次、只读一次——此处不再重建）；`verify` 携带启动前复验所需的
/// 冻结事实（失败源引用面 + 已核验的窗口请求 + prompt 证据）。
#[derive(Debug)]
pub(super) struct ScTeachingGrant {
    pub attempt: ScTeachingAttemptRef,
    pub reredrive_prompt: String,
    pub verify: ScTeachingStartVerify,
}

/// compile 通道纯 facts（无 I/O）：诊断原文 + **未截断**指令全集；
/// source 与 raw 同 locator，摘要/字节数归一化 compile 正文。
pub(super) fn compile_failure_facts(
    diagnostics: &[CompilerDiagnostic],
    instruction_lines: Vec<String>,
    raw: &ScEvidenceRef,
    compile_source: &str,
    round: &ScRoundIdentity,
) -> ScFailureFacts {
    let blocking_reasons = compile_blocking_reasons(diagnostics, &instruction_lines);
    ScFailureFacts {
        failure_class: ScFailureClass::Compile,
        blocking_reasons,
        repair_instructions: instruction_lines,
        raw: raw.clone(),
        source: round_source_ref(raw, compile_source, &round.round_id),
        existing_failure_id: None,
        attempt_key: None,
    }
}

/// IR 预检通道纯 facts：两文本面（operational → 呈现原文）优先，typed-only
/// 才稳定渲染；IR 无独立指令行（修复文本是 prompt 常量尾段），双计禁止。
pub(super) fn ir_failure_facts(
    preflight: &PreflightVerdict,
    raw: &ScEvidenceRef,
    compile_source: &str,
    round: &ScRoundIdentity,
) -> ScFailureFacts {
    ScFailureFacts {
        failure_class: ScFailureClass::IrPreflight,
        blocking_reasons: ir_blocking_reasons(preflight),
        repair_instructions: Vec::new(),
        raw: raw.clone(),
        source: round_source_ref(raw, compile_source, &round.round_id),
        existing_failure_id: None,
        attempt_key: None,
    }
}

/// 结构门通道纯 facts：原 typed 失败集逐条渲染（sidecar 原样，不重算）；
/// source 由调用方给**真实**引用面（sealed canonical revision ref + 该
/// revision 的真值摘要/字节；缺 ref 或读不到 ⇒ 显式 Missing，绝不借 split
/// locator 冒充 revision）；existing_failure_id 取 seal 后的内容寻址 id。
pub(super) fn structure_failure_facts(
    failure: &PlanningCompileFailure,
    structure_failures: &PlanningFailureSet,
    raw: &ScEvidenceRef,
    source: ScEvidenceRef,
) -> ScFailureFacts {
    ScFailureFacts {
        failure_class: ScFailureClass::StructureGate,
        blocking_reasons: structure_blocking_reasons(structure_failures),
        repair_instructions: Vec::new(),
        raw: raw.clone(),
        source,
        existing_failure_id: Some(failure.failure_id.clone()),
        attempt_key: None,
    }
}

/// FailureObserved 追加 → 严格读回 → 读回行必须逐字等于本轮事件（否则
/// store 稳定码上浮）。成功返回的 `entry` 是读回行，不是内存副本。
pub(super) fn observe_sc_failure(
    lifecycle: &LifecycleStore,
    chain: &ScFailureChainIdentity,
    round: &ScRoundIdentity,
    facts: ScFailureFacts,
) -> Result<ScObservedFailure, ScFailureEvidenceError> {
    const STEP: &str = "observe_sc_failure";
    let event = ScFailureEvidenceEvent::FailureObserved {
        round_id: round.round_id.clone(),
        failure_class: Some(facts.failure_class.clone()),
        blocking_reasons: facts.blocking_reasons.clone(),
        repair_instructions: facts.repair_instructions.clone(),
        raw: facts.raw.clone(),
        source: facts.source.clone(),
        existing_failure_id: facts.existing_failure_id.clone(),
        attempt_key: facts.attempt_key.clone(),
    };
    lifecycle.append_sc_failure_evidence(chain, &event)?;
    let read = lifecycle.read_sc_failure_evidence(chain)?;
    let key = failure_observed_key(&round.round_id);
    let entry = read
        .entries
        .iter()
        .find(|entry| entry.entry_key == key)
        .ok_or_else(|| {
            sc_failure_step_error(
                "sc_failure_observed_readback_missing",
                STEP,
                Some(round.round_id.as_str()),
                "本轮 FailureObserved 未出现在严格读回中",
            )
        })?;
    if entry.event != event {
        return Err(sc_failure_step_error(
            "sc_failure_entry_conflict",
            STEP,
            Some(round.round_id.as_str()),
            "本轮 FailureObserved 严格读回与写入正文不一致",
        ));
    }
    Ok(ScObservedFailure {
        facts,
        round: round.clone(),
        entry: entry.clone(),
    })
}

/// compile 失败门：事实写读 → 既有纯判据（raw 归档 + Eligible）→ 三臂共用的
/// 教学链接线（真实失败源 → 完整 prompt 审计 → 预算裁决 → 既有窗口 claim
/// → 真实判定写读）→ 返回 grant（调用方唯一启动点）。
#[allow(clippy::too_many_arguments)]
#[allow(clippy::result_large_err)] // 运行收尾的唯一 typed 出口（同既有先例）。
pub(super) fn gate_compile_failure(
    engine: &mut WorkspaceEngine,
    lifecycle: &LifecycleStore,
    bound: &ScBoundProviderRound,
    previous: Option<&(ScEvidenceRef, ScRoundIdentity)>,
    compile_source: &str,
    diagnostics: &[CompilerDiagnostic],
    instruction_lines: Vec<String>,
    binding: &ScTeachingChainBinding,
    design_requirement_ids: &[String],
    detail: &str,
) -> Result<Option<ScTeachingGrant>, SingleCandidateProviderRunError> {
    // 摘要用原 Vec 现算（在 move 进 repair_instructions 之前；与原式逐字同值，
    // 不依赖读回文本）。
    let instruction_digest = sc_sha256(&instruction_lines.join("\n"));
    let (raw, round) = current_round_parts(&mut *engine, previous, detail)?;
    let facts = compile_failure_facts(diagnostics, instruction_lines, raw, compile_source, round);
    let observed = observe_sc_failure(lifecycle, &bound.chain, round, facts)
        .map_err(|error| evidence_store_failure(&mut *engine, &error, detail))?;
    if !raw_archived(&observed.facts.raw) {
        observe_sc_teaching_decision(
            lifecycle,
            &bound.chain,
            &observed,
            ScTeachingDecision::NotRequested,
        )
        .map_err(|error| evidence_store_failure(&mut *engine, &error, detail))?;
        return Ok(None);
    }
    // 资格判据零改动（typed 完备才授资格；失败源不可用不产生资格）。
    let payload = match teaching_verdict(diagnostics, &[]) {
        TeachingVerdict::NotEligible { .. } => {
            observe_sc_teaching_decision(
                lifecycle,
                &bound.chain,
                &observed,
                ScTeachingDecision::Ineligible,
            )
            .map_err(|error| evidence_store_failure(&mut *engine, &error, detail))?;
            return Ok(None);
        }
        TeachingVerdict::Eligible { payload } => payload,
    };
    run_sc_teaching_chain(
        &mut *engine,
        lifecycle,
        bound,
        &observed,
        binding,
        COMPILE_BUILDER_ID,
        |source| {
            prepare_compile_teaching_prompt(
                &payload,
                &observed.facts.repair_instructions,
                design_requirement_ids,
                source,
            )
        },
        compile_typed_kinds(diagnostics),
        instruction_digest,
        detail,
    )
}

/// IR 预检失败门：与 compile 同序；资格来自 `PreflightVerdict::to_teaching_verdict`。
#[allow(clippy::too_many_arguments)]
#[allow(clippy::result_large_err)]
pub(super) fn gate_ir_failure(
    engine: &mut WorkspaceEngine,
    lifecycle: &LifecycleStore,
    bound: &ScBoundProviderRound,
    previous: Option<&(ScEvidenceRef, ScRoundIdentity)>,
    compile_source: &str,
    preflight: &PreflightVerdict,
    binding: &ScTeachingChainBinding,
    design_requirement_ids: &[String],
    detail: &str,
) -> Result<Option<ScTeachingGrant>, SingleCandidateProviderRunError> {
    let (raw, round) = current_round_parts(&mut *engine, previous, detail)?;
    let facts = ir_failure_facts(preflight, raw, compile_source, round);
    let observed = observe_sc_failure(lifecycle, &bound.chain, round, facts)
        .map_err(|error| evidence_store_failure(&mut *engine, &error, detail))?;
    if !raw_archived(&observed.facts.raw) {
        observe_sc_teaching_decision(
            lifecycle,
            &bound.chain,
            &observed,
            ScTeachingDecision::NotRequested,
        )
        .map_err(|error| evidence_store_failure(&mut *engine, &error, detail))?;
        return Ok(None);
    }
    match preflight.to_teaching_verdict() {
        TeachingVerdict::NotEligible { .. } => {
            observe_sc_teaching_decision(
                lifecycle,
                &bound.chain,
                &observed,
                ScTeachingDecision::Ineligible,
            )
            .map_err(|error| evidence_store_failure(&mut *engine, &error, detail))?;
            Ok(None)
        }
        TeachingVerdict::Eligible { .. } => run_sc_teaching_chain(
            &mut *engine,
            lifecycle,
            bound,
            &observed,
            binding,
            IR_BUILDER_ID,
            |source| prepare_ir_teaching_prompt(preflight, design_requirement_ids, source),
            vec![stable_code_kind("typed_ir_validation")],
            sc_sha256(&preflight.presentation_reasons().join("\n")),
            detail,
        ),
    }
}

/// 结构门失败门：无独立资格判据（原行为：窗口 claim 即判定），raw 未归档
/// 显式 NotRequested；revised source 只取 sealed canonical revision ref +
/// 该 revision 的真实摘要/字节（split locator 冒充 ⇒ 拒绝）。
#[allow(clippy::too_many_arguments)]
#[allow(clippy::result_large_err)]
pub(super) fn gate_structure_failure(
    engine: &mut WorkspaceEngine,
    lifecycle: &LifecycleStore,
    bound: &ScBoundProviderRound,
    previous: Option<&(ScEvidenceRef, ScRoundIdentity)>,
    failure: &PlanningCompileFailure,
    structure_failures: &PlanningFailureSet,
    binding: &ScTeachingChainBinding,
    detail: &str,
) -> Result<Option<ScTeachingGrant>, SingleCandidateProviderRunError> {
    let (raw, round) = current_round_parts(&mut *engine, previous, detail)?;
    // 事实面的 source 必须是该轮 sealed 的 canonical revision 真值：缺 ref
    // 或该 revision 不可读 ⇒ 显式 Missing（事件级
    // `sc_failure_source_ref_missing`），绝不半截事实。
    let source = match &failure.source_revision_ref {
        Some(locator) => match structure_source_digest(lifecycle, &bound.chain.scope, locator) {
            Ok((sha256, bytes)) => ScEvidenceRef::Available {
                locator: locator.clone(),
                sha256,
                bytes,
            },
            Err(error) => ScEvidenceRef::Missing {
                code: error.code,
                detail: error.detail,
                round_id: round.round_id.clone(),
            },
        },
        None => ScEvidenceRef::Missing {
            code: "sc_failure_source_ref_missing".to_string(),
            detail: "结构失败缺少 source_revision_ref，不能借用 split locator 冒充 revision"
                .to_string(),
            round_id: round.round_id.clone(),
        },
    };
    let facts = structure_failure_facts(failure, structure_failures, raw, source);
    let observed = observe_sc_failure(lifecycle, &bound.chain, round, facts)
        .map_err(|error| evidence_store_failure(&mut *engine, &error, detail))?;
    if !raw_archived(&observed.facts.raw) {
        observe_sc_teaching_decision(
            lifecycle,
            &bound.chain,
            &observed,
            ScTeachingDecision::NotRequested,
        )
        .map_err(|error| evidence_store_failure(&mut *engine, &error, detail))?;
        return Ok(None);
    }
    run_sc_teaching_chain(
        &mut *engine,
        lifecycle,
        bound,
        &observed,
        binding,
        STRUCTURE_BUILDER_ID,
        |source| prepare_structure_teaching_prompt(failure, structure_failures, source),
        vec![stable_code_kind(STRUCTURE_STABLE_CODE)],
        // 指令摘要取 legacy 指令 body（与今日同文本 ⇒ 同值；不含 source 与
        // typed 全因）。
        sc_sha256(&legacy_author_structure_repair_body(failure)),
        detail,
    )
}

/// 三臂共用的教学链接线（唯一实现）：真实失败源解析 → 完整 prompt 装配 →
/// 不可变落盘 + 严格读回 → 预算裁决 → 窗口请求（授权前复验失败源）→ 既有
/// 窗口 claim → 真实判定写读 → 返回 grant。
///
/// 顺序不可换：任何 claim/启动之前，失败源、完整 prompt 与预算裁决必须已经
/// durable 且读回核对过；预算拒绝 ⇒ 显式 `NotRequested`、零 claim、零发送、
/// 零启动；失败源/渲染/审计失败 ⇒ 真实缺提示记录 + 显式 `NotRequested`，
/// 再以稳定码上浮（终局由调用方的终局处理器组包——本函数不吞终局）。
#[allow(clippy::too_many_arguments)]
#[allow(clippy::result_large_err)] // 运行收尾的唯一 typed 出口（同既有先例）。
fn run_sc_teaching_chain(
    engine: &mut WorkspaceEngine,
    lifecycle: &LifecycleStore,
    bound: &ScBoundProviderRound,
    observed: &ScObservedFailure,
    binding: &ScTeachingChainBinding,
    builder_id: &str,
    prepare: impl FnOnce(
        &ScResolvedTeachingSource,
    ) -> Result<ScPreparedTeachingPrompt, ScTeachingPromptError>,
    failure_typed_kinds: Vec<PlanningFailureKind>,
    instruction_digest: String,
    detail: &str,
) -> Result<Option<ScTeachingGrant>, SingleCandidateProviderRunError> {
    // 失败源输入只四个引用（不构造 `ScRoundEvidence`、不伪教学决定）。
    let input = ScTeachingSourceInput {
        identity: &observed.round,
        failure_class: &observed.facts.failure_class,
        raw: &observed.facts.raw,
        source: &observed.facts.source,
    };
    let resolved = match resolve_sc_teaching_source(lifecycle, &bound.chain.scope, &input) {
        Ok(resolved) => resolved,
        Err(error) => {
            return Err(fail_sc_teaching_chain(
                &mut *engine,
                lifecycle,
                &bound.chain,
                observed,
                builder_id,
                &error.code,
                &error.detail,
                detail,
            ));
        }
    };
    let prepared = match prepare(&resolved) {
        Ok(prepared) => prepared,
        Err(error) => {
            return Err(fail_sc_teaching_chain(
                &mut *engine,
                lifecycle,
                &bound.chain,
                observed,
                builder_id,
                &error.code,
                &error.detail,
                detail,
            ));
        }
    };
    // prompt 审计：全文落盘 + 严格读回（全文只装配/落盘/读回这一次）。
    let prompt_evidence = match audit_sc_teaching_prompt(
        lifecycle,
        &bound.chain,
        &observed.round.round_id,
        sc_teaching_target_ordinal(&observed.round),
        &prepared,
    ) {
        Ok(evidence) => evidence,
        Err(error) => {
            return Err(fail_sc_teaching_chain(
                &mut *engine,
                lifecycle,
                &bound.chain,
                observed,
                builder_id,
                &error.code,
                &error.detail,
                detail,
            ));
        }
    };
    // 预算裁决：超限 ⇒ 全文已写已读（可审计）+ 显式 `NotRequested`（真实
    // 未请求消费窗口——不是假 Denied），零 claim、零发送、零启动。
    if let ScTeachingPromptBudgetVerdict::RejectedBudget {
        required_bytes,
        max_bytes,
    } = &prepared.budget_verdict
    {
        return Err(reject_sc_teaching_budget(
            &mut *engine,
            lifecycle,
            &bound.chain,
            observed,
            *required_bytes,
            *max_bytes,
            detail,
        ));
    }
    let request = match sc_teaching_failure_request(
        binding,
        &resolved,
        failure_typed_kinds,
        &prompt_evidence,
        instruction_digest,
    ) {
        Ok(request) => request,
        Err(error) => {
            return Err(fail_sc_teaching_chain(
                &mut *engine,
                lifecycle,
                &bound.chain,
                observed,
                builder_id,
                &error.code,
                &error.detail,
                detail,
            ));
        }
    };
    // 授权前硬前置：请求引用的失败源必须与真实失败源一致（错配 ⇒ 零 claim）。
    if let Err(error) =
        verify_sc_teaching_request_sources(lifecycle, &bound.chain.scope, &input, &request)
    {
        return Err(fail_sc_teaching_chain(
            &mut *engine,
            lifecycle,
            &bound.chain,
            observed,
            builder_id,
            &error.code,
            &error.detail,
            detail,
        ));
    }
    let (decision, attempt) = authorize_sc_teaching_reredrive(lifecycle, &mut *engine, &request)?;
    observe_sc_teaching_decision(lifecycle, &bound.chain, observed, decision)
        .map_err(|error| evidence_store_failure(&mut *engine, &error, detail))?;
    Ok(attempt.map(|attempt| ScTeachingGrant {
        attempt,
        reredrive_prompt: prepared.text,
        verify: ScTeachingStartVerify {
            identity: observed.round.clone(),
            failure_class: observed.facts.failure_class.clone(),
            raw: observed.facts.raw.clone(),
            source: observed.facts.source.clone(),
            request,
            prompt_evidence,
        },
    }))
}

/// 教学链失败的统一收口（真实未请求面）：先落**真实**的缺提示记录
/// （`PromptUnavailable` + `Missing` 真实原因）与显式 `NotRequested` 判定
/// （真实未请求消费窗口——不是假 Denied），再把带稳定码的错误上浮；终局由
/// 调用方的终局处理器组包（本函数不吞终局、不静默降级）。
#[allow(clippy::result_large_err)] // 运行收尾的唯一 typed 出口（同既有先例）。
#[allow(clippy::too_many_arguments)] // 统一失败收口：每个参数都是必须原样上交的独立真实事实。
fn fail_sc_teaching_chain(
    engine: &mut WorkspaceEngine,
    lifecycle: &LifecycleStore,
    chain: &ScFailureChainIdentity,
    observed: &ScObservedFailure,
    builder_id: &str,
    code: &str,
    cause: &str,
    original: &str,
) -> SingleCandidateProviderRunError {
    if let Err(error) = record_sc_prompt_unavailable(
        lifecycle,
        chain,
        &observed.round.round_id,
        sc_teaching_target_ordinal(&observed.round),
        builder_id,
        code,
        cause,
    ) {
        return evidence_store_failure(engine, &error, original);
    }
    if let Err(error) =
        observe_sc_teaching_decision(lifecycle, chain, observed, ScTeachingDecision::NotRequested)
    {
        return evidence_store_failure(engine, &error, original);
    }
    SingleCandidateProviderRunError::Message(format!("{code}: {cause}；原始失败原因：{original}"))
}

/// 预算拒绝的收口：全文已写已读（可审计）⇒ 只补显式 `NotRequested` 判定
/// （真实未请求消费窗口），零 claim、零发送、零启动。
#[allow(clippy::result_large_err)] // 运行收尾的唯一 typed 出口（同既有先例）。
fn reject_sc_teaching_budget(
    engine: &mut WorkspaceEngine,
    lifecycle: &LifecycleStore,
    chain: &ScFailureChainIdentity,
    observed: &ScObservedFailure,
    required_bytes: u64,
    max_bytes: u64,
    original: &str,
) -> SingleCandidateProviderRunError {
    if let Err(error) =
        observe_sc_teaching_decision(lifecycle, chain, observed, ScTeachingDecision::NotRequested)
    {
        return evidence_store_failure(engine, &error, original);
    }
    SingleCandidateProviderRunError::Message(format!(
        "{SC_TEACHING_PROMPT_BUDGET_EXCEEDED}: required_bytes={required_bytes} \
         max_bytes={max_bytes} round={}；原始失败原因：{original}",
        observed.round.round_id
    ))
}

/// 当前轮（raw + 轮身份）绑定：缺失按 Result 稳定码上浮（persist Failed +
/// Message），不 expect、不伪造轮身份。
#[allow(clippy::result_large_err)] // 运行收尾的唯一 typed 出口（同既有先例）。
fn current_round_parts<'a>(
    engine: &mut WorkspaceEngine,
    previous: Option<&'a (ScEvidenceRef, ScRoundIdentity)>,
    detail: &str,
) -> Result<(&'a ScEvidenceRef, &'a ScRoundIdentity), SingleCandidateProviderRunError> {
    match previous {
        Some((raw, round)) => Ok((raw, round)),
        None => {
            let error = sc_failure_step_error(
                "sc_failure_round_not_bound",
                "observe_sc_failure",
                None,
                "SC 失败观察缺少当前轮绑定",
            );
            Err(evidence_store_failure(engine, &error, detail))
        }
    }
}

/// raw 是否已真实归档（仅 `Available` 视为有证据；`Missing` 不是证据）。
fn raw_archived(raw: &ScEvidenceRef) -> bool {
    matches!(raw, ScEvidenceRef::Available { .. })
}

/// compile/IR 的 source 引用：与 raw 同 locator，摘要/字节数为归一化正文
/// 的真值；raw 缺失时 source 也显式缺失（本轮身份，不借别轮）。
fn round_source_ref(raw: &ScEvidenceRef, source: &str, round_id: &str) -> ScEvidenceRef {
    match raw {
        ScEvidenceRef::Available { locator, .. } => ScEvidenceRef::Available {
            locator: locator.clone(),
            sha256: sc_sha256(source),
            bytes: source.len() as u64,
        },
        ScEvidenceRef::Missing { code, detail, .. } => ScEvidenceRef::Missing {
            code: code.clone(),
            detail: detail.clone(),
            round_id: round_id.to_string(),
        },
    }
}

/// 证据写读失败的统一收口：persist Failed + Message，保留 store code/step
/// 与原失败原文（不吞原始原因、不新增启动）。
#[allow(clippy::result_large_err)] // 运行收尾的唯一 typed 出口（同既有先例）。
fn evidence_store_failure(
    engine: &mut WorkspaceEngine,
    error: &ScFailureEvidenceError,
    detail: &str,
) -> SingleCandidateProviderRunError {
    engine.persist_single_candidate_terminal_phase(
        crate::product::models::SingleCandidatePhase::Failed,
    );
    SingleCandidateProviderRunError::Message(format!(
        "SC 失败证据写读失败：{error}；原始失败原因：{detail}"
    ))
}

/// P10：终局（ChainTerminal）接线——独立子模块（P19d 在该子模块实现真实
/// 组包与发布；本文件只留事实投影与三臂教学链接线）。
#[path = "sc_failure_terminal.rs"]
pub(super) mod sc_failure_terminal;

/// 终局面的窄再导出：既有调用点（transport 与 run/single_candidate）按原
/// 路径取用；真实组包/发布在子模块内实现，不在本文件复制第二份。
pub(super) use sc_failure_terminal::{
    SC_TERMINAL_CLASS_TEACHING_INTERRUPTED, teaching_launch_observations, terminal_exit_error,
};

#[cfg(test)]
#[path = "sc_failure_evidence_tests/adapter_facts.rs"]
mod tests;

#[cfg(test)]
#[path = "sc_failure_evidence_consumer_tests.rs"]
mod consumer_tests;
