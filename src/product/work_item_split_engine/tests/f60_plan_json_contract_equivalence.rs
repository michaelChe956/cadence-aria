// F-60 P1b：Plan JSON 子链增量契约等价回归（worktree 基重做）。
// 方案：cadence/designs/2026-09-25_技术方案_F60一次成功率_v1.0.md 因素五 P1（v1.1）。
//
// 同一 request 下，Outline 初次／增量修订／二次修订、Split 初次／局部 redo、
// Draft 初次／反馈轮必须满足：
// - 最新 nonce：每轮新生成，修订轮 prompt 不携带上一轮 nonce；
// - 真实来源 ID：来源条款与最小正确示例注入 request 实际 spec ID；无真实 ID 时
//   占位必须显式标注（不伪造来源骗 gate），修订轮指回上一版 outline 的既有来源；
// - 必填 schema 与 options 语义：修订／redo 轮不依赖会话历史补合同（契约等价）；
// - Draft：反馈轮不丢 [self_check]、封闭字段合同与 sentinel 规则，维持 15,600B 预算。

use crate::product::work_item_split_engine::types::prompt_nonce;

#[test]
fn outline_revision_prompt_carries_source_spec_id_rule_and_example_with_real_ids() {
    let (mut request, issue, _repository) = split_prompt_fixture();
    request.story_spec_ids = vec!["story_spec_0007".to_string()];
    request.design_spec_ids = vec!["design_spec_0009".to_string()];

    let (prompt, nonce) = build_outline_revision_prompt(
        &request,
        &issue,
        "补充前后端依赖边",
        &RoutingReferenceContext::Legacy,
    );

    // 来源条款点名本请求已确认的真实 spec ID，禁止空数组。
    assert!(
        prompt.contains("source_story_spec_ids/source_design_spec_ids"),
        "修订 prompt 必须携带来源 ID 条款：{prompt}"
    );
    assert!(
        prompt.contains("story_spec_0007") && prompt.contains("design_spec_0009"),
        "修订来源条款必须点名 request 中的真实 spec ID：{prompt}"
    );
    assert!(
        prompt.contains("禁止空数组"),
        "修订来源条款必须显式禁止空数组：{prompt}"
    );

    // 最小正确示例注入真实 ID（outline 级 + 两个 work_item_outlines 项 = 3 处）。
    assert_eq!(
        prompt
            .matches("\"source_story_spec_ids\":[\"story_spec_0007\"]")
            .count(),
        3,
        "修订示例必须在 outline 级与每个 work_item_outlines 项注入真实 story spec ID：{prompt}"
    );
    assert_eq!(
        prompt
            .matches("\"source_design_spec_ids\":[\"design_spec_0009\"]")
            .count(),
        3,
        "修订示例必须在 outline 级与每个 work_item_outlines 项注入真实 design spec ID：{prompt}"
    );
    assert!(
        !prompt.contains("\"source_story_spec_ids\":[]"),
        "修订示例不得出现空 source_story_spec_ids 数组：{prompt}"
    );

    // 增量纪律不破坏：修订轮不重复全量 story/design/repository 上下文。
    assert!(
        !prompt.contains("[confirmed_story_specs]"),
        "修订 prompt 不应重复全量 story 上下文：{prompt}"
    );
    assert!(
        !prompt.contains("[repository_structure_summary]"),
        "修订 prompt 不应重复仓库结构上下文：{prompt}"
    );

    // 本轮 nonce 自洽。
    assert!(
        prompt.contains(&format!("nonce=\"{nonce}\"")),
        "修订 prompt 必须携带本轮 nonce sentinel：{prompt}"
    );
}

#[test]
fn outline_revision_prompt_marks_placeholder_and_points_to_previous_outline_when_request_empty() {
    let (request, issue, _repository) = split_prompt_fixture();
    assert!(request.story_spec_ids.is_empty());
    assert!(request.design_spec_ids.is_empty());

    let (prompt, _nonce) = build_outline_revision_prompt(
        &request,
        &issue,
        "修复验证可达性",
        &RoutingReferenceContext::Legacy,
    );

    // 无真实 ID 时：示例回退占位但必须显式标注，不得冒充真实来源。
    assert_eq!(
        prompt
            .matches("\"source_story_spec_ids\":[\"story_spec_0001\"]")
            .count(),
        3,
        "request 无 story spec ID 时修订示例必须给非空占位：{prompt}"
    );
    assert!(
        prompt.contains("仅为形状占位"),
        "修订示例占位必须显式标注不是真实来源：{prompt}"
    );
    assert!(
        !prompt.contains("\"source_story_spec_ids\":[]"),
        "修订示例不得出现空 source 数组：{prompt}"
    );

    // 来源条款指回上一版 outline 的既有来源，禁止空数组与虚构。
    assert!(
        prompt.contains("上一版 outline"),
        "request 无真实 ID 时修订条款必须指回上一版 outline 的既有来源：{prompt}"
    );
    assert!(
        prompt.contains("禁止空数组"),
        "修订条款必须显式禁止空数组：{prompt}"
    );
}

#[test]
fn outline_initial_prompt_marks_placeholder_example_explicitly() {
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
        prompt.contains("仅为形状占位"),
        "初次 prompt 的占位示例必须显式标注不是真实来源：{prompt}"
    );
    assert!(
        prompt.contains(
            "work_item_outlines[] 每项的 source_story_spec_ids/source_design_spec_ids 必须填写 [confirmed_story_specs]/[confirmed_design_specs] 中的真实 spec ID，禁止空数组"
        ),
        "初次来源条款保持指向 [confirmed_*_specs] 小节：{prompt}"
    );
}

#[test]
fn outline_initial_prompt_with_real_ids_has_no_placeholder_annotation() {
    let (mut request, issue, repository) = split_prompt_fixture();
    request.story_spec_ids = vec!["story_spec_0003".to_string()];
    request.design_spec_ids = vec!["design_spec_0005".to_string()];

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
        !prompt.contains("仅为形状占位"),
        "有真实 ID 时初次示例不应携带占位标注：{prompt}"
    );
    assert_eq!(
        prompt
            .matches("\"source_story_spec_ids\":[\"story_spec_0003\"]")
            .count(),
        3,
        "初次示例必须注入真实 story spec ID：{prompt}"
    );
}

#[test]
fn outline_revision_contract_shares_gate_required_terms_with_initial() {
    let (mut request, issue, repository) = split_prompt_fixture();
    request.story_spec_ids = vec!["story_spec_0007".to_string()];
    request.design_spec_ids = vec!["design_spec_0009".to_string()];
    request.force_frontend_backend_split = Some(true);

    let (initial, _initial_nonce) = build_outline_prompt_with_nonce(
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
    let (revision, _revision_nonce) = build_outline_revision_prompt(
        &request,
        &issue,
        "补充前后端依赖边",
        &RoutingReferenceContext::Legacy,
    );

    // 契约等价：gate 必需条款在初次/修订同源出现，修订不依赖会话历史补合同。
    for term in [
        "最后必须输出一个 nonce sentinel JSON block",
        "后端只解析最后一个 nonce 匹配的 <ARIA_STRUCTURED_OUTPUT",
        "JSON 字符串内不得直接包含未转义英文双引号",
        "estimated_context_tokens(1..=50000)",
        "session_fit=\"fits_single_agent_session\"",
        "严格按以下 JSON schema 输出",
        "最小正确示例",
        "[user_options]",
        "force_frontend_backend_split=true",
        "如果 A depends_on B，则 A 与 B 不得拥有相同路径",
    ] {
        assert!(
            initial.contains(term),
            "初次 outline prompt 缺少 gate 必需条款 `{term}`：{initial}"
        );
        assert!(
            revision.contains(term),
            "修订 outline prompt 缺少 gate 必需条款 `{term}`（增量契约不等价）：{revision}"
        );
    }

    // schema 同源：两轮携带同一份完整 JSON schema。
    assert!(initial.contains(WORK_ITEM_PLAN_OUTLINE_OUTPUT_SCHEMA));
    assert!(revision.contains(WORK_ITEM_PLAN_OUTLINE_OUTPUT_SCHEMA));
}

#[test]
fn outline_nonce_is_fresh_per_round() {
    let (mut request, issue, repository) = split_prompt_fixture();
    request.story_spec_ids = vec!["story_spec_0001".to_string()];
    request.design_spec_ids = vec!["design_spec_0002".to_string()];

    let (initial_prompt, initial_nonce) = build_outline_prompt_with_nonce(
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
    let (revision_one_prompt, revision_one_nonce) = build_outline_revision_prompt(
        &request,
        &issue,
        "第一轮返修",
        &RoutingReferenceContext::Legacy,
    );
    let (revision_two_prompt, revision_two_nonce) = build_outline_revision_prompt(
        &request,
        &issue,
        "第二轮返修",
        &RoutingReferenceContext::Legacy,
    );

    let nonces = [&initial_nonce, &revision_one_nonce, &revision_two_nonce];
    for (index, nonce) in nonces.iter().enumerate() {
        for (other_index, other) in nonces.iter().enumerate() {
            if index != other_index {
                assert_ne!(
                    nonce, other,
                    "每轮 nonce 必须新生成，不得跨轮复用（轮次 {index} 与 {other_index}）"
                );
            }
        }
    }

    // 修订轮 prompt 不携带上一轮 nonce（防止旧轮 nonce 随历史复制到末端）。
    assert!(
        !revision_one_prompt.contains(&format!("nonce=\"{initial_nonce}\"")),
        "修订 prompt 不得携带初次轮 nonce：{revision_one_prompt}"
    );
    assert!(
        !revision_two_prompt.contains(&format!("nonce=\"{revision_one_nonce}\"")),
        "二次修订 prompt 不得携带上一修订轮 nonce：{revision_two_prompt}"
    );
    // 每轮 prompt 携带自己的 nonce。
    assert!(initial_prompt.contains(&format!("nonce=\"{initial_nonce}\"")));
    assert!(revision_one_prompt.contains(&format!("nonce=\"{revision_one_nonce}\"")));
    assert!(revision_two_prompt.contains(&format!("nonce=\"{revision_two_nonce}\"")));
}

#[test]
fn split_revision_prompt_restates_user_options_and_openspec_constraint_summary() {
    let (mut request, issue, repository) = split_prompt_fixture();
    request.story_spec_ids = vec!["story_spec_0002".to_string()];
    request.design_spec_ids = vec!["design_spec_0004".to_string()];
    request.include_integration_tests = Some(true);
    request.force_frontend_backend_split = Some(true);

    let redo_specs = vec![RedoSpec {
        old_id: "work_item_0001".to_string(),
        feedback: "拆得太粗".to_string(),
    }];
    let prompt = build_revision_prompt(
        &request,
        &issue,
        &repository,
        &[],
        &redo_specs,
        &[],
        &[],
        "(empty)",
        &RoutingReferenceContext::Legacy,
    );

    // 必要选项约束按 request 重述（Final Compile 会按 options 校验 kind 组成）。
    assert!(
        prompt.contains("[user_options]"),
        "redo prompt 必须重述 [user_options]：{prompt}"
    );
    assert!(
        prompt.contains("include_integration_tests: true"),
        "redo prompt 必须重述开启的 integration 选项：{prompt}"
    );
    assert!(
        prompt.contains("include_e2e_tests: false"),
        "redo prompt 必须完整重述选项值：{prompt}"
    );
    assert!(
        prompt.contains("force_frontend_backend_split: true"),
        "redo prompt 必须重述前后端拆分选项：{prompt}"
    );
    assert!(
        prompt.contains("require_execution_plan_confirm: false"),
        "redo prompt 必须完整重述确认选项：{prompt}"
    );

    // 来源约束摘要重述（work item 双来源追踪由 Final Compile 强制）。
    assert!(
        prompt.contains("[openspec_constraint_summary]"),
        "redo prompt 必须重述 [openspec_constraint_summary]：{prompt}"
    );
    assert!(
        prompt.contains("story_spec_ids: story_spec_0002"),
        "redo prompt 必须点名真实 story spec ID：{prompt}"
    );
    assert!(
        prompt.contains("design_spec_ids: design_spec_0004"),
        "redo prompt 必须点名真实 design spec ID：{prompt}"
    );

    // redo-only 输出边界保持。
    assert!(
        prompt.contains("仅输出需要重做的 work_items"),
        "redo prompt 保持 redo-only 输出边界：{prompt}"
    );
}

#[test]
fn split_nonce_is_fresh_between_initial_and_redo() {
    let (request, issue, repository) = split_prompt_fixture();
    let redo_specs = vec![RedoSpec {
        old_id: "work_item_0001".to_string(),
        feedback: "拆得太粗".to_string(),
    }];

    let initial_prompt = build_split_prompt(
        &request,
        &issue,
        &repository,
        &[],
        &[],
        "(empty)",
        &RoutingReferenceContext::Legacy,
    );
    let redo_prompt = build_revision_prompt(
        &request,
        &issue,
        &repository,
        &[],
        &redo_specs,
        &[],
        &[],
        "(empty)",
        &RoutingReferenceContext::Legacy,
    );

    let initial_nonce = prompt_nonce(&initial_prompt);
    let redo_nonce = prompt_nonce(&redo_prompt);
    assert_ne!(
        initial_nonce, redo_nonce,
        "Split 初次与 redo 的 nonce 必须各自新生成"
    );
    assert!(
        !redo_prompt.contains(&format!("nonce=\"{initial_nonce}\"")),
        "redo prompt 不得携带初次轮 nonce：{redo_prompt}"
    );
}

#[test]
fn draft_prompt_with_feedback_retains_self_check_closed_fields_sentinel_and_budget() {
    let outline = parse_work_item_plan_outline_output(valid_outline_author_output())
        .expect("outline output")
        .outline
        .expect("outline");

    let invocation = build_work_item_draft_invocation(
        &outline,
        "outline_backend",
        WorkItemGenerationMode::Serial,
        &[],
        Some("reviewer must_fix：验收标准必须绑定可观测结果状态"),
        &RoutingReferenceContext::Legacy,
    )
    .expect("draft invocation");

    let prompt = &invocation.prompt;
    assert!(
        prompt.contains("[user_or_reviewer_feedback]"),
        "反馈轮必须携带反馈小节：{prompt}"
    );
    assert!(
        prompt.contains("reviewer must_fix"),
        "反馈内容必须注入 prompt：{prompt}"
    );
    assert!(
        prompt.contains("[self_check]"),
        "反馈轮不得丢失 [self_check]：{prompt}"
    );
    assert!(
        prompt.contains("[canonical_field_contract]"),
        "反馈轮不得丢失封闭字段合同：{prompt}"
    );
    assert!(
        prompt.contains("最后必须输出一个 nonce sentinel") || prompt.contains("nonce sentinel"),
        "反馈轮不得丢失 sentinel 规则：{prompt}"
    );
    assert!(
        prompt.contains("<ARIA_STRUCTURED_OUTPUT nonce=\""),
        "反馈轮必须携带 sentinel 标签规则：{prompt}"
    );
    assert!(
        prompt.len() < WORK_ITEM_DRAFT_PROMPT_QUALITY_BUDGET_BYTES,
        "反馈轮 prompt 必须维持在质量预算内：{} bytes",
        prompt.len()
    );
}
