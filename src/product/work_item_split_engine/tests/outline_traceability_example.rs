// Bug A 回归：outline prompt 的「最小正确示例」不得使用空
// source_story_spec_ids/source_design_spec_ids 数组。
// 校验器 validate_outline_traceability_and_scopes 强制每个 work_item_outlines
// 项同时引用 story spec 与 design spec，示例若为空数组，弱模型 provider
// （如 kimi_code）照抄示例会导致第一轮 outline 必失败。

#[test]
fn outline_prompt_example_injects_real_source_spec_ids() {
    let (mut request, issue, repository) = split_prompt_fixture();
    request.story_spec_ids = vec!["story_spec_0007".to_string()];
    request.design_spec_ids = vec!["design_spec_0009".to_string()];

    let (prompt, _nonce) = build_outline_prompt_with_nonce(
        &request,
        &issue,
        &repository,
        &[],
        &[],
        "",
        &[],
        &[],
        &RoutingReferenceContext::Legacy,
    );

    // outline 级 + work_item_outlines 两项 = 3 处，全部非空且复用真实 ID。
    assert_eq!(
        prompt.matches("\"source_story_spec_ids\":[\"story_spec_0007\"]").count(),
        3,
        "示例必须在 outline 级与每个 work_item_outlines 项注入真实 story spec ID：{prompt}"
    );
    assert_eq!(
        prompt.matches("\"source_design_spec_ids\":[\"design_spec_0009\"]").count(),
        3,
        "示例必须在 outline 级与每个 work_item_outlines 项注入真实 design spec ID：{prompt}"
    );
    assert!(
        !prompt.contains("\"source_story_spec_ids\":[]"),
        "示例不得出现空 source_story_spec_ids 数组：{prompt}"
    );
    assert!(
        !prompt.contains("\"source_design_spec_ids\":[]"),
        "示例不得出现空 source_design_spec_ids 数组：{prompt}"
    );
}

#[test]
fn outline_prompt_example_injects_multiple_real_source_spec_ids() {
    let (mut request, issue, repository) = split_prompt_fixture();
    request.story_spec_ids = vec!["story_spec_0001".to_string(), "story_spec_0002".to_string()];
    request.design_spec_ids = vec!["design_spec_0003".to_string()];

    let (prompt, _nonce) = build_outline_prompt_with_nonce(
        &request,
        &issue,
        &repository,
        &[],
        &[],
        "",
        &[],
        &[],
        &RoutingReferenceContext::Legacy,
    );

    assert_eq!(
        prompt
            .matches("\"source_story_spec_ids\":[\"story_spec_0001\",\"story_spec_0002\"]")
            .count(),
        3,
        "多个真实 story spec ID 必须完整注入示例：{prompt}"
    );
    assert_eq!(
        prompt.matches("\"source_design_spec_ids\":[\"design_spec_0003\"]").count(),
        3,
        "真实 design spec ID 必须完整注入示例：{prompt}"
    );
}

#[test]
fn outline_prompt_example_uses_placeholder_source_ids_when_request_empty() {
    let (request, issue, repository) = split_prompt_fixture();
    assert!(request.story_spec_ids.is_empty());
    assert!(request.design_spec_ids.is_empty());

    let (prompt, _nonce) = build_outline_prompt_with_nonce(
        &request,
        &issue,
        &repository,
        &[],
        &[],
        "",
        &[],
        &[],
        &RoutingReferenceContext::Legacy,
    );

    assert_eq!(
        prompt.matches("\"source_story_spec_ids\":[\"story_spec_0001\"]").count(),
        3,
        "request 无 story spec ID 时示例必须给非空占位（占位需与真实 ID 零填充风格一致）：{prompt}"
    );
    assert_eq!(
        prompt.matches("\"source_design_spec_ids\":[\"design_spec_0001\"]").count(),
        3,
        "request 无 design spec ID 时示例必须给非空占位（占位需与真实 ID 零填充风格一致）：{prompt}"
    );
    assert!(
        !prompt.contains("\"source_story_spec_ids\":[]"),
        "示例不得出现空 source_story_spec_ids 数组：{prompt}"
    );
    assert!(
        !prompt.contains("\"source_design_spec_ids\":[]"),
        "示例不得出现空 source_design_spec_ids 数组：{prompt}"
    );
}

#[test]
fn outline_prompt_contract_forbids_empty_source_spec_id_arrays() {
    let (request, issue, repository) = split_prompt_fixture();

    let (prompt, _nonce) = build_outline_prompt_with_nonce(
        &request,
        &issue,
        &repository,
        &[],
        &[],
        "",
        &[],
        &[],
        &RoutingReferenceContext::Legacy,
    );

    assert!(
        prompt.contains(
            "work_item_outlines[] 每项的 source_story_spec_ids/source_design_spec_ids 必须填写 [confirmed_story_specs]/[confirmed_design_specs] 中的真实 spec ID，禁止空数组"
        ),
        "strict_output_contract 必须显式禁止空 source spec ID 数组：{prompt}"
    );
}
