//! final_confirm 写入范围门禁的路径域归一回归测试。
//!
//! 现场形态（issue_0001 / WI-001 / busi）：多仓 Work Item 的 scope 以聚合根为基准
//! （`busi/src/notifications.ts`、`busi/test/**`），而 git 事实是仓内相对路径
//! （`src/notifications.ts`）。修复前两者直接比较、永不相等，合法改动被
//! `work_item_diff_scope_violation` 拒绝。
//!
//! 覆盖：
//! - LC attempt（`target_snapshot = Some`）：仓内相对 diff + 根相对带前缀 scope 放行；
//! - 范围外文件仍被拒；forbidden 在根相对写法下真正生效；
//! - 单仓（`target_snapshot = None`）行为不变；
//! - 前缀解析失败 fail-closed；路径卫生（`.git`）沿用。

use super::*;
use crate::product::coding_models::AttemptTargetSnapshot;
use crate::product::coding_workspace_engine::gates::validate_changed_files_in_scope;
use crate::product::logical_codebase::{
    LogicalCodebaseManifest, LogicalCodebaseStore, LogicalRepositoryId, RepositoryCheckoutId,
};
use uuid::Uuid;

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_0001";
const WORK_ITEM_ID: &str = "work_item_0001";

struct LcGateFixture {
    _root: tempfile::TempDir,
    store: CodingAttemptStore,
    engine: CodingWorkspaceEngine,
    attempt: CodingExecutionAttempt,
    worktree: PathBuf,
}

fn member_snapshot(canonical_path: PathBuf) -> AttemptTargetSnapshot {
    AttemptTargetSnapshot {
        logical_repository_id: LogicalRepositoryId(Uuid::new_v4()),
        checkout_id: RepositoryCheckoutId(Uuid::new_v4()),
        physical_repository_id: "repository_0001".to_string(),
        canonical_path,
        git_dir_identity: "sha256:test".to_string(),
        revision: Some("abcdef".to_string()),
        policy_digest: "policy_digest".to_string(),
        membership_revision: 1,
        captured_at: "2026-10-15T00:00:00Z".to_string(),
        capture_source: "test".to_string(),
    }
}

/// 构造 LC 场景：聚合根 `<root>/aggregate`，成员 checkout `<root>/aggregate/busi`，
/// 工作 worktree 是独立 git 仓；`write_manifest=false` 用于验证前缀解析 fail-closed。
fn lc_gate_fixture(
    exclusive: &[&str],
    forbidden: &[&str],
    member_dir_in_root: bool,
    write_manifest: bool,
) -> LcGateFixture {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = CodingAttemptStore::new(paths.clone());
    let aggregate = root.path().join("aggregate");
    let member = if member_dir_in_root {
        aggregate.join("busi")
    } else {
        root.path().join("elsewhere").join("busi")
    };
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree dir");
    init_test_git_repo(&worktree);
    let head = git_stdout(&worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();

    let snapshot = member_snapshot(member);
    if write_manifest {
        let manifest = LogicalCodebaseManifest::new(
            PROJECT_ID,
            aggregate,
            vec![snapshot.logical_repository_id],
        );
        LogicalCodebaseStore::new(paths.clone())
            .save_manifest(PROJECT_ID, &manifest)
            .expect("save manifest");
    }

    LifecycleStore::new(paths.clone())
        .create_work_item(CreateWorkItemInput {
            id: Some(WORK_ITEM_ID.to_string()),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: "repository_0001".to_string(),
            title: "busi 通知领域模型".to_string(),
            exclusive_write_scopes: exclusive.iter().map(|s| (*s).to_string()).collect(),
            forbidden_write_scopes: forbidden.iter().map(|s| (*s).to_string()).collect(),
            ..Default::default()
        })
        .expect("create work item");

    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: WORK_ITEM_ID.to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree.clone()),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: None,
                review_rounds: 0,
                permission_modes: Default::default(),
            },
            target_snapshot: Some(snapshot),
            max_auto_rework: 2,
        })
        .expect("create attempt");
    let attempt = store
        .update_attempt_head_commit(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            Some(head),
        )
        .expect("head commit");

    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    LcGateFixture {
        _root: root,
        store,
        engine,
        attempt,
        worktree,
    }
}

/// 在 worktree 内写文件并暂存（porcelain 逐文件列出，不折叠成目录）。
fn stage_files(worktree: &Path, files: &[&str]) {
    for relative in files {
        let absolute = worktree.join(relative);
        fs::create_dir_all(absolute.parent().expect("parent")).expect("mkdir");
        fs::write(&absolute, "// changed\n").expect("write");
    }
    run_test_git(worktree, &["add", "--all"]);
}

#[tokio::test]
async fn lc_gate_allows_repo_relative_diff_for_root_relative_prefixed_scopes() {
    // 现场形态：scope=busi/src/notifications.ts + busi/test/**，diff=仓内相对。
    let fx = lc_gate_fixture(
        &["busi/src/notifications.ts", "busi/test/**"],
        &["api/src/**", "gateway/src/**", "frontend/src/**"],
        true,
        true,
    );
    stage_files(
        &fx.worktree,
        &["src/notifications.ts", "test/notifications.test.ts"],
    );

    fx.engine
        .run_completion_gates(&fx.attempt)
        .await
        .expect("合法改动必须越过写入范围门禁");
    // 收尾：未经 git 暂存的脏文件不在本用例范围；store 仍可读证明夹具完整。
    assert!(
        fx.store
            .get_attempt(&fx.attempt.project_id, &fx.attempt.issue_id, &fx.attempt.id)
            .is_ok()
    );
}

#[tokio::test]
async fn lc_gate_rejects_file_outside_exclusive_scope_with_repo_relative_payload() {
    let fx = lc_gate_fixture(
        &["busi/src/notifications.ts", "busi/test/**"],
        &[],
        true,
        true,
    );
    stage_files(&fx.worktree, &["src/other.ts"]);

    let error = fx
        .engine
        .run_completion_gates(&fx.attempt)
        .await
        .expect_err("范围外文件必须被拒");
    match error {
        CodingWorkspaceEngineError::WorkItemDiffScopeViolation(path) => {
            assert_eq!(path, "src/other.ts", "payload 保持仓内相对路径");
        }
        other => panic!("expected diff scope violation, got {other:?}"),
    }
}

#[tokio::test]
async fn lc_gate_forbidden_scope_in_root_relative_form_takes_effect() {
    // 修复前 forbidden 对仓内相对路径永不命中（静默空转）；修复后根相对写法真正生效。
    // exclusive 用仓内相对写法放行该文件，使拒绝只可能来自 forbidden 分支。
    let fx = lc_gate_fixture(&["src/**"], &["busi/src/**"], true, true);
    stage_files(&fx.worktree, &["src/notifications.ts"]);

    let error = fx
        .engine
        .run_completion_gates(&fx.attempt)
        .await
        .expect_err("forbidden 命中必须拒绝");
    assert!(matches!(
        error,
        CodingWorkspaceEngineError::WorkItemDiffScopeViolation(ref path)
            if path == "src/notifications.ts"
    ));
}

/// Bug1-A：中文证据路径（真实 UTF-8，现场 WI-004 形态）必须命中
/// `cadence/verify/**`；同结构 ASCII 对照同时放行（证明非权限放宽）。
/// 修复前 porcelain 转义串（带引号八进制）进范围判定，合法交付被误拒。
#[tokio::test]
async fn lc_gate_allows_real_utf8_chinese_evidence_paths() {
    let fx = lc_gate_fixture(&["cadence/verify/**"], &[], true, true);
    stage_files(
        &fx.worktree,
        &[
            "cadence/verify/2026-10-16_验收报告_v1.0.md",
            "cadence/verify/2026-10-16_acceptance_report_v1.0.md",
        ],
    );

    fx.engine
        .run_completion_gates(&fx.attempt)
        .await
        .expect("真实 UTF-8 中文证据路径命中 cadence/verify/** 必须放行");
    assert!(
        fx.store
            .get_attempt(&fx.attempt.project_id, &fx.attempt.issue_id, &fx.attempt.id)
            .is_ok()
    );
}

/// Bug1-A：中文 forbidden 真实路径命中后稳定拒绝（exclusive 同时声明该目录，
/// 使拒绝只能来自 forbidden 分支），报错 payload 为真实路径。
#[tokio::test]
async fn lc_gate_forbidden_keeps_denying_real_utf8_chinese_paths() {
    let fx = lc_gate_fixture(&["cadence/verify/**"], &["cadence/verify/**"], true, true);
    stage_files(
        &fx.worktree,
        &["cadence/verify/2026-10-16_验收报告_v1.0.md"],
    );

    let error = fx
        .engine
        .run_completion_gates(&fx.attempt)
        .await
        .expect_err("中文 forbidden 命中必须拒绝");
    assert!(matches!(
        &error,
        CodingWorkspaceEngineError::WorkItemDiffScopeViolation(path)
            if path == "cadence/verify/2026-10-16_验收报告_v1.0.md"
    ));
}

/// Bug1-A 洗白反例（WI-003 形态）：scope 未声明证据目录时中文证据路径仍拒，
/// 报错 payload 为真实路径——采集根修不放宽任何越界判定。
#[tokio::test]
async fn lc_gate_rejects_chinese_evidence_dir_when_policy_lacks_it() {
    let fx = lc_gate_fixture(&["src/**"], &[], true, true);
    stage_files(
        &fx.worktree,
        &["cadence/verify/2026-10-16_验收报告_v1.0.md"],
    );

    let error = fx
        .engine
        .run_completion_gates(&fx.attempt)
        .await
        .expect_err("未声明目录必须拒绝");
    assert!(matches!(
        &error,
        CodingWorkspaceEngineError::WorkItemDiffScopeViolation(path)
            if path == "cadence/verify/2026-10-16_验收报告_v1.0.md"
    ));
}

/// Bug1-A Review Focus #2：porcelain `-z` 后状态清单含无效 UTF-8 文件名时
/// 必须 fail-closed（引擎 `Git(Parse)` 错误），绝不用替换字符继续或因采集
/// 失败静默放行。
#[tokio::test]
async fn lc_gate_fails_closed_when_status_paths_are_not_utf8() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let fx = lc_gate_fixture(&["cadence/verify/**"], &[], true, true);
    fs::create_dir_all(fx.worktree.join("cadence/verify")).expect("verify dir");
    let invalid_name = OsStr::from_bytes(b"cadence/verify/bad-\xff-report.md");
    fs::write(fx.worktree.join(invalid_name), "bad bytes\n").expect("write invalid name");
    let output = StdCommand::new("git")
        .arg("add")
        .arg("--")
        .arg(invalid_name)
        .current_dir(&fx.worktree)
        .output()
        .expect("git add invalid utf8 path");
    assert!(
        output.status.success(),
        "git add invalid utf8 path: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let error = fx
        .engine
        .run_completion_gates(&fx.attempt)
        .await
        .expect_err("无效 UTF-8 路径必须 fail-closed");
    assert!(
        matches!(
            error,
            CodingWorkspaceEngineError::Git(GitWorkspaceError::Parse(_))
        ),
        "unexpected error: {error:?}"
    );
}

/// F1（P2）：rename 源路径必须进清单——exclusive=src/** 下把 scope 外
/// 文件 `git mv` 进 scope 时，越界删除（源路径）不得因目标命中而放行。
#[tokio::test]
async fn lc_gate_rejects_rename_whose_source_is_out_of_scope() {
    let fx = lc_gate_fixture(&["src/**"], &[], true, true);
    fs::create_dir_all(fx.worktree.join("cadence/verify")).expect("verify dir");
    fs::create_dir_all(fx.worktree.join("src")).expect("src dir");
    fs::write(fx.worktree.join("cadence/verify/report.md"), "report\n").expect("write source");
    run_test_git(&fx.worktree, &["add", "--all"]);
    run_test_git(&fx.worktree, &["commit", "-m", "base with report"]);
    run_test_git(
        &fx.worktree,
        &["mv", "cadence/verify/report.md", "src/report.md"],
    );

    let error = fx
        .engine
        .run_completion_gates(&fx.attempt)
        .await
        .expect_err("越界 rename 源路径必须拒绝");
    assert!(matches!(
        &error,
        CodingWorkspaceEngineError::WorkItemDiffScopeViolation(path)
            if path == "cadence/verify/report.md"
    ));
}

/// F1 对照：同 scope 内重命名（src/a → src/b）源与目标均命中，必须放行
/// ——堵越界删除不得引入对合法重命名的误拒。
#[tokio::test]
async fn lc_gate_allows_rename_within_declared_scope() {
    let fx = lc_gate_fixture(&["src/**"], &[], true, true);
    fs::create_dir_all(fx.worktree.join("src")).expect("src dir");
    fs::write(fx.worktree.join("src/a.ts"), "a\n").expect("write a");
    run_test_git(&fx.worktree, &["add", "--all"]);
    run_test_git(&fx.worktree, &["commit", "-m", "base with a"]);
    run_test_git(&fx.worktree, &["mv", "src/a.ts", "src/b.ts"]);

    fx.engine
        .run_completion_gates(&fx.attempt)
        .await
        .expect("同 scope 重命名：源与目标均命中必须放行");
}

#[tokio::test]
async fn lc_gate_fails_closed_when_manifest_is_missing() {
    let fx = lc_gate_fixture(&["busi/**"], &[], true, false);
    stage_files(&fx.worktree, &["src/notifications.ts"]);

    let error = fx
        .engine
        .run_completion_gates(&fx.attempt)
        .await
        .expect_err("前缀无法解析必须 fail-closed");
    assert!(
        matches!(
            error,
            CodingWorkspaceEngineError::WorkItemScopeDomainUnresolved(_)
        ),
        "实际: {error:?}"
    );
    assert!(
        error
            .to_string()
            .starts_with("work_item_scope_domain_unresolved"),
        "稳定码前缀，实际: {error}"
    );
}

#[test]
fn scope_member_prefix_resolves_member_dir_relative_to_provider_context_root() {
    let fx = lc_gate_fixture(&["busi/**"], &[], true, true);
    assert_eq!(
        fx.engine
            .scope_member_prefix(&fx.attempt)
            .expect("prefix resolves"),
        Some("busi".to_string())
    );
}

#[test]
fn scope_member_prefix_fails_closed_when_member_is_outside_provider_context_root() {
    let fx = lc_gate_fixture(&["busi/**"], &[], false, true);
    let error = fx
        .engine
        .scope_member_prefix(&fx.attempt)
        .expect_err("成员不在聚合根下必须 fail-closed");
    assert!(matches!(
        error,
        CodingWorkspaceEngineError::WorkItemScopeDomainUnresolved(_)
    ));
}

#[test]
fn scope_member_prefix_is_none_for_single_repo_attempt() {
    let fx = lc_gate_fixture(&["busi/**"], &[], true, true);
    let mut legacy = fx.attempt.clone();
    legacy.target_snapshot = None;
    assert_eq!(
        fx.engine
            .scope_member_prefix(&legacy)
            .expect("legacy attempt has no prefix"),
        None
    );
}

#[test]
fn single_repo_scope_semantics_are_unchanged_without_member_prefix() {
    let root = tempdir().expect("tempdir");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree dir");
    init_test_git_repo(&worktree);
    let worktree_path = worktree.clone();
    fs::create_dir_all(worktree.join("src")).expect("src dir");
    let changed = vec!["src/notifications.ts".to_string()];

    // 仓内相对写法的 scope：照常放行。
    validate_changed_files_in_scope(
        &["src/**".to_string()],
        &[],
        &changed,
        Some(&worktree_path),
        None,
    )
    .expect("repo-relative scope allows");
    // 无前缀时，根相对写法的 scope 与旧行为一致：不放行（不放宽单仓语义）。
    assert!(matches!(
        validate_changed_files_in_scope(
            &["busi/src/**".to_string()],
            &[],
            &changed,
            Some(&worktree_path),
            None,
        ),
        Err(CodingWorkspaceEngineError::WorkItemDiffScopeViolation(_))
    ));
    // 单仓 forbidden 仍按仓内相对生效。
    assert!(matches!(
        validate_changed_files_in_scope(
            &[],
            &["src/**".to_string()],
            &changed,
            Some(&worktree_path),
            None,
        ),
        Err(CodingWorkspaceEngineError::WorkItemDiffScopeViolation(_))
    ));
    // exclusive 为空：不限定，行为不变。
    validate_changed_files_in_scope(&[], &[], &changed, Some(&worktree_path), None)
        .expect("empty exclusive does not constrain");
}

/// Bug1-A 防回归：带引号八进制转义形态（修复前采集侧产出）不参与 scope
/// 命中——不做启发式反转义；若该形态重新出现必须以 violation 暴露。
#[test]
fn scope_gate_rejects_escaped_octal_paths_instead_of_unescaping() {
    let root = tempdir().expect("tempdir");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(worktree.join("cadence/verify")).expect("verify dir");
    init_test_git_repo(&worktree);
    let worktree_path = worktree.clone();
    let escaped = concat!(
        "\"cadence/verify/2026-10-16_\\351\\252\\214\\346\\224\\266",
        "\\346\\212\\245\\345\\221\\212_v1.0.md\""
    );

    assert!(matches!(
        validate_changed_files_in_scope(
            &["cadence/verify/**".to_string()],
            &[],
            &[escaped.to_string()],
            Some(&worktree_path),
            None,
        ),
        Err(CodingWorkspaceEngineError::WorkItemDiffScopeViolation(_))
    ));
}

#[test]
fn scope_gate_keeps_runtime_path_hygiene_even_with_prefix_domain() {
    let root = tempdir().expect("tempdir");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree dir");
    init_test_git_repo(&worktree);
    let worktree_path = worktree.clone();

    // `.git` 下的路径即使 scope 宽到 busi/** 也必须拒绝。
    let error = validate_changed_files_in_scope(
        &["busi/**".to_string()],
        &[],
        &[".git/config".to_string()],
        Some(&worktree_path),
        Some("busi"),
    )
    .expect_err(".git 写入必须拒绝");
    assert!(matches!(
        error,
        CodingWorkspaceEngineError::WorkItemDiffScopeViolation(ref path) if path == ".git/config"
    ));
}

#[test]
fn prefixed_gate_accepts_both_path_domains_and_rejects_other_member_scopes() {
    let root = tempdir().expect("tempdir");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree dir");
    init_test_git_repo(&worktree);
    let worktree_path = worktree.clone();
    fs::create_dir_all(worktree.join("src")).expect("src dir");
    fs::create_dir_all(worktree.join("test")).expect("test dir");
    let changed = vec![
        "src/notifications.ts".to_string(),
        "test/notifications.test.ts".to_string(),
    ];

    // 根相对写法放行。
    validate_changed_files_in_scope(
        &[
            "busi/src/notifications.ts".to_string(),
            "busi/test/**".to_string(),
        ],
        &["api/src/**".to_string()],
        &changed,
        Some(&worktree_path),
        Some("busi"),
    )
    .expect("root-relative scopes allow repo-relative diff");
    // 仓内相对写法同样放行（兼容旧计划）。
    validate_changed_files_in_scope(
        &["src/**".to_string(), "test/**".to_string()],
        &[],
        &changed,
        Some(&worktree_path),
        Some("busi"),
    )
    .expect("repo-relative scopes allow");
    // 他仓前缀的 exclusive 不会授权本仓路径。
    assert!(matches!(
        validate_changed_files_in_scope(
            &["api/src/**".to_string()],
            &[],
            &changed,
            Some(&worktree_path),
            Some("busi"),
        ),
        Err(CodingWorkspaceEngineError::WorkItemDiffScopeViolation(_))
    ));
}
