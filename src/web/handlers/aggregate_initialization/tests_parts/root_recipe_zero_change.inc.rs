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

/// 零变更失败的诊断必须**带出 provider 的真实输出**：零变更本身不说明原因
/// （命令没被执行 / 命令执行了但失败 / 命令按契约幂等无变更，三种情形的
/// provider 输出完全不同）。只显示 `zero_change` 一句时用户无从下手
/// （本仓 2026-11-12 真机：Cadence-skills 脚本的 JSON 报告整段丢失）。
#[tokio::test]
async fn root_recipe_zero_change_diagnostic_carries_provider_output() {
    let fixture = trust_route_fixture_with(
        false,
        RecipeFixtureOptions {
            zero_change_pre_check: true,
            ..RecipeFixtureOptions::default()
        },
    );
    let operations = root_policy_operations(&fixture);
    let operation_id = root_policy_start(&fixture, "root-recipe-zero-change-output-1").await;
    let operation = root_policy_poll_terminal(&operations, &operation_id).await;

    let error = operation.error.as_ref().expect("classified error record");
    let diagnostic = error
        .stderr_summary
        .as_deref()
        .expect("the zero-change failure must keep a diagnosable summary");
    assert!(
        diagnostic.contains("zero_change"),
        "the failure fact must state the zero change: {diagnostic}"
    );
    // 测试 provider 的输出以「# Story Spec」开头（FakeStreamingProvider 的
    // 固定产出）；诊断里出现它 ⇒ provider 的真实输出确实带进了失败面。
    assert!(
        diagnostic.contains("Story Spec"),
        "the diagnostic must carry the provider output: {diagnostic}"
    );
}

/// 零变更 **且脚本报告成功** ⇒ 判**成功**：Cadence-skills 四条命令的契约就是
/// 幂等（重跑报告 all-skipped / 0 created / 0 updated、产物无变化）。本仓
/// 2026-11-12 真机：已初始化的聚合根上重跑 recipe ⇒ 工作区零变更 ⇒ 只看
/// 「有无变更」的旧判据把幂等成功判成失败，「重新初始化」永远走不通。
#[tokio::test]
async fn root_recipe_zero_change_with_successful_script_report_completes_the_step() {
    let fixture = trust_route_fixture_with(
        false,
        RecipeFixtureOptions {
            zero_change_pre_check: true,
            recipe_report: Some(
                "{\"mirror\": \"cn\", \"mode\": \"run\", \"overall\": \"success\", \
                 \"phases\": [{\"phase\": \"base-tools\", \"result\": \"skipped\"}]}"
                    .to_string(),
            ),
            ..RecipeFixtureOptions::default()
        },
    );
    let operations = root_policy_operations(&fixture);
    let operation_id = root_policy_start(&fixture, "root-recipe-idempotent-1").await;
    let operation = root_policy_poll_terminal(&operations, &operation_id).await;

    let pre_check = operation
        .steps
        .iter()
        .find(|step| {
            step.step_id
                == crate::product::logical_codebase::AggregateInitializationStepKind::PreCheck
        })
        .expect("pre_check step record");
    assert_eq!(
        pre_check.status.as_str(),
        "completed",
        "an idempotent recipe turn (script reported success, zero change) must complete: {operation:?}"
    );
}
