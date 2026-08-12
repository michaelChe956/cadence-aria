// 三元键 shared worktree（Task 9）：多仓 CRUD + 仓维锁 + 单仓红线。
// 由 `tests.rs` 以 `include!` 引入（与 `coding_attempt_store/tests/` 同模式），
// 共享 tests.rs 模块作用域内的 `setup()` / `PROJECT_ID` / `API_MEMBER` 等常量与导入。
// 多仓 issue 每个目标仓独立 worktree，键域 (project, issue, logical_repository_id)；
// 单仓老路径 issue-shared-worktree.json 完全不变（红线）。复用 Task 7 稳定 UUID 常量。

fn repo_worktree_input(repository_id: LogicalRepositoryId) -> UpsertRepoSharedWorktreeInput {
    UpsertRepoSharedWorktreeInput {
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        repository_id,
        branch_name: format!("worktree-{}", repository_id.0),
        worktree_path: std::path::PathBuf::from(format!("/tmp/aria-worktree-{}", repository_id.0)),
        base_branch: "main".to_string(),
    }
}

/// 多仓 CRUD：upsert 填 target_repository_id=Some(repository_id)、repository_id 存 UUID 字符串；
/// get/list 回读一致；upsert 幂等更新分支名；delete 后 get/list 清空。
#[test]
fn repo_shared_worktree_crud_upsert_get_list_delete_uses_target_repository_id() {
    let (_tmp, store) = setup();

    let created = store
        .upsert_repo_shared_worktree(repo_worktree_input(API_MEMBER))
        .expect("upsert repo shared worktree");
    assert_eq!(created.target_repository_id, Some(API_MEMBER));
    assert_eq!(created.repository_id, API_MEMBER.0.to_string());
    assert_eq!(created.branch_name, format!("worktree-{}", API_MEMBER.0));

    let loaded = store
        .get_repo_shared_worktree(PROJECT_ID, ISSUE_ID, API_MEMBER)
        .expect("get repo shared worktree")
        .expect("repo shared worktree exists");
    assert_eq!(loaded, created);
    assert_eq!(loaded.target_repository_id, Some(API_MEMBER));

    assert_eq!(
        store
            .list_repo_shared_worktrees(PROJECT_ID, ISSUE_ID)
            .expect("list repo shared worktrees"),
        vec![API_MEMBER]
    );

    let mut update = repo_worktree_input(API_MEMBER);
    update.branch_name = "worktree-api-v2".to_string();
    let updated = store
        .upsert_repo_shared_worktree(update)
        .expect("upsert repo shared worktree again");
    assert_eq!(updated.branch_name, "worktree-api-v2");
    assert_eq!(updated.id, created.id, "upsert 应原地更新而非新建记录");

    let reloaded = store
        .get_repo_shared_worktree(PROJECT_ID, ISSUE_ID, API_MEMBER)
        .expect("get repo shared worktree after update")
        .expect("repo shared worktree still exists");
    assert_eq!(reloaded.branch_name, "worktree-api-v2");
    assert_eq!(
        store
            .list_repo_shared_worktrees(PROJECT_ID, ISSUE_ID)
            .expect("list after update"),
        vec![API_MEMBER],
        "upsert 不得复制同一仓库的 shared worktree 记录"
    );

    store
        .delete_repo_shared_worktree(PROJECT_ID, ISSUE_ID, API_MEMBER)
        .expect("delete repo shared worktree");
    assert_eq!(
        store
            .get_repo_shared_worktree(PROJECT_ID, ISSUE_ID, API_MEMBER)
            .expect("get after delete"),
        None
    );
    assert!(
        store
            .list_repo_shared_worktrees(PROJECT_ID, ISSUE_ID)
            .expect("list after delete")
            .is_empty()
    );
}

/// delete 幂等：被删对象不存在时视为成功。
#[test]
fn delete_repo_shared_worktree_succeeds_when_absent() {
    let (_tmp, store) = setup();
    store
        .delete_repo_shared_worktree(PROJECT_ID, ISSUE_ID, API_MEMBER)
        .expect("delete absent repo shared worktree must succeed");
}

/// 仓维锁竞争：同 target 两个并发 acquire，一个获 lease，另一个得到稳定码 repo_worktree_active。
#[test]
fn repo_shared_worktree_concurrent_same_target_one_lease_one_repo_worktree_active() {
    let (_tmp, store) = setup();
    store
        .upsert_repo_shared_worktree(repo_worktree_input(API_MEMBER))
        .expect("seed repo shared worktree");

    let (first, second) = std::thread::scope(|scope| {
        let store = &store;
        let first = scope.spawn(move || {
            store.try_acquire_repo_worktree_lock(
                PROJECT_ID,
                ISSUE_ID,
                API_MEMBER,
                "work_item_0001",
                "lease_0001",
            )
        });
        let second = scope.spawn(move || {
            store.try_acquire_repo_worktree_lock(
                PROJECT_ID,
                ISSUE_ID,
                API_MEMBER,
                "work_item_0002",
                "lease_0002",
            )
        });
        (first.join().unwrap(), second.join().unwrap())
    });

    let (lease, active_error) = match (first, second) {
        (Ok(lease), Err(error)) => (lease, error),
        (Err(error), Ok(lease)) => (lease, error),
        _ => panic!("同 target 并发必须恰好一个成功一个 repo_worktree_active"),
    };
    assert!(lease.acquired, "获胜方必须真正取得 lease");
    assert!(
        matches!(
            &active_error,
            ProductStoreError::Io(message) if message.contains("repo_worktree_active")
        ),
        "仓维锁竞争失败必须返回稳定码 repo_worktree_active，实际: {active_error:?}"
    );
}

/// 异仓并行：不同 repository_id 各自独立，同一 work_item_id 在异仓可同时取得 lease。
#[test]
fn repo_shared_worktree_distinct_targets_acquire_independently() {
    let (_tmp, store) = setup();
    store
        .upsert_repo_shared_worktree(repo_worktree_input(API_MEMBER))
        .expect("seed api repo worktree");
    store
        .upsert_repo_shared_worktree(repo_worktree_input(WEB_MEMBER))
        .expect("seed web repo worktree");

    let api_lease = store
        .try_acquire_repo_worktree_lock(
            PROJECT_ID,
            ISSUE_ID,
            API_MEMBER,
            "work_item_0001",
            "lease_api",
        )
        .expect("api repo acquires lease");
    let web_lease = store
        .try_acquire_repo_worktree_lock(
            PROJECT_ID,
            ISSUE_ID,
            WEB_MEMBER,
            "work_item_0001",
            "lease_web",
        )
        .expect("web repo acquires lease in parallel");

    assert!(api_lease.acquired);
    assert!(web_lease.acquired);
    assert_eq!(api_lease.worktree.target_repository_id, Some(API_MEMBER));
    assert_eq!(web_lease.worktree.target_repository_id, Some(WEB_MEMBER));

    assert_eq!(
        store
            .list_repo_shared_worktrees(PROJECT_ID, ISSUE_ID)
            .expect("list both repo worktrees"),
        vec![API_MEMBER, WEB_MEMBER]
    );
}

/// 释放/校验/标记完成：release 清锁、validate 校验 owner、mark 记录完成并释放当前项。
#[test]
fn repo_shared_worktree_release_validate_mark_roundtrip() {
    let (_tmp, store) = setup();
    store
        .upsert_repo_shared_worktree(repo_worktree_input(API_MEMBER))
        .expect("seed repo shared worktree");

    let lease = store
        .try_acquire_repo_worktree_lock(
            PROJECT_ID,
            ISSUE_ID,
            API_MEMBER,
            "work_item_0001",
            "lease_owner",
        )
        .expect("acquire lease");
    assert!(lease.acquired);

    store
        .validate_repo_worktree_lock_owner(
            PROJECT_ID,
            ISSUE_ID,
            API_MEMBER,
            "work_item_0001",
            "lease_owner",
        )
        .expect("owner validation passes");

    let wrong_owner = store
        .validate_repo_worktree_lock_owner(
            PROJECT_ID,
            ISSUE_ID,
            API_MEMBER,
            "work_item_0001",
            "someone_else",
        )
        .unwrap_err();
    assert!(
        matches!(
            wrong_owner,
            ProductStoreError::Conflict {
                kind: "repo_worktree_lock_owner",
                ..
            }
        ),
        "错误 owner 应返回 repo_worktree_lock_owner 冲突，实际: {wrong_owner:?}"
    );

    let marked = store
        .mark_repo_worktree_completed_item(
            PROJECT_ID,
            ISSUE_ID,
            API_MEMBER,
            "work_item_0001",
            "lease_owner",
        )
        .expect("mark completed item");
    assert_eq!(
        marked.last_completed_work_item_id.as_deref(),
        Some("work_item_0001")
    );
    assert_eq!(marked.current_active_work_item_id, None);
    assert_eq!(marked.current_lock_owner_id, None);

    let released = store
        .release_repo_worktree_lock(
            PROJECT_ID,
            ISSUE_ID,
            API_MEMBER,
            "work_item_0001",
            "lease_owner",
        )
        .expect("release lock");
    assert_eq!(released.current_active_work_item_id, None);
    assert_eq!(released.current_lock_owner_id, None);
}

/// 单仓红线：老 issue 级方法与多仓方法并列共存，互不干扰；
/// 多仓 upsert 写 shared-worktrees/{id}.json，不得覆盖 issue-shared-worktree.json。
#[test]
fn legacy_issue_shared_worktree_coexists_with_repo_shared_worktree() {
    let (_tmp, store) = setup();
    store
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            branch_name: "legacy-worktree".to_string(),
            worktree_path: std::path::PathBuf::from("/tmp/legacy-worktree"),
            base_branch: "main".to_string(),
        })
        .expect("legacy issue worktree upsert");
    store
        .upsert_repo_shared_worktree(repo_worktree_input(API_MEMBER))
        .expect("repo worktree upsert");

    let legacy = store
        .get_issue_shared_worktree(PROJECT_ID, ISSUE_ID)
        .expect("legacy get")
        .expect("legacy record exists");
    assert_eq!(legacy.branch_name, "legacy-worktree");
    assert_eq!(
        legacy.target_repository_id, None,
        "单仓老方法 target 恒为 None"
    );

    let repo = store
        .get_repo_shared_worktree(PROJECT_ID, ISSUE_ID, API_MEMBER)
        .expect("repo get")
        .expect("repo record exists");
    assert_eq!(repo.target_repository_id, Some(API_MEMBER));

    store
        .delete_repo_shared_worktree(PROJECT_ID, ISSUE_ID, API_MEMBER)
        .expect("delete repo worktree");
    assert!(
        store
            .get_issue_shared_worktree(PROJECT_ID, ISSUE_ID)
            .expect("legacy get after repo delete")
            .is_some(),
        "删除多仓记录不得影响单仓老记录"
    );
}
