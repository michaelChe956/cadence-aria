//! 评审提示词的审查对象：`HandoffRevision` 契约与能力，不是交接摘要的自然语言承诺。
//!
//! 对应 change `remove-work-item-handoff` 工作包 1.10、1.11。

use super::super::super::prompts::{
    code_review_material_protocol, coding_completion_report_contract,
    coding_delta_execution_protocol, coding_execution_protocol,
    group_final_review_material_protocol,
};

/// 1.10：两个协议都不得以交接摘要承诺为审查对象，也不得点名已移除字段。
#[test]
fn reviewer_protocols_do_not_review_handoff_summary_promises() {
    for (name, protocol) in [
        ("code_review", code_review_material_protocol()),
        ("group_final_review", group_final_review_material_protocol()),
    ] {
        assert!(
            !protocol.contains("handoff 承诺"),
            "{name} 协议不得要求确认交接摘要承诺是否闭环"
        );
        assert!(
            !protocol.contains("handoff 未闭环"),
            "{name} 协议不得以交接摘要未闭环作为否决理由"
        );
        assert!(
            !protocol.contains("tests_run"),
            "{name} 协议不得点名已移除的 tests_run 字段"
        );
        assert!(
            !protocol.contains("test_result_summary"),
            "{name} 协议不得点名已移除的 test_result_summary 字段"
        );
    }
}

/// 1.10：跨 unit 交接的审查对象必须是 `HandoffRevision` 的契约与能力语义。
#[test]
fn group_final_review_protocol_reviews_handoff_revision_contracts() {
    let protocol = group_final_review_material_protocol();

    assert!(
        protocol.contains("HandoffRevision"),
        "group final review 协议必须以 HandoffRevision 为跨 unit 审查对象"
    );
    assert!(
        protocol.contains("provided_contracts") || protocol.contains("契约"),
        "group final review 协议必须要求审查交接契约"
    );
    assert!(
        protocol.contains("provided_capabilities") || protocol.contains("能力"),
        "group final review 协议必须要求审查交接能力"
    );
}

/// 1.11：改写不得削弱既有否决依据与 verdict 取值口径。
#[test]
fn reviewer_protocols_retain_non_handoff_veto_grounds() {
    let code_review = code_review_material_protocol();
    let group_final_review = group_final_review_material_protocol();

    // verdict 取值口径
    for (name, protocol) in [
        ("code_review", &code_review),
        ("group_final_review", &group_final_review),
    ] {
        assert!(
            protocol.contains("verdict 只能使用 approve、request_changes、blocked"),
            "{name} 协议的 verdict 取值口径不得改变"
        );
    }

    // 验证证据类否决依据必须保留
    assert!(
        code_review.contains("缺少 required 验证命令的执行证据"),
        "code review 协议必须保留 required 验证命令缺证据的 finding 要求"
    );
    assert!(
        code_review.contains("没有实际测试被执行"),
        "code review 协议必须保留空测试不算有效覆盖的要求"
    );

    // group final review 的复合条件中，除 handoff 项外的两项必须保留
    assert!(
        group_final_review.contains("验证证据缺失"),
        "group final review 协议必须保留验证证据缺失的否决依据"
    );
    assert!(
        group_final_review.contains("Forbidden Write Scopes"),
        "group final review 协议必须保留写入范围越界检查"
    );
}

/// 1.11：Coder 侧的 TDD 与测试要求不受本变更影响。
#[test]
fn coder_protocols_retain_tdd_requirements() {
    for (name, protocol) in [
        ("coding_execution", coding_execution_protocol()),
        ("coding_delta_execution", coding_delta_execution_protocol()),
    ] {
        assert!(
            protocol.contains("test-driven-development"),
            "{name} 协议必须保留写代码前调用 test-driven-development 的要求"
        );
    }
    assert!(
        coding_execution_protocol().contains("TDD/测试要求"),
        "coding_execution 协议必须保留执行清单覆盖 TDD/测试要求"
    );
}

/// 人工事项是交接内容，不是阻塞理由。
///
/// 实测死机：Coder 因执行环境无浏览器、无法完成三项人工核对，输出
/// operational_gate blocker 并停在 stage=coding/status=running，而该决策在 coding
/// 阶段没有任何门禁落地，用户无按钮可点。人工核对本就不该由 Coder 执行。
#[test]
fn coder_protocol_excludes_manual_items_from_its_own_scope() {
    let protocol = coding_execution_protocol();
    assert!(
        protocol.contains("人工事项不属于你的执行范围"),
        "Coder 执行协议必须声明人工事项不在其执行范围内"
    );
    assert!(
        protocol.contains("不得因无法执行它们而报阻塞、拒绝完成或降低完成度"),
        "Coder 执行协议必须禁止把人工事项当成阻塞理由"
    );
    assert!(
        protocol.contains("缺少浏览器、设备、外部账号等人工环境不是运维阻塞"),
        "Coder 执行协议必须排除人工环境缺失作为 operational_gate 理由"
    );
}

/// 完成报告必须单列待人工处理清单。
#[test]
fn coding_completion_report_requires_pending_manual_section() {
    let contract = coding_completion_report_contract();
    assert!(
        contract.contains("待人工处理"),
        "完成报告契约必须要求单列待人工处理小节"
    );
    assert!(
        contract.contains("该清单是交接内容，不是未完成项"),
        "完成报告契约必须明确待人工清单不是未完成项"
    );
}

/// 两个 reviewer 协议都不得因待人工事项否决。
#[test]
fn reviewer_protocols_do_not_reject_pending_manual_items() {
    for (name, protocol) in [
        ("code_review", code_review_material_protocol()),
        ("group_final_review", group_final_review_material_protocol()),
    ] {
        assert!(
            protocol.contains("待人工处理事项不是缺陷"),
            "{name} 协议必须声明待人工事项不是缺陷"
        );
        assert!(
            protocol.contains("manual_check"),
            "{name} 协议必须点明 manual_check 类验收标准的归属"
        );
    }
    assert!(
        group_final_review_material_protocol()
            .contains("必须在 summary 中汇总整组的待人工处理清单"),
        "group final review 必须把待人工清单汇总给人工接手"
    );
}
