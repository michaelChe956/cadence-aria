// -------------------------------------------------------------------
// owner 2026-10-29 入口语义：聚合初始化「跳过四条 Cadence-skills 命令」
// 只发生在**冷启动自动推进**（bootstrap/advance，full_recipe_requested =
// false，走「根已初始化」检测）；**手动**「启动聚合初始化」与「重新初始化」
// 一律跑完整五步（含四条命令）——skills 更新后的重装途径。
//
// fixture 复用 trust_route 的生产同形依赖图（gateway factory 携带
// `Some(paths)` + 跳过式政策发布面）。差别只在聚合根是否预先具备两个
// 判定文件（`AGENTS.md`、`.claude/rules/language.md`）。
// 跳过式回执的断言在 `tests_parts/bootstrap_advance.inc.rs`（advance 路径）。
// -------------------------------------------------------------------

/// 预写「聚合根已做过 Cadence-skills 初始化」的两个判定文件（相对聚合根）。
const PREINITIALIZED_AGENTS: &str = "# 聚合根规则\n\n- 成员统一遵守语言规范。\n";
const PREINITIALIZED_LANGUAGE: &str = "# 语言规则\n\n- 必须使用中文回答。\n";

fn preinitialize_aggregate_root(fixture: &TrustRouteFixture) {
    let root = fixture.root.join("aggregate-root");
    std::fs::create_dir_all(&root).expect("aggregate root");
    std::fs::write(root.join("AGENTS.md"), PREINITIALIZED_AGENTS).expect("write root AGENTS.md");
    std::fs::create_dir_all(root.join(".claude").join("rules")).expect("root rules dir");
    std::fs::write(
        root.join(".claude").join("rules").join("language.md"),
        PREINITIALIZED_LANGUAGE,
    )
    .expect("write root language rule");
}

/// POST 重新初始化入口（owner 2026-10-29），返回 operation_id。
async fn root_reinitialization_start(fixture: &TrustRouteFixture, idempotency_key: &str) -> String {
    let uri = format!(
        "/api/projects/project_0001/logical-codebases/{}/reinitializations",
        fixture.lc_id
    );
    let (status, body) = response_json(
        post_json(
            &fixture.app,
            &uri,
            serde_json::json!({"idempotency_key": idempotency_key}),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    body["operation_id"]
        .as_str()
        .expect("operation id")
        .to_string()
}

/// owner 2026-10-29 入口语义：**手动**点「启动聚合初始化」在**已初始化
/// 根**上仍必须跑完整五步（含四条 Cadence-skills 命令）——skills 更新后
/// 这是唯一的重装途径。跳过式检测只属于冷启动自动推进，见
/// `tests_parts/bootstrap_advance.inc.rs` 的 advance 用例。
#[tokio::test]
async fn manual_initialization_runs_every_command_on_an_initialized_root() {
    let fixture = trust_route_fixture(false);
    preinitialize_aggregate_root(&fixture);
    let operations = root_policy_operations(&fixture);
    let operation_id = root_policy_start(&fixture, "skip-skills-initialized-1").await;

    let operation = root_policy_poll_terminal(&operations, &operation_id).await;
    assert_eq!(
        operation.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Completed,
        "skipped initialization must still complete: {:?}",
        operation.error
    );
    assert!(
        operation
            .steps
            .iter()
            .all(|step| step.status.as_str() == "completed"),
        "a manual initialization always runs every step: {:?}",
        operation
            .steps
            .iter()
            .map(|step| step.status.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        fixture.factory.audit().stream_launches(),
        3,
        "a manual initialization must run the full recipe even on an initialized root"
    );
    assert_eq!(
        root_policy_receipts(&fixture)
            .list_commands("project_0001", &operation_id)
            .expect("command receipts")
            .len(),
        4,
        "all four recipe commands must be audited on the manual path"
    );

    // 手动路径记录 `forced` 判定（入口已是「用户明确要求跑完整五步」），
    // 绝不落 `skipped`。
    let decision = operation
        .skills_initialization
        .clone()
        .expect("a manual initialization must record its forced decision");
    assert_eq!(
        decision.kind,
        crate::product::logical_codebase::aggregate_initialization::AggregateSkillsInitializationKind::Forced
    );

    // 政策发布成功：artifact 是真正的聚合政策正文（非自举桩），locator
    // 字节与正文一致。完整 recipe 的 RuleAndMcpConfig 命令由 provider 重写
    // 根规则材料，故冻结正文来自 provider 生成的两个来源。
    let canonical_root = root_policy_canonical_root(&fixture);
    let artifact = root_policy_store(&fixture)
        .get("project_0001")
        .expect("policy artifact read")
        .expect("published policy artifact");
    assert!(
        !artifact.is_bootstrap_placeholder(),
        "a manual initialization must publish the real aggregate policy"
    );
    assert_eq!(
        artifact.policy_text,
        root_policy_expected_text(ROOT_POLICY_TEST_AGENTS, ROOT_POLICY_TEST_LANGUAGE)
    );
    assert_eq!(
        std::fs::read(canonical_root.join(&artifact.policy_id)).expect("policy locator bytes"),
        artifact.policy_text.as_bytes()
    );

    // 完整 recipe receipt：四条命令摘要 + 冻结 policy/rule 双摘要，无跳过证据。
    let receipt = root_policy_receipts(&fixture)
        .get("project_0001", &operation_id)
        .expect("final receipt read")
        .expect("finalized root receipt");
    assert_eq!(
        receipt.commands.len(),
        4,
        "a manual receipt must carry all four command audits"
    );
    assert_eq!(receipt.canonical_root, canonical_root);
    assert_eq!(receipt.policy_digest, artifact.digest);
    assert_eq!(
        receipt.rule_digest,
        crate::product::logical_codebase::root_recipe_receipt::root_rule_digest(&canonical_root)
            .expect("root rule digest")
            .expect("root rule entry")
    );
    assert!(
        receipt.skills_initialization.is_none(),
        "a manual receipt must not carry skip evidence"
    );

    // 前端可读的步骤状态同样全 completed（GET 纯投影）。
    let uri = format!(
        "/api/projects/project_0001/logical-codebases/{}/initializations/{operation_id}",
        fixture.lc_id
    );
    let (status, body) = get_json(&fixture.app, &uri).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["steps"]
            .as_array()
            .expect("steps")
            .iter()
            .all(|step| step["status"] == "completed"),
        "manual initialization must project every step completed: {body}"
    );
}

/// 验收 ②：聚合根未初始化 → 仍走完整五步（三个 provider turn + 四条
/// 命令审计），不落跳过判定。
#[tokio::test]
async fn uninitialized_root_still_runs_the_full_recipe() {
    let fixture = trust_route_fixture(false);
    let operations = root_policy_operations(&fixture);
    let operation_id = root_policy_start(&fixture, "skip-skills-uninitialized-1").await;

    let operation = root_policy_poll_terminal(&operations, &operation_id).await;
    assert_eq!(
        operation.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Completed,
        "{:?}",
        operation.error
    );
    assert!(operation.steps.iter().all(|step| step.status.as_str() == "completed"));
    // 手动入口一律记录 `forced` 判定（入口本身即「跑完整五步」表达），
    // 未初始化根的行为与之前一致：四条命令照跑。
    let decision = operation
        .skills_initialization
        .clone()
        .expect("a manual initialization must record its forced decision");
    assert_eq!(
        decision.kind,
        crate::product::logical_codebase::aggregate_initialization::AggregateSkillsInitializationKind::Forced
    );
    assert_eq!(fixture.factory.audit().stream_launches(), 3);
    root_policy_all_commands_allowed(&fixture, &operation_id);
    let receipt = root_policy_receipts(&fixture)
        .get("project_0001", &operation_id)
        .expect("final receipt read")
        .expect("finalized root receipt");
    assert_eq!(receipt.commands.len(), 4);
    assert!(receipt.skills_initialization.is_none());
}

/// 验收 ③：重新初始化入口 → 即便聚合根已初始化也跑完整五步。
#[tokio::test]
async fn reinitialization_runs_every_command_on_an_initialized_root() {
    let fixture = trust_route_fixture(false);
    preinitialize_aggregate_root(&fixture);
    let operations = root_policy_operations(&fixture);
    let operation_id = root_reinitialization_start(&fixture, "skip-skills-forced-1").await;

    let operation = root_policy_poll_terminal(&operations, &operation_id).await;
    assert_eq!(
        operation.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Completed,
        "{:?}",
        operation.error
    );
    assert!(operation.full_recipe_requested);
    assert!(
        operation.steps.iter().all(|step| step.status.as_str() == "completed"),
        "a forced reinitialization must run all five steps: {:?}",
        operation
            .steps
            .iter()
            .map(|step| step.status.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        fixture.factory.audit().stream_launches(),
        3,
        "forced reinitialization must start all three provider turns"
    );
    let decision = operation
        .skills_initialization
        .clone()
        .expect("the forced decision must be recorded");
    assert_eq!(
        decision.kind,
        crate::product::logical_codebase::aggregate_initialization::AggregateSkillsInitializationKind::Forced
    );
    assert_eq!(
        decision.reason_code,
        crate::product::logical_codebase::aggregate_initialization::AGGREGATE_SKILLS_FORCED_REASON
    );
    root_policy_all_commands_allowed(&fixture, &operation_id);
    let receipt = root_policy_receipts(&fixture)
        .get("project_0001", &operation_id)
        .expect("final receipt read")
        .expect("finalized root receipt");
    assert_eq!(receipt.commands.len(), 4);
    assert!(
        receipt.skills_initialization.is_none(),
        "the forced path must freeze a full recipe receipt, not a skipped one"
    );
}
