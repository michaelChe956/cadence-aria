use super::*;
use crate::product::cadence_skills::routing_reference::RoutingReferenceContext;

#[test]
fn coding_plan_repair_prompt_contracts_require_canonical_schema_and_legacy_mapping() {
    let attempt = test_attempt("coding_attempt_prompt_contract");
    let context = CodingExecutionContext::default();
    let coding_prompt = build_coding_prompt(
        &attempt,
        &context,
        None,
        None,
        &RoutingReferenceContext::Legacy,
    );
    assert_plan_defect_output_contract(&coding_prompt, "plan_defect_findings");
    assert!(coding_prompt.contains("普通 implementation defect"));

    for prompt in [
        code_review_material_protocol(&RoutingReferenceContext::Legacy),
        group_final_review_material_protocol(&RoutingReferenceContext::Legacy),
    ] {
        assert_plan_defect_output_contract(&prompt, "findings");
        assert!(prompt.contains("普通 implementation defect"));
    }
}

#[test]
fn plan_defect_output_contract_declares_field_value_constraints() {
    let contract = crate::product::plan_repair::plan_defect_structured_output_contract();

    assert!(contract.contains("severity 只能使用 error、warning"));
    assert!(contract.contains("confidence 只能使用 low、medium、high"));
    assert!(contract.contains("repair_target 必须是对象"));
    assert!(contract.contains("logical_work_item_ids"));
    assert!(contract.contains("work_item_revision_ids"));
}

/// 3.6 矩阵族③根因修复（A 件）：契约文案中 defect_class 取值清单与
/// `PlanDefectClass` 的 serde 变体名必须双向逐字一致（仿 work_item_split_engine
/// 的 weak_model_precision 教学↔校验器对齐断言先例）：
/// 1) 契约→枚举：契约枚举行里的每个取值都必须能反序列化为合法 `PlanDefectClass`
///    （教学不得发明取值）；
/// 2) 枚举→契约：serde 自身的 unknown variant 错误清单（`expected one of ...`）
///    列出的每个合法变体名都必须逐字出现在契约里（枚举加变体/改名时测试必红，
///    防止教学与 schema 漂移——族③现场即弱模型输出 defect_class=plan_defect
///    被拒后无教学可依）；
/// 3) 契约的取值个数声明（「8 个取值」）与实际枚举数一致。
#[test]
fn plan_defect_output_contract_defect_class_values_align_with_serde_variants_bidirectionally() {
    use crate::product::models::PlanDefectClass;

    let contract = crate::product::plan_repair::plan_defect_structured_output_contract();

    // 抽取契约中的 defect_class 逐字枚举清单（
    // 「- defect_class 只能逐字使用以下 N 个取值之一：a、b、…；」
    // 到行尾「；」为止）。
    let prefix = "- defect_class 只能逐字使用以下 ";
    let start = contract
        .find(prefix)
        .expect("契约必须包含 defect_class 取值枚举行");
    let enumeration_start = start
        + contract[start..]
            .find("：")
            .expect("枚举行必须以全角冒号引出取值清单")
        + "：".len();
    let enumeration_end = enumeration_start
        + contract[enumeration_start..]
            .find("；")
            .expect("取值清单必须以全角分号收尾");
    let taught_values = contract[enumeration_start..enumeration_end]
        .split('、')
        .map(str::trim)
        .collect::<Vec<_>>();
    assert!(!taught_values.is_empty(), "取值清单不得为空: {contract}");

    // 方向 1：契约枚举的每个取值都必须是合法 serde 变体（教学不得发明取值）。
    for taught in &taught_values {
        let parsed = serde_json::from_str::<PlanDefectClass>(&format!("\"{taught}\""));
        assert!(
            parsed.is_ok(),
            "契约枚举的 defect_class 取值 {taught} 必须能反序列化为 PlanDefectClass 合法变体"
        );
    }

    // 方向 2：serde 的 unknown variant 错误清单列出全部合法变体名，逐一要求
    // 逐字出现在契约里（枚举新增变体时该清单自动变化，测试必红）。
    let unknown_variant_error =
        serde_json::from_str::<PlanDefectClass>("\"__definitely_not_a_plan_defect_class__\"")
            .expect_err("未知变体必须被 serde 拒绝");
    let error_text = unknown_variant_error.to_string();
    let expected_list_start = error_text
        .find("expected one of ")
        .expect("serde unknown variant 错误必须携带 expected one of 清单")
        + "expected one of ".len();
    let expected_list = error_text[expected_list_start..]
        .split(" at line")
        .next()
        .unwrap_or(&error_text[expected_list_start..]);
    let serde_variant_names = expected_list
        .split(", ")
        .map(|name| name.trim_matches('`'))
        .collect::<Vec<_>>();
    assert_eq!(
        serde_variant_names.len(),
        taught_values.len(),
        "契约取值清单与 serde 变体清单必须同长: serde={serde_variant_names:?} 契约={taught_values:?}"
    );
    for variant in &serde_variant_names {
        assert!(
            taught_values.contains(variant),
            "契约 defect_class 取值清单必须逐字包含 serde 变体 {variant}: {contract}"
        );
    }

    // 方向 3：取值个数声明与实际清单一致（同步教学文案里的数字）。
    let declared_count = contract[start..enumeration_start]
        .trim_start_matches(prefix)
        .trim_end_matches('：')
        .trim_end_matches("个取值之一")
        .trim();
    assert_eq!(
        declared_count,
        taught_values.len().to_string(),
        "枚举行的取值个数声明必须与实际取值数一致"
    );
}

/// 3.6 矩阵族④（route×target 矩阵）根因修复：契约必须逐字枚举
/// 「defect_class → recommended_route → repair_target」合法矩阵，且该矩阵与
/// 校验器的真实判决双向对齐（教学不得臆造、不得与 schema 漂移）。现场 7 例
/// （kimi×3 / claude×1：`route OperationalGate does not accept target
/// CurrentWorkItem`；`route VerificationRetry does not accept target
/// CurrentWorkItem`；`unknown variant \`OperationalGate\`` 大写枚举名；
/// pi：`defect class DependencyGraphInvalid requires target Subgraph, got
/// CurrentWorkItem`）证明：仅枚举 defect_class 8 取值不足，模型从未见过
/// 合法 route×target 组合，恰一次教学重驱后仍重犯。
///
/// 断言分三层，全部从校验器真实行为派生（非文案硬编码）：
/// 1) 变体清单经 serde unknown variant 的 `expected one of` 反射获取；
/// 2) 每个 defect_class 的合法 route 取 `default_route`，合法 target 配置由
///    逐候选跑 `validate()` 的真实判决确定（恰一种），再要求契约逐字包含
///    派生出的 `class → route → target` 行——校验器改判决而契约未同步即红；
/// 3) 反例必须逐字回灌校验器实际产出的错误原文（大写枚举名回显、无 target
///    路线硬塞 target、kind 错配、省略 target、空 id 列表、容器字段名）。
#[test]
fn plan_defect_output_contract_route_target_matrix_aligns_with_validator() {
    use crate::product::plan_repair::{
        PlanDefectClass, PlanDefectConfidence, PlanDefectFinding, PlanDefectSeverity,
        PlanRepairError, RepairTargetKind, default_route,
    };

    fn probe(
        defect_class: &str,
        route: &str,
        target: Option<&RepairTargetKind>,
        target_ids_present: bool,
    ) -> Result<(), PlanRepairError> {
        let repair_target = target
            .map(|kind| {
                serde_json::json!({
                    "kind": serde_json::to_value(kind).expect("kind serde"),
                    "logical_work_item_ids": if target_ids_present { vec!["WI-1"] } else { Vec::<&str>::new() },
                    "work_item_revision_ids": if target_ids_present { vec!["WIR-1"] } else { Vec::<&str>::new() },
                })
            })
            .map(|value| serde_json::from_value(value).expect("repair target serde"));
        let finding = PlanDefectFinding {
            finding_id: "finding-1".to_string(),
            severity: PlanDefectSeverity::Error,
            defect_class: serde_json::from_str(&format!("\"{defect_class}\""))
                .expect("defect class"),
            reason_code: "reason".to_string(),
            message: "message".to_string(),
            evidence: Vec::new(),
            contract_refs: Vec::new(),
            capability_refs: Vec::new(),
            repair_target,
            recommended_route: serde_json::from_str(&format!("\"{route}\""))
                .expect("recommended route"),
            confidence: PlanDefectConfidence::Low,
        };
        finding.validate()
    }

    fn invalid_repair_target_message(
        class: &str,
        route: &str,
        target: &RepairTargetKind,
    ) -> String {
        match probe(class, route, Some(target), true) {
            Err(PlanRepairError::InvalidRepairTarget(message)) => message,
            other => {
                panic!("{class}×{route}×{target:?} 必须被 InvalidRepairTarget 拒绝: {other:?}")
            }
        }
    }

    let contract = crate::product::plan_repair::plan_defect_structured_output_contract();

    // 变体清单反射：serde 的 expected one of 清单即 schema 真源。
    let unknown_class = serde_json::from_str::<PlanDefectClass>("\"__not_a_defect_class__\"")
        .expect_err("未知 defect_class 必须被 serde 拒绝")
        .to_string();
    let class_list_start = unknown_class
        .find("expected one of ")
        .expect("unknown variant 错误必须携带 expected one of")
        + "expected one of ".len();
    let class_list = unknown_class[class_list_start..]
        .split(" at line")
        .next()
        .unwrap_or(&unknown_class[class_list_start..]);
    let class_names = class_list
        .split(", ")
        .map(|name| name.trim_matches('`'))
        .collect::<Vec<_>>();
    assert!(!class_names.is_empty(), "defect_class 变体清单不得为空");

    for class_name in &class_names {
        let parsed_class: PlanDefectClass =
            serde_json::from_str(&format!("\"{class_name}\"")).expect("合法 defect_class");
        let route = default_route(&parsed_class);
        let route_name = serde_json::to_value(&route)
            .expect("route serde")
            .as_str()
            .expect("route 必须是字符串枚举")
            .to_string();

        // 行为探测：候选配置（null + 三个 kind）中真实合法的必须恰一种。
        let candidates = [
            None,
            Some(RepairTargetKind::CurrentWorkItem),
            Some(RepairTargetKind::UpstreamWorkItem),
            Some(RepairTargetKind::Subgraph),
        ];
        let legal = candidates
            .iter()
            .filter(|candidate| probe(class_name, &route_name, candidate.as_ref(), true).is_ok())
            .collect::<Vec<_>>();
        assert_eq!(
            legal.len(),
            1,
            "{class_name} 必须恰有一种合法 repair_target 配置（校验器判决）: {legal:?}"
        );
        let expected_row = match legal[0] {
            None => format!("{class_name} → {route_name} → repair_target=null"),
            Some(kind) => {
                let kind_name = serde_json::to_value(kind)
                    .expect("kind serde")
                    .as_str()
                    .expect("kind 必须是字符串枚举")
                    .to_string();
                format!("{class_name} → {route_name} → repair_target.kind={kind_name}")
            }
        };
        assert!(
            contract.contains(&expected_row),
            "契约必须逐字枚举校验器认可的合法组合 {expected_row}: {contract}"
        );
    }

    // 反例 1：Rust 变体名（首字母大写）不得当作 JSON 取值；大写的 route 名正是
    // 现场 kimi 的 `unknown variant \`OperationalGate\``。错误原文中的
    // `route OperationalGate ...` 是 Rust 变体名回显，JSON 必须写 snake_case。
    let gate_route = default_route(&PlanDefectClass::OperationalBlocker);
    assert!(
        contract.contains(&format!("反例：recommended_route={gate_route:?}")),
        "契约必须逐字给出大写 route 反例 recommended_route={gate_route:?}: {contract}"
    );
    assert!(
        contract.contains("必须写 operational_gate"),
        "契约必须指明错误回显中的大写名为 Rust 变体名、JSON 写 snake_case: {contract}"
    );
    assert!(
        contract.contains("禁止大写枚举名或容器字段名"),
        "契约必须显式禁止大写枚举名与容器字段名当取值: {contract}"
    );

    // 反例 2：无 target 路线硬塞 target（现场两例：operational_gate 与
    // verification_retry 配 current_work_item），逐字回灌校验器错误原文。
    for (class, route, kind) in [
        (
            "operational_blocker",
            "operational_gate",
            RepairTargetKind::CurrentWorkItem,
        ),
        (
            "verification_incomplete",
            "verification_retry",
            RepairTargetKind::CurrentWorkItem,
        ),
    ] {
        let message = invalid_repair_target_message(class, route, &kind);
        assert!(
            contract.contains(&message),
            "契约必须逐字回灌反例错误原文 {message}: {contract}"
        );
    }

    // 反例 3：kind 错配（现场 pi 例）+ 省略 target 必需项。
    for (class, route, kind) in [
        (
            "dependency_graph_invalid",
            "plan_repair",
            RepairTargetKind::CurrentWorkItem,
        ),
        (
            "current_work_item_invalid",
            "plan_repair",
            RepairTargetKind::UpstreamWorkItem,
        ),
    ] {
        let message = invalid_repair_target_message(class, route, &kind);
        assert!(
            contract.contains(&message),
            "契约必须逐字回灌错配反例错误原文 {message}: {contract}"
        );
    }
    let missing_target = match probe("dependency_graph_invalid", "plan_repair", None, true) {
        Err(PlanRepairError::InvalidRepairTarget(message)) => message,
        other => panic!("省略必需 target 必须被 InvalidRepairTarget 拒绝: {other:?}"),
    };
    assert!(
        contract.contains(&missing_target),
        "契约必须逐字回灌省略 target 的错误原文 {missing_target}: {contract}"
    );
    let empty_ids = match probe(
        "dependency_graph_invalid",
        "plan_repair",
        Some(&RepairTargetKind::Subgraph),
        false,
    ) {
        Err(PlanRepairError::InvalidRepairTarget(message)) => message,
        other => panic!("空 id 列表必须被 InvalidRepairTarget 拒绝: {other:?}"),
    };
    assert!(
        contract.contains(&empty_ids),
        "契约必须逐字回灌空 id 列表的错误原文 {empty_ids}: {contract}"
    );

    // 反例 4：human_triage 不在 finding 可输出的 route 集合内（serde 合法但
    // 校验器拒绝），错误原文逐字回灌。
    let human_triage = match probe("implementation_defect", "human_triage", None, true) {
        Err(PlanRepairError::InvalidFinding(message)) => message,
        other => panic!("human_triage 必须被 InvalidFinding 拒绝: {other:?}"),
    };
    assert!(
        contract.contains(&human_triage),
        "契约必须逐字回灌 human_triage 反例错误原文 {human_triage}: {contract}"
    );
}

pub(super) fn assert_plan_defect_output_contract(prompt: &str, container: &str) {
    assert!(prompt.contains(container), "missing {container}: {prompt}");
    for field in [
        "defect_class",
        "reason_code",
        "contract_refs",
        "capability_refs",
        "repair_target",
        "recommended_route",
        "confidence",
        "evidence",
    ] {
        assert!(prompt.contains(field), "missing {field}: {prompt}");
    }
}
