//! REQ-COD-06 适配核查（multi-repo-group-coding WP4）：多 target 拆分增殖
//! 形态下每 target-attempt 独立走交付链（worktree prepare → branch/commit/
//! push → ReviewRequest），A 仓 push 失败不影响 B 仓交付；issue 级 per-WI
//! 交付聚合（`compute_issue_delivery_summary`）经物化 units 覆盖桶内全部
//! WI——partial failure 显式呈现，全交付后完成门（`maybe_complete_issue_delivery`）
//! 把 issue 标 Completed（REQ-COD-06 完成级别）。

use super::*;
use crate::product::coding_attempt_store::CodingGitOperationPhase;
use crate::product::coding_attempt_store::IssueDeliveryOverall;
use crate::product::coding_models::{
    AttemptTargetSnapshot, CodingAttemptStatus, CodingExecutionAttempt, PushStatus,
};
use crate::product::logical_codebase::{LogicalRepositoryId, RepositoryCheckoutId};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use uuid::Uuid;

fn group_target_snapshot(tag: &str) -> AttemptTargetSnapshot {
    AttemptTargetSnapshot {
        logical_repository_id: LogicalRepositoryId(Uuid::new_v4()),
        checkout_id: RepositoryCheckoutId(Uuid::new_v4()),
        physical_repository_id: format!("repo_{tag}"),
        canonical_path: PathBuf::from(format!("/nonexistent/checkout-{tag}")),
        git_dir_identity: format!("git-dir-{tag}"),
        revision: Some(format!("rev-{tag}")),
        policy_digest: "policy-digest".to_string(),
        membership_revision: 1,
        captured_at: "2026-09-19T00:00:00Z".to_string(),
        capture_source: "test".to_string(),
    }
}

/// 按 target 建桶 group attempt（各自冻结快照+OQ2 命名 branch+桶内 units）。
fn seed_target_group_attempt(
    store: &CodingAttemptStore,
    tag: &str,
    base_branch: &str,
    first_work_item_id: &str,
) -> CodingExecutionAttempt {
    let snapshot = group_target_snapshot(tag);
    store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: DELIVERY_PROJECT_ID.to_string(),
            issue_id: DELIVERY_ISSUE_ID.to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: first_work_item_id.to_string(),
            base_branch: base_branch.to_string(),
            branch_name: format!(
                "aria/issues/{DELIVERY_ISSUE_ID}/{}",
                snapshot.logical_repository_id.0
            ),
            worktree_path: None,
            provider_config_snapshot: delivery_provider_snapshot(),
            target_snapshot: Some(snapshot),
            max_auto_rework: 2,
        })
        .expect("group attempt")
}

fn seed_bucket_units(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    bucket_work_items: &[&str],
) {
    for (index, work_item_id) in bucket_work_items.iter().enumerate() {
        store
            .create_coding_unit(CreateCodingExecutionUnitInput {
                attempt_id: attempt.id.clone(),
                project_id: DELIVERY_PROJECT_ID.to_string(),
                issue_id: DELIVERY_ISSUE_ID.to_string(),
                plan_id: "work_item_plan_0001".to_string(),
                logical_work_item_id: work_item_id.to_string(),
                work_item_revision_id: format!("revision_{work_item_id}"),
                dependency_logical_work_item_ids: Vec::new(),
                order_index: index as u32,
                status: CodingExecutionUnitStatus::Completed,
            })
            .expect("coding unit");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn multi_target_delivery_chains_are_independent_per_repo() {
    let root = tempdir().expect("root");
    let repo_api = root.path().join("repo_api");
    let remote_api = root.path().join("remote_api.git");
    let repo_web = root.path().join("repo_web");
    let remote_web = root.path().join("remote_web.git");
    for dir in [&repo_api, &remote_api, &repo_web, &remote_web] {
        fs::create_dir_all(dir).expect("dir");
    }
    init_test_git_repo(&repo_api);
    init_test_git_repo(&repo_web);
    run_test_git(&remote_api, &["init", "--bare"]);
    run_test_git(&remote_web, &["init", "--bare"]);
    // api 远端 pre-receive hook 拒绝 push（首交必失败）；web 远端干净。
    let hook = remote_api.join("hooks/pre-receive");
    fs::write(&hook, "#!/bin/sh\nexit 1\n").expect("rejecting hook");
    let mut permissions = fs::metadata(&hook).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&hook, permissions).expect("hook executable");
    run_test_git(
        &repo_api,
        &["remote", "add", "origin", &remote_api.to_string_lossy()],
    );
    run_test_git(
        &repo_web,
        &["remote", "add", "origin", &remote_web.to_string_lossy()],
    );
    let base_api = git_stdout(&repo_api, &["branch", "--show-current"])
        .trim()
        .to_string();
    let base_web = git_stdout(&repo_web, &["branch", "--show-current"])
        .trim()
        .to_string();

    let paths = ProductAppPaths::new(root.path().join(".aria"));
    IssueStore::new(paths.clone())
        .create(CreateProductIssueInput {
            project_id: DELIVERY_PROJECT_ID.to_string(),
            repo_id: Some("repository_0001".to_string()),
            logical_codebase_id: None,
            title: "multi-target delivery".to_string(),
            description: None,
            change_id: None,
        })
        .expect("issue");
    let store = CodingAttemptStore::new(paths);
    seed_delivery_work_item(&store, "work_item_0001", "repo_api");
    seed_delivery_work_item(&store, "work_item_0002", "repo_api");
    seed_delivery_work_item(&store, "work_item_0003", "repo_web");

    // 多 target 拆分增殖形态：api 桶 [w1, w2]、web 桶 [w3]——两
    // target-attempt 各持冻结快照（per-(plan,target) 唯一性）+各自仓基分支。
    let attempt_api = seed_target_group_attempt(&store, "api", &base_api, "work_item_0001");
    seed_bucket_units(&store, &attempt_api, &["work_item_0001", "work_item_0002"]);
    let attempt_web = seed_target_group_attempt(&store, "web", &base_web, "work_item_0003");
    seed_bucket_units(&store, &attempt_web, &["work_item_0003"]);

    let (prepare_tx, _prepare_rx) = tokio::sync::mpsc::channel(8);
    let prepare_engine =
        CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), prepare_tx);
    let prepared_api = prepare_engine
        .execute_worktree_prepare(&attempt_api, &repo_api)
        .await
        .expect("prepare api worktree");
    let prepared_web = prepare_engine
        .execute_worktree_prepare(&attempt_web, &repo_web)
        .await
        .expect("prepare web worktree");
    assert_ne!(
        prepared_api.worktree_path, prepared_web.worktree_path,
        "per-target worktrees must be distinct"
    );
    fs::write(
        prepared_api
            .worktree_path
            .as_ref()
            .unwrap()
            .join("api-change.txt"),
        "api change\n",
    )
    .expect("api change");
    fs::write(
        prepared_web
            .worktree_path
            .as_ref()
            .unwrap()
            .join("web-change.txt"),
        "web change\n",
    )
    .expect("web change");

    // 三轮 execute_review_request（首交×2+重推）累计事件数超出 channel 容量，
    // 起消费任务防 send 背压阻塞（单线程 runtime 下 tokio::spawn 即可）。
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(8);
    let event_drain = tokio::spawn(async move { while event_rx.recv().await.is_some() {} });
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);

    // A 仓（api）首交：远端 hook 拒绝 → push 失败，交付链完成态 Completed(Failed)。
    let failed_api = engine
        .execute_review_request(&prepared_api, "origin", "feat: api")
        .await
        .expect("api review request");
    assert_eq!(failed_api.push_status, PushStatus::Failed);
    assert!(failed_api.push_error.is_some());

    // B 仓（web）交付链不受 A 失败影响：push 成功。
    let pushed_web = engine
        .execute_review_request(&prepared_web, "origin", "feat: web")
        .await
        .expect("web review request");
    assert_eq!(pushed_web.push_status, PushStatus::Pushed);
    assert_eq!(pushed_web.push_error, None);

    // journal 与 ReviewRequest 每 attempt 独立分区（互不串扰）。
    let journal_api = store
        .get_coding_git_operation(&prepared_api)
        .expect("api journal")
        .expect("api journal persisted");
    assert_eq!(journal_api.phase, CodingGitOperationPhase::Completed);
    assert_eq!(journal_api.push_status, Some(PushStatus::Failed));
    let journal_web = store
        .get_coding_git_operation(&prepared_web)
        .expect("web journal")
        .expect("web journal persisted");
    assert_eq!(journal_web.phase, CodingGitOperationPhase::Completed);
    assert_eq!(journal_web.push_status, Some(PushStatus::Pushed));
    assert_ne!(journal_api.branch_name, journal_web.branch_name);
    assert_eq!(
        store
            .list_review_requests(DELIVERY_PROJECT_ID, DELIVERY_ISSUE_ID, &prepared_api.id)
            .expect("api review requests")
            .len(),
        1
    );
    assert_eq!(
        store
            .list_review_requests(DELIVERY_PROJECT_ID, DELIVERY_ISSUE_ID, &prepared_web.id)
            .expect("web review requests")
            .len(),
        1
    );

    // issue 级 per-WI 交付聚合：partial failure 显式（api 桶两 WI 承载失败态、
    // web 桶 WI 不受影响——非首个 WI 经 units 覆盖取桶 attempt 状态）。
    let summary = store
        .compute_issue_delivery_summary(DELIVERY_PROJECT_ID, DELIVERY_ISSUE_ID)
        .expect("delivery summary");
    assert_eq!(summary.overall, IssueDeliveryOverall::Partial);
    for work_item_id in ["work_item_0001", "work_item_0002"] {
        let entry = summary
            .entries
            .iter()
            .find(|entry| entry.work_item_id == work_item_id)
            .expect("api bucket entry");
        assert_eq!(
            entry.branch_name.as_deref(),
            Some(attempt_api.branch_name.as_str())
        );
        assert_eq!(entry.push_status, Some(PushStatus::Failed));
        assert!(entry.push_error.is_some());
    }
    let web_entry = summary
        .entries
        .iter()
        .find(|entry| entry.work_item_id == "work_item_0003")
        .expect("web bucket entry");
    assert_eq!(
        web_entry.branch_name.as_deref(),
        Some(attempt_web.branch_name.as_str())
    );
    assert_eq!(web_entry.push_status, Some(PushStatus::Pushed));

    // A 仓重推（修复远端后）：重开 Completed(Failed) → Pushed。
    fs::remove_file(&hook).expect("remove rejecting hook");
    let retried_api = engine
        .execute_review_request(&prepared_api, "origin", "feat: api")
        .await
        .expect("api retry review request");
    assert_eq!(retried_api.push_status, PushStatus::Pushed);

    // 完成级别（REQ-COD-06）：两 target-attempt 均 Completed 后 AllPushed →
    for attempt_id in [&attempt_api.id, &attempt_web.id] {
        let mut attempt = store
            .get_attempt(DELIVERY_PROJECT_ID, DELIVERY_ISSUE_ID, attempt_id)
            .expect("load attempt");
        attempt.status = CodingAttemptStatus::Completed;
        store
            .write_coding_attempt_for_test(&attempt)
            .expect("complete attempt");
    }
    let summary = store
        .compute_issue_delivery_summary(DELIVERY_PROJECT_ID, DELIVERY_ISSUE_ID)
        .expect("delivery summary after retry");
    assert_eq!(summary.overall, IssueDeliveryOverall::AllPushed);
    engine
        .maybe_complete_issue_delivery(&prepared_web)
        .expect("complete issue delivery");
    let issue = IssueStore::new(store.paths())
        .get(DELIVERY_PROJECT_ID, DELIVERY_ISSUE_ID)
        .expect("issue");
    assert_eq!(issue.status, IssueStatus::Completed);
    event_drain.abort();
}
