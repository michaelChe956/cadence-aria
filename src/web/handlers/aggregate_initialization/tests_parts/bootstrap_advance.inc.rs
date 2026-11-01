// -------------------------------------------------------------------
// A（REQ-REG-10「自动推进」）的 HTTP 面契约：
//
// - `POST .../bootstrap/advance` 在 owner 报告的真机现场（后三步
//   not_started + prepare）真正把首建链推进起来，且重复调用不双推；
// - 失败步骤停在 failed（不自动重试、不改写成 completed）；
// - GET `/bootstrap` 在推进后仍然零写入；
// - 冷启动自动推进（`full_recipe_requested = false`）在**已初始化根**上
//   跳过四条 Cadence-skills 命令，只发布政策并冻结跳过式 receipt——
//   与手动「启动聚合初始化」（强制完整五步，见 skip_skills.inc.rs）区分。
// -------------------------------------------------------------------

/// POST `/bootstrap/advance`（无请求体语义，发送空 JSON）。
async fn advance_post(
    app: &axum::Router,
    lc_id: &str,
) -> (StatusCode, serde_json::Value) {
    let uri = format!("/api/projects/project_0001/logical-codebases/{lc_id}/bootstrap/advance");
    response_json(post_json(app, &uri, serde_json::json!({})).await).await
}

/// 读取投影里的单步状态。
fn bootstrap_step_status(projection: &serde_json::Value, step: &str) -> String {
    projection["steps"]
        .as_array()
        .expect("steps array")
        .iter()
        .find(|entry| entry["step"] == step)
        .unwrap_or_else(|| panic!("step {step} missing: {projection}"))["status"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

fn bootstrap_step_actions(projection: &serde_json::Value, step: &str) -> Vec<String> {
    projection["steps"]
        .as_array()
        .expect("steps array")
        .iter()
        .find(|entry| entry["step"] == step)
        .unwrap_or_else(|| panic!("step {step} missing: {projection}"))["allowed_actions"]
        .as_array()
        .expect("allowed_actions")
        .iter()
        .filter_map(|action| action.as_str().map(str::to_string))
        .collect()
}

fn advanced_step_names(body: &serde_json::Value) -> Vec<String> {
    body["advanced_steps"]
        .as_array()
        .expect("advanced_steps")
        .iter()
        .filter_map(|step| step.as_str().map(str::to_string))
        .collect()
}

/// 唯一根配方 operation 的 id（advance 派发的 idempotency_key 由服务端
/// 派生，测试只断言「只有一个」并按 durable 记录取回它）。
fn sole_operation_id(fixture: &TrustRouteFixture) -> String {
    let mut operations = root_policy_operations(fixture)
        .list("project_0001")
        .expect("operation list");
    assert_eq!(
        operations.len(),
        1,
        "advance must produce exactly one root recipe operation"
    );
    operations.remove(0).operation_id
}

/// 给 LС 播种一个真实活跃成员 + checkout：identity 与 manifest/checkout
/// 两步由此 Completed，现场只剩后三步 not_started。
fn advance_seed_member(fixture: &TrustRouteFixture) {
    let repo = fixture.root.join("aggregate-root").join("repo-a");
    std::fs::create_dir_all(&repo).expect("member repo");
    let git = |arguments: &[&str]| {
        let output = std::process::Command::new("git")
            .args(arguments)
            .current_dir(&repo)
            .output()
            .expect("git must start");
        assert!(output.status.success(), "git {arguments:?} failed");
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    git(&["init", "-b", "main"]);
    git(&["config", "user.email", "advance@test.local"]);
    git(&["config", "user.name", "Advance Test"]);
    std::fs::create_dir_all(repo.join(".claude").join("rules")).expect("member rules dir");
    std::fs::write(repo.join("README.md"), "# member\n").expect("member readme");
    std::fs::write(
        repo.join(".claude").join("rules").join("language.md"),
        "# member rule\n",
    )
    .expect("member rule");
    git(&["add", "."]);
    git(&["commit", "-m", "init"]);

    let canonical = std::fs::canonicalize(&repo).expect("canonical member");
    let source =
        crate::product::repository_store::resolve_repository_source(&canonical).expect("source");
    let member_id = crate::product::logical_codebase::LogicalRepositoryId(uuid::Uuid::new_v4());
    let checkout_id = crate::product::logical_codebase::RepositoryCheckoutId(uuid::Uuid::new_v4());
    let lc_store = crate::product::logical_codebase::LogicalCodebaseStore::for_lc(
        fixture.paths.clone(),
        fixture.lc_id.clone(),
    );
    let mut manifest = lc_store
        .load_manifest("project_0001")
        .expect("manifest read")
        .expect("the fixture persists a manifest");
    manifest.member_ids = vec![member_id];
    manifest.membership_revision = manifest.membership_revision.saturating_add(1);
    lc_store
        .save_manifest("project_0001", &manifest)
        .expect("manifest");
    lc_store
        .save_member(
            "project_0001",
            &crate::product::logical_codebase::types::CodebaseMemberRecord {
                logical_repository_id: member_id,
                physical_repository_id: "repository_member_a".to_string(),
                alias: "repo-a".to_string(),
                role: "member".to_string(),
                ordinal: 1,
                source_identity: source.clone(),
                repo_type: crate::product::logical_codebase::types::RepositoryType::Unknown,
                tech_stack: Vec::new(),
                owner: None,
                tags: Vec::new(),
                default_ref: None,
                checkout_ids: vec![checkout_id],
                status: crate::product::logical_codebase::types::MemberStatus::Active,
                created_at: "2026-10-04T00:00:00Z".to_string(),
                updated_at: "2026-10-04T00:00:00Z".to_string(),
            },
        )
        .expect("member");
    lc_store
        .save_checkout(
            "project_0001",
            &crate::product::logical_codebase::types::RepositoryCheckoutRecord {
                checkout_id,
                logical_repository_id: member_id,
                physical_repository_id: "repository_member_a".to_string(),
                kind: crate::product::logical_codebase::types::CheckoutKind::Main,
                canonical_path: canonical.clone(),
                checkout_path_hash: crate::product::id::repo_hash_for_path(
                    canonical.to_string_lossy().as_ref(),
                ),
                git_dir_identity: source.git_dir_identity(),
                revision: None,
                availability:
                    crate::product::logical_codebase::types::CheckoutAvailability::Available,
                observed_at: "2026-10-04T00:00:00Z".to_string(),
                created_at: "2026-10-04T00:00:00Z".to_string(),
                updated_at: "2026-10-04T00:00:00Z".to_string(),
            },
        )
        .expect("checkout");
}

/// ①③④⑤（HTTP 面）：advance 把 owner 报告现场真正推起来、重复调用不双推、
/// 失败步骤停住不重试、GET 仍零写入。
#[tokio::test]
async fn bootstrap_advance_drives_the_field_state_and_stops_at_a_failed_step() {
    let fixture = trust_route_fixture(false);
    advance_seed_member(&fixture);
    let base = format!(
        "/api/projects/project_0001/logical-codebases/{}",
        fixture.lc_id
    );

    // 现场：只有后三步未开始，且各自只暴露 prepare。
    let (status, before) = get_json(&fixture.app, &format!("{base}/bootstrap")).await;
    assert_eq!(status, StatusCode::OK, "{before}");
    assert_eq!(before["planning_ready"], false, "{before}");
    for step in ["rules_policy", "member_index", "aggregate_index_active"] {
        assert_eq!(bootstrap_step_status(&before, step), "not_started", "{before}");
        assert_eq!(
            bootstrap_step_actions(&before, step),
            vec!["prepare".to_string()],
            "{before}"
        );
    }

    // 第一次 advance：把根配方长任务（rules/policy + member index）推起来。
    let (status, first) = advance_post(&fixture.app, &fixture.lc_id).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(advanced_step_names(&first), vec!["rules_policy".to_string()]);
    assert_eq!(first["stopped_reason"], "in_progress", "{first}");
    assert_eq!(first["stopped_step"], "rules_policy", "{first}");

    // 第二次 advance：同一根配方不得被推第二遍。
    let (status, second) = advance_post(&fixture.app, &fixture.lc_id).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert!(
        !advanced_step_names(&second).contains(&"rules_policy".to_string()),
        "a repeated advance must not push the root recipe again: {second}"
    );
    let operation_id = sole_operation_id(&fixture);

    // 根配方收口：rules/policy 与 member index 均 Completed。
    let operations = root_policy_operations(&fixture);
    let operation = root_policy_poll_terminal(&operations, &operation_id).await;
    assert_eq!(
        operation.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Completed,
        "{:?}",
        operation.error
    );
    assert!(
        !operation.full_recipe_requested,
        "the advance path must go through the detection gate"
    );
    let (status, after_recipe) = get_json(&fixture.app, &format!("{base}/bootstrap")).await;
    assert_eq!(status, StatusCode::OK, "{after_recipe}");
    assert_eq!(
        bootstrap_step_status(&after_recipe, "rules_policy"),
        "completed",
        "{after_recipe}"
    );
    assert_eq!(
        bootstrap_step_status(&after_recipe, "member_index"),
        "completed",
        "{after_recipe}"
    );

    // detached index build 以必缺 codegraph 二进制确定性失败 → advance 停在
    // failed 且绝不改写成 completed/重试。
    let mut failed_projection = None;
    for _ in 0..600 {
        let (status, projection) = get_json(&fixture.app, &format!("{base}/bootstrap")).await;
        assert_eq!(status, StatusCode::OK, "{projection}");
        if bootstrap_step_status(&projection, "aggregate_index_active") == "failed" {
            failed_projection = Some(projection);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let failed_projection =
        failed_projection.expect("the detached index build must fail deterministically");
    assert_eq!(failed_projection["planning_ready"], false, "{failed_projection}");
    let (status, halted) = advance_post(&fixture.app, &fixture.lc_id).await;
    assert_eq!(status, StatusCode::OK, "{halted}");
    assert_eq!(halted["stopped_reason"], "failed", "{halted}");
    assert_eq!(halted["stopped_step"], "aggregate_index_active", "{halted}");
    assert_eq!(halted["stopped_reason_code"], "aggregate_index_failed", "{halted}");
    assert!(
        advanced_step_names(&halted).is_empty(),
        "a failed step may never be retried automatically: {halted}"
    );
    assert_eq!(
        bootstrap_step_status(&halted["projection"], "aggregate_index_active"),
        "failed",
        "{halted}"
    );

    // ⑤ GET 只读：推进之后再次 GET 投影不产生任何 durable 写入。
    let inventory_before = aria_inventory(&fixture.root);
    let (status, _) = get_json(&fixture.app, &format!("{base}/bootstrap")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        aria_inventory(&fixture.root),
        inventory_before,
        "GET must stay a pure projection"
    );
}

/// owner 2026-10-29 验收 ②：**已初始化根**上的**自动推进**跳过四条
/// Cadence-skills 命令，只发布政策并冻结跳过式 receipt
/// （手动「启动聚合初始化」在同样现场强制完整五步，见 skip_skills.inc.rs）。
#[tokio::test]
async fn bootstrap_advance_skips_the_four_commands_on_an_initialized_root() {
    let fixture = trust_route_fixture(false);
    preinitialize_aggregate_root(&fixture);
    advance_seed_member(&fixture);

    let (status, body) = advance_post(&fixture.app, &fixture.lc_id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        advanced_step_names(&body).contains(&"rules_policy".to_string()),
        "advance must prepare the root recipe: {body}"
    );

    let operations = root_policy_operations(&fixture);
    let operation_id = sole_operation_id(&fixture);
    let operation = root_policy_poll_terminal(&operations, &operation_id).await;
    assert_eq!(
        operation.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Completed,
        "{:?}",
        operation.error
    );
    assert!(
        !operation.full_recipe_requested,
        "the advance path must not force the full recipe"
    );
    assert_eq!(
        operation
            .steps
            .iter()
            .map(|step| step.status.as_str())
            .collect::<Vec<_>>(),
        vec!["completed", "completed", "skipped", "skipped", "skipped"],
        "an initialized root must skip the four skills commands on the auto path"
    );
    let decision = operation
        .skills_initialization
        .clone()
        .expect("the skip decision must be durable on the operation");
    assert_eq!(
        decision.kind,
        crate::product::logical_codebase::aggregate_initialization::AggregateSkillsInitializationKind::Skipped
    );
    assert_eq!(
        fixture.factory.audit().stream_launches(),
        0,
        "the auto path must start zero provider turns on an initialized root"
    );
    assert!(
        root_policy_receipts(&fixture)
            .list_commands("project_0001", &operation_id)
            .expect("command receipts")
            .is_empty(),
        "a skipped auto run must audit no command"
    );

    // 跳过式 receipt：无命令摘要、冻结 policy/rule 双摘要与判定依据。
    let receipt = root_policy_receipts(&fixture)
        .get("project_0001", &operation_id)
        .expect("final receipt read")
        .expect("skipped final receipt");
    let canonical_root = root_policy_canonical_root(&fixture);
    assert!(receipt.commands.is_empty());
    assert_eq!(receipt.canonical_root, canonical_root);
    let receipt_decision = receipt
        .skills_initialization
        .as_ref()
        .expect("a skipped receipt must carry the skip evidence");
    assert!(receipt_decision.is_skipped());
    assert_eq!(receipt_decision.markers, decision.markers);

    // 政策来自预先存在的根材料（未被 provider 覆盖）。
    let artifact = root_policy_store(&fixture)
        .get("project_0001")
        .expect("policy artifact read")
        .expect("published policy artifact");
    assert!(!artifact.is_bootstrap_placeholder());
    assert_eq!(
        artifact.policy_text,
        root_policy_expected_text(PREINITIALIZED_AGENTS, PREINITIALIZED_LANGUAGE)
    );
    assert_eq!(receipt.policy_digest, artifact.digest);
}

/// advance 端点对未知 LC fail-closed（与 GET/actions 同一 authority 语义），
/// 不猜路径、不产生任何 durable 事实。
#[tokio::test]
async fn bootstrap_advance_rejects_an_unknown_logical_codebase() {
    let fixture = trust_route_fixture(false);
    let (status, body) = advance_post(&fixture.app, "logical_codebase_missing").await;
    assert_ne!(status, StatusCode::OK, "{body}");
    assert!(
        root_policy_operations(&fixture)
            .list("project_0001")
            .expect("operation list")
            .is_empty(),
        "an unknown logical codebase must not start any operation"
    );
}
