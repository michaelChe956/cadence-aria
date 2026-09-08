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
