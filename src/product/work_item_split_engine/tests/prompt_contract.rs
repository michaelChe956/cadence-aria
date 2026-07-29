#[test]
fn single_item_prompt_scopes_writing_plans_to_pre_confirmation_candidate() {
    let outline = parse_work_item_plan_outline_output(valid_outline_author_output())
        .expect("outline output")
        .outline
        .expect("outline");

    let invocation = build_work_item_draft_invocation(
        &outline,
        "outline_backend",
        WorkItemGenerationMode::Serial,
        &[],
        None,
    )
    .expect("draft invocation");

    for required in [
        "当前仅处于 human-confirmation 之前的候选阶段",
        "writing-plans 的拆分、TDD、验证与交接质量纪律",
        "不得创建 cadence/plans/ 或任何 workspace 文件",
        "不得提前执行 writing-plans 的落盘步骤",
        "仅在最后一个 nonce sentinel block 返回唯一 Canonical Contract Candidate JSON",
        // 落盘职责条款并入 workspace 文件禁令（语义保留）。
        "canonical writeback 与正式 Plan 落盘由 human-confirmation gate 与 daemon 负责",
    ] {
        assert!(
            invocation.prompt.contains(required),
            "draft prompt must scope writing-plans persistence to a later phase; missing {required}: {}",
            invocation.prompt
        );
    }
}

#[test]
fn single_item_prompt_uses_compact_contract_without_duplicate_schema_or_outline() {
    let outline = parse_work_item_plan_outline_output(valid_outline_author_output())
        .expect("outline output")
        .outline
        .expect("outline");

    let invocation = build_work_item_draft_invocation(
        &outline,
        "outline_backend",
        WorkItemGenerationMode::Serial,
        &[],
        None,
    )
    .expect("draft invocation");

    assert!(invocation.prompt.contains("[confirmed_plan_trace]"));
    assert!(
        !invocation.prompt.contains("[confirmed_outline]"),
        "draft prompt must not duplicate the complete outline: {}",
        invocation.prompt
    );
    assert!(
        !invocation.prompt.contains("[output_schema]"),
        "parser schema must not be copied into the provider prompt: {}",
        invocation.prompt
    );
    assert!(
        invocation
            .prompt
            .contains("必须且只能含所列字段"),
        "draft prompt must retain the canonical contract field whitelist in the shorthand field contract: {}",
        invocation.prompt
    );
    assert!(
        invocation
            .prompt
            .contains("verification_plan: obj{checks:"),
        "draft prompt must retain the verification-plan field contract in the shorthand field contract: {}",
        invocation.prompt
    );
    assert!(
        invocation.prompt.len() < WORK_ITEM_DRAFT_PROMPT_QUALITY_BUDGET_BYTES,
        "draft prompt must remain below the quality budget: {} bytes",
        invocation.prompt.len()
    );
}

#[test]
fn single_item_prompt_accepts_maximum_legal_trusted_command_catalog_within_budget() {
    let mut outline = parse_work_item_plan_outline_output(valid_outline_author_output())
        .expect("outline output")
        .outline
        .expect("outline");
    let catalog = &mut outline.work_item_outlines[0].trusted_verification_commands;
    catalog.clear();
    for index in 0..3 {
        catalog.push(crate::product::models::TrustedDraftVerificationCommand {
            command: format!("{index}{}", "c".repeat(47)),
            cwd: format!("{index}{}", "w".repeat(15)),
            purpose: format!("{index}{}", "p".repeat(31)),
            source_ref: format!("{index}{}", "s".repeat(if index == 2 { 16 } else { 31 })),
        });
    }

    assert_eq!(
        crate::product::models::trusted_draft_verification_command_catalog_prompt_bytes(catalog),
        crate::product::models::MAX_TRUSTED_DRAFT_VERIFICATION_CATALOG_PROMPT_BYTES,
        "maximum legal catalog must use the prompt's exact rendered byte projection"
    );

    let invocation = build_work_item_draft_invocation(
        &outline,
        "outline_backend",
        WorkItemGenerationMode::Serial,
        &[],
        None,
    )
    .expect("maximum legal catalog must remain invocable");

    assert!(
        invocation.prompt.len() < WORK_ITEM_DRAFT_PROMPT_QUALITY_BUDGET_BYTES,
        "maximum legal catalog must remain under the quality budget: {} bytes",
        invocation.prompt.len()
    );
}

#[test]
fn single_item_prompt_rejects_short_catalog_that_exceeds_semantic_limit_before_provider_invocation()
{
    let mut outline = parse_work_item_plan_outline_output(valid_outline_author_output())
        .expect("outline output")
        .outline
        .expect("outline");
    let catalog = &mut outline.work_item_outlines[0].trusted_verification_commands;
    catalog.clear();
    for index in 0..4 {
        catalog.push(crate::product::models::TrustedDraftVerificationCommand {
            command: format!("test-{index}"),
            cwd: ".".to_string(),
            purpose: "unit".to_string(),
            source_ref: "repo/path".to_string(),
        });
    }
    let prompt = crate::product::work_item_split_engine::prompts::build_work_item_draft_prompt(
        &outline,
        &outline.work_item_outlines[0],
        WorkItemGenerationMode::Serial,
        &[],
        &[],
        None,
        "nonce",
    );
    assert!(
        prompt.len() < WORK_ITEM_DRAFT_PROMPT_MAX_BYTES,
        "semantic catalog violation must be checked before the full prompt hard backstop: {} bytes",
        prompt.len()
    );

    let error = build_work_item_draft_invocation(
        &outline,
        "outline_backend",
        WorkItemGenerationMode::Serial,
        &[],
        None,
    )
    .expect_err("short catalog exceeding maxItems must fail before provider invocation");

    assert_eq!(
        error.code,
        "trusted_verification_command_catalog_too_large"
    );
}

#[test]
fn single_item_prompt_rejects_oversized_trusted_command_catalog_before_provider_invocation() {
    let mut outline = parse_work_item_plan_outline_output(valid_outline_author_output())
        .expect("outline output")
        .outline
        .expect("outline");
    let catalog = &mut outline.work_item_outlines[0].trusted_verification_commands;
    catalog.clear();
    for index in 0..20 {
        catalog.push(crate::product::models::TrustedDraftVerificationCommand {
            command: format!("{index}{}", "c".repeat(47)),
            cwd: format!("{index}{}", "w".repeat(15)),
            purpose: format!("{index}{}", "p".repeat(31)),
            source_ref: format!("{index}{}", "s".repeat(31)),
        });
    }

    let error = build_work_item_draft_invocation(
        &outline,
        "outline_backend",
        WorkItemGenerationMode::Serial,
        &[],
        None,
    )
    .expect_err("oversized trusted catalog must fail before provider invocation");

    assert_eq!(
        error.code,
        "trusted_verification_command_catalog_too_large"
    );
    assert_eq!(
        error.message,
        "outline outline_backend trusted verification command catalog exceeds the maximum of 3 entries"
    );
}

#[test]
fn single_item_prompt_projects_planning_discipline_into_canonical_fields() {
    let outline = parse_work_item_plan_outline_output(valid_outline_author_output())
        .expect("outline output")
        .outline
        .expect("outline");

    let invocation = build_work_item_draft_invocation(
        &outline,
        "outline_backend",
        WorkItemGenerationMode::Serial,
        &[],
        None,
    )
    .expect("draft invocation");

    for required in [
        "Draft 专有 Canonical projection 优先于 [allowed_outputs] 的通用表述",
        "目标、范围和非目标映射到 identity、goal、write_policy、non_goals",
        "TDD 与验证映射到 tasks、acceptance_criteria、verification_checks",
        "依赖、交接和风险映射到 input_contracts、output_contracts、handoff_contract、blocker_rules",
        "不得输出 writing-plans 的 Markdown Plan 或新增 JSON 字段",
    ] {
        assert!(
            invocation.prompt.contains(required),
            "draft prompt must make the planning-to-canonical projection explicit; missing {required}: {}",
            invocation.prompt
        );
    }
}

#[test]
fn single_item_prompt_requires_observable_acceptance_criteria_and_forbids_process_evidence() {
    let outline = parse_work_item_plan_outline_output(valid_outline_author_output())
        .expect("outline output")
        .outline
        .expect("outline");

    let invocation = build_work_item_draft_invocation(
        &outline,
        "outline_backend",
        WorkItemGenerationMode::Serial,
        &[],
        None,
    )
    .expect("draft invocation");

    for required in [
        "acceptance criterion 的 statement 必须描述从最终代码状态、验证命令输出、人工检查结果或 handoff 字段可观测的结果状态",
        "禁止把提交历史、提交顺序、开发时序、分支操作历史作为 acceptance criterion",
        "non_zero_test_execution 表示验证命令执行时实际运行了非零数量的测试，是当前可观测的执行结果；它不表达测试曾先失败、不表达提交顺序、不表达任何开发时序。",
    ] {
        assert!(
            invocation.prompt.contains(required),
            "draft prompt must define observable acceptance-criterion boundaries; missing {required}: {}",
            invocation.prompt
        );
    }
}

#[test]
fn single_item_prompt_requires_registration_projection_and_self_check() {
    let outline = parse_work_item_plan_outline_output(valid_outline_author_output())
        .expect("outline output")
        .outline
        .expect("outline");

    let invocation = build_work_item_draft_invocation(
        &outline,
        "outline_backend",
        WorkItemGenerationMode::Serial,
        &[],
        None,
    )
    .expect("draft invocation");

    for required in [
        "[trusted_verification_command_catalog]",
        "[registration]",
        "[projection]",
        "[self_check]",
        "done_when_refs 只能引用 criterion_id",
        "required=true 的 command 必须逐字来自可信目录",
    ] {
        assert!(
            invocation.prompt.contains(required),
            "draft prompt must contain closed-contract protocol {required}: {}",
            invocation.prompt
        );
    }
}

#[test]
fn single_item_prompt_forbids_required_checks_without_a_trusted_command() {
    let outline = parse_work_item_plan_outline_output(valid_outline_author_output())
        .expect("outline output")
        .outline
        .expect("outline");

    let invocation = build_work_item_draft_invocation(
        &outline,
        "outline_backend",
        WorkItemGenerationMode::Serial,
        &[],
        None,
    )
    .expect("draft invocation");

    assert!(
        invocation.prompt.contains(
            "可信目录为空时，所有 verification_checks 必须 required=false 且 command=null"
        ),
        "draft prompt must forbid required checks without a trusted command"
    );
}

#[test]
fn single_item_prompt_requires_reviewer_checks_to_equal_criteria() {
    let outline = parse_work_item_plan_outline_output(valid_outline_author_output())
        .expect("outline output")
        .outline
        .expect("outline");

    let invocation = build_work_item_draft_invocation(
        &outline,
        "outline_backend",
        WorkItemGenerationMode::Serial,
        &[],
        None,
    )
    .expect("draft invocation");

    assert!(
        invocation.prompt.contains(
            "reviewer_check_refs 必须与全部且仅 acceptance criterion ID 集合完全一致"
        ),
        "draft prompt must require reviewer checks to contain only acceptance criterion IDs"
    );
}

#[test]
fn single_item_prompt_includes_closed_typed_canonical_field_contract() {
    let outline = parse_work_item_plan_outline_output(valid_outline_author_output())
        .expect("outline output")
        .outline
        .expect("outline");

    let invocation = build_work_item_draft_invocation(
        &outline,
        "outline_backend",
        WorkItemGenerationMode::Serial,
        &[],
        None,
    )
    .expect("draft invocation");

    for required in [
        "[canonical_field_contract]",
        "schema_version: integer literal 1",
        "goal: obj{summary: string}",
        "tasks: [obj{task_id: str+, statement: string, requirement_refs: [string], done_when_refs: [string]}]",
        "blocker_rules: [obj{reason_code: str+, route:",
        "design_traceability: [obj{source_type: string, source_id: string, requirement_id: string}]",
    ] {
        assert!(
            invocation.prompt.contains(required),
            "draft prompt must define its closed typed Canonical fields; missing {required}: {}",
            invocation.prompt
        );
    }
}

#[test]
fn work_item_plan_prompts_keep_json_contract_without_markdown_schema() {
    let outline = parse_work_item_plan_outline_output(valid_outline_author_output())
        .expect("outline output")
        .outline
        .expect("outline");

    let invocation = build_work_item_draft_invocation(
        &outline,
        "outline_backend",
        WorkItemGenerationMode::Serial,
        &[],
        None,
    )
    .expect("draft invocation");

    assert!(invocation.prompt.contains("[canonical_field_contract]"));
    assert!(invocation.prompt.contains("[cadence_project_rules]"));
    assert!(invocation.prompt.contains("AGENTS.md"));
    assert!(invocation.prompt.contains("CLAUDE.md"));
    assert!(
        !invocation
            .prompt
            .contains(&["Cadence-", "skills/"].concat())
    );
    assert!(
        !invocation.prompt.contains("[artifact_schema_contract]"),
        "Work Item Plan JSON prompt must not receive a Markdown artifact schema: {}",
        invocation.prompt
    );
    assert!(
        !invocation.prompt.contains("[artifact_schema_review_gate]"),
        "Work Item Plan JSON prompt must not receive a Markdown reviewer gate: {}",
        invocation.prompt
    );
}

#[test]
fn single_item_prompt_relaxes_handoff_provided_contract_refs_for_terminal_items() {
    let outline = parse_work_item_plan_outline_output(valid_outline_author_output())
        .expect("outline output")
        .outline
        .expect("outline");

    let invocation = build_work_item_draft_invocation(
        &outline,
        "outline_backend",
        WorkItemGenerationMode::Serial,
        &[],
        None,
    )
    .expect("draft invocation");

    for required in [
        "provided_contract_refs: 唯一 str+ 数组（无下游消费者时为空数组）",
        "provided_contract_refs 元素唯一且非空白",
        "仅列出被下游 WorkItem input_contracts 消费的契约 ref",
        "无下游消费者（链路末端）时必须为空数组",
    ] {
        assert!(
            invocation.prompt.contains(required),
            "draft prompt must state the terminal-item handoff rule; missing {required}: {}",
            invocation.prompt
        );
    }
    assert!(
        !invocation
            .prompt
            .contains("required_fields、provided_contract_refs、reviewer_check_refs 均非空且不重复"),
        "draft prompt must not require non-empty provided_contract_refs: {}",
        invocation.prompt
    );
    assert!(
        invocation.prompt.len() < WORK_ITEM_DRAFT_PROMPT_QUALITY_BUDGET_BYTES,
        "draft prompt must remain below the quality budget: {} bytes",
        invocation.prompt.len()
    );
}
