//! DEF-PVR-ALL 确定性 capability 补齐器（3.6 全量收敛轮主方案）。
//!
//! 门重测根因（codex rep1/rep2 全败、repeated_fingerprint 轮转不收敛）：
//! `required_capability_missing` 的 required_action 本就是「追加逐字给定行」
//! 的纯机械操作，但模型返修重写整份计划而不逐字应用补丁行，同族缺口
//! 跨指纹轮转。本模块把这两类机械缺口在 author 落盘前确定性修复：
//!
//! - `required_capability_missing`：把缺失 capability 逐字并入 provider 对应
//!   Outputs 契约块的**最后一条** `- capabilities:` 行（`lower_outputs` 对
//!   同一契约多条 capabilities 行是「最后一条生效」语义，改写末行即补齐；
//!   `split_value` 逗号拆分语义保证旧行片段全保留+新片段加入）；契约块
//!   无 capabilities 行时在该 `- contract_id:` 行后插入新行。
//! - `unconsumed_required_handoff`：从 provider `### Handoff Schema` 的
//!   `- provided_contract_refs:` 值中删除未消费引用（`split_values` 跨行
//!   聚合语义，逐行删值；剩余引用重接，空则写合法空值 `[]`）。
//!
//! 修复落在 markdown source 层再走正常重编译（source/IR/report 新鲜一致，
//! `verify_publish_freshness` 链路零变化）；残余 Error（`required_contract_missing`/
//! `unknown_provider_logical_work_item`/环/重复等非机械类）不拦截，照常产生
//! 机械 verdict 走 F5-A 模型返修。保守边界：只消费上述两个 code 的 Error
//! finding，其余 code 一律不碰。补齐全程零 verdict、零预算、零指纹——
//! repeated_fingerprint 闸门冷态不受影响。

use std::collections::BTreeMap;

use crate::product::work_item_contract::{
    ContractFindingSeverity, DependencyContractGraph, build_dependency_contract_graph,
    validate_dependency_contract_graph,
};
use crate::product::work_item_plan_compiler::{
    CompilerDiagnostic, PlanCandidateIr, WorkItemPlanSourceContext, compile_work_item_plan,
    parse_work_item_plan,
};

/// 收敛上限。每轮补齐严格消除至少一个机械缺口（provider capability 集合
/// 单调增长、未消费引用单调减少），实测 1 轮收敛；上限仅防御程序性退化，
/// 触顶时残余 findings 照常走模型返修。
const MAX_CONTRACT_AUTOREPAIR_ROUNDS: usize = 8;

pub(crate) struct ContractAutorepairOutcome {
    pub(crate) source: String,
    /// 确定性补齐日志（无时间戳/随机源，进 timeline 消息供人审阅）。
    pub(crate) applied: Vec<String>,
}

/// 收敛循环：compile → 逐轮确定性补齐 → 重编译，直到无可机械修复缺口或
/// 达上限。首个 compile 失败原样上抛（与既有 author 落盘路径同语义）；
/// 补齐产物再编译失败同样上抛——补丁行不合法属程序缺陷，fail-closed
/// 不允许静默回退掩盖。返回最终 IR、最终 source 与补齐日志。
pub(crate) fn converge_work_item_plan_source(
    source: &str,
    context: WorkItemPlanSourceContext,
) -> Result<(PlanCandidateIr, String, Vec<String>), Vec<CompilerDiagnostic>> {
    let mut source = source.to_string();
    let mut applied = Vec::new();
    let mut ir = compile_work_item_plan(&source, &context)?;
    for _ in 0..MAX_CONTRACT_AUTOREPAIR_ROUNDS {
        let Some(outcome) = apply_contract_autorepairs(&source, &ir) else {
            break;
        };
        source = outcome.source;
        applied.extend(outcome.applied);
        ir = compile_work_item_plan(&source, &context)?;
    }
    Ok((ir, source, applied))
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

/// 对单轮 IR 的机械契约缺口应用确定性修复；无可修缺口（干净候选或仅剩
/// 非机械类 finding）返回 None。
pub(crate) fn apply_contract_autorepairs(
    source: &str,
    ir: &PlanCandidateIr,
) -> Option<ContractAutorepairOutcome> {
    let contracts = ir
        .items
        .iter()
        .map(|item| item.contract.clone())
        .collect::<Vec<_>>();
    // Err（重复 identity 等）不含两类机械缺口，交给模型返修。
    let graph = build_dependency_contract_graph(&contracts).ok()?;
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
        return None;
    }
    let ast = parse_work_item_plan(source).ok()?;
    let mut replacements = BTreeMap::<usize, String>::new();
    let mut inserts = BTreeMap::<usize, Vec<String>>::new();
    let mut applied = Vec::new();

    // capability 缺口按 (provider, contract_id) 聚合：同契约多条缺失并入
    // 同一行（BTreeMap 键序确定，日志与行文均确定）。
    let mut gaps_by_contract =
        BTreeMap::<(String, String), std::collections::BTreeSet<String>>::new();
    for gap in capability_gaps {
        // 多个消费方可各自报出同一缺失 capability：组内去重，避免重复追加。
        gaps_by_contract
            .entry((gap.provider, gap.contract_id))
            .or_default()
            .insert(gap.capability);
    }
    for ((provider, contract_id), capabilities) in gaps_by_contract {
        match outputs_capabilities_anchor(&ast, &provider, &contract_id) {
            OutputsAnchor::MergeInto { line, value } => {
                let mut merged = value;
                for capability in &capabilities {
                    if !merged.is_empty() {
                        merged.push_str(", ");
                    }
                    merged.push_str(capability);
                }
                replacements.insert(line, format!("- capabilities: {merged}"));
            }
            OutputsAnchor::InsertAfter { line } => {
                inserts.insert(
                    line,
                    vec![format!(
                        "- capabilities: {}",
                        capabilities.iter().cloned().collect::<Vec<_>>().join(", ")
                    )],
                );
            }
            OutputsAnchor::NotFound => continue,
        }
        applied.push(format!(
            "{provider}/{contract_id} 追加 capability {}",
            capabilities.iter().cloned().collect::<Vec<_>>().join(", ")
        ));
    }

    // 未消费 handoff 引用按 provider 聚合，逐行删值。
    let mut refs_by_provider = BTreeMap::<String, Vec<String>>::new();
    for gap in handoff_gaps {
        refs_by_provider
            .entry(gap.provider)
            .or_default()
            .push(gap.contract_ref);
    }
    for (provider, contract_refs) in refs_by_provider {
        let mut removed_any = false;
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
            removed_any = true;
            replacements.insert(
                line,
                format!(
                    "- provided_contract_refs: {}",
                    if remaining.is_empty() {
                        "[]".to_string()
                    } else {
                        remaining.join(", ")
                    }
                ),
            );
        }
        if removed_any {
            applied.push(format!(
                "{provider} 移除未消费 handoff 引用 {}",
                contract_refs.join(", ")
            ));
        }
    }

    if applied.is_empty() {
        return None;
    }
    let repaired = rewrite_lines(source, &replacements, &inserts);
    Some(ContractAutorepairOutcome {
        source: repaired,
        applied,
    })
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

/// 按行号应用替换/插入（前向单遍重建，行号互不位移；替换与插入按行号
/// 精确对应；保留原 source 的收尾换行形态）。
fn rewrite_lines(
    source: &str,
    replacements: &BTreeMap<usize, String>,
    inserts: &BTreeMap<usize, Vec<String>>,
) -> String {
    let had_trailing_newline = source.ends_with('\n');
    let mut lines = Vec::with_capacity(source.lines().count() + inserts.len());
    for (index, line) in source.lines().enumerate() {
        let line_no = index + 1;
        match replacements.get(&line_no) {
            Some(replacement) => lines.push(replacement.clone()),
            None => lines.push(line.to_string()),
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

    #[test]
    fn merges_missing_capability_into_provider_last_capabilities_line() {
        let source = capability_gap_candidate();
        let outcome = apply_contract_autorepairs(&source, &compile(&source))
            .expect("capability gap must be repairable");
        assert!(
            outcome
                .source
                .contains("- capabilities: api.levels.read, api.levels.write\n"),
            "missing capability must merge verbatim into the provider line:\n{}",
            outcome.source
        );
        assert_eq!(outcome.applied.len(), 1);
        assert!(
            outcome.applied[0].contains("追加 capability api.levels.write"),
            "deterministic log entry: {}",
            outcome.applied[0]
        );
        // 重编译后机械契约校验干净。
        let repaired_ir = compile(&outcome.source);
        let contracts = repaired_ir
            .items
            .iter()
            .map(|item| item.contract.clone())
            .collect::<Vec<_>>();
        let graph = build_dependency_contract_graph(&contracts).expect("graph");
        assert!(
            validate_dependency_contract_graph(&graph).is_valid(),
            "repaired source must validate clean"
        );
    }

    #[test]
    fn merge_targets_the_last_capabilities_line_of_the_contract_block() {
        // `lower_outputs` 同一契约多行 capabilities 为「最后一条生效」；
        // 补齐必须并入末行而不是新增一行（新增行会整体顶掉既有能力）。
        let source = clean_candidate()
            .replace(
                "- capabilities: api.levels.read\n",
                "- capabilities: api.levels.stale\n- capabilities: api.levels.read\n",
            )
            .replace(
                "- required_capabilities: api.levels.read\n",
                "- required_capabilities: api.levels.read, api.levels.write\n",
            );
        let outcome = apply_contract_autorepairs(&source, &compile(&source))
            .expect("capability gap must be repairable");
        assert!(
            outcome
                .source
                .contains("- capabilities: api.levels.read, api.levels.write\n"),
            "merge must land on the live (last) capabilities line:\n{}",
            outcome.source
        );
        assert!(
            !outcome
                .source
                .contains("api.levels.write, api.levels.write"),
            "repair must stay idempotent-free of duplicated appends"
        );
    }

    #[test]
    fn inserts_new_capabilities_line_when_contract_block_has_none() {
        // 目标契约块无 capabilities 行（section 级语法仍满足：前一个契约块
        // 带 capabilities 键），补齐在该 `- contract_id:` 行后插入新行。
        let source = clean_candidate()
            .replace(
                "- contract_id: contract.levels-api\n- capabilities: api.levels.read\n",
                "- contract_id: contract.audit\n- capabilities: audit.evidence\n- contract_id: contract.levels-api\n",
            )
            .replace(
                "- required_capabilities: api.levels.read\n",
                "- required_capabilities: api.levels.read, api.levels.write\n",
            );
        let outcome = apply_contract_autorepairs(&source, &compile(&source))
            .expect("gap on a capabilities-less block must insert a line");
        assert!(
            outcome.source.contains(
                "- contract_id: contract.levels-api\n- capabilities: api.levels.read, api.levels.write\n"
            ),
            "a new capabilities line must follow the contract_id line:\n{}",
            outcome.source
        );
        let repaired_ir = compile(&outcome.source);
        let contracts = repaired_ir
            .items
            .iter()
            .map(|item| item.contract.clone())
            .collect::<Vec<_>>();
        let graph = build_dependency_contract_graph(&contracts).expect("graph");
        assert!(validate_dependency_contract_graph(&graph).is_valid());
    }

    #[test]
    fn removes_unconsumed_handoff_ref_and_rewrites_to_legal_empty_list() {
        let outcome = apply_contract_autorepairs(REP4_FIXTURE, &compile(REP4_FIXTURE))
            .expect("unconsumed handoff must be repairable");
        assert!(
            outcome.source.contains("- provided_contract_refs: []\n"),
            "sole ref removal must rewrite the legal empty value:\n{}",
            outcome.source
        );
        assert_eq!(outcome.applied.len(), 1);
        let repaired_ir = compile(&outcome.source);
        let contracts = repaired_ir
            .items
            .iter()
            .map(|item| item.contract.clone())
            .collect::<Vec<_>>();
        let graph = build_dependency_contract_graph(&contracts).expect("graph");
        assert!(validate_dependency_contract_graph(&graph).is_valid());
    }

    #[test]
    fn keeps_sibling_refs_when_removing_one_from_a_multi_ref_line() {
        let source = clean_candidate().replace(
            "- provided_contract_refs: contract.levels-api\n",
            "- provided_contract_refs: contract.levels-api, contract.levels-extra\n",
        );
        // contract.levels-extra 未被消费 → 删它，保留仍被消费的 levels-api。
        let outcome = apply_contract_autorepairs(&source, &compile(&source))
            .expect("multi-ref handoff gap must be repairable");
        assert!(
            outcome
                .source
                .contains("- provided_contract_refs: contract.levels-api\n"),
            "consumed sibling ref must survive the removal:\n{}",
            outcome.source
        );
        assert!(!outcome.source.contains("contract.levels-extra"));
    }

    #[test]
    fn clean_and_non_mechanical_sources_return_none() {
        assert!(
            apply_contract_autorepairs(&clean_candidate(), &compile(&clean_candidate())).is_none()
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
        assert!(apply_contract_autorepairs(&cycled, &ir).is_none());
    }

    #[test]
    fn converge_loop_repairs_until_clean_and_reports_log() {
        let (ir, source, applied) = converge_work_item_plan_source(
            &capability_gap_candidate(),
            WorkItemPlanSourceContext {
                target_repository_id: "repo_fixture".to_string(),
            },
        )
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
}
