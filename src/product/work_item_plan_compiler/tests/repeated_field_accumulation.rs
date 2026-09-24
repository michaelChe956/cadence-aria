//! lowering 重复字段累积语义契约(TDD)——REQ-WSC-02 场景 6-10 / F-52 P0。
//!
//! 现场依据(F-52):issue_0002 / workspace_session_0009 的 CT-001 在真实交付中有
//! 6 行 `- capabilities:`,而 `lower_outputs` 对重复行做 last-write-wins 覆盖
//! (`lower_inputs` 的 `required_capabilities` 同源),canonical 契约只剩最后 1 行
//! 逗号切分值,`missing_capabilities` 精确集合差因此产生假缺口,reviewer 按硬规则
//! 被迫 must_fix(R3/R5/R7 假阳性工厂)。
//!
//! 契约:同一 contract 的重复 `capabilities`/`required_capabilities` 行按出现
//! 顺序累积为全体行 split 值的并集,完全相同(字节级)的值仅保留首次出现;contract
//! 边界 flush 严格隔离;值形态零改写(不排序、不改大小写/标点/内部空白)。真实缺
//! 能力仍报 Error——累积只消除重复,不制造能力。
//!
//! fixture 提取纪律(承 F-46/F-48 教训):python `json.load` 读 durable
//! `artifact_versions.json` 的 v3 markdown 后逐字节写入,不得经 `jq -r`。

use crate::product::work_item_contract::{
    ContractFindingSeverity, build_dependency_contract_graph, project_contract_capability_coverage,
    validate_dependency_contract_graph,
};
use crate::product::work_item_plan_compiler::{
    PlanCandidateIr, PlanCandidateItemIr, WORK_ITEM_PLAN_COMPILER_VERSION,
    WorkItemPlanSourceContext, compile_work_item_plan, normalize_delivery_before_compile,
};

/// F-52 durable 原文(v3,sha256 前缀 340fe709)逐字节提取件。
const F52_PLAN_CT001_FIELD_V3: &str = include_str!("../fixtures/f52_plan_ct001_field_v3.md");

/// F-52 CT-001 的 6 行复合/独立能力混排(v1/v3 族共同形态;v1 的第 6 行为
/// 逗号复合行,拆分后与第 5 行字节级重复)。
const F52_CT001_SIX_CAPABILITY_LINES: &str = "\
- capabilities: judge(board) 返回结构化判定结果对象
- capabilities: 四个方向连成五子及以上判定为获胜并给出获胜连线坐标
- capabilities: 满盘且无五连判定为平局
- capabilities: 被阻断的四连与不足五连判定为进行中
- capabilities: CommonJS 与浏览器全局暴露同一个 judge 函数
- capabilities: 仓库根存在可供静态托管的 gomoku-rules.js 文件, CommonJS 与浏览器全局暴露同一个 judge 函数";

/// 最小合法单 Work Item 计划;Inputs/Outputs 段内容按需注入。
fn minimal_plan(inputs: &str, outputs: &str) -> String {
    let inputs = if inputs.is_empty() {
        String::new()
    } else {
        format!("{inputs}\n")
    };
    format!(
        "# Work Item Plan\n\
         \n\
         ## Work Item WI-001: Lowering accumulation fixture\n\
         \n\
         ### Identity\n\
         - schema_version: 1\n\
         - logical_work_item_id: WI-001\n\
         - title: Lowering accumulation fixture\n\
         - kind: backend\n\
         \n\
         ### Goal\n\
         - summary: WHEN the lowering fixture runs THE SYSTEM SHALL provide a valid backend work item.\n\
         \n\
         ### Non Goals\n\
         - non_goals: Browser rendering is out of scope.\n\
         \n\
         ### Dependencies\n\
         - depends_on: []\n\
         \n\
         ### Inputs\n\
         {inputs}\
         ### Outputs\n\
         {outputs}\n\
         ### Tasks\n\
         - task_id: TASK-001\n\
         - statement: WHEN GET /api/levels is requested THE SYSTEM SHALL return configured levels.\n\
         - requirement_refs: REQ-WSC-02\n\
         - done_when_refs: AC-001\n\
         \n\
         ### Write Policy\n\
         - exclusive_scopes: src/product/levels/**\n\
         - forbidden_scopes: web/**\n\
         \n\
         ### Acceptance Criteria\n\
         - criterion_id: AC-001\n\
         - statement: WHEN GET /api/levels is requested THE SYSTEM SHALL return configured levels.\n\
         - required_evidence: source_diff\n\
         - required_evidence: non_zero_test_execution\n\
         \n\
         ### Verification\n\
         - check_id: CHECK-001\n\
         - command: cargo test --locked --lib levels_api\n\
         - manual_instruction: Confirm the endpoint returns the configured levels JSON.\n\
         - required: true\n\
         - non_zero_test_execution_required: true\n\
         \n\
         ### Handoff Schema\n\
         - required_fields: commit_sha\n\
         - provided_contract_refs: contract.levels-api\n\
         - reviewer_check_refs: AC-001\n\
         \n\
         ### Blockers\n\
         - reason_code: levels_api_contract_invalid\n\
         - route: plan_repair_current\n\
         - target_contract_refs: contract.levels-api\n\
         \n\
         ### Traceability\n\
         - source_type: design_spec\n\
         - source_id: design_spec_levels_0001\n\
         - requirement_id: REQ-WSC-02\n"
    )
}

fn compile_plan(source: &str) -> PlanCandidateIr {
    compile_work_item_plan(
        source,
        &WorkItemPlanSourceContext {
            target_repository_id: "repository_0001".to_string(),
        },
    )
    .expect("测试计划必须 lower 为 typed IR")
}

fn compile_fixture(source: &str) -> PlanCandidateIr {
    let normalized = normalize_delivery_before_compile(source);
    compile_work_item_plan(
        &normalized.source,
        &WorkItemPlanSourceContext {
            target_repository_id: "repository_0001".to_string(),
        },
    )
    .expect("fixture 必须经生产归一化链路 lower 为 typed IR")
}

fn output_capabilities(ir: &PlanCandidateIr, contract_id: &str) -> Vec<String> {
    ir.items[0]
        .contract
        .output_contracts
        .iter()
        .find(|output| output.contract_id == contract_id)
        .unwrap_or_else(|| panic!("Outputs 必须包含 {contract_id}"))
        .capabilities
        .clone()
}

fn input_required_capabilities(ir: &PlanCandidateIr, contract_id: &str) -> Vec<String> {
    ir.items[0]
        .contract
        .input_contracts
        .iter()
        .find(|input| input.contract_id == contract_id)
        .unwrap_or_else(|| panic!("Inputs 必须包含 {contract_id}"))
        .required_capabilities
        .clone()
}

fn coverage_entries(
    ir: &PlanCandidateIr,
) -> Vec<crate::product::work_item_contract::ContractCapabilityCoverage> {
    let contracts = ir
        .items
        .iter()
        .map(|item| item.contract.clone())
        .collect::<Vec<_>>();
    let graph = build_dependency_contract_graph(&contracts)
        .expect("fixture IR 必须构建 dependency contract graph");
    project_contract_capability_coverage(&graph)
}

fn graph_errors(ir: &PlanCandidateIr) -> Vec<String> {
    let contracts = ir
        .items
        .iter()
        .map(|item| item.contract.clone())
        .collect::<Vec<_>>();
    let graph = build_dependency_contract_graph(&contracts)
        .expect("fixture IR 必须构建 dependency contract graph");
    validate_dependency_contract_graph(&graph)
        .findings
        .into_iter()
        .filter(|finding| finding.severity == ContractFindingSeverity::Error)
        .map(|finding| format!("{}:{}", finding.code, finding.message))
        .collect()
}

/// 场景 6:同一 Outputs contract 的 N 行 capabilities(复合/独立混排)累积为
/// 全体行 split 值的并集,不得 last-write-wins。
#[test]
fn scenario6_repeated_capability_lines_accumulate_union_in_order() {
    let ir = compile_plan(&minimal_plan(
        "",
        &format!("- contract_id: CT-001\n{F52_CT001_SIX_CAPABILITY_LINES}"),
    ));
    assert_eq!(
        output_capabilities(&ir, "CT-001"),
        vec![
            "judge(board) 返回结构化判定结果对象",
            "四个方向连成五子及以上判定为获胜并给出获胜连线坐标",
            "满盘且无五连判定为平局",
            "被阻断的四连与不足五连判定为进行中",
            "CommonJS 与浏览器全局暴露同一个 judge 函数",
            "仓库根存在可供静态托管的 gomoku-rules.js 文件",
        ]
    );
}

/// 场景 6:完全相同的 capability 值稳定去重,仅保留首次出现。
#[test]
fn scenario6_identical_duplicate_values_keep_first_seen_only() {
    let ir = compile_plan(&minimal_plan(
        "",
        "- contract_id: CT-001\n\
         - capabilities: alpha.read\n\
         - capabilities: alpha.read\n\
         - capabilities: alpha.read\n",
    ));
    assert_eq!(output_capabilities(&ir, "CT-001"), vec!["alpha.read"]);
}

/// 场景 7:同一 Inputs contract 的多行 required_capabilities 与 capabilities
/// 同构累积并集,不得 last-write-wins。
#[test]
fn scenario7_repeated_required_capability_lines_accumulate_isomorphically() {
    let ir = compile_plan(&minimal_plan(
        "- contract_id: CT-001\n\
         - provider_logical_work_item_id: WI-001\n\
         - required_capabilities: alpha.read\n\
         - required_capabilities: beta.write, alpha.read\n\
         - required_capabilities: alpha.read\n\
         - compatibility_policy: require_all",
        "- contract_id: CT-OUT\n\
         - capabilities: unused",
    ));
    assert_eq!(
        input_required_capabilities(&ir, "CT-001"),
        vec!["alpha.read", "beta.write"]
    );
}

/// 场景 8:相邻 contract 在 contract 边界严格 flush,字段不得跨 contract 归并
/// 或串项;首个 contract_id 之前的错位字段行不归给任何 contract。
#[test]
fn scenario8_adjacent_contracts_flush_strictly_at_boundaries() {
    let ir = compile_plan(&minimal_plan(
        "",
        "- capabilities: stray.before-any-contract\n\
         - contract_id: CT-ALPHA\n\
         - capabilities: alpha.one\n\
         - capabilities: alpha.two\n\
         - contract_id: CT-BETA\n\
         - capabilities: beta.one\n",
    ));
    let outputs = &ir.items[0].contract.output_contracts;
    assert_eq!(outputs.len(), 2, "两个 contract_id 必须各自成 entry");
    assert_eq!(
        output_capabilities(&ir, "CT-ALPHA"),
        vec!["alpha.one", "alpha.two"]
    );
    assert_eq!(output_capabilities(&ir, "CT-BETA"), vec!["beta.one"]);
    for output in outputs {
        assert!(
            !output
                .capabilities
                .contains(&"stray.before-any-contract".to_string()),
            "错位字段不得归给任何 contract:{:?}",
            output.capabilities
        );
    }
}

/// 负向:`[]` 空值保持既有「无能力」语义;未知结构化 key 在 parse 层
/// fail-closed 拒绝,不会流入 lowering。
#[test]
fn empty_list_value_and_unknown_key_stay_fail_closed() {
    let ir = compile_plan(&minimal_plan(
        "",
        "- contract_id: CT-001\n\
         - capabilities: []\n\
         - contract_id: CT-002\n\
         - capabilities: beta.real\n",
    ));
    assert_eq!(output_capabilities(&ir, "CT-001"), Vec::<String>::new());
    assert_eq!(output_capabilities(&ir, "CT-002"), vec!["beta.real"]);

    let error = compile_work_item_plan(
        &minimal_plan(
            "",
            "- contract_id: CT-001\n\
             - capabilities: alpha.read\n\
             - colour: red\n",
        ),
        &WorkItemPlanSourceContext {
            target_repository_id: "repository_0001".to_string(),
        },
    )
    .expect_err("未知结构化 key 必须 fail-closed");
    assert!(
        error
            .iter()
            .any(|diagnostic| diagnostic.code == "unknown_structured_key"),
        "必须返回 unknown_structured_key 诊断:{error:?}"
    );
}

/// 场景 9:累积与去重不改写 capability 值原文——顺序、大小写、标点、内部
/// 空白零触碰;大小写不同即不同值,不去重。
#[test]
fn scenario9_accumulation_preserves_capability_value_shape_byte_for_byte() {
    let ir = compile_plan(&minimal_plan(
        "",
        "- contract_id: CT-001\n\
         - capabilities: alpha.Keep CASE,  beta.with  double  spaces\n\
         - capabilities: gamma.「全角、标点」(U+300D)收尾\n\
         - capabilities: alpha.Keep CASE\n\
         - capabilities: commonjs\n\
         - capabilities: CommonJS\n",
    ));
    assert_eq!(
        output_capabilities(&ir, "CT-001"),
        vec![
            "alpha.Keep CASE",
            "beta.with  double  spaces",
            "gamma.「全角、标点」(U+300D)收尾",
            "commonjs",
            "CommonJS",
        ]
    );
}

/// 场景 6(fixture 端到端):F-52 真实 6 行形态经生产归一化链路编译后,
/// capability coverage 不再产生 missing_capabilities 假阳性,dependency
/// graph 校验无 required_capability_missing Error。
#[test]
fn f52_ct001_field_fixture_no_longer_reports_missing_capabilities() {
    let ir = compile_fixture(F52_PLAN_CT001_FIELD_V3);

    let missing: Vec<String> = coverage_entries(&ir)
        .into_iter()
        .flat_map(|entry| entry.missing_capabilities)
        .collect();
    assert!(
        missing.is_empty(),
        "F-52 真实形态不得再有 missing_capabilities 假阳性:{missing:?}"
    );

    let errors = graph_errors(&ir);
    let capability_errors = errors
        .iter()
        .filter(|message| message.starts_with("required_capability_missing"))
        .collect::<Vec<_>>();
    assert!(
        capability_errors.is_empty(),
        "不得再有 required_capability_missing Error:{capability_errors:?}"
    );
}

/// 场景 6 负例(fixture):删除一行真实能力后,累积语义不得把它「救」回来——
/// coverage 仍报缺口,graph 校验仍返回 required_capability_missing Error。
#[test]
fn f52_fixture_removing_real_capability_stays_error() {
    let removed = "- capabilities: 满盘且无五连判定为平局";
    assert_eq!(
        F52_PLAN_CT001_FIELD_V3.matches(removed).count(),
        1,
        "被删能力行必须在 fixture 中恰好出现一次"
    );
    let mutated = F52_PLAN_CT001_FIELD_V3.replacen(removed, "", 1);
    let ir = compile_fixture(&mutated);

    let missing: Vec<String> = coverage_entries(&ir)
        .into_iter()
        .flat_map(|entry| entry.missing_capabilities)
        .collect();
    assert_eq!(
        missing,
        vec!["满盘且无五连判定为平局"],
        "删一行真实能力后缺口必须恰好是该能力"
    );
    let errors = graph_errors(&ir);
    assert!(
        errors.iter().any(
            |message| message.starts_with("required_capability_missing:")
                && message.contains("满盘且无五连判定为平局")
        ),
        "必须仍返回 required_capability_missing Error:{errors:?}"
    );
}

/// 递归收集两个 JSON 值的叶级差异路径(数组长度不同时记数组路径本身)。
fn json_diff_paths<'a>(
    left: &'a serde_json::Value,
    right: &'a serde_json::Value,
    path: String,
    out: &mut Vec<String>,
) {
    match (left, right) {
        (serde_json::Value::Object(l), serde_json::Value::Object(r)) => {
            let mut keys: Vec<&str> = l.keys().map(String::as_str).collect();
            for key in r.keys() {
                if !l.contains_key(key) {
                    keys.push(key);
                }
            }
            for key in keys {
                let child_path = format!("{path}/{key}");
                match (l.get(key), r.get(key)) {
                    (Some(lv), Some(rv)) => json_diff_paths(lv, rv, child_path, out),
                    _ => out.push(child_path),
                }
            }
        }
        (serde_json::Value::Array(l), serde_json::Value::Array(r)) => {
            if l.len() == r.len() {
                for (index, (lv, rv)) in l.iter().zip(r.iter()).enumerate() {
                    json_diff_paths(lv, rv, format!("{path}/{index}"), out);
                }
            } else {
                out.push(path);
            }
        }
        _ => {
            if left != right {
                out.push(path);
            }
        }
    }
}

/// 场景 10:含重复字段的相同 source 在「修复前(last-write-wins 合成 IR)」与
/// 「修复后(实际编译 IR)」的差异仅体现为重复字段的累积并集;顶层
/// source_revision_hash 与 compiler_version 不变(freshness 链零触碰)。
#[test]
fn scenario10_recompiled_ir_diff_confines_to_repeated_field_semantics() {
    let ir = compile_fixture(F52_PLAN_CT001_FIELD_V3);
    assert_eq!(ir.compiler_version, WORK_ITEM_PLAN_COMPILER_VERSION);

    // 合成「修复前」IR:全部多行重复字段回退为「仅最后一行的 split 值」,
    // 其余字段与实际编译结果共用同一对象树,保证差异只可能来自这些字段。
    let mut pre_fix = ir.clone();
    fn last_line_only(contract_field: &mut Vec<String>) {
        if let Some(last) = contract_field.pop() {
            contract_field.clear();
            contract_field.push(last);
        }
    }
    let pre_fix_ref: &mut Vec<PlanCandidateItemIr> = &mut pre_fix.items;
    for item in pre_fix_ref.iter_mut() {
        for output in item.contract.output_contracts.iter_mut() {
            last_line_only(&mut output.capabilities);
        }
        for input in item.contract.input_contracts.iter_mut() {
            last_line_only(&mut input.required_capabilities);
        }
    }
    assert_ne!(pre_fix, ir, "累积语义必须改变含重复字段的 IR");

    let left = serde_json::to_value(&pre_fix).expect("pre-fix IR 必须可序列化");
    let right = serde_json::to_value(&ir).expect("post-fix IR 必须可序列化");
    let mut paths = Vec::new();
    json_diff_paths(&left, &right, String::new(), &mut paths);
    paths.sort();
    assert_eq!(
        paths,
        vec![
            "/items/0/contract/output_contracts/0/capabilities",
            "/items/1/contract/input_contracts/0/required_capabilities",
            "/items/1/contract/output_contracts/0/capabilities",
            "/items/2/contract/output_contracts/0/capabilities",
        ],
        "IR 差异必须恰好源于 4 处重复字段(WI-001 CT-001 输出、WI-002 CT-001 输入、\
         WI-002 CT-002 输出、WI-003 CT-003 输出):{paths:?}"
    );

    // freshness 链零触碰:同一 source 的 revision hash 与编译器版本在修复前后同源。
    let digest = {
        use sha2::Digest;
        let normalized = normalize_delivery_before_compile(F52_PLAN_CT001_FIELD_V3);
        hex::encode(sha2::Sha256::digest(normalized.source.as_bytes()))
    };
    assert_eq!(ir.source_revision_hash, digest);
}
