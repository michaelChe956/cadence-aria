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

/// 脚本**显式报告失败**且本 turn 已有 allowlisted 落盘变更 ⇒ 失败优先于
/// 变更（本仓 2026-11-16 真机缺陷：pre-check 报告 overall=failed/rc1 且
/// 14 条 allowlisted 变更被误判 Completed）。变更 receipt 必须在拒绝前
/// durable 保留，失败面带上报告里的真实失败细节。
#[tokio::test]
async fn root_recipe_script_failure_with_changes_fails_the_step_and_keeps_change_receipts() {
    let fixture = trust_route_fixture_with(
        false,
        RecipeFixtureOptions {
            zero_change_pre_check: false,
            recipe_report: Some(
                "{\"mirror\": \"cn\", \"mode\": \"run\", \"overall\": \"failed\", \
                 \"phases\": [{\"phase\": \"pre-check\", \"result\": \"failed\", \"rc\": 1}]}"
                    .to_string(),
            ),
            ..RecipeFixtureOptions::default()
        },
    );
    let operations = root_policy_operations(&fixture);
    let operation_id = root_policy_start(&fixture, "root-recipe-failed-with-changes-1").await;
    let operation = root_policy_poll_terminal(&operations, &operation_id).await;

    assert_eq!(
        operation.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Failed,
        "an explicit script failure must fail its step even with allowlisted changes: {:?}",
        operation.error
    );
    assert_eq!(
        operation.failed_step,
        Some(crate::product::logical_codebase::AggregateInitializationStepKind::PreCheck),
        "the explicit script failure must land on the pre_check step"
    );
    let error = operation.error.as_ref().expect("classified error record");
    assert_eq!(
        error.provider.as_deref(),
        Some("claude_code"),
        "the failure fact must name the provider: {error:?}"
    );
    assert_eq!(
        error.cause.as_deref(),
        Some("root_recipe_script_failed"),
        "changes must not mask an explicit script failure: {error:?}"
    );
    let diagnostic = error
        .stderr_summary
        .as_deref()
        .expect("the script failure must keep a diagnosable summary");
    assert!(
        diagnostic.contains("root recipe script reported failure"),
        "the failure fact must state the explicit script failure: {diagnostic}"
    );
    assert!(
        diagnostic.contains("pre-check"),
        "the failure fact must carry the real report details: {diagnostic}"
    );
    assert!(
        diagnostic.contains("\"overall\": \"failed\""),
        "the failure fact must carry the failed overall verdict: {diagnostic}"
    );

    // 失败优先于变更，但变更事实必须 durable 保留：pre_check 命令的
    // per-LC receipt 里 allowlisted observed_changes 非空。
    let mut commands = root_policy_receipts(&fixture)
        .list_commands("project_0001", &operation_id)
        .expect("command receipts");
    commands.sort_by_key(|receipt| receipt.command_index);
    let pre_check_receipt = commands
        .iter()
        .find(|receipt| receipt.command_index == 1)
        .expect("pre_check command receipt");
    assert_eq!(
        pre_check_receipt.verdict,
        crate::product::logical_codebase::root_recipe_receipt::RootRecipeCommandVerdict::Allowed,
        "the pre_check change must be audited Allowed: {pre_check_receipt:?}"
    );
    assert!(
        !pre_check_receipt.observed_changes.is_empty(),
        "the allowlisted observed changes must be retained on the receipt: {pre_check_receipt:?}"
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
            "an explicit script failure must not advance the later steps: {step:?}"
        );
    }
}

/// 脚本显式报告失败且**零变更** ⇒ 同样判失败：显式失败优先，成败不再由
/// 「有没有变更」单独决定（失败面同带报告里的真实失败细节）。
#[tokio::test]
async fn root_recipe_script_failure_with_zero_change_fails_the_step() {
    let fixture = trust_route_fixture_with(
        false,
        RecipeFixtureOptions {
            zero_change_pre_check: true,
            recipe_report: Some(
                "{\"mirror\": \"cn\", \"mode\": \"run\", \"overall\": \"failed\", \
                 \"phases\": [{\"phase\": \"pre-check\", \"result\": \"failed\", \"rc\": 1}]}"
                    .to_string(),
            ),
            ..RecipeFixtureOptions::default()
        },
    );
    let operations = root_policy_operations(&fixture);
    let operation_id = root_policy_start(&fixture, "root-recipe-failed-zero-change-1").await;
    let operation = root_policy_poll_terminal(&operations, &operation_id).await;

    assert_eq!(
        operation.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Failed,
        "an explicit script failure must fail its step even with zero change: {:?}",
        operation.error
    );
    assert_eq!(
        operation.failed_step,
        Some(crate::product::logical_codebase::AggregateInitializationStepKind::PreCheck),
        "the explicit script failure must land on the pre_check step"
    );
    let error = operation.error.as_ref().expect("classified error record");
    assert_eq!(
        error.cause.as_deref(),
        Some("root_recipe_script_failed"),
        "the failure must classify as an explicit script failure, not zero change: {error:?}"
    );
    let diagnostic = error
        .stderr_summary
        .as_deref()
        .expect("the script failure must keep a diagnosable summary");
    assert!(
        diagnostic.contains("root recipe script reported failure"),
        "the failure fact must state the explicit script failure: {diagnostic}"
    );
    assert!(
        diagnostic.contains("pre-check"),
        "the failure fact must carry the real report details: {diagnostic}"
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
            "an explicit script failure must not advance the later steps: {step:?}"
        );
    }
}

/// 脚本报告成功**且有落盘变更** ⇒ 失败门不得误伤：pre_check 完成、
/// operation 走到终态 Completed（显式失败门只拦 `overall=failed`）。
#[tokio::test]
async fn root_recipe_successful_script_report_with_changes_completes_the_step() {
    let fixture = trust_route_fixture_with(
        false,
        RecipeFixtureOptions {
            zero_change_pre_check: false,
            recipe_report: Some(
                "{\"mirror\": \"cn\", \"mode\": \"run\", \"overall\": \"success\", \
                 \"phases\": [{\"phase\": \"pre-check\", \"result\": \"success\", \
                 \"created\": 3, \"updated\": 11}]}"
                    .to_string(),
            ),
            ..RecipeFixtureOptions::default()
        },
    );
    let operations = root_policy_operations(&fixture);
    let operation_id = root_policy_start(&fixture, "root-recipe-success-with-changes-1").await;
    let operation = root_policy_poll_terminal(&operations, &operation_id).await;

    assert_eq!(
        operation.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Completed,
        "a successful script report with real changes must complete the operation: {:?}",
        operation.error
    );
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
        "a successful report turn with changes must complete the pre_check step: {operation:?}"
    );
}
