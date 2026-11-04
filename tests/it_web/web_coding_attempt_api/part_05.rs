use cadence_aria::product::issue_store::{CreateProductIssueInput, IssueStore};
use cadence_aria::product::logical_codebase::store::legacy_logical_codebase_id;

/// 块3（remove-single-repository-path）夹具预迁移段：project + repository 注册
/// （真实 HTTP 路由），issue 经 store 落盘并携带确定性 legacy 别名 LC 归属
/// （非假 id）。issue 走 store 而非 HTTP 是为了保住「work item 先于身份迁移
/// 创建」的顺序——迁移的兼容回填（migration_executor_backfill）只处理迁移前
/// 已存在的 work item/story（写 target_repository_id、involved/focus），
/// HTTP 创建则要求 LC 权威先在，会把顺序倒过来。
async fn bootstrap_project_repo_issue(
    app: axum::Router,
    repo_path: &std::path::Path,
) -> ProductAppPaths {
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Coding","description":null}),
    )
    .await;
    let repository =
        register_repository_and_wait(app.clone(), json!({"name":"Repo","path":repo_path})).await;
    assert_eq!(repository["repository_id"], "repository_0001");
    let app_paths = ProductAppPaths::new(workspace_root_from_app(app.clone()).await.join(".aria"));
    IssueStore::new(app_paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: Some("repository_0001".to_string()),
            logical_codebase_id: legacy_logical_codebase_id("project_0001"),
            title: "爬楼梯".to_string(),
            description: Some("实现 O(n) 算法".to_string()),
            change_id: None,
            base_branch: None,
        })
        .expect("create issue");
    app_paths
}

/// 块3 夹具迁移段：把 project_0001 的 repository_0001 按生产 v1.2→v1.3 身份
/// 迁移升成 LC 权威（member/checkout/identity-registry/manifest 由
/// IdentityMigrationExecutor 落盘，LC record 由 require_logical_codebase
/// 首读时的 migrate_legacy 自动生成，issue selection 由迁移回填写入），
/// 再建 active 聚合索引（generate 入口经 PlanningContextResolver 读取，
/// 缺失即 aggregate_index_active_missing），最后物化四家 provider 的
/// Confirmed capability 行（bootstrap 行恒 Unknown，仅真实 probe 可写
/// Confirmed；正向夹具不物化「已探测」事实会在 launch 分格 fail-closed 为
/// provider_capability_launch_not_confirmed。fake 在 gateway 中映射为
/// claude_code，四家全 Confirmed 与 it_core 同款复刻）。
///
/// 聚合索引走**生产聚合索引操作 + 假 CodeGraph CLI**（`aggregate_index_fixture`）：
/// 夹具不真跑 `codegraph sync .`——那是本机重活（抢全局 `/tmp/.codegraph` 锁、
/// 吃 `AggregateIndexBudget::INCREMENTAL` 的 30s 预算），并行夹具互踩会 422
/// `codegraph_timeout`，整批用例红在与被测行为无关的夹具前置上。假 CLI 只替换
/// CLI 应答，真实 git 成员快照 / 成员覆盖校验 / durable 世代落盘 / active
/// 投影全部走生产链路（⇒ 读侧 freshness 判定为 active，不会触发真实同步）。
/// rebuild 端点本身的行为覆盖由 `web_aggregate_index_api` 的用例经同一假
/// CLI 承担（端点覆盖不丢，只是与本夹具前置解耦）。
async fn migrate_rebuild_and_seed_capabilities(app_paths: &ProductAppPaths) {
    IdentityMigrationExecutor::new(app_paths.clone())
        .ensure_identity_schema("project_0001")
        .expect("migrate fixture repository into logical codebase authority");
    crate::aggregate_index_fixture::seed_active_aggregate_index(
        app_paths,
        "project_0001",
        &legacy_logical_codebase_id("project_0001"),
    );
    crate::provider_capability_fixture::seed_confirmed_capability(
        app_paths,
        "project_0001",
        &[
            cadence_aria::product::logical_codebase::provider_gateway::ProviderRefType::ClaudeCode,
            cadence_aria::product::logical_codebase::provider_gateway::ProviderRefType::Codex,
            cadence_aria::product::logical_codebase::provider_gateway::ProviderRefType::Pi,
            cadence_aria::product::logical_codebase::provider_gateway::ProviderRefType::KimiCode,
        ],
    );
}

/// 迁移后 LC 成员的 logical id 列表：story/design generate 请求的 involved
/// 钉定值（REQ-PLN-04 确认门要求 involved 非空，夹具不走 AI 自决回写）。
pub(crate) fn lc_member_logical_ids(app_paths: &ProductAppPaths) -> Vec<String> {
    LogicalCodebaseStore::new(app_paths.clone())
        .list_members("project_0001")
        .expect("list fixture logical members")
        .into_iter()
        .map(|member| member.logical_repository_id.0.to_string())
        .collect()
}

/// 块3 夹具迁移后段：经真实 HTTP 路由生成 story/design spec 并确认会话。
/// generate 请求显式钉定 involved（含 design——REQ-PLN-04 对 Story 与 Design
/// 确认门同源），否则确认被 400 involved_repositories_undetermined 拦截。
async fn generate_and_confirm_story_and_design(
    app: &axum::Router,
    involved_repository_ids: Vec<String>,
) {
    // generate 经 build_with_fresh_index 读 active 索引：夹具已发布 fresh 世代
    // ⇒ freshness 判定 active，不会触发真实 codegraph sync；若判定 stale 则会
    // 真跑 sync（与其它夹具的 codegraph 调用互踩 ⇒ codegraph_sync_failed 422），
    // 故持 crate 级锁串行。
    let _codegraph_guard = crate::CODEGRAPH_FIXTURE_LOCK.lock().await;
    let (status, story) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({
            "title":"爬楼梯 Story",
            "involved_repository_ids": involved_repository_ids,
            "author_provider":"fake",
            "reviewer_provider":"fake"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{story}");
    let (status, confirmed) = request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/confirm",
        json!({"confirmed_by":"human"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{confirmed}");
    let (status, design) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/design-specs:generate",
        json!({
            "title":"爬楼梯 Design",
            "story_spec_ids":["story_spec_0001"],
            "involved_repository_ids": involved_repository_ids,
            "author_provider":"fake",
            "reviewer_provider":"fake"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{design}");
    let (status, confirmed) = request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0002/confirm",
        json!({"confirmed_by":"human"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{confirmed}");
}

/// 仓库登记助手（真实 HTTP 路由 + 等初始化操作落定）——crate 级可见：
/// 聚合索引端点用例（`web_aggregate_index_api`）复用同款登记前置。
pub(crate) async fn register_repository_and_wait(app: axum::Router, request: Value) -> Value {
    let (status, accepted) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        request,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");

    let operation_id = accepted["operation_id"].as_str().expect("operation id");
    let operation_uri =
        format!("/api/projects/project_0001/repository-initializations/{operation_id}");
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let (status, snapshot) =
            request_json(app.clone(), Method::GET, &operation_uri, json!({})).await;
        assert_eq!(status, StatusCode::OK, "{snapshot}");
        match snapshot["status"].as_str() {
            Some("completed") => return snapshot["result"]["repository"].clone(),
            Some("failed") => panic!("repository initialization failed: {snapshot}"),
            _ => {
                if tokio::time::Instant::now() >= deadline {
                    panic!("repository initialization did not finish: {snapshot}");
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }
    }
}

pub(crate) async fn request_json(
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

pub(crate) fn assert_global_attempt_id(value: &Value) -> String {
    let id = value["attempt_id"].as_str().expect("attempt id");
    let uuid = id.strip_prefix("coding_attempt_").expect("attempt prefix");
    assert_eq!(uuid.len(), 32);
    uuid::Uuid::parse_str(uuid).expect("valid attempt UUID");
    id.to_string()
}

pub(crate) fn scoped_attempt_uri(attempt_id: &str, suffix: &str) -> String {
    format!("/api/projects/project_0001/issues/issue_0001/coding-attempts/{attempt_id}{suffix}")
}

pub(crate) fn inject_invalid_group_second_work_item(
    app_paths: &ProductAppPaths,
) -> (PathBuf, Vec<u8>) {
    let projection_path = app_paths
        .issue_root("project_0001", "issue_0001")
        .join(
            "work-item-revisions/work_item_plan_0001/plan-projection-bundles/plan_projection_bundle_0001.json",
        );
    let original_projection = fs::read(&projection_path).expect("plan projection bundle");
    let mut invalid_projection: Value =
        serde_json::from_slice(&original_projection).expect("parse plan projection bundle");
    invalid_projection["coder_group_context"]["ordered_logical_work_item_ids"][1] =
        json!("../invalid_work_item");
    fs::write(
        &projection_path,
        serde_json::to_vec_pretty(&invalid_projection)
            .expect("serialize invalid plan projection bundle"),
    )
    .expect("write invalid plan projection bundle");
    (projection_path, original_projection)
}

pub(crate) fn restore_group_second_work_item(fixture: (PathBuf, Vec<u8>)) {
    let (projection_path, original_projection) = fixture;
    fs::write(projection_path, original_projection).expect("restore plan projection bundle");
}

pub(crate) fn assert_group_attempt_creation_rolled_back(app_paths: &ProductAppPaths) {
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    assert!(
        coding_store
            .list_attempts_for_work_item("project_0001", "issue_0001", "work_item_0001")
            .expect("list attempts after failed create")
            .is_empty()
    );
    let issue_root = app_paths.issue_lifecycle_root("project_0001", "issue_0001");
    let attempts_root = issue_root.join("coding-attempts");
    if attempts_root.exists() {
        assert_eq!(
            fs::read_dir(attempts_root)
                .expect("coding attempts root")
                .count(),
            0
        );
    }
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let shared_worktree = lifecycle
        .get_issue_shared_worktree("project_0001", "issue_0001")
        .expect("shared worktree");
    if let Some(shared_worktree) = shared_worktree {
        assert_eq!(shared_worktree.current_active_work_item_id, None);
    }
}

/// 成员仓的**专属底座**：成员仓的公共非 git 父目录 = LC 的聚合根
/// （`common_non_git_parent` 由成员仓路径推出）。
///
/// 聚合根不是可以随便指的目录：生产聚合索引链会在 `manifest.provider_context_root`
/// 写 `codegraph.json`（排除清单）并建 `.codegraph` 库目录。成员仓若直接落在
/// 系统临时目录（`tempfile::tempdir()` ⇒ `/tmp/.tmpX`），聚合根就是**共享的
/// `/tmp`** ⇒ 排除清单写到 `/tmp/codegraph.json`、库目录落到 `/tmp/.codegraph`
/// （与 codegraph 自身的全局状态目录同名 ⇒ 互相污染），且任何走真实 CLI 的路径
/// 都会把整个 `/tmp` 当项目索引 ⇒ 机器上任何残留目录（被杀掉的测试运行留下的
/// 上万个 `.tmp*`）都会拖垮 30s 增量预算
/// （`AggregateIndexBudget::INCREMENTAL.fail_secs = 30`）⇒ `codegraph_timeout`
/// ⇒ 422 `aggregate_index_unavailable`。
///
/// ⇒ 成员仓统一落在这个专属底座内：聚合根随之落在底座（小目录）内。
/// 底座里的残留成员仓按**创建进程 pid** 回收（进程已消失 ⇒ 该轮被杀的残留），
/// 避免底座随运行次数无限膨胀。
fn member_repo_base() -> std::path::PathBuf {
    let base = std::env::temp_dir().join("aria-it-web-member-repos");
    std::fs::create_dir_all(&base).expect("member repo base");
    gc_stale_member_repos(&base);
    base
}

/// 回收底座里属于**已消失进程**的残留成员仓（成员仓目录名带创建进程 pid）。
fn gc_stale_member_repos(base: &std::path::Path) {
    // 非 Linux 无 /proc ⇒ 无法判活 ⇒ 不删（宁可底座膨胀也不误删在用目录）。
    if !std::path::Path::new("/proc").is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Some(pid) = name
            .strip_prefix("member-")
            .and_then(|rest| rest.split('-').next())
        else {
            continue;
        };
        let Ok(pid) = pid.parse::<u32>() else {
            continue;
        };
        if std::path::Path::new("/proc").join(pid.to_string()).exists() {
            continue;
        }
        let _ = std::fs::remove_dir_all(entry.path());
    }
}

/// 成员仓目录：落在专属底座内（底座即聚合根）。
fn member_repo_dir() -> tempfile::TempDir {
    let base = member_repo_base();
    tempfile::Builder::new()
        .prefix(&format!("member-{}-", std::process::id()))
        .tempdir_in(&base)
        .expect("repo")
}

pub(crate) fn git_repo() -> tempfile::TempDir {
    let dir = member_repo_dir();
    run_git(dir.path(), &["init"]);
    run_git(dir.path(), &["config", "user.email", "aria@example.com"]);
    run_git(dir.path(), &["config", "user.name", "Aria Test"]);
    fs::write(dir.path().join("README.md"), "# repo\n").expect("seed readme");
    // 聚合索引布局规则要求每个成员至少有 1 个可索引源码文件
    // （aggregate_index_member_coverage_failed），README 不算源码。
    fs::create_dir_all(dir.path().join("src")).expect("repo src dir");
    fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").expect("repo source");
    run_git(dir.path(), &["add", "."]);
    run_git(dir.path(), &["commit", "-m", "initial"]);
    dir
}

pub(crate) fn run_git(cwd: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .expect("git");
    assert!(status.success());
}

pub(crate) fn prepare_attempt_with_worktree(
    store: &CodingAttemptStore,
    repo_path: &std::path::Path,
    project_id: &str,
    issue_id: &str,
    attempt_id: &str,
) -> CodingExecutionAttempt {
    let attempt = store
        .get_attempt(project_id, issue_id, attempt_id)
        .expect("attempt");
    if !branch_exists(repo_path, &attempt.branch_name) {
        run_git(repo_path, &["branch", &attempt.branch_name, "HEAD"]);
    }
    let worktree_path = if let Some(issue_id) = attempt.branch_name.strip_prefix("aria/issues/") {
        repo_path
            .join(".worktrees")
            .join("aria-issues")
            .join(issue_id)
    } else {
        repo_path
            .join(".worktrees")
            .join("aria-work-items")
            .join(&attempt.work_item_id)
            .join(format!("attempt-{}", attempt.attempt_no))
    };
    if !worktree_path.exists() {
        run_git(
            repo_path,
            &[
                "worktree",
                "add",
                worktree_path.to_str().expect("worktree path"),
                &attempt.branch_name,
            ],
        );
    }
    store
        .update_attempt_worktree_path(project_id, issue_id, attempt_id, worktree_path)
        .expect("update worktree path")
}

fn branch_exists(repo_path: &std::path::Path, branch_name: &str) -> bool {
    Command::new("git")
        .args([
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch_name}"),
        ])
        .current_dir(repo_path)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}
