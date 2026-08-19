// 场景 F（REQ-PROMPT-01/02 分流）：逻辑代码库上下文构建 work-item draft prompt
// 时注入 authority_root/policy_revision/policy_digest，声明指针块不作为政策正文、
// 未加载聚合政策时只报告阻塞；Legacy 单仓上下文保留原文且无 authority_root。
//
// 可达路径说明（T3/T4 接线核实）：work-item draft prompt 的可达 web 路径是
// `story-specs:generate` → `build_workspace_context_message` →
// `workflow_discipline_for(session, routing_context)`，其中 routing_context 由
// `web/workspace_context/builder.rs::routing_reference_context_for_project` 从
// manifest + `AggregatePolicyArtifact` 派生（Logical）或退回 Legacy。T2 coding
// prompt 的 Logical 分流（`coding_workspace_engine` 经
// `routing_reference_context_from_policy`）需要完整 gateway
// `ValidatedSessionLaunchPolicy` 且已由 `provider_context_builder.rs` /
// `coding_workspace_engine/tests` 单测覆盖；本 it_web 测试选择 T3/T4 可达路径。

use cadence_aria::product::issue_store::{CreateProductIssueInput, IssueStore};
use cadence_aria::product::logical_codebase::aggregate_index::{
    AggregateIndexMemberSnapshot, AggregateIndexRecord, AggregateIndexStatus, AggregateIndexStore,
};
use cadence_aria::product::logical_codebase::policy::AggregatePolicyArtifactStore;

/// 单成员逻辑代码库 prompt fixture：manifest + member + checkout + selection + active
/// aggregate index + policy bootstrap。Planning 读取会做 freshness assess，所以成员 main
/// checkout 必须是带提交的真实 Git 仓，已发布的 index evidence 必须对应该提交。
fn seed_logical_codebase_prompt(app_paths: &ProductAppPaths, member_id: LogicalRepositoryId) {
    let aggregate_root = app_paths.root().join("aggregate-root");
    std::fs::create_dir_all(&aggregate_root).unwrap();
    let checkout_path = aggregate_root.join("api");
    std::fs::create_dir_all(&checkout_path).expect("create api checkout fixture dir");
    git(&checkout_path, &["init", "-q"]);
    std::fs::write(checkout_path.join("lib.rs"), "pub fn fixture() {}\n")
        .expect("write api checkout fixture source");
    git(&checkout_path, &["add", "lib.rs"]);
    git(
        &checkout_path,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.test",
            "commit",
            "-qm",
            "initial fixture",
        ],
    );
    let revision = git_out(&checkout_path, &["rev-parse", "HEAD"]);
    let manifest =
        LogicalCodebaseManifest::new(PROJECT_ID, aggregate_root.clone(), vec![member_id]);
    LogicalCodebaseStore::new(app_paths.clone())
        .save_manifest(PROJECT_ID, &manifest)
        .unwrap();
    let now = "2026-08-10T00:00:00Z".to_string();
    LogicalCodebaseStore::new(app_paths.clone())
        .save_member(
            PROJECT_ID,
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
            PROJECT_ID,
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
            PROJECT_ID,
            "issue_0001",
            vec![member_id],
            Vec::new(),
            Vec::new(),
            None,
        ))
        .unwrap();
    let index = AggregateIndexRecord::building(
        "aggregate_index_0001".to_string(),
        PROJECT_ID.to_string(),
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
        .create(PROJECT_ID, index.clone())
        .unwrap();
    let mut activated = index;
    activated.status = AggregateIndexStatus::Active;
    AggregateIndexStore::new(app_paths.clone())
        .replace_active(PROJECT_ID, activated)
        .unwrap();
    AggregatePolicyArtifactStore::new(app_paths.clone())
        .ensure_bootstrap(&manifest)
        .unwrap();
}

fn find_generation_context_message(response: &Value) -> String {
    response["workspace_session"]["messages"]
        .as_array()
        .expect("workspace session messages")
        .iter()
        .find(|message| {
            message["content"]
                .as_str()
                .is_some_and(|content| content.contains("Workspace 生成任务已准备"))
        })
        .expect("generation context message")["content"]
        .as_str()
        .expect("context content")
        .to_string()
}

#[tokio::test]
async fn pointer_publication_scenario_f_logical_context_injects_authority_reference() {
    let root = tempdir().expect("root");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    // 先建多仓 issue（repo_id=None，成为 issue_0001），再写 selection。
    IssueStore::new(app_paths.clone())
        .create(CreateProductIssueInput {
            project_id: PROJECT_ID.to_string(),
            repo_id: None,
            logical_codebase_id: None,
            title: "多仓聚合 Story".to_string(),
            description: Some("跨 api 仓库的聚合变更".to_string()),
            change_id: None,
        })
        .expect("logical issue");
    let member_id = LogicalRepositoryId(uuid::Uuid::from_u128(1));
    seed_logical_codebase_prompt(&app_paths, member_id);

    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    let (status, story_response) = request_json(
        app,
        Method::POST,
        &format!("/api/projects/{PROJECT_ID}/issues/issue_0001/story-specs:generate"),
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
    assert_eq!(status, StatusCode::OK, "logical story generate: {story_response}");

    let content = find_generation_context_message(&story_response);
    let authority_root = std::fs::canonicalize(app_paths.root().join("aggregate-root"))
        .expect("canonicalize aggregate-root")
        .to_string_lossy()
        .to_string();
    assert!(
        content.contains(&format!("authority_root: {authority_root}")),
        "logical prompt must inject authority_root: {content}"
    );
    assert!(
        content.contains("policy_id: "),
        "logical prompt must inject policy_id: {content}"
    );
    assert!(
        content.contains("policy_revision: "),
        "logical prompt must inject policy_revision: {content}"
    );
    assert!(
        content.contains("policy_digest: "),
        "logical prompt must inject policy_digest: {content}"
    );
    assert!(
        content.contains("不作为政策正文"),
        "logical prompt must declare pointer block is not policy body: {content}"
    );
    assert!(
        content.contains("只报告阻塞"),
        "logical prompt must fail closed (report blocking only): {content}"
    );
    assert!(
        !content.contains("当前目标仓库根目录的 AGENTS.md 与 CLAUDE.md 是本任务的流程规则依据"),
        "logical prompt must not carry legacy original routing reference: {content}"
    );
}

#[tokio::test]
async fn pointer_publication_scenario_f_legacy_context_keeps_original_reference() {
    let root = tempdir().expect("root");
    let repo = {
        let dir = tempdir().expect("repo");
        let status = Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .status()
            .expect("git init");
        assert!(status.success());
        dir
    };
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Prompt","description":null}),
    )
    .await;
    crate::create_repository_and_wait(
        app.clone(),
        PROJECT_ID,
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        &format!("/api/projects/{PROJECT_ID}/issues"),
        json!({"title":"登录会话过期","description":"描述","repository_id":"repository_0001"}),
    )
    .await;

    let (status, story_response) = request_json(
        app,
        Method::POST,
        &format!("/api/projects/{PROJECT_ID}/issues/issue_0001/story-specs:generate"),
        json!({
            "title":"登录会话过期提示",
            "author_provider":"fake",
            "reviewer_provider":"codex",
            "review_rounds":3,
            "superpowers_enabled":false,
            "openspec_enabled":true
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "legacy story generate: {story_response}");

    let content = find_generation_context_message(&story_response);
    assert!(
        content.contains("[cadence_project_rules]"),
        "legacy prompt must carry original routing reference: {content}"
    );
    assert!(
        content.contains("当前目标仓库根目录的 AGENTS.md 与 CLAUDE.md"),
        "legacy prompt must reference target repo rule files: {content}"
    );
    assert!(
        !content.contains("authority_root:"),
        "legacy prompt must not inject authority_root: {content}"
    );
}
