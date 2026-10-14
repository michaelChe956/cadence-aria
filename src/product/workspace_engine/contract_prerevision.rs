//! F5 回灌扩展：canonical 契约机械校验前移进 SC 修订循环（3.6 矩阵 codex×重 根治）。
//!
//! 根因（codex×重 6 连败）：上游 WI 输出契约 capabilities 不逐字覆盖下游
//! require_all——该缺口此前只在 Approval 阶段的 initial plan publication compile
//! （`plan_projection.rs` 的 `prepare_initial_plan_publication`）里由
//! `validate_dependency_contract_graph` 机械比对发现；修订循环内的 reviewer
//! （flash）抓不住这类缺口，循环空转收敛后仍要等到 Approval compile 才失败落人工门。
//!
//! 本模块把同一份校验器前移到 SC author 每轮落盘时
//! （`complete_single_candidate_work_item_plan_author`，`validate_plan_candidate_ir`
//! 之后、`put_mechanical_report`/CAS 之前）：对 IR 既有 canonical 契约
//! （`PlanCandidateIr.items[].contract`）`build_dependency_contract_graph` →
//! `validate_dependency_contract_graph`，Error 级 findings 适配成机械
//! `ReviewVerdict`（verdict=Revise、review_gate=RequiresRevision），经
//! `complete_review` 既有 ingestion 路由（与 reviewer verdict 完全同一条链）：
//!
//! - F5-A 回灌：`complete_review` 注入 `latest_review_verdict`，下一轮 SC author
//!   重跑由 `single_candidate_pending_revision_verdict` 判定为修订轮，机械
//!   findings 逐条回灌进返修 prompt（`[review_findings]`，required_action 模板
//!   给出可直接照抄的逐字补丁行「在 provider X 的 Outputs 契约 Y 的
//!   capabilities 列表追加一行（逐字复制）：- capability Z」，capability 原串
//!   与 finding 的 capability_ref 逐字同源）。
//! - 预算/收敛：policy 路由把机械缺口按 `ContractGap`+`Repairable` 分类进
//!   `TriggerAggregateRepair`（`repairs_used` 计数与 reviewer 返修轮同池），
//!   自动重驱 SC author；连续 2 轮同指纹（`FindingFingerprint` 与 F5-B 闸门
//!   同一 `classify_finding` 口径）由既有 `RepeatedFingerprint` 人工门兜底，
//!   不新增任何终止逻辑。
//! - Warning 级 findings 以 Suggestion+Advisory 拼进同一 verdict（可见但不参与
//!   闸门与预算）；首轮无 Error 缺口路径零变化（不产生 verdict、不改路由）。
//!
//! 确定性契约：`validate_dependency_contract_graph` 的 findings 经
//! `sorted_report` 稳定排序，本模块的 message/evidence/required_action 均为
//! 确定性 format（无时间戳/随机源），同输入同文本——保证跨轮指纹稳定，
//! 闸门语义可复现。

use crate::product::work_item_contract::{
    CanonicalWorkItemContract, ContractFindingSeverity, ContractValidationFinding,
    ContractValidationReport, DependencyContractGraph, build_dependency_contract_graph,
    validate_dependency_contract_graph,
};
use crate::product::work_item_plan_compiler::PlanCandidateIr;
use crate::product::work_item_plan_policy::{FindingClassHint, ReviewFindingCategory};
use crate::web::workspace_ws_types::{
    MachineRepairDto, MachineRepairStatus, ReviewFinding, ReviewFindingSeverity, ReviewGate,
    ReviewVerdict, ReviewVerdictType,
};

/// 对 SC 本轮 IR 的 canonical 契约跑 approval 同源机械校验；存在 Error 级
/// findings 时返回机械返修 verdict，否则返回 None（首轮/干净候选零变化）。
pub(crate) fn single_candidate_contract_prerevision_verdict(
    ir: &PlanCandidateIr,
) -> Option<ReviewVerdict> {
    let contracts = ir
        .items
        .iter()
        .map(|item| item.contract.clone())
        .collect::<Vec<_>>();
    contract_prerevision_verdict_for(&contracts)
}

fn contract_prerevision_verdict_for(
    contracts: &[CanonicalWorkItemContract],
) -> Option<ReviewVerdict> {
    let graph = match build_dependency_contract_graph(contracts) {
        Ok(graph) => graph,
        Err(report) => return verdict_from_report(&report, None),
    };
    let report = validate_dependency_contract_graph(&graph);
    verdict_from_report(&report, Some(&graph))
}

/// Error 级 findings 存在时才产生机械返修 verdict；Warning 级仅在已有
/// verdict 时搭车（干净/仅告警候选零变化）。
fn verdict_from_report(
    report: &ContractValidationReport,
    graph: Option<&DependencyContractGraph>,
) -> Option<ReviewVerdict> {
    let has_error = report
        .findings
        .iter()
        .any(|finding| finding.severity == ContractFindingSeverity::Error);
    if !has_error {
        return None;
    }
    // validate_dependency_contract_graph 的 findings 已由 sorted_report 稳定
    // 排序；适配层保持同序，保证同输入同文本。
    let findings = report
        .findings
        .iter()
        .map(|finding| contract_finding_to_review_finding(finding, graph, None))
        .collect::<Vec<_>>();
    let error_count = findings
        .iter()
        .filter(|finding| finding.severity == ReviewFindingSeverity::MustFix)
        .count();
    let summary = format!("canonical 契约机械校验发现 {error_count} 项 Error 级缺口，候选必须返修");
    let mut comments = String::new();
    comments.push_str(&format!(
        "canonical 契约机械校验（validate_dependency_contract_graph，与 Approval compile 同源）发现 {error_count} 项 Error 级缺口：\n"
    ));
    for finding in &findings {
        if finding.severity == ReviewFindingSeverity::MustFix {
            comments.push_str(&format!("- {}\n", finding.message));
        }
    }
    comments
        .push_str("以上缺口均为机械比对结论（非 reviewer 判断）；逐条按 required_action 修复。");
    Some(ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments,
        summary,
        findings,
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    })
}

/// 机械 verdict 的确定性可读全文（`complete_review` 的
/// `record_review_message` 载体，进 timeline 消息流供人审阅）。
pub(crate) fn contract_prerevision_readable_output(verdict: &ReviewVerdict) -> String {
    let mut output = String::new();
    output.push_str("[contract_prerevision] 机械 canonical 契约校验未通过。\n");
    for finding in &verdict.findings {
        output.push_str(&format!(
            "- [{}] {}\n",
            match finding.severity {
                ReviewFindingSeverity::Blocking => "blocking",
                ReviewFindingSeverity::MustFix => "must_fix",
                ReviewFindingSeverity::Suggestion => "suggestion",
            },
            finding.message
        ));
    }
    output
}

/// `ContractValidationFinding` → `ReviewFinding` 适配：
/// - Error → MustFix + category=ContractGap + class_hint=Repairable（policy 侧
///   归入自动返修通道，与 reviewer 契约缺口同池消费预算）；
/// - Warning → Suggestion + class_hint=Advisory（advisory：不参与指纹闸门/预算）；
/// - `code` 拼进 message 前缀（机械可识别）；`logical_work_item_id`/
///   `contract_ref`/`capability_ref` 拼进 evidence；`required_action` 按 code
///   模板化（provider 从 graph 反查补全）。
fn contract_finding_to_review_finding(
    finding: &ContractValidationFinding,
    graph: Option<&DependencyContractGraph>,
    machine_repair: Option<MachineRepairDto>,
) -> ReviewFinding {
    let logical_id = finding.logical_work_item_id.as_deref();
    let contract_ref = finding.contract_ref.as_deref();
    let capability_ref = finding.capability_ref.as_deref();
    let provider = graph.and_then(|graph| {
        let consumer = logical_id?;
        let contract_id = contract_ref?;
        provider_for_consumer_contract(graph, consumer, contract_id)
    });
    let (severity, category, class_hint) = match finding.severity {
        ContractFindingSeverity::Error => (
            ReviewFindingSeverity::MustFix,
            Some(ReviewFindingCategory::ContractGap),
            Some(FindingClassHint::Repairable),
        ),
        ContractFindingSeverity::Warning => (
            ReviewFindingSeverity::Suggestion,
            None,
            Some(FindingClassHint::Advisory),
        ),
    };
    ReviewFinding {
        severity,
        message: format!("{}: {}", finding.code, finding.message),
        evidence: evidence_text(logical_id, contract_ref, capability_ref),
        required_action: required_action_text(finding, provider),
        category,
        class_hint,
        contract_field: contract_field_text(finding),
        // SG7（§0.6.3）：machine_repair 只由服务端已证 L1 的同 source/
        // context 显式配对产生（`machine_repair_dto_for_instruction`），
        // 不从 message/class_hint/severity 猜资格；无配对恒 None。
        machine_repair,
    }
}

/// SG7（plan v1.1 §0.6.3）：服务端 L1 指令 → wire `MachineRepairDto` 的
/// 纯投影兼显式配对闸——仅当指令与当前候选的 source hash/context
/// digest 完全一致时返回 Some（identity 字段逐字取自指令；`instruction_ref`
/// 按 `instruction_digest` 内容寻址，与冻结件 `structural-prompts/` 同
/// 约定，纯派生零 I/O）。漂移/缺指令一律 None：本函数不从 finding 文本
/// 反查指令，配对由持有两侧身份的调用方显式完成。
#[allow(dead_code)] // L1 接线（SG6a 链/SG10a）前仅测试调用，同文件 SG2 先例
pub(crate) fn machine_repair_dto_for_instruction(
    instruction: &StructuralRepairInstruction,
    current_source_hash: &str,
    current_context_digest: &str,
) -> Option<MachineRepairDto> {
    if instruction.source_hash != current_source_hash
        || instruction.context_digest != current_context_digest
    {
        return None;
    }
    Some(MachineRepairDto {
        repair_id: instruction.repair_id.clone(),
        failure_id: instruction.failure_id.clone(),
        source_hash: instruction.source_hash.clone(),
        context_digest: instruction.context_digest.clone(),
        status: MachineRepairStatus::Offered,
        instruction_ref: format!(
            "structural-repair-instruction/{}",
            instruction.instruction_digest
        ),
    })
}

/// 定位三元组拼进 evidence（确定性键值对，机械可识别）。
fn evidence_text(
    logical_work_item_id: Option<&str>,
    contract_ref: Option<&str>,
    capability_ref: Option<&str>,
) -> String {
    let mut parts = Vec::new();
    if let Some(id) = logical_work_item_id {
        parts.push(format!("work_item={id}"));
    }
    if let Some(contract_ref) = contract_ref {
        parts.push(format!("contract={contract_ref}"));
    }
    if let Some(capability) = capability_ref {
        parts.push(format!("capability={capability}"));
    }
    parts.join(" ")
}

/// 跨轮指纹定位器：机械 finding 以确定性投影构造（REQ-TOP-04：消费方 WI +
/// contract + capability），消费方 WI 使受限提取命中稳定域，同一缺口跨轮
/// 同指纹；缺能力缺口必须精确到 capability 级，避免同一契约的两个不同缺口
/// 共享指纹。
fn contract_field_text(finding: &ContractValidationFinding) -> Option<String> {
    let consumer = finding.logical_work_item_id.as_deref();
    let projection = match (
        finding.code.as_str(),
        &finding.contract_ref,
        &finding.capability_ref,
    ) {
        ("required_capability_missing", Some(contract_id), Some(capability)) => {
            Some(format!("{contract_id}::{capability}"))
        }
        (_, Some(contract_id), _) => Some(contract_id.clone()),
        _ => finding.code.clone().into(),
    };
    projection.map(|projection| match consumer {
        Some(consumer) => format!("{consumer}::{projection}"),
        None => projection,
    })
}

/// F5-A 机械 verdict 与 SG2 机器指令共用的 capability 逐字补丁行模板
/// （单一事实源：两通道不给第二套语义）。
pub(crate) fn capability_patch_action_text(
    provider: &str,
    contract_ref: &str,
    capability: &str,
) -> String {
    format!(
        "在 {provider} 的 Outputs 契约 {contract_ref} 的 capabilities 列表追加一行（逐字复制）：- {capability}"
    )
}

/// required_action 按 code 模板化；provider 从 graph 反查补全（对
/// required_capability_missing/required_contract_missing，finding 的
/// logical_work_item_id 是消费方，provider 只存在于图边）。
fn required_action_text(finding: &ContractValidationFinding, provider: Option<&str>) -> String {
    let consumer = finding.logical_work_item_id.as_deref();
    let contract_ref = finding.contract_ref.as_deref();
    let capability = finding.capability_ref.as_deref();
    match finding.code.as_str() {
        "required_capability_missing" => capability_patch_action_text(
            provider.unwrap_or("（provider）"),
            contract_ref.unwrap_or("（contract）"),
            capability.unwrap_or("（capability）"),
        ),
        "required_contract_missing" => format!(
            "provider {} 需在输出契约中提供 {}（消费方 {} 依赖该契约）",
            provider.unwrap_or("（provider）"),
            contract_ref.unwrap_or("（contract）"),
            consumer.unwrap_or("（consumer）"),
        ),
        "unknown_provider_logical_work_item" => format!(
            "消费方引用的 provider {} 不在本 plan 内：修正 provider_logical_work_item_id 或补齐对应 work item（契约 {}）",
            consumer.unwrap_or("（provider）"),
            contract_ref.unwrap_or("（contract）"),
        ),
        "duplicate_dependency_contract_edge" => format!(
            "同一依赖边只声明一次契约 {}：合并重复声明",
            contract_ref.unwrap_or("（contract）"),
        ),
        "dependency_cycle" => {
            // SG2（REQ-SG-02A 禁止假删边）：不再只指示「调整 depends_on」，
            // 携带真实闭包与保护项——Inputs/Plan Intent/Handoff 同步、保
            // 真实供需/AC 与已确认仓序。
            format!(
                "拆除依赖环（{}）：保持全部真实 Inputs/AC 与已确认仓序，不得仅调整 depends_on 打破循环——Inputs、Plan Intent、Handoff/引用必要闭包必须同步；删/替依赖必须给出已满足同一供需的证据",
                finding.message
            )
        }
        "unconsumed_required_handoff" => format!(
            "终端 work item 的 handoff 契约 {} 无消费方：从 provided_contract_refs 移除该引用，或补齐下游消费方",
            contract_ref.unwrap_or("（contract）"),
        ),
        "duplicate_logical_work_item_identity" => {
            "logical_work_item_identity 重复：合并或重命名重复的 work item".to_string()
        }
        _ => format!("按机械校验结论修复契约声明：{}", finding.message),
    }
}

/// 反查 (consumer, contract_id) 所在依赖边的 provider。
fn provider_for_consumer_contract<'a>(
    graph: &'a DependencyContractGraph,
    consumer: &str,
    contract_id: &str,
) -> Option<&'a str> {
    graph
        .edges
        .iter()
        .find(|edge| {
            edge.to == consumer
                && edge
                    .required_contracts
                    .iter()
                    .any(|required| required.contract_id == contract_id)
        })
        .map(|edge| edge.from.as_str())
}

// ===================== SG2 精确机器指令纯 producer（REQ-SG-02/02A/04A、REQ-PLN-10）=====================
//
// 本节是 plan §0.5 / 设计 §4.2 的纯 producer：从 F1.x/SG1 的 typed 失败与
// 单源有效图报告确定性生成 `StructuralRepairInstruction`（事实 + 候选解 +
// 约束），零 store/provider I/O、零模型调用。指令内容直改语义已由
// `contract_autorepair` 迁为机器指令（REQ-SG-02A：内容修复经原授权 author，
// 本模块只产指令与中文 required_action，不发布自动执行授权——执行接线由
// SG5/SG6a 落地，wire 投影由 SG7 落地，本包不动 ReviewFinding 字段）。

use crate::product::planning_failure::{
    LastValidStructureRefs, PlanningCycleWitnessHop, PlanningEdgeSource, PlanningFailure,
    PlanningFailureKind, PlanningFailureSet,
};
use crate::product::workspace_engine::draft_batch::compile_support::{
    EffectivePlanningEdge, PlanningStructureReport, PlanningStructureSpan,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// SG2 受保护业务事实（plan §0.5）：值全由外层从已确认业务基线/原授权
/// 读取，指令只引用不创造。本类型不声明机器可证明的开放语义等价——
/// 开放语义等价仍由原 reviewer 判断（REQ-PLN-10）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ProtectedBusinessFacts {
    pub business_request_digest: String,
    pub confirmed_design_digest: Option<String>,
    /// 不得删除/改写的真实业务字段域（如 Inputs、Acceptance Criteria）。
    pub protected_field_refs: Vec<String>,
    /// 不得删除的真实供需契约引用。
    pub protected_contract_refs: Vec<String>,
    /// 已授权逻辑成员（归属重组候选的闭包）。
    pub authorized_member_ids: Vec<String>,
}

/// 机器修复指令的解形态（设计 §4.2 固定候选族）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum StructuralRepairSolution {
    /// 精确操作（候选族 1–3 的可逐字执行形态）。`uniqueness` 仅在局部固定
    /// grammar 映射或完整核验历史值时携带（REQ-PLN-10：拓扑/删边空间不能
    /// 因只生成一个建议就称唯一）；其余精确操作不声明唯一。
    Precise {
        operations: Vec<StructuralRepairOperation>,
        uniqueness: Option<PreciseUniqueness>,
    },
    /// 候选（候选族 3–5）：安全候选结构上不携带唯一标签，author 在保护
    /// 约束下选择或在约束下求解（无精确内容解的靶向重组）。
    Candidates {
        candidates: Vec<StructuralRepairCandidate>,
    },
}

/// 唯一性证据的两种合法来源（REQ-PLN-10：仅局部固定 grammar 映射或
/// 完整核验历史值可声明唯一）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum PreciseUniqueness {
    /// 局部固定 grammar 映射（如非法二级 heading→唯一合法模板）。
    FixedGrammarMapping { mapping: String },
    /// 完整核验历史值（resolved fact digest + 整组 last valid refs）。
    HistoryVerified { resolved_fact_digest: String },
}

/// 精确操作：旧 autorepair 内容直改语义迁为指令载荷（REQ-SG-02A）。
/// `ReplaceLine`/`InsertLines`/`DeleteLines` 是可机械逐字应用的行编辑
/// （contract_autorepair 本地收敛消费）；`RewriteWithGrammarTemplate`/
/// `RestoreResolvedFact` 是 author 执行域的表达/历史恢复指令。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum StructuralRepairOperation {
    /// 1-based 行替换：before/after 逐字（应用前后可核对）。
    ReplaceLine {
        line: usize,
        before: String,
        after: String,
    },
    /// 在 `after_line` 行后逐字插入 `lines`。
    InsertLines {
        after_line: usize,
        lines: Vec<String>,
    },
    /// 删除行：`reason` 为确定性删除理由。typed heading 等承载业务内容的
    /// 行永不进入该分支（未知 key 大码 heading 不删行）。
    DeleteLines { lines: Vec<usize>, reason: String },
    /// 语法模板重表达（非法 heading）：合法模板 + 保留值，author 重写。
    RewriteWithGrammarTemplate {
        line: usize,
        raw_line: String,
        template: String,
    },
    /// 历史恢复（候选族 2）：从整组 last valid refs 恢复已解决结构事实；
    /// 需新内容融合时仍由 author 实施后全量复验。
    RestoreResolvedFact {
        resolved_fact_digest: String,
        difference: String,
    },
}

/// 安全候选（不携带唯一标签）：title/constraints/evidence 全部来自已读
/// 事实；`verified_predicates` 是机器可验证谓词，开放语义仍归 reviewer。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StructuralRepairCandidate {
    pub title: String,
    pub constraints: Vec<String>,
    pub evidence: Vec<String>,
    pub verified_predicates: Vec<String>,
}

/// SG2 精确机器修复指令（REQ-SG-02 全字段绑定）：repair/failure identity、
/// 规则版本、source hash/context digest、定位、实际冲突边及全部来源、
/// 允许修改闭包、禁止操作（保护项具体化）、protected business facts、
/// 精确操作或候选、中文 required_action、expected predicates、可用历史
/// refs 与 instruction digest。纯数据，不构成入站许可。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StructuralRepairInstruction {
    pub repair_id: String,
    pub failure_id: String,
    pub rule_version: String,
    pub source_hash: String,
    pub context_digest: String,
    pub affected_work_item_ids: Vec<String>,
    pub affected_spans: Vec<PlanningStructureSpan>,
    /// 实际冲突边及全部来源（witness 逐跳 join 有效图；非拓扑失败为空）。
    pub conflict_edges: Vec<EffectivePlanningEdge>,
    /// 允许修改闭包（允许触碰的字段/归属域）。
    pub allowed_change_closure: Vec<String>,
    /// 禁止操作（REQ-SG-02A 保护项的具体化：删必需 Inputs/AC、仅改
    /// Dependencies、改仓序等假修复一律显式拒绝）。
    pub forbidden_operations: Vec<String>,
    pub protected_facts: ProtectedBusinessFacts,
    pub solution: StructuralRepairSolution,
    /// 中文 required_action（确定性生成，同输入同文本）。
    pub required_action: String,
    /// 执行后必须成立的结构谓词（全量复验口径）。
    pub expected_predicates: Vec<String>,
    /// 可用历史 refs（仅历史恢复类指令携带）。
    pub previous_valid_refs: Option<LastValidStructureRefs>,
    /// 指令确定性摘要（绑定全部身份/约束/操作字段，防重复派发）。
    pub instruction_digest: String,
}

/// 从 typed 失败集合 + SG1 单源有效图报告 + 受保护业务事实确定性生成
/// 机器修复指令（plan §0.5 SG2 签名）。纯 producer：零 I/O、零模型调用、
/// 零 source mutation。`Operational`/`Unknown` 失败不产指令（fail-closed，
/// 沿原安全等待/协议恢复通道，不驱 author）。
#[allow(dead_code)]
pub(crate) fn build_structural_repair_instructions(
    failures: &PlanningFailureSet,
    report: &PlanningStructureReport,
    protected_facts: &ProtectedBusinessFacts,
) -> Vec<StructuralRepairInstruction> {
    failures
        .failures
        .iter()
        .enumerate()
        .filter_map(|(index, failure)| {
            instruction_for_failure(failure, index, report, protected_facts)
        })
        .collect()
}

/// 单条 typed 失败 → 机器指令（fail-closed：Operational/Unknown 及事实
/// 不完整的拓扑失败不产指令，交原安全等待/协议恢复通道）。
#[allow(dead_code)]
fn instruction_for_failure(
    failure: &PlanningFailure,
    index: usize,
    report: &PlanningStructureReport,
    protected_facts: &ProtectedBusinessFacts,
) -> Option<StructuralRepairInstruction> {
    let base = InstructionBase {
        failure_id: format!("{}#{}", failure.diagnostic_ref, index),
        repair_id: format!(
            "structural-repair/{}/{}#{index}",
            failure.source_hash.chars().take(16).collect::<String>(),
            failure.kind.stable_code()
        ),
        rule_version: report.rule_version.clone(),
        source_hash: report.source_hash.clone(),
        context_digest: report.context_digest.clone(),
        affected_work_item_ids: failure.evidence.affected_ids.clone(),
    };
    let instruction = match &failure.kind {
        PlanningFailureKind::DependencyCycle { witness } => {
            dependency_cycle_instruction(base, report, protected_facts, witness)?
        }
        PlanningFailureKind::InvalidWorkItemHeading {
            line,
            field,
            raw_line,
            example,
        } => heading_instruction(base, protected_facts, *line, field, raw_line, example),
        PlanningFailureKind::UnknownProvider {
            provider,
            consumer,
            contract_id,
        } => unknown_provider_instruction(base, protected_facts, provider, consumer, contract_id),
        PlanningFailureKind::RequiredContractMissing {
            provider,
            consumer,
            contract_id,
        } => required_contract_missing_instruction(
            base,
            protected_facts,
            provider,
            consumer,
            contract_id,
        ),
        PlanningFailureKind::RequiredCapabilityMissing {
            provider,
            consumer,
            contract_id,
            capability,
        } => required_capability_missing_instruction(
            base,
            protected_facts,
            provider,
            consumer,
            contract_id,
            capability,
        ),
        PlanningFailureKind::TargetMismatch {
            logical_work_item_id,
            declared_target,
            resolved_target,
        } => target_mismatch_instruction(
            base,
            protected_facts,
            logical_work_item_id,
            declared_target.as_deref(),
            resolved_target,
        ),
        PlanningFailureKind::StructuralRegression {
            resolved_fact_digest,
            last_valid_refs,
            difference,
        } => regression_instruction(
            base,
            protected_facts,
            resolved_fact_digest,
            last_valid_refs,
            difference,
        ),
        // 工程/运行内部错与未 typed 诊断：不驱 author（classify 同口径）。
        PlanningFailureKind::Operational { .. } | PlanningFailureKind::Unknown { .. } => {
            return None;
        }
    };
    Some(seal_structural_repair_instruction(instruction))
}

/// 指令公共身份段（由失败/报告单源派生）。
#[allow(dead_code)]
struct InstructionBase {
    failure_id: String,
    repair_id: String,
    rule_version: String,
    source_hash: String,
    context_digest: String,
    affected_work_item_ids: Vec<String>,
}

/// REQ-SG-02A 保护项的具体化（所有指令共享的禁止操作）。
#[allow(dead_code)]
fn base_forbidden_operations() -> Vec<String> {
    vec![
        "删除必需 Inputs 或真实供需引用（假删边不是修复）".to_string(),
        "删除或放宽任何验收 AC".to_string(),
        "仅修改 Dependencies 一处而 Inputs/Plan Intent/Handoff 引用不变".to_string(),
        "改变已确认仓序或扩大成员写权限".to_string(),
    ]
}

/// 依赖环指令（R1 形态，设计 §4.2/§4.3）：required_action 携两方向边真实
/// 来源（canonical_input 契约+定位 / change_order 仓序归属）、仓序位置与
/// Inputs 真实闭包；解域开放——归属/拆分重组与删/替依赖（需证据）两族
/// 候选，不携带唯一标签。witness 逐跳必须 join 到有效图真实边，任一跳
/// 失配即 fail-closed 不产指令（Kahn 残余列表不是 witness，不冒充邻接）。
#[allow(dead_code)]
fn dependency_cycle_instruction(
    base: InstructionBase,
    report: &PlanningStructureReport,
    protected_facts: &ProtectedBusinessFacts,
    witness: &[PlanningCycleWitnessHop],
) -> Option<StructuralRepairInstruction> {
    if witness.is_empty() {
        return None;
    }
    let mut conflict_edges = Vec::with_capacity(witness.len());
    let mut hop_lines = Vec::with_capacity(witness.len());
    let mut canonical_contracts = Vec::new();
    let mut spans: Vec<PlanningStructureSpan> = Vec::new();
    for hop in witness {
        let edge = report
            .effective_edges
            .iter()
            .find(|edge| edge.from == hop.from && edge.to == hop.to)
            .cloned()?;
        conflict_edges.push(edge.clone());
        let mut source_parts = Vec::new();
        if edge.sources.contains(&PlanningEdgeSource::CanonicalInput) {
            canonical_contracts.extend(edge.contract_refs.iter().cloned());
            spans.extend(edge.spans.iter().cloned());
            let span_text = edge
                .spans
                .iter()
                .map(|span| format!("{} 行{}", span.field, span.line))
                .collect::<Vec<_>>()
                .join("；");
            source_parts.push(format!(
                "canonical_input：真实输入契约 {}，定位 {span_text}",
                edge.contract_refs.join("、")
            ));
        }
        if edge.sources.contains(&PlanningEdgeSource::ChangeOrder) {
            let earlier = edge.order_indices.first().copied().unwrap_or_default();
            let later = edge.order_indices.get(1).copied().unwrap_or(earlier);
            let from_target = report
                .physical_targets
                .get(&edge.from)
                .cloned()
                .unwrap_or_else(|| "（未解析归属）".to_string());
            let to_target = report
                .physical_targets
                .get(&edge.to)
                .cloned()
                .unwrap_or_else(|| "（未解析归属）".to_string());
            source_parts.push(format!(
                "change_order：仓序自动边，{} 归属 {}（仓序位 {earlier}）先于 {} 归属 {}（仓序位 {later}）",
                edge.from, from_target, edge.to, to_target
            ));
        }
        hop_lines.push(format!(
            "- {}→{}（来源 {}）",
            edge.from,
            edge.to,
            source_parts.join("；")
        ));
    }
    canonical_contracts.sort();
    canonical_contracts.dedup();
    let evidence = hop_lines.clone();
    let mut expected_predicates = vec![
        "effective_graph_acyclic".to_string(),
        "acceptance_criteria_preserved".to_string(),
        "change_order_unchanged".to_string(),
        "declared_dependencies_consistent_with_effective_inputs".to_string(),
        "logical_targets_within_authorized_members".to_string(),
    ];
    expected_predicates.extend(
        canonical_contracts
            .iter()
            .map(|contract| format!("canonical_inputs_preserved:{contract}")),
    );
    let mut forbidden = base_forbidden_operations();
    forbidden.push("以无环为由删除真实供需（无环不是业务正确证明）".to_string());
    Some(StructuralRepairInstruction {
        failure_id: base.failure_id,
        repair_id: base.repair_id,
        rule_version: base.rule_version,
        source_hash: base.source_hash,
        context_digest: base.context_digest,
        affected_work_item_ids: base.affected_work_item_ids,
        affected_spans: spans,
        conflict_edges,
        allowed_change_closure: vec![
            "环上 work item 的成员归属/Plan Intent 归属重组（限已授权成员）".to_string(),
            "环上 work item 的 Inputs/Outputs/Handoff 契约必要闭包同步".to_string(),
            "Dependencies 声明（须与真实 Inputs 同步一致）".to_string(),
            "拆分/合并验证任务的重组（真实 Inputs 与 AC 完整保留）".to_string(),
        ],
        forbidden_operations: forbidden,
        protected_facts: protected_facts.clone(),
        solution: StructuralRepairSolution::Candidates {
            candidates: vec![
                StructuralRepairCandidate {
                    title: "归属/任务重组：将环上消费方重组到仓序兼容的已授权验证成员，或拆分集成验证任务".to_string(),
                    constraints: vec![
                        "真实 Inputs 引用逐字保留".to_string(),
                        "全部 AC 保留".to_string(),
                        "不改变已确认 Design 仓序".to_string(),
                        "归属仅限已授权成员".to_string(),
                    ],
                    evidence: evidence.clone(),
                    verified_predicates: vec![
                        "canonical_inputs_preserved".to_string(),
                        "change_order_unchanged".to_string(),
                    ],
                },
                StructuralRepairCandidate {
                    title: "删/替依赖：仅在已证明同一供需被满足时删除或替换数据流边".to_string(),
                    constraints: vec![
                        "删除/替换前必须给出已满足同一供需的证据（同契约/同能力由其他路径提供）".to_string(),
                        "不得以无环为由删除必需 Inputs".to_string(),
                    ],
                    evidence,
                    verified_predicates: vec![
                        "supply_demand_evidenced_before_edge_removal".to_string(),
                    ],
                },
            ],
        },
        required_action: format!(
            "依赖环（真实有向 witness，逐跳来源与定位）：\n{}\n保持环上全部真实 Inputs（含 AC）与已确认仓序，不改变 Design 仓序；按已授权成员与验证位置重组归属或拆分验证任务；若选择删/替依赖，必须给出已满足同一供需的证据；不得仅修改 Dependencies 一处——Inputs、Plan Intent、Handoff/引用必要闭包必须同步。",
            hop_lines.join("\n")
        ),
        expected_predicates,
        previous_valid_refs: None,
        instruction_digest: String::new(),
    })
}

/// 非法二级 heading 指令（候选族 1 精确表达纠正）：原行/合法模板/保留值，
/// 固定 grammar 映射携唯一证据；保留原业务段与行，不按共享 unknown key
/// 码删行（REQ-SG-02 第 414 行场景）。
#[allow(dead_code)]
fn heading_instruction(
    base: InstructionBase,
    protected_facts: &ProtectedBusinessFacts,
    line: usize,
    field: &str,
    raw_line: &str,
    example: &str,
) -> StructuralRepairInstruction {
    let mut forbidden = base_forbidden_operations();
    forbidden.push("删除 heading 原行或其业务内容（不按共享 unknown key 码删行）".to_string());
    StructuralRepairInstruction {
        failure_id: base.failure_id,
        repair_id: base.repair_id,
        rule_version: base.rule_version,
        source_hash: base.source_hash,
        context_digest: base.context_digest,
        affected_work_item_ids: base.affected_work_item_ids,
        affected_spans: vec![PlanningStructureSpan {
            field: field.to_string(),
            line,
        }],
        conflict_edges: Vec::new(),
        allowed_change_closure: vec![
            format!("第 {line} 行按合法 WI heading grammar 重写（业务值保留）"),
            "受影响段落的必要结构重排（不删业务内容）".to_string(),
        ],
        forbidden_operations: forbidden,
        protected_facts: protected_facts.clone(),
        solution: StructuralRepairSolution::Precise {
            operations: vec![StructuralRepairOperation::RewriteWithGrammarTemplate {
                line,
                raw_line: raw_line.to_string(),
                template: example.to_string(),
            }],
            uniqueness: Some(PreciseUniqueness::FixedGrammarMapping {
                mapping: format!("{field}→{example}"),
            }),
        },
        required_action: format!(
            "第 {line} 行出现非法二级 heading（字段 {field}，原行「{raw_line}」）：保留原业务段与行内容，按合法模板「{example}」以合法 WI 结构重写该行；不得删除该行或其业务内容，不按共享 unknown key 码删行。"
        ),
        expected_predicates: vec![
            "work_item_headings_legal".to_string(),
            "raw_line_business_content_preserved".to_string(),
            "grammar_sections_complete".to_string(),
        ],
        previous_valid_refs: None,
        instruction_digest: String::new(),
    }
}

/// 未知 provider 指令（候选族 3/4）：修正引用或补齐 provider work item，
/// 均保持真实供需；不删 Inputs 引用。
#[allow(dead_code)]
fn unknown_provider_instruction(
    base: InstructionBase,
    protected_facts: &ProtectedBusinessFacts,
    provider: &str,
    consumer: &str,
    contract_id: &str,
) -> StructuralRepairInstruction {
    supply_demand_instruction(
        base,
        protected_facts,
        format!(
            "消费方 {consumer} 引用的 provider {provider} 不在本 plan 内（契约 {contract_id}）：修正 provider_logical_work_item_id 为批内真实 work item，或补齐提供该契约的 work item；不得删除该真实 Inputs 引用来消除缺口。"
        ),
        vec![
            StructuralRepairCandidate {
                title: format!(
                    "修正引用：provider_logical_work_item_id 指向批内真实 provider（契约 {contract_id}）"
                ),
                constraints: vec!["真实供需不变（消费方 Inputs 引用逐字保留）".to_string()],
                evidence: vec![format!("{consumer} 通过契约 {contract_id} 引用 {provider}")],
                verified_predicates: vec!["all_canonical_providers_resolvable".to_string()],
            },
            StructuralRepairCandidate {
                title: format!("补齐 provider：新增或恢复提供 {contract_id} 的 work item"),
                constraints: vec![
                    "补齐方的 Outputs 契约逐字覆盖消费方 required_capabilities".to_string(),
                ],
                evidence: vec![format!("契约 {contract_id} 当前无 provider")],
                verified_predicates: vec![format!("contract_supply_present:{contract_id}")],
            },
        ],
        vec![
            "all_canonical_providers_resolvable".to_string(),
            format!("contract_supply_present:{contract_id}"),
        ],
    )
}

/// 契约缺口指令：provider 补输出契约或重组供需（需证据）。
#[allow(dead_code)]
fn required_contract_missing_instruction(
    base: InstructionBase,
    protected_facts: &ProtectedBusinessFacts,
    provider: &str,
    consumer: &str,
    contract_id: &str,
) -> StructuralRepairInstruction {
    supply_demand_instruction(
        base,
        protected_facts,
        format!(
            "provider {provider} 需在输出契约中提供 {contract_id}（消费方 {consumer} 依赖该契约）：补齐输出契约，或重组供需（改由已满足同一供需的既有成员提供，需证据）；不得删除消费方真实需求。"
        ),
        vec![
            StructuralRepairCandidate {
                title: format!(
                    "补齐输出契约：{provider} 在 Outputs 提供 {contract_id}（含消费方要求的 capabilities）"
                ),
                constraints: vec!["required_capabilities 逐字覆盖消费方要求".to_string()],
                evidence: vec![format!("{consumer} 依赖 {provider} 的契约 {contract_id}")],
                verified_predicates: vec![format!("contract_supplied:{contract_id}")],
            },
            StructuralRepairCandidate {
                title: "重组供需：消费方改由已满足同一供需的既有成员提供（需证据）".to_string(),
                constraints: vec!["改接前必须证明同一供需已由目标成员满足".to_string()],
                evidence: vec![format!("契约 {contract_id} 的消费方 {consumer}")],
                verified_predicates: vec![format!("supply_demand_satisfied:{contract_id}")],
            },
        ],
        vec![
            format!("contract_supplied:{contract_id}"),
            "acceptance_criteria_preserved".to_string(),
        ],
    )
}

/// capability 缺口指令：逐字追加行或删/替需求（需证据）——开放解域，
/// 不携带唯一标签。
#[allow(dead_code)]
fn required_capability_missing_instruction(
    base: InstructionBase,
    protected_facts: &ProtectedBusinessFacts,
    provider: &str,
    consumer: &str,
    contract_id: &str,
    capability: &str,
) -> StructuralRepairInstruction {
    supply_demand_instruction(
        base,
        protected_facts,
        format!(
            "在 {provider} 的 Outputs 契约 {contract_id} 的 capabilities 列表追加一行（逐字复制）：- {capability}；或重组供需（删/替需求必须给出已满足同一供需的证据）；不得删除消费方 {consumer} 的真实需求来消除缺口。"
        ),
        vec![
            StructuralRepairCandidate {
                title: format!(
                    "补齐能力：在 {provider} 的 Outputs 契约 {contract_id} 的 capabilities 列表追加一行（逐字复制）：- {capability}"
                ),
                constraints: vec!["追加逐字行，不删既有 capabilities".to_string()],
                evidence: vec![format!(
                    "{consumer} 经契约 {contract_id} 要求能力 {capability}（provider {provider} 未逐字提供）"
                )],
                verified_predicates: vec![format!(
                    "capability_satisfied:{contract_id}::{capability}"
                )],
            },
            StructuralRepairCandidate {
                title: "删/替需求：仅当已满足同一供需的证据成立".to_string(),
                constraints: vec![
                    "删除/替换该能力需求前必须证明同一供需已由其他路径满足".to_string(),
                ],
                evidence: vec![format!("{consumer} 对 {capability} 的真实需求")],
                verified_predicates: vec![format!(
                    "supply_demand_evidenced:{contract_id}::{capability}"
                )],
            },
        ],
        vec![
            format!("capability_satisfied:{contract_id}::{capability}"),
            "acceptance_criteria_preserved".to_string(),
        ],
    )
}

/// 归属不一致指令（候选族 4）：按授权成员重组归属或走原业务门调整授权。
#[allow(dead_code)]
fn target_mismatch_instruction(
    base: InstructionBase,
    protected_facts: &ProtectedBusinessFacts,
    logical_work_item_id: &str,
    declared_target: Option<&str>,
    resolved_target: &str,
) -> StructuralRepairInstruction {
    let declared = declared_target.unwrap_or("（未声明）");
    supply_demand_instruction(
        base,
        protected_facts,
        format!(
            "work item {logical_work_item_id} 的归属与授权成员解析不一致（声明 {declared} / 解析 {resolved_target}）：按已授权成员与 Plan Intent 归属重组该 work item 的归属声明；真实 Inputs 与 AC 保留；若业务确需新归属，沿原业务门调整成员授权（不是结构修复）。"
        ),
        vec![
            StructuralRepairCandidate {
                title: format!(
                    "归属重组：修正 {logical_work_item_id} 的归属声明至已授权成员（与 Plan Intent 解析一致）"
                ),
                constraints: vec![
                    "归属仅限已授权成员".to_string(),
                    "真实 Inputs/AC 保留，不改变已确认仓序".to_string(),
                ],
                evidence: vec![format!("声明 {declared} / 解析 {resolved_target}")],
                verified_predicates: vec!["logical_target_matches_authorized_member".to_string()],
            },
            StructuralRepairCandidate {
                title: "成员授权调整：业务确需该归属时走原业务门（不属结构修复）".to_string(),
                constraints: vec!["不扩大写权限，不混聚合 native target 与业务 target".to_string()],
                evidence: vec![format!("解析归属 {resolved_target} 不在授权成员内")],
                verified_predicates: vec!["member_authorization_via_business_gate".to_string()],
            },
        ],
        vec![
            "logical_target_matches_authorized_member".to_string(),
            "acceptance_criteria_preserved".to_string(),
        ],
    )
}

/// 已解决结构事实回归指令（候选族 2 历史恢复）：整组旧合法 refs +
/// resolved fact digest，历史核验携唯一证据；需新内容融合时仍由 author
/// 实施后全量复验，不依赖模型记忆（REQ-SG-04A）。
#[allow(dead_code)]
fn regression_instruction(
    base: InstructionBase,
    protected_facts: &ProtectedBusinessFacts,
    resolved_fact_digest: &str,
    last_valid_refs: &LastValidStructureRefs,
    difference: &str,
) -> StructuralRepairInstruction {
    let mut forbidden = base_forbidden_operations();
    forbidden.push("跳过旧合法 refs 仅凭模型记忆修复（回归修复必须携旧事实）".to_string());
    StructuralRepairInstruction {
        failure_id: base.failure_id,
        repair_id: base.repair_id,
        rule_version: base.rule_version,
        source_hash: base.source_hash,
        context_digest: base.context_digest,
        affected_work_item_ids: base.affected_work_item_ids,
        affected_spans: Vec::new(),
        conflict_edges: Vec::new(),
        allowed_change_closure: vec![
            "恢复 resolved fact 覆盖的结构字段为旧合法值".to_string(),
            "新内容融合（业务反馈保留，author 实施后全量复验）".to_string(),
        ],
        forbidden_operations: forbidden,
        protected_facts: protected_facts.clone(),
        solution: StructuralRepairSolution::Precise {
            operations: vec![StructuralRepairOperation::RestoreResolvedFact {
                resolved_fact_digest: resolved_fact_digest.to_string(),
                difference: difference.to_string(),
            }],
            uniqueness: Some(PreciseUniqueness::HistoryVerified {
                resolved_fact_digest: resolved_fact_digest.to_string(),
            }),
        },
        required_action: format!(
            "已解决结构事实回归（差异：{difference}）：优先从旧合法 refs（source {}/{}/{}/{}）恢复不变的结构字段；需新内容融合时仍由 author 实施并全量复验，不依赖模型记忆。",
            last_valid_refs.source_ref,
            last_valid_refs.ir_ref,
            last_valid_refs.report_ref,
            resolved_fact_digest
        ),
        expected_predicates: vec![
            "resolved_structure_facts_hold_again".to_string(),
            format!("regression_cleared:{resolved_fact_digest}"),
            "effective_graph_acyclic".to_string(),
        ],
        previous_valid_refs: Some(last_valid_refs.clone()),
        instruction_digest: String::new(),
    }
}

/// 供需/归属族指令的公共组装（候选形态，无唯一标签）。
// SG2：纯指令 producer 待 SG3a.1/SG5/SG6a 接线（尚未完整接线不发布自动
// 能力），按 SG1 纯核心同一惯例豁免未接线 dead_code。
#[allow(dead_code)]
fn supply_demand_instruction(
    base: InstructionBase,
    protected_facts: &ProtectedBusinessFacts,
    required_action: String,
    candidates: Vec<StructuralRepairCandidate>,
    expected_predicates: Vec<String>,
) -> StructuralRepairInstruction {
    StructuralRepairInstruction {
        failure_id: base.failure_id,
        repair_id: base.repair_id,
        rule_version: base.rule_version,
        source_hash: base.source_hash,
        context_digest: base.context_digest,
        affected_work_item_ids: base.affected_work_item_ids,
        affected_spans: Vec::new(),
        conflict_edges: Vec::new(),
        allowed_change_closure: vec![
            "受影响 work item 的 Inputs/Outputs/Handoff 契约声明（真实供需保留）".to_string(),
            "受影响 work item 的成员归属/Plan Intent 归属（限已授权成员）".to_string(),
        ],
        forbidden_operations: base_forbidden_operations(),
        protected_facts: protected_facts.clone(),
        solution: StructuralRepairSolution::Candidates { candidates },
        required_action,
        expected_predicates,
        previous_valid_refs: None,
        instruction_digest: String::new(),
    }
}

/// 指令摘要封口：digest 绑定全部身份/约束/操作字段（封口前为空串参与
/// 序列化，防止自引用），同输入恒等、任何字段漂移即变化。SG2 内
/// contract_autorepair 的本地 producer 复用同一封口（单一 digest 权威）。
pub(crate) fn seal_structural_repair_instruction(
    mut instruction: StructuralRepairInstruction,
) -> StructuralRepairInstruction {
    let material = serde_json::to_string(&instruction).unwrap_or_else(|error| {
        unreachable!("structural repair instruction serialization is infallible: {error}")
    });
    instruction.instruction_digest = hex::encode(Sha256::digest(material.as_bytes()));
    instruction
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gap_source_and_ir() -> (String, PlanCandidateIr) {
        // rep4 基线：终端 WI-003 的 handoff 提供行按 campaign 口径清空
        // （unconsumed_required_handoff 是 Error 级，混入会干扰缺口断言），
        // 再给 WI-002 的 require_all 塞一个 WI-001 未提供的 capability。
        let source = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/product/work_item_plan_compiler/fixtures/work-item-plan-rep4.md"
        ))
        .replace(
            "- provided_contract_refs: contract.levels-integration",
            "- provided_contract_refs: []",
        )
        .replace(
            "- required_capabilities: api.levels.read\n",
            "- required_capabilities: api.levels.read, api.levels.write\n",
        );
        let ir = crate::product::work_item_plan_compiler::compile_work_item_plan(
            &source,
            &crate::product::work_item_plan_compiler::WorkItemPlanSourceContext {
                target_repository_id: "repo_fixture".to_string(),
                member_targets: None,
            },
        )
        .expect("compile gap fixture");
        (source, ir)
    }

    fn compiled_gap_ir() -> PlanCandidateIr {
        gap_source_and_ir().1
    }

    fn compiled_clean_ir() -> PlanCandidateIr {
        let source = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/product/work_item_plan_compiler/fixtures/work-item-plan-rep4.md"
        ))
        .replace(
            "- provided_contract_refs: contract.levels-integration",
            "- provided_contract_refs: []",
        );
        crate::product::work_item_plan_compiler::compile_work_item_plan(
            &source,
            &crate::product::work_item_plan_compiler::WorkItemPlanSourceContext {
                target_repository_id: "repo_fixture".to_string(),
                member_targets: None,
            },
        )
        .expect("compile clean fixture")
    }

    /// E3-1 适配层映射：Error→MustFix+ContractGap+Repairable，code 进 message
    /// 前缀，定位三元组进 evidence，required_action 模板含 provider/contract/
    /// capability 逐字补齐指令。
    #[test]
    fn contract_prerevision_verdict_maps_error_findings_into_review_verdict() {
        let verdict = single_candidate_contract_prerevision_verdict(&compiled_gap_ir())
            .expect("capability gap must produce a mechanical revise verdict");
        assert_eq!(verdict.verdict, ReviewVerdictType::Revise);
        assert_eq!(verdict.review_gate, ReviewGate::RequiresRevision);
        assert!(verdict.work_item_plan_review.is_none());
        assert!(verdict.structured_output_diagnostic.is_none());

        let finding = verdict
            .findings
            .iter()
            .find(|finding| finding.message.contains("required_capability_missing"))
            .expect("capability gap finding must be present");
        assert_eq!(finding.severity, ReviewFindingSeverity::MustFix);
        assert_eq!(finding.category, Some(ReviewFindingCategory::ContractGap));
        assert_eq!(finding.class_hint, Some(FindingClassHint::Repairable));
        assert!(
            finding.message.starts_with("required_capability_missing: "),
            "mechanical code must prefix the message: {}",
            finding.message
        );
        assert!(
            finding.message.contains("api.levels.write"),
            "message must name the missing capability: {}",
            finding.message
        );
        for locator in ["WI-002", "contract.levels-api", "api.levels.write"] {
            assert!(
                finding.evidence.contains(locator),
                "evidence must carry locator {locator}: {}",
                finding.evidence
            );
        }
        // 契约前移升级：required_action 必须是逐字补丁行形态
        // （在 <provider WI> 的 Outputs 契约 <CT-id> 的 capabilities 列表追加
        // 一行（逐字复制）：- <capability 原串>），capability 原串与 finding
        // 的 capability_ref（evidence 的 capability= 定位串）逐字同源。
        assert!(
            finding.required_action.contains(
                "在 WI-001 的 Outputs 契约 contract.levels-api 的 capabilities 列表追加一行（逐字复制）：- api.levels.write"
            ),
            "required_action must be a verbatim patch line: {}",
            finding.required_action
        );
        let capability = finding
            .evidence
            .split_whitespace()
            .find_map(|part| part.strip_prefix("capability="))
            .expect("evidence must carry the capability locator");
        assert!(
            finding
                .required_action
                .ends_with(&format!("：- {capability}")),
            "patch line must quote the capability verbatim from capability_ref: {}",
            finding.required_action
        );
        assert!(
            finding
                .contract_field
                .as_deref()
                .is_some_and(|field| field.contains("contract.levels-api")
                    && field.contains("api.levels.write")),
            "contract_field must locate the capability gap: {:?}",
            finding.contract_field
        );
    }

    /// E3-1 干净候选零变化：无 Error findings 时不产生任何 verdict。
    #[test]
    fn contract_prerevision_verdict_stays_none_for_clean_contracts() {
        assert!(
            single_candidate_contract_prerevision_verdict(&compiled_clean_ir()).is_none(),
            "clean candidate must not produce a mechanical verdict"
        );
    }

    /// E3-1/勘察 C：Warning 级 findings 以 Suggestion+Advisory 拼进同一 verdict，
    /// 不单独触发 verdict、不参与闸门口径。
    #[test]
    fn contract_prerevision_warnings_ride_along_as_advisory_suggestions() {
        let graph = build_dependency_contract_graph(
            &compiled_clean_ir()
                .items
                .iter()
                .map(|item| item.contract.clone())
                .collect::<Vec<_>>(),
        )
        .expect("clean fixture builds a graph");
        let warning = ContractValidationFinding {
            code: "process_evidence_acceptance_criterion".to_string(),
            severity: ContractFindingSeverity::Warning,
            logical_work_item_id: Some("WI-001".to_string()),
            contract_ref: Some("AC-001".to_string()),
            capability_ref: None,
            message: "acceptance criterion AC-001 must describe an observable result".to_string(),
            kind: None,
        };
        let finding = contract_finding_to_review_finding(&warning, Some(&graph), None);
        assert_eq!(finding.severity, ReviewFindingSeverity::Suggestion);
        assert_eq!(finding.class_hint, Some(FindingClassHint::Advisory));
        assert_eq!(finding.category, None);
        assert!(
            finding
                .message
                .starts_with("process_evidence_acceptance_criterion: "),
            "warning code must prefix the message: {}",
            finding.message
        );

        // Warning-only 报告不产生 verdict（零变化门槛）。
        let warning_only = ContractValidationReport {
            findings: vec![warning],
        };
        assert!(verdict_from_report(&warning_only, Some(&graph)).is_none());
    }

    /// E3-4/勘察 D：同输入同文本（findings 排序稳定、模板确定性）——
    /// 机械 message 确定性是跨轮指纹稳定与闸门可复现的前提。
    #[test]
    fn contract_prerevision_verdict_is_deterministic_for_identical_input() {
        let first =
            single_candidate_contract_prerevision_verdict(&compiled_gap_ir()).expect("gap verdict");
        let second = single_candidate_contract_prerevision_verdict(&compiled_gap_ir())
            .expect("gap verdict rebuilt");
        assert_eq!(
            first, second,
            "identical IR must yield byte-identical mechanical verdicts"
        );
        assert_eq!(
            contract_prerevision_readable_output(&first),
            contract_prerevision_readable_output(&second)
        );
    }

    /// E3-1：build 阶段 Err（duplicate identity）同样转机械 verdict，不丢缺口。
    #[test]
    fn contract_prerevision_covers_duplicate_identity_build_failure() {
        let mut contracts = compiled_clean_ir()
            .items
            .iter()
            .map(|item| item.contract.clone())
            .collect::<Vec<_>>();
        let duplicated = contracts[0].clone();
        contracts.push(duplicated);
        let verdict = contract_prerevision_verdict_for(&contracts)
            .expect("duplicate identity must produce a mechanical revise verdict");
        assert!(verdict.findings.iter().any(|finding| {
            finding
                .message
                .contains("duplicate_logical_work_item_identity")
        }));
    }

    // ===================== SG2：精确机器指令与 autorepair 清切（REQ-SG-02/02A/04A、REQ-PLN-10）=====================
    //
    // plan Task SG2 Step 1：`sg_02_no_fake_edge_removal` 族。全部基于真实
    // SG1 纯核心投影 + 手工 typed 失败观察指令身份/refs/来源/谓词，不只
    // 匹配中文文案。

    use crate::product::logical_codebase::LogicalRepositoryId;
    use crate::product::planning_failure::{
        PlanningFailureEvidence, PlanningFailureOrigin, PlanningFailureSet, PlanningStage,
    };
    use crate::product::work_item_contract::{ContractCompatibilityPolicy, RequiredInputContract};
    use crate::product::workspace_engine::draft_batch::compile_support::{
        PlanningStructureCanonicalInput, PlanningStructureContext, PlanningStructureItem,
        PlanningStructureSpan, project_effective_planning_graph,
    };
    use std::collections::BTreeMap;
    use uuid::Uuid;

    /// 稳定 UUID（与 SG1 测试同法：禁止运行时随机，保证可复现）。
    const fn sg02_stable_uuid(seed: u16) -> Uuid {
        let mut bytes = [0u8; 16];
        bytes[14] = (seed >> 8) as u8;
        bytes[15] = seed as u8;
        bytes[6] = 0x70;
        bytes[8] = 0x80;
        Uuid::from_bytes(bytes)
    }

    fn sg02_structure_context(
        change_order: Vec<LogicalRepositoryId>,
        physical_by_logical: BTreeMap<LogicalRepositoryId, String>,
    ) -> PlanningStructureContext {
        PlanningStructureContext {
            source_revision_ref: "plan-source/sg-02".to_string(),
            plan_candidate_ir_ref: "plan-ir/sg-02".to_string(),
            source_hash: "sg02-source-hash-0000000000000000000000000000000".to_string(),
            compiler_version: "compiler/sg-02".to_string(),
            rule_version: "rule/sg-02".to_string(),
            context_digest: "sg-02-context-digest".to_string(),
            confirmed_design_id: Some("design_0001".to_string()),
            confirmed_design_digest: Some("design-digest-0001".to_string()),
            member_table_digest: Some("member-table-digest-0001".to_string()),
            selection_digest: Some("selection-digest-0001".to_string()),
            physical_by_logical,
            change_order,
            options_digest: "options-digest-0001".to_string(),
            baseline_tree_digest: Some("baseline-digest-0001".to_string()),
            intent_digest: Some("intent-digest-0001".to_string()),
        }
    }

    fn sg02_structure_item(
        logical_work_item_id: &str,
        logical_target: Option<LogicalRepositoryId>,
        inputs: &[(&str, &str)],
    ) -> PlanningStructureItem {
        PlanningStructureItem {
            logical_work_item_id: logical_work_item_id.to_string(),
            short_id: logical_work_item_id.to_string(),
            title: format!("Work Item {logical_work_item_id}"),
            declared_member: None,
            logical_target,
            physical_target: None,
            canonical_inputs: inputs
                .iter()
                .enumerate()
                .map(
                    |(index, (provider, contract_id))| PlanningStructureCanonicalInput {
                        contract: RequiredInputContract {
                            contract_id: contract_id.to_string(),
                            provider_logical_work_item_id: provider.to_string(),
                            required_capabilities: vec![format!("{contract_id}.capability")],
                            compatibility_policy: ContractCompatibilityPolicy::RequireAll,
                        },
                        span: PlanningStructureSpan {
                            field: format!("input_contracts[{index}]"),
                            line: 10 + index,
                        },
                    },
                )
                .collect(),
            declared_depends_on: Vec::new(),
            source_spans: vec![PlanningStructureSpan {
                field: "identity".to_string(),
                line: 1,
            }],
        }
    }

    /// R1 事故（设计 §4.3，与 SG1 同构）：WI-005 归 gateway（仓序在前）、
    /// Inputs 要求 WI-004（frontend，仓序在后）→ 数据流边 WI-004→WI-005 与
    /// 仓序自动边 WI-005→WI-004 互相等待。
    fn sg02_r1_fixture() -> (Vec<PlanningStructureItem>, PlanningStructureContext) {
        let gateway = LogicalRepositoryId(sg02_stable_uuid(0x0201));
        let frontend = LogicalRepositoryId(sg02_stable_uuid(0x0202));
        let physical_by_logical = BTreeMap::from([
            (gateway, "repository_gateway".to_string()),
            (frontend, "repository_frontend".to_string()),
        ]);
        let items = vec![
            sg02_structure_item("WI-004", Some(frontend), &[]),
            sg02_structure_item(
                "WI-005",
                Some(gateway),
                &[("WI-004", "contract.e2e-verification")],
            ),
        ];
        let context = sg02_structure_context(vec![gateway, frontend], physical_by_logical);
        (items, context)
    }

    fn sg02_protected_facts(
        gateway: LogicalRepositoryId,
        frontend: LogicalRepositoryId,
    ) -> ProtectedBusinessFacts {
        ProtectedBusinessFacts {
            business_request_digest: "sg02-business-request-digest".to_string(),
            confirmed_design_digest: Some("sg02-design-digest".to_string()),
            protected_field_refs: vec!["Inputs".to_string(), "Acceptance Criteria".to_string()],
            protected_contract_refs: vec!["contract.e2e-verification".to_string()],
            authorized_member_ids: vec![gateway.0.to_string(), frontend.0.to_string()],
        }
    }

    /// 手工 typed 失败（F1.1 载荷形态）：定位/身份沿 SG1 structure_failure
    /// 口径，供非拓扑 kind 用例构造失败集合。
    fn sg02_failure(kind: PlanningFailureKind, affected_ids: &[&str]) -> PlanningFailure {
        PlanningFailure {
            kind,
            origin: PlanningFailureOrigin::EffectiveGraph,
            stage: PlanningStage::Plan,
            diagnostic_ref: "planning-structure/sg02".to_string(),
            source_ref: "plan-source/sg-02".to_string(),
            source_hash: "sg02-source-hash-0000000000000000000000000000000".to_string(),
            evidence: PlanningFailureEvidence {
                affected_ids: affected_ids.iter().map(|id| id.to_string()).collect(),
                details: Default::default(),
            },
        }
    }

    fn sg02_r1_instructions() -> (PlanningStructureReport, Vec<StructuralRepairInstruction>) {
        let (items, context) = sg02_r1_fixture();
        let gateway = LogicalRepositoryId(sg02_stable_uuid(0x0201));
        let frontend = LogicalRepositoryId(sg02_stable_uuid(0x0202));
        let report = project_effective_planning_graph(&items, &context);
        assert!(
            !report.structurally_valid,
            "R1 fixture must carry the dependency cycle"
        );
        let facts = sg02_protected_facts(gateway, frontend);
        let instructions = build_structural_repair_instructions(&report.failures, &report, &facts);
        (report, instructions)
    }

    /// R1（REQ-PLN-09 场景/设计 §4.3）：机器指令的 required_action 必须同时
    /// 携两方向边的真实来源（canonical_input 契约+定位 / change_order 仓序
    /// 归属）、仓序位置与 Inputs 真实闭包；指令身份绑定 report 的
    /// refs/rule/digest，且确定性可重建。
    #[test]
    fn sg_02_no_fake_edge_removal_r1_required_action_carries_both_sources_order_and_inputs_closure()
    {
        let (report, instructions) = sg02_r1_instructions();
        assert_eq!(
            instructions.len(),
            1,
            "单个 DependencyCycle 失败产一条机器指令"
        );
        let instruction = &instructions[0];
        // 身份绑定：refs/hash/digest/rule 全部来自 SG1 报告单源。
        assert_eq!(instruction.source_hash, report.source_hash);
        assert_eq!(instruction.context_digest, report.context_digest);
        assert_eq!(instruction.rule_version, report.rule_version);
        assert!(!instruction.instruction_digest.is_empty());

        // 实际冲突边及来源：两方向边逐字 join 有效图（删一来源不等于删边）。
        assert_eq!(instruction.conflict_edges.len(), 2);
        let data_flow = instruction
            .conflict_edges
            .iter()
            .find(|edge| edge.from == "WI-004" && edge.to == "WI-005")
            .expect("数据流边 WI-004→WI-005 必须在冲突边内");
        assert_eq!(data_flow.sources, vec![PlanningEdgeSource::CanonicalInput]);
        let order_edge = instruction
            .conflict_edges
            .iter()
            .find(|edge| edge.from == "WI-005" && edge.to == "WI-004")
            .expect("仓序自动边 WI-005→WI-004 必须在冲突边内");
        assert_eq!(order_edge.sources, vec![PlanningEdgeSource::ChangeOrder]);
        assert_eq!(order_edge.order_indices, vec![0, 1]);

        // required_action：两来源 + 仓序 + Inputs 真实闭包（契约+字段定位）。
        for fragment in [
            "WI-004→WI-005",
            "canonical_input",
            "contract.e2e-verification",
            "input_contracts[0]",
            "WI-005→WI-004",
            "change_order",
            "仓序",
            "repository_gateway",
            "repository_frontend",
        ] {
            assert!(
                instruction.required_action.contains(fragment),
                "required_action 必须携带真实事实「{fragment}」：{}",
                instruction.required_action
            );
        }

        // 确定性：同输入重建全等；受保护事实变化改变指令摘要（绑定防漂移）。
        assert_eq!(
            build_structural_repair_instructions(
                &report.failures,
                &report,
                &instruction.protected_facts
            ),
            instructions
        );
        let mut drifted = instruction.protected_facts.clone();
        drifted.business_request_digest = "drifted-request".to_string();
        let changed = build_structural_repair_instructions(&report.failures, &report, &drifted);
        assert_ne!(
            changed[0].instruction_digest, instruction.instruction_digest,
            "instruction_digest 必须绑定 protected facts"
        );
    }

    /// 多个安全候选不给唯一标签（REQ-SG-02A/REQ-PLN-10）：依赖环与归属
    /// 不一致都是开放重组域——Candidates 形态、结构上无唯一性字段。
    #[test]
    fn sg_02_no_fake_edge_removal_multiple_safe_candidates_carry_no_unique_label() {
        let (_, instructions) = sg02_r1_instructions();
        let instruction = &instructions[0];
        let StructuralRepairSolution::Candidates { candidates } = &instruction.solution else {
            panic!("依赖环指令必须是候选形态，不得冒充唯一解");
        };
        assert!(
            candidates.len() >= 2,
            "安全候选至少两族：归属/拆分重组与删/替依赖（需证据）"
        );
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.title.contains("归属")
                    || candidate.title.contains("重组")),
            "必须含归属/任务重组候选：{:?}",
            candidates.iter().map(|c| &c.title).collect::<Vec<_>>()
        );
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.title.contains("删") && candidate.title.contains("依赖")),
            "必须含「删/替依赖需证据」候选：{:?}",
            candidates.iter().map(|c| &c.title).collect::<Vec<_>>()
        );

        // TargetMismatch（归属开放域）同样候选形态、无唯一标签。
        let (items, context) = sg02_r1_fixture();
        let report = project_effective_planning_graph(&items, &context);
        let facts = sg02_protected_facts(
            LogicalRepositoryId(sg02_stable_uuid(0x0201)),
            LogicalRepositoryId(sg02_stable_uuid(0x0202)),
        );
        let set = PlanningFailureSet {
            failures: vec![sg02_failure(
                PlanningFailureKind::TargetMismatch {
                    logical_work_item_id: "WI-005".to_string(),
                    declared_target: Some("repository_frontend".to_string()),
                    resolved_target: "repository_gateway".to_string(),
                },
                &["WI-005"],
            )],
        };
        let mismatch = build_structural_repair_instructions(&set, &report, &facts);
        assert_eq!(mismatch.len(), 1);
        assert!(
            matches!(
                &mismatch[0].solution,
                StructuralRepairSolution::Candidates { .. }
            ),
            "归属重组不携带唯一标签"
        );
    }

    /// 仅局部固定 grammar / 完整核验历史给唯一证据（REQ-PLN-10）：heading
    /// →FixedGrammarMapping（模板重表达）；回归→HistoryVerified（旧合法
    /// refs）；capability 缺口等其余域不声明唯一。
    #[test]
    fn sg_02_no_fake_edge_removal_uniqueness_only_for_fixed_grammar_or_verified_history() {
        let (items, context) = sg02_r1_fixture();
        let report = project_effective_planning_graph(&items, &context);
        let facts = sg02_protected_facts(
            LogicalRepositoryId(sg02_stable_uuid(0x0201)),
            LogicalRepositoryId(sg02_stable_uuid(0x0202)),
        );

        // 非法二级 heading（REQ-SG-02 第 414 行场景）：固定 grammar 映射唯一。
        let heading = build_structural_repair_instructions(
            &PlanningFailureSet {
                failures: vec![sg02_failure(
                    PlanningFailureKind::InvalidWorkItemHeading {
                        line: 414,
                        field: "work_item_heading".to_string(),
                        raw_line: "## 验收说明".to_string(),
                        example: "## Work Item WI-005: 端到端验证".to_string(),
                    },
                    &["WI-005"],
                )],
            },
            &report,
            &facts,
        );
        assert_eq!(heading.len(), 1);
        let StructuralRepairSolution::Precise {
            operations,
            uniqueness,
        } = &heading[0].solution
        else {
            panic!("heading 指令必须是精确操作形态");
        };
        assert_eq!(
            operations,
            &vec![StructuralRepairOperation::RewriteWithGrammarTemplate {
                line: 414,
                raw_line: "## 验收说明".to_string(),
                template: "## Work Item WI-005: 端到端验证".to_string(),
            }],
            "heading 指令携原行+合法模板，保留原业务段"
        );
        assert_eq!(
            uniqueness,
            &Some(PreciseUniqueness::FixedGrammarMapping {
                mapping: "work_item_heading→## Work Item WI-005: 端到端验证".to_string(),
            })
        );

        // 已解决结构事实回归：历史核验唯一证据 + 整组旧合法 refs。
        let last_valid_refs = LastValidStructureRefs {
            source_ref: "source/old".to_string(),
            ir_ref: "ir/old".to_string(),
            report_ref: "report/old".to_string(),
        };
        let regression = build_structural_repair_instructions(
            &PlanningFailureSet {
                failures: vec![sg02_failure(
                    PlanningFailureKind::StructuralRegression {
                        resolved_fact_digest: "resolved-fact-0001".to_string(),
                        last_valid_refs: last_valid_refs.clone(),
                        difference: "WI-005→WI-004 仓序冲突回归".to_string(),
                    },
                    &["WI-005", "WI-004"],
                )],
            },
            &report,
            &facts,
        );
        assert_eq!(regression.len(), 1);
        assert_eq!(regression[0].previous_valid_refs, Some(last_valid_refs));
        let StructuralRepairSolution::Precise { uniqueness, .. } = &regression[0].solution else {
            panic!("回归指令必须是精确操作形态");
        };
        assert_eq!(
            uniqueness,
            &Some(PreciseUniqueness::HistoryVerified {
                resolved_fact_digest: "resolved-fact-0001".to_string(),
            })
        );

        // capability 缺口：精确追加存在，但解域开放（删/替需求需证据）——
        // 不声明唯一。
        let capability = build_structural_repair_instructions(
            &PlanningFailureSet {
                failures: vec![sg02_failure(
                    PlanningFailureKind::RequiredCapabilityMissing {
                        provider: "WI-001".to_string(),
                        consumer: "WI-002".to_string(),
                        contract_id: "contract.levels-api".to_string(),
                        capability: "api.levels.write".to_string(),
                    },
                    &["WI-001", "WI-002"],
                )],
            },
            &report,
            &facts,
        );
        assert_eq!(capability.len(), 1);
        assert!(
            matches!(
                &capability[0].solution,
                StructuralRepairSolution::Candidates { .. }
            ),
            "capability 缺口不携带唯一标签"
        );
    }

    /// 删必需 Input/AC 或只改 Dependencies 拒（REQ-SG-02A 禁止假删边场景）：
    /// 保护项具体化为 forbidden_operations + 全量复验谓词 + 受保护事实
    /// 逐字绑定；Operational/Unknown 不产指令（fail-closed 不驱 author）。
    #[test]
    fn sg_02_no_fake_edge_removal_rejects_input_ac_deletion_and_dependencies_only_edits() {
        let (_, instructions) = sg02_r1_instructions();
        let instruction = &instructions[0];

        let forbidden = instruction.forbidden_operations.join("\n");
        assert!(
            forbidden.contains("Inputs"),
            "禁止操作必须显式拒绝删必需 Inputs：{forbidden}"
        );
        assert!(
            forbidden.contains("AC") || forbidden.contains("验收"),
            "禁止操作必须显式拒绝删/放宽 AC：{forbidden}"
        );
        assert!(
            forbidden.contains("Dependencies"),
            "禁止操作必须显式拒绝仅改 Dependencies：{forbidden}"
        );
        assert!(
            forbidden.contains("仓序"),
            "禁止操作必须显式拒绝改已确认仓序：{forbidden}"
        );
        // 允许闭包内不得出现删除语义（删除不是“允许的修改”）。
        assert!(
            !instruction
                .allowed_change_closure
                .iter()
                .any(|allowed| allowed.contains("删除")),
            "允许闭包不得包含删除：{:?}",
            instruction.allowed_change_closure
        );

        // 全量复验谓词：无环不是业务正确证明，Inputs/AC/仓序保留各自成谓词。
        for predicate in [
            "effective_graph_acyclic",
            "canonical_inputs_preserved",
            "acceptance_criteria_preserved",
            "change_order_unchanged",
        ] {
            assert!(
                instruction
                    .expected_predicates
                    .iter()
                    .any(|item| item.contains(predicate)),
                "expected_predicates 必须含 {predicate}：{:?}",
                instruction.expected_predicates
            );
        }
        // 受保护事实逐字绑定（authorized members 可指导归属重组闭包）。
        assert_eq!(
            instruction.protected_facts.authorized_member_ids.len(),
            2,
            "授权成员事实进入指令"
        );
        assert_eq!(
            instruction.protected_facts.protected_contract_refs,
            vec!["contract.e2e-verification".to_string()]
        );

        // Operational/Unknown：零指令（安全等待/协议恢复，不驱 author）。
        let (items, context) = sg02_r1_fixture();
        let report = project_effective_planning_graph(&items, &context);
        let facts = sg02_protected_facts(
            LogicalRepositoryId(sg02_stable_uuid(0x0201)),
            LogicalRepositoryId(sg02_stable_uuid(0x0202)),
        );
        for kind in [
            PlanningFailureKind::Operational {
                stable_code: "internal_store_error".to_string(),
                cause: "store unreachable".to_string(),
            },
            PlanningFailureKind::Unknown {
                code: "legacy_wire_code".to_string(),
            },
        ] {
            let set = PlanningFailureSet {
                failures: vec![sg02_failure(kind, &["WI-005"])],
            };
            assert!(
                build_structural_repair_instructions(&set, &report, &facts).is_empty(),
                "Operational/Unknown 不产机器指令（fail-closed）"
            );
        }
    }

    /// SG7 Step1 `sg_07_machine_repair_optional_literal_cutover`（producer→
    /// wire 完整绑定臂）：真实 SG2 producer（`build_contract_autorepair_
    /// instructions`，同 source/IR/context）产出 L1 指令，经显式配对闸投影
    /// 为 `MachineRepairDto` 后由真实 contract 适配进 ReviewFinding——wire
    /// JSON 携带与指令逐字同源的 source/context/repair/instruction_ref。
    #[test]
    fn sg_07_machine_repair_contract_pairing_binds_wire_identity() {
        let (source, ir) = gap_source_and_ir();
        let context = crate::product::work_item_plan_compiler::WorkItemPlanSourceContext {
            target_repository_id: "repo_fixture".to_string(),
            member_targets: None,
        };
        let instructions =
            super::super::contract_autorepair::build_contract_autorepair_instructions(
                &source, &ir, &context,
            );
        assert_eq!(
            instructions.len(),
            1,
            "capability gap fixture must produce exactly one L1 instruction"
        );
        let instruction = &instructions[0];

        let dto = machine_repair_dto_for_instruction(
            instruction,
            &instruction.source_hash,
            &instruction.context_digest,
        )
        .expect("same source/context pairing must project Some");
        assert_eq!(dto.repair_id, instruction.repair_id);
        assert_eq!(dto.failure_id, instruction.failure_id);
        assert_eq!(dto.source_hash, instruction.source_hash);
        assert_eq!(dto.context_digest, instruction.context_digest);
        assert_eq!(dto.status, MachineRepairStatus::Offered);
        assert_eq!(
            dto.instruction_ref,
            format!(
                "structural-repair-instruction/{}",
                instruction.instruction_digest
            )
        );

        // 真实 contract 适配：Some 只附加 machine_repair，业务面
        // category/class_hint/severity/required_action 原样保留。
        let contracts = ir
            .items
            .iter()
            .map(|item| item.contract.clone())
            .collect::<Vec<_>>();
        let graph = build_dependency_contract_graph(&contracts).expect("graph");
        let report = validate_dependency_contract_graph(&graph);
        let gap_finding = report
            .findings
            .iter()
            .find(|finding| finding.code == "required_capability_missing")
            .expect("capability gap finding");
        let paired = contract_finding_to_review_finding(gap_finding, Some(&graph), Some(dto));
        let baseline = contract_finding_to_review_finding(gap_finding, Some(&graph), None);
        assert_eq!(paired.severity, baseline.severity);
        assert_eq!(paired.category, baseline.category);
        assert_eq!(paired.class_hint, baseline.class_hint);
        assert_eq!(paired.required_action, baseline.required_action);
        assert_eq!(paired.message, baseline.message);
        assert_ne!(paired.machine_repair, None);

        // 到 wire 的完整绑定：ReviewVerdict 序列化后 machine_repair 可达。
        let verdict = ReviewVerdict {
            verdict: ReviewVerdictType::Revise,
            comments: "mechanical".to_string(),
            summary: "gap".to_string(),
            findings: vec![paired],
            review_gate: ReviewGate::RequiresRevision,
            work_item_plan_review: None,
            structured_output_diagnostic: None,
        };
        let wire = serde_json::to_value(&verdict).expect("wire json");
        assert!(
            wire["findings"][0]["machine_repair"]["repair_id"]
                .as_str()
                .is_some_and(|value| value == instruction.repair_id),
            "machine repair must reach the wire bound to the instruction identity: {wire}"
        );
    }

    /// SG7 Step1：配对闸 fail-closed——source hash 或 context digest 任一
    /// 漂移即 None（不从 message/class_hint 猜资格）；干净候选零指令、
    /// Warning finding 无指令可配恒 None。
    #[test]
    fn sg_07_machine_repair_pairing_is_none_on_drift_or_missing_instruction() {
        let (source, ir) = gap_source_and_ir();
        let context = crate::product::work_item_plan_compiler::WorkItemPlanSourceContext {
            target_repository_id: "repo_fixture".to_string(),
            member_targets: None,
        };
        let instructions =
            super::super::contract_autorepair::build_contract_autorepair_instructions(
                &source, &ir, &context,
            );
        let instruction = &instructions[0];

        let drifted_source = machine_repair_dto_for_instruction(
            instruction,
            &"0f".repeat(64),
            &instruction.context_digest,
        );
        assert_eq!(drifted_source, None, "source hash drift must not pair");
        let drifted_context = machine_repair_dto_for_instruction(
            instruction,
            &instruction.source_hash,
            &"1f".repeat(64),
        );
        assert_eq!(drifted_context, None, "context digest drift must not pair");

        // 干净候选：SG2 producer 零指令 → 无可配对，Some 不存在。
        let clean_source = source.replace(
            "- required_capabilities: api.levels.read, api.levels.write\n",
            "- required_capabilities: api.levels.read\n",
        );
        let clean_ir = crate::product::work_item_plan_compiler::compile_work_item_plan(
            &clean_source,
            &context,
        )
        .expect("clean compile");
        assert!(
            super::super::contract_autorepair::build_contract_autorepair_instructions(
                &clean_source,
                &clean_ir,
                &context
            )
            .is_empty(),
            "clean candidate must carry no L1 instruction to pair"
        );

        // Warning finding：不产指令（仅 Error 级缺口进 producer），适配恒 None。
        let warning = ContractValidationFinding {
            code: "process_evidence_acceptance_criterion".to_string(),
            severity: ContractFindingSeverity::Warning,
            logical_work_item_id: Some("WI-001".to_string()),
            contract_ref: Some("AC-001".to_string()),
            capability_ref: None,
            message: "acceptance criterion must describe an observable result".to_string(),
            kind: None,
        };
        let finding = contract_finding_to_review_finding(&warning, None, None);
        assert_eq!(finding.machine_repair, None);
    }
}
