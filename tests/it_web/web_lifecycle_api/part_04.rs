// ---- Logical 分支族（issue/story/design specs Logical 注入）+ Confirm 逻辑族 + review_status 投影 ----
// 自 part_02.rs 纯移动拆分（无行为变化）；helpers（seed_logical_codebase 等）仍留 part_02（include! 平铺同作用域）。

#[tokio::test]
async fn logical_issue_lifecycle_does_not_require_repo_id() {
    // 多仓 issue（manifest+selection，无 repo_id）→ GET issue_lifecycle → 200，
    // 不报 repository_required / repository 相关 4xx。
    let root = tempdir().expect("root");
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Lifecycle","description":null}),
    )
    .await;

    // 先建多仓 issue（repo_id=None，成为 issue_0001），再写 selection —— 避免 selection 的
    // issue_0001 目录被 count_entries 计入导致 issue 变成 issue_0002。
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    IssueStore::new(app_paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: None,
            logical_codebase_id: None,
            title: "多仓聚合 Issue".to_string(),
            description: Some("跨 api 仓库的聚合变更".to_string()),
            change_id: None,
                   base_branch: None,
 })
        .expect("multi-repo issue");
    let member_id = LogicalRepositoryId(uuid::Uuid::from_u128(1));
    seed_logical_codebase(&app_paths, member_id);

    let (status, lifecycle) = request_json(
        app,
        Method::GET,
        "/api/issues/issue_0001/lifecycle?project_id=project_0001",
        json!({}),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Logical issue_lifecycle must not require repo_id: {lifecycle}"
    );
    assert_ne!(lifecycle["code"], "repository_required");
    assert_eq!(lifecycle["issue"]["repo_id"], Value::Null);
    assert_eq!(lifecycle["story_specs"].as_array().unwrap().len(), 0);
    assert_eq!(lifecycle["work_items"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn generate_story_specs_logical_branch_injects_aggregate_prompt() {
    // 多仓 issue → POST story-specs:generate → 200，story 为草稿态聚合视野
    // （aggregate_codebase=Some），session context message 含 inventory + 聚合指令。
    let root = tempdir().expect("root");
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Lifecycle","description":null}),
    )
    .await;

    // 先建多仓 issue（repo_id=None，成为 issue_0001），再写 selection。
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    IssueStore::new(app_paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: None,
            logical_codebase_id: None,
            title: "多仓聚合 Story".to_string(),
            description: Some("跨 api 仓库的聚合变更".to_string()),
            change_id: None,
                   base_branch: None,
 })
        .expect("multi-repo issue");
    let member_id = LogicalRepositoryId(uuid::Uuid::from_u128(1));
    seed_logical_codebase(&app_paths, member_id);

    let (status, story_response) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({
            "title":"聚合 Story Spec",
            "author_provider":"fake",
            "reviewer_provider":"codex",
            "review_rounds":3,
            "superpowers_enabled":false,
            "openspec_enabled":false
        }),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Logical story-specs:generate must succeed: {story_response}"
    );
    // 草稿态聚合 story（involved 空、Draft），repository_id 为空串（Logical 无单仓 repo_id）。
    let story = &story_response["story_specs"][0];
    assert_eq!(story["repository_id"], "");

    // session context message 注入 inventory + 聚合视野指令。
    let messages = story_response["workspace_session"]["messages"]
        .as_array()
        .unwrap();
    let context = messages
        .iter()
        .find(|message| {
            message["role"] == "system"
                && message["content"]
                    .as_str()
                    .is_some_and(|content| content.contains("Workspace 生成任务已准备"))
        })
        .expect("generation context message");
    let content = context["content"].as_str().unwrap();
    assert!(
        content.contains("聚合代码库成员清单"),
        "缺 inventory：{content}"
    );
    assert!(
        content.contains("00000000-0000-0000-0000-000000000001"),
        "缺成员行：{content}"
    );
    assert!(
        content.contains("involved_repository_ids"),
        "缺聚合指令：{content}"
    );
    assert!(
        content.contains("禁止回落到任意单一 primary 仓库"),
        "缺禁止 primary 回落指令：{content}"
    );
}

#[tokio::test]
async fn generate_story_specs_pinned_involved_derives_focus_for_member_anchored_launch() {
    // r23 指纹漂移维度钉死(story resume superseded 根因):AI 自决流的首轮
    // launch 锚聚合根视图(记录 involved 空 → aggregate_root_view),AI 回写
    // involved/focus 后 revision 轮锚成员 checkout → 指纹 target 三元组 +
    // git identity 维度漂移 → 按设计 superseded(resume_fingerprint_mismatch)。
    // 调用方钉定 involved 是让首轮即锚成员的唯一入口,但 pin 面 previously
    // 只落 involved、focus 恒 None——钉定后路由恒 TargetMissing
    // (workspace_repository.rs story None+involved 非空臂),pin API 结构性
    // 不可用。修:钉定 involved 非空时 focus=involved 首成员,story 首轮
    // 即锚成员 checkout,门上修订指纹可比对(原生恢复可达)。
    let root = tempdir().expect("root");
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Lifecycle","description":null}),
    )
    .await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    IssueStore::new(app_paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: None,
            logical_codebase_id: None,
            title: "多仓聚合 Story pin".to_string(),
            description: Some("alpha 仓交付".to_string()),
            change_id: None,
                   base_branch: None,
})
        .expect("multi-repo issue");
    let member_id = LogicalRepositoryId(uuid::Uuid::from_u128(1));
    seed_logical_codebase(&app_paths, member_id);

    let (status, story_response) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({
            "title":"聚合 Story Spec(钉定单成员)",
            "involved_repository_ids": [member_id.0.to_string()],
            "author_provider":"fake",
            "reviewer_provider":"codex",
            "review_rounds":1,
            "superpowers_enabled":false,
            "openspec_enabled":false
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "钉定 involved 的 story generate 必须成功: {story_response}"
    );
    let story_id = story_response["story_specs"][0]["story_spec_id"]
        .as_str()
        .expect("story id");
    let record = LifecycleStore::new(app_paths)
        .list_story_specs("project_0001", "issue_0001")
        .expect("story records")
        .into_iter()
        .find(|record| record.id == story_id)
        .expect("created story record");
    assert_eq!(record.involved_repository_ids, vec![member_id]);
    assert_eq!(
        record.focus_repository_id,
        Some(member_id),
        "钉定 involved 非空时必须派生 focus(否则路由恒 TargetMissing,首轮锚聚合根,修订指纹必漂移 supersede)"
    );
}

#[tokio::test]
async fn generate_design_specs_logical_branch_injects_aggregate_prompt() {
    // 多仓 issue → POST design-specs:generate → 200，design 为草稿态聚合视野
    // （aggregate_codebase=Some），session context message 注入 aggregate_design prompt
    // （inventory + involved/change_order/depends_on 指令）。C1 回归保护：Design Logical
    // 分支不得因 issue.repo_id=None 在 workspace_entity_context（issue_repo_id）处失败。
    let root = tempdir().expect("root");
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Lifecycle","description":null}),
    )
    .await;

    // 先建多仓 issue（repo_id=None，成为 issue_0001），再写 selection。
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    IssueStore::new(app_paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: None,
            logical_codebase_id: None,
            title: "多仓聚合 Design".to_string(),
            description: Some("跨 api 仓库的聚合设计".to_string()),
            change_id: None,
                   base_branch: None,
 })
        .expect("multi-repo issue");
    let member_id = LogicalRepositoryId(uuid::Uuid::from_u128(1));
    seed_logical_codebase(&app_paths, member_id);

    // Design 生成要求至少一个 Confirmed story（validate_confirmed_story_specs）。
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            title: "前置 Story Spec".to_string(),
            aggregate_codebase: None,
        })
        .expect("confirmed story");
    lifecycle
        .update_spec_confirmation_status(
            "project_0001",
            "issue_0001",
            &story.id,
            LifecycleConfirmationStatus::Confirmed,
        )
        .expect("confirm story");

    let (status, design_response) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/design-specs:generate",
        json!({
            "title":"聚合 Design Spec",
            "story_spec_ids":[story.id],
            "author_provider":"fake",
            "reviewer_provider":"codex",
            "review_rounds":3,
            "superpowers_enabled":false,
            "openspec_enabled":false
        }),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "Logical design-specs:generate must succeed: {design_response}"
    );
    // 草稿态聚合 design（involved 空、Draft）。
    assert_eq!(
        design_response["design_specs"][0]["confirmation_status"],
        "draft"
    );

    // session context message 注入 inventory + 聚合视野指令（含 change_order/depends_on）。
    let messages = design_response["workspace_session"]["messages"]
        .as_array()
        .unwrap();
    let context = messages
        .iter()
        .find(|message| {
            message["role"] == "system"
                && message["content"]
                    .as_str()
                    .is_some_and(|content| content.contains("Workspace 生成任务已准备"))
        })
        .expect("generation context message");
    let content = context["content"].as_str().unwrap();
    assert!(
        content.contains("聚合代码库成员清单"),
        "缺 inventory：{content}"
    );
    assert!(
        content.contains("00000000-0000-0000-0000-000000000001"),
        "缺成员行：{content}"
    );
    assert!(
        content.contains("involved_repository_ids"),
        "缺聚合指令：{content}"
    );
    assert!(
        content.contains("禁止回落到任意单一 primary 仓库"),
        "缺禁止 primary 回落指令：{content}"
    );
    assert!(
        content.contains("change_order"),
        "缺 change_order 指令：{content}"
    );
    assert!(
        content.contains("depends_on"),
        "缺 depends_on 依据：{content}"
    );
}


#[tokio::test]
async fn generate_design_specs_pinned_scope_pins_involved_in_context_prompt() {
    // pi-7 现场(原样输出钉定,e78a97c6)→ add-multi-repo-issue-entry(组2 2.4):
    // 钉定块上界化后,单成员 LC(focus=∅→上界=resolved effective=[alpha],即
    // 「单仓=上界 1」兼容形)仍必须注入钉定块且恰钉 [alpha]——单仓钉定行为
    // 不变断言:AI 输入面上界=[alpha],involved 非空子集即 [alpha] 本身。
    let root = tempdir().expect("root");
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Lifecycle","description":null}),
    )
    .await;

    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    IssueStore::new(app_paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: None,
            logical_codebase_id: None,
            title: "多仓聚合 Design 钉定".to_string(),
            description: Some("跨 api 仓库的聚合设计".to_string()),
            change_id: None,
            base_branch: None,
        })
        .expect("multi-repo issue");
    let member_id = LogicalRepositoryId(uuid::Uuid::from_u128(1));
    seed_logical_codebase(&app_paths, member_id);

    // Design 生成要求至少一个 Confirmed story(validate_confirmed_story_specs)。
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            title: "前置 Story Spec".to_string(),
            aggregate_codebase: None,
        })
        .expect("confirmed story");
    lifecycle
        .update_spec_confirmation_status(
            "project_0001",
            "issue_0001",
            &story.id,
            LifecycleConfirmationStatus::Confirmed,
        )
        .expect("confirm story");

    // 请求钉定单成员 involved+change_order(pinned_design_generate_body 同款面)。
    let (status, design_response) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/design-specs:generate",
        json!({
            "title":"聚合 Design Spec 钉定",
            "story_spec_ids":[story.id],
            "involved_repository_ids":[member_id.0.to_string()],
            "change_order":[member_id.0.to_string()],
            "author_provider":"fake",
            "reviewer_provider":"codex",
            "review_rounds":3,
            "superpowers_enabled":false,
            "openspec_enabled":false
        }),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "pinned Logical design-specs:generate must succeed: {design_response}"
    );
    // 钉定落 record 出生值(DTO 无 involved 投影,经 store 断言;非 AI 自决空)。
    let design_id = design_response["design_specs"][0]["design_spec_id"]
        .as_str()
        .expect("design spec id");
    let design_record = lifecycle
        .list_design_specs("project_0001", "issue_0001")
        .expect("list design specs")
        .into_iter()
        .find(|design| design.id == design_id)
        .expect("persisted design record");
    assert_eq!(
        design_record.involved_repository_ids,
        vec![member_id],
        "出生 involved 必须等于请求钉定"
    );
    assert_eq!(
        design_record.change_order,
        vec![member_id],
        "出生 change_order 必须等于请求钉定"
    );

    // session context message 注入上界钉定块:单成员上界=[alpha],非空子集唯一。
    let messages = design_response["workspace_session"]["messages"]
        .as_array()
        .unwrap();
    let context = messages
        .iter()
        .find(|message| {
            message["role"] == "system"
                && message["content"]
                    .as_str()
                    .is_some_and(|content| content.contains("Workspace 生成任务已准备"))
        })
        .expect("generation context message");
    let content = context["content"].as_str().unwrap();
    assert!(
        content.contains("聚合视野上界钉定"),
        "缺上界钉定块标题:{content}"
    );
    assert!(
        content.contains("involved_repository_ids 上界 = [00000000-0000-0000-0000-000000000001]"),
        "缺上界集合实值(单成员上界恰为 [alpha]):{content}"
    );
    assert!(
        content.contains("非空子集") && content.contains("界外"),
        "缺子集允许/界外即拒措辞:{content}"
    );
    assert!(
        content.contains("声明 blocker"),
        "缺范围异议修订出口:{content}"
    );
}

/// 多成员 focus fixture（add-multi-repo-issue-entry 组2）：N 成员真实 git checkout +
/// manifest + active members/checkouts + selection（include=全部成员，focus=勾选集）
/// + active index + policy bootstrap，使 `RepositoryRouting::load_for_issue` 判 Logical
/// 且 resolved 上界 = focus 原集（`resolved_upper_bound` 同源）。
fn seed_logical_codebase_members_with_focus(
    app_paths: &ProductAppPaths,
    members: &[(LogicalRepositoryId, &str)],
    focus: Vec<LogicalRepositoryId>,
) {
    let aggregate_root = app_paths.root().join("aggregate-root");
    std::fs::create_dir_all(&aggregate_root).expect("create aggregate-root fixture dir");
    let member_ids = members.iter().map(|(id, _)| *id).collect::<Vec<_>>();
    let manifest =
        LogicalCodebaseManifest::new("project_0001", aggregate_root.clone(), member_ids.clone());
    LogicalCodebaseStore::new(app_paths.clone())
        .save_manifest("project_0001", &manifest)
        .unwrap();
    let now = "2026-08-10T00:00:00Z".to_string();
    let mut snapshots = Vec::new();
    for (index, (member_id, alias)) in members.iter().enumerate() {
        let checkout_path = aggregate_root.join(alias);
        std::fs::create_dir_all(&checkout_path).expect("create checkout fixture dir");
        let init = Command::new("git")
            .args(["init", "-q"])
            .current_dir(&checkout_path)
            .output()
            .expect("initialize checkout git repository");
        assert!(
            init.status.success(),
            "git init fixture failed: {}",
            String::from_utf8_lossy(&init.stderr)
        );
        std::fs::write(checkout_path.join("lib.rs"), "pub fn fixture() {}\n")
            .expect("write checkout fixture source");
        let stage = Command::new("git")
            .args(["add", "lib.rs"])
            .current_dir(&checkout_path)
            .output()
            .expect("stage checkout fixture source");
        assert!(
            stage.status.success(),
            "git add fixture failed: {}",
            String::from_utf8_lossy(&stage.stderr)
        );
        let commit = Command::new("git")
            .args([
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.test",
                "commit",
                "-qm",
                "initial fixture",
            ])
            .current_dir(&checkout_path)
            .output()
            .expect("commit checkout fixture source");
        assert!(
            commit.status.success(),
            "git commit fixture failed: {}",
            String::from_utf8_lossy(&commit.stderr)
        );
        let revision_output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&checkout_path)
            .output()
            .expect("read checkout fixture revision");
        assert!(
            revision_output.status.success(),
            "git rev-parse fixture failed: {}",
            String::from_utf8_lossy(&revision_output.stderr)
        );
        let revision = String::from_utf8(revision_output.stdout)
            .expect("git revision is UTF-8")
            .trim()
            .to_string();
        let checkout_id = RepositoryCheckoutId(uuid::Uuid::from_u128(index as u128 + 1));
        let physical_repository_id = format!("repository_{alias}");
        LogicalCodebaseStore::new(app_paths.clone())
            .save_member(
                "project_0001",
                &CodebaseMemberRecord {
                    logical_repository_id: *member_id,
                    physical_repository_id: physical_repository_id.clone(),
                    alias: alias.to_string(),
                    role: "service".to_string(),
                    ordinal: index as u32 + 1,
                    source_identity: RepositorySourceIdentity::from_git_parts(
                        &checkout_path,
                        checkout_path.join(".git"),
                        Some(format!("ssh://git@example.test/acme/{alias}.git")),
                    ),
                    repo_type: RepositoryType::Backend,
                    tech_stack: vec!["rust".to_string()],
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: vec![checkout_id],
                    status: MemberStatus::Active,
                    created_at: now.clone(),
                    updated_at: now.clone(),
                },
            )
            .unwrap();
        LogicalCodebaseStore::new(app_paths.clone())
            .save_checkout(
                "project_0001",
                &RepositoryCheckoutRecord {
                    checkout_id,
                    logical_repository_id: *member_id,
                    physical_repository_id,
                    kind: CheckoutKind::Main,
                    canonical_path: checkout_path,
                    checkout_path_hash: format!("sha256:checkout-{alias}"),
                    git_dir_identity: format!("sha256:git-dir-{alias}"),
                    revision: Some(revision.clone()),
                    availability: CheckoutAvailability::Available,
                    observed_at: now.clone(),
                    created_at: now.clone(),
                    updated_at: now.clone(),
                },
            )
            .unwrap();
        snapshots.push(AggregateIndexMemberSnapshot::indexed(
            *member_id,
            checkout_id,
            revision,
            false,
            now.clone(),
        ));
    }
    IssueCodebaseSelectionStore::new(app_paths.clone())
        .save(&IssueCodebaseSelection::explicit(
            "project_0001",
            "issue_0001",
            member_ids,
            Vec::new(),
            focus,
            None,
        ))
        .unwrap();
    let index = AggregateIndexRecord::building(
        "aggregate_index_0002".to_string(),
        "project_0001".to_string(),
        manifest.membership_revision,
        snapshots,
        now.clone(),
    );
    AggregateIndexStore::new(app_paths.clone())
        .create("project_0001", index.clone())
        .unwrap();
    let mut activated = index;
    activated.status = AggregateIndexStatus::Active;
    AggregateIndexStore::new(app_paths.clone())
        .replace_active("project_0001", activated)
        .unwrap();
    AggregatePolicyArtifactStore::new(app_paths.clone())
        .ensure_bootstrap(&manifest)
        .unwrap();
}

/// 组2 共用前置:project + 多仓 issue + 双成员 focus fixture + Confirmed story。
/// 返回 (root, app, app_paths, story_id);focus=[api] 为 resolved 上界。
async fn multi_member_focus_design_fixture()
-> (tempfile::TempDir, axum::Router, ProductAppPaths, String) {
    let root = tempdir().expect("root");
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Lifecycle","description":null}),
    )
    .await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    IssueStore::new(app_paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: None,
            logical_codebase_id: None,
            title: "多仓聚合 Design 上界".to_string(),
            description: Some("跨 api/web 仓库的聚合设计".to_string()),
            change_id: None,
            base_branch: None,
        })
        .expect("multi-repo issue");
    let api = LogicalRepositoryId(uuid::Uuid::from_u128(1));
    let web = LogicalRepositoryId(uuid::Uuid::from_u128(2));
    seed_logical_codebase_members_with_focus(&app_paths, &[(api, "api"), (web, "web")], vec![api]);

    // Design 生成要求至少一个 Confirmed story(validate_confirmed_story_specs)。
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            title: "前置 Story Spec".to_string(),
            aggregate_codebase: None,
        })
        .expect("confirmed story");
    lifecycle
        .update_spec_confirmation_status(
            "project_0001",
            "issue_0001",
            &story.id,
            LifecycleConfirmationStatus::Confirmed,
        )
        .expect("confirm story");
    (root, app, app_paths, story.id)
}

#[tokio::test]
async fn generate_design_specs_pins_selection_focus_bound_in_context_prompt() {
    // add-multi-repo-issue-entry(组2 2.1):上界=selection.focus=[api](resolved 同源,
    // 与出生值/write-back/preflight 四面一口径);请求未钉 involved → prompt 钉定块
    // 仍注入上界集合+子集允许+界外即拒(AI 在勾选授权范围内自决收敛,可少于上界)。
    let (_root, app, _app_paths, story_id) = multi_member_focus_design_fixture().await;
    let api = uuid::Uuid::from_u128(1).to_string();
    let web = uuid::Uuid::from_u128(2).to_string();

    let (status, design_response) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/design-specs:generate",
        json!({
            "title":"聚合 Design Spec 上界钉定",
            "story_spec_ids":[story_id],
            "author_provider":"fake",
            "reviewer_provider":"codex",
            "review_rounds":3,
            "superpowers_enabled":false,
            "openspec_enabled":false
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "未钉 involved 的 Logical design-specs:generate 必须 200: {design_response}"
    );

    let messages = design_response["workspace_session"]["messages"]
        .as_array()
        .unwrap();
    let context = messages
        .iter()
        .find(|message| {
            message["role"] == "system"
                && message["content"]
                    .as_str()
                    .is_some_and(|content| content.contains("Workspace 生成任务已准备"))
        })
        .expect("generation context message");
    let content = context["content"].as_str().unwrap();
    assert!(
        content.contains("聚合视野上界钉定"),
        "缺上界钉定块标题:{content}"
    );
    assert!(
        content.contains(&format!("involved_repository_ids 上界 = [{api}]")),
        "缺上界集合实值(应恰为 focus=[api]):{content}"
    );
    assert!(
        !content.contains(&format!("involved_repository_ids 上界 = [{api}, {web}]")),
        "上界不得放宽为全部 effective 成员:{content}"
    );
    assert!(content.contains("非空子集"), "缺子集允许措辞:{content}");
    assert!(
        content.contains("界外") && content.contains("拒绝"),
        "缺界外即拒措辞:{content}"
    );
}

#[tokio::test]
async fn generate_design_specs_rejects_involved_outside_selection_focus_bound() {
    // add-multi-repo-issue-entry(组2 2.3,k3 P2-5):出生值校验——请求
    // involved_repository_ids ⊄ resolved 上界(focus=[api],请求含界外 web)→ 拒,
    // 收敛早于 preflight(web 为 effective 成员,存量口径会放行=修前缺口)。
    let (_root, app, _app_paths, story_id) = multi_member_focus_design_fixture().await;
    let api = uuid::Uuid::from_u128(1).to_string();
    let web = uuid::Uuid::from_u128(2).to_string();

    let (status, body) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/design-specs:generate",
        json!({
            "title":"界外出生值 Design",
            "story_spec_ids":[story_id],
            "involved_repository_ids":[api, web],
            "change_order":[api, web],
            "author_provider":"fake",
            "reviewer_provider":"codex",
            "review_rounds":3,
            "superpowers_enabled":false,
            "openspec_enabled":false
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "界外 involved 出生值必须 422 拒(error.rs 稳定码映射): {body}"
    );
    assert_eq!(
        body["code"], "involved_repository_not_effective",
        "出生值拒错误码: {body}"
    );
}

// ---- Task 6：confirm gate（多仓 involved + change_order 校验，3b 收紧）----
// ① involved 非空（REQ-PLN-04「AI 不确定即 blocker」）；② Design involved>1 必须 change_order
// （决策 3b，REQ-PLN-05 收紧）。单仓（logical_codebase_ref None）不校验，保持 Legacy 行为不变。
use cadence_aria::product::lifecycle_store::{
    AggregateDesignSpecScope, AggregateStorySpecScope, CreateDesignSpecInput,
};

/// 建多仓 issue + 聚合视野 Story/Design spec + 其 workspace session，返回 (root, router, session_id)。
/// 直接经 LifecycleStore 构造聚合视野（involved/change_order 由调用方指定），确认门只读 spec。
/// 必须保留返回的 TempDir（root）直到请求完成，否则目录被清理导致 404。
async fn create_logical_confirm_fixture(
    kind: WorkspaceType,
    involved: Vec<LogicalRepositoryId>,
    change_order: Vec<LogicalRepositoryId>,
) -> (tempfile::TempDir, axum::Router, String) {
    let root = tempdir().expect("root");
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    IssueStore::new(app_paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: None,
            logical_codebase_id: None,
            title: "多仓聚合 confirm".to_string(),
            description: None,
            change_id: None,
                   base_branch: None,
 })
        .expect("multi-repo issue");
    let effective = involved.clone();
    let entity_id = match kind {
        WorkspaceType::Story => {
            let story = lifecycle
                .create_story_spec(CreateStorySpecInput {
                    project_id: "project_0001".to_string(),
                    issue_id: "issue_0001".to_string(),
                    repository_id: String::new(),
                    title: "多仓 Story".to_string(),
                    aggregate_codebase: Some(AggregateStorySpecScope {
                        logical_codebase_ref: uuid::Uuid::from_u128(0x0100),
                        effective_member_ids: effective,
                        involved_repository_ids: involved,
                        focus_repository_id: None,
                    }),
                })
                .expect("logical story");
            story.id
        }
        WorkspaceType::Design => {
            let design = lifecycle
                .create_design_spec(CreateDesignSpecInput {
                    project_id: "project_0001".to_string(),
                    issue_id: "issue_0001".to_string(),
                    story_spec_ids: Vec::new(),
                    title: "多仓 Design".to_string(),
                    aggregate_codebase: Some(AggregateDesignSpecScope {
                        logical_codebase_ref: uuid::Uuid::from_u128(0x0100),
                        effective_member_ids: effective,
                        involved_repository_ids: involved,
                        change_order,
                    }),
                })
                .expect("logical design");
            design.id
        }
        _ => panic!("only Story/Design supported"),
    };
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id,
            workspace_type: kind,
            author_provider: ProviderName::Fake,
            reviewer_provider: Some(ProviderName::Codex),

            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .expect("workspace session");
    (root, app, session.id)
}

#[tokio::test]
async fn confirm_logical_design_without_change_order_is_blocked() {
    // 多仓 Design（involved>1，无 change_order）→ confirm → 4xx blocker（3b）。
    let m1 = LogicalRepositoryId(uuid::Uuid::from_u128(1));
    let m2 = LogicalRepositoryId(uuid::Uuid::from_u128(2));
    let (_root, app, session_id) =
        create_logical_confirm_fixture(WorkspaceType::Design, vec![m1, m2], Vec::new()).await;
    let (status, body) = request_json(
        app,
        Method::POST,
        &format!("/api/workspace-sessions/{session_id}/confirm"),
        json!({ "confirmed_by": "human" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "多仓 Design 缺 change_order 必须 4xx 阻断: {body}"
    );
    assert_eq!(body["code"], "change_order_required_for_logical_codebase");
}

#[tokio::test]
async fn confirm_logical_story_with_involved_succeeds() {
    // 多仓 Story（involved 非空）→ confirm → 200（3b：Story 不要求 change_order）。
    let member = LogicalRepositoryId(uuid::Uuid::from_u128(1));
    let (_root, app, session_id) =
        create_logical_confirm_fixture(WorkspaceType::Story, vec![member], Vec::new()).await;
    let (status, body) = request_json(
        app,
        Method::POST,
        &format!("/api/workspace-sessions/{session_id}/confirm"),
        json!({ "confirmed_by": "human" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "多仓 Story confirm 应成功: {body}");
    assert_eq!(body["status"], "confirmed");
}

#[tokio::test]
async fn confirm_logical_story_without_involved_is_blocked() {
    // 多仓 Story（involved 空）→ confirm → 4xx blocker（REQ-PLN-04「AI 不确定即 blocker」）。
    let (_root, app, session_id) =
        create_logical_confirm_fixture(WorkspaceType::Story, Vec::new(), Vec::new()).await;
    let (status, body) = request_json(
        app,
        Method::POST,
        &format!("/api/workspace-sessions/{session_id}/confirm"),
        json!({ "confirmed_by": "human" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "多仓 Story 缺 involved 必须 4xx 阻断: {body}"
    );
    assert_eq!(body["code"], "involved_repositories_undetermined");
}

#[tokio::test]
async fn confirm_logical_design_with_change_order_succeeds() {
    // 多仓 Design（involved>1，有 change_order）→ confirm → 200。
    let m1 = LogicalRepositoryId(uuid::Uuid::from_u128(1));
    let m2 = LogicalRepositoryId(uuid::Uuid::from_u128(2));
    let (_root, app, session_id) =
        create_logical_confirm_fixture(WorkspaceType::Design, vec![m1, m2], vec![m1, m2]).await;
    let (status, body) = request_json(
        app,
        Method::POST,
        &format!("/api/workspace-sessions/{session_id}/confirm"),
        json!({ "confirmed_by": "human" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "多仓 Design 带 change_order confirm 应成功: {body}"
    );
    assert_eq!(body["status"], "confirmed");
}

#[tokio::test]
async fn confirm_legacy_single_repo_story_without_involved_succeeds() {
    // 红线：单仓（logical_codebase_ref None）不校验 involved，保持 Legacy 行为不变。
    let root = tempdir().expect("root");
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    IssueStore::new(app_paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: Some("repository_0001".to_string()),
            logical_codebase_id: None,
            title: "单仓 Story confirm".to_string(),
            description: None,
            change_id: None,
                   base_branch: None,
 })
        .expect("legacy issue");
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            title: "单仓 Story".to_string(),
            aggregate_codebase: None,
        })
        .expect("legacy story");
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: story.id,
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::Fake,
            reviewer_provider: Some(ProviderName::Codex),

            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .expect("legacy session");
    let (status, body) = request_json(
        app,
        Method::POST,
        &format!("/api/workspace-sessions/{}/confirm", session.id),
        json!({ "confirmed_by": "human" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "单仓 Story confirm 不得被新门拦截: {body}"
    );
    assert_eq!(body["status"], "confirmed");
}

// F-26b（0497 现场锚）：design 审核与 plan Review Round 在 workspace timeline 有
// reviewer 节点证据，但生命周期工作台（issue 面）零展示——用户无从确认
// 「review 已做」。修复后：spec DTO 附带 review_status 投影（active reviewer 节点
// → running；否则 completed reviewer 节点 → completed；无证据 → null）。
#[tokio::test]
async fn lifecycle_projects_review_status_from_workspace_timeline_reviewer_runs() {
    use cadence_aria::web::workspace_ws_types::{
        ProviderConfigSnapshot, TimelineNode, TimelineNodeStatus, TimelineNodeType, WorkspaceStage,
    };

    fn reviewer_run_node(node_id: &str, status: TimelineNodeStatus) -> TimelineNode {
        TimelineNode {
            node_id: node_id.to_string(),
            node_type: TimelineNodeType::ReviewerRun,
            agent: Some(ProviderName::Codex),
            stage: WorkspaceStage::CrossReview,
            round: Some(1),
            status,
            title: "Review Round 1".to_string(),
            summary: None,
            started_at: "2026-09-21T00:00:00Z".to_string(),
            completed_at: Some("2026-09-21T00:01:00Z".to_string()),
            duration_ms: Some(60_000),
            artifact_ref: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Codex),
                review_rounds: 1,
                permission_modes: Default::default(),
            },
            retry: None,
        }
    }

    let root = tempdir().expect("root");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    cadence_aria::product::project_store::ProjectStore::new(app_paths.clone())
        .create(cadence_aria::product::project_store::CreateProjectInput {
            name: "Lifecycle".to_string(),
            description: None,
        })
        .expect("project");
    let lifecycle = LifecycleStore::new(app_paths.clone());
    IssueStore::new(app_paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: Some("repository_0001".to_string()),
            logical_codebase_id: None,
            title: "review evidence issue".to_string(),
            description: None,
            change_id: None,
                   base_branch: None,
 })
        .expect("issue");
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            title: "review evidence story".to_string(),
            aggregate_codebase: None,
        })
        .expect("story");
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: story.id,
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::Fake,
            reviewer_provider: Some(ProviderName::Codex),

            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .expect("session");

    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    let lifecycle_uri = "/api/issues/issue_0001/lifecycle?project_id=project_0001";

    // 1) 无 reviewer 节点 → review_status 缺省（null）。
    lifecycle
        .save_timeline_nodes(&session.id, &[])
        .expect("empty timeline nodes");
    let (status, body) = request_json(app.clone(), Method::GET, lifecycle_uri, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["story_specs"][0]["review_status"].is_null(),
        "no reviewer evidence must project null review_status: {body}"
    );

    // 2) 仅有 completed reviewer_run → completed。
    lifecycle
        .save_timeline_nodes(
            &session.id,
            &[reviewer_run_node(
                "timeline_node_002",
                TimelineNodeStatus::Completed,
            )],
        )
        .expect("completed reviewer node");
    let (status, body) = request_json(app.clone(), Method::GET, lifecycle_uri, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["story_specs"][0]["review_status"], "completed",
        "completed reviewer_run must project review_status=completed: {body}"
    );

    // 3) completed + active 并存 → running（进行中优先）。
    lifecycle
        .save_timeline_nodes(
            &session.id,
            &[
                reviewer_run_node("timeline_node_002", TimelineNodeStatus::Completed),
                reviewer_run_node("timeline_node_003", TimelineNodeStatus::Active),
            ],
        )
        .expect("active reviewer node");
    let (status, body) = request_json(app, Method::GET, lifecycle_uri, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["story_specs"][0]["review_status"], "running",
        "an active reviewer_run must project review_status=running: {body}"
    );
}
