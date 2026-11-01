// -------------------------------------------------------------------
// Task 4（aggregate-policy-root-publication / REQ-BOOT-05、REQ-ENV-12）：
// 生产末命令收口、失败传播与显式恢复。fixture 复用 trust_route 的
// 生产同构依赖图（Some(paths) driver + 规则材料在 RuleAndMcpConfig
// 命令时机生成）；中断 seam 见 production_dependencies.inc.rs 的
// `root_policy_test_faults`（仅 cfg(test)，按 operation 作用域隔离）。
// -------------------------------------------------------------------

/// LC-scoped operation store 便捷句柄。
fn root_policy_operations(fixture: &TrustRouteFixture) -> AggregateInitializationOperationStore {
    AggregateInitializationOperationStore::for_lc(fixture.paths.clone(), fixture.lc_id.clone())
}

fn root_policy_receipts(
    fixture: &TrustRouteFixture,
) -> crate::product::logical_codebase::RootRecipeReceiptStore {
    crate::product::logical_codebase::RootRecipeReceiptStore::for_lc(
        fixture.paths.clone(),
        fixture.lc_id.clone(),
    )
}

fn root_policy_store(
    fixture: &TrustRouteFixture,
) -> crate::product::logical_codebase::policy::AggregatePolicyArtifactStore {
    crate::product::logical_codebase::policy::AggregatePolicyArtifactStore::for_lc(
        fixture.paths.clone(),
        fixture.lc_id.clone(),
    )
}

fn root_policy_canonical_root(fixture: &TrustRouteFixture) -> std::path::PathBuf {
    std::fs::canonicalize(fixture.root.join("aggregate-root")).expect("canonical aggregate root")
}

/// 不可变发布输出在产品 store 上的路径（`<lc_scope>/aggregate-initializations/
/// {operation_id}/policy-publication.json`）。
fn root_policy_publication_path(
    fixture: &TrustRouteFixture,
    operation_id: &str,
) -> std::path::PathBuf {
    fixture
        .paths
        .logical_codebases_root("project_0001")
        .join(&fixture.lc_id)
        .join("aggregate-initializations")
        .join(operation_id)
        .join("policy-publication.json")
}

/// 最终 receipt 在产品 store 上的路径（`aggregate-recipe-receipts/{op}.json`）。
fn root_policy_final_receipt_path(
    fixture: &TrustRouteFixture,
    operation_id: &str,
) -> std::path::PathBuf {
    fixture
        .paths
        .logical_codebases_root("project_0001")
        .join(&fixture.lc_id)
        .join("aggregate-recipe-receipts")
        .join(format!("{operation_id}.json"))
}

/// 单条命令审计 receipt 的路径（`.../{op}/commands/{index:02}.json`）。
fn root_policy_command_receipt_path(
    fixture: &TrustRouteFixture,
    operation_id: &str,
    command_index: usize,
) -> std::path::PathBuf {
    fixture
        .paths
        .logical_codebases_root("project_0001")
        .join(&fixture.lc_id)
        .join("aggregate-recipe-receipts")
        .join(operation_id)
        .join("commands")
        .join(format!("{command_index:02}.json"))
}

/// 冻结正文格式（Global Constraints）：固定标题 + 每来源
/// `## 来源：<相对路径>` + 完整原字节 + LF 分隔。
fn root_policy_expected_text(agents: &str, language: &str) -> String {
    format!(
        "# 聚合政策\n\n## 来源：AGENTS.md\n\n{agents}\n\n\
         ## 来源：.claude/rules/language.md\n\n{language}\n\n"
    )
}

/// POST 创建五步 operation（trust 门初始即 Ready），返回 operation_id。
async fn root_policy_start(fixture: &TrustRouteFixture, idempotency_key: &str) -> String {
    let uri = format!(
        "/api/projects/project_0001/logical-codebases/{}/initializations",
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

async fn root_policy_poll_terminal(
    operations: &AggregateInitializationOperationStore,
    operation_id: &str,
) -> crate::product::logical_codebase::AggregateInitializationOperation {
    for _ in 0..600 {
        let operation = operations
            .get("project_0001", operation_id)
            .expect("operation is durable");
        match operation.status {
            crate::product::logical_codebase::AggregateInitializationOperationStatus::Created
            | crate::product::logical_codebase::AggregateInitializationOperationStatus::Running => {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            _ => return operation,
        }
    }
    panic!("operation {operation_id} did not reach a terminal state");
}

/// 显式 Continue（member_index retry 面）：从 Failed 的 checkpoint 续跑。
/// `command_suffix` 保证同 operation 多次 Continue 的 command_id 唯一
/// （同 command 重放不再推进）。
async fn root_policy_continue(
    fixture: &TrustRouteFixture,
    operation_id: &str,
    command_suffix: &str,
) {
    let action_uri = format!(
        "/api/projects/project_0001/logical-codebases/{}/bootstrap/actions",
        fixture.lc_id
    );
    let (status, action) = response_json(
        post_json(
            &fixture.app,
            &action_uri,
            serde_json::json!({
                "command_id": format!("cmd-root-policy-{operation_id}-{command_suffix}"),
                "step": "member_index",
                "action": "retry",
                "expected_revision": null,
                "expected_object_id": operation_id,
            }),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{action}");
    assert_eq!(action["outcome"], "accepted", "{action}");
}

fn root_policy_all_commands_allowed(
    fixture: &TrustRouteFixture,
    operation_id: &str,
) -> Vec<crate::product::logical_codebase::root_recipe_receipt::RootRecipeCommandReceipt> {
    let mut commands = root_policy_receipts(fixture)
        .list_commands("project_0001", operation_id)
        .expect("command receipts");
    commands.sort_by_key(|receipt| receipt.command_index);
    assert_eq!(
        commands
            .iter()
            .map(|receipt| receipt.command_index)
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4],
        "exactly one audit receipt per frozen command, in index order"
    );
    assert!(
        commands.iter().all(|receipt| {
            receipt.verdict
                == crate::product::logical_codebase::root_recipe_receipt::RootRecipeCommandVerdict::Allowed
        }),
        "all four frozen commands must be audited Allowed"
    );
    commands
}

#[tokio::test]
async fn root_policy_production_final_command_publishes_and_freezes() {
    let fixture = trust_route_fixture_with(
        false,
        RecipeFixtureOptions {
            fault_first_final_turn: true,
            ..RecipeFixtureOptions::default()
        },
    );
    let operations = root_policy_operations(&fixture);
    let operation_id = root_policy_start(&fixture, "root-policy-freeze-1").await;

    // 首个末命令 turn 注入一次中断：命令 1–3 已成功执行并审计，末命令
    // 尚未执行（无 04 审计、provider 启动计数不含该 turn）。
    let interrupted = root_policy_poll_terminal(&operations, &operation_id).await;
    assert_eq!(
        interrupted.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Failed
    );
    assert_eq!(
        interrupted.failed_step,
        Some(AggregateInitializationStepKind::OpenspecAndExamples)
    );
    assert_eq!(fixture.factory.audit().stream_launches(), 2);
    let receipts = root_policy_receipts(&fixture);
    let mut commands = receipts
        .list_commands("project_0001", &operation_id)
        .expect("command receipts");
    commands.sort_by_key(|receipt| receipt.command_index);
    assert_eq!(
        commands
            .iter()
            .map(|receipt| receipt.command_index)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert!(commands.iter().all(|receipt| {
        receipt.verdict
            == crate::product::logical_codebase::root_recipe_receipt::RootRecipeCommandVerdict::Allowed
    }));

    // 前三命令后：current 仍是原自举引用，未发布任何政策材料。
    let policy = root_policy_store(&fixture);
    let base = policy
        .get("project_0001")
        .expect("policy artifact read")
        .expect("bootstrap artifact exists");
    assert!(base.is_bootstrap_placeholder());
    assert!(
        policy
            .get_recipe_policy_publication("project_0001", &operation_id)
            .expect("publication read")
            .is_none()
    );
    assert!(
        receipts
            .get("project_0001", &operation_id)
            .expect("final receipt read")
            .is_none()
    );

    // 显式 Continue：末命令真正执行 → 04 审计 Allowed → 审计窗口全部
    // 关闭后发布权威正文 → 冻结最终 receipt → 步骤 checkpoint/完成。
    root_policy_continue(&fixture, &operation_id, "run").await;
    let completed = root_policy_poll_terminal(&operations, &operation_id).await;
    assert_eq!(
        completed.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Completed,
        "{:?}",
        completed.error
    );
    assert_eq!(completed.steps.len(), 5, "five steps, no sixth step");
    assert!(completed.steps.iter().all(|step| {
        step.status
            == crate::product::logical_codebase::aggregate_initialization::AggregateInitializationStepStatus::Completed
    }));

    let artifact = policy
        .get("project_0001")
        .expect("policy artifact read")
        .expect("published artifact");
    assert!(!artifact.is_bootstrap_placeholder());
    assert_eq!(artifact.revision, 2, "bootstrap base revision 1 → 2");
    let expected = root_policy_expected_text(ROOT_POLICY_TEST_AGENTS, ROOT_POLICY_TEST_LANGUAGE);
    assert_eq!(artifact.policy_text, expected, "frozen aggregation format");

    // 根 locator 原字节 == artifact 正文；独立 SHA-256 == artifact digest。
    let canonical_root = root_policy_canonical_root(&fixture);
    let locator = canonical_root.join(&artifact.policy_id);
    let root_bytes = std::fs::read(&locator).expect("published policy bytes at the root locator");
    assert_eq!(root_bytes, artifact.policy_text.as_bytes());
    let independent_digest = format!("sha256:{:x}", Sha256::digest(&root_bytes));
    assert_eq!(independent_digest, artifact.digest);

    // receipt 三方一致：policy digest == artifact digest；rule digest 对
    // AGENTS.md 原字节独立复算一致；finalized_at 复用候选发布时间。
    let receipt = receipts
        .get("project_0001", &operation_id)
        .expect("final receipt read")
        .expect("finalized root receipt");
    assert_eq!(receipt.policy_digest, artifact.digest);
    let agents_bytes = std::fs::read(canonical_root.join("AGENTS.md")).expect("AGENTS.md bytes");
    assert_eq!(
        receipt.rule_digest,
        format!("sha256:{:x}", Sha256::digest(&agents_bytes))
    );
    assert_eq!(receipt.finalized_at, artifact.created_at);
    assert_eq!(receipt.commands.len(), 4);
    assert!(receipt.commands.iter().all(|summary| {
        summary.verdict
            == crate::product::logical_codebase::root_recipe_receipt::RootRecipeCommandVerdict::Allowed
    }));

    // 只保留原 3 个 step 级 stream launch（中断的 turn 不计 launch）。
    assert_eq!(fixture.factory.audit().stream_launches(), 3);

    // 不可变发布输出在场，来源恰为 AGENTS + language。
    let output = policy
        .get_recipe_policy_publication("project_0001", &operation_id)
        .expect("publication read")
        .expect("publication output");
    assert_eq!(
        output
            .sources
            .iter()
            .map(|source| source.relative_path.clone())
            .collect::<Vec<_>>(),
        vec!["AGENTS.md", ".claude/rules/language.md"]
    );
    assert_eq!(output.artifact, artifact);
}

/// 发布失败三负例（缺 language / locator 冲突 / 输出写失败）的公共断言：
/// operation Failed 于 OpenspecAndExamples、无最终 receipt、命令 04 审计
/// Allowed 保留、AGENTS 字节不变、current 仍为自举桩——绝不 warn 后
/// Completed。
async fn assert_publish_failure_state(fixture: &TrustRouteFixture, operation_id: &str, case: &str) {
    let operations = root_policy_operations(fixture);
    let failed = root_policy_poll_terminal(&operations, operation_id).await;
    assert_eq!(
        failed.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Failed,
        "{case}: publication failure must fail the final step, not warn-and-complete: {:?}",
        failed.error
    );
    assert_eq!(
        failed.failed_step,
        Some(AggregateInitializationStepKind::OpenspecAndExamples),
        "{case}"
    );
    let receipts = root_policy_receipts(fixture);
    assert!(
        receipts
            .get("project_0001", operation_id)
            .expect("final receipt read")
            .is_none(),
        "{case}: no final receipt may be signed"
    );
    // 末命令真正执行过且 Allowed：失败只归发布收口，不归命令本身。
    root_policy_all_commands_allowed(fixture, operation_id);
    let canonical_root = root_policy_canonical_root(fixture);
    assert_eq!(
        std::fs::read(canonical_root.join("AGENTS.md")).expect("AGENTS bytes"),
        ROOT_POLICY_TEST_AGENTS.as_bytes(),
        "{case}: provider-generated rule bytes must stay untouched"
    );
    let policy = root_policy_store(fixture);
    let current = policy
        .get("project_0001")
        .expect("policy artifact read")
        .expect("bootstrap artifact");
    assert!(
        current.is_bootstrap_placeholder(),
        "{case}: current artifact must stay at the bootstrap base"
    );
}

#[tokio::test]
async fn root_policy_production_publish_failure_marks_final_step_failed() {
    // (a) 缺 language：RuleAndMcpConfig 只生成 AGENTS，无
    // .claude/rules/language.md → 发布来源收集失败。
    let fixture = trust_route_fixture_with(
        false,
        RecipeFixtureOptions {
            write_language_rule: false,
            ..RecipeFixtureOptions::default()
        },
    );
    let operation_id = root_policy_start(&fixture, "root-policy-publish-fail-language-1").await;
    assert_publish_failure_state(&fixture, &operation_id, "missing language").await;
    assert!(
        !root_policy_canonical_root(&fixture)
            .join(".claude")
            .join("rules")
            .join("language.md")
            .exists(),
        "missing language stays absent"
    );

    // (b) locator 冲突：末命令前预置用户占位的候选 locator（不同字节），
    // 发布 no-clobber 拒绝覆盖用户文件。
    let fixture = trust_route_fixture_with(
        false,
        RecipeFixtureOptions {
            fault_first_final_turn: true,
            ..RecipeFixtureOptions::default()
        },
    );
    let operations = root_policy_operations(&fixture);
    let operation_id = root_policy_start(&fixture, "root-policy-publish-fail-conflict-1").await;
    let interrupted = root_policy_poll_terminal(&operations, &operation_id).await;
    assert_eq!(
        interrupted.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Failed
    );
    let manifest = crate::product::logical_codebase::LogicalCodebaseStore::for_lc(
        fixture.paths.clone(),
        fixture.lc_id.clone(),
    )
    .load_manifest("project_0001")
    .expect("manifest read")
    .expect("manifest exists");
    let uuid = manifest.logical_codebase_id.to_string();
    let locator =
        root_policy_canonical_root(&fixture).join(format!("policy/project_0001/{uuid}/2"));
    std::fs::create_dir_all(locator.parent().expect("locator parent"))
        .expect("create locator parents");
    std::fs::write(&locator, "user-owned conflicting bytes\n").expect("seed conflicting locator");
    root_policy_continue(&fixture, &operation_id, "run").await;
    assert_publish_failure_state(&fixture, &operation_id, "locator conflict").await;
    assert_eq!(
        std::fs::read(&locator).expect("locator bytes"),
        "user-owned conflicting bytes\n".as_bytes(),
        "locator conflict must never overwrite user files"
    );

    // (c) 写失败：不可变输出目标被目录占位 → 输出阶段持久化失败。
    let fixture = trust_route_fixture_with(
        false,
        RecipeFixtureOptions {
            fault_first_final_turn: true,
            ..RecipeFixtureOptions::default()
        },
    );
    let operations = root_policy_operations(&fixture);
    let operation_id = root_policy_start(&fixture, "root-policy-publish-fail-write-1").await;
    let interrupted = root_policy_poll_terminal(&operations, &operation_id).await;
    assert_eq!(
        interrupted.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Failed
    );
    let output_path = root_policy_publication_path(&fixture, &operation_id);
    std::fs::create_dir_all(&output_path).expect("block the publication output path");
    root_policy_continue(&fixture, &operation_id, "run").await;
    assert_publish_failure_state(&fixture, &operation_id, "output write failure").await;
    assert!(
        output_path.is_dir(),
        "the blocked output path must stay untouched"
    );
}

#[tokio::test]
async fn root_policy_production_receipt_failure_is_not_completed() {
    // receipt 签发阶段注入一次写失败：发布完成后、finalize 落盘前中断
    // ——末步必须 Failed（绝不 warn 后 Completed），Allowed 末命令与
    // publication 保留。注：不用文件路径占位模拟（receipt 目录的其他
    // 读者会被毒化），由 cfg(test) seam 在 finalize 边界注入。
    let fixture = trust_route_fixture_with(false, RecipeFixtureOptions::default());
    let operations = root_policy_operations(&fixture);
    let key = "root-policy-receipt-fail-1";
    let operation_id = deterministic_operation_id("project_0001", key);
    root_policy_test_faults::arm(&operation_id, root_policy_test_faults::Phase::ReceiptWrite);
    let started = root_policy_start(&fixture, key).await;
    assert_eq!(started, operation_id);

    let failed = root_policy_poll_terminal(&operations, &operation_id).await;
    assert_eq!(
        failed.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Failed,
        "{:?}",
        failed.error
    );
    assert_eq!(
        failed.failed_step,
        Some(AggregateInitializationStepKind::OpenspecAndExamples)
    );
    // Allowed 末命令与 publication 均保留。
    root_policy_all_commands_allowed(&fixture, &operation_id);
    let policy = root_policy_store(&fixture);
    let output = policy
        .get_recipe_policy_publication("project_0001", &operation_id)
        .expect("publication read")
        .expect("publication output retained");
    assert_eq!(output.artifact.revision, 2);
    let current = policy
        .get("project_0001")
        .expect("policy artifact read")
        .expect("current artifact");
    assert_eq!(current, output.artifact);
    // 无最终 receipt。
    assert!(
        root_policy_receipts(&fixture)
            .get("project_0001", &operation_id)
            .expect("final receipt read")
            .is_none(),
        "the receipt write failure must leave no final receipt"
    );

    // 显式 Continue 复用候选而非重执行命令：launch 计数不增、同候选
    // revision/time 补签 receipt。
    let launches_before = fixture.factory.audit().stream_launches();
    assert_eq!(launches_before, 3);
    root_policy_continue(&fixture, &operation_id, "resume").await;
    let completed = root_policy_poll_terminal(&operations, &operation_id).await;
    assert_eq!(
        completed.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Completed,
        "{:?}",
        completed.error
    );
    assert_eq!(
        fixture.factory.audit().stream_launches(),
        launches_before,
        "resume must not launch the provider again"
    );
    let resumed = policy
        .get_recipe_policy_publication("project_0001", &operation_id)
        .expect("publication read")
        .expect("publication output");
    assert_eq!(resumed, output, "the frozen candidate must be reused as-is");
    let receipt = root_policy_receipts(&fixture)
        .get("project_0001", &operation_id)
        .expect("final receipt read")
        .expect("finalized root receipt");
    assert_eq!(receipt.policy_digest, resumed.artifact.digest);
    assert_eq!(receipt.finalized_at, resumed.artifact.created_at);
    root_policy_all_commands_allowed(&fixture, &operation_id);
}

#[tokio::test]
async fn root_policy_resume_after_command_audit() {
    let fixture = trust_route_fixture_with(false, RecipeFixtureOptions::default());
    let operations = root_policy_operations(&fixture);
    // operation_id 确定（project+key 派生）：发布前武装中断，保证窗口
    // 出现在 command04 审计 durable 之后、发布开始之前。
    let key = "root-policy-resume-audit-1";
    let operation_id = deterministic_operation_id("project_0001", key);
    root_policy_test_faults::arm(
        &operation_id,
        root_policy_test_faults::Phase::AfterCommandAudit,
    );
    let started = root_policy_start(&fixture, key).await;
    assert_eq!(started, operation_id);

    let failed = root_policy_poll_terminal(&operations, &operation_id).await;
    assert_eq!(
        failed.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Failed
    );
    assert_eq!(
        failed.failed_step,
        Some(AggregateInitializationStepKind::OpenspecAndExamples)
    );
    // 窗口事实：command04 已 durable、output 尚无。
    let command04_path = root_policy_command_receipt_path(&fixture, &operation_id, 4);
    let command04_bytes = std::fs::read(&command04_path).expect("command04 audit is durable");
    root_policy_all_commands_allowed(&fixture, &operation_id);
    let policy = root_policy_store(&fixture);
    assert!(
        policy
            .get_recipe_policy_publication("project_0001", &operation_id)
            .expect("publication read")
            .is_none()
    );
    let launches_before = fixture.factory.audit().stream_launches();
    assert_eq!(launches_before, 3);

    // Continue 冻结一次 candidate：launch 计数不增、旧 command04 bytes
    // 不变、不追加第二条 04。
    root_policy_continue(&fixture, &operation_id, "resume").await;
    let completed = root_policy_poll_terminal(&operations, &operation_id).await;
    assert_eq!(
        completed.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Completed,
        "{:?}",
        completed.error
    );
    assert_eq!(
        fixture.factory.audit().stream_launches(),
        launches_before,
        "resume must not launch the provider again"
    );
    assert_eq!(
        std::fs::read(&command04_path).expect("command04 audit bytes"),
        command04_bytes,
        "the durable command04 audit must stay byte-identical"
    );
    root_policy_all_commands_allowed(&fixture, &operation_id);
    let output = policy
        .get_recipe_policy_publication("project_0001", &operation_id)
        .expect("publication read")
        .expect("publication output");
    assert_eq!(output.artifact.revision, 2);
    let receipt = root_policy_receipts(&fixture)
        .get("project_0001", &operation_id)
        .expect("final receipt read")
        .expect("finalized root receipt");
    assert_eq!(receipt.policy_digest, output.artifact.digest);
    assert_eq!(receipt.finalized_at, output.artifact.created_at);
}

#[tokio::test]
async fn root_policy_resume_after_body_publication() {
    let fixture = trust_route_fixture_with(false, RecipeFixtureOptions::default());
    let operations = root_policy_operations(&fixture);
    let key = "root-policy-resume-body-1";
    let operation_id = deterministic_operation_id("project_0001", key);
    root_policy_test_faults::arm(
        &operation_id,
        root_policy_test_faults::Phase::AfterPublication,
    );
    root_policy_start(&fixture, key).await;
    let failed = root_policy_poll_terminal(&operations, &operation_id).await;
    assert_eq!(
        failed.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Failed
    );

    // 发布已完成：root 新正文在、current 已是候选、receipt 尚无。
    let policy = root_policy_store(&fixture);
    let output = policy
        .get_recipe_policy_publication("project_0001", &operation_id)
        .expect("publication read")
        .expect("publication output");
    let candidate = output.artifact.clone();
    let canonical_root = root_policy_canonical_root(&fixture);
    let locator = canonical_root.join(&candidate.policy_id);
    assert_eq!(
        std::fs::read(&locator).expect("published body bytes"),
        candidate.policy_text.as_bytes()
    );
    assert_eq!(
        policy
            .get("project_0001")
            .expect("policy artifact read")
            .expect("current artifact"),
        candidate
    );
    assert!(
        root_policy_receipts(&fixture)
            .get("project_0001", &operation_id)
            .expect("final receipt read")
            .is_none()
    );

    // 构造「正文后」中断窗口：locator 已落盘、current 仍 base——把
    // current artifact 回置自举桩（模拟 locator 落盘后、current 替换前
    // 的进程中断），不可变输出保持冻结。
    let output_path = root_policy_publication_path(&fixture, &operation_id);
    let output_bytes_before = std::fs::read(&output_path).expect("frozen output bytes");
    let artifact_path = fixture
        .paths
        .logical_codebases_root("project_0001")
        .join(&fixture.lc_id)
        .join("aggregate-policy.json");
    std::fs::remove_file(&artifact_path).expect("drop the current artifact");
    let manifest = crate::product::logical_codebase::LogicalCodebaseStore::for_lc(
        fixture.paths.clone(),
        fixture.lc_id.clone(),
    )
    .load_manifest("project_0001")
    .expect("manifest read")
    .expect("manifest exists");
    policy
        .ensure_bootstrap(&manifest)
        .expect("restore the bootstrap base artifact");
    let base = policy
        .get("project_0001")
        .expect("policy artifact read")
        .expect("base artifact");
    assert!(base.is_bootstrap_placeholder(), "window: current is base");

    // Continue 用同 candidate revision/time 补 artifact/receipt。
    let launches_before = fixture.factory.audit().stream_launches();
    assert_eq!(launches_before, 3);
    root_policy_continue(&fixture, &operation_id, "resume").await;
    let completed = root_policy_poll_terminal(&operations, &operation_id).await;
    assert_eq!(
        completed.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Completed,
        "{:?}",
        completed.error
    );
    assert_eq!(
        fixture.factory.audit().stream_launches(),
        launches_before,
        "resume must not launch the provider again"
    );
    assert_eq!(
        policy
            .get("project_0001")
            .expect("policy artifact read")
            .expect("current artifact"),
        candidate,
        "the frozen candidate must complete the artifact with the same revision/time"
    );
    assert_eq!(
        std::fs::read(&output_path).expect("output bytes after resume"),
        output_bytes_before,
        "the immutable output must stay byte-identical"
    );
    let receipt = root_policy_receipts(&fixture)
        .get("project_0001", &operation_id)
        .expect("final receipt read")
        .expect("finalized root receipt");
    assert_eq!(receipt.policy_digest, candidate.digest);
    assert_eq!(receipt.finalized_at, candidate.created_at);
    assert_eq!(
        std::fs::read(&locator).expect("published body bytes"),
        candidate.policy_text.as_bytes()
    );
}

#[tokio::test]
async fn root_policy_resume_after_current_artifact() {
    let fixture = trust_route_fixture_with(false, RecipeFixtureOptions::default());
    let operations = root_policy_operations(&fixture);
    let key = "root-policy-resume-artifact-1";
    let operation_id = deterministic_operation_id("project_0001", key);
    root_policy_test_faults::arm(
        &operation_id,
        root_policy_test_faults::Phase::AfterPublication,
    );
    root_policy_start(&fixture, key).await;
    let failed = root_policy_poll_terminal(&operations, &operation_id).await;
    assert_eq!(
        failed.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Failed
    );

    // 窗口事实：current 已是候选、receipt 无。
    let policy = root_policy_store(&fixture);
    let output = policy
        .get_recipe_policy_publication("project_0001", &operation_id)
        .expect("publication read")
        .expect("publication output");
    let candidate = output.artifact.clone();
    assert_eq!(
        policy
            .get("project_0001")
            .expect("policy artifact read")
            .expect("current artifact"),
        candidate
    );
    let receipts = root_policy_receipts(&fixture);
    assert!(
        receipts
            .get("project_0001", &operation_id)
            .expect("final receipt read")
            .is_none()
    );
    let output_path = root_policy_publication_path(&fixture, &operation_id);
    let output_bytes_before = std::fs::read(&output_path).expect("frozen output bytes");
    let command04_path = root_policy_command_receipt_path(&fixture, &operation_id, 4);
    let command04_bytes = std::fs::read(&command04_path).expect("command04 audit bytes");
    let launches_before = fixture.factory.audit().stream_launches();
    assert_eq!(launches_before, 3);

    // Continue：不触发 successor 冲突、不涨 revision。
    root_policy_continue(&fixture, &operation_id, "resume").await;
    let completed = root_policy_poll_terminal(&operations, &operation_id).await;
    assert_eq!(
        completed.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Completed,
        "{:?}",
        completed.error
    );
    assert_eq!(
        fixture.factory.audit().stream_launches(),
        launches_before,
        "resume must not launch the provider again"
    );
    assert_eq!(
        std::fs::read(&output_path).expect("output bytes after resume"),
        output_bytes_before,
        "no successor rewrite: the immutable output stays frozen (same revision/time)"
    );
    assert_eq!(
        std::fs::read(&command04_path).expect("command04 audit bytes"),
        command04_bytes
    );
    root_policy_all_commands_allowed(&fixture, &operation_id);
    let receipt = receipts
        .get("project_0001", &operation_id)
        .expect("final receipt read")
        .expect("finalized root receipt");
    assert_eq!(receipt.policy_digest, candidate.digest);
    assert_eq!(receipt.finalized_at, candidate.created_at);
}

#[tokio::test]
async fn root_policy_resume_after_final_receipt() {
    let fixture = trust_route_fixture_with(false, RecipeFixtureOptions::default());
    let operations = root_policy_operations(&fixture);
    let key = "root-policy-resume-receipt-1";
    let operation_id = deterministic_operation_id("project_0001", key);
    root_policy_test_faults::arm(
        &operation_id,
        root_policy_test_faults::Phase::AfterFinalReceipt,
    );
    root_policy_start(&fixture, key).await;
    let failed = root_policy_poll_terminal(&operations, &operation_id).await;
    assert_eq!(
        failed.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Failed,
        "receipt signed but the turn must fail before the step checkpoint"
    );

    // 窗口事实：receipt 已在、末步未 checkpoint。
    let receipts = root_policy_receipts(&fixture);
    let receipt = receipts
        .get("project_0001", &operation_id)
        .expect("final receipt read")
        .expect("finalized root receipt");
    let receipt_path = root_policy_final_receipt_path(&fixture, &operation_id);
    let receipt_bytes = std::fs::read(&receipt_path).expect("frozen receipt bytes");
    let output_path = root_policy_publication_path(&fixture, &operation_id);
    let output_bytes = std::fs::read(&output_path).expect("frozen output bytes");
    let launches_before = fixture.factory.audit().stream_launches();
    assert_eq!(launches_before, 3);

    // Continue 复用相同 finalized_at：receipt/output bytes 不变，operation
    // 最终 Completed。
    root_policy_continue(&fixture, &operation_id, "resume").await;
    let completed = root_policy_poll_terminal(&operations, &operation_id).await;
    assert_eq!(
        completed.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Completed,
        "{:?}",
        completed.error
    );
    assert_eq!(
        fixture.factory.audit().stream_launches(),
        launches_before,
        "resume must not launch the provider again"
    );
    assert_eq!(
        std::fs::read(&receipt_path).expect("receipt bytes after resume"),
        receipt_bytes,
        "the finalized receipt must stay byte-identical (same finalized_at)"
    );
    assert_eq!(
        std::fs::read(&output_path).expect("output bytes after resume"),
        output_bytes
    );
    root_policy_all_commands_allowed(&fixture, &operation_id);
    assert_eq!(
        receipts
            .get("project_0001", &operation_id)
            .expect("final receipt read")
            .expect("finalized root receipt"),
        receipt
    );
}

/// 漂移负例公共骨架：先构造指定窗口的中断态，注入漂移后 Continue 必须
/// 拒绝——零新增 launch、审计与不可变 output/user bytes 不被覆盖、
/// operation 停在 Failed（可重试面保留）。返回漂移注入前后的对照由
/// `tamper` 闭包完成，其返回值为须保持不变的 (path, bytes) 列表。
async fn assert_resume_rejects_drift(
    fixture: &TrustRouteFixture,
    operation_id: &str,
    case: &str,
    tamper: impl FnOnce(&TrustRouteFixture) -> Vec<(std::path::PathBuf, Vec<u8>)>,
) {
    let launches_before = fixture.factory.audit().stream_launches();
    let command_paths: Vec<(std::path::PathBuf, Vec<u8>)> = (1..=4)
        .map(|index| {
            let path = root_policy_command_receipt_path(fixture, operation_id, index);
            let bytes = std::fs::read(&path).expect("command audit bytes");
            (path, bytes)
        })
        .collect();
    let output_path = root_policy_publication_path(fixture, operation_id);
    let output_bytes_before = output_path
        .exists()
        .then(|| std::fs::read(&output_path).expect("frozen output bytes"));
    let tampered = tamper(fixture);

    root_policy_continue(fixture, operation_id, "resume").await;
    let operations = root_policy_operations(fixture);
    let rejected = root_policy_poll_terminal(&operations, operation_id).await;
    assert_eq!(
        rejected.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Failed,
        "{case}: audit/source/identity drift must reject the resume: {:?}",
        rejected.error
    );
    assert_eq!(
        rejected.failed_step,
        Some(AggregateInitializationStepKind::OpenspecAndExamples),
        "{case}"
    );
    assert_eq!(
        fixture.factory.audit().stream_launches(),
        launches_before,
        "{case}: rejected resume must add zero provider launches"
    );
    for (path, bytes) in &command_paths {
        // 篡改闭包自身改写的审计文件（audit-identity 漂移注入）由下方
        // tampered 断言按篡改后字节核对；其余审计必须逐字节不变。
        if tampered
            .iter()
            .any(|(tampered_path, _)| tampered_path == path)
        {
            continue;
        }
        assert_eq!(
            &std::fs::read(path).expect("command audit bytes"),
            bytes,
            "{case}: durable audits must never be overwritten"
        );
    }
    if let Some(bytes) = &output_bytes_before {
        assert_eq!(
            &std::fs::read(&output_path).expect("output bytes"),
            bytes,
            "{case}: the immutable publication output must stay frozen"
        );
    }
    for (path, bytes) in &tampered {
        assert_eq!(
            &std::fs::read(path).expect("tampered file bytes"),
            bytes,
            "{case}: user bytes must stay untouched"
        );
    }
}

#[tokio::test]
async fn root_policy_resume_rejects_audit_source_or_identity_drift() {
    // 窗口 1（命令审计后，output 尚无）：source 清单与字节漂移——新增
    // 规则文件 + 修改 language 字节。
    let fixture = trust_route_fixture_with(false, RecipeFixtureOptions::default());
    let key = "root-policy-drift-source-1";
    let operation_id = deterministic_operation_id("project_0001", key);
    root_policy_test_faults::arm(
        &operation_id,
        root_policy_test_faults::Phase::AfterCommandAudit,
    );
    root_policy_start(&fixture, key).await;
    root_policy_poll_terminal(&root_policy_operations(&fixture), &operation_id).await;
    assert_resume_rejects_drift(
        &fixture,
        &operation_id,
        "source list/bytes drift",
        |fixture| {
            let root = root_policy_canonical_root(fixture);
            let language = root.join(".claude").join("rules").join("language.md");
            let extra = root.join(".claude").join("rules").join("extra.md");
            std::fs::write(&extra, "# extra rule\n").expect("add a new rule source");
            std::fs::write(&language, "# 语言规则\n\n- 被篡改的规则。\n")
                .expect("mutate the language rule bytes");
            vec![(extra, "# extra rule\n".as_bytes().to_vec())]
        },
    )
    .await;

    // 窗口 2（正文后，current 仍 base）：AGENTS 入口字节漂移。
    let fixture = trust_route_fixture_with(false, RecipeFixtureOptions::default());
    let key = "root-policy-drift-agents-1";
    let operation_id = deterministic_operation_id("project_0001", key);
    root_policy_test_faults::arm(
        &operation_id,
        root_policy_test_faults::Phase::AfterPublication,
    );
    root_policy_start(&fixture, key).await;
    root_policy_poll_terminal(&root_policy_operations(&fixture), &operation_id).await;
    assert_resume_rejects_drift(&fixture, &operation_id, "AGENTS bytes drift", |fixture| {
        let agents = root_policy_canonical_root(fixture).join("AGENTS.md");
        std::fs::write(&agents, "# 被篡改的根入口\n").expect("mutate the root entry");
        vec![(agents, "# 被篡改的根入口\n".as_bytes().to_vec())]
    })
    .await;

    // 窗口 3（current artifact 后）：locator 字节被替换——非发布所有的
    // 正文不可被当作可复用候选（重入发布 no-clobber 冲突拒绝）。
    let fixture = trust_route_fixture_with(false, RecipeFixtureOptions::default());
    let key = "root-policy-drift-locator-1";
    let operation_id = deterministic_operation_id("project_0001", key);
    root_policy_test_faults::arm(
        &operation_id,
        root_policy_test_faults::Phase::AfterPublication,
    );
    root_policy_start(&fixture, key).await;
    root_policy_poll_terminal(&root_policy_operations(&fixture), &operation_id).await;
    assert_resume_rejects_drift(&fixture, &operation_id, "locator bytes drift", |fixture| {
        let output = root_policy_store(fixture)
            .get_recipe_policy_publication("project_0001", &operation_id)
            .expect("publication read")
            .expect("publication output");
        let locator = root_policy_canonical_root(fixture).join(&output.artifact.policy_id);
        let tampered = b"tampered locator bytes\n".to_vec();
        std::fs::write(&locator, &tampered).expect("mutate the published locator");
        vec![(locator, tampered)]
    })
    .await;

    // 窗口 4（receipt 后）：非发布 owned 的新文件——恢复快照必须拒绝。
    let fixture = trust_route_fixture_with(false, RecipeFixtureOptions::default());
    let key = "root-policy-drift-newfile-1";
    let operation_id = deterministic_operation_id("project_0001", key);
    root_policy_test_faults::arm(
        &operation_id,
        root_policy_test_faults::Phase::AfterFinalReceipt,
    );
    root_policy_start(&fixture, key).await;
    root_policy_poll_terminal(&root_policy_operations(&fixture), &operation_id).await;
    assert_resume_rejects_drift(
        &fixture,
        &operation_id,
        "non-publication new file",
        |fixture| {
            let notes = root_policy_canonical_root(fixture).join("notes.md");
            std::fs::write(&notes, "user note\n").expect("drop a foreign file in the root");
            vec![(notes, "user note\n".as_bytes().to_vec())]
        },
    )
    .await;

    // 身份漂移：末条审计的 canonical root 被篡改——不返回 false 再执行
    // provider，必须报错。
    let fixture = trust_route_fixture_with(false, RecipeFixtureOptions::default());
    let key = "root-policy-drift-audit-identity-1";
    let operation_id = deterministic_operation_id("project_0001", key);
    root_policy_test_faults::arm(
        &operation_id,
        root_policy_test_faults::Phase::AfterCommandAudit,
    );
    root_policy_start(&fixture, key).await;
    root_policy_poll_terminal(&root_policy_operations(&fixture), &operation_id).await;
    assert_resume_rejects_drift(&fixture, &operation_id, "audit identity drift", |fixture| {
        let command04_path = root_policy_command_receipt_path(fixture, &operation_id, 4);
        let mut receipt: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&command04_path).expect("04 audit"))
                .expect("04 audit json");
        receipt["canonical_root"] = serde_json::json!("/drifted/audit/root");
        let tampered = serde_json::to_vec_pretty(&receipt).expect("serialize drifted audit");
        std::fs::write(&command04_path, &tampered).expect("mutate the 04 audit identity");
        vec![(command04_path, tampered)]
    })
    .await;
}

#[tokio::test]
async fn root_policy_recipe_completed_keeps_index_gate_independent() {
    let fixture = trust_route_fixture_with(false, RecipeFixtureOptions::default());
    let operations = root_policy_operations(&fixture);
    let operation_id = root_policy_start(&fixture, "root-policy-index-gate-1").await;
    let completed = root_policy_poll_terminal(&operations, &operation_id).await;
    assert_eq!(
        completed.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Completed,
        "{:?}",
        completed.error
    );
    // 政策/receipt 完成：真正文（非桩）+ 最终 receipt 三方一致。
    let policy = root_policy_store(&fixture);
    let artifact = policy
        .get("project_0001")
        .expect("policy artifact read")
        .expect("published artifact");
    assert!(!artifact.is_bootstrap_placeholder());
    let receipt = root_policy_receipts(&fixture)
        .get("project_0001", &operation_id)
        .expect("final receipt read")
        .expect("finalized root receipt");
    assert_eq!(receipt.policy_digest, artifact.digest);
    let receipt_path = root_policy_final_receipt_path(&fixture, &operation_id);
    let receipt_bytes = std::fs::read(&receipt_path).expect("frozen receipt bytes");

    // 索引未就绪 → planning_ready=false（recipe Completed 不代替整体
    // readiness）。
    let bootstrap_uri = format!(
        "/api/projects/project_0001/logical-codebases/{}/bootstrap",
        fixture.lc_id
    );
    let step_status = |projection: &serde_json::Value, step: &str| {
        projection["steps"]
            .as_array()
            .expect("steps array")
            .iter()
            .find(|entry| entry["step"] == step)
            .unwrap_or_else(|| panic!("step {step} missing: {projection}"))["status"]
            .clone()
    };
    let (status, projection) = get_json(&fixture.app, &bootstrap_uri).await;
    assert_eq!(status, StatusCode::OK, "{projection}");
    assert_eq!(projection["planning_ready"], false, "{projection}");
    assert_eq!(step_status(&projection, "member_index"), "completed");
    assert_eq!(step_status(&projection, "rules_policy"), "completed");
    assert_ne!(
        step_status(&projection, "aggregate_index_active"),
        "completed",
        "{projection}"
    );

    // detached index build 以必缺 codegraph 二进制确定性失败 → Failed
    // 记录落盘（等待其出现，避免与 GET 竞态）。
    let index_store =
        crate::product::logical_codebase::aggregate_index::AggregateIndexStore::for_lc(
            fixture.paths.clone(),
            &fixture.lc_id,
        );
    let mut failed_index_seen = false;
    for _ in 0..600 {
        if index_store
            .records("project_0001")
            .expect("index records")
            .iter()
            .any(|record| {
                matches!(
                    record.status,
                    crate::product::logical_codebase::aggregate_index::AggregateIndexStatus::Failed
                )
            })
        {
            failed_index_seen = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        failed_index_seen,
        "the detached index build must fail deterministically (missing codegraph binary)"
    );

    // index 失败不回滚已完成 recipe：投影/operation/receipt 全部不变。
    let (status, after) = get_json(&fixture.app, &bootstrap_uri).await;
    assert_eq!(status, StatusCode::OK, "{after}");
    assert_eq!(after["planning_ready"], false, "{after}");
    assert_eq!(step_status(&after, "member_index"), "completed");
    assert_eq!(step_status(&after, "rules_policy"), "completed");
    assert_ne!(step_status(&after, "aggregate_index_active"), "completed");
    let operation_after = operations
        .get("project_0001", &operation_id)
        .expect("operation read");
    assert_eq!(
        operation_after.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Completed
    );
    assert_eq!(
        std::fs::read(&receipt_path).expect("receipt bytes"),
        receipt_bytes,
        "a failed index build must not roll back the completed recipe receipt"
    );
}
