#[test]
fn work_item_plan_review_accepts_valid_string_affects_items() {
    let json = r#"{
        "verdict": "pass",
        "review_scope": "outline",
        "summary": "Outline 可以继续",
        "generation_round_id": "round_0001",
        "affects_items": [
            "outline_api",
            "outline_ui"
        ],
        "findings": []
    }"#;

    let verdict = parse_work_item_plan_review_json(
        json,
        "raw comments",
        &["outline_api".to_string(), "outline_ui".to_string()],
        WorkItemPlanReviewScope::Outline,
    )
    .expect("valid string outline references should be normalized");

    assert_eq!(verdict.verdict, ReviewVerdictType::Pass);
    let review = verdict
        .work_item_plan_review
        .expect("work item plan extension");
    assert_eq!(review.affects_items.len(), 2);
    assert_eq!(
        review.affects_items[0].target_outline_id.as_deref(),
        Some("outline_api")
    );
    assert_eq!(
        review.affects_items[1].target_outline_id.as_deref(),
        Some("outline_ui")
    );
}

#[test]
fn work_item_plan_review_derives_affected_items_from_finding_targets() {
    let json = r#"{
        "verdict": "pass",
        "review_scope": "outline",
        "summary": "Outline 可以继续，但有一条非阻塞建议",
        "generation_round_id": "round_0001",
        "findings": [{
            "severity": "suggestion",
            "target_outline_id": "outline_api",
            "message": "handoff 可以更明确",
            "evidence": "当前 handoff 缺少闭包透传说明",
            "required_action": "补充 handoff 说明"
        }]
    }"#;

    let verdict = parse_work_item_plan_review_json(
        json,
        "raw comments",
        &["outline_api".to_string(), "outline_ui".to_string()],
        WorkItemPlanReviewScope::Outline,
    )
    .expect("finding targets should provide affected outline references");

    assert_eq!(verdict.verdict, ReviewVerdictType::Pass);
    let review = verdict
        .work_item_plan_review
        .expect("work item plan extension");
    assert_eq!(review.affects_items.len(), 1);
    assert_eq!(
        review.affects_items[0].target_outline_id.as_deref(),
        Some("outline_api")
    );
}

#[test]
fn work_item_plan_review_prefers_finding_targets_over_legacy_affects_items() {
    let json = r#"{
        "verdict": "pass",
        "review_scope": "outline",
        "summary": "finding 目标是当前结构化结果的权威来源",
        "generation_round_id": "round_0001",
        "affects_items": [
            "outline_missing_1",
            "outline_missing_2"
        ],
        "findings": [{
            "severity": "suggestion",
            "target_outline_id": "outline_api",
            "message": "handoff 可以更明确",
            "evidence": "当前 handoff 缺少闭包透传说明",
            "required_action": "补充 handoff 说明"
        }]
    }"#;

    let verdict = parse_work_item_plan_review_json(
        json,
        "raw comments",
        &["outline_api".to_string(), "outline_ui".to_string()],
        WorkItemPlanReviewScope::Outline,
    )
    .expect("finding targets should take precedence over stale legacy references");

    let review = verdict
        .work_item_plan_review
        .expect("work item plan extension");
    assert_eq!(review.affects_items.len(), 1);
    assert_eq!(
        review.affects_items[0].target_outline_id.as_deref(),
        Some("outline_api")
    );
    assert!(review.warnings.is_empty());
}

#[test]
fn outline_pass_keeps_unknown_finding_target_as_warning() {
    let json = r#"{
        "verdict": "pass",
        "review_scope": "outline",
        "summary": "Outline 可以继续",
        "generation_round_id": "round_0001",
        "findings": [{
            "severity": "suggestion",
            "target_outline_id": "outline_missing",
            "message": "可选建议引用了旧 outline id",
            "evidence": "reviewer 使用了过期名称",
            "required_action": "人工定位对应 outline"
        }]
    }"#;

    let verdict = parse_work_item_plan_review_json(
        json,
        "raw comments",
        &["outline_api".to_string()],
        WorkItemPlanReviewScope::Outline,
    )
    .expect("pass should survive an invalid auxiliary finding target");

    assert_eq!(verdict.verdict, ReviewVerdictType::Pass);
    let review = verdict
        .work_item_plan_review
        .expect("work item plan extension");
    assert!(review.affects_items.is_empty());
    assert!(review
        .warnings
        .iter()
        .any(|warning| warning.contains("outline_missing")));
}

#[test]
fn outline_effective_revise_rejects_unknown_finding_target() {
    let value = serde_json::json!({
        "verdict": "pass",
        "review_scope": "outline",
        "summary": "finding 会把 pass 升级为 revise",
        "generation_round_id": "round_0001",
        "findings": [{
            "severity": "must_fix",
            "target_outline_id": "outline_missing",
            "message": "依赖图缺少前置节点",
            "evidence": "depends_on 为空",
            "required_action": "补充依赖"
        }]
    });

    let error = parse_work_item_plan_review_value(
        &value,
        "raw comments",
        &["outline_api".to_string()],
        WorkItemPlanReviewScope::Outline,
    )
    .expect_err("effective revise must keep strict outline targeting");

    assert_eq!(
        error,
        ReviewStructuredOutputErrorCode::InvalidOutlineReference
    );
}

#[test]
fn batch_review_keeps_legacy_affects_items_as_authoritative_source() {
    let json = r#"{
        "verdict": "needs_human",
        "review_scope": "batch",
        "summary": "整组需要人工判断",
        "generation_round_id": "round_0001",
        "affects_items": [{"target_outline_id":"outline_api"}],
        "findings": [{
            "severity": "suggestion",
            "target_outline_id": "outline_missing",
            "message": "旧版 batch finding 携带了非协议目标字段",
            "evidence": "辅助字段不应覆盖 affects_items",
            "required_action": "无"
        }]
    }"#;

    let verdict = parse_work_item_plan_review_json(
        json,
        "raw comments",
        &["outline_api".to_string()],
        WorkItemPlanReviewScope::Batch,
    )
    .expect("batch should keep using its legacy affects_items contract");

    let review = verdict
        .work_item_plan_review
        .expect("work item plan extension");
    assert_eq!(review.affects_items.len(), 1);
    assert_eq!(
        review.affects_items[0].target_outline_id.as_deref(),
        Some("outline_api")
    );
    assert!(review.warnings.is_empty());
}
