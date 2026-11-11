//! 确定性契约缺口的机器指令 producer 与本地收敛器（DEF-PVR-ALL 主方案，
//! SG2 内容直改迁机器指令）。
//!
//! 门重测根因（codex rep1/rep2 全败、repeated_fingerprint 轮转不收敛）：
//! `required_capability_missing` 的 required_action 本就是「追加逐字给定行」
//! 的纯机械操作，但模型返修重写整份计划而不逐字应用补丁行，同族缺口
//! 跨指纹轮转。SG2（REQ-SG-02A、设计 §3.1/§11）把全部内容直改决定迁为
//! `StructuralRepairInstruction`（精确行操作，不声明唯一）：
//!
//! - `required_capability_missing`：缺失 capability 逐字并入 provider 对应
//!   Outputs 契约块的**最后一条** `- capabilities:` 行（`lower_outputs` 对
//!   同一契约多条 capabilities 行是「最后一条生效」语义；`split_value`
//!   逗号拆分语义保证旧行片段全保留+新片段加入）；契约块无 capabilities
//!   行时在该 `- contract_id:` 行后插入新行。
//! - `unconsumed_required_handoff`：从 provider `### Handoff Schema` 的
//!   `- provided_contract_refs:` 值中删除未消费引用（`split_values` 跨行
//!   聚合语义，逐行删值；剩余引用重接，空则写合法空值 `[]`）。
//! - `unknown_structured_key`：删除该诊断指向的未知结构化 key 源行（仅当
//!   该行 strip 后为 `- <key>:` 或 `<key>:` 形态才删；未知 section/非
//!   key-value 行共用同一 code，保持失败关闭。F1.2：携带 typed kind 的
//!   诊断（非法二级 heading，REQ-SG-02 第414行场景）按类型排除，绝不按
//!   共享码删 heading 行；重复 trusted command 的资格同样只认结构身份）。
//!
//! producer 是纯函数（零 source mutation、零 provider/模型调用）；本地
//! 收敛循环只机械应用指令的精确行编辑并重编译（source/IR/report 新鲜
//! 一致，`verify_publish_freshness` 链路零变化），零 verdict、零预算、
//! 零指纹——8 轮本地循环不触发 AI 乘法。残余 Error（`required_contract_
//! missing`/`unknown_provider_logical_work_item`/环/重复等非机械类）不拦
//! 截，照常产生机械 verdict 走 F5-A 模型返修。author 执行域接线（heading/
//! 回归/候选类指令驱原授权 author）由 SG5/SG6a 落地；本模块不发布新自动
//! 能力、不扩大已确认产物的修改权。

use std::collections::{BTreeMap, BTreeSet};

use crate::product::work_item_contract::{
    ContractFindingSeverity, DependencyContractGraph, build_dependency_contract_graph,
    validate_dependency_contract_graph,
};
use crate::product::work_item_plan_compiler::{
    CompilerDiagnostic, PlanCandidateIr, WorkItemPlanSourceContext, compile_work_item_plan,
    parse_work_item_plan,
};
use crate::product::workspace_engine::contract_prerevision::{
    ProtectedBusinessFacts, StructuralRepairInstruction, StructuralRepairOperation,
    StructuralRepairSolution, capability_patch_action_text, seal_structural_repair_instruction,
};
use crate::product::workspace_engine::draft_batch::compile_support::PlanningStructureSpan;
use sha2::{Digest, Sha256};

/// 收敛上限。每轮补齐严格消除至少一个机械缺口（provider capability 集合
/// 单调增长、未消费引用单调减少），实测 1 轮收敛；上限仅防御程序性退化，
/// 触顶时残余 findings 照常走模型返修。本地确定性循环：零 provider/模型
/// 调用（设计 §11「8 轮本地循环不触发 AI 乘法」）。
const MAX_CONTRACT_AUTOREPAIR_ROUNDS: usize = 8;

/// SG2 规则域版本：本模块确定性发现规则的稳定标识（指令身份绑定）。
const CONTRACT_AUTOREPAIR_RULE_VERSION: &str = "contract_autorepair/v1";

/// 收敛循环（SG2 指令驱动）：compile → producer 生成机器指令 → 机械应用
/// 指令精确行编辑 → 重编译，直到无可机械修复缺口或达上限。首编译的
/// lowering/parse 错误仅在**所有**诊断均可由行号精确映射的机械缺口（未知
/// 结构化 key 行删除 / 重复 trusted command）时修复；其余诊断保持原样
/// 失败关闭。每轮补齐后的编译失败同样上抛——补丁行不合法属程序缺陷，
/// 绝不静默回退掩盖。返回最终 IR、最终 source 与补齐日志。
pub(crate) fn converge_work_item_plan_source(
    source: &str,
    context: WorkItemPlanSourceContext,
) -> Result<(PlanCandidateIr, String, Vec<String>), Vec<CompilerDiagnostic>> {
    let mut source = source.to_string();
    let mut applied = Vec::new();
    for _ in 0..MAX_CONTRACT_AUTOREPAIR_ROUNDS {
        match compile_work_item_plan(&source, &context) {
            Ok(ir) => {
                let instructions = build_contract_autorepair_instructions(&source, &ir, &context);
                let Some((repaired, log)) =
                    apply_structural_repair_operations(&source, &instructions)
                else {
                    return Ok((ir, source, applied));
                };
                source = repaired;
                applied.extend(log);
            }
            Err(diagnostics) => {
                let instructions =
                    build_compile_failure_repair_instructions(&source, &diagnostics, &context);
                let Some((repaired, log)) =
                    apply_structural_repair_operations(&source, &instructions)
                else {
                    return Err(diagnostics);
                };
                source = repaired;
                applied.extend(log);
            }
        }
    }
    compile_work_item_plan(&source, &context).map(|ir| (ir, source, applied))
}

/// SG2：compile 成功路径的机器指令 producer——对 IR canonical 契约校验的
/// 两类机械缺口（required_capability_missing / unconsumed_required_handoff）
/// 生成精确行编辑指令。纯函数：零 source mutation、零 provider/模型调用；
/// `uniqueness` 恒 None（REQ-PLN-10：内容补齐不声明唯一解）。required_action
/// 复用 `contract_prerevision::required_action_text` 同一 F5-A 模板口径
/// （单一事实源，不给第二套语义）。
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
    // Err（重复 identity 等）不含两类机械缺口，交模型返修。
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
    // 同一行编辑（BTreeMap 键序确定，指令与日志均确定）。
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
        match outputs_capabilities_anchor(&ast, &provider, &contract_id) {
            OutputsAnchor::MergeInto { line, value } => {
                let mut merged = value.clone();
                for capability in &capabilities {
                    if !merged.is_empty() {
                        merged.push_str(", ");
                    }
                    merged.push_str(capability);
                }
                instructions.push(capability_instruction(
                    &identity,
                    &provider,
                    &contract_id,
                    &capabilities,
                    StructuralRepairOperation::ReplaceLine {
                        line,
                        before: format!("- capabilities: {value}"),
                        after: format!("- capabilities: {merged}"),
                    },
                    line,
                ));
            }
            OutputsAnchor::InsertAfter { line } => {
                instructions.push(capability_instruction(
                    &identity,
                    &provider,
                    &contract_id,
                    &capabilities,
                    StructuralRepairOperation::InsertLines {
                        after_line: line,
                        lines: vec![format!(
                            "- capabilities: {}",
                            capabilities.iter().cloned().collect::<Vec<_>>().join(", ")
                        )],
                    },
                    line,
                ));
            }
            OutputsAnchor::NotFound => continue,
        }
    }

    // 未消费 handoff 引用按 provider 聚合，逐行生成重写指令。
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
            let remaining = parts
                .iter()
                .filter(|part| !contract_refs.contains(part))
                .cloned()
                .collect::<Vec<_>>();
            if remaining.len() == parts.len() {
                continue;
            }
            instructions.push(handoff_instruction(
                &identity,
                &provider,
                &contract_refs,
                StructuralRepairOperation::ReplaceLine {
                    line,
                    before: format!("- provided_contract_refs: {value}"),
                    after: format!(
                        "- provided_contract_refs: {}",
                        if remaining.is_empty() {
                            "[]".to_string()
                        } else {
                            remaining.join(", ")
                        }
                    ),
                },
                line,
            ));
        }
    }
    instructions
}

/// SG2：compile 失败路径的机器指令 producer——只消费明确映射到「自身源行」
/// 的未知结构化 key 诊断与重复 trusted command 诊断，生成精确删行指令；
/// 携带 typed kind 的诊断（非法二级 heading 等）永不进入删行分支（REQ-SG-02
/// 第 414 行场景：不按共享 unknown key 码删 heading 行），任一诊断不可精确
/// 映射则该路径整体 fail-closed（空指令）。纯函数：零 source mutation、
/// 零模型调用。
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

/// 本地指令公共组装（producer 域内事实组装 + 摘要封口）。
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

/// capability 缺口指令：逐字行合并/插入（旧「末行生效」补齐语义迁指令）。
fn capability_instruction(
    identity: &InstructionIdentity,
    provider: &str,
    contract_id: &str,
    capabilities: &std::collections::BTreeSet<String>,
    operation: StructuralRepairOperation,
    anchor_line: usize,
) -> StructuralRepairInstruction {
    let patch_lines: Vec<String> = capabilities
        .iter()
        .map(|capability| capability_patch_action_text(provider, contract_id, capability))
        .collect();
    let expected_predicates = capabilities
        .iter()
        .map(|capability| format!("capability_satisfied:{contract_id}::{capability}"))
        .collect();
    build_local_instruction(
        identity,
        format!("required_capability_missing/{provider}/{contract_id}"),
        vec![provider.to_string()],
        vec![PlanningStructureSpan {
            field: format!("outputs.{contract_id}.capabilities"),
            line: anchor_line,
        }],
        vec![format!(
            "{provider} Outputs 契约 {contract_id} 的 capabilities 列表（仅追加，不删既有条目）"
        )],
        vec![
            "删除消费方的 required_capabilities 需求（假修复）".to_string(),
            "删除 provider 既有 capabilities".to_string(),
        ],
        StructuralRepairSolution::Precise {
            operations: vec![operation],
            uniqueness: None,
        },
        patch_lines.join("\n"),
        expected_predicates,
        vec![contract_id.to_string()],
    )
}

/// 未消费 handoff 引用指令：provided_contract_refs 逐行重写。
fn handoff_instruction(
    identity: &InstructionIdentity,
    provider: &str,
    contract_refs: &[String],
    operation: StructuralRepairOperation,
    anchor_line: usize,
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
            "{provider} ### Handoff Schema 的 provided_contract_refs 值列表（仅移除未消费引用）"
        )],
        vec![
            "移除仍被消费的 handoff 引用".to_string(),
            "删除消费方对仍需契约的真实需求".to_string(),
        ],
        StructuralRepairSolution::Precise {
            operations: vec![operation],
            uniqueness: None,
        },
        format!(
            "从 {provider} 的 ### Handoff Schema 的 provided_contract_refs 中移除未消费引用 {}（保留仍被消费的引用；全部未消费时写合法空值 []）；或补齐下游消费方。",
            contract_refs.join("、")
        ),
        vec![format!("no_unconsumed_required_handoff:{provider}")],
        contract_refs.to_vec(),
    )
}

/// 未知结构化 key 行删除指令（fail-closed：任一不可映射即整批放弃）。
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
            // 整体 fail-closed，不产生任何删行指令。
            return Vec::new();
        };
        if seen_lines.insert(line) {
            instructions.push(build_local_instruction(
                &identity,
                format!("unknown_structured_key/行{line}/{}", diagnostic.field),
                Vec::new(),
                vec![PlanningStructureSpan {
                    field: diagnostic.field.clone(),
                    line,
                }],
                vec!["该未知 key 自身源行的删除（业务内容先迁移合法字段）".to_string()],
                vec![
                    "删除未知 section/非 key-value 行/合法字段（共用 code，不误删）".to_string(),
                    "删除承载业务供需/AC 的合法内容".to_string(),
                ],
                StructuralRepairSolution::Precise {
                    operations: vec![StructuralRepairOperation::DeleteLines {
                        lines: vec![line],
                        reason: format!("删除未知结构化 key 行 {line}：{}", diagnostic.field),
                    }],
                    uniqueness: None,
                },
                format!(
                    "删除未知结构化 key 行 {line}（key：{}）——该行是 STRUCTURED_KEYS 白名单外的自创 key 行；若该行承载业务内容，先迁移到合法字段再删除该 key 行。",
                    diagnostic.field
                ),
                vec![format!("unknown_structured_key_cleared:行{line}")],
                Vec::new(),
            ));
        }
    }
    instructions
}

/// 重复 trusted command 删除指令（fail-closed：仅当全部诊断均为该结构
/// 身份且逐条可映射）。
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
        // 含非空 manual instruction 的重复 check 仅删 command 行；无独立
        // 人工说明的 check 整块删除，避免留下既无 command 又无手工步骤的
        // 非法 check。
        let (lines, reason) = if repair.has_manual_instruction {
            (
                vec![repair.command_line],
                format!(
                    "{}/{} 移除重复 trusted command（保留 manual_instruction）",
                    repair.work_item_id, repair.check_id
                ),
            )
        } else {
            (
                (repair.check_start_line..=repair.check_end_line).collect::<Vec<_>>(),
                format!(
                    "{}/{} 删除仅含重复 trusted command 的 Verification check",
                    repair.work_item_id, repair.check_id
                ),
            )
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
                "{}/{} Verification check 的重复 command（manual_instruction 保留）",
                repair.work_item_id, repair.check_id
            )],
            vec![
                "删除首处（非重复）trusted command".to_string(),
                "删除仍被引用的 manual_instruction 业务说明".to_string(),
            ],
            StructuralRepairSolution::Precise {
                operations: vec![StructuralRepairOperation::DeleteLines {
                    lines: lines.clone(),
                    reason,
                }],
                uniqueness: None,
            },
            format!(
                "{}/{} 存在重复 trusted command：移除重复 command 行（保留 manual_instruction 与既有验证语义）；无独立人工说明的 check 整块删除。",
                repair.work_item_id, repair.check_id
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

/// 机械应用指令的精确行编辑操作（本地收敛唯一 mutation 点）：
/// `ReplaceLine`/`InsertLines`/`DeleteLines`；author 执行域操作（模板重写/
/// 历史恢复）不经本地应用（SG5/SG6a 接线）。无可应用操作返回 None。
fn apply_structural_repair_operations(
    source: &str,
    instructions: &[StructuralRepairInstruction],
) -> Option<(String, Vec<String>)> {
    let mut replacements = BTreeMap::<usize, String>::new();
    let mut inserts = BTreeMap::<usize, Vec<String>>::new();
    let mut deletions = BTreeSet::new();
    let mut applied = Vec::new();
    for instruction in instructions {
        let StructuralRepairSolution::Precise { operations, .. } = &instruction.solution else {
            continue;
        };
        for operation in operations {
            match operation {
                StructuralRepairOperation::ReplaceLine {
                    line,
                    before,
                    after,
                } => {
                    replacements.insert(*line, after.clone());
                    applied.push(format!("行 {line} 替换：{before} → {after}"));
                }
                StructuralRepairOperation::InsertLines { after_line, lines } => {
                    inserts.insert(*after_line, lines.clone());
                    applied.push(format!("行 {after_line} 后插入：{}", lines.join("；")));
                }
                StructuralRepairOperation::DeleteLines { lines, reason } => {
                    deletions.extend(lines.iter().copied());
                    applied.push(reason.clone());
                }
                // author 执行域操作（heading 模板/历史恢复）不经本地应用。
                StructuralRepairOperation::RewriteWithGrammarTemplate { .. }
                | StructuralRepairOperation::RestoreResolvedFact { .. } => {}
            }
        }
    }
    if applied.is_empty() {
        return None;
    }
    Some((
        rewrite_source(source, &replacements, &inserts, &deletions),
        applied,
    ))
}

/// 按行号应用替换/插入/删除（前向单遍重建，行号互不位移；保留原 source
/// 收尾换行形态）。
fn rewrite_source(
    source: &str,
    replacements: &BTreeMap<usize, String>,
    inserts: &BTreeMap<usize, Vec<String>>,
    deletions: &BTreeSet<usize>,
) -> String {
    let had_trailing_newline = source.ends_with('\n');
    let mut lines = Vec::with_capacity(source.lines().count() + inserts.len());
    for (index, line) in source.lines().enumerate() {
        let line_no = index + 1;
        if !deletions.contains(&line_no) {
            match replacements.get(&line_no) {
                Some(replacement) => lines.push(replacement.clone()),
                None => lines.push(line.to_string()),
            }
        }
        if let Some(inserted) = inserts.get(&line_no) {
            lines.extend(inserted.iter().cloned());
        }
    }
    let mut rewritten = lines.join("\n");
    if had_trailing_newline {
        rewritten.push('\n');
    }
    rewritten
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
    // F1.2（REQ-SG-02 第414行场景）：机械删除资格切 compiler typed——
    // 携带 typed kind 的诊断（如非法二级 heading，携 source 原行/field/
    // example）不是可删除的未知 key 行，按类型整体排除，绝不按共享
    // unknown_structured_key 码删行；未 typed 诊断仍走逐行精确映射。
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
    // 排除机械删除，交 typed 指令通道处理。
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

/// provider 对应 Outputs 契约块的 capabilities 锚点。
enum OutputsAnchor {
    /// 契约块最后一条 `- capabilities:` 行（行号 + 原值）。
    MergeInto { line: usize, value: String },
    /// 契约块仅有 `- contract_id:` 行（其后插入新 capabilities 行）。
    InsertAfter { line: usize },
    /// provider 或契约块不存在（required_contract_missing 族，不属机械补齐）。
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
    let mut last_capabilities = None;
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
                last_capabilities = Some((field.value.line, field.value.value.clone()));
            }
            _ => {}
        }
    }
    match (contract_line, last_capabilities) {
        (Some(_line), Some((cap_line, value))) => OutputsAnchor::MergeInto {
            line: cap_line,
            value,
        },
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
/// `contract_prerevision::required_action_text` 同一查询，保证补齐落点与
/// 返修指令指认的 provider 一致）。
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

#[cfg(test)]
mod tests {
    use super::*;

    const REP4_FIXTURE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/product/work_item_plan_compiler/fixtures/work-item-plan-rep4.md"
    ));

    fn compile(source: &str) -> PlanCandidateIr {
        compile_work_item_plan(
            source,
            &WorkItemPlanSourceContext {
                target_repository_id: "repo_fixture".to_string(),
                member_targets: None,
            },
        )
        .expect("fixture compiles")
    }

    fn clean_candidate() -> String {
        REP4_FIXTURE.replace(
            "- provided_contract_refs: contract.levels-integration",
            "- provided_contract_refs: []",
        )
    }

    fn capability_gap_candidate() -> String {
        clean_candidate().replace(
            "- required_capabilities: api.levels.read\n",
            "- required_capabilities: api.levels.read, api.levels.write\n",
        )
    }

    /// 指令精确操作视图（SG2：旧 patch 断言迁指令断言）。
    fn precise_operations(
        instruction: &crate::product::workspace_engine::contract_prerevision::StructuralRepairInstruction,
    ) -> &[StructuralRepairOperation] {
        let crate::product::workspace_engine::contract_prerevision::StructuralRepairSolution::Precise { operations, .. } =
            &instruction.solution
        else {
            panic!("expected precise operations, got {:?}", instruction.solution);
        };
        operations
    }

    fn assert_validates_clean(source: &str) {
        let repaired_ir = compile(source);
        let contracts = repaired_ir
            .items
            .iter()
            .map(|item| item.contract.clone())
            .collect::<Vec<_>>();
        let graph = build_dependency_contract_graph(&contracts).expect("graph");
        assert!(
            validate_dependency_contract_graph(&graph).is_valid(),
            "source must validate clean:\n{source}"
        );
    }

    #[test]
    fn merges_missing_capability_into_provider_last_capabilities_line() {
        let source = capability_gap_candidate();
        let instructions =
            build_contract_autorepair_instructions(&source, &compile(&source), &source_context());
        assert_eq!(
            instructions.len(),
            1,
            "capability gap must produce one instruction"
        );
        let operations = precise_operations(&instructions[0]);
        let StructuralRepairOperation::ReplaceLine {
            line,
            before,
            after,
        } = &operations[0]
        else {
            panic!("capability merge must be a precise line replacement");
        };
        assert_eq!(
            *line,
            source
                .lines()
                .position(|line| line == "- capabilities: api.levels.read")
                .expect("fixture carries the provider capabilities line")
                + 1
        );
        assert_eq!(before, "- capabilities: api.levels.read");
        assert_eq!(after, "- capabilities: api.levels.read, api.levels.write");
        // 本地应用指令后重编译，机械契约校验干净。
        let (_ir, repaired, applied) = converge_work_item_plan_source(&source, source_context())
            .expect("local convergence must apply the instruction");
        assert!(
            repaired.contains("- capabilities: api.levels.read, api.levels.write\n"),
            "repaired source must merge the capability verbatim:\n{repaired}"
        );
        assert_eq!(applied.len(), 1);
        assert_validates_clean(&repaired);
    }

    #[test]
    fn merge_targets_the_last_capabilities_line_of_the_contract_block() {
        // `lower_outputs` 同一契约多行 capabilities 为「最后一条生效」；
        // 指令必须并入末行而不是新增一行（新增行会整体顶掉既有能力）。
        let source = clean_candidate()
            .replace(
                "- capabilities: api.levels.read\n",
                "- capabilities: api.levels.stale\n- capabilities: api.levels.read\n",
            )
            .replace(
                "- required_capabilities: api.levels.read\n",
                "- required_capabilities: api.levels.read, api.levels.write\n",
            );
        let instructions =
            build_contract_autorepair_instructions(&source, &compile(&source), &source_context());
        let operations = precise_operations(&instructions[0]);
        let StructuralRepairOperation::ReplaceLine { line, .. } = &operations[0] else {
            panic!("merge must be a line replacement");
        };
        let last = source
            .lines()
            .enumerate()
            .filter(|(_, line)| *line == "- capabilities: api.levels.read")
            .map(|(index, _)| index + 1)
            .last()
            .expect("fixture carries two capabilities lines");
        assert_eq!(*line, last, "merge must target the live (last) line");
        let (_ir, repaired, _applied) = converge_work_item_plan_source(&source, source_context())
            .expect("converge must succeed");
        assert!(
            repaired.contains("- capabilities: api.levels.read, api.levels.write\n"),
            "merge must land on the live capabilities line:\n{repaired}"
        );
        assert!(
            !repaired.contains("api.levels.write, api.levels.write"),
            "repair must stay idempotent-free of duplicated appends"
        );
    }

    #[test]
    fn inserts_new_capabilities_line_when_contract_block_has_none() {
        // 目标契约块无 capabilities 行（section 级语法仍满足：前一个契约块
        // 带 capabilities 键），指令在该 `- contract_id:` 行后插入新行。
        let source = clean_candidate()
            .replace(
                "- contract_id: contract.levels-api\n- capabilities: api.levels.read\n",
                "- contract_id: contract.audit\n- capabilities: audit.evidence\n- contract_id: contract.levels-api\n",
            )
            .replace(
                "- required_capabilities: api.levels.read\n",
                "- required_capabilities: api.levels.read, api.levels.write\n",
            );
        let instructions =
            build_contract_autorepair_instructions(&source, &compile(&source), &source_context());
        let operations = precise_operations(&instructions[0]);
        let StructuralRepairOperation::InsertLines { after_line, lines } = &operations[0] else {
            panic!("capabilities-less block must get a line insert");
        };
        assert_eq!(
            *after_line,
            source
                .lines()
                .position(|line| line == "- contract_id: contract.levels-api")
                .expect("fixture carries the target contract block")
                + 1
        );
        assert_eq!(
            lines,
            &vec!["- capabilities: api.levels.read, api.levels.write".to_string()]
        );
        let (_ir, repaired, _applied) = converge_work_item_plan_source(&source, source_context())
            .expect("converge must succeed");
        assert!(
            repaired.contains(
                "- contract_id: contract.levels-api\n- capabilities: api.levels.read, api.levels.write\n"
            ),
            "a new capabilities line must follow the contract_id line:\n{repaired}"
        );
        assert_validates_clean(&repaired);
    }

    #[test]
    fn removes_unconsumed_handoff_ref_and_rewrites_to_legal_empty_list() {
        let source = REP4_FIXTURE;
        let instructions =
            build_contract_autorepair_instructions(source, &compile(source), &source_context());
        assert_eq!(
            instructions.len(),
            1,
            "unconsumed handoff must produce one instruction"
        );
        let operations = precise_operations(&instructions[0]);
        let StructuralRepairOperation::ReplaceLine { after, .. } = &operations[0] else {
            panic!("handoff removal must be a line replacement");
        };
        assert_eq!(after, "- provided_contract_refs: []");
        let (_ir, repaired, applied) = converge_work_item_plan_source(source, source_context())
            .expect("local convergence must apply the instruction");
        assert!(
            repaired.contains("- provided_contract_refs: []\n"),
            "sole ref removal must rewrite the legal empty value:\n{repaired}"
        );
        assert_eq!(applied.len(), 1);
        assert_validates_clean(&repaired);
    }

    #[test]
    fn keeps_sibling_refs_when_removing_one_from_a_multi_ref_line() {
        let source = clean_candidate().replace(
            "- provided_contract_refs: contract.levels-api\n",
            "- provided_contract_refs: contract.levels-api, contract.levels-extra\n",
        );
        // contract.levels-extra 未被消费 → 删它，保留仍被消费的 levels-api。
        let instructions =
            build_contract_autorepair_instructions(&source, &compile(&source), &source_context());
        let operations = precise_operations(&instructions[0]);
        let StructuralRepairOperation::ReplaceLine { after, .. } = &operations[0] else {
            panic!("multi-ref handoff gap must be a line replacement");
        };
        assert_eq!(after, "- provided_contract_refs: contract.levels-api");
        let (_ir, repaired, _applied) = converge_work_item_plan_source(&source, source_context())
            .expect("converge must succeed");
        assert!(
            repaired.contains("- provided_contract_refs: contract.levels-api\n"),
            "consumed sibling ref must survive the removal:\n{repaired}"
        );
        assert!(!repaired.contains("contract.levels-extra"));
    }

    #[test]
    fn clean_and_non_mechanical_sources_return_none() {
        assert!(
            build_contract_autorepair_instructions(
                &clean_candidate(),
                &compile(&clean_candidate()),
                &source_context()
            )
            .is_empty()
        );
        // 依赖环不在两类机械缺口内：不拦截。
        let cycled = clean_candidate().replace(
            "### Inputs\n\n### Outputs\n- contract_id: contract.levels-api",
            "### Inputs\n- contract_id: contract.level-selector\n- provider_logical_work_item_id: WI-002\n- required_capabilities: ui.level-selector.rendered\n- compatibility_policy: require_all\n\n### Outputs\n- contract_id: contract.levels-api",
        );
        let ir = compile(&cycled);
        let contracts = ir
            .items
            .iter()
            .map(|item| item.contract.clone())
            .collect::<Vec<_>>();
        let graph = build_dependency_contract_graph(&contracts).expect("graph");
        assert!(
            !validate_dependency_contract_graph(&graph).is_valid(),
            "cycle fixture must actually carry a non-mechanical gap"
        );
        assert!(build_contract_autorepair_instructions(&cycled, &ir, &source_context()).is_empty());
    }

    #[test]
    fn converge_loop_repairs_until_clean_and_reports_log() {
        let (ir, source, applied) =
            converge_work_item_plan_source(&capability_gap_candidate(), source_context())
                .expect("converge must succeed");
        assert!(
            source.contains("- capabilities: api.levels.read, api.levels.write\n"),
            "converged source must carry the repair"
        );
        assert_eq!(applied.len(), 1);
        let contracts = ir
            .items
            .iter()
            .map(|item| item.contract.clone())
            .collect::<Vec<_>>();
        let graph = build_dependency_contract_graph(&contracts).expect("graph");
        assert!(validate_dependency_contract_graph(&graph).is_valid());
    }
    #[test]
    fn converge_repairs_duplicate_trusted_command_without_losing_manual_instruction() {
        let source = clean_candidate().replacen(
            "- command: cargo test --locked --lib levels_api\n- manual_instruction: Confirm the endpoint returns the configured levels JSON.\n- required: true\n- non_zero_test_execution_required: true",
            "- command: cargo test --locked --lib levels_api\n- manual_instruction: Confirm the endpoint returns the configured levels JSON.\n- required: true\n- non_zero_test_execution_required: true\n- check_id: CHECK-004\n- command: cargo test --locked --lib levels_api\n- manual_instruction: Confirm the endpoint still returns configured levels JSON.\n- required: true\n- non_zero_test_execution_required: true",
            1,
        );
        let (ir, source, applied) = converge_work_item_plan_source(
            &source,
            WorkItemPlanSourceContext {
                target_repository_id: "repo_fixture".to_string(),
                member_targets: None,
            },
        )
        .expect("duplicate trusted command must be mechanically repaired");

        let verification = &ir.items[0].contract.verification_checks;
        assert_eq!(verification.len(), 2);
        assert_eq!(
            verification[0].command.as_deref(),
            Some("cargo test --locked --lib levels_api")
        );
        assert_eq!(verification[1].command, None);
        assert_eq!(
            source
                .matches("- command: cargo test --locked --lib levels_api")
                .count(),
            1
        );
        assert!(!source.contains("CHECK-004\n- command: cargo test --locked --lib levels_api"));
        assert!(source.contains(
            "- check_id: CHECK-004\n- manual_instruction: Confirm the endpoint still returns configured levels JSON."
        ));
        assert_eq!(applied.len(), 1);
        assert!(applied[0].contains("重复 trusted command"));
    }
    #[test]
    fn converge_removes_duplicate_check_without_manual_instruction() {
        let source = clean_candidate().replacen(
            "- command: cargo test --locked --lib levels_api\n- manual_instruction: Confirm the endpoint returns the configured levels JSON.\n- required: true\n- non_zero_test_execution_required: true",
            "- command: cargo test --locked --lib levels_api\n- manual_instruction: Confirm the endpoint returns the configured levels JSON.\n- required: true\n- non_zero_test_execution_required: true\n- check_id: CHECK-004\n- command: cargo test --locked --lib levels_api\n- required: true\n- non_zero_test_execution_required: true",
            1,
        );
        let (ir, source, applied) = converge_work_item_plan_source(
            &source,
            WorkItemPlanSourceContext {
                target_repository_id: "repo_fixture".to_string(),
                member_targets: None,
            },
        )
        .expect("duplicate check without manual instruction must be removed");

        assert_eq!(ir.items[0].contract.verification_checks.len(), 1);
        assert!(!source.contains("CHECK-004"));
        assert_eq!(applied.len(), 1);
        assert!(applied[0].contains("删除仅含重复 trusted command"));
    }

    /// 复验 F-41：修订轮 author 在 `### Handoff Schema` 自创 `requested_fields`
    /// （`STRUCTURED_KEYS` 白名单外）→ parse 阶段 `unknown_structured_key` 直接终态死。
    fn unknown_structured_key_candidate() -> String {
        clean_candidate().replacen(
            "- required_fields: commit_sha\n",
            "- required_fields: commit_sha\n- requested_fields: 无\n",
            1,
        )
    }

    fn source_context() -> WorkItemPlanSourceContext {
        WorkItemPlanSourceContext {
            target_repository_id: "repo_fixture".to_string(),
            member_targets: None,
        }
    }

    fn unknown_key_diagnostic(line: usize, field: &str) -> CompilerDiagnostic {
        CompilerDiagnostic {
            code: "unknown_structured_key".to_string(),
            line,
            field: field.to_string(),
            message: "未知结构化 key 必须拒绝。".to_string(),
            repair_example: "- kind: backend".to_string(),
            kind: None,
            lowering_fault: None,
            ir_fault: None,
        }
    }

    #[test]
    fn converge_removes_unknown_structured_key_line_and_recompiles_clean() {
        let source = unknown_structured_key_candidate();
        let diagnostics = compile_work_item_plan(&source, &source_context())
            .expect_err("unknown structured key must fail the first compile");
        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "unknown_structured_key"
                    && diagnostic.field == "requested_fields"
            }),
            "first compile must report the unknown key: {diagnostics:?}"
        );

        let (ir, source, applied) = converge_work_item_plan_source(&source, source_context())
            .expect("unknown structured key line must be mechanically repaired");
        assert!(!source.contains("requested_fields"));
        assert!(
            source.contains(
                "- required_fields: commit_sha\n- provided_contract_refs: contract.levels-api\n"
            ),
            "neighbouring legal fields must survive the deletion:\n{source}"
        );
        assert_eq!(
            source.lines().count(),
            unknown_structured_key_candidate().lines().count() - 1
        );
        // 返回的 IR 由删除后的 source 新鲜编译得到。
        assert_eq!(ir.items.len(), 3);
        assert_eq!(applied.len(), 1);
        assert!(
            applied[0].contains("未知结构化 key") && applied[0].contains("requested_fields"),
            "deterministic log entry: {}",
            applied[0]
        );
    }

    #[test]
    fn unknown_structured_key_repair_requires_the_line_to_carry_the_key() {
        let source = unknown_structured_key_candidate();
        // 行号错位（指向相邻的合法行）与行号越界均不产生删行指令。
        assert!(
            build_compile_failure_repair_instructions(
                &source,
                &[unknown_key_diagnostic(50, "requested_fields")],
                &source_context()
            )
            .is_empty()
        );
        assert!(
            build_compile_failure_repair_instructions(
                &source,
                &[unknown_key_diagnostic(
                    source.lines().count() + 1,
                    "requested_fields"
                )],
                &source_context()
            )
            .is_empty()
        );
        // 命中行时指令按诊断行号精确删除。
        let target = source
            .lines()
            .position(|line| line == "- requested_fields: 无")
            .expect("fixture carries the unknown key line")
            + 1;
        let instructions = build_compile_failure_repair_instructions(
            &source,
            &[unknown_key_diagnostic(target, "requested_fields")],
            &source_context(),
        );
        assert_eq!(instructions.len(), 1);
        let operations = precise_operations(&instructions[0]);
        assert_eq!(
            operations,
            &vec![StructuralRepairOperation::DeleteLines {
                lines: vec![target],
                reason: format!("删除未知结构化 key 行 {target}：requested_fields"),
            }]
        );
    }

    #[test]
    fn converge_keeps_same_code_non_key_lines_fail_closed() {
        // 未知 section 与非 key/value 行共用 `unknown_structured_key` code：
        // 诊断 field 与源行内容不符，绝不误删。
        let unknown_section = clean_candidate().replacen(
            "### Blockers\n",
            "### Unexpected Section\n### Blockers\n",
            1,
        );
        let malformed_line = clean_candidate().replacen(
            "- required_fields: commit_sha\n",
            "requested_fields 无\n- required_fields: commit_sha\n",
            1,
        );
        for source in [unknown_section, malformed_line] {
            let diagnostics = converge_work_item_plan_source(&source, source_context())
                .expect_err("non-key lines sharing the code must stay fail-closed");
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == "unknown_structured_key"),
                "{diagnostics:?}"
            );
        }
    }

    #[test]
    fn converge_keeps_non_duplicate_diagnostics_fail_closed() {
        let source = REP4_FIXTURE.replacen("- kind: backend\n", "", 1);
        let diagnostics = converge_work_item_plan_source(
            &source,
            WorkItemPlanSourceContext {
                target_repository_id: "repo_fixture".to_string(),
                member_targets: None,
            },
        )
        .expect_err("non-mechanical compiler diagnostics must remain errors");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.field == "kind" && diagnostic.code != "lowering_error"
        }));
        assert!(
            !diagnostics
                .iter()
                .any(is_duplicate_trusted_command_diagnostic)
        );
    }

    // —— F1.2（REQ-SG-01/02/02A）：机械删除资格切 compiler typed ——

    #[test]
    fn sg_f1_2_typed_heading_never_deleted_as_unknown_key() {
        use crate::product::planning_failure::PlanningFailureKind;

        // REQ-SG-02 第414行场景（R2 防线）：非法二级 heading 与未知 key
        // 共享 unknown_structured_key code；heading 分支携带 typed kind，
        // 机械删除资格按 typed payload 排除——绝不按共享码删 heading 行，
        // 混合集合同样整体 fail-closed。
        let source = unknown_structured_key_candidate().replacen(
            "### Blockers\n",
            "## 验收说明\n### Blockers\n",
            1,
        );
        let diagnostics = compile_work_item_plan(&source, &source_context())
            .expect_err("heading + unknown key 必须编译失败");
        let heading = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.field == "work_item_heading")
            .expect("heading 诊断必须存在");
        assert_eq!(heading.code, "unknown_structured_key");
        assert!(matches!(
            &heading.kind,
            Some(PlanningFailureKind::InvalidWorkItemHeading {
                line,
                raw_line,
                ..
            }) if *line >= 1 && raw_line == "## 验收说明"
        ));
        assert!(
            build_compile_failure_repair_instructions(&source, &diagnostics, &source_context())
                .is_empty(),
            "typed heading 排除机械删除资格，混合集合整体 fail-closed（零删行指令）"
        );
        assert!(
            converge_work_item_plan_source(&source, source_context()).is_err(),
            "heading 错误不经机械删除收敛"
        );
    }

    #[test]
    fn sg_f1_2_duplicate_command_qualification_is_structural_not_message() {
        use crate::product::planning_failure::PlanningFailureKind;

        // 资格切 compiler typed（F1.2）：重复 trusted command 的机械删除
        // 资格只认结构身份（code+field）且未携带 typed kind——message
        // 文案漂移不改变判定（REQ-SG-01：不从自由文本分类）。
        let drifted = CompilerDiagnostic {
            code: "lowering_error".to_string(),
            message: "文案漂移后的重复 trusted command 描述".to_string(),
            repair_example: "- command: cargo test --locked --lib target".to_string(),
            ..unknown_key_diagnostic(9, "trusted_commands")
        };
        assert!(is_duplicate_trusted_command_diagnostic(&drifted));
        let mut other_field = drifted.clone();
        other_field.field = "kind".to_string();
        assert!(!is_duplicate_trusted_command_diagnostic(&other_field));
        let mut typed = drifted.clone();
        typed.kind = Some(PlanningFailureKind::InvalidWorkItemHeading {
            line: 1,
            field: "work_item_heading".to_string(),
            raw_line: "## 验收说明".to_string(),
            example: "## Work Item WI-001: fixture".to_string(),
        });
        assert!(
            !is_duplicate_trusted_command_diagnostic(&typed),
            "携带 typed kind 的诊断不参与机械删除"
        );
    }

    // —— SG2（REQ-SG-02/02A、REQ-PLN-10）：内容直改迁机器指令 ——

    /// 内容 autorepair 迁机器指令：producer 是纯函数（零 source mutation、
    /// 零 provider/模型调用、无 8×AI 循环），输出为精确行编辑指令且不声明
    /// 唯一解；required_action 与 F5-A 机械 verdict 同一模板口径（单一事实
    /// 源，不给第二套语义）；本地收敛仍确定性应用指令并重编译干净。
    #[test]
    fn sg_02_no_fake_edge_removal_content_autorepair_migrated_to_pure_instructions() {
        use crate::product::workspace_engine::contract_prerevision::{
            StructuralRepairOperation, StructuralRepairSolution,
        };
        use sha2::{Digest, Sha256};

        let source = capability_gap_candidate();
        let context = source_context();
        let ir = compile(&source);
        let instructions = build_contract_autorepair_instructions(&source, &ir, &context);
        assert_eq!(instructions.len(), 1, "capability 缺口产一条精确补齐指令");
        let instruction = &instructions[0];

        // 纯 producer 的身份绑定与确定性（无隐藏状态/循环副作用）。
        assert_eq!(
            instruction.source_hash,
            hex::encode(Sha256::digest(source.as_bytes())),
            "指令 source_hash 绑定待修 source"
        );
        assert!(!instruction.instruction_digest.is_empty());
        assert_eq!(
            build_contract_autorepair_instructions(&source, &ir, &context),
            instructions,
            "同输入两次构建全等（确定性）"
        );
        assert_eq!(instruction.rule_version, CONTRACT_AUTOREPAIR_RULE_VERSION);

        // 精确操作为逐字行合并（旧「末行生效」补齐语义迁为指令载荷）。
        let StructuralRepairSolution::Precise {
            operations,
            uniqueness,
        } = &instruction.solution
        else {
            panic!("capability 补齐必须是精确操作形态");
        };
        assert!(uniqueness.is_none(), "内容补齐不携带唯一标签（REQ-PLN-10）");
        assert_eq!(
            operations,
            &vec![StructuralRepairOperation::ReplaceLine {
                line: source
                    .lines()
                    .position(|line| line == "- capabilities: api.levels.read")
                    .expect("fixture carries the provider capabilities line")
                    + 1,
                before: "- capabilities: api.levels.read".to_string(),
                after: "- capabilities: api.levels.read, api.levels.write".to_string(),
            }],
            "操作逐字指向 provider 的生效 capabilities 行"
        );

        // required_action 与 F5-A verdict 同一模板（单一口径）。
        assert!(
            instruction.required_action.contains(
                "在 WI-001 的 Outputs 契约 contract.levels-api 的 capabilities 列表追加一行（逐字复制）：- api.levels.write"
            ),
            "required_action 复用 F5-A 逐字补丁行模板：{}",
            instruction.required_action
        );

        // 本地收敛仍机械应用指令并重编译干净（8 轮本地循环零 AI 调用）。
        let (repaired_ir, repaired, applied) = converge_work_item_plan_source(&source, context)
            .expect("local convergence must still apply the instruction");
        assert!(
            repaired.contains("- capabilities: api.levels.read, api.levels.write\n"),
            "收敛后的 source 携带逐字补齐：\n{repaired}"
        );
        assert_eq!(applied.len(), 1);
        let contracts = repaired_ir
            .items
            .iter()
            .map(|item| item.contract.clone())
            .collect::<Vec<_>>();
        let graph = build_dependency_contract_graph(&contracts).expect("graph");
        assert!(
            validate_dependency_contract_graph(&graph).is_valid(),
            "收敛后机械契约校验必须干净"
        );
    }

    /// 未知 key 大码 heading 不删行（REQ-SG-02 第 414 行场景）：typed
    /// heading 诊断（非法二级 heading）与未知 key 共享 code，但携带 typed
    /// kind——指令 producer 永不为其生成删行指令；混合集合整体 fail-closed，
    /// 纯 heading 缺口同样零删行指令、不经本地收敛。
    #[test]
    fn sg_02_no_fake_edge_removal_unknown_key_big_code_heading_never_deleted() {
        use crate::product::planning_failure::PlanningFailureKind;

        let context = source_context();
        let mixed = unknown_structured_key_candidate().replacen(
            "### Blockers\n",
            "## 验收说明\n### Blockers\n",
            1,
        );
        let diagnostics = compile_work_item_plan(&mixed, &context)
            .expect_err("heading + unknown key 必须编译失败");
        let heading = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.field == "work_item_heading")
            .expect("heading 诊断必须存在");
        assert_eq!(heading.code, "unknown_structured_key");
        assert!(matches!(
            &heading.kind,
            Some(PlanningFailureKind::InvalidWorkItemHeading { line, raw_line, .. })
                if *line >= 1 && raw_line == "## 验收说明"
        ));
        assert!(
            build_compile_failure_repair_instructions(&mixed, &diagnostics, &context).is_empty(),
            "typed heading 与未知 key 混合集合整体 fail-closed：零删行指令（绝不按共享码删 heading 行）"
        );
        assert!(
            converge_work_item_plan_source(&mixed, context.clone()).is_err(),
            "heading 错误不经机械删行收敛"
        );

        // 纯 heading 缺口：同样零删行指令（不删业务行）。
        let heading_only =
            clean_candidate().replacen("### Blockers\n", "## 验收说明\n### Blockers\n", 1);
        let heading_diagnostics = compile_work_item_plan(&heading_only, &context)
            .expect_err("pure heading fixture must fail compile");
        assert!(
            build_compile_failure_repair_instructions(
                &heading_only,
                &heading_diagnostics,
                &context
            )
            .is_empty(),
            "typed heading 诊断不产生任何删行指令"
        );
    }
}
