use super::*;

#[test]
fn code_review_prompt_requires_a_routing_receipt_before_the_final_json() {
    let protocol = code_review_material_protocol();

    assert!(protocol.contains("首个用户可见消息必须是工作流路由回执"));
    assert!(protocol.contains("最终审查结论必须只输出一个 JSON 对象"));
    assert!(!protocol.contains("CRITICAL: Return ONLY a single JSON object"));
}

#[test]
fn review_prompts_forbid_braces_outside_the_final_verdict_json() {
    for protocol in [
        code_review_material_protocol(),
        group_final_review_material_protocol(),
    ] {
        assert!(protocol.contains("除最终结论 JSON 外"));
        assert!(protocol.contains("不得出现 { 或 }"));
        assert!(protocol.contains("证据中的 JSON 片段必须改写为自然语言描述"));
    }
}
