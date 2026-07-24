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
        "仅在最后一个 nonce sentinel block 返回 Canonical Contract Candidate JSON",
        "human-confirmation gate 与 daemon 后续负责 canonical writeback 和正式 Plan 落盘",
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
            .contains("canonical_contract 必须且只能包含 schema_version"),
        "draft prompt must retain the canonical contract field whitelist: {}",
        invocation.prompt
    );
    assert!(
        invocation
            .prompt
            .contains("verification_plan 只能包含 checks"),
        "draft prompt must retain the verification-plan field contract: {}",
        invocation.prompt
    );
    assert!(
        invocation.prompt.len() < 11_000,
        "draft prompt must remain below the provider-context guardrail: {} bytes",
        invocation.prompt.len()
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
        "goal: object {summary: string}",
        "tasks: array of {task_id: non-empty string, statement: string, requirement_refs: string array, done_when_refs: string array}",
        "blocker_rules: array of {reason_code: non-empty string, route:",
        "design_traceability: array of {source_type: string, source_id: string, requirement_id: string}",
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
    assert!(invocation.prompt.contains("[cadence_original_routing_rules]"));
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
