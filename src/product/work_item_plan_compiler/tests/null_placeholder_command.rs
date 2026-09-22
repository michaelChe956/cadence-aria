use super::{WorkItemPlanSourceContext, compile_work_item_plan};

const REP4_FIXTURE: &str = include_str!("../fixtures/work-item-plan-rep4.md");

#[test]
fn null_literal_command_lowers_to_manual_only_check_without_trusted_commands() {
    let context = WorkItemPlanSourceContext {
        target_repository_id: "repo-levels".to_string(),
    };
    // prompt 教学的 manual check 形态：显式 `- command: null` + manual_instruction。
    let source = REP4_FIXTURE.replacen(
        "- command: cargo test --locked --lib levels_api",
        "- command: null",
        1,
    );
    let ir = compile_work_item_plan(&source, &context)
        .expect("command: null + manual_instruction 必须 lower 为 manual-only check");
    assert_eq!(ir.items[0].verification_plan.checks.len(), 1);
    assert_eq!(ir.items[0].verification_plan.checks[0].command, None);
    assert!(
        ir.items[0].trusted_commands.is_empty(),
        "字面量 null 不得注册为 trusted command"
    );
    assert_eq!(
        ir.items[0].verification_plan.checks[0]
            .manual_instruction
            .as_deref(),
        Some("Confirm the endpoint returns the configured levels JSON.")
    );
    // 其余 item 的真实 command 投影不受影响。
    assert_eq!(
        ir.items[1..]
            .iter()
            .flat_map(|item| item.trusted_commands.iter())
            .map(|command| command.command.as_str())
            .collect::<Vec<_>>(),
        [
            "pnpm test level-select",
            "cargo test --locked --test levels_integration"
        ]
    );

    // 大小写与前后空格同样归一为未声明。
    for variant in ["Null", "NULL", " none ", "None"] {
        let source = REP4_FIXTURE.replacen(
            "- command: cargo test --locked --lib levels_api",
            &format!("- command: {variant}"),
            1,
        );
        let ir = compile_work_item_plan(&source, &context).expect("占位字面量必须归一为未声明");
        assert_eq!(
            ir.items[0].verification_plan.checks[0].command, None,
            "{variant} 必须归一"
        );
        assert!(ir.items[0].trusted_commands.is_empty(), "{variant}");
    }

    // 空值保持既有 None 语义。
    let blank = REP4_FIXTURE.replacen(
        "- command: cargo test --locked --lib levels_api",
        "- command:   ",
        1,
    );
    let ir = compile_work_item_plan(&blank, &context).expect("空 command 保持 manual-only 语义");
    assert_eq!(ir.items[0].verification_plan.checks[0].command, None);
    assert!(ir.items[0].trusted_commands.is_empty());

    // manual_instruction 同规则：占位 null 不再污染 trusted command purpose。
    let placeholder_instruction = REP4_FIXTURE.replacen(
        "- manual_instruction: Confirm the endpoint returns the configured levels JSON.",
        "- manual_instruction: null",
        1,
    );
    let ir = compile_work_item_plan(&placeholder_instruction, &context)
        .expect("manual_instruction 占位归一后仍可编译");
    assert_eq!(
        ir.items[0].verification_plan.checks[0].manual_instruction,
        None
    );
    assert_eq!(ir.items[0].trusted_commands[0].purpose, "CHECK-001");

    // 慎守：归一后 command 与 manual_instruction 同时缺位仍必须失败关闭。
    let guard = source.replacen(
        "- manual_instruction: Confirm the endpoint returns the configured levels JSON.\n",
        "",
        1,
    );
    let diagnostics = compile_work_item_plan(&guard, &context)
        .expect_err("null 归一后无 command 且无 manual_instruction 必须报错");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.field == "Verification"
            && diagnostic
                .message
                .contains("至少包含 command 或 manual_instruction")
    }));
}
