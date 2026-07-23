use super::*;

#[test]
fn code_review_prompt_requires_a_routing_receipt_before_the_final_json() {
    let protocol = code_review_material_protocol();

    assert!(protocol.contains("首个用户可见消息必须是工作流路由回执"));
    assert!(protocol.contains("最终审查结论必须只输出一个 JSON 对象"));
    assert!(!protocol.contains("CRITICAL: Return ONLY a single JSON object"));
}
