
// ---------------------------------------------------------------------------
// A04：Failed identity journal 修复 + 成员规则缺失的 readiness 等待面与恢复链。
// ---------------------------------------------------------------------------

/// 场景构造：legacy 布局 + 既有 fault injector 在 authority 写后中断一次；
/// 真实 git remote 漂移让原链复核 fail-closed 落 Failed journal。
struct FailFirstAuthorityWrite;

impl MigrationFaultInjector for FailFirstAuthorityWrite {
    fn after_authority_write(
        &self,
        _project_id: &str,
        _mapping: &RepositoryIdentityMapping,
    ) -> Result<(), ProductStoreError> {
        Err(ProductStoreError::Io(
            "test-injected interruption after the authority write".to_string(),
        ))
    }
}

fn legacy_record(id: &str, path: &Path) -> RepositoryRecord {
    RepositoryRecord {
        id: id.to_string(),
        project_id: PROJECT_ID.to_string(),
        name: id.to_string(),
        path: path.to_path_buf(),
        repo_hash: format!("legacy-hash-{id}"),
        runtime_root: PathBuf::from("/unused/.aria/runtime"),
        default_policy_preset: "manual-write".to_string(),
        default_provider_mode: "fake".to_string(),
        created_at: "2026-09-29T00:00:00Z".to_string(),
        updated_at: "2026-09-29T00:00:00Z".to_string(),
        logical_repository_id: None,
        primary_checkout_id: None,
        identity_schema_version: 0,
    }
}

fn legacy_lc_id(project_id: &str) -> String {
    use sha2::Digest;
    let digest = format!("{:x}", sha2::Sha256::digest(project_id.as_bytes()));
    format!("logical_codebase_{}", &digest[..32])
}

#[tokio::test]
async fn a04_failed_identity_and_missing_rules_require_product_repair_before_read_switch() {
    let root = tempdir().expect("root");
    let root_path = root.path().to_path_buf();
    crate::provider_capability_fixture::seed_local_skills_source(&root_path);
    let paths = ProductAppPaths::new(root_path.join(".aria"));

    // legacy 存量布局：两个真实 git 成员仓（含实际规则文件；代表性查询
    // 的验收要求命中 ≥2 个成员）+ repos.json seed。
    let legacy_root = root_path.join("legacy");
    let repository = legacy_root.join("repository");
    let repository_b = legacy_root.join("repository_0002");
    for (member, remote) in [
        (&repository, "ssh://git@example.test/acme/api.git"),
        (&repository_b, "ssh://git@example.test/acme/web.git"),
    ] {
        git_repo_at(member);
        std::fs::write(member.join("lib.rs"), "pub fn cross_repo_greeting() {}\n")
            .expect("write lib");
        run_git(member, &["add", "."]);
        run_git(member, &["commit", "-q", "-m", "init"]);
        commit_rule_file(member, "# language rule\n\n- Use Rust 2024 edition.\n");
        run_git(member, &["remote", "add", "origin", remote]);
    }

    let (app, factory) = a04_app(&root_path);
    let (status, _) = request_json(
        &app,
        Method::POST,
        "/api/projects",
        json!({"name":"A04 identity repair","description":null}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // 片4（REQ-BOOT-07）：选择器前置，种 project 作用域 Confirmed。必须在
    // project 记录落地后：seed 会先建 `projects/project_0001/` 目录，若先
    // seed 再 POST /api/projects，序号扫描（只看目录名）会把新 project 分到
    // project_0002，后续硬编码 project_0001 的路由即 404 project_not_found。
    crate::provider_capability_fixture::seed_confirmed_claude(&paths, PROJECT_ID);
    write_json(
        &paths.project_root(PROJECT_ID).join("repos.json"),
        &vec![
            legacy_record("repository_0001", &repository),
            legacy_record("repository_0002", &repository_b),
        ],
    )
    .expect("seed legacy repos.json");

    // 既有 migration fault injector：authority 写后中断（journal 停在
    // WritingAuthority，registry 已有 active 条目）。
    IdentityMigrationExecutor::with_fault_injector(
        paths.clone(),
        Arc::new(FailFirstAuthorityWrite),
    )
    .ensure_through_authority(PROJECT_ID)
    .expect_err("injected interruption must surface");

    // 真实 source digest 漂移：origin remote 改名 → 原链复核 fail-closed 落
    // Failed journal（产品路径，非 JSON 编辑）。
    run_git(
        &repository,
        &[
            "remote",
            "set-url",
            "origin",
            "ssh://git@example.test/acme/renamed.git",
        ],
    );
    IdentityMigrationExecutor::new(paths.clone())
        .ensure_identity_schema(PROJECT_ID)
        .expect_err("drifted source identity must fail closed");

    let lc_id = legacy_lc_id(PROJECT_ID);
    let repair_uri =
        format!("/api/projects/{PROJECT_ID}/logical-codebases/{lc_id}/identity-repair");
    let bootstrap = bootstrap_uri(&lc_id);
    let members_uri = format!("/api/projects/{PROJECT_ID}/logical-codebases/{lc_id}/members");

    // Task 7 HTTP 断言：GET repair 直读 journal/registry/repos.json 低层事实，
    // 不经普通成员列表；digest/候选/影响范围可见。
    let (status, diagnostic) = request_json(&app, Method::GET, &repair_uri, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{diagnostic}");
    assert_eq!(diagnostic["phase"], "failed");
    assert_eq!(diagnostic["project_id"], PROJECT_ID);
    assert!(
        diagnostic["source_repos_digest"]
            .as_str()
            .unwrap_or("")
            .starts_with("sha256:")
    );
    assert_eq!(
        diagnostic["observed_source_repos_digest"],
        diagnostic["source_repos_digest"]
    );
    assert_eq!(diagnostic["mappings"].as_array().map(Vec::len), Some(2));
    let conflicts = diagnostic["conflicts"].as_array().expect("conflicts");
    assert!(
        conflicts.iter().any(|conflict| conflict
            .as_str()
            .unwrap_or("")
            .starts_with("mapping_source_identity_mismatch")),
        "digest drift must be visible: {diagnostic}"
    );
    assert!(!diagnostic["impact"].as_array().unwrap().is_empty());
    // journal_updated_at 可补读（POST 的 expected 身份来源）。
    let journal_updated_at = diagnostic["journal_updated_at"]
        .as_str()
        .expect("journal updated_at")
        .to_string();

    // mapping 未确认前的零变更基线：authority 子树字节、成员数、provider
    // 启动计数、read mode（未切读）。
    let authority_before = authority_inventory(&root_path, &lc_id);
    let (status, members_before) = request_json(&app, Method::GET, &members_uri, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{members_before}");
    let member_count_before = members_before["members"].as_array().map(Vec::len);
    assert_eq!(factory.audit().stream_launches(), 0);
    assert_ne!(diagnostic["read_mode"], json!("logical"));

    // 提交错误 mapping：仍 waiting（拒绝、零写入）。
    let (status, rejected) = request_json(
        &app,
        Method::POST,
        &repair_uri,
        json!({
            "command_id": "cmd-a04-wrong-mapping-1",
            "expected_journal_updated_at": journal_updated_at,
            "action": "submit_mapping",
            "mapping": {
                "legacy_repository_id": "repository_0001",
                "source_identity_digest": "sha256:wrong-digest",
                "logical_repository_id": "00000000-0000-0000-0000-000000000001",
                "primary_checkout_id": "00000000-0000-0000-0000-000000000002",
                "physical_repository_id": "repository_0001",
                "idempotency_key": "",
            },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{rejected}");
    assert_eq!(rejected["code"], "identity_repair_rejected");
    assert_eq!(authority_inventory(&root_path, &lc_id), authority_before);

    // 恢复同一实际来源（origin remote 改回）→ 冲突清空、候选可见。
    run_git(
        &repository,
        &[
            "remote",
            "set-url",
            "origin",
            "ssh://git@example.test/acme/api.git",
        ],
    );
    let (status, clean) = request_json(&app, Method::GET, &repair_uri, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{clean}");
    assert_eq!(clean["conflicts"], json!([]));
    let candidates = clean["candidates"].as_array().expect("candidates");
    assert_eq!(candidates.len(), 1);
    let candidate = &candidates[0];
    let journal_updated_at = clean["journal_updated_at"]
        .as_str()
        .expect("journal updated_at after clean")
        .to_string();

    // 提交完整 mapping：staged（journal 保留 Failed + repair audit），read
    // mode/authority 仍未切换。
    let (status, staged) = request_json(
        &app,
        Method::POST,
        &repair_uri,
        json!({
            "command_id": "cmd-a04-correct-mapping-1",
            "expected_journal_updated_at": journal_updated_at,
            "action": "submit_mapping",
            "mapping": {
                "legacy_repository_id": candidate["legacy_repository_id"],
                "source_identity_digest": candidate["source_identity_digest"],
                "logical_repository_id": candidate["logical_repository_id"],
                "primary_checkout_id": candidate["primary_checkout_id"],
                "physical_repository_id": candidate["physical_repository_id"],
                "idempotency_key": "",
            },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{staged}");
    assert_eq!(staged["phase"], "failed");
    let journal_updated_at = staged["journal_updated_at"]
        .as_str()
        .expect("journal updated_at after staging")
        .to_string();
    assert_ne!(
        journal_updated_at,
        clean["journal_updated_at"].as_str().unwrap_or(""),
        "staging must append the durable repair audit"
    );
    assert_eq!(authority_inventory(&root_path, &lc_id), authority_before);
    assert_ne!(staged["read_mode"], json!("logical"));

    // Revalidate：核验成功后才切换读；原 journal 保留（audit 可补读）。
    let (status, repaired) = request_json(
        &app,
        Method::POST,
        &repair_uri,
        json!({
            "command_id": "cmd-a04-revalidate-1",
            "expected_journal_updated_at": journal_updated_at,
            "action": "revalidate",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{repaired}");
    // 切读 marker（`switching_reads` + read_mode=logical_authoritative）即
    // 核验成功后的 durable 终态；此前任何阶段 read_mode 都不得是 logical。
    assert_eq!(repaired["phase"], "switching_reads");
    assert_eq!(repaired["read_mode"], "logical_authoritative");
    // 成员数与 provider 计数在修复全程不变。
    let (status, members_after) = request_json(&app, Method::GET, &members_uri, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{members_after}");
    assert_eq!(
        members_after["members"].as_array().map(Vec::len),
        member_count_before
    );
    assert_eq!(factory.audit().stream_launches(), 0);

    // ---- 缺失成员规则：readiness 等待面 → 注入中断检查点 → 恢复 → 原链继续 ----
    let rules_path = repository.join(".claude/rules/language.md");
    std::fs::remove_file(&rules_path).expect("delete the actual member rules");

    // bootstrap 投影的 rules_policy 步只读检查（Task 1.6 起为 root
    // authority 三源谓词，成员规则检查归属 provider admission 预检）：
    // waiting_for_human + root_recipe_operation_missing（policy 可解析但
    // 本 LC 尚无 root recipe operation），提供 Prepare。
    let (status, waiting) = request_json(&app, Method::GET, &bootstrap, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{waiting}");
    let rules_step = bootstrap_step(&waiting, "rules_policy");
    assert_eq!(rules_step["status"], "waiting_for_human");
    assert_eq!(
        rules_step["failure"]["reason_code"],
        "root_recipe_operation_missing"
    );
    let allowed = rules_step["allowed_actions"].as_array().expect("actions");
    assert!(allowed.contains(&json!("prepare")));

    // identity 修复完成后（marker 终态非 Failed、成员在册）冷启动身份步完成。
    assert_eq!(bootstrap_step(&waiting, "identity")["status"], "completed");

    // 触发 root recipe(成员规则仍缺失)。fix-admission-member-rules-scope
    // 新合同(取代 lcg Task 3a/A9「任何相位阻断」):有效 bootstrap 凭据豁免
    // 成员规则门——裸成员的 provider turn 不再被 admission 预检拒绝
    // (member_language_rules_missing 不进 durable error),失败检查点改由
    // 注入的 pre_check 中断保留(第一次启动即消费,deterministic checkpoint,
    // 中断先于成功启动,launch 计数不变)。
    let (status, accepted) = request_json(
        &app,
        Method::POST,
        &format!("/api/projects/{PROJECT_ID}/logical-codebases/{lc_id}/initializations"),
        json!({"idempotency_key":"key-a04-rules-missing"}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    let operation_id = accepted["operation_id"]
        .as_str()
        .expect("operation id")
        .to_string();
    let denied = poll_until(
        &app,
        &bootstrap,
        |projection| bootstrap_step(projection, "member_index")["status"] == "failed",
        "the injected pre_check interruption fails the first bootstrap run",
    )
    .await;
    // 失败事实落在 durable operation 记录的 error(投影 detail 只带 stage
    // 摘要):pre_check turn 的失败可补读,且必须是注入的中断而非
    // admission 阻断(凭据豁免已生效)。
    let (_, operation_denied) = request_json(
        &app,
        Method::GET,
        &format!(
            "/api/projects/{PROJECT_ID}/logical-codebases/{lc_id}/initializations/{operation_id}"
        ),
        json!({}),
    )
    .await;
    assert_eq!(
        operation_denied["error"]["code"], "aggregate_pre_check_failed",
        "durable error must name the interrupted turn: {operation_denied}"
    );
    // 契约迁移(报 Main):error.message 已是 copy 标题,机器事实按契约在
    // details.action/detail(dto/aggregate_initialization.rs 明文,
    // operation_http.rs 同款先例),断言语义不变。
    let durable_details = &operation_denied["error"]["details"];
    let durable_action = durable_details["action"].as_str().unwrap_or("");
    let durable_detail = durable_details["detail"].as_str().unwrap_or("");
    assert!(
        !durable_detail.contains("member_language_rules_missing")
            && !durable_action.contains("member_language_rules_missing"),
        "bootstrap credential waives the member rules gate, got: {operation_denied}"
    );
    assert!(
        !durable_detail.contains("provider admission preflight denied")
            && !durable_action.contains("provider admission preflight denied"),
        "the failure must be the injected interruption, not an admission denial: {operation_denied}"
    );
    assert!(
        durable_action.contains("gateway start_streaming failed")
            && durable_detail
                .contains("test-injected interruption during the pre_check provider turn"),
        "the injected turn failure must be observable and attributable: {operation_denied}"
    );
    assert_eq!(
        factory.audit().stream_launches(),
        0,
        "the injected interruption must fire before a successful launch"
    );

    // Task 4(生产同构,与 a03 同款):根规则材料由 provider 在
    // RuleAndMcpConfig 轮生成(fixture 轮次合同),窗口内无需手工预写
    // AGENTS;成员规则不恢复(真实裸成员推进 planning-ready)。

    // 显式 Retry:注入中断已消费,原链继续(三个 provider turn 各启动一次,
    // detached index build 落 active);成员文件树零写(bootstrap 不生成
    // 成员规则——规则文件保持删除状态)。
    let (status, ready_result) = request_json(
        &app,
        Method::POST,
        &format!("{}/actions", bootstrap),
        json!({
            "command_id": "cmd-a04-member-index-retry-1",
            "step": "member_index",
            "action": "retry",
            "expected_revision": denied["membership_revision"].as_u64(),
            "expected_object_id": operation_id,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{ready_result}");

    let ready = poll_until(
        &app,
        &bootstrap,
        |projection| projection["planning_ready"] == true,
        "the bare member chain reaches planning_ready after the explicit retry",
    )
    .await;
    assert_eq!(
        factory.audit().stream_launches(),
        3,
        "the bare chain must launch exactly three provider turns"
    );
    // 成员仓零写:被删除的规则文件不被重建(bootstrap 不复制/生成成员规则)。
    assert!(
        !rules_path.exists(),
        "member rules must stay absent (zero write)"
    );
    // 冻结正确的 envelope/policy digest：两次补读一致且非空。
    let digest = ready["policy"]["policy_digest"]
        .as_str()
        .expect("policy digest")
        .to_string();
    assert!(digest.starts_with("sha256:"));
    let (status, reread) = request_json(&app, Method::GET, &bootstrap, json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        reread["policy"]["policy_digest"].as_str(),
        Some(digest.as_str())
    );
    assert_eq!(reread["planning_ready"], true);
}
