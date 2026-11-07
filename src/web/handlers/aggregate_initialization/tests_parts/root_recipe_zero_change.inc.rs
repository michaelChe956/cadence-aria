// REQ-BOOT-17：root recipe turn 零变更必须显式失败（端到端：provider turn
// 正常结束但零落盘 ⇒ 该 step 判失败，失败事实含 provider / step / 零变更，
// 后续 step 不再推进）。

/// 零动作的 pre_check turn ⇒ 该 step 判失败（不静默通过）、失败事实含
/// provider / step / 零变更三项、后续 step 保持 pending。
#[tokio::test]
async fn root_recipe_zero_change_turn_fails_the_step_with_diagnostic_facts() {
    let fixture = trust_route_fixture_with(
        false,
        RecipeFixtureOptions {
            zero_change_pre_check: true,
            ..RecipeFixtureOptions::default()
        },
    );
    let operations = root_policy_operations(&fixture);
    let operation_id = root_policy_start(&fixture, "root-recipe-zero-change-1").await;
    let operation = root_policy_poll_terminal(&operations, &operation_id).await;

    assert_eq!(
        operation.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Failed,
        "a zero-change provider turn must fail its step: {:?}",
        operation.error
    );
    assert_eq!(
        operation.failed_step,
        Some(crate::product::logical_codebase::AggregateInitializationStepKind::PreCheck),
        "the zero-change failure must land on the pre_check step"
    );
    let error = operation.error.as_ref().expect("classified error record");
    assert_eq!(
        error.provider.as_deref(),
        Some("claude_code"),
        "the failure fact must name the provider: {error:?}"
    );
    let diagnostic = error
        .stderr_summary
        .as_deref()
        .expect("the zero-change failure must keep a diagnosable summary");
    assert!(
        diagnostic.contains("zero_change"),
        "the failure fact must state the zero change: {diagnostic}"
    );
    assert!(
        diagnostic.contains("pre_check"),
        "the failure fact must name the step: {diagnostic}"
    );

    // 后续 step 不推进：RuleAndMcpConfig / OpenspecAndExamples 仍 pending。
    for step_kind in [
        crate::product::logical_codebase::AggregateInitializationStepKind::RuleAndMcpConfig,
        crate::product::logical_codebase::AggregateInitializationStepKind::OpenspecAndExamples,
    ] {
        let step = operation
            .steps
            .iter()
            .find(|step| step.step_id == step_kind)
            .expect("step record");
        assert_eq!(
            step.status.as_str(),
            "pending",
            "a zero-change turn must not advance the later steps: {step:?}"
        );
    }
}
