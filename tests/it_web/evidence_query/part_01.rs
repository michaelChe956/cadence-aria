// C-4 Task 9（REQ-COD-05 it_web 验收）：跨仓只读证据检索 12 场景（part 1/3）。
//
// 本 part 提供多仓 fixture（真实 git 成员仓 + 聚合根 + 合成 aggregate index 直接
// 落盘 store，codegraph 真实 CLI 建索引/查询）+ 场景 ①②③④。
//
// 端点：POST /api/evidence-query body {token, role, query}；成功 200
// {text, truncated, index_stale, budget_remaining}；失败走稳定码 JSON
// {code, message, details}（见 web/error.rs 的 evidence_* 段）。
//
// 端点发现选型：it_web 走 handler oneshot（build_web_router_with_evidence(true)），
// 不启动真实 serve_web，因此不需要 `.aria/web-endpoint` 端口文件；令牌注入仍走真实
// `issue_evidence_token`（T3/T8 生产路径）写 worktree `.aria/evidence-token` + 公共
// exclude。详见 task-9-report.md。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use axum::http::{Method, StatusCode};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::coding_models::{
    AttemptTargetSnapshot, CodingAttemptScope, CodingAttemptStatus, CodingExecutionAttempt,
    CodingExecutionStage,
};
use cadence_aria::product::json_store::write_json;
use cadence_aria::product::logical_codebase::aggregate_index::{
    AggregateIndexMemberSnapshot, AggregateIndexRecord, AggregateIndexStatus, AggregateIndexStore,
};
use cadence_aria::product::logical_codebase::evidence_budget::EVIDENCE_ATTEMPT_CHAR_QUOTA;
use cadence_aria::product::logical_codebase::evidence_token::issue_evidence_token;
use cadence_aria::product::logical_codebase::{
    CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, LogicalCodebaseLayout,
    LogicalCodebaseManifest, LogicalCodebaseStore, LogicalRepositoryId, MemberStatus,
    RepositoryCheckoutId, RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType,
};
use cadence_aria::product::models::ProviderName;
use cadence_aria::web::app::build_web_router_with_evidence;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use cadence_aria::web::workspace_ws_types::ProviderConfigSnapshot;
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};

pub(crate) const PROJECT_ID: &str = "project_0001";
pub(crate) const ISSUE_ID: &str = "issue_0001";
pub(crate) const ATTEMPT_ID: &str = "coding_attempt_0001";

const NOW: &str = "2026-08-14T00:00:00Z";

/// 聚合根源码种子模式：标准（跨仓/本仓/非成员三命中）或超长符号（单次 12k 截断）。
#[derive(Clone, Copy)]
pub(crate) enum EvidenceSourceSeed {
    Standard,
    HugeSymbol,
}

pub(crate) struct EvidenceFixture {
    pub root: TempDir,
    pub aggregate_root: PathBuf,
    pub repo: PathBuf,
    pub worktree: PathBuf,
    pub paths: ProductAppPaths,
    pub api_id: LogicalRepositoryId,
    pub web_id: LogicalRepositoryId,
    pub api_checkout_id: RepositoryCheckoutId,
    pub web_checkout_id: RepositoryCheckoutId,
    pub aggregate_index_id: String,
    pub attempt: CodingExecutionAttempt,
    pub token: String,
}

impl EvidenceFixture {
    pub fn app(&self) -> axum::Router {
        build_web_router_with_evidence(
            WebAppState::new(
                self.root.path().to_path_buf(),
                WebRuntime::new_fake(self.root.path().to_path_buf()),
            ),
            true,
        )
    }
}

/// 标准 fixture（codegraph 真实建索引）。
pub(crate) fn seed_evidence_fixture() -> EvidenceFixture {
    seed_evidence_fixture_with_options(EvidenceSourceSeed::Standard, true)
}

/// 可配置 fixture：`codegraph_indexed=false` 时聚合根不建 codegraph 索引，
/// 用于 503 evidence_query_failed 场景（真实 `codegraph query` 非零退出）。
pub(crate) fn seed_evidence_fixture_with_options(
    seed: EvidenceSourceSeed,
    codegraph_indexed: bool,
) -> EvidenceFixture {
    let root = tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));

    // 1. 聚合根：真实 git 成员仓 api/、web/ + 非成员 other/。
    let aggregate_root = root.path().join("aggregate-root");
    let api_src = aggregate_root.join("api").join("src");
    let web_src = aggregate_root.join("web").join("src");
    let other_dir = aggregate_root.join("other");
    fs::create_dir_all(&api_src).expect("api src");
    fs::create_dir_all(&web_src).expect("web src");
    fs::create_dir_all(&other_dir).expect("other dir");

    git_init_repo(&aggregate_root.join("api"));
    git_init_repo(&aggregate_root.join("web"));
    fs::write(
        api_src.join("lib.rs"),
        "pub fn cross_repo_symbol() -> u32 { 7 }\npub fn api_only_symbol() -> u32 { 1 }\n",
    )
    .expect("api source");
    match seed {
        EvidenceSourceSeed::Standard => {
            fs::write(
                web_src.join("app.ts"),
                "export function cross_repo_symbol() { return 7; }\n",
            )
            .expect("web source");
        }
        EvidenceSourceSeed::HugeSymbol => {
            // 短查询词 "pref" 命中 10 条超长符号定义（> 12k 渲染字符，触发单次截断）。
            let huge = format!("pref{}suff", "x".repeat(1390));
            for index in 1..=12 {
                fs::write(
                    web_src.join(format!("f{index}.ts")),
                    format!("export const {huge} = {index};\n"),
                )
                .expect("huge web source");
            }
        }
    }
    fs::write(
        other_dir.join("notes.ts"),
        "const cross_repo_symbol = 1;\nconst api_only_symbol = 2;\n",
    )
    .expect("other source");
    git_commit_all(&aggregate_root.join("api"));
    git_commit_all(&aggregate_root.join("web"));
    if codegraph_indexed {
        codegraph_init(&aggregate_root);
    }

    // 2. 令牌签发 repo（真实 git 仓库 + issue 级 linked worktree）。
    let repo = root.path().join("repo");
    git_init_repo(&repo);
    fs::write(repo.join("README.md"), "# repo\n").expect("repo readme");
    git_commit_all(&repo);
    let worktree = repo
        .join(".worktrees")
        .join("aria-issues")
        .join(ISSUE_ID);
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            worktree.to_str().expect("worktree path UTF-8"),
            "-b",
            "aria/issues/issue_0001",
        ],
    );

    // 3. logical codebase：manifest（active_aggregate_index_id）+ 成员 + checkout。
    let api_id = LogicalRepositoryId(uuid::Uuid::new_v4());
    let web_id = LogicalRepositoryId(uuid::Uuid::new_v4());
    let api_checkout_id = RepositoryCheckoutId(uuid::Uuid::new_v4());
    let web_checkout_id = RepositoryCheckoutId(uuid::Uuid::new_v4());
    let aggregate_index_id = format!("aggregate_index_{}", uuid::Uuid::new_v4().simple());

    let logical = LogicalCodebaseStore::new(paths.clone());
    let manifest = LogicalCodebaseManifest {
        schema_version: 1,
        project_id: PROJECT_ID.to_string(),
        logical_codebase_id: uuid::Uuid::new_v4(),
        provider_context_root: aggregate_root.clone(),
        layout: LogicalCodebaseLayout::CommonNonGitParent,
        membership_revision: 1,
        member_ids: vec![api_id, web_id],
        active_aggregate_index_id: Some(aggregate_index_id.clone()),
        context_policy_digest: String::new(),
        created_at: NOW.to_string(),
        updated_at: NOW.to_string(),
    };
    logical
        .save_manifest(PROJECT_ID, &manifest)
        .expect("save manifest");
    logical
        .save_member(
            PROJECT_ID,
            &member_record(api_id, "api", api_checkout_id, "repo_api"),
        )
        .expect("save api member");
    logical
        .save_member(
            PROJECT_ID,
            &member_record(web_id, "web", web_checkout_id, "repo_web"),
        )
        .expect("save web member");
    logical
        .save_checkout(
            PROJECT_ID,
            &checkout_record(api_id, api_checkout_id, aggregate_root.join("api")),
        )
        .expect("save api checkout");
    logical
        .save_checkout(
            PROJECT_ID,
            &checkout_record(web_id, web_checkout_id, aggregate_root.join("web")),
        )
        .expect("save web checkout");

    // 合成 aggregate index record 直接落盘 store（不跑聚合初始化 coordinator）。
    let index_store = AggregateIndexStore::new(paths.clone());
    index_store
        .create(
            PROJECT_ID,
            index_record(
                &aggregate_index_id,
                api_id,
                web_id,
                api_checkout_id,
                web_checkout_id,
                "rev-api",
                "rev-web",
                &aggregate_root,
            ),
        )
        .expect("create aggregate index record");

    // 4. attempt record（Running + target_snapshot 锚定 api 成员）。
    let snapshot = target_snapshot(
        api_id,
        api_checkout_id,
        &aggregate_root.join("api"),
        "rev-api",
        1,
    );
    let attempt = attempt_fixture(CodingAttemptStatus::Running, Some(snapshot));
    write_json(&attempt_record_path(&paths, ATTEMPT_ID), &attempt).expect("write attempt record");

    // 5. 真实令牌签发（T3/T8 生产路径：worktree .aria/evidence-token + 公共 exclude + 哈希记录）。
    let token = issue_evidence_token(&paths, &repo, &attempt).expect("issue evidence token");

    EvidenceFixture {
        root,
        aggregate_root,
        repo,
        worktree,
        paths,
        api_id,
        web_id,
        api_checkout_id,
        web_checkout_id,
        aggregate_index_id,
        attempt,
        token,
    }
}

pub(crate) async fn evidence_query(
    app: axum::Router,
    token: &str,
    role: &str,
    query: &str,
) -> (StatusCode, Value) {
    crate::web_coding_attempt_api::request_json(
        app,
        Method::POST,
        "/api/evidence-query",
        json!({"token": token, "role": role, "query": query}),
    )
    .await
}

pub(crate) fn attempt_record_path(paths: &ProductAppPaths, attempt_id: &str) -> PathBuf {
    paths
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("coding-attempts")
        .join(format!("{attempt_id}.json"))
}

pub(crate) fn index_record_path(paths: &ProductAppPaths, id: &str) -> PathBuf {
    paths
        .aggregate_indexes_root(PROJECT_ID)
        .join(format!("{id}.json"))
}

pub(crate) fn audit_file_path(paths: &ProductAppPaths) -> PathBuf {
    paths
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("coding-attempts")
        .join(ATTEMPT_ID)
        .join("evidence-audit.jsonl")
}

pub(crate) fn read_first_audit(paths: &ProductAppPaths) -> Value {
    let content = fs::read_to_string(audit_file_path(paths)).expect("read audit file");
    let first = content.lines().next().expect("first audit line");
    serde_json::from_str(first).expect("audit JSON")
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn index_record(
    id: &str,
    api_id: LogicalRepositoryId,
    web_id: LogicalRepositoryId,
    api_checkout_id: RepositoryCheckoutId,
    web_checkout_id: RepositoryCheckoutId,
    api_revision: &str,
    web_revision: &str,
    codegraph_root: &Path,
) -> AggregateIndexRecord {
    AggregateIndexRecord {
        aggregate_index_id: id.to_string(),
        project_id: PROJECT_ID.to_string(),
        membership_revision: 1,
        status: AggregateIndexStatus::Active,
        member_snapshots: vec![
            AggregateIndexMemberSnapshot::indexed(
                api_id,
                api_checkout_id,
                api_revision.to_string(),
                false,
                NOW.to_string(),
            ),
            AggregateIndexMemberSnapshot::indexed(
                web_id,
                web_checkout_id,
                web_revision.to_string(),
                false,
                NOW.to_string(),
            ),
        ],
        observed_after_member_snapshots: Vec::new(),
        codegraph_version: "1.5.0".to_string(),
        codegraph_root: codegraph_root.to_path_buf(),
        config_digest: String::new(),
        created_at: NOW.to_string(),
        updated_at: NOW.to_string(),
        supersedes_aggregate_index_id: None,
        warning: None,
    }
}

fn target_snapshot(
    logical_id: LogicalRepositoryId,
    checkout_id: RepositoryCheckoutId,
    canonical_path: &Path,
    revision: &str,
    membership_revision: u64,
) -> AttemptTargetSnapshot {
    AttemptTargetSnapshot {
        logical_repository_id: logical_id,
        checkout_id,
        physical_repository_id: format!("repo_{}", logical_id.0),
        canonical_path: canonical_path.to_path_buf(),
        git_dir_identity: "git-dir-id".to_string(),
        revision: Some(revision.to_string()),
        policy_digest: "policy-digest".to_string(),
        membership_revision,
        captured_at: NOW.to_string(),
        capture_source: "test".to_string(),
    }
}

fn attempt_fixture(
    status: CodingAttemptStatus,
    target_snapshot: Option<AttemptTargetSnapshot>,
) -> CodingExecutionAttempt {
    CodingExecutionAttempt {
        id: ATTEMPT_ID.to_string(),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        work_item_id: "work_item_0001".to_string(),
        attempt_no: 1,
        scope: CodingAttemptScope::WorkItem,
        status,
        version: 0,
        manual_recovery_reason: None,
        admission_ticket_consumed_at: None,
        stage: CodingExecutionStage::Coding,
        base_branch: "main".to_string(),
        branch_name: "aria/issues/issue_0001".to_string(),
        worktree_path: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::Fake,
            reviewer: None,
            review_rounds: 0,
            permission_modes: Default::default(),
        },
        rework_count: 0,
        max_auto_rework: 0,
        work_item_group_id: None,
        current_work_item_id: Some("work_item_0001".to_string()),
        active_unit_id: None,
        head_commit: None,
        pushed_remote: None,
        review_request_id: None,
        provider_conversations: Vec::new(),
        created_at: NOW.to_string(),
        updated_at: NOW.to_string(),
        target_snapshot,
        completed_at: None,
    }
}

fn member_record(
    id: LogicalRepositoryId,
    alias: &str,
    checkout_id: RepositoryCheckoutId,
    physical_id: &str,
) -> CodebaseMemberRecord {
    CodebaseMemberRecord {
        logical_repository_id: id,
        physical_repository_id: physical_id.to_string(),
        alias: alias.to_string(),
        role: "backend".to_string(),
        ordinal: 0,
        source_identity: RepositorySourceIdentity {
            scheme: "git_dir_only_v1".to_string(),
            key_digest: format!("sha256:{alias}"),
            canonical_git_dir: PathBuf::from(format!("/workspace/{alias}/.git")),
            canonical_origin: None,
            first_seen_path_hash: format!("hash:{alias}"),
        },
        repo_type: RepositoryType::Unknown,
        tech_stack: Vec::new(),
        owner: None,
        tags: Vec::new(),
        default_ref: None,
        checkout_ids: vec![checkout_id],
        status: MemberStatus::Active,
        created_at: NOW.to_string(),
        updated_at: NOW.to_string(),
    }
}

fn checkout_record(
    logical_id: LogicalRepositoryId,
    checkout_id: RepositoryCheckoutId,
    canonical_path: PathBuf,
) -> RepositoryCheckoutRecord {
    RepositoryCheckoutRecord {
        checkout_id,
        logical_repository_id: logical_id,
        physical_repository_id: format!("repo_{}", logical_id.0),
        kind: CheckoutKind::Main,
        canonical_path,
        checkout_path_hash: "checkout-hash".to_string(),
        git_dir_identity: "git-dir-id".to_string(),
        revision: None,
        availability: CheckoutAvailability::Available,
        observed_at: NOW.to_string(),
        created_at: NOW.to_string(),
        updated_at: NOW.to_string(),
    }
}

fn git_init_repo(dir: &Path) {
    fs::create_dir_all(dir).expect("repo dir");
    crate::web_coding_attempt_api::run_git(dir, &["init"]);
    crate::web_coding_attempt_api::run_git(dir, &["config", "user.email", "aria@example.com"]);
    crate::web_coding_attempt_api::run_git(dir, &["config", "user.name", "Aria Test"]);
}

fn git_commit_all(dir: &Path) {
    crate::web_coding_attempt_api::run_git(dir, &["add", "-A"]);
    crate::web_coding_attempt_api::run_git(dir, &["commit", "-m", "initial"]);
}

fn run_git(cwd: &Path, args: &[&str]) {
    crate::web_coding_attempt_api::run_git(cwd, args);
}

fn git_output(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git output");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn codegraph_init(root: &Path) {
    let status = Command::new("codegraph")
        .args(["init", "."])
        .current_dir(root)
        .status()
        .expect("codegraph init");
    assert!(status.success(), "codegraph init failed");
}

// ---- ① Coder 合法查询：跨仓命中 + 返回文本 + 审计落盘 ----
#[tokio::test]
async fn coder_cross_member_query_returns_text_and_persists_audit() {
    let fx = seed_evidence_fixture();
    let app = fx.app();

    let (status, body) = evidence_query(app, &fx.token, "coder", "cross_repo_symbol").await;

    assert_eq!(status, StatusCode::OK, "cross-member query: {body}");
    let text = body["text"].as_str().expect("text");
    assert!(
        text.contains("web/src/app.ts"),
        "cross-member web hit must be present: {text}"
    );
    assert!(
        text.contains("cross_repo_symbol"),
        "symbol must be present: {text}"
    );
    assert!(!text.contains("api/"), "target member hit filtered: {text}");
    assert!(!text.contains("other/"), "non-member hit filtered: {text}");
    assert_eq!(body["truncated"], json!(false));
    assert_eq!(body["index_stale"], json!(false));
    let result_chars = text.chars().count();
    assert_eq!(
        body["budget_remaining"],
        json!(EVIDENCE_ATTEMPT_CHAR_QUOTA - result_chars)
    );

    let audit = read_first_audit(&fx.paths);
    assert_eq!(audit["attempt_id"], ATTEMPT_ID);
    assert_eq!(audit["role"], "coder(role_self_reported)");
    assert_eq!(audit["query"], "cross_repo_symbol");
    assert_eq!(audit["hit_count"], 1);
    assert_eq!(audit["result_chars"], result_chars);
    assert_eq!(audit["snapshot_refs"], json!(["web/src/app.ts"]));
    assert_eq!(
        audit["budget_remaining"],
        json!(EVIDENCE_ATTEMPT_CHAR_QUOTA - result_chars)
    );
}

// ---- ② 无令牌 / 错令牌 → 401 evidence_unauthorized ----
#[tokio::test]
async fn missing_or_wrong_token_returns_401_evidence_unauthorized() {
    let fx = seed_evidence_fixture();
    let app = fx.app();

    let (empty_status, empty_body) = evidence_query(app.clone(), "", "coder", "cross_repo_symbol").await;
    assert_eq!(empty_status, StatusCode::UNAUTHORIZED, "{empty_body}");
    assert_eq!(empty_body["code"], "evidence_unauthorized");

    let (wrong_status, wrong_body) =
        evidence_query(app, "deadbeef-wrong-token", "coder", "cross_repo_symbol").await;
    assert_eq!(wrong_status, StatusCode::UNAUTHORIZED, "{wrong_body}");
    assert_eq!(wrong_body["code"], "evidence_unauthorized");
}

// ---- ③ attempt 非 Running → 403 evidence_forbidden ----
#[tokio::test]
async fn non_running_attempt_returns_403_evidence_forbidden() {
    let fx = seed_evidence_fixture();
    let mut completed = fx.attempt.clone();
    completed.status = CodingAttemptStatus::Completed;
    write_json(&attempt_record_path(&fx.paths, ATTEMPT_ID), &completed)
        .expect("write completed attempt");

    let app = fx.app();
    let (status, body) = evidence_query(app, &fx.token, "coder", "cross_repo_symbol").await;

    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "evidence_forbidden");
}

// ---- ④ 本仓 / 非成员命中被过滤（跨仓空结果仍 200） ----
#[tokio::test]
async fn target_and_non_member_hits_are_filtered() {
    let fx = seed_evidence_fixture();
    let app = fx.app();

    // api_only_symbol 仅存在于本仓 api/（目标成员）与非成员 other/，过滤后无跨仓命中。
    let (status, body) = evidence_query(app, &fx.token, "coder", "api_only_symbol").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["text"], json!(""));
    assert_eq!(body["truncated"], json!(false));
    assert_eq!(body["budget_remaining"], json!(EVIDENCE_ATTEMPT_CHAR_QUOTA));

    let audit = read_first_audit(&fx.paths);
    assert_eq!(audit["query"], "api_only_symbol");
    assert_eq!(audit["hit_count"], 0);
    assert_eq!(audit["snapshot_refs"], json!([]));
    assert_eq!(audit["result_chars"], 0);
}
