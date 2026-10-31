// 从 single_candidate.rs 移出（large_file_guard 登记上限锁定，纯移动）：
// r41/r45 可教解析失败与 IR 教学重驱提示词的定向用例。
use super::*;

/// r41 B2:unknown_structured_key 属可教面(一次自愈机会,F-48 先例)。
#[test]
fn unknown_structured_key_is_teachable() {
    assert!(is_teachable_parse_failure("unknown_structured_key"));
    assert!(is_teachable_parse_failure("missing_section"));
    assert!(!is_teachable_parse_failure("internal_error"));
}

/// r41 A2:IR 重驱 prompt 附已登记需求编号清单与约束指令。
#[test]
fn ir_reredrive_prompt_appends_registered_requirement_ids() {
    let reasons = vec![
        "unknown_requirement_ref: task references unknown design requirement NFR-1".to_string(),
    ];
    let ids = vec!["REQ-ENV-01".to_string(), "NFR-PERF-02".to_string()];
    let prompt = build_work_item_plan_ir_reredrive_prompt(&reasons, &ids);
    assert!(prompt.contains("[design_requirements] REQ-ENV-01、NFR-PERF-02"));
    assert!(prompt.contains("清单外编号一律删除或改为清单内编号"));
}

/// r45 扩 AC:done_when 的 AC 引用清单外被拒(第五变体锚点:
/// AC-002/AC-003 编造;产品 unknown_done_when_ref 只查 WI 内自洽)。
#[test]
fn ir_reredrive_prompt_lists_ac_namespace() {
    let reasons = vec![
        "unknown_done_when_ref: task references unknown acceptance criterion AC-002".to_string(),
    ];
    let ids = vec!["REQ-ENV-01".to_string(), "AC-001".to_string()];
    let prompt = build_work_item_plan_ir_reredrive_prompt(&reasons, &ids);
    assert!(prompt.contains("AC-001"));
    assert!(
        prompt.contains("done_when 的引用只能取自上列清单"),
        "清单约束必须覆盖 done_when/AC 命名空间: {prompt}"
    );
}

/// r41 A2:清单空时不注入(无 spec 上下文的老会话零变化)。
#[test]
fn ir_reredrive_prompt_omits_empty_requirement_list() {
    let reasons = vec!["acceptance_criterion_without_reviewer_check: WI-001 AC-1".to_string()];
    let prompt = build_work_item_plan_ir_reredrive_prompt(&reasons, &[]);
    assert!(!prompt.contains("[design_requirements]"));
}
