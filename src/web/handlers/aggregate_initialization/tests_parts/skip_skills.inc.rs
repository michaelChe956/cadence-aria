// -------------------------------------------------------------------
// owner 2026-10-29：聚合初始化「跳过四条 Cadence-skills 命令」与
// 「重新初始化（聚合）」入口的生产同构验收。
//
// fixture 复用 trust_route 的生产同形依赖图（gateway factory 携带
// `Some(paths)` + 跳过式政策发布面）。差别只在聚合根是否预先具备两个
// 判定文件（`AGENTS.md`、`.claude/rules/language.md`）。
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

/// 验收 ①④：聚合根已初始化 → 跳过四条命令（零 provider 启动、零命令
/// 审计、无跳过反证），聚合政策真实发布、跳过式 receipt 冻结，判定依据
/// 可从 operation 记录读到。
#[tokio::test]
async fn initialized_root_skips_the_recipe_but_publishes_the_aggregate_policy() {
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
    assert_eq!(
        operation
            .steps
            .iter()
            .map(|step| step.status.as_str())
            .collect::<Vec<_>>(),
        vec!["completed", "completed", "skipped", "skipped", "skipped"]
    );
    assert_eq!(
        fixture.factory.audit().stream_launches(),
        0,
        "an initialized root must start zero provider turns"
    );
    assert!(
        root_policy_receipts(&fixture)
            .list_commands("project_0001", &operation_id)
            .expect("command receipts")
            .is_empty(),
        "no command may be audited when the four commands were skipped"
    );

    // ④ 判定依据：判定类别 + reason + 两个判定文件摘要。
    let decision = operation
        .skills_initialization
        .clone()
        .expect("skip decision must be durable on the operation");
    assert_eq!(
        decision.kind,
        crate::product::logical_codebase::aggregate_initialization::AggregateSkillsInitializationKind::Skipped
    );
    assert_eq!(
        decision.reason_code,
        crate::product::logical_codebase::aggregate_initialization::AGGREGATE_SKILLS_SKIPPED_REASON
    );
    assert_eq!(
        decision
            .markers
            .iter()
            .map(|marker| marker.relative_path.as_str())
            .collect::<Vec<_>>(),
        vec!["AGENTS.md", ".claude/rules/language.md"]
    );
    assert_eq!(
        decision.markers[0].digest,
        format!("sha256:{:x}", Sha256::digest(PREINITIALIZED_AGENTS.as_bytes()))
    );

    // 政策发布成功：artifact 是真正的聚合政策正文（非自举桩），locator
    // 字节与正文一致。
    let canonical_root = root_policy_canonical_root(&fixture);
    let artifact = root_policy_store(&fixture)
        .get("project_0001")
        .expect("policy artifact read")
        .expect("published policy artifact");
    assert!(
        !artifact.is_bootstrap_placeholder(),
        "skipped initialization must publish the real aggregate policy"
    );
    assert_eq!(
        artifact.policy_text,
        root_policy_expected_text(PREINITIALIZED_AGENTS, PREINITIALIZED_LANGUAGE)
    );
    assert_eq!(
        std::fs::read(canonical_root.join(&artifact.policy_id)).expect("policy locator bytes"),
        artifact.policy_text.as_bytes()
    );

    // 跳过式最终 receipt：无命令摘要、冻结 policy/rule 双摘要与判定依据。
    let receipt = root_policy_receipts(&fixture)
        .get("project_0001", &operation_id)
        .expect("final receipt read")
        .expect("skipped final receipt");
    assert!(
        receipt.commands.is_empty(),
        "a skipped receipt must not claim any audited command"
    );
    assert_eq!(receipt.canonical_root, canonical_root);
    assert_eq!(receipt.policy_digest, artifact.digest);
    assert_eq!(
        receipt.rule_digest,
        crate::product::logical_codebase::root_recipe_receipt::root_rule_digest(&canonical_root)
            .expect("root rule digest")
            .expect("root rule entry")
    );
    let receipt_decision = receipt
        .skills_initialization
        .as_ref()
        .expect("skipped receipt must carry the skip evidence");
    assert!(receipt_decision.is_skipped());
    assert_eq!(receipt_decision.markers, decision.markers);

    // 前端可读的步骤状态同样暴露 skipped（GET 投影）。
    let uri = format!(
        "/api/projects/project_0001/logical-codebases/{}/initializations/{operation_id}",
        fixture.lc_id
    );
    let (status, body) = get_json(&fixture.app, &uri).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["steps"]
            .as_array()
            .expect("steps")
            .iter()
            .map(|step| step["status"].as_str().unwrap_or_default())
            .collect::<Vec<_>>(),
        vec!["completed", "completed", "skipped", "skipped", "skipped"]
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
    assert!(operation.skills_initialization.is_none());
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
