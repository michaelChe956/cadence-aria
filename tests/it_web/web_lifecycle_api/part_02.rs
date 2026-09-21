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
            work_item_plan_options: None,
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
        .uri(format!(
            "/api/projects/{project_id}/repositories/{repository_id}"
        ))
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
use cadence_aria::product::issue_store::{CreateProductIssueInput, IssueStore};
use cadence_aria::product::lifecycle_store::CreateStorySpecInput;
use cadence_aria::product::logical_codebase::{
    CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, IssueCodebaseSelection,
    IssueCodebaseSelectionStore, LogicalCodebaseManifest, LogicalCodebaseStore,
    LogicalRepositoryId, MemberStatus, RepositoryCheckoutId, RepositoryCheckoutRecord,
    RepositorySourceIdentity, RepositoryType,
    aggregate_index::{
        AggregateIndexMemberSnapshot, AggregateIndexRecord, AggregateIndexStatus,
        AggregateIndexStore,
    },
    policy::AggregatePolicyArtifactStore,
};
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
        .args(["add", "lib.rs"])
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
    let manifest =
        LogicalCodebaseManifest::new("project_0001", aggregate_root.clone(), vec![member_id]);
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
