// Task 11（REQ-ENV-07 / REQ-PROMPT-01/02 it_web 验收）共享 fixture 与 HTTP/git helper。
//
// 场景 A-E 复用同一多成员逻辑代码库夹具：每个成员是真实 git 仓库（默认分支 main）
// + 可选 bare `origin` 远端，manifest 的 provider_context_root 指向 aggregate-root。
// 「有 origin / 无 origin」开关用于场景 D 的 push 失败注入（与 C-5 coordinator 单测
// `setup(&[("no-remote", false), ...])` 同构：无 origin → git push 失败 → 条目 Failed）。
//
// 场景 F 的 prompt 分流 fixture 与 A-E 不同（无需真实 git 远端，只需 manifest +
// policy artifact + 可达 aggregate-root），在 part_05 内独立构造。

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::coding_attempt_store::CodingAttemptStore;
use cadence_aria::product::coding_models::ReviewRequestOwnerKind;
use cadence_aria::product::logical_codebase::{
    CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, IssueCodebaseSelection,
    IssueCodebaseSelectionStore, LogicalCodebaseManifest, LogicalCodebaseStore, LogicalRepositoryId,
    MemberStatus, PointerBlockFields, RepositoryCheckoutId, RepositoryCheckoutRecord,
    RepositorySourceIdentity, RepositoryType, render_pointer_block,
};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::{TempDir, tempdir};
use tower::ServiceExt;

const PROJECT_ID: &str = "project_0001";
const PUBLICATIONS_URI: &str =
    "/api/projects/project_0001/logical-codebase/pointer-publications";

/// 运行 git 命令并断言成功。
fn git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("run git fixture command");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// 运行 git 命令并返回 trimmed stdout（调用方自行断言失败语义）。
fn git_out(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("run git fixture command");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// HTTP JSON 请求（it_web 通用形态）。
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

async fn create_publication(app: &axum::Router, batch_kind: &str) -> (StatusCode, Value) {
    request_json(
        app.clone(),
        Method::POST,
        PUBLICATIONS_URI,
        json!({"batch_kind": batch_kind}),
    )
    .await
}

async fn retry_repo(
    app: &axum::Router,
    publication_id: &str,
    member_repo_id: &str,
) -> (StatusCode, Value) {
    request_json(
        app.clone(),
        Method::POST,
        &format!("{PUBLICATIONS_URI}/{publication_id}/retry-repo"),
        json!({"member_repo_id": member_repo_id}),
    )
    .await
}

async fn revoke(app: &axum::Router, publication_id: &str) -> (StatusCode, Value) {
    request_json(
        app.clone(),
        Method::POST,
        &format!("{PUBLICATIONS_URI}/{publication_id}/revoke"),
        json!({}),
    )
    .await
}

#[derive(Clone)]
struct MemberRepo {
    logical_id: LogicalRepositoryId,
    checkout_id: RepositoryCheckoutId,
    repo_path: PathBuf,
    bare_remote: Option<PathBuf>,
}

impl MemberRepo {
    fn member_record(&self) -> CodebaseMemberRecord {
        let now = "2026-08-14T00:00:00Z".to_string();
        CodebaseMemberRecord {
            logical_repository_id: self.logical_id,
            physical_repository_id: format!("repo_{}", self.logical_id.0),
            alias: format!("member_{}", self.logical_id.0.simple()),
            role: "service".to_string(),
            ordinal: 1,
            source_identity: RepositorySourceIdentity::from_git_parts(
                &self.repo_path,
                self.repo_path.join(".git"),
                Some(format!(
                    "ssh://git@example.test/acme/{}.git",
                    self.logical_id.0
                )),
            ),
            repo_type: RepositoryType::Unknown,
            tech_stack: Vec::new(),
            owner: None,
            tags: Vec::new(),
            default_ref: None,
            checkout_ids: vec![self.checkout_id],
            status: MemberStatus::Active,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    fn checkout_record(&self) -> RepositoryCheckoutRecord {
        let now = "2026-08-14T00:00:00Z".to_string();
        RepositoryCheckoutRecord {
            checkout_id: self.checkout_id,
            logical_repository_id: self.logical_id,
            physical_repository_id: format!("repo_{}", self.logical_id.0),
            kind: CheckoutKind::Main,
            canonical_path: self.repo_path.clone(),
            checkout_path_hash: format!("sha256:{}", self.logical_id.0),
            git_dir_identity: format!("sha256:git-{}", self.logical_id.0),
            revision: None,
            availability: CheckoutAvailability::Available,
            observed_at: now.clone(),
            created_at: now.clone(),
            updated_at: now,
        }
    }

    fn head_sha(&self) -> String {
        git_out(&self.repo_path, &["rev-parse", "HEAD"])
    }
}

/// 创建单个成员 git 仓库（默认分支 main；`with_origin` 决定是否配置 bare 远端）。
fn setup_member(tmp: &Path, name: &str, with_origin: bool) -> MemberRepo {
    let logical_id = LogicalRepositoryId(uuid::Uuid::new_v4());
    let checkout_id = RepositoryCheckoutId(uuid::Uuid::new_v4());
    let repo_path = tmp.join(name);
    std::fs::create_dir_all(&repo_path).unwrap();
    git(&repo_path, &["init"]);
    git(&repo_path, &["config", "user.email", "test@example.com"]);
    git(&repo_path, &["config", "user.name", "Test User"]);
    std::fs::write(repo_path.join("README.md"), "base\n").unwrap();
    git(&repo_path, &["add", "README.md"]);
    git(&repo_path, &["commit", "-m", "base"]);

    let bare_remote = if with_origin {
        let remote_path = tmp.join(format!("{name}-origin.git"));
        std::fs::create_dir_all(&remote_path).unwrap();
        git(&remote_path, &["init", "--bare"]);
        git(
            &repo_path,
            &["remote", "add", "origin", remote_path.to_str().unwrap()],
        );
        git(&repo_path, &["branch", "-m", "main"]);
        git(&repo_path, &["push", "-u", "origin", "main"]);
        Some(remote_path)
    } else {
        // 无 origin 成员同样统一为 main 分支（仅不配置远端）；用于场景 D 的
        // push 失败注入，修复时可直接 `git push -u origin main`。
        git(&repo_path, &["branch", "-m", "main"]);
        None
    };

    MemberRepo {
        logical_id,
        checkout_id,
        repo_path,
        bare_remote,
    }
}

struct PointerFixture {
    root: TempDir,
    app: axum::Router,
    members: Vec<MemberRepo>,
    logical_codebase_id: String,
    aggregate_root: PathBuf,
}

/// 播种多成员逻辑代码库 fixture（manifest + members + checkouts + selection + policy）。
fn setup_pointer_fixture(member_specs: &[(&str, bool)]) -> PointerFixture {
    let root = tempdir().expect("root");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    cadence_aria::product::project_store::ProjectStore::new(app_paths.clone())
        .create(cadence_aria::product::project_store::CreateProjectInput {
            name: "pointer fixture".to_string(),
            description: None,
        })
        .unwrap();
    let members: Vec<MemberRepo> = member_specs
        .iter()
        .map(|(name, with_origin)| setup_member(root.path(), name, *with_origin))
        .collect();

    let aggregate_root = root.path().join("aggregate-root");
    std::fs::create_dir_all(&aggregate_root).unwrap();
    let manifest = LogicalCodebaseManifest::new(
        PROJECT_ID,
        aggregate_root.clone(),
        members.iter().map(|member| member.logical_id).collect(),
    );
    let logical_codebase_id = manifest.logical_codebase_id.to_string();
    let store = LogicalCodebaseStore::new(app_paths.clone());
    store.save_manifest(PROJECT_ID, &manifest).unwrap();
    for member in &members {
        store
            .save_member(PROJECT_ID, &member.member_record())
            .unwrap();
        store
            .save_checkout(PROJECT_ID, &member.checkout_record())
            .unwrap();
    }
    IssueCodebaseSelectionStore::new(app_paths.clone())
        .save(&IssueCodebaseSelection::explicit(
            PROJECT_ID,
            "issue_0001",
            members.iter().map(|member| member.logical_id).collect(),
            Vec::new(),
            Vec::new(),
            None,
        ))
        .unwrap();
    cadence_aria::product::logical_codebase::policy::AggregatePolicyArtifactStore::new(
        app_paths.clone(),
    )
    .ensure_bootstrap(&manifest)
    .unwrap();

    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));

    PointerFixture {
        root,
        app,
        members,
        logical_codebase_id,
        aggregate_root,
    }
}

impl PointerFixture {
    fn app_paths(&self) -> ProductAppPaths {
        ProductAppPaths::new(self.root.path().join(".aria"))
    }

    fn add_member(&mut self, name: &str, with_origin: bool) -> MemberRepo {
        let member = setup_member(self.root.path(), name, with_origin);
        let store = LogicalCodebaseStore::new(self.app_paths());
        store
            .save_member(PROJECT_ID, &member.member_record())
            .unwrap();
        store
            .save_checkout(PROJECT_ID, &member.checkout_record())
            .unwrap();
        let mut manifest = store.load_manifest(PROJECT_ID).unwrap().unwrap();
        manifest.member_ids.push(member.logical_id);
        manifest.membership_revision += 1;
        store.save_manifest(PROJECT_ID, &manifest).unwrap();
        self.members.push(MemberRepo {
            logical_id: member.logical_id,
            checkout_id: member.checkout_id,
            repo_path: member.repo_path,
            bare_remote: member.bare_remote,
        });
        self.members.last().cloned().expect("pushed member")
    }

    fn review_requests_root(&self, publication_id: &str) -> PathBuf {
        self.root
            .path()
            .join(".aria/projects")
            .join(PROJECT_ID)
            .join("logical-codebase/pointer-publications")
            .join(publication_id)
            .join("review-requests")
    }

    /// 把「已合并」的指针块写入成员主 checkout（不 commit，模拟人工合并结果），
    /// 使下一次全量发布对该成员 classify_merge → Skip（幂等重发断言用）。
    fn plant_merged_pointer_block(&self, member: &MemberRepo) {
        let block = render_pointer_block(&PointerBlockFields {
            logical_codebase_id: self.logical_codebase_id.clone(),
            repo_id: member.logical_id.0.to_string(),
            canonical_policy_locator: self.aggregate_root.to_string_lossy().into_owned(),
            pointer_version: 1,
        });
        std::fs::write(member.repo_path.join(".aria-pointer.md"), block).unwrap();
    }
}

fn remote_has_branch(bare: &Path, branch: &str) -> bool {
    let output = Command::new("git")
        .args(["show-ref", "--verify", &format!("refs/heads/{branch}")])
        .current_dir(bare)
        .output()
        .expect("run git show-ref");
    output.status.success() && !String::from_utf8_lossy(&output.stdout).trim().is_empty()
}

/// 从 bare 远端读取某分支上某文件的 blob 内容（断言标记块 commit 用）。
fn remote_branch_file(bare: &Path, branch: &str, file: &str) -> String {
    git_out(bare, &["show", &format!("refs/heads/{branch}:{file}")])
}

fn entry<'a>(publication: &'a Value, member_repo_id: &str) -> &'a Value {
    publication["entries"]
        .as_array()
        .expect("entries array")
        .iter()
        .find(|entry| entry["member_repo_id"] == member_repo_id)
        .unwrap_or_else(|| panic!("entry for {member_repo_id} missing: {publication}"))
}

/// 断言「review request 落盘在 pointer-publications 分区且 owner_kind 正确」。
fn assert_pointer_review_requests(
    fixture: &PointerFixture,
    publication_id: &str,
    expect_revoked: bool,
) {
    let requests = CodingAttemptStore::new(fixture.app_paths())
        .list_pointer_review_requests(PROJECT_ID, publication_id)
        .expect("list pointer review requests");
    assert_eq!(
        requests.len(),
        fixture.members.len(),
        "one review request per member"
    );
    for member in &fixture.members {
        let member_repo_id = member.logical_id.0.to_string();
        let request = requests
            .iter()
            .find(|request| request.id == format!("rr-{publication_id}-{member_repo_id}"))
            .unwrap_or_else(|| panic!("review request for {member_repo_id} missing"));
        assert_eq!(
            request.owner_kind,
            ReviewRequestOwnerKind::PointerPublication
        );
        assert_eq!(request.attempt_id, format!("pointer-pub-{publication_id}"));
        assert_eq!(request.revoked, expect_revoked);
    }
}
