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
        &RoutingReferenceContext::Legacy,
    )
    .expect("draft invocation");

    for required in [
        "不得创建 cadence/plans/ 或任何 workspace 文件",
        "不得提前执行 writing-plans 的落盘步骤",
        "仅在最后一个 nonce sentinel block 返回唯一 Canonical Contract Candidate JSON",
        // 落盘职责条款并入 workspace 文件禁令（语义保留）。
        "canonical writeback 与正式 Plan 落盘由 human-confirmation gate 与 daemon 负责",
    ] {
        assert!(
            invocation.prompt.contains(required),
            "draft prompt must retain the C-layer output/writeback boundary; missing {required}: {}",
            invocation.prompt
        );
    }
    assert!(
        !invocation
            .prompt
            .contains("writing-plans 的拆分、TDD、验证与交接质量纪律"),
        "draft prompt must not teach the B-layer planning process: {}",
        invocation.prompt
    );
}

#[test]
fn single_item_prompt_behavior_layer_removed_but_output_boundaries_retained() {
    let mut outline = parse_work_item_plan_outline_output(valid_outline_author_output())
        .expect("outline output")
        .outline
        .expect("outline");
    outline.work_item_outlines[0].target_repository_id = Some(
        crate::product::logical_codebase::LogicalRepositoryId(uuid::Uuid::from_u128(1)),
    );
    let invocation = build_work_item_draft_invocation(
        &outline,
        "outline_backend",
        WorkItemGenerationMode::Serial,
        &[],
        None,
        &RoutingReferenceContext::Legacy,
    )
    .expect("draft invocation");

    for removed in [
        "必调 Skill：using-superpowers → writing-plans。",
        "[superpowers_contract]",
        "必须遵守 using-superpowers 的先读规则与 writing-plans 的计划结构要求。",
        "生成的是计划和任务拆分，不执行代码修改。",
        "每个 outline/draft 必须给出后续 coding agent 可执行的目标、范围、非目标、TDD 顺序、结构化验证方案、依赖输入、交接输出和风险；其中 draft 只有存在目标仓库可信证据时才可给出 command，证据不足必须进入 manual/repair/blocker，不得臆造命令。",
        "每个 outline/draft 的 TDD 与验证闭环必须在当前项的 exclusive_write_scopes 和已完成 depends_on handoff 下实际可执行；不得把后续 Work Item 才会提供的注册、接线、生成或部署作为当前项验证的前提。无法根据目标仓库事实建立该闭环时，必须调整拆分或进入既有 repair/blocker 路由。",
        "当前仅处于 human-confirmation 之前的候选阶段：必须读取并遵守 writing-plans 的拆分、TDD、验证与交接质量纪律；只将这些纪律体现在本候选中。",
    ] {
        assert!(
            !invocation.prompt.contains(removed),
            "B-layer text must be absent: {removed}: {}",
            invocation.prompt
        );
    }
    assert!(!invocation.prompt.contains("[superpowers_contract]"));

    for retained in [
        "不得输出 writing-plans 的 Markdown Plan 或新增 JSON 字段",
        "不得提前执行 writing-plans 的落盘步骤",
        "当前 Draft 的 target_repository_id 必须逐字保留 [current_work_item_outline] 的 target_repository_id",
        "source_story_spec_ids/source_design_spec_ids",
        "00000000-0000-0000-0000-000000000001",
        "[openspec_contract]",
        "[allowed_outputs]",
        "[forbidden_outputs]",
        "exclusive_scopes",
        "verification_checks",
        "handoff_contract",
        "候选，不能写入 canonical artifact",
    ] {
        assert!(
            invocation.prompt.contains(retained),
            "C-layer contract text must remain: {retained}: {}",
            invocation.prompt
        );
    }
    assert_eq!(
        invocation
            .prompt
            .matches("不得输出 writing-plans 的 Markdown Plan 或新增 JSON 字段")
            .count(),
        1
    );
    assert_eq!(
        invocation
            .prompt
            .matches("不得提前执行 writing-plans 的落盘步骤")
            .count(),
        1
    );
}

#[test]
fn single_item_prompt_merge_gate_keeps_c_layer_and_excludes_b_layer_schema() {
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
        &RoutingReferenceContext::Legacy,
    )
    .expect("draft invocation");

    for required in [
        "[openspec_contract]",
        "[canonical_field_contract]",
        "[allowed_outputs]",
        "[forbidden_outputs]",
        "不得输出 writing-plans 的 Markdown Plan 或新增 JSON 字段",
        "候选，不能写入 canonical artifact",
    ] {
        assert!(
            invocation.prompt.contains(required),
            "prompt merge gate must retain C-layer contract: {required}"
        );
    }
    for forbidden in [
        "writing-plans 的拆分、TDD、验证与交接质量纪律",
        "[superpowers_contract]",
        "[output_schema]",
        "[artifact_schema_contract]",
    ] {
        assert!(
            !invocation.prompt.contains(forbidden),
            "prompt merge gate must exclude B-layer or duplicated schema: {forbidden}"
        );
    }
    assert!(
        invocation.prompt.len() < WORK_ITEM_DRAFT_PROMPT_QUALITY_BUDGET_BYTES
            && invocation.prompt.len() < WORK_ITEM_DRAFT_PROMPT_MAX_BYTES,
        "prompt must remain within quality and hard byte limits: {} bytes",
        invocation.prompt.len()
    );
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
        &RoutingReferenceContext::Legacy,
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
        invocation.prompt.contains("必须且只能含所列字段"),
        "draft prompt must retain the canonical contract field whitelist in the shorthand field contract: {}",
        invocation.prompt
    );
    assert!(
        invocation.prompt.contains("verification_plan: obj{checks:"),
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
        &RoutingReferenceContext::Legacy,
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
        &RoutingReferenceContext::Legacy,
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
        &RoutingReferenceContext::Legacy,
    )
    .expect_err("short catalog exceeding maxItems must fail before provider invocation");

    assert_eq!(error.code, "trusted_verification_command_catalog_too_large");
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
        &RoutingReferenceContext::Legacy,
    )
    .expect_err("oversized trusted catalog must fail before provider invocation");

    assert_eq!(error.code, "trusted_verification_command_catalog_too_large");
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
        &RoutingReferenceContext::Legacy,
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
        &RoutingReferenceContext::Legacy,
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
        &RoutingReferenceContext::Legacy,
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

/// 可信目录为空时禁止伪造命令，但人工核对仍可 required=true。
///
/// 实测死循环：旧规则要求「可信目录为空时所有 verification_checks 必须
/// required=false」，把人工核对也一起禁掉。纯静态页面这类没有测试框架的 outline
/// 因此无法把任何验收标准设为必需，而 reviewer 按 outline 的 verification_intent
/// 反复要求 required=true——author 与 reviewer 各自遵守指令，结论必然冲突。
#[test]
fn single_item_prompt_routes_manual_items_to_acceptance_criteria() {
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
        &RoutingReferenceContext::Legacy,
    )
    .expect("draft invocation");

    for required in [
        "可信目录为空时所有 check 必须 command=null",
        "需人工操作或目视确认的 verification_intent 必须表达为 acceptance_criteria 的 required_evidence=[manual_check]",
        "verification_checks 的 required=true 仅限 Coder 可自行执行的命令或只读检查",
        "人工事项由末端人工确认，不构成自动阶段阻塞",
    ] {
        assert!(
            invocation.prompt.contains(required),
            "draft prompt must route manual items to acceptance criteria, not Coder execution; missing {required}"
        );
    }
    assert!(
        !invocation
            .prompt
            .contains("所有 verification_checks 必须 required=false"),
        "draft prompt must not force every check to be optional when the catalog is empty"
    );
    assert!(
        !invocation.prompt.contains("人工核对必须 required=true"),
        "draft prompt must not make manual checks a Coder delivery precondition"
    );
}

/// 空可信目录的占位文本也不得暗示「人工核对只能可选」。
#[test]
fn empty_trusted_command_catalog_placeholder_allows_required_manual_checks() {
    let mut outline = parse_work_item_plan_outline_output(valid_outline_author_output())
        .expect("outline output")
        .outline
        .expect("outline");
    outline.work_item_outlines[0]
        .trusted_verification_commands
        .clear();

    let invocation = build_work_item_draft_invocation(
        &outline,
        "outline_backend",
        WorkItemGenerationMode::Serial,
        &[],
        None,
        &RoutingReferenceContext::Legacy,
    )
    .expect("draft invocation");

    assert!(
        invocation
            .prompt
            .contains("Manual checks with command=null may still be required=true"),
        "empty-catalog placeholder must state that manual checks can remain required"
    );
    assert!(
        invocation
            .prompt
            .contains("the draft MUST include a route=operational_gate blocker"),
        "empty-catalog placeholder must require an operational_gate blocker"
    );
    assert!(
        invocation
            .prompt
            .contains("manual check (command=null) cannot substitute for that blocker"),
        "empty-catalog placeholder must forbid substituting a manual check for the operational_gate blocker"
    );
}

/// Draft Prompt 正文必须与现行校验器硬规则逐字对齐：
/// - 可信目录为空时 draft 必须含 route=operational_gate 的 blocker（manual check 不能替代）。
/// - plan_repair_current / plan_repair_upstream / subgraph_replan 路由 blocker 的
///   target_contract_refs 必须非空且每个 ref 逐字等于已登记 input/output contract_id。
///
/// 旧冲突句「不得因缺人工环境输出 operational_gate blocker」必须消失。
#[test]
fn single_item_prompt_aligns_draft_hard_rules_with_validators() {
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
        &RoutingReferenceContext::Legacy,
    )
    .expect("draft invocation");

    for required in [
        "不得把可由人工确认的事项升级为 operational_gate",
        "可信目录为空时，必须输出 route=operational_gate blocker；manual check 不能替代该 blocker",
        "plan_repair_current / plan_repair_upstream / subgraph_replan 路由的 blocker，target_contract_refs 必须非空，且每个 ref 逐字等于已登记 input/output contract_id",
    ] {
        assert!(
            invocation.prompt.contains(required),
            "draft prompt must state validator hard rules; missing {required}"
        );
    }
    assert!(
        !invocation
            .prompt
            .contains("不得因缺人工环境输出 operational_gate blocker"),
        "draft prompt must not keep the contradictory clause that forbids operational_gate when manual environment is missing"
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
        &RoutingReferenceContext::Legacy,
    )
    .expect("draft invocation");

    assert!(
        invocation
            .prompt
            .contains("reviewer_check_refs 必须与全部且仅 acceptance criterion ID 集合完全一致"),
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
        &RoutingReferenceContext::Legacy,
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
        &RoutingReferenceContext::Legacy,
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
        &RoutingReferenceContext::Legacy,
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
        !invocation.prompt.contains(
            "required_fields、provided_contract_refs、reviewer_check_refs 均非空且不重复"
        ),
        "draft prompt must not require non-empty provided_contract_refs: {}",
        invocation.prompt
    );
    assert!(
        invocation.prompt.len() < WORK_ITEM_DRAFT_PROMPT_QUALITY_BUDGET_BYTES,
        "draft prompt must remain below the quality budget: {} bytes",
        invocation.prompt.len()
    );
}

/// input_contracts.contract_id 是对上游的引用而非新命名。
///
/// 实测缺陷：prompt 只说 input_contracts 有 contract_id 字段，未约束它必须逐字等于
/// 上游 output_contracts 的 ID，provider 因此按 ic_/oc_ 前缀自行命名，导致依赖契约图
/// 校验报 required_contract_missing + unconsumed_required_handoff。
#[test]
fn draft_prompt_requires_input_contract_ids_to_reference_upstream_output_contracts() {
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
        &RoutingReferenceContext::Legacy,
    )
    .expect("draft invocation");

    for required in [
        "input_contracts 的 contract_id 与 required_capabilities 元素都是对上游的引用而非新命名",
        "不得改写前缀（如 oc_ 换成 ic_）、意译或自行描述",
        "provider_logical_work_item_id 必须是真正声明该 contract 的上游 logical_work_item_id",
        "输出前把每个 input_contracts 的 contract_id 与 required_capabilities 元素在 [直接依赖的可消费交接合同] 中做字面量查找",
    ] {
        assert!(
            invocation.prompt.contains(required),
            "draft prompt must constrain input contract references; missing {required}"
        );
    }
}

/// Pi once emitted every `required_evidence` as a scalar string because the schema
/// notation `[source_diff|...]` could be read as "pick one". The field contract must
/// spell out that the value is an array even for a single element.
#[test]
fn single_item_prompt_spells_out_required_evidence_as_array() {
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
        &RoutingReferenceContext::Legacy,
    )
    .expect("draft invocation");

    assert!(
        invocation.prompt.contains(
            "required_evidence: [source_diff|non_zero_test_execution|manual_check|handoff_field]（必为数组，单元素也需成数组）"
        ),
        "draft prompt must spell out required_evidence as an array to prevent scalar output"
    );
}

/// Pi omitted `canonical_contract.verification_checks` entirely because the field
/// contract listed it on the same line as the draft-level `verification_plan`,
/// reading as one field. The contract must name both owners explicitly and say
/// both have to be emitted.
#[test]
fn single_item_prompt_names_both_verification_check_owners() {
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
        &RoutingReferenceContext::Legacy,
    )
    .expect("draft invocation");

    for required in [
        "canonical_contract.verification_checks",
        "（canonical_contract 必填字段）",
        "draft.verification_plan",
        "两处都必须输出，不得只写一处",
    ] {
        assert!(
            invocation.prompt.contains(required),
            "draft prompt must name both verification check owners; missing {required}"
        );
    }
}

#[test]
fn work_item_plan_markdown_prompt_inlines_grammar_boundaries_and_real_findings() {
    let (request, issue, repository) = split_prompt_fixture();
    let prompt =
        crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_prompt(
            &request,
            &issue,
            &repository,
            crate::product::work_item_split_engine::prompts::WorkItemPlanMarkdownAuthorContext {
                story_context: "story_spec_0001: level selection",
                design_context: "design_spec_0001: levels API",
                repository_structure: "src/product/levels; web/src/levels; tests/integration",
                routing_context: &RoutingReferenceContext::Legacy,
                trusted_command_catalog: &[],
            },
        )
        .expect("markdown author prompt");

    for section in crate::product::work_item_plan_compiler::grammar::STRUCTURED_SECTIONS {
        let heading = format!("### {section}");
        assert!(
            prompt.contains(&heading),
            "markdown prompt must inline every structured section; missing {heading}"
        );
    }
    for required in [
        crate::product::work_item_plan_compiler::grammar::EARS_STATEMENT_TEMPLATE,
        "未知结构化 key 必须拒绝（fail_closed）",
        "不得从 issue、outline、prompt 或 runtime 补齐 markdown 缺失字段",
        "明确允许新增和维护 tests/integration/**",
        "GET / 只验证三个容器与 level-select.js 加载",
        "通过静态脚本响应或等价可执行证据验证 web/level-select.js 对 /api/levels 的引用",
    ] {
        assert!(
            prompt.contains(required),
            "markdown prompt must retain grammar/boundary/few-shot evidence; missing {required}"
        );
    }

    let dependency_key = crate::product::work_item_plan_compiler::grammar::DEPENDENCIES_KEY;
    let item_id_prefix = crate::product::work_item_plan_compiler::grammar::ITEM_ID_PREFIX;
    for example in [
        format!("`- {dependency_key}: []`"),
        format!("`- {dependency_key}: {item_id_prefix}001`"),
        format!("`- {dependency_key}: {item_id_prefix}001`\n`- {dependency_key}: {item_id_prefix}002`"),
    ] {
        assert!(
            prompt.contains(&example),
            "完整 author 必须教授合法 Dependencies 形态：{example}"
        );
    }
    assert!(
        prompt.contains("禁止括号列表、空格或逗号分隔多值"),
        "完整 author 必须禁止 parser 不接受的 Dependencies 多值写法"
    );

    let golden: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/product/work_item_plan_policy/fixtures/golden_findings.json"
    )))
    .expect("golden findings JSON");
    let expected_few_shot_ids = crate::product::work_item_split_engine::prompts::WORK_ITEM_PLAN_FEW_SHOT_IDS
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    let selected_few_shot_ids = golden
        .as_array()
        .expect("golden findings array")
        .iter()
        .filter_map(|entry| {
            let id = entry.get("id")?.as_str()?;
            expected_few_shot_ids.contains(id).then_some((id, entry))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        selected_few_shot_ids
            .iter()
            .map(|(id, _)| *id)
            .collect::<std::collections::BTreeSet<_>>(),
        expected_few_shot_ids,
        "markdown few-shot selection must be keyed by the exported raw-provider ID set, not fixture order"
    );
    for (id, entry) in selected_few_shot_ids {
        let finding = entry.get("finding").expect("finding payload");
        for field in ["message", "evidence", "required_action"] {
            let value = finding
                .get(field)
                .and_then(serde_json::Value::as_str)
                .expect("provider raw finding field");
            assert!(
                prompt.contains(value),
                "markdown prompt must inline provider raw few-shot {id} field {field}: {value}"
            );
        }
    }
    for forbidden in [
        "请读取 aria 仓 fixture",
        "<ARIA_STRUCTURED_OUTPUT",
        "canonical_field_contract",
        "class_hint",
        "annotated_variant",
        "human_annotation",
        "annotated_fields",
        "rep2-f1-annotated",
        "rep3-f1-annotated",
        "rep4-f1-annotated",
    ] {
        assert!(
            !prompt.contains(forbidden),
            "markdown author prompt must not retain private JSON/sentinel/classifier instruction {forbidden}: {prompt}"
        );
    }
    let (_, minimum_source) = prompt
        .split_once("[minimum_legal_source] 仅示语法形状；按当前上下文替换，勿照抄。\n")
        .expect("markdown prompt must inline a minimum source");
    let (minimum_source, _) = minimum_source
        .split_once("[real_finding_few_shot]")
        .expect("minimum source must precede few-shot findings");
    assert_eq!(
        crate::product::work_item_plan_compiler::parse_work_item_plan(minimum_source)
            .expect("minimum prompt source must satisfy the compiler grammar")
            .items
            .len(),
        1
    );
    assert!(
        prompt.len() < WORK_ITEM_DRAFT_PROMPT_QUALITY_BUDGET_BYTES,
        "markdown prompt must remain below the existing quality budget: {} bytes",
        prompt.len()
    );
}

#[test]
fn work_item_plan_markdown_outline_prompt_is_parser_oriented_and_excludes_full_author_few_shot() {
    let (request, issue, repository) = split_prompt_fixture();
    let prompt = crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_outline_prompt(
        &request,
        &issue,
        &repository,
        "story_spec_0001: level selection",
        "design_spec_0001: levels API",
        "src/product/levels; web/src/levels; tests/integration",
        &RoutingReferenceContext::Legacy,
    )
    .expect("markdown outline prompt");

    assert!(prompt.contains("用于服务端机械计数"));
    assert!(prompt.contains("[markdown_grammar]"));
    assert!(prompt.contains("[minimum_legal_source]"));
    assert!(prompt.contains("不得输出 JSON、code fence、解释、source hash"));

    let dependency_key = crate::product::work_item_plan_compiler::grammar::DEPENDENCIES_KEY;
    assert!(
        crate::product::work_item_plan_compiler::grammar::STRUCTURED_KEYS.contains(&dependency_key),
        "Dependencies key must remain part of the markdown grammar whitelist"
    );
    let item_id_prefix = crate::product::work_item_plan_compiler::grammar::ITEM_ID_PREFIX;
    assert!(
        prompt.contains(&format!(
            "每个 Work Item 必须以 `{item_id_prefix}<三位数字>` 编号，从 `{item_id_prefix}001` 起"
        )),
        "轻量 outline 必须明确从 WI-001 开始的编号规则"
    );
    for example in [
        format!("`- {dependency_key}: []`"),
        format!("`- {dependency_key}: {item_id_prefix}001`"),
        format!("`- {dependency_key}: {item_id_prefix}001`\n`- {dependency_key}: {item_id_prefix}002`"),
    ] {
        assert!(
            prompt.contains(&example),
            "轻量 outline 必须给出合法 Dependencies 形态：{example}"
        );
    }
    assert!(
        prompt.contains("禁止括号列表、空格或逗号分隔多值"),
        "轻量 outline 必须禁止 parser 不接受的 Dependencies 多值写法"
    );
    assert!(
        prompt.contains(&format!(
            "值仅 `[]` 或 `{item_id_prefix}<digits>`",
        )),
        "轻量 outline 必须仅允许 grammar 定义的 Dependencies 值"
    );
    assert!(
        !prompt.contains("[real_finding_few_shot]"),
        "轻量 outline 不得携带完整 author 的 few-shot"
    );
    assert!(
        !prompt.contains("原始输出直接成为 source revision"),
        "轻量 outline 不得伪装成 full markdown source author"
    );
    assert!(prompt.len() < WORK_ITEM_DRAFT_PROMPT_QUALITY_BUDGET_BYTES);
}

#[test]
fn work_item_plan_markdown_mechanical_count_uses_compiler_parser() {
    let source = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/openspec/changes/rearch-workitem-plan-pipeline/fixtures/work-item-plan-rep4.md"
    ));

    assert_eq!(
        crate::product::work_item_split_engine::parse::count_work_item_plan_candidates(source)
            .expect("rep4 markdown must pass the compiler parser"),
        3,
        "candidate count must come from the parsed markdown AST rather than client input"
    );
}

#[test]
fn work_item_plan_markdown_mechanical_count_fails_closed_on_invalid_source() {
    let error = crate::product::work_item_split_engine::parse::count_work_item_plan_candidates(
        "# Work Item Plan\n\n## Work Item WI-001: invalid\n",
    )
    .expect_err("invalid markdown outline must not produce a guessed candidate count");

    assert!(
        error
            .iter()
            .any(|diagnostic| diagnostic.code == "missing_section"),
        "compiler diagnostics must be returned for an invalid source: {error:?}"
    );
}
