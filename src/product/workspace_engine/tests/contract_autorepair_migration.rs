//! O1（裁决二 §2.2/§2.4）迁移用例：contract autorepair 的本地 source
//! mutation 收敛（八轮循环 + 删行/替换/删整 check）清切为「机器指令交
//! 原授权 author」——平台只诊断与开方，不替 author 改写业务内容。
//!
//! 覆盖：四类收窄口径各一正一拒 + 两条 caller 路径的 source 字节不变
//! 证明；原 `contract_autorepair.rs` 内联测试块按 8.4 迁入并改写断言
//! 口径（source 不变、指令完整、author 新产物及复验）。
use super::*;
use crate::product::json_store::write_json;
use crate::product::models::SingleCandidatePhase;
use crate::product::planning_failure::PlanningFailureKind;
use crate::product::work_item_contract::{
    build_dependency_contract_graph, validate_dependency_contract_graph,
};
use crate::product::work_item_plan_compiler::{
    CompilerDiagnostic, PlanCandidateIr, WorkItemPlanSourceContext, compile_work_item_plan,
};
use crate::product::work_item_plan_policy::{RunPolicy, WorkItemPlanFlowKind};
use crate::product::work_item_plan_source_store::WorkItemPlanSourceStore;
use crate::product::workspace_engine::contract_autorepair::{self, ContractConvergeOutcome};
use crate::product::workspace_engine::contract_prerevision::{
    StructuralRepairInstruction, StructuralRepairOperation, StructuralRepairSolution,
};
use sha2::{Digest, Sha256};

const REP4_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/product/work_item_plan_compiler/fixtures/work-item-plan-rep4.md"
));

/// campaign 同口径：终端 WI-003 handoff 提供行清空（干净基线）。
fn clean_candidate() -> String {
    REP4_FIXTURE.replace(
        "- provided_contract_refs: contract.levels-integration",
        "- provided_contract_refs: []",
    )
}

/// WI-002/WI-003 的 require_all 塞入 WI-001 未提供的 capability。
fn capability_gap_candidate() -> String {
    clean_candidate().replace(
        "- required_capabilities: api.levels.read\n",
        "- required_capabilities: api.levels.read, api.levels.write\n",
    )
}

fn source_context() -> WorkItemPlanSourceContext {
    WorkItemPlanSourceContext {
        target_repository_id: "repo_fixture".to_string(),
        member_targets: None,
    }
}

fn compile(source: &str) -> PlanCandidateIr {
    compile_work_item_plan(source, &source_context()).expect("fixture compiles")
}

/// O1：指令解中所有「行删除/替换/插入」类操作必须为空——四类收窄
/// 指令只携带事实与 author 处置口径，不携带本地行编辑。
fn line_edit_operations(
    instructions: &[StructuralRepairInstruction],
) -> Vec<&StructuralRepairOperation> {
    let mut operations = Vec::new();
    for instruction in instructions {
        if let StructuralRepairSolution::Precise {
            operations: precise,
            ..
        } = &instruction.solution
        {
            operations.extend(precise.iter().filter(|operation| {
                matches!(
                    operation,
                    StructuralRepairOperation::ReplaceLine { .. }
                        | StructuralRepairOperation::InsertLines { .. }
                        | StructuralRepairOperation::DeleteLines { .. }
                )
            }));
        }
    }
    operations
}

fn assert_validates_clean(source: &str) {
    let ir = compile(source);
    let contracts = ir
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

// —— 复验 F-41：修订轮 author 在 `### Handoff Schema` 自创 `requested_fields`
// （STRUCTURED_KEYS 白名单外）→ parse 阶段 `unknown_structured_key`。
fn unknown_structured_key_candidate() -> String {
    clean_candidate().replacen(
        "- required_fields: commit_sha\n",
        "- required_fields: commit_sha\n- requested_fields: 无\n",
        1,
    )
}

/// 重复 trusted command（重复 check 携带非空 manual_instruction 支）。
fn duplicate_with_manual_candidate() -> String {
    clean_candidate().replacen(
        "- command: cargo test --locked --lib levels_api\n- manual_instruction: Confirm the endpoint returns the configured levels JSON.\n- required: true\n- non_zero_test_execution_required: true",
        "- command: cargo test --locked --lib levels_api\n- manual_instruction: Confirm the endpoint returns the configured levels JSON.\n- required: true\n- non_zero_test_execution_required: true\n- check_id: CHECK-004\n- command: cargo test --locked --lib levels_api\n- manual_instruction: Confirm the endpoint still returns configured levels JSON.\n- required: true\n- non_zero_test_execution_required: true",
        1,
    )
}

/// 重复 trusted command（重复 check 无 manual_instruction 支）。
fn duplicate_without_manual_candidate() -> String {
    clean_candidate().replacen(
        "- command: cargo test --locked --lib levels_api\n- manual_instruction: Confirm the endpoint returns the configured levels JSON.\n- required: true\n- non_zero_test_execution_required: true",
        "- command: cargo test --locked --lib levels_api\n- manual_instruction: Confirm the endpoint returns the configured levels JSON.\n- required: true\n- non_zero_test_execution_required: true\n- check_id: CHECK-004\n- command: cargo test --locked --lib levels_api\n- required: true\n- non_zero_test_execution_required: true",
        1,
    )
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

// ===================== O1 四类收窄口径（各一正一拒） =====================

/// `required_capability_missing`：converge 返回 Ok + 指令；source 字节
/// 不变（重塑后无 repaired_source，`apply_structural_repair_operations`
/// 已删除即编译期保证）；指令集不含任何删除/改写行操作。
#[test]
fn capability_gap_yields_instruction_not_mutation() {
    let source = capability_gap_candidate();
    let context = source_context();
    let outcome = contract_autorepair::converge_work_item_plan_source(&source, context.clone())
        .expect("capability gap must compile; converge must emit instructions without repairing");
    let ContractConvergeOutcome {
        ir,
        instructions,
        audit,
    } = outcome;
    // 入参 source 的 sha256 与指令 source_hash 一致（绑定 author 原文）。
    assert_eq!(
        hex::encode(Sha256::digest(source.as_bytes())),
        instructions[0].source_hash
    );
    assert_eq!(ir.items.len(), 3, "IR 由 author 原始 source 编译");
    assert!(
        audit.equivalence_canonicalizations.is_empty(),
        "单仓 fixture 无等价规范化"
    );
    assert_eq!(
        instructions.len(),
        1,
        "capability gap must produce one instruction"
    );
    assert!(
        instructions[0]
            .repair_id
            .contains("required_capability_missing")
    );
    assert!(
        instructions[0]
            .required_action
            .contains("回查原业务与实现承诺"),
        "required_action 必须是回查口径：{}",
        instructions[0].required_action
    );
    assert!(
        line_edit_operations(&instructions).is_empty(),
        "O1：指令不得携带任何删除/改写行操作"
    );
    // 确定性：同输入两次 converge 全等。
    let again = contract_autorepair::converge_work_item_plan_source(&source, context)
        .expect("second converge must succeed");
    assert_eq!(again.instructions, instructions);
    assert_eq!(again.audit, audit);
}

/// `unconsumed_required_handoff`：不删 ref/替换 `[]`，产完整来源事实指令
/// （provider、原始行值、未消费引用集）。
#[test]
fn handoff_ref_kept_with_full_source_facts() {
    let source = REP4_FIXTURE.to_string();
    let outcome = contract_autorepair::converge_work_item_plan_source(&source, source_context())
        .expect("rep4 must compile");
    let instructions = outcome.instructions;
    assert_eq!(instructions.len(), 1);
    let instruction = &instructions[0];
    assert!(
        instruction
            .repair_id
            .contains("unconsumed_required_handoff")
    );
    // 完整来源事实：provider、原始行值、未消费引用。
    assert!(
        instruction.required_action.contains("WI-003"),
        "provider 事实：{}",
        instruction.required_action
    );
    assert!(
        instruction
            .required_action
            .contains("contract.levels-integration"),
        "未消费引用事实：{}",
        instruction.required_action
    );
    assert!(
        instruction
            .required_action
            .contains("provided_contract_refs"),
        "原始行事实：{}",
        instruction.required_action
    );
    assert!(line_edit_operations(&instructions).is_empty());
}

/// `unknown_structured_key`：不按诊断行删业务承载行，只产「保留内容、
/// 迁往合法字段」指令；converge 保持 Err（编译失败不被本地修复）。
#[test]
fn unknown_key_business_content_preserved() {
    let source = unknown_structured_key_candidate();
    let context = source_context();
    let diagnostics = contract_autorepair::converge_work_item_plan_source(&source, context.clone())
        .expect_err("unknown key must stay a compile failure (no local repair)");
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "unknown_structured_key" && diagnostic.field == "requested_fields"
        }),
        "compile must report the unknown key: {diagnostics:?}"
    );
    let instructions = contract_autorepair::build_compile_failure_repair_instructions(
        &source,
        &diagnostics,
        &context,
    );
    assert_eq!(instructions.len(), 1);
    let instruction = &instructions[0];
    assert!(instruction.repair_id.contains("unknown_structured_key"));
    assert!(
        instruction.required_action.contains("迁往合法字段"),
        "指令必须是保留内容/迁移口径：{}",
        instruction.required_action
    );
    assert!(line_edit_operations(&instructions).is_empty());
}

/// 重复 trusted command：指令指出重复及受影响 check/ref，manual 与无
/// manual 两支处置均在指令内；平台不删 command/整 check。
#[test]
fn duplicate_trusted_command_both_branches_in_instruction() {
    let context = source_context();
    for (name, source) in [
        ("manual", duplicate_with_manual_candidate()),
        ("no-manual", duplicate_without_manual_candidate()),
    ] {
        let diagnostics = compile_work_item_plan(&source, &context)
            .expect_err("{name} fixture must fail compile on duplicate command");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.field == "trusted_commands"),
            "{name}: {diagnostics:?}"
        );
        let instructions = contract_autorepair::build_compile_failure_repair_instructions(
            &source,
            &diagnostics,
            &context,
        );
        assert_eq!(instructions.len(), 1, "{name}: exactly one instruction");
        let instruction = &instructions[0];
        assert!(
            instruction.repair_id.contains("duplicate_trusted_command"),
            "{name}: {}",
            instruction.repair_id
        );
        assert!(
            instruction.repair_id.contains("CHECK-004")
                && instruction
                    .affected_work_item_ids
                    .contains(&"WI-001".to_string()),
            "{name}: 受影响 check/ref 必须进入指令身份"
        );
        // 两支处置均在指令文本内。
        assert!(
            instruction.required_action.contains("manual_instruction")
                && instruction.required_action.contains("整块删除"),
            "{name}: 两支处置口径：{}",
            instruction.required_action
        );
        assert!(
            line_edit_operations(&instructions).is_empty(),
            "{name}: O1 不删 command/整 check"
        );
    }
}

// ===================== O1 拒绝场景（8.5） =====================

/// 已 current/reviewed 产物不改：completion 只产指令（verdict 路由），
/// 持久化 source 与 author 原始输出逐字节一致（REP4 自带 unconsumed
/// handoff，旧路径会改写为 `[]`）。
#[tokio::test]
async fn persisted_or_reviewed_source_never_touched() {
    let (_tmp, lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    author_round_record(&lifecycle, &mut engine);

    engine
        .complete_single_candidate_work_item_plan_author(
            REP4_FIXTURE.to_string(),
            fixture_sc_target(&engine),
            fixture_sc_member_targets(&engine),
            false,
        )
        .await
        .expect("unconsumed-handoff candidate must complete with instructions only");

    let persisted = persisted_source(&lifecycle, &engine);
    assert_eq!(
        persisted, REP4_FIXTURE,
        "completion must not mutate the author source"
    );
    assert!(persisted.contains("- provided_contract_refs: contract.levels-integration"));
    // 只产指令：缺口沿既有机械 verdict 通道（F5-A）交 author 返修。
    let pending = engine
        .single_candidate_pending_revision_verdict()
        .expect("unconsumed handoff gap must route a mechanical revision round");
    assert!(
        pending
            .findings
            .iter()
            .any(|finding| finding.message.starts_with("unconsumed_required_handoff: ")),
        "handoff finding must ride the verdict: {:?}",
        pending
            .findings
            .iter()
            .map(|f| &f.message)
            .collect::<Vec<_>>()
    );
}

/// 整 check 不得无故消失（8.5）：重复 command 的无 manual 支不再由平台
/// 删整 check——指令只指出重复与受影响 check。
#[test]
fn whole_check_not_dropped() {
    let source = duplicate_without_manual_candidate();
    let context = source_context();
    let diagnostics = contract_autorepair::converge_work_item_plan_source(&source, context.clone())
        .expect_err("duplicate trusted command must stay a compile failure");
    let instructions = contract_autorepair::build_compile_failure_repair_instructions(
        &source,
        &diagnostics,
        &context,
    );
    assert_eq!(instructions.len(), 1);
    let instruction = &instructions[0];
    assert!(instruction.required_action.contains("CHECK-004"));
    assert!(
        line_edit_operations(&instructions).is_empty(),
        "整 check 不得由平台删除"
    );
    // 受影响 check 的行定位进入 span（author 可回查）。
    let span = &instruction.affected_spans[0];
    assert!(span.line >= 1, "span 必须携带行定位：{span:?}");
}

/// 未知 key 携业务正文不得消失（8.5）：指令必须携带该行原文并要求
/// 先迁移再随新 revision 处置。
#[test]
fn unknown_key_payload_not_vanished() {
    let source = unknown_structured_key_candidate();
    let context = source_context();
    let diagnostics = contract_autorepair::converge_work_item_plan_source(&source, context.clone())
        .expect_err("unknown key must stay a compile failure");
    let instructions = contract_autorepair::build_compile_failure_repair_instructions(
        &source,
        &diagnostics,
        &context,
    );
    assert_eq!(instructions.len(), 1);
    let instruction = &instructions[0];
    assert!(
        instruction
            .required_action
            .contains("- requested_fields: 无"),
        "业务正文原文必须随指令携带：{}",
        instruction.required_action
    );
    assert!(
        instruction.required_action.contains("迁移"),
        "处置口径必须是先迁移：{}",
        instruction.required_action
    );
    assert!(line_edit_operations(&instructions).is_empty());
}

// ===================== Review Focus ④：两条 caller 路径 =====================

/// completion 路径（`workspace_engine/single_candidate.rs` 的 converge 调用）：
/// 最终持久化 source 与 author 原始输出逐字节一致，IR 由原始 source
/// 编译，缺口走机械 verdict（F5-A author 返修），不再二次暗修。
#[tokio::test]
async fn engine_completion_path_does_not_mutate_source() {
    let (_tmp, lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    author_round_record(&lifecycle, &mut engine);

    let author_source = capability_gap_candidate();
    engine
        .complete_single_candidate_work_item_plan_author(
            author_source.clone(),
            fixture_sc_target(&engine),
            fixture_sc_member_targets(&engine),
            false,
        )
        .await
        .expect("gapped candidate must complete with instructions, not mutation");

    let persisted = persisted_source(&lifecycle, &engine);
    assert_eq!(
        persisted, author_source,
        "completion path must keep the author source byte-identical"
    );
    // 缺口未消失：机械 verdict 携带 capability finding 路由 author 返修。
    let pending = engine
        .single_candidate_pending_revision_verdict()
        .expect("capability gap must route a mechanical revision round");
    assert!(
        pending
            .findings
            .iter()
            .any(|finding| finding.message.starts_with("required_capability_missing: ")),
        "capability finding must ride the verdict: {:?}",
        pending
            .findings
            .iter()
            .map(|f| &f.message)
            .collect::<Vec<_>>()
    );
    // 持久化 IR 与 author source 新鲜一致（重编译逐字节同源）。
    let ir = compile_work_item_plan(&persisted, &source_context())
        .expect("author source must recompile");
    assert_eq!(ir.items.len(), 3);
}

// ============== 原 contract_autorepair.rs 内联测试迁入（8.4 改写口径） ==============

/// 迁自 `merges_missing_capability_into_provider_last_capabilities_line`：
/// capability 指令锚定 provider 生效 capabilities 行（定位事实保留），
/// 但不再携带行替换操作。
#[test]
fn capability_instruction_anchors_provider_capabilities_line_without_edit() {
    let source = capability_gap_candidate();
    let context = source_context();
    let instructions = contract_autorepair::build_contract_autorepair_instructions(
        &source,
        &compile(&source),
        &context,
    );
    assert_eq!(instructions.len(), 1);
    let span = &instructions[0].affected_spans[0];
    assert_eq!(
        span.line,
        source
            .lines()
            .position(|line| line == "- capabilities: api.levels.read")
            .expect("fixture carries the provider capabilities line")
            + 1
    );
    assert_eq!(span.field, "outputs.contract.levels-api.capabilities");
    assert!(
        instructions[0].required_action.contains("api.levels.write"),
        "缺失 capability 事实：{}",
        instructions[0].required_action
    );
    assert!(line_edit_operations(&instructions).is_empty());
}

/// 迁自 `merge_targets_the_last_capabilities_line_of_the_contract_block`：
/// 同一契约多行 capabilities「最后一条生效」——指令锚定末行（新增行会
/// 整体顶掉既有能力的旧坑不再由平台规避，定位事实仍指生效行）。
#[test]
fn capability_instruction_targets_the_last_capabilities_line() {
    let source = clean_candidate()
        .replace(
            "- capabilities: api.levels.read\n",
            "- capabilities: api.levels.stale\n- capabilities: api.levels.read\n",
        )
        .replace(
            "- required_capabilities: api.levels.read\n",
            "- required_capabilities: api.levels.read, api.levels.write\n",
        );
    let instructions = contract_autorepair::build_contract_autorepair_instructions(
        &source,
        &compile(&source),
        &source_context(),
    );
    assert_eq!(instructions.len(), 1);
    let span = &instructions[0].affected_spans[0];
    let last = source
        .lines()
        .enumerate()
        .filter(|(_, line)| *line == "- capabilities: api.levels.read")
        .map(|(index, _)| index + 1)
        .last()
        .expect("fixture carries two capabilities lines");
    assert_eq!(span.line, last, "定位必须指向生效（末）行");
    assert!(line_edit_operations(&instructions).is_empty());
}

/// 迁自 `inserts_new_capabilities_line_when_contract_block_has_none`：
/// 契约块无 capabilities 行时，指令锚定 `- contract_id:` 行（事实定位），
/// 不再由平台插入新行。
#[test]
fn capabilities_less_block_anchor_keeps_contract_id_line_without_insert() {
    let source = clean_candidate()
        .replace(
            "- contract_id: contract.levels-api\n- capabilities: api.levels.read\n",
            "- contract_id: contract.audit\n- capabilities: audit.evidence\n- contract_id: contract.levels-api\n",
        )
        .replace(
            "- required_capabilities: api.levels.read\n",
            "- required_capabilities: api.levels.read, api.levels.write\n",
        );
    let instructions = contract_autorepair::build_contract_autorepair_instructions(
        &source,
        &compile(&source),
        &source_context(),
    );
    assert_eq!(instructions.len(), 1);
    let span = &instructions[0].affected_spans[0];
    assert_eq!(
        span.line,
        source
            .lines()
            .position(|line| line == "- contract_id: contract.levels-api")
            .expect("fixture carries the target contract block")
            + 1
    );
    assert!(line_edit_operations(&instructions).is_empty());
}

/// 迁自 `removes_unconsumed_handoff_ref_and_rewrites_to_legal_empty_list`：
/// 不再改写为合法空值 `[]`——指令携带原值与未消费引用事实；author 按
/// 指令补齐能力后复验干净（author 新产物及复验口径）。
#[test]
fn handoff_instruction_keeps_original_value_and_author_repair_revalidates() {
    let source = REP4_FIXTURE.to_string();
    let context = source_context();
    let instructions = contract_autorepair::build_contract_autorepair_instructions(
        &source,
        &compile(&source),
        &context,
    );
    assert_eq!(instructions.len(), 1);
    assert!(
        instructions[0]
            .required_action
            .contains("contract.levels-integration"),
        "原值事实：{}",
        instructions[0].required_action
    );
    assert!(line_edit_operations(&instructions).is_empty());

    // author 新产物及复验：按指令口径（回查后移除未消费引用）产出新
    // revision，复验干净且 converge 不再产指令。
    let author_revised = source.replace(
        "- provided_contract_refs: contract.levels-integration",
        "- provided_contract_refs: []",
    );
    assert_validates_clean(&author_revised);
    let outcome = contract_autorepair::converge_work_item_plan_source(&author_revised, context)
        .expect("author revision must converge");
    assert!(
        outcome.instructions.is_empty(),
        "author 修复后的产物不再产指令"
    );
}

/// 迁自 `keeps_sibling_refs_when_removing_one_from_a_multi_ref_line`：
/// 多引用行只指认未消费引用，仍被消费的兄弟引用事实保留。
#[test]
fn handoff_instruction_names_only_unconsumed_refs() {
    let source = clean_candidate().replace(
        "- provided_contract_refs: contract.levels-api\n",
        "- provided_contract_refs: contract.levels-api, contract.levels-extra\n",
    );
    let instructions = contract_autorepair::build_contract_autorepair_instructions(
        &source,
        &compile(&source),
        &source_context(),
    );
    assert_eq!(instructions.len(), 1);
    let action = &instructions[0].required_action;
    assert!(
        action.contains("contract.levels-extra"),
        "未消费引用指认：{action}"
    );
    assert!(
        action.contains("contract.levels-api, contract.levels-extra"),
        "原始行完整事实：{action}"
    );
    assert!(line_edit_operations(&instructions).is_empty());
}

/// 迁自 `clean_and_non_mechanical_sources_return_none`（口径不变）。
#[test]
fn clean_and_non_mechanical_sources_return_none() {
    assert!(
        contract_autorepair::build_contract_autorepair_instructions(
            &clean_candidate(),
            &compile(&clean_candidate()),
            &source_context()
        )
        .is_empty()
    );
    // 依赖环不在四类收窄缺口内：不产指令。
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
    assert!(
        contract_autorepair::build_contract_autorepair_instructions(
            &cycled,
            &ir,
            &source_context()
        )
        .is_empty()
    );
}

/// 迁自 `converge_loop_repairs_until_clean_and_reports_log`（改写：无修复
/// 轮次、无日志——converge 只产指令；author 修复后的产物复验不再产指令）。
#[test]
fn converge_reports_gap_instructions_without_repair_rounds() {
    let source = capability_gap_candidate();
    let context = source_context();
    let outcome = contract_autorepair::converge_work_item_plan_source(&source, context.clone())
        .expect("converge must succeed without repair rounds");
    assert_eq!(outcome.instructions.len(), 1);
    // author 按指令产出新 revision（补齐 capability）后复验干净。
    let author_revised = source.replace(
        "- capabilities: api.levels.read\n",
        "- capabilities: api.levels.read, api.levels.write\n",
    );
    assert_validates_clean(&author_revised);
    let revised = contract_autorepair::converge_work_item_plan_source(&author_revised, context)
        .expect("author revision must converge");
    assert!(revised.instructions.is_empty());
}

/// 迁自 `converge_repairs_duplicate_trusted_command_without_losing_manual_
/// instruction`（改写：不修——Err + 指令；manual 分支事实进入指令）。
#[test]
fn duplicate_with_manual_branch_facts_in_instruction() {
    let source = duplicate_with_manual_candidate();
    let context = source_context();
    let diagnostics = contract_autorepair::converge_work_item_plan_source(&source, context.clone())
        .expect_err("duplicate trusted command must not be locally repaired");
    let instructions = contract_autorepair::build_compile_failure_repair_instructions(
        &source,
        &diagnostics,
        &context,
    );
    assert_eq!(instructions.len(), 1);
    let instruction = &instructions[0];
    // manual 分支事实：受影响 check 携带非空 manual_instruction。
    assert!(
        instruction
            .required_action
            .contains("保留 manual_instruction 与既有验证语义"),
        "manual 分支处置：{}",
        instruction.required_action
    );
    assert!(line_edit_operations(&instructions).is_empty());
}

/// 迁自 `converge_removes_duplicate_check_without_manual_instruction`
/// （改写：整 check 不被平台删除，无 manual 支事实进入指令）。
#[test]
fn duplicate_without_manual_branch_facts_in_instruction() {
    let source = duplicate_without_manual_candidate();
    let context = source_context();
    let diagnostics = contract_autorepair::converge_work_item_plan_source(&source, context.clone())
        .expect_err("duplicate check must not be locally dropped");
    let instructions = contract_autorepair::build_compile_failure_repair_instructions(
        &source,
        &diagnostics,
        &context,
    );
    assert_eq!(instructions.len(), 1);
    assert!(
        instructions[0].required_action.contains("无独立人工说明"),
        "无 manual 分支处置：{}",
        instructions[0].required_action
    );
    assert!(line_edit_operations(&instructions).is_empty());
}

/// 迁自 `converge_removes_unknown_structured_key_line_and_recompiles_clean`
/// （改写：不删行——Err 保持；指令要求先迁内容）。
#[test]
fn unknown_structured_key_stays_compile_failure_with_migration_instruction() {
    let source = unknown_structured_key_candidate();
    let context = source_context();
    let diagnostics = contract_autorepair::converge_work_item_plan_source(&source, context.clone())
        .expect_err("unknown structured key must fail the compile");
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "unknown_structured_key" && diagnostic.field == "requested_fields"
        }),
        "first compile must report the unknown key: {diagnostics:?}"
    );
    let instructions = contract_autorepair::build_compile_failure_repair_instructions(
        &source,
        &diagnostics,
        &context,
    );
    assert_eq!(instructions.len(), 1);
    assert!(
        instructions[0].required_action.contains("迁往合法字段"),
        "迁移口径：{}",
        instructions[0].required_action
    );
    assert!(line_edit_operations(&instructions).is_empty());
}

/// 迁自 `unknown_structured_key_repair_requires_the_line_to_carry_the_key`
/// （改写：行号精确映射仍是指令定位资格——错位/越界 fail-closed 零指令；
/// 命中行的指令 span 按诊断行号定位）。
#[test]
fn unknown_structured_key_instruction_requires_the_line_to_carry_the_key() {
    let source = unknown_structured_key_candidate();
    let context = source_context();
    // 行号错位（指向相邻的合法行）与行号越界均不产指令。
    assert!(
        contract_autorepair::build_compile_failure_repair_instructions(
            &source,
            &[unknown_key_diagnostic(50, "requested_fields")],
            &context
        )
        .is_empty()
    );
    assert!(
        contract_autorepair::build_compile_failure_repair_instructions(
            &source,
            &[unknown_key_diagnostic(
                source.lines().count() + 1,
                "requested_fields"
            )],
            &context
        )
        .is_empty()
    );
    // 命中行时指令 span 按诊断行号精确锚定。
    let target = source
        .lines()
        .position(|line| line == "- requested_fields: 无")
        .expect("fixture carries the unknown key line")
        + 1;
    let instructions = contract_autorepair::build_compile_failure_repair_instructions(
        &source,
        &[unknown_key_diagnostic(target, "requested_fields")],
        &context,
    );
    assert_eq!(instructions.len(), 1);
    assert_eq!(instructions[0].affected_spans[0].line, target);
    assert!(line_edit_operations(&instructions).is_empty());
}

/// 迁自 `converge_keeps_same_code_non_key_lines_fail_closed`（口径不变：
/// 未知 section 与非 key/value 行共用 code，绝不产指令）。
#[test]
fn converge_keeps_same_code_non_key_lines_fail_closed() {
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
        let diagnostics =
            contract_autorepair::converge_work_item_plan_source(&source, source_context())
                .expect_err("non-key lines sharing the code must stay fail-closed");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "unknown_structured_key"),
            "{diagnostics:?}"
        );
        assert!(
            contract_autorepair::build_compile_failure_repair_instructions(
                &source,
                &diagnostics,
                &source_context()
            )
            .is_empty(),
            "不可精确映射不产指令"
        );
    }
}

/// 迁自 `converge_keeps_non_duplicate_diagnostics_fail_closed`（口径不变）。
#[test]
fn converge_keeps_non_duplicate_diagnostics_fail_closed() {
    let source = REP4_FIXTURE.replacen("- kind: backend\n", "", 1);
    let diagnostics =
        contract_autorepair::converge_work_item_plan_source(&source, source_context())
            .expect_err("non-mechanical compiler diagnostics must remain errors");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.field == "kind" && diagnostic.code != "lowering_error")
    );
    assert!(
        contract_autorepair::build_compile_failure_repair_instructions(
            &source,
            &diagnostics,
            &source_context()
        )
        .is_empty()
    );
}

// —— F1.2（REQ-SG-01/02/02A）：指令资格切 compiler typed ——

/// 迁自 `sg_f1_2_typed_heading_never_deleted_as_unknown_key`（改写：typed
/// heading 永不进入指令，混合集合整体 fail-closed）。
#[test]
fn sg_f1_2_typed_heading_never_produces_unknown_key_instruction() {
    let source = unknown_structured_key_candidate().replacen(
        "### Blockers\n",
        "## 验收说明\n### Blockers\n",
        1,
    );
    let context = source_context();
    let diagnostics =
        compile_work_item_plan(&source, &context).expect_err("heading + unknown key 必须编译失败");
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
        contract_autorepair::build_compile_failure_repair_instructions(
            &source,
            &diagnostics,
            &context
        )
        .is_empty(),
        "typed heading 排除指令资格，混合集合整体 fail-closed（零指令）"
    );
    assert!(
        contract_autorepair::converge_work_item_plan_source(&source, context).is_err(),
        "heading 错误不被本地收敛"
    );
}

/// 迁自 `sg_f1_2_duplicate_command_qualification_is_structural_not_message`
/// （改写：资格判定经 producer 公共面验证——message 漂移不改变判定，
/// typed kind/异 field 排除）。
#[test]
fn sg_f1_2_duplicate_command_qualification_is_structural_not_message() {
    let source = duplicate_with_manual_candidate();
    let context = source_context();
    let diagnostics =
        compile_work_item_plan(&source, &context).expect_err("duplicate fixture must fail compile");
    // message 文案漂移：资格不受影响（结构身份 code+field）。
    let drifted: Vec<CompilerDiagnostic> = diagnostics
        .iter()
        .map(|diagnostic| {
            let mut drifted = diagnostic.clone();
            drifted.message = "文案漂移后的重复 trusted command 描述".to_string();
            drifted
        })
        .collect();
    assert_eq!(
        contract_autorepair::build_compile_failure_repair_instructions(&source, &drifted, &context)
            .len(),
        1,
        "message 漂移不改变结构资格"
    );
    // 携带 typed kind 的诊断不参与（零指令）。
    let typed: Vec<CompilerDiagnostic> = diagnostics
        .iter()
        .map(|diagnostic| {
            let mut typed = drifted[0].clone();
            typed.line = diagnostic.line;
            typed.field = diagnostic.field.clone();
            typed.kind = Some(PlanningFailureKind::InvalidWorkItemHeading {
                line: 1,
                field: "work_item_heading".to_string(),
                raw_line: "## 验收说明".to_string(),
                example: "## Work Item WI-001: fixture".to_string(),
            });
            typed
        })
        .collect();
    assert!(
        contract_autorepair::build_compile_failure_repair_instructions(&source, &typed, &context)
            .is_empty(),
        "携带 typed kind 的诊断不参与指令资格"
    );
}

// —— SG2（REQ-SG-02/02A、REQ-PLN-10）：内容直改迁机器指令（O1 收窄） ——

/// 迁自 `sg_02_no_fake_edge_removal_content_autorepair_migrated_to_pure_
/// instructions`（改写：纯指令 producer——零 source mutation、无行编辑
/// 操作、Candidates 解形态、身份绑定与确定性）。
#[test]
fn sg_02_content_autorepair_migrated_to_author_instructions() {
    let source = capability_gap_candidate();
    let context = source_context();
    let ir = compile(&source);
    let instructions =
        contract_autorepair::build_contract_autorepair_instructions(&source, &ir, &context);
    assert_eq!(instructions.len(), 1, "capability 缺口产一条指令");
    let instruction = &instructions[0];

    // 纯 producer 的身份绑定与确定性（无隐藏状态/循环副作用）。
    assert_eq!(
        instruction.source_hash,
        hex::encode(Sha256::digest(source.as_bytes())),
        "指令 source_hash 绑定待修 source"
    );
    assert!(!instruction.instruction_digest.is_empty());
    assert_eq!(
        contract_autorepair::build_contract_autorepair_instructions(&source, &ir, &context),
        instructions,
        "同输入两次构建全等（确定性）"
    );
    assert_eq!(instruction.rule_version, "contract_autorepair/v1");

    // O1 收窄：解形态为 Candidates（无精确行操作、不声明唯一解）。
    assert!(
        matches!(&instruction.solution, StructuralRepairSolution::Candidates { candidates }
            if candidates.len() == 1),
        "capability 指令必须是 Candidates 解形态：{:?}",
        instruction.solution
    );
    assert!(line_edit_operations(&instructions).is_empty());

    // required_action 为回查口径（不再携带逐字补丁行模板）。
    assert!(
        instruction.required_action.contains("回查原业务与实现承诺"),
        "required_action 必须是回查口径：{}",
        instruction.required_action
    );
    assert!(
        !instruction.required_action.contains("逐字复制"),
        "O1：不再逐字打补丁：{}",
        instruction.required_action
    );
}

/// 迁自 `sg_02_no_fake_edge_removal_unknown_key_big_code_heading_never_
/// deleted`（改写：typed heading 与未知 key 混合集合零指令；纯 heading
/// 缺口同样零指令）。
#[test]
fn sg_02_unknown_key_big_code_heading_never_deleted() {
    let context = source_context();
    let mixed = unknown_structured_key_candidate().replacen(
        "### Blockers\n",
        "## 验收说明\n### Blockers\n",
        1,
    );
    let diagnostics =
        compile_work_item_plan(&mixed, &context).expect_err("heading + unknown key 必须编译失败");
    assert!(
        contract_autorepair::build_compile_failure_repair_instructions(
            &mixed,
            &diagnostics,
            &context
        )
        .is_empty(),
        "typed heading 与未知 key 混合集合整体 fail-closed：零指令"
    );
    assert!(
        contract_autorepair::converge_work_item_plan_source(&mixed, context.clone()).is_err(),
        "heading 错误不经本地收敛"
    );

    // 纯 heading 缺口：同样零指令（不删业务行）。
    let heading_only =
        clean_candidate().replacen("### Blockers\n", "## 验收说明\n### Blockers\n", 1);
    let heading_diagnostics = compile_work_item_plan(&heading_only, &context)
        .expect_err("pure heading fixture must fail compile");
    assert!(
        contract_autorepair::build_compile_failure_repair_instructions(
            &heading_only,
            &heading_diagnostics,
            &context
        )
        .is_empty(),
        "typed heading 诊断不产生任何指令"
    );
}

// ===================== engine 测试夹具（与 single_candidate 测试同源） =====================

/// 驱动 complete_single_candidate_work_item_plan_author 需要的 durable 形态
/// （与 tests/single_candidate/contract_prerevision.rs::author_round_record
/// 同款：Generate 相位、无候选 refs 首轮流）。
fn author_round_record(lifecycle: &LifecycleStore, engine: &mut WorkspaceEngine) {
    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("load session");
    record.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
    record.run_policy = RunPolicy::Interactive;
    record.single_candidate_phase = Some(SingleCandidatePhase::Generate);
    record.work_item_plan_source_revision_ref = None;
    record.plan_candidate_ir_ref = None;
    record.mechanical_report_ref = None;
    write_json(
        &lifecycle
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist author round session");
    engine.session = WorkspaceSession::from_record(record);
}

fn persisted_source(lifecycle: &LifecycleStore, engine: &WorkspaceEngine) -> String {
    let scope = engine.session();
    let store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
    let source_ref = scope
        .work_item_plan_source_revision_ref
        .as_deref()
        .expect("durable source ref");
    store
        .get_source_revision(
            &crate::product::work_item_plan_source_store::SourceStoreScope {
                project_id: scope.project_id.clone(),
                issue_id: scope.issue_id.clone(),
                plan_id: scope.entity_id.clone(),
            },
            source_ref,
        )
        .expect("reload persisted source")
        .source
}
