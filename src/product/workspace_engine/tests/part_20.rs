#[test]
fn build_work_item_plan_outline_review_input_includes_boundary_rules() {
    let (_tmp, _checkpoint_store, _lifecycle, _plan_id, engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_outline_review_boundary");
    let outline_payload = work_item_plan_outline_artifact();
    let ArtifactPayload::WorkItemPlanOutlineCandidate { outline_candidate } = outline_payload else {
        panic!("expected outline candidate artifact");
    };

    let input = engine
        .build_work_item_plan_outline_review_input(&outline_candidate)
        .expect("outline review input");

    assert_work_item_plan_boundary_rules(&input.prompt);
    assert!(input.prompt.contains("estimated_context_tokens"));
    assert!(input.prompt.contains("session_fit"));
    for field in [
        "\"id\"",
        "\"project_id\"",
        "\"issue_id\"",
        "\"source_story_spec_ids\"",
        "\"source_design_spec_ids\"",
        "\"strategy_summary\"",
        "\"work_item_outlines\"",
        "\"dependency_graph\"",
        "\"risks\"",
        "\"handoff_strategy\"",
        "\"status\"",
        "\"outline_id\"",
        "\"title\"",
        "\"kind\"",
        "\"goal\"",
        "\"scope\"",
        "\"non_goals\"",
        "\"estimated_context_tokens\"",
        "\"session_fit\"",
        "\"exclusive_write_scopes\"",
        "\"forbidden_write_scopes\"",
        "\"depends_on\"",
        "\"verification_intent\"",
        "\"handoff_notes\"",
    ] {
        assert!(
            input.prompt.contains(field),
            "outline reviewer prompt must include complete candidate field {field}"
        );
    }
    for required in [
        "40k",
        "50k",
        "最大内聚",
        "最少拆分",
        "不必要拆分",
        "[outline_unnecessary_split]",
    ] {
        assert!(
            input.prompt.contains(required),
            "outline reviewer prompt must include `{required}`: {}",
            input.prompt
        );
    }
    assert!(input.prompt.contains("severity=must_fix"));
    assert!(input.prompt.contains("target_outline_id"));
    assert!(!input.prompt.contains("小于 20k"));
    for required_contract in [
        "不超过 40k 属正常范围",
        "40001..=50000",
        "超过 50k 必须返回 `revise` 并要求拆分",
        "发现不必要拆分时必须给出 severity=must_fix",
        "message 必须以 [outline_unnecessary_split] 开头",
        "target_outline_id 引用其中一个现有 outline",
        "evidence 列出全部可合并 outline ID",
        "required_action 明确要求合并",
    ] {
        assert!(
            input.prompt.contains(required_contract),
            "outline reviewer prompt must preserve contract `{required_contract}`: {}",
            input.prompt
        );
    }
    assert!(
        !input.prompt.contains("\"code\""),
        "outline review schema must reuse ReviewFinding without a code field: {}",
        input.prompt
    );
    assert!(input.prompt.contains(
        "\"generation_round_id\":\"generation_round_unknown\""
    ));
    assert!(input.prompt.contains("\"target_outline_id\":\"outline id\""));
    assert!(input.prompt.contains("从 findings[].target_outline_id 推导"));
    assert!(
        !input.prompt.contains("\"affects_items\""),
        "new outline review schema should not duplicate affected outline references"
    );
    assert_review_contract(&input, "work_item_plan_outline_review");
}
