//! v1.3 R5：issue 唯一归属代码库（单仓/逻辑），routing/resolver 按 lc_id 子树解析。
//!
//! 这里建立**两个真实的逻辑代码库**（各自独立 `logical-codebases/{lc_id}/` 子树、
//! 真实 Git member checkout、active index、policy），经真实 HTTP 创建逻辑 issue，
//! 断言：lc_id 归属持久化、primary 校验、selection 按 lc_id 键落盘、Story 可达、
//! 以及混合 project 下两 codebase 的 issue 互不串扰。

use std::fs;
use std::path::PathBuf;

use axum::http::{Method, StatusCode};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::issue_store::IssueStore;
use cadence_aria::product::logical_codebase::{
    CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, IssueCodebaseSelection,
    IssueCodebaseSelectionStore, LogicalCodebaseCreateInput, LogicalCodebaseManifest,
    LogicalCodebaseStore, LogicalRepositoryId, MemberStatus, ProviderGatewayError,
    RepositoryCheckoutId, RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType,
    SelectionPolicy,
    aggregate_index::{
        AggregateIndexMemberSnapshot, AggregateIndexRecord, AggregateIndexStatus,
        AggregateIndexStore,
    },
    policy::AggregatePolicyArtifactStore,
    resolve_issue_logical_codebase_id,
};
use cadence_aria::product::project_store::{CreateProjectInput, ProjectStore};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::gateway_factory::LogicalCodebaseGatewayFactory;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use serde_json::json;
use tempfile::TempDir;
use uuid::Uuid;

use crate::planning::{git, git_stdout};
use crate::{assert_error, request};

const PROJECT_ID: &str = "project_0001";

struct IssueAttributionFixture {
    _workspace: TempDir,
    paths: ProductAppPaths,
    lc_a: String,
    lc_b: String,
    app: axum::Router,
    factory: std::sync::Arc<LogicalCodebaseGatewayFactory>,
}

fn seed_logical_codebase(paths: &ProductAppPaths, name: &str, aggregate_root: PathBuf) -> String {
    // 创建真实逻辑代码库记录 + 空子树（POST /logical-codebases 的 store 层语义）。
    let record = LogicalCodebaseStore::new(paths.clone())
        .create(
            PROJECT_ID,
            LogicalCodebaseCreateInput {
                name: name.to_string(),
                aggregate_root: aggregate_root.clone(),
            },
        )
        .expect("create logical codebase record");
    let lc_id = record.id;
    let logical = LogicalCodebaseStore::for_lc(paths.clone(), lc_id.clone());

    // 真实 Git member checkout：resolver 的 freshness 会跑真实 git 证据采集。
    let member_root = aggregate_root.join("svc");
    fs::create_dir_all(&member_root).expect("create member checkout");
    git(&member_root, &["init", "-q"]);
    fs::write(member_root.join("lib.rs"), "pub fn svc() {}\n").expect("write member source");
    git(&member_root, &["add", "lib.rs"]);
    git(
        &member_root,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-qm",
            "initial member source",
        ],
    );

    let member_id = LogicalRepositoryId(Uuid::new_v4());
    let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
    let manifest =
        LogicalCodebaseManifest::new(PROJECT_ID, aggregate_root.clone(), vec![member_id]);
    logical
        .save_manifest(PROJECT_ID, &manifest)
        .expect("save lc manifest");
    let now = "2026-08-19T00:00:00Z".to_string();
    logical
        .save_member(
            PROJECT_ID,
            &CodebaseMemberRecord {
                logical_repository_id: member_id,
                physical_repository_id: format!("repository_{name}"),
                alias: name.to_string(),
                role: "service".to_string(),
                ordinal: 1,
                source_identity: RepositorySourceIdentity::from_git_parts(
                    &member_root,
                    member_root.join(".git"),
                    None,
                ),
                repo_type: RepositoryType::Backend,
                tech_stack: vec!["rust".to_string()],
                owner: None,
                tags: Vec::new(),
                default_ref: Some("main".to_string()),
                checkout_ids: vec![checkout_id],
                status: MemberStatus::Active,
                created_at: now.clone(),
                updated_at: now.clone(),
            },
        )
        .expect("save lc member");
    logical
        .save_checkout(
            PROJECT_ID,
            &RepositoryCheckoutRecord {
                checkout_id,
                logical_repository_id: member_id,
                physical_repository_id: format!("repository_{name}"),
                kind: CheckoutKind::Main,
                canonical_path: member_root.clone(),
                checkout_path_hash: format!("sha256:checkout-{name}"),
                git_dir_identity: format!("sha256:git-dir-{name}"),
                revision: Some(git_stdout(&member_root, &["rev-parse", "HEAD"])),
                availability: CheckoutAvailability::Available,
                observed_at: now.clone(),
                created_at: now.clone(),
                updated_at: now.clone(),
            },
        )
        .expect("save lc checkout");

    // active index（fresh：与 manifest 修订号、git HEAD 一致，不触发 CodeGraph 同步）。
    let snapshots = vec![AggregateIndexMemberSnapshot::indexed(
        member_id,
        checkout_id,
        git_stdout(&member_root, &["rev-parse", "HEAD"]),
        false,
        now.clone(),
    )];
    let mut active = AggregateIndexRecord::building(
        format!("index_{name}"),
        PROJECT_ID.to_string(),
        manifest.membership_revision,
        snapshots,
        now,
    );
    active.status = AggregateIndexStatus::Active;
    active.codegraph_root = aggregate_root.clone();
    active.config_digest = "fixture".to_string();
    AggregateIndexStore::for_lc(paths.clone(), lc_id.clone())
        .create(PROJECT_ID, active)
        .expect("publish lc active index");
    AggregatePolicyArtifactStore::for_lc(paths.clone(), lc_id.clone())
        .ensure_bootstrap(&manifest)
        .expect("bootstrap lc aggregate policy");

    lc_id
}

impl IssueAttributionFixture {
    fn new() -> Self {
        let workspace = tempfile::tempdir().expect("workspace");
        let root = workspace.path().to_path_buf();
        let paths = ProductAppPaths::new(root.join(".aria"));
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "codebase kinds".to_string(),
                description: None,
            })
            .expect("create project");
        let lc_a = seed_logical_codebase(&paths, "svc-a", root.join("aggregate-a"));
        let lc_b = seed_logical_codebase(&paths, "svc-b", root.join("aggregate-b"));
        let state = WebAppState::new(root.clone(), WebRuntime::new_fake(root.clone()));
        let factory = state
            .gateway_factory()
            .expect("gateway factory installed by default")
            .clone();
        Self {
            _workspace: workspace,
            paths,
            lc_a,
            lc_b,
            app: build_web_router(state),
            factory,
        }
    }

    async fn create_issue(
        &self,
        logical_codebase_id: &str,
        repository_id: &str,
    ) -> (StatusCode, serde_json::Value) {
        request(
            &self.app,
            Method::POST,
            &format!("/api/projects/{PROJECT_ID}/issues"),
            json!({
                "repository_id": repository_id,
                "logical_codebase_id": logical_codebase_id,
                "title": "logical codebase issue",
                "description": "attributed to exactly one codebase"
            }),
        )
        .await
    }
}

#[tokio::test]
async fn logical_issue_persists_lc_attribution_selection_and_reaches_story() {
    let fixture = IssueAttributionFixture::new();

    let (status, issue) = fixture
        .create_issue(&fixture.lc_a, "repository_svc-a")
        .await;
    assert_eq!(status, StatusCode::OK, "create logical issue: {issue}");
    let issue_id = issue["issue_id"].as_str().expect("issue id").to_string();

    // 归属持久化到 issue 记录。
    let stored = IssueStore::new(fixture.paths.clone())
        .get(PROJECT_ID, &issue_id)
        .expect("load issue");
    assert_eq!(
        stored.logical_codebase_id.as_deref(),
        Some(fixture.lc_a.as_str())
    );

    // selection 以 lc_id 键落盘在 LC 子树，且记录自述 lc_id 归属。
    let selection =
        IssueCodebaseSelectionStore::for_lc(fixture.paths.clone(), fixture.lc_a.clone())
            .load(PROJECT_ID, &issue_id)
            .expect("load lc selection")
            .expect("selection persisted");
    assert_eq!(
        selection.selection_policy,
        cadence_aria::product::logical_codebase::SelectionPolicy::AllMembers
    );
    assert_eq!(
        selection.logical_codebase_id.as_deref(),
        Some(fixture.lc_a.as_str())
    );
    let lc_selection_path =
        fixture
            .paths
            .lc_codebase_selection_path(PROJECT_ID, &fixture.lc_a, &issue_id);
    assert!(
        lc_selection_path.is_file(),
        "selection keyed by lc_id: {lc_selection_path:?}"
    );

    // Story 可达：routing/resolver 从 lc_id 子树解析 manifest/index/policy。
    let (story_status, story) = request(
        &fixture.app,
        Method::POST,
        &format!("/api/projects/{PROJECT_ID}/issues/{issue_id}/story-specs:generate"),
        json!({
            "title": "logical codebase Story",
            "author_provider": "fake",
            "reviewer_provider": "codex",
            "review_rounds": 1,
            "superpowers_enabled": false,
            "openspec_enabled": false
        }),
    )
    .await;
    assert_eq!(story_status, StatusCode::OK, "Story reachable: {story}");
}

#[tokio::test]
async fn logical_issue_rejects_repository_outside_lc_active_members() {
    let fixture = IssueAttributionFixture::new();

    // repository_svc-b 属于另一逻辑代码库，不能作为 svc-a 的 primary。
    assert_error(
        fixture
            .create_issue(&fixture.lc_a, "repository_svc-b")
            .await,
        StatusCode::NOT_FOUND,
        "repository_not_found",
    );
    // 未知 repository 同样 404。
    assert_error(
        fixture
            .create_issue(&fixture.lc_a, "repository_unknown")
            .await,
        StatusCode::NOT_FOUND,
        "repository_not_found",
    );
    // 未知 lc_id → 404 logical_codebase_not_found。
    assert_error(
        fixture
            .create_issue("logical_codebase_missing", "repository_svc-a")
            .await,
        StatusCode::NOT_FOUND,
        "logical_codebase_not_found",
    );
}

#[tokio::test]
async fn mixed_project_two_codebases_issues_do_not_cross_interfere() {
    let fixture = IssueAttributionFixture::new();

    let (status_a, issue_a) = fixture
        .create_issue(&fixture.lc_a, "repository_svc-a")
        .await;
    assert_eq!(status_a, StatusCode::OK, "create issue in LC A: {issue_a}");
    let issue_a_id = issue_a["issue_id"]
        .as_str()
        .expect("issue A id")
        .to_string();

    let (status_b, issue_b) = fixture
        .create_issue(&fixture.lc_b, "repository_svc-b")
        .await;
    assert_eq!(status_b, StatusCode::OK, "create issue in LC B: {issue_b}");
    let issue_b_id = issue_b["issue_id"]
        .as_str()
        .expect("issue B id")
        .to_string();

    // 各 issue 的 selection 归属各自的 LC 子树，互不重叠。
    let selection_a =
        IssueCodebaseSelectionStore::for_lc(fixture.paths.clone(), fixture.lc_a.clone())
            .load(PROJECT_ID, &issue_a_id)
            .expect("load LC A selection")
            .expect("LC A selection");
    let selection_b =
        IssueCodebaseSelectionStore::for_lc(fixture.paths.clone(), fixture.lc_b.clone())
            .load(PROJECT_ID, &issue_b_id)
            .expect("load LC B selection")
            .expect("LC B selection");
    assert_eq!(
        selection_a.logical_codebase_id.as_deref(),
        Some(fixture.lc_a.as_str())
    );
    assert_eq!(
        selection_b.logical_codebase_id.as_deref(),
        Some(fixture.lc_b.as_str())
    );

    // Story 各自可达，且 inventory 只含本 LC 成员（不串扰）。
    let (_, story_a) = request(
        &fixture.app,
        Method::POST,
        &format!("/api/projects/{PROJECT_ID}/issues/{issue_a_id}/story-specs:generate"),
        json!({
            "title": "LC A Story",
            "author_provider": "fake",
            "reviewer_provider": "codex",
            "review_rounds": 1,
            "superpowers_enabled": false,
            "openspec_enabled": false
        }),
    )
    .await;
    let (_, story_b) = request(
        &fixture.app,
        Method::POST,
        &format!("/api/projects/{PROJECT_ID}/issues/{issue_b_id}/story-specs:generate"),
        json!({
            "title": "LC B Story",
            "author_provider": "fake",
            "reviewer_provider": "codex",
            "review_rounds": 1,
            "superpowers_enabled": false,
            "openspec_enabled": false
        }),
    )
    .await;

    let context_a = story_a["workspace_session"]["messages"]
        .as_array()
        .and_then(|messages| {
            messages.iter().find(|message| {
                message["role"] == "system"
                    && message["content"]
                        .as_str()
                        .is_some_and(|content| content.contains("Workspace 生成任务已准备"))
            })
        })
        .expect("LC A context message");
    let context_b = story_b["workspace_session"]["messages"]
        .as_array()
        .and_then(|messages| {
            messages.iter().find(|message| {
                message["role"] == "system"
                    && message["content"]
                        .as_str()
                        .is_some_and(|content| content.contains("Workspace 生成任务已准备"))
            })
        })
        .expect("LC B context message");

    let content_a = context_a["content"].as_str().expect("LC A inventory");
    let content_b = context_b["content"].as_str().expect("LC B inventory");
    assert!(content_a.contains("svc-a"), "LC A inventory: {content_a}");
    assert!(content_b.contains("svc-b"), "LC B inventory: {content_b}");
    assert!(
        !content_a.contains("svc-b"),
        "cross-LC leak in A: {content_a}"
    );
    assert!(
        !content_b.contains("svc-a"),
        "cross-LC leak in B: {content_b}"
    );
}

#[tokio::test]
async fn non_default_lc_issue_gateway_resolves_lc_scoped_policy() {
    let fixture = IssueAttributionFixture::new();

    // 非默认 LC（lc_b）issue 的归属反查必须得到该 lc_id（WS 会话 gateway 的输入）。
    let (status, issue) = fixture
        .create_issue(&fixture.lc_b, "repository_svc-b")
        .await;
    assert_eq!(status, StatusCode::OK, "create issue in LC B: {issue}");
    let issue_id = issue["issue_id"].as_str().expect("issue id").to_string();
    let resolved = resolve_issue_logical_codebase_id(&fixture.paths, PROJECT_ID, &issue_id)
        .expect("resolve issue lc")
        .expect("logical issue must resolve to an lc");
    assert_eq!(resolved, fixture.lc_b);

    // gateway 按该 lc_id 构建成功（policy/capability 解析到 lc_b 子树）；
    // project 级默认首 LC（legacy None）路径无 manifest → PolicyMissing，证明不串扰。
    let gateway = fixture
        .factory
        .build_for_lc(PROJECT_ID, Some(&fixture.lc_b))
        .expect("gateway for non-default lc");
    drop(gateway);

    let legacy = fixture.factory.build_for_lc(PROJECT_ID, None);
    assert!(matches!(
        legacy,
        Err(ProviderGatewayError::PolicyMissing(ref id)) if id == PROJECT_ID
    ));
}
// ---- add-multi-repo-issue-entry 组 3（REQ-MRE-01）：创建链 explicit focus selection ----

/// 4 成员逻辑代码库 fixture（稳定 UUID；复选/上界语义按 change design 关键点 0）。
struct FocusSelectionFixture {
    _workspace: TempDir,
    paths: ProductAppPaths,
    lc_id: String,
    member_ids: Vec<LogicalRepositoryId>,
    app: axum::Router,
}

impl FocusSelectionFixture {
    fn new() -> Self {
        let workspace = tempfile::tempdir().expect("workspace");
        let root = workspace.path().to_path_buf();
        let paths = ProductAppPaths::new(root.join(".aria"));
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "focus selection".to_string(),
                description: None,
            })
            .expect("create project");

        let aggregate_root = root.join("aggregate-focus");
        let aliases = ["api", "web", "job", "shared"];
        let member_ids: Vec<LogicalRepositoryId> = aliases
            .iter()
            .enumerate()
            .map(|(index, _)| LogicalRepositoryId(Uuid::from_u128(0xf0cu128 + index as u128)))
            .collect();

        let record = LogicalCodebaseStore::new(paths.clone())
            .create(
                PROJECT_ID,
                LogicalCodebaseCreateInput {
                    name: "focus-lc".to_string(),
                    aggregate_root: aggregate_root.clone(),
                },
            )
            .expect("create logical codebase record");
        let lc_id = record.id;
        let logical = LogicalCodebaseStore::for_lc(paths.clone(), lc_id.clone());
        let manifest =
            LogicalCodebaseManifest::new(PROJECT_ID, aggregate_root.clone(), member_ids.clone());
        logical
            .save_manifest(PROJECT_ID, &manifest)
            .expect("save lc manifest");

        let now = "2026-10-10T00:00:00Z".to_string();
        let mut snapshots = Vec::new();
        for ((alias, member_id), ordinal) in aliases.iter().zip(&member_ids).zip(1..) {
            let member_root = aggregate_root.join(alias);
            fs::create_dir_all(&member_root).expect("create member checkout");
            git(&member_root, &["init", "-q"]);
            fs::write(member_root.join("lib.rs"), "pub fn member() {}\n")
                .expect("write member source");
            git(&member_root, &["add", "lib.rs"]);
            git(
                &member_root,
                &[
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=test@example.com",
                    "commit",
                    "-qm",
                    "initial member source",
                ],
            );
            let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
            logical
                .save_member(
                    PROJECT_ID,
                    &CodebaseMemberRecord {
                        logical_repository_id: *member_id,
                        physical_repository_id: format!("repository_{alias}"),
                        alias: alias.to_string(),
                        role: "service".to_string(),
                        ordinal,
                        source_identity: RepositorySourceIdentity::from_git_parts(
                            &member_root,
                            member_root.join(".git"),
                            None,
                        ),
                        repo_type: RepositoryType::Backend,
                        tech_stack: vec!["rust".to_string()],
                        owner: None,
                        tags: Vec::new(),
                        default_ref: Some("main".to_string()),
                        checkout_ids: vec![checkout_id],
                        status: MemberStatus::Active,
                        created_at: now.clone(),
                        updated_at: now.clone(),
                    },
                )
                .expect("save lc member");
            logical
                .save_checkout(
                    PROJECT_ID,
                    &RepositoryCheckoutRecord {
                        checkout_id,
                        logical_repository_id: *member_id,
                        physical_repository_id: format!("repository_{alias}"),
                        kind: CheckoutKind::Main,
                        canonical_path: member_root.clone(),
                        checkout_path_hash: format!("sha256:checkout-{alias}"),
                        git_dir_identity: format!("sha256:git-dir-{alias}"),
                        revision: Some(git_stdout(&member_root, &["rev-parse", "HEAD"])),
                        availability: CheckoutAvailability::Available,
                        observed_at: now.clone(),
                        created_at: now.clone(),
                        updated_at: now.clone(),
                    },
                )
                .expect("save lc checkout");
            snapshots.push(AggregateIndexMemberSnapshot::indexed(
                *member_id,
                checkout_id,
                git_stdout(&member_root, &["rev-parse", "HEAD"]),
                false,
                now.clone(),
            ));
        }
        let mut active = AggregateIndexRecord::building(
            "index_focus".to_string(),
            PROJECT_ID.to_string(),
            manifest.membership_revision,
            snapshots,
            now,
        );
        active.status = AggregateIndexStatus::Active;
        active.codegraph_root = aggregate_root;
        active.config_digest = "fixture".to_string();
        AggregateIndexStore::for_lc(paths.clone(), lc_id.clone())
            .create(PROJECT_ID, active)
            .expect("publish lc active index");
        AggregatePolicyArtifactStore::for_lc(paths.clone(), lc_id.clone())
            .ensure_bootstrap(&manifest)
            .expect("bootstrap lc aggregate policy");

        let state = WebAppState::new(root.clone(), WebRuntime::new_fake(root.clone()));
        Self {
            _workspace: workspace,
            paths,
            lc_id,
            member_ids,
            app: build_web_router(state),
        }
    }

    async fn create_issue_with_focus(
        &self,
        focus: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        let mut body = json!({
            "repository_id": "repository_api",
            "logical_codebase_id": self.lc_id,
            "title": "multi repo issue",
            "description": "explicit focus selection",
        });
        if let Some(focus) = focus {
            body["focus_repository_ids"] = focus;
        }
        request(
            &self.app,
            Method::POST,
            &format!("/api/projects/{PROJECT_ID}/issues"),
            body,
        )
        .await
    }

    fn load_selection(&self, issue_id: &str) -> IssueCodebaseSelection {
        IssueCodebaseSelectionStore::for_lc(self.paths.clone(), self.lc_id.clone())
            .load(PROJECT_ID, issue_id)
            .expect("load selection")
            .expect("selection persisted")
    }

    fn issue_count(&self) -> usize {
        IssueStore::new(self.paths.clone())
            .list(PROJECT_ID)
            .expect("list issues")
            .len()
    }

    fn member_id_strings(&self, indexes: &[usize]) -> Vec<String> {
        indexes
            .iter()
            .map(|index| self.member_ids[*index].0.to_string())
            .collect()
    }
}

#[tokio::test]
async fn logical_issue_with_full_focus_persists_explicit_selection() {
    let fixture = FocusSelectionFixture::new();
    let focus = json!(fixture.member_id_strings(&[0, 1, 2, 3]));

    let (status, issue) = fixture.create_issue_with_focus(Some(focus)).await;
    assert_eq!(status, StatusCode::OK, "create with focus: {issue}");
    let issue_id = issue["issue_id"].as_str().expect("issue id").to_string();

    let selection = fixture.load_selection(&issue_id);
    assert_eq!(selection.selection_policy, SelectionPolicy::Explicit);
    assert_eq!(
        selection.logical_codebase_id.as_deref(),
        Some(fixture.lc_id.as_str())
    );
    // 勾选集 = focus = include（REQ-MRE-01 场景：4 成员勾选落 focus，
    // durable 校验 focus⊆include 通过）。
    assert_eq!(selection.focus_repository_ids, fixture.member_ids.clone());
    assert_eq!(
        selection.included_repository_ids,
        fixture.member_ids.clone()
    );
    assert!(selection.excluded_repository_ids.is_empty());
    selection
        .validate_for_save()
        .expect("durable focus⊆include validation passes");
    assert_eq!(
        selection.resolve_effective_members(),
        fixture.member_ids.clone()
    );
}

#[tokio::test]
async fn logical_issue_with_partial_focus_persists_checked_subset() {
    let fixture = FocusSelectionFixture::new();
    // 勾 2/4：上界=勾选集原样（子集授权），include 同步=勾选集。
    let focus = json!(fixture.member_id_strings(&[1, 3]));

    let (status, issue) = fixture.create_issue_with_focus(Some(focus)).await;
    assert_eq!(status, StatusCode::OK, "create with partial focus: {issue}");
    let issue_id = issue["issue_id"].as_str().expect("issue id").to_string();

    let selection = fixture.load_selection(&issue_id);
    assert_eq!(selection.selection_policy, SelectionPolicy::Explicit);
    let expected = vec![fixture.member_ids[1], fixture.member_ids[3]];
    assert_eq!(selection.focus_repository_ids, expected);
    assert_eq!(selection.included_repository_ids, expected);
    assert_eq!(selection.resolve_effective_members(), expected);
}

#[tokio::test]
async fn logical_issue_with_single_focus_is_single_select_equivalent() {
    let fixture = FocusSelectionFixture::new();
    // 勾 1 等价单选（REQ-MRE-01）：explicit selection focus=include=[该成员]。
    let focus = json!(fixture.member_id_strings(&[2]));

    let (status, issue) = fixture.create_issue_with_focus(Some(focus)).await;
    assert_eq!(status, StatusCode::OK, "create with single focus: {issue}");
    let issue_id = issue["issue_id"].as_str().expect("issue id").to_string();

    let selection = fixture.load_selection(&issue_id);
    assert_eq!(selection.selection_policy, SelectionPolicy::Explicit);
    let expected = vec![fixture.member_ids[2]];
    assert_eq!(selection.focus_repository_ids, expected);
    assert_eq!(selection.included_repository_ids, expected);
    assert_eq!(selection.resolve_effective_members(), expected);
}

#[tokio::test]
async fn logical_issue_without_focus_keeps_all_members_backcompat() {
    let fixture = FocusSelectionFixture::new();

    // 不携带 focus_repository_ids（存量请求体）→ 维持 all_members 存量兼容。
    let (status, issue) = fixture.create_issue_with_focus(None).await;
    assert_eq!(status, StatusCode::OK, "create without focus: {issue}");
    let issue_id = issue["issue_id"].as_str().expect("issue id").to_string();

    let selection = fixture.load_selection(&issue_id);
    assert_eq!(selection.selection_policy, SelectionPolicy::AllMembers);
    assert!(selection.focus_repository_ids.is_empty());
    assert!(selection.included_repository_ids.is_empty());

    // 显式空数组语义等同缺省（=未选择成员范围），同样维持 all_members。
    let (status, issue) = fixture.create_issue_with_focus(Some(json!([]))).await;
    assert_eq!(status, StatusCode::OK, "create with empty focus: {issue}");
    let issue_id = issue["issue_id"].as_str().expect("issue id").to_string();
    let selection = fixture.load_selection(&issue_id);
    assert_eq!(selection.selection_policy, SelectionPolicy::AllMembers);
    assert!(selection.focus_repository_ids.is_empty());
}

#[tokio::test]
async fn logical_issue_rejects_focus_outside_lc_active_members() {
    let fixture = FocusSelectionFixture::new();
    let before = fixture.issue_count();

    // 界外 UUID（非本 LC 成员）混入勾选集 → 422 fail-closed，且不留半成品 issue。
    let focus = json!([
        fixture.member_ids[0].0.to_string(),
        Uuid::from_u128(0xdeadbefu128).to_string(),
    ]);
    assert_error(
        fixture.create_issue_with_focus(Some(focus)).await,
        StatusCode::UNPROCESSABLE_ENTITY,
        "focus_repository_outside_active_members",
    );
    // 非 UUID 字符串同口径拒绝。
    assert_error(
        fixture
            .create_issue_with_focus(Some(json!(["not-a-uuid"])))
            .await,
        StatusCode::UNPROCESSABLE_ENTITY,
        "focus_repository_outside_active_members",
    );
    assert_eq!(
        fixture.issue_count(),
        before,
        "no issue persisted on rejection"
    );
}
