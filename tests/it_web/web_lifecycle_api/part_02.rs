#[tokio::test]
async fn issue_lifecycle_backfills_legacy_spec_versions_and_returns_markdown_preview() {
    let root = tempdir().expect("root");
    let repo = git_repo();
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
    crate::create_repository_and_wait(
        app.clone(),
        "project_0001",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"爬楼梯问题","description":"写个 Python 程序解决爬楼梯","repository_id":"repository_0001"}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({"title":"爬楼梯问题 Story Spec"}),
    )
    .await;

    let story_path = root
        .path()
        .join(".aria/projects/project_0001/issues/issue_0001/story-specs/story_spec_0001.json");
    let mut story: Value =
        serde_json::from_str(&fs::read_to_string(&story_path).expect("story file"))
            .expect("story json");
    story["current_version"] = Value::Null;
    fs::write(
        &story_path,
        serde_json::to_string_pretty(&story).expect("story json text"),
    )
    .expect("write story");

    let markdown = "## 范围\n\n覆盖爬楼梯问题。\n\n## 功能需求\n\n[REQ-001] 使用 O(n) 时间复杂度。";
    request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/message",
        json!({"role":"provider","content":markdown}),
    )
    .await;

    let (status, lifecycle) = request_json(
        app.clone(),
        Method::GET,
        "/api/issues/issue_0001/lifecycle?project_id=project_0001",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(lifecycle["story_specs"][0]["current_version"], 1);
    assert!(
        lifecycle["story_specs"][0]["current_markdown_preview"]
            .as_str()
            .expect("markdown preview")
            .contains("[REQ-001] 使用 O(n) 时间复杂度")
    );

    let version_path = root.path().join(
        ".aria/projects/project_0001/issues/issue_0001/versions/story_spec_0001/version_0001.json",
    );
    let version: Value =
        serde_json::from_str(&fs::read_to_string(version_path).expect("version file"))
            .expect("version json");
    assert_eq!(version["markdown"], markdown);

    let (status, lifecycle_again) = request_json(
        app,
        Method::GET,
        "/api/issues/issue_0001/lifecycle?project_id=project_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(lifecycle_again["story_specs"][0]["current_version"], 1);
    let versions_root = root
        .path()
        .join(".aria/projects/project_0001/issues/issue_0001/versions/story_spec_0001");
    let version_count = fs::read_dir(versions_root).expect("versions root").count();
    assert_eq!(version_count, 1);
}

#[tokio::test]
async fn lifecycle_returns_artifact_versions() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
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
    crate::create_repository_and_wait(
        app.clone(),
        "project_0001",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"爬楼梯问题","description":"写个 Python 程序解决爬楼梯","repository_id":"repository_0001"}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({"title":"爬楼梯问题 Story Spec"}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/confirm",
        json!({"confirmed_by":"human"}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/design-specs:generate",
        json!({
            "title":"爬楼梯问题 Design Spec",
            "story_spec_ids":["story_spec_0001"]
        }),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0002/confirm",
        json!({"confirmed_by":"human"}),
    )
    .await;
    bootstrap_confirmed_work_item_session(root.path(), ProviderName::Fake, ProviderName::Fake)
        .await;

    let lifecycle = LifecycleStore::new(app_paths);
    lifecycle
        .append_artifact_version(
            "workspace_session_0001",
            ArtifactVersion {
                version: 1,
                payload: ArtifactPayload::Markdown {
                    markdown: "## 功能需求\n\n[REQ-001] 计算爬楼梯方案数。".to_string(),
                    diff: None,
                },
                generated_by: ProviderName::Fake,
                reviewed_by: Some(ProviderName::Fake),
                review_verdict: None,
                confirmed_by: Some("human".to_string()),
                is_current: true,
                created_at: "2026-05-20T00:00:00Z".to_string(),
                source_node_id: "timeline_node_story_001".to_string(),
            },
        )
        .expect("append story artifact version");
    lifecycle
        .append_artifact_version(
            "workspace_session_0002",
            ArtifactVersion {
                version: 1,
                payload: ArtifactPayload::Markdown {
                    markdown: "## 关键决策\n\n[DEC-001] 使用动态规划。".to_string(),
                    diff: None,
                },
                generated_by: ProviderName::Fake,
                reviewed_by: Some(ProviderName::Fake),
                review_verdict: None,
                confirmed_by: None,
                is_current: true,
                created_at: "2026-05-20T00:01:00Z".to_string(),
                source_node_id: "timeline_node_design_001".to_string(),
            },
        )
        .expect("append design artifact version");
    lifecycle
        .append_artifact_version(
            "workspace_session_0003",
            ArtifactVersion {
                version: 1,
                payload: ArtifactPayload::Markdown {
                    markdown: "## 实施计划\n\n[TASK-001] 实现 climb_stairs。".to_string(),
                    diff: None,
                },
                generated_by: ProviderName::Fake,
                reviewed_by: Some(ProviderName::Fake),
                review_verdict: None,
                confirmed_by: None,
                is_current: true,
                created_at: "2026-05-20T00:02:00Z".to_string(),
                source_node_id: "timeline_node_work_item_001".to_string(),
            },
        )
        .expect("append work item artifact version");

    let (status, response) = request_json(
        app,
        Method::GET,
        "/api/issues/issue_0001/lifecycle?project_id=project_0001",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let story_versions = response["story_specs"][0]["artifact_versions"]
        .as_array()
        .expect("story artifact_versions");
    assert_eq!(story_versions.len(), 1);
    assert_eq!(story_versions[0]["version"], 1);
    assert!(
        story_versions[0]["markdown"]
            .as_str()
            .expect("story markdown")
            .contains("功能需求")
    );

    let design_versions = response["design_specs"][0]["artifact_versions"]
        .as_array()
        .expect("design artifact_versions");
    assert_eq!(design_versions.len(), 1);
    assert_eq!(design_versions[0]["version"], 1);
    assert!(
        design_versions[0]["markdown"]
            .as_str()
            .expect("design markdown")
            .contains("关键决策")
    );

    let work_item_versions = response["work_items"][0]["artifact_versions"]
        .as_array()
        .expect("work item artifact_versions");
    assert_eq!(work_item_versions.len(), 1);
    assert_eq!(work_item_versions[0]["version"], 1);
    assert!(
        work_item_versions[0]["markdown"]
            .as_str()
            .expect("work item markdown")
            .contains("实施计划")
    );
}

#[tokio::test]
async fn workspace_session_missing_message_and_run_next_return_not_found() {
    let root = tempdir().expect("root");
    let repo = git_repo();
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
    crate::create_repository_and_wait(
        app.clone(),
        "project_0001",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"登录会话过期","description":"描述","repository_id":"repository_0001"}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({"title":"登录会话过期提示"}),
    )
    .await;

    let (status, message_error) = request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_missing/message",
        json!({"role":"user","content":"请强调重新登录按钮"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(message_error["code"], "workspace_session_not_found");

    let (status, run_next_error) = request_json(
        app,
        Method::POST,
        "/api/workspace-sessions/workspace_session_missing/run-next",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(run_next_error["code"], "workspace_session_not_found");
}

#[tokio::test]
async fn workspace_session_ambiguous_returns_conflict() {
    let root = tempdir().expect("root");
    let repo = git_repo();
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
    crate::create_repository_and_wait(
        app.clone(),
        "project_0001",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"登录会话过期","description":"描述","repository_id":"repository_0001"}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({"title":"登录会话过期提示"}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"重复会话","description":"描述","repository_id":"repository_0001"}),
    )
    .await;

    let first_session_path = root
        .path()
        .join(".aria/projects/project_0001/issues/issue_0001/workspace-sessions/workspace_session_0001.json");
    let duplicate_root = root
        .path()
        .join(".aria/projects/project_0001/issues/issue_0002/workspace-sessions");
    fs::create_dir_all(&duplicate_root).expect("duplicate workspace sessions root");
    fs::copy(
        first_session_path,
        duplicate_root.join("workspace_session_0001.json"),
    )
    .expect("duplicate workspace session");

    let (status, error) = request_json(
        app,
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/message",
        json!({"role":"user","content":"请强调重新登录按钮"}),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["code"], "workspace_session_ambiguous");
}

#[tokio::test]
async fn workspace_session_message_rejects_invalid_role_and_empty_content() {
    let root = tempdir().expect("root");
    let repo = git_repo();
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
    crate::create_repository_and_wait(
        app.clone(),
        "project_0001",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"登录会话过期","description":"描述","repository_id":"repository_0001"}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({"title":"登录会话过期提示"}),
    )
    .await;

    for body in [
        json!({"role":"","content":"请强调重新登录按钮"}),
        json!({"role":"unknown","content":"请强调重新登录按钮"}),
        json!({"role":"user","content":"   "}),
    ] {
        let (status, error) = request_json(
            app.clone(),
            Method::POST,
            "/api/workspace-sessions/workspace_session_0001/message",
            body,
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(error["code"], "invalid_workspace_message");
    }
}

async fn bootstrap_confirmed_work_item_session(
    root_path: &std::path::Path,
    author_provider: ProviderName,
    reviewer_provider: ProviderName,
) {
    let app_paths = ProductAppPaths::new(root_path.join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths);
    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some("work_item_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            design_spec_ids: vec!["design_spec_0001".to_string()],
            title: "实现会话过期提示".to_string(),
            plan_status: WorkItemPlanStatus::Confirmed,
            ..Default::default()
        })
        .expect("create work item");
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "work_item_0001".to_string(),
            workspace_type: WorkspaceType::WorkItem,
            author_provider,
            reviewer_provider,
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
        })
        .expect("create work item session");
    lifecycle
        .update_workspace_session_status(&session.id, WorkspaceSessionStatus::Confirmed)
        .expect("confirm work item session");
}

async fn request_json(
    app: axum::Router,
    method: Method,
    uri: &str,
    body: Value,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request");
    let response = app.oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

async fn delete_repository_with_idempotency_key(
    app: axum::Router,
    project_id: &str,
    repository_id: &str,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(Method::DELETE)
        .uri(format!("/api/projects/{project_id}/repositories/{repository_id}"))
        .header("content-type", "application/json")
        .header("Idempotency-Key", "test-delete-repo-lifecycle-0001")
        .body(Body::from("{}".to_string()))
        .expect("delete repository request");
    let response = app
        .oneshot(request)
        .await
        .expect("delete repository response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("delete repository body");
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

fn git_repo() -> tempfile::TempDir {
    let dir = tempdir().expect("repo");
    let status = Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .status()
        .expect("git init");
    assert!(status.success());
    dir
}

// ---- 方案 X 阶段1：聚合代码库（Logical）Web 接入回归测试（Task 5）----
// 多仓 issue 无 repo_id（manifest + selection 同时存在 → RepositoryRouting::Logical）：
// 1. issue_lifecycle 不要求 repo_id（Logical 分支不报 repository_required）；
// 2. generate_story_specs Logical 分支经 PlanningContextResolver + 草稿态 create + 注入
//    aggregate prompt（session context message 含 inventory + 聚合视野指令）。
use cadence_aria::product::logical_codebase::{
    aggregate_index::{
        AggregateIndexMemberSnapshot, AggregateIndexRecord, AggregateIndexStatus, AggregateIndexStore,
    },
    policy::AggregatePolicyArtifactStore,
    CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, IssueCodebaseSelection,
    IssueCodebaseSelectionStore, LogicalCodebaseManifest, LogicalCodebaseStore, LogicalRepositoryId,
    MemberStatus, RepositoryCheckoutId, RepositoryCheckoutRecord, RepositorySourceIdentity,
    RepositoryType,
};
use cadence_aria::product::issue_store::{CreateProductIssueInput, IssueStore};
use cadence_aria::product::lifecycle_store::CreateStorySpecInput;
use cadence_aria::product::models::LifecycleConfirmationStatus;

/// 多仓场景 fixture：写 manifest + active member + checkout + 显式 selection + active
/// aggregate index + policy bootstrap，使 `RepositoryRouting::load_for_issue` 判为 Logical。
///
/// PlanningContextResolver 会在读取 active index 前采集成员 Git evidence，因此这里的
/// main checkout 必须是带提交的真实 Git 仓，且已发布快照必须匹配该提交。
fn seed_logical_codebase(app_paths: &ProductAppPaths, member_id: LogicalRepositoryId) {
    let aggregate_root = app_paths.root().join("aggregate-root");
    std::fs::create_dir_all(&aggregate_root).expect("create aggregate-root fixture dir");
    let checkout_path = aggregate_root.join("api");
    std::fs::create_dir_all(&checkout_path).expect("create api checkout fixture dir");
    let init = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&checkout_path)
        .output()
        .expect("initialize api checkout git repository");
    assert!(
        init.status.success(),
        "git init fixture failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    std::fs::write(checkout_path.join("lib.rs"), "pub fn fixture() {}\n")
        .expect("write api checkout fixture source");
    let commit = Command::new("git")
        .args([
            "add",
            "lib.rs",
        ])
        .current_dir(&checkout_path)
        .output()
        .expect("stage api checkout fixture source");
    assert!(
        commit.status.success(),
        "git add fixture failed: {}",
        String::from_utf8_lossy(&commit.stderr)
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
        .expect("commit api checkout fixture source");
    assert!(
        commit.status.success(),
        "git commit fixture failed: {}",
        String::from_utf8_lossy(&commit.stderr)
    );
    let revision_output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&checkout_path)
        .output()
        .expect("read api checkout fixture revision");
    assert!(
        revision_output.status.success(),
        "git rev-parse fixture failed: {}",
        String::from_utf8_lossy(&revision_output.stderr)
    );
    let revision = String::from_utf8(revision_output.stdout)
        .expect("git revision is UTF-8")
        .trim()
        .to_string();
    let manifest = LogicalCodebaseManifest::new("project_0001", aggregate_root.clone(), vec![member_id]);
    LogicalCodebaseStore::new(app_paths.clone())
        .save_manifest("project_0001", &manifest)
        .unwrap();
    let now = "2026-08-10T00:00:00Z".to_string();
    LogicalCodebaseStore::new(app_paths.clone())
        .save_member(
            "project_0001",
            &CodebaseMemberRecord {
                logical_repository_id: member_id,
                physical_repository_id: "repository_api".to_string(),
                alias: "api".to_string(),
                role: "service".to_string(),
                ordinal: 1,
                source_identity: RepositorySourceIdentity::from_git_parts(
                    &checkout_path,
                    checkout_path.join(".git"),
                    Some("ssh://git@example.test/acme/api.git".to_string()),
                ),
                repo_type: RepositoryType::Backend,
                tech_stack: vec!["rust".to_string()],
                owner: None,
                tags: Vec::new(),
                default_ref: None,
                checkout_ids: vec![RepositoryCheckoutId(uuid::Uuid::nil())],
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
                checkout_id: RepositoryCheckoutId(uuid::Uuid::nil()),
                logical_repository_id: member_id,
                physical_repository_id: "repository_api".to_string(),
                kind: CheckoutKind::Main,
                canonical_path: checkout_path,
                checkout_path_hash: "sha256:checkout".to_string(),
                git_dir_identity: "sha256:git-dir".to_string(),
                revision: Some(revision.clone()),
                availability: CheckoutAvailability::Available,
                observed_at: now.clone(),
                created_at: now.clone(),
                updated_at: now.clone(),
            },
        )
        .unwrap();
    IssueCodebaseSelectionStore::new(app_paths.clone())
        .save(&IssueCodebaseSelection::explicit(
            "project_0001",
            "issue_0001",
            vec![member_id],
            Vec::new(),
            Vec::new(),
            None,
        ))
        .unwrap();
    let index = AggregateIndexRecord::building(
        "aggregate_index_0001".to_string(),
        "project_0001".to_string(),
        manifest.membership_revision,
        vec![AggregateIndexMemberSnapshot::indexed(
            member_id,
            RepositoryCheckoutId(uuid::Uuid::nil()),
            revision,
            false,
            now.clone(),
        )],
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
    assert!(content.contains("聚合代码库成员清单"), "缺 inventory：{content}");
    assert!(
        content.contains("00000000-0000-0000-0000-000000000001"),
        "缺成员行：{content}"
    );
    assert!(content.contains("involved_repository_ids"), "缺聚合指令：{content}");
    assert!(
        content.contains("禁止回落到任意单一 primary 仓库"),
        "缺禁止 primary 回落指令：{content}"
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
        design_response["design_specs"][0]["confirmation_status"], "draft"
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
    assert!(content.contains("聚合代码库成员清单"), "缺 inventory：{content}");
    assert!(
        content.contains("00000000-0000-0000-0000-000000000001"),
        "缺成员行：{content}"
    );
    assert!(content.contains("involved_repository_ids"), "缺聚合指令：{content}");
    assert!(
        content.contains("禁止回落到任意单一 primary 仓库"),
        "缺禁止 primary 回落指令：{content}"
    );
    assert!(content.contains("change_order"), "缺 change_order 指令：{content}");
    assert!(content.contains("depends_on"), "缺 depends_on 依据：{content}");
}

// ---- Task 6：confirm gate（多仓 involved + change_order 校验，3b 收紧）----
// confirm_workspace_entity 在 Confirmed 前对多仓（logical_codebase_ref Some）Spec 校验：
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
            reviewer_provider: ProviderName::Codex,
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
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
    let (_root, app, session_id) = create_logical_confirm_fixture(
        WorkspaceType::Design,
        vec![m1, m2],
        vec![m1, m2],
    )
    .await;
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
            reviewer_provider: ProviderName::Codex,
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
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
