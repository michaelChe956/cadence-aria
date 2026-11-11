//! 确定性契约缺口的机器指令 producer 与单次编译收敛入口（DEF-PVR-ALL
//! 主方案 + O1 裁决二 §2.2/§2.4：内容 autorepair 迁原授权 author）。
//!
//! 平台只诊断与开方，不替 author 改写 source：四类内容缺口一律产出
//! `StructuralRepairInstruction`（复用 `contract_prerevision` 既有类型）
//! 交原授权 author 在保护闭包内产出新 revision——
//!
//! - `required_capability_missing`：不自动追加 capability（consumer 提出
//!   需求不证明 provider 已实现能力）；产「回查原业务与实现承诺、保护
//!   闭包内产新 revision」指令。
//! - `unconsumed_required_handoff`：不删 ref/替换 `[]`（不消费不等于
//!   不需要）；产携带 provider、原始行值与未消费引用集的完整来源事实
//!   指令。
//! - `unknown_structured_key`：不按诊断行删业务承载行；只产「保留内容、
//!   迁往合法字段」指令（携带 typed kind 的诊断按类型排除，混合集合
//!   整体 fail-closed）。
//! - 重复 trusted command：不本地删 command/整 check；产「指出重复及
//!   受影响 check/ref（manual 与无 manual 两支处置均交 author）」指令。
//!
//! producer 是纯函数（零 source mutation、零 provider/模型调用、零修复
//! 轮次）；本地八轮 mutation 收敛与 `apply_structural_repair_operations`/
//! `rewrite_source` 已按 O1 裁决移除，不保同名旧行为 shim。残余 Error
//! 走既有机械 verdict 通道（F5-A 模型返修），source/IR/report 保持
//! author 产物同源新鲜。

use std::collections::{BTreeMap, BTreeSet};

use crate::product::work_item_contract::{
    ContractFindingSeverity, DependencyContractGraph, build_dependency_contract_graph,
    validate_dependency_contract_graph,
};
use crate::product::work_item_plan_compiler::{
    CompiledPlanAudit, CompilerDiagnostic, PlanCandidateIr, WorkItemPlanSourceContext,
    compile_work_item_plan_audited, parse_work_item_plan,
};
use crate::product::workspace_engine::contract_prerevision::{
    ProtectedBusinessFacts, StructuralRepairCandidate, StructuralRepairInstruction,
    StructuralRepairSolution, seal_structural_repair_instruction,
};
use crate::product::workspace_engine::draft_batch::compile_support::PlanningStructureSpan;
use sha2::{Digest, Sha256};

/// SG2 规则域版本：本模块确定性发现规则的稳定标识（指令身份绑定）。
const CONTRACT_AUTOREPAIR_RULE_VERSION: &str = "contract_autorepair/v1";

/// O1（裁决二）：converge 出口——单次编译的 IR、收窄后四类 producer 的
/// 机器指令与 Task 1 等价解析审计。不再返回 repaired_source、不再有
/// 修复轮次：source 改写归原授权 author。
#[derive(Debug)]
pub(crate) struct ContractConvergeOutcome {
    pub ir: PlanCandidateIr,
    pub instructions: Vec<StructuralRepairInstruction>,
    pub audit: CompiledPlanAudit,
}

/// 单次收敛入口（O1 收窄）：`compile_work_item_plan_audited` 一次——
/// 失败直接 `Err(diagnostics)`（不修，交 handler 教学重驱/终态面）；
/// 成功则产出机器指令（不应用）与等价解析审计。source 字节不变。
pub(crate) fn converge_work_item_plan_source(
    source: &str,
    context: WorkItemPlanSourceContext,
) -> Result<ContractConvergeOutcome, Vec<CompilerDiagnostic>> {
    let (ir, audit) = compile_work_item_plan_audited(source, &context)?;
    let instructions = build_contract_autorepair_instructions(source, &ir, &context);
    Ok(ContractConvergeOutcome {
        ir,
        instructions,
        audit,
    })
}

/// 指令文本渲染（与 handler `format_compile_failure_reasons` 同源的
/// `身份:动作` 形态，供失败面/owner 呈现拼接）。
pub(crate) fn format_contract_autorepair_instructions(
    instructions: &[StructuralRepairInstruction],
) -> Vec<String> {
    instructions
        .iter()
        .map(|instruction| format!("{}:{}", instruction.repair_id, instruction.required_action))
        .collect()
}

/// Task 3 评审 Minor-3（Task 4 补正）：契约修复指令进入 owner 可见面
/// （handler 终态 error / 指令事件 / completion message / 教学重驱 prompt）
/// 的条数上限——`required_action` 每条含禁则/证据多句，无界透传会撑爆
/// 日志/通知面。**全部面共用本常量与 [`cap_autorepair_instruction_lines`]**，
/// 不得各写一套阈值。
pub(crate) const MAX_AUTOREPAIR_INSTRUCTION_LINES: usize = 4;

/// 指令行进入 owner 可见面前的截断：条数上限 + 计数摘要行收口。
pub(crate) fn cap_autorepair_instruction_lines(lines: Vec<String>) -> Vec<String> {
    let total = lines.len();
    let mut bounded: Vec<String> = lines
        .into_iter()
        .take(MAX_AUTOREPAIR_INSTRUCTION_LINES)
        .collect();
    if bounded.len() < total {
        bounded.push(format!(
            "（共 {total} 条契约修复指令，此处仅逐条呈现前 {MAX_AUTOREPAIR_INSTRUCTION_LINES} 条；其余以计数摘要收口，不无界透传）"
        ));
    }
    bounded
}

/// SG2：compile 成功路径的机器指令 producer——对 IR canonical 契约校验的
/// 两类缺口（required_capability_missing / unconsumed_required_handoff）
/// 生成交原授权 author 的机器指令。纯函数：零 source mutation、零
/// provider/模型调用；`uniqueness` 恒 None（REQ-PLN-10：内容补齐不声明
/// 唯一解）。O1 收窄：指令只携带 typed 缺口事实、定位与 author 处置
/// 口径，不携带行编辑操作。
pub(crate) fn build_contract_autorepair_instructions(
    source: &str,
    ir: &PlanCandidateIr,
    context: &WorkItemPlanSourceContext,
) -> Vec<StructuralRepairInstruction> {
    let contracts = ir
        .items
        .iter()
        .map(|item| item.contract.clone())
        .collect::<Vec<_>>();
    // Err（重复 identity 等）不含两类缺口，交模型返修。
    let Ok(graph) = build_dependency_contract_graph(&contracts) else {
        return Vec::new();
    };
    let report = validate_dependency_contract_graph(&graph);

    let mut capability_gaps = Vec::new();
    let mut handoff_gaps = Vec::new();
    for finding in &report.findings {
        if finding.severity != ContractFindingSeverity::Error {
            continue;
        }
        match finding.code.as_str() {
            "required_capability_missing" => {
                // 定位三元组/provider 任一缺失即放弃该条（不整体短路），
                // 残余走模型返修。
                let (Some(consumer), Some(contract_id), Some(capability)) = (
                    finding.logical_work_item_id.as_deref(),
                    finding.contract_ref.as_deref(),
                    finding.capability_ref.as_deref(),
                ) else {
                    continue;
                };
                let Some(provider) = provider_for_consumer_contract(&graph, consumer, contract_id)
                else {
                    continue;
                };
                capability_gaps.push(CapabilityGap {
                    provider: provider.to_string(),
                    contract_id: contract_id.to_string(),
                    capability: capability.to_string(),
                });
            }
            "unconsumed_required_handoff" => {
                // 该 finding 的 logical_work_item_id 即 provider。
                let (Some(provider), Some(contract_ref)) = (
                    finding.logical_work_item_id.as_deref(),
                    finding.contract_ref.as_deref(),
                ) else {
                    continue;
                };
                handoff_gaps.push(HandoffGap {
                    provider: provider.to_string(),
                    contract_ref: contract_ref.to_string(),
                });
            }
            _ => {}
        }
    }
    if capability_gaps.is_empty() && handoff_gaps.is_empty() {
        return Vec::new();
    }
    let Ok(ast) = parse_work_item_plan(source) else {
        return Vec::new();
    };
    let identity = InstructionIdentity::for_compiled(source, context, ir);
    let mut instructions = Vec::new();

    // capability 缺口按 (provider, contract_id) 聚合：同契约多条缺失并入
    // 同一指令（BTreeMap 键序确定，指令与日志均确定）。
    let mut gaps_by_contract =
        BTreeMap::<(String, String), std::collections::BTreeSet<String>>::new();
    for gap in capability_gaps {
        // 多个消费方可各自报出同一缺失 capability：组内去重。
        gaps_by_contract
            .entry((gap.provider, gap.contract_id))
            .or_default()
            .insert(gap.capability);
    }
    for ((provider, contract_id), capabilities) in gaps_by_contract {
        let anchor_line = match outputs_capabilities_anchor(&ast, &provider, &contract_id) {
            OutputsAnchor::MergeInto { line, .. } => line,
            OutputsAnchor::InsertAfter { line } => line,
            OutputsAnchor::NotFound => continue,
        };
        instructions.push(capability_instruction(
            &identity,
            &provider,
            &contract_id,
            &capabilities,
            anchor_line,
        ));
    }

    // 未消费 handoff 引用按 provider 聚合，逐行生成完整来源事实指令。
    let mut refs_by_provider = BTreeMap::<String, Vec<String>>::new();
    for gap in handoff_gaps {
        refs_by_provider
            .entry(gap.provider)
            .or_default()
            .push(gap.contract_ref);
    }
    for (provider, contract_refs) in refs_by_provider {
        for (line, value) in handoff_provided_ref_fields(&ast, &provider) {
            let parts = split_ref_parts(&value);
            let unconsumed = parts
                .iter()
                .filter(|part| contract_refs.contains(part))
                .cloned()
                .collect::<Vec<_>>();
            if unconsumed.is_empty() {
                continue;
            }
            instructions.push(handoff_instruction(
                &identity,
                &provider,
                &unconsumed,
                line,
                &value,
            ));
        }
    }
    instructions
}

/// SG2：compile 失败路径的机器指令 producer——只消费明确映射到「自身源
/// 行」的未知结构化 key 诊断与重复 trusted command 诊断，生成交原授权
/// author 的机器指令；携带 typed kind 的诊断（非法二级 heading 等）永不
/// 进入指令分支（REQ-SG-02 第 414 行场景：不按共享 unknown key 码处置
/// heading 行），任一诊断不可精确映射则该路径整体 fail-closed（空指令）。
/// 纯函数：零 source mutation、零模型调用。O1 收窄：不删行——未知 key
/// 产「保留内容、迁往合法字段」指令，重复 command 产「指出重复及受影
/// 响 check/ref」指令。
pub(crate) fn build_compile_failure_repair_instructions(
    source: &str,
    diagnostics: &[CompilerDiagnostic],
    context: &WorkItemPlanSourceContext,
) -> Vec<StructuralRepairInstruction> {
    let unknown_key = unknown_structured_key_instructions(source, diagnostics, context);
    if !unknown_key.is_empty() {
        return unknown_key;
    }
    duplicate_trusted_command_instructions(source, diagnostics, context)
}

/// 指令身份材料（producer 域内真实可得的事实，不虚构上层 digest）。
struct InstructionIdentity {
    source_hash: String,
    context_digest: String,
    /// 本层事实域内的业务基线摘要：当前交付的 canonical 契约投影（供需/
    /// 能力的机器可读事实）。request 级业务 digest 绑定由 SG5 revision
    /// prompt 层携带，本层不虚构。
    business_baseline_digest: String,
    authorized_member_ids: Vec<String>,
}

impl InstructionIdentity {
    fn for_compiled(
        source: &str,
        context: &WorkItemPlanSourceContext,
        ir: &PlanCandidateIr,
    ) -> Self {
        let contracts = ir
            .items
            .iter()
            .map(|item| &item.contract)
            .collect::<Vec<_>>();
        let projection = serde_json::to_string(&contracts)
            .unwrap_or_else(|error| unreachable!("contract projection serialization: {error}"));
        Self {
            source_hash: hex::encode(Sha256::digest(source.as_bytes())),
            context_digest: compile_context_digest(context),
            business_baseline_digest: format!(
                "canonical-contract-projection:{}",
                hex::encode(Sha256::digest(projection.as_bytes()))
            ),
            authorized_member_ids: authorized_member_ids(context),
        }
    }

    fn for_compile_failure(source: &str, context: &WorkItemPlanSourceContext) -> Self {
        let source_hash = hex::encode(Sha256::digest(source.as_bytes()));
        Self {
            business_baseline_digest: format!("delivered-source:{source_hash}"),
            source_hash,
            context_digest: compile_context_digest(context),
            authorized_member_ids: authorized_member_ids(context),
        }
    }
}

/// compile 上下文 digest（指令 context_digest 绑定）：对 target 与成员
/// 归属表投影的确定性摘要。
fn compile_context_digest(context: &WorkItemPlanSourceContext) -> String {
    let member_targets = context.member_targets.as_ref().map(|targets| {
        let pairs: Vec<String> = targets
            .physical_by_logical
            .iter()
            .map(|(logical, physical)| format!("{logical:?}->{physical}"))
            .collect();
        format!(
            "members=[{}];fallback={:?}",
            pairs.join(","),
            targets.fallback_physical_id
        )
    });
    let material = match member_targets {
        Some(member_targets) => format!("{};{member_targets}", context.target_repository_id),
        None => format!("{};members=none", context.target_repository_id),
    };
    hex::encode(Sha256::digest(material.as_bytes()))
}

fn authorized_member_ids(context: &WorkItemPlanSourceContext) -> Vec<String> {
    context
        .member_targets
        .as_ref()
        .map(|targets| {
            targets
                .physical_by_logical
                .keys()
                .map(|logical| format!("{logical:?}"))
                .collect()
        })
        .unwrap_or_default()
}

/// 指令公共组装（producer 域内事实组装 + 摘要封口）。
#[allow(clippy::too_many_arguments)] // 组装面:逐字段对应 StructuralRepairInstruction 的必填事实。
fn build_local_instruction(
    identity: &InstructionIdentity,
    failure_binding: String,
    affected_work_item_ids: Vec<String>,
    affected_spans: Vec<PlanningStructureSpan>,
    allowed_change_closure: Vec<String>,
    forbidden_operations: Vec<String>,
    solution: StructuralRepairSolution,
    required_action: String,
    expected_predicates: Vec<String>,
    protected_contract_refs: Vec<String>,
) -> StructuralRepairInstruction {
    seal_structural_repair_instruction(StructuralRepairInstruction {
        repair_id: format!(
            "contract-autorepair/{CONTRACT_AUTOREPAIR_RULE_VERSION}/{failure_binding}"
        ),
        failure_id: format!("contract-validation/{failure_binding}"),
        rule_version: CONTRACT_AUTOREPAIR_RULE_VERSION.to_string(),
        source_hash: identity.source_hash.clone(),
        context_digest: identity.context_digest.clone(),
        affected_work_item_ids,
        affected_spans,
        conflict_edges: Vec::new(),
        allowed_change_closure,
        forbidden_operations,
        protected_facts: ProtectedBusinessFacts {
            business_request_digest: identity.business_baseline_digest.clone(),
            confirmed_design_digest: None,
            protected_field_refs: vec![
                "Inputs.required_capabilities".to_string(),
                "Outputs.capabilities".to_string(),
                "Handoff Schema.provided_contract_refs".to_string(),
            ],
            protected_contract_refs,
            authorized_member_ids: identity.authorized_member_ids.clone(),
        },
        solution,
        required_action,
        expected_predicates,
        previous_valid_refs: None,
        instruction_digest: String::new(),
    })
}

/// capability 缺口指令（O1 收窄）：不携带行编辑操作——consumer 提出需求
/// 不证明 provider 已实现能力；author 必须回查原业务与实现承诺、在保护
/// 闭包内产出新 revision，不能仅给缺口打补丁。指令携带 typed 缺口事实
/// （provider、契约、缺失能力集）与生效 capabilities 行定位。
fn capability_instruction(
    identity: &InstructionIdentity,
    provider: &str,
    contract_id: &str,
    capabilities: &std::collections::BTreeSet<String>,
    anchor_line: usize,
) -> StructuralRepairInstruction {
    let capabilities_text = capabilities.iter().cloned().collect::<Vec<_>>().join("、");
    let expected_predicates = capabilities
        .iter()
        .map(|capability| format!("capability_satisfied:{contract_id}::{capability}"))
        .collect::<Vec<_>>();
    build_local_instruction(
        identity,
        format!("required_capability_missing/{provider}/{contract_id}"),
        vec![provider.to_string()],
        vec![PlanningStructureSpan {
            field: format!("outputs.{contract_id}.capabilities"),
            line: anchor_line,
        }],
        vec![format!(
            "{provider} Outputs 契约 {contract_id} 的能力声明（回查原业务与实现承诺后在保护闭包内修订）"
        )],
        vec![
            "平台代为追加 capability（O1：平台不替 author 改写业务内容）".to_string(),
            "删除消费方的 required_capabilities 需求（假修复）".to_string(),
            "删除 provider 既有 capabilities".to_string(),
        ],
        StructuralRepairSolution::Candidates {
            candidates: vec![StructuralRepairCandidate {
                title: format!(
                    "回查原业务与实现承诺：核对 {contract_id} 是否应提供 {capabilities_text}，在保护闭包内产出新 revision（补齐 provider capabilities 或修正消费方需求）"
                ),
                constraints: vec![
                    "先核对消费方需求与 provider 实现承诺，不直接给缺口打补丁".to_string(),
                    "消费方真实需求与 provider 既有 capabilities 不得删除".to_string(),
                ],
                evidence: vec![
                    format!("provider={provider}"),
                    format!("contract={contract_id}"),
                    format!("缺失 capabilities={capabilities_text}"),
                    format!("定位=outputs.{contract_id}.capabilities 行{anchor_line}"),
                ],
                verified_predicates: expected_predicates.clone(),
            }],
        },
        format!(
            "回查原业务与实现承诺：消费方要求 {contract_id} 提供 {capabilities_text}，provider {provider} 的 Outputs 契约未声明该能力。核对业务事实后在保护闭包内产出新 revision（补齐 provider capabilities，或修正消费方需求）——平台不替 author 追加 capability。"
        ),
        expected_predicates,
        vec![contract_id.to_string()],
    )
}

/// 未消费 handoff 引用指令（O1 收窄）：不携带删 ref/替换 `[]` 的行编辑
/// ——不消费不等于不需要；指令携带完整来源事实（provider、原始行值、
/// 未消费引用集），author 可合法重组（移除引用或补齐下游消费方），
/// 平台不得删真实边过门。
fn handoff_instruction(
    identity: &InstructionIdentity,
    provider: &str,
    unconsumed_refs: &[String],
    anchor_line: usize,
    line_value: &str,
) -> StructuralRepairInstruction {
    build_local_instruction(
        identity,
        format!("unconsumed_required_handoff/{provider}/行{anchor_line}"),
        vec![provider.to_string()],
        vec![PlanningStructureSpan {
            field: "handoff_schema.provided_contract_refs".to_string(),
            line: anchor_line,
        }],
        vec![format!(
            "{provider} ### Handoff Schema 的 provided_contract_refs 声明（回查供需事实后在保护闭包内重组）"
        )],
        vec![
            "平台代为删除引用或改写为 []（O1：平台不替 author 改写业务内容）".to_string(),
            "移除仍被消费的 handoff 引用".to_string(),
            "删除消费方对仍需契约的真实需求".to_string(),
        ],
        StructuralRepairSolution::Candidates {
            candidates: vec![StructuralRepairCandidate {
                title: format!(
                    "回查 handoff 供需事实：核对 {} 是否仍需由 {provider} 提供，在保护闭包内产出新 revision（移除未消费引用，或补齐下游消费方）",
                    unconsumed_refs.join("、")
                ),
                constraints: vec![
                    "真实供需边不得凭「未消费」直接删除（不消费不等于不需要）".to_string(),
                    "仍被消费的引用与消费方真实需求不得删除".to_string(),
                ],
                evidence: vec![
                    format!("provider={provider}"),
                    format!("行{anchor_line} 原值=provided_contract_refs: {line_value}"),
                    format!("未消费引用={}", unconsumed_refs.join("、")),
                ],
                verified_predicates: vec![format!("no_unconsumed_required_handoff:{provider}")],
            }],
        },
        format!(
            "回查 handoff 供需事实：{provider} 的 ### Handoff Schema 行 {anchor_line} 声明 provided_contract_refs（原值：{line_value}），其中 {} 无任何消费方。核对业务事实后在保护闭包内产出新 revision（移除未消费引用，或补齐下游消费方）——平台不替 author 删引用或改写为 []。",
            unconsumed_refs.join("、")
        ),
        vec![format!("no_unconsumed_required_handoff:{provider}")],
        unconsumed_refs.to_vec(),
    )
}

/// 未知结构化 key 指令（O1 收窄）：不按诊断行删业务承载行——只产
/// 「保留内容、迁往合法字段」指令，携带该行原文（fail-closed：任一
/// 不可映射即整批放弃）。
fn unknown_structured_key_instructions(
    source: &str,
    diagnostics: &[CompilerDiagnostic],
    context: &WorkItemPlanSourceContext,
) -> Vec<StructuralRepairInstruction> {
    let identity = InstructionIdentity::for_compile_failure(source, context);
    let mut instructions = Vec::new();
    let mut seen_lines = BTreeSet::new();
    for diagnostic in diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == UNKNOWN_STRUCTURED_KEY_CODE)
    {
        let Some(line) = unknown_structured_key_line(source, diagnostic) else {
            // 行号错位/越界/typed heading（REQ-SG-02 第 414 行场景）：
            // 整体 fail-closed，不产生任何指令。
            return Vec::new();
        };
        if seen_lines.insert(line) {
            let raw_line = source
                .lines()
                .nth(line.saturating_sub(1))
                .unwrap_or_default()
                .trim()
                .to_string();
            instructions.push(build_local_instruction(
                &identity,
                format!("unknown_structured_key/行{line}/{}", diagnostic.field),
                Vec::new(),
                vec![PlanningStructureSpan {
                    field: diagnostic.field.clone(),
                    line,
                }],
                vec!["该未知 key 承载的业务内容（先迁移到合法字段，随 author 新 revision 处置）".to_string()],
                vec![
                    "平台代为删除业务承载行（O1：平台不替 author 改写业务内容）".to_string(),
                    "删除未知 section/非 key-value 行/合法字段（共用 code，不误删）".to_string(),
                    "丢弃该行承载的业务供需/AC 内容".to_string(),
                ],
                StructuralRepairSolution::Candidates {
                    candidates: vec![StructuralRepairCandidate {
                        title: format!(
                            "保留内容、迁往合法字段：key「{}」在 STRUCTURED_KEYS 白名单外",
                            diagnostic.field
                        ),
                        constraints: vec![
                            "该行承载的业务正文不得消失（先迁移后处置）".to_string(),
                            "迁移目标必须是本 section 的合法字段".to_string(),
                        ],
                        evidence: vec![
                            format!("行{line} 原文={raw_line}"),
                            format!("key={}", diagnostic.field),
                        ],
                        verified_predicates: vec![format!(
                            "unknown_structured_key_cleared:行{line}"
                        )],
                    }],
                },
                format!(
                    "保留业务内容、迁往合法字段：行 {line} 的 key「{}」在 STRUCTURED_KEYS 白名单外，该行原文为「{raw_line}」。先将其承载的业务正文迁移到合法字段，再随新 revision 移除该自创 key 行——平台不替 author 删业务承载行。",
                    diagnostic.field
                ),
                vec![format!("unknown_structured_key_cleared:行{line}")],
                Vec::new(),
            ));
        }
    }
    instructions
}

/// 重复 trusted command 指令（O1 收窄）：不本地删 command/整 check——
/// producer 指出重复及受影响 check/ref（manual 与无 manual 两支处置均
/// 交 author；fail-closed：仅当全部诊断均为该结构身份且逐条可映射）。
fn duplicate_trusted_command_instructions(
    source: &str,
    diagnostics: &[CompilerDiagnostic],
    context: &WorkItemPlanSourceContext,
) -> Vec<StructuralRepairInstruction> {
    if diagnostics.is_empty()
        || !diagnostics
            .iter()
            .all(is_duplicate_trusted_command_diagnostic)
    {
        return Vec::new();
    }
    let Ok(ast) = parse_work_item_plan(source) else {
        return Vec::new();
    };
    let identity = InstructionIdentity::for_compile_failure(source, context);
    let mut instructions = Vec::new();
    for diagnostic in diagnostics {
        let Some(repair) = duplicate_trusted_command_repair(&ast, source, diagnostic.line) else {
            return Vec::new();
        };
        instructions.push(build_local_instruction(
            &identity,
            format!(
                "duplicate_trusted_command/{}/{}",
                repair.work_item_id, repair.check_id
            ),
            vec![repair.work_item_id.clone()],
            vec![PlanningStructureSpan {
                field: format!("verification.{}.command", repair.check_id),
                line: repair.command_line,
            }],
            vec![format!(
                "{}/{} Verification check 的重复 command（两支处置均由 author 决定）",
                repair.work_item_id, repair.check_id
            )],
            vec![
                "平台代为删除 command 或整 check（O1：平台不替 author 删验收证据）".to_string(),
                "删除首处（非重复）trusted command".to_string(),
                "删除仍被引用的 manual_instruction 业务说明".to_string(),
            ],
            StructuralRepairSolution::Candidates {
                candidates: vec![StructuralRepairCandidate {
                    title: format!(
                        "{}/{} 存在重复 trusted command：回查验证承诺后去重",
                        repair.work_item_id, repair.check_id
                    ),
                    constraints: vec![
                        "验证语义与验收证据不得无故消失".to_string(),
                        "首处（非重复）trusted command 不得删除".to_string(),
                    ],
                    evidence: vec![
                        format!("重复 command 行={}", repair.command_line),
                        format!(
                            "受影响 check={}/{}（行{}—行{}）",
                            repair.work_item_id,
                            repair.check_id,
                            repair.check_start_line,
                            repair.check_end_line
                        ),
                        format!(
                            "manual_instruction={}",
                            if repair.has_manual_instruction {
                                "非空（保留分支适用）"
                            } else {
                                "缺席（整块处置分支适用）"
                            }
                        ),
                    ],
                    verified_predicates: vec![format!(
                        "duplicate_trusted_command_cleared:{}/{}",
                        repair.work_item_id, repair.check_id
                    )],
                }],
            },
            format!(
                "回查重复 trusted command：{}/{} 在行 {} 存在重复 command，受影响 check 行区间 {}—{}。两支处置均由 author 决定：check 携带非空 manual_instruction 时移除重复 command 行（保留 manual_instruction 与既有验证语义）；无独立人工说明时由 author 决定整块删除该重复 check 或补齐人工说明——平台不替 author 删 command/删整 check。",
                repair.work_item_id,
                repair.check_id,
                repair.command_line,
                repair.check_start_line,
                repair.check_end_line
            ),
            vec![format!(
                "duplicate_trusted_command_cleared:{}/{}",
                repair.work_item_id, repair.check_id
            )],
            Vec::new(),
        ));
    }
    instructions
}

struct DuplicateTrustedCommandRepair {
    work_item_id: String,
    check_id: String,
    command_line: usize,
    check_start_line: usize,
    check_end_line: usize,
    has_manual_instruction: bool,
}

/// 未知结构化 key 的 parse 阶段诊断码（与 `parse.rs::UNKNOWN_STRUCTURED_KEY_CODE`
/// 同源：`grammar::DIAGNOSTIC_CODES[1]`）。
const UNKNOWN_STRUCTURED_KEY_CODE: &str =
    crate::product::work_item_plan_compiler::grammar::DIAGNOSTIC_CODES[1];

/// 单条 `unknown_structured_key` 诊断能否精确映射到「该 key 自己的源行」。
/// F1.2：携带 typed kind 的诊断（如非法二级 heading）先按类型整体排除；
/// 其余仅当诊断行号在界内且该行 strip 后为 `- <key>:` 或 `<key>:` 形态时
/// 返回该行号；行号错位/越界、或 field 与行内容不符（未知 section、非
/// key/value 行等）返回 None。
fn unknown_structured_key_line(source: &str, diagnostic: &CompilerDiagnostic) -> Option<usize> {
    // F1.2（REQ-SG-02 第414行场景）：指令资格切 compiler typed——
    // 携带 typed kind 的诊断（如非法二级 heading，携 source 原行/field/
    // example）不是可处置的未知 key 行，按类型整体排除，绝不按共享
    // unknown_structured_key 码处置；未 typed 诊断仍走逐行精确映射。
    if diagnostic.kind.is_some() {
        return None;
    }
    let key = diagnostic.field.trim();
    if key.is_empty() {
        return None;
    }
    let line = source.lines().nth(diagnostic.line.checked_sub(1)?)?;
    let stripped = line.trim();
    (stripped.starts_with(&format!("- {key}:")) || stripped.starts_with(&format!("{key}:")))
        .then_some(diagnostic.line)
}

fn is_duplicate_trusted_command_diagnostic(diagnostic: &CompilerDiagnostic) -> bool {
    // F1.2（REQ-SG-01/02A）：资格切 compiler typed——只认 compiler 源分支
    // 的结构身份（code+field）且诊断未携带 typed kind；不再从自由文本
    // message 匹配资格（不从 message 分类）。携带 typed kind 的诊断一律
    // 排除指令资格，交 typed 指令通道处理。
    diagnostic.kind.is_none()
        && diagnostic.code == "lowering_error"
        && diagnostic.field == "trusted_commands"
}

fn duplicate_trusted_command_repair(
    ast: &crate::product::work_item_plan_compiler::WorkItemPlanAst,
    source: &str,
    command_line: usize,
) -> Option<DuplicateTrustedCommandRepair> {
    for item in &ast.items {
        let Some(verification) = item
            .sections
            .iter()
            .find(|section| section.name.value == "Verification")
        else {
            continue;
        };
        let Some(command_index) = verification
            .fields
            .iter()
            .position(|field| field.key.value == "command" && field.value.line == command_line)
        else {
            continue;
        };
        let check_start_index = verification.fields[..=command_index]
            .iter()
            .rposition(|field| field.key.value == "check_id")?;
        let next_check_index = verification.fields[command_index + 1..]
            .iter()
            .position(|field| field.key.value == "check_id")
            .map(|index| command_index + 1 + index);
        let check_end_line = next_check_index
            .map(|index| verification.fields[index].value.line.saturating_sub(1))
            .unwrap_or_else(|| {
                verification_section_end_line(source, command_line).saturating_sub(1)
            });
        let check_fields = &verification.fields
            [check_start_index..next_check_index.unwrap_or(verification.fields.len())];
        let check_id = verification.fields[check_start_index].value.value.clone();
        let has_manual_instruction = check_fields.iter().any(|field| {
            field.key.value == "manual_instruction" && !is_explicit_none(&field.value.value)
        });
        return Some(DuplicateTrustedCommandRepair {
            work_item_id: item.id.value.clone(),
            check_id,
            command_line,
            check_start_line: verification.fields[check_start_index].value.line,
            check_end_line,
            has_manual_instruction,
        });
    }
    None
}

fn verification_section_end_line(source: &str, command_line: usize) -> usize {
    source
        .lines()
        .enumerate()
        .skip(command_line)
        .find_map(|(index, line)| {
            (line.starts_with("### ") || line.starts_with("## ")).then_some(index + 1)
        })
        .unwrap_or_else(|| source.lines().count())
}

fn is_explicit_none(value: &str) -> bool {
    let value = value.trim();
    value.is_empty() || value.eq_ignore_ascii_case("null") || value.eq_ignore_ascii_case("none")
}

struct CapabilityGap {
    provider: String,
    contract_id: String,
    capability: String,
}

struct HandoffGap {
    provider: String,
    contract_ref: String,
}

/// provider 对应 Outputs 契约块的 capabilities 锚点（O1 收窄：仅定位，
/// 不携带待改写原值）。
enum OutputsAnchor {
    /// 契约块最后一条 `- capabilities:` 行（行号；`lower_outputs` 末行生效）。
    MergeInto { line: usize },
    /// 契约块仅有 `- contract_id:` 行（其后为 capabilities 声明落点）。
    InsertAfter { line: usize },
    /// provider 或契约块不存在（required_contract_missing 族，不属指令域）。
    NotFound,
}

fn outputs_capabilities_anchor(
    ast: &crate::product::work_item_plan_compiler::WorkItemPlanAst,
    provider: &str,
    contract_id: &str,
) -> OutputsAnchor {
    let Some(item) = ast_item_for_logical_id(ast, provider) else {
        return OutputsAnchor::NotFound;
    };
    let Some(outputs) = item
        .sections
        .iter()
        .find(|section| section.name.value == "Outputs")
    else {
        return OutputsAnchor::NotFound;
    };
    let mut contract_line = None;
    let mut last_capabilities_line = None;
    for field in &outputs.fields {
        match field.key.value.as_str() {
            "contract_id" => {
                if contract_line.is_some() {
                    // 目标契约块已结束。
                    break;
                }
                if field.value.value == contract_id {
                    contract_line = Some(field.value.line);
                }
            }
            "capabilities" if contract_line.is_some() => {
                // 块内多行 capabilities：`lower_outputs` 末行生效，
                // 持续覆盖以锚定末行。
                last_capabilities_line = Some(field.value.line);
            }
            _ => {}
        }
    }
    match (contract_line, last_capabilities_line) {
        (Some(_), Some(cap_line)) => OutputsAnchor::MergeInto { line: cap_line },
        (Some(line), None) => OutputsAnchor::InsertAfter { line },
        (None, _) => OutputsAnchor::NotFound,
    }
}
/// provider `### Handoff Schema` 中全部 provided_contract_refs 行（行号+原值）。
fn handoff_provided_ref_fields(
    ast: &crate::product::work_item_plan_compiler::WorkItemPlanAst,
    provider: &str,
) -> Vec<(usize, String)> {
    let Some(item) = ast_item_for_logical_id(ast, provider) else {
        return Vec::new();
    };
    item.sections
        .iter()
        .find(|section| section.name.value == "Handoff Schema")
        .into_iter()
        .flat_map(|section| section.fields.iter())
        .filter(|field| field.key.value == "provided_contract_refs")
        .map(|field| (field.value.line, field.value.value.clone()))
        .collect()
}
fn ast_item_for_logical_id<'a>(
    ast: &'a crate::product::work_item_plan_compiler::WorkItemPlanAst,
    logical_id: &str,
) -> Option<&'a crate::product::work_item_plan_compiler::WorkItemPlanItemAst> {
    ast.items.iter().find(|item| {
        item.sections
            .iter()
            .find(|section| section.name.value == "Identity")
            .into_iter()
            .flat_map(|section| section.fields.iter())
            .any(|field| {
                field.key.value == "logical_work_item_id" && field.value.value == logical_id
            })
    })
}

/// 与 `lower.rs::split_value` 同口径拆分（逗号分隔、trim、滤空与 `[]`）。
fn split_ref_parts(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty() && *part != "[]")
        .map(str::to_string)
        .collect()
}

/// 反查 (consumer, contract_id) 所在依赖边的 provider（与
/// `contract_prerevision::required_action_text` 同一查询，保证指令指认的
/// provider 与返修文本一致）。
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
