//! G5（终局关闸缺口）：RecoverCoding／RestartCoding 恢复链的工作树锁复位。
//!
//! 现场（issue_0004/WI-001）：runner 死亡 → 确认接管显式释放 WI 级共享
//! 工作树锁 → RecoverCoding 把 attempt CAS 回 Running 并重启 runner →
//! 编码段入口校验 `validate_attempt_issue_shared_worktree_lock_if_present`
//! 以 `issue_worktree_lock_owner` 冲突死亡 → 回 AwaitingManualRecovery，
//! 「恢复即失败」死循环；restart 对 AMR 409，无任何产品清理面。
//!
//! 修复语义（复用 C1/C2 租约三态判定，不新建体系）：
//! `ensure_issue_worktree_lock_for_resumed_attempt` 在恢复/重开路径把锁
//! 复位到该 attempt 名下——自持原样继续；锁已释放/自持死亡残留/瞬态
//! lease 残留 → 重新获取并绑定；活跃他人停等；死亡他人指向既有确认
//! 接管面；证据未知 fail-closed。

use super::*;
use crate::product::logical_codebase::logical_test_fixture::{self, LogicalFixture};

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_0001";
const WORK_ITEM_ID: &str = "work_item_0001";
const OTHER_WORK_ITEM_ID: &str = "work_item_0002";

struct ResumeLockFixture {
    _root: tempfile::TempDir,
    lifecycle: LifecycleStore,
    store: CodingAttemptStore,
    logical: LogicalFixture,
    attempt: CodingExecutionAttempt,
}

fn seed_running_attempt_with_bound_lock() -> ResumeLockFixture {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    // REQ-ROUTE-01：准入/租约的唯一凭证是 LC 权威 + 冻结快照，且快照 attempt
    // 只按仓维分类租约——故本夹具必须先播种完整 LC 权威，并全程使用仓维记录。
    let logical = logical_test_fixture::seed_complete(&paths, root.path(), PROJECT_ID, ISSUE_ID);
    let lifecycle = LifecycleStore::new(paths.clone());
    let worktree = logical.member_path.clone();
    logical_test_fixture::upsert_repo_worktree(
        &lifecycle,
        PROJECT_ID,
        ISSUE_ID,
        logical.member_id,
        &worktree,
    );
    let store = CodingAttemptStore::new(paths);
    let target_snapshot = logical.snapshot(&store.paths());
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: WORK_ITEM_ID.to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: Some(target_snapshot),
            max_auto_rework: 2,
        })
        .expect("attempt");
    logical_test_fixture::bind_repo_lock_to_attempt(
        &lifecycle,
        PROJECT_ID,
        ISSUE_ID,
        logical.member_id,
        WORK_ITEM_ID,
        &attempt.id,
    );
    let attempt = store
        .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &attempt.id)
        .expect("running attempt");
    ResumeLockFixture {
        _root: root,
        lifecycle,
        store,
        logical,
        attempt,
    }
}

fn engine_for(fixture: &ResumeLockFixture) -> CodingWorkspaceEngine {
    let (tx, _rx) = mpsc::channel(16);
    CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), tx)
}

fn lock_record(fixture: &ResumeLockFixture) -> crate::product::models::IssueSharedWorktree {
    fixture
        .lifecycle
        .get_repo_shared_worktree(PROJECT_ID, ISSUE_ID, fixture.logical.member_id)
        .expect("lock record")
        .expect("lock record present")
}

#[test]
fn resume_after_takeover_release_reacquires_and_rebinds_lock() {
    let fixture = seed_running_attempt_with_bound_lock();
    // runner 死亡 → AwaitingManualRecovery。
    fixture
        .store
        .transition_to_awaiting_manual_recovery(
            &fixture.attempt.id,
            "coding_runner_failed_while_running",
        )
        .expect("manual recovery");
    // 确认接管显式释放锁（A09 takeover 的落盘效果）。
    fixture
        .lifecycle
        .release_repo_worktree_lock_by_owner(
            PROJECT_ID,
            ISSUE_ID,
            fixture.logical.member_id,
            &fixture.attempt.id,
        )
        .expect("takeover released the lock");
    let engine = engine_for(&fixture);

    // 修复前现场复现：锁释放后恢复回 Running，编码段入口校验即冲突。
    let recovered = fixture
        .store
        .recover_attempt_from_manual_recovery(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
        .expect("recover to running");
    let residue = engine
        .validate_attempt_issue_shared_worktree_lock_if_present(&recovered)
        .expect_err("G5 现场：released lock breaks coding start validation");
    assert!(matches!(
        residue,
        CodingWorkspaceEngineError::Store(ProductStoreError::Conflict {
            kind: "repo_worktree_lock_owner",
            ..
        })
    ));

    // 修复：恢复路径复位锁到该 attempt 名下，编码段校验通过。
    engine
        .ensure_issue_worktree_lock_for_resumed_attempt(&recovered)
        .expect("resume lock rebind");
    engine
        .validate_attempt_issue_shared_worktree_lock_if_present(&recovered)
        .expect("coding start validation passes after rebind");
    let record = lock_record(&fixture);
    assert_eq!(
        record.current_active_work_item_id.as_deref(),
        Some(WORK_ITEM_ID)
    );
    assert_eq!(
        record.current_lock_owner_id.as_deref(),
        Some(recovered.id.as_str())
    );
}

#[test]
fn resume_with_self_held_lock_while_awaiting_recovery_keeps_or_rebinds_owner() {
    let fixture = seed_running_attempt_with_bound_lock();
    fixture
        .store
        .transition_to_awaiting_manual_recovery(
            &fixture.attempt.id,
            "coding_runner_failed_while_running",
        )
        .expect("manual recovery");
    let engine = engine_for(&fixture);
    // AMR 非活跃：自持锁按死亡残留判别，释放后重绑（幂等，owner 不变）。
    engine
        .ensure_issue_worktree_lock_for_resumed_attempt(&fixture.attempt)
        .expect("self residue rebind");
    let record = lock_record(&fixture);
    assert_eq!(
        record.current_lock_owner_id.as_deref(),
        Some(fixture.attempt.id.as_str())
    );
    assert_eq!(
        record.current_active_work_item_id.as_deref(),
        Some(WORK_ITEM_ID)
    );
}

#[test]
fn resume_with_transient_lease_residue_adopts_and_rebinds() {
    let fixture = seed_running_attempt_with_bound_lock();
    fixture
        .store
        .transition_to_awaiting_manual_recovery(
            &fixture.attempt.id,
            "coding_runner_failed_while_running",
        )
        .expect("manual recovery");
    // 恢复 spawn 失败遗留的瞬态 lease 残留（active=WI，owner 未绑 attempt）。
    fixture
        .lifecycle
        .release_repo_worktree_lock_by_owner(
            PROJECT_ID,
            ISSUE_ID,
            fixture.logical.member_id,
            &fixture.attempt.id,
        )
        .expect("clear owner");
    fixture
        .lifecycle
        .try_acquire_repo_worktree_lock(
            PROJECT_ID,
            ISSUE_ID,
            fixture.logical.member_id,
            WORK_ITEM_ID,
            "repo_worktree_lease_transient_residue",
        )
        .expect("transient residue");
    let engine = engine_for(&fixture);
    engine
        .ensure_issue_worktree_lock_for_resumed_attempt(&fixture.attempt)
        .expect("transient residue adoption");
    let record = lock_record(&fixture);
    assert_eq!(
        record.current_lock_owner_id.as_deref(),
        Some(fixture.attempt.id.as_str())
    );
    assert_eq!(
        record.current_active_work_item_id.as_deref(),
        Some(WORK_ITEM_ID)
    );
}

#[test]
fn resume_with_foreign_active_holder_stops_waiting_without_preemption() {
    let fixture = seed_running_attempt_with_bound_lock();
    fixture
        .store
        .transition_to_awaiting_manual_recovery(
            &fixture.attempt.id,
            "coding_runner_failed_while_running",
        )
        .expect("manual recovery");
    fixture
        .lifecycle
        .release_repo_worktree_lock_by_owner(
            PROJECT_ID,
            ISSUE_ID,
            fixture.logical.member_id,
            &fixture.attempt.id,
        )
        .expect("clear owner");
    // 同 issue 另一 WI 的活跃 attempt B 持锁。
    let other = fixture
        .store
        .create_attempt(CreateCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: OTHER_WORK_ITEM_ID.to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            // 他人持有场景同样必须是「带快照的合法 attempt」：无快照会走
            // `target_snapshot_missing_for_logical`，测不到「活跃他人停等」本体。
            target_snapshot: Some(fixture.logical.snapshot(&fixture.store.paths())),
            max_auto_rework: 2,
        })
        .expect("other attempt");
    fixture
        .lifecycle
        .try_acquire_repo_worktree_lock(
            PROJECT_ID,
            ISSUE_ID,
            fixture.logical.member_id,
            OTHER_WORK_ITEM_ID,
            "repo_worktree_lease_b",
        )
        .expect("foreign lease");
    fixture
        .lifecycle
        .bind_repo_worktree_lock_to_attempt(
            PROJECT_ID,
            ISSUE_ID,
            fixture.logical.member_id,
            OTHER_WORK_ITEM_ID,
            &other.id,
        )
        .expect("bind foreign");
    let other = fixture
        .store
        .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &other.id)
        .expect("foreign running");

    let engine = engine_for(&fixture);
    let error = engine
        .ensure_issue_worktree_lock_for_resumed_attempt(&fixture.attempt)
        .expect_err("foreign active holder must stop waiting");
    assert!(
        error.to_string().contains("coding_run_already_running"),
        "unexpected error: {error}"
    );
    // 绝不抢占：锁仍归 B。
    let record = lock_record(&fixture);
    assert_eq!(
        record.current_lock_owner_id.as_deref(),
        Some(other.id.as_str())
    );
    assert_eq!(
        record.current_active_work_item_id.as_deref(),
        Some(OTHER_WORK_ITEM_ID)
    );
}

#[test]
fn resume_with_foreign_dead_holder_requires_takeover_surface() {
    let fixture = seed_running_attempt_with_bound_lock();
    fixture
        .store
        .transition_to_awaiting_manual_recovery(
            &fixture.attempt.id,
            "coding_runner_failed_while_running",
        )
        .expect("manual recovery");
    fixture
        .lifecycle
        .release_repo_worktree_lock_by_owner(
            PROJECT_ID,
            ISSUE_ID,
            fixture.logical.member_id,
            &fixture.attempt.id,
        )
        .expect("clear owner");
    let other = fixture
        .store
        .create_attempt(CreateCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: OTHER_WORK_ITEM_ID.to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            // 他人持有场景同样必须是「带快照的合法 attempt」：无快照会走
            // `target_snapshot_missing_for_logical`，测不到「活跃他人停等」本体。
            target_snapshot: Some(fixture.logical.snapshot(&fixture.store.paths())),
            max_auto_rework: 2,
        })
        .expect("other attempt");
    fixture
        .lifecycle
        .try_acquire_repo_worktree_lock(
            PROJECT_ID,
            ISSUE_ID,
            fixture.logical.member_id,
            OTHER_WORK_ITEM_ID,
            "repo_worktree_lease_b",
        )
        .expect("foreign lease");
    fixture
        .lifecycle
        .bind_repo_worktree_lock_to_attempt(
            PROJECT_ID,
            ISSUE_ID,
            fixture.logical.member_id,
            OTHER_WORK_ITEM_ID,
            &other.id,
        )
        .expect("bind foreign");
    // B 终态（Failed）仍占锁：死亡他人（先经合法转换 Running→Failed）。
    fixture
        .store
        .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &other.id)
        .expect("foreign running");
    fixture
        .store
        .update_attempt_status(
            PROJECT_ID,
            ISSUE_ID,
            &other.id,
            crate::product::coding_models::CodingAttemptStatus::Failed,
        )
        .expect("foreign failed");

    let engine = engine_for(&fixture);
    let error = engine
        .ensure_issue_worktree_lock_for_resumed_attempt(&fixture.attempt)
        .expect_err("foreign dead holder must require takeover");
    assert!(
        error.to_string().contains("coding_lease_takeover_required"),
        "unexpected error: {error}"
    );
    // 未确认接管不抢占：锁仍归终态 B，等待项投影走既有 lease_takeover 面。
    let record = lock_record(&fixture);
    assert_eq!(
        record.current_lock_owner_id.as_deref(),
        Some(other.id.as_str())
    );
}

#[test]
fn resume_without_worktree_record_is_noop_for_snapshot_attempt() {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let logical = logical_test_fixture::seed_complete(&paths, root.path(), PROJECT_ID, ISSUE_ID);
    let store = CodingAttemptStore::new(paths.clone());
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: WORK_ITEM_ID.to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: Some(logical.snapshot(&store.paths())),
            max_auto_rework: 2,
        })
        .expect("attempt");
    let attempt = store
        .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &attempt.id)
        .expect("running");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    // 无租约事实（从未启锁的 attempt，仓维记录不存在）：不拦截（C2 同语义）。
    engine
        .ensure_issue_worktree_lock_for_resumed_attempt(&attempt)
        .expect("legacy attempt without lock record");
}

// ─── 多仓热修：恢复锁复位按 attempt 路由收窄 ───
//
// 事故推演（WI-001 AMR 恢复死循环）：issue 级 classify 扫到兄弟仓
// 预绑定锁（活跃异主）→ `coding_run_already_running` → RecoverCoding
// 必然失败。修复后 scoped 分类只看本 attempt 路由的仓：自持 AMR 死亡
// 残留 → 释放+重绑本仓，兄弟仓锁事实原样保留。

fn multi_repo_snapshot_for(
    repo: crate::product::logical_codebase::LogicalRepositoryId,
    suffix: &str,
) -> crate::product::coding_models::AttemptTargetSnapshot {
    crate::product::coding_models::AttemptTargetSnapshot {
        logical_repository_id: repo,
        checkout_id: crate::product::logical_codebase::RepositoryCheckoutId(uuid::Uuid::new_v4()),
        physical_repository_id: format!("repository_{suffix}"),
        canonical_path: std::path::PathBuf::from(format!("/tmp/repository_{suffix}")),
        git_dir_identity: format!("sha256:{suffix}"),
        revision: Some("abcdef".to_string()),
        policy_digest: "policy_digest".to_string(),
        membership_revision: 1,
        captured_at: "2026-10-14T00:00:00Z".to_string(),
        capture_source: "test".to_string(),
    }
}

#[test]
fn resume_multi_repo_amr_self_held_residue_rebinds_own_repo_and_keeps_sibling_lock() {
    use crate::product::coding_models::CodingAttemptStatus;
    use crate::product::lifecycle_store::UpsertRepoSharedWorktreeInput;
    use crate::product::logical_codebase::{
        IssueCodebaseSelection, IssueCodebaseSelectionStore, LogicalCodebaseManifest,
        LogicalCodebaseStore, LogicalRepositoryId,
    };
    use uuid::Uuid;

    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    // sibling UUID 恒小于 own：旧 issue 级 classify 按 UUID 升序必然先取
    // 兄弟仓预绑定锁（事故形态）。
    let own_repo = LogicalRepositoryId(Uuid::from_u128(2));
    let sibling_repo = LogicalRepositoryId(Uuid::from_u128(1));
    let authority = LogicalCodebaseStore::new(paths.clone());
    let manifest = LogicalCodebaseManifest::new(
        PROJECT_ID,
        root.path().join("aggregate-root"),
        vec![own_repo, sibling_repo],
    );
    authority
        .save_manifest(PROJECT_ID, &manifest)
        .expect("manifest");
    IssueCodebaseSelectionStore::new(paths.clone())
        .save(&IssueCodebaseSelection::explicit(
            PROJECT_ID,
            ISSUE_ID,
            vec![own_repo, sibling_repo],
            Vec::new(),
            vec![own_repo, sibling_repo],
            None,
        ))
        .expect("selection");
    let lifecycle = LifecycleStore::new(paths.clone());
    for (repo, name) in [
        (own_repo, "own-worktree"),
        (sibling_repo, "sibling-worktree"),
    ] {
        let worktree = root.path().join(name);
        fs::create_dir_all(&worktree).expect("worktree dir");
        lifecycle
            .upsert_repo_shared_worktree(UpsertRepoSharedWorktreeInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                repository_id: repo,
                branch_name: "main".to_string(),
                worktree_path: worktree,
                base_branch: "main".to_string(),
            })
            .expect("repo shared worktree");
    }
    let store = CodingAttemptStore::new(paths.clone());
    let provider = ProviderConfigSnapshot {
        author: ProviderName::Codex,
        reviewer: Some(ProviderName::ClaudeCode),
        review_rounds: 1,
        permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
    };
    let own = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: WORK_ITEM_ID.to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001/own".to_string(),
            worktree_path: None,
            provider_config_snapshot: provider.clone(),
            target_snapshot: Some(multi_repo_snapshot_for(own_repo, "0001")),
            max_auto_rework: 2,
        })
        .expect("own attempt");
    let sibling = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: OTHER_WORK_ITEM_ID.to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001/sibling".to_string(),
            worktree_path: None,
            provider_config_snapshot: provider,
            target_snapshot: Some(multi_repo_snapshot_for(sibling_repo, "0002")),
            max_auto_rework: 2,
        })
        .expect("sibling attempt");
    // own 仓自持（首项合法自绑）；sibling 仓预绑定到未启动的兄弟 attempt
    //（Created 亦是活跃态）。
    for (repo, work_item_id, attempt) in [
        (own_repo, WORK_ITEM_ID, &own),
        (sibling_repo, OTHER_WORK_ITEM_ID, &sibling),
    ] {
        let lease = lifecycle
            .try_acquire_repo_worktree_lock(
                PROJECT_ID,
                ISSUE_ID,
                repo,
                work_item_id,
                &format!("repo_worktree_lease_{}", attempt.id),
            )
            .expect("acquire repo lease");
        assert!(lease.acquired);
        lifecycle
            .bind_repo_worktree_lock_to_attempt(
                PROJECT_ID,
                ISSUE_ID,
                repo,
                work_item_id,
                &attempt.id,
            )
            .expect("bind repo lease");
    }
    // 事故收尾形态：own runner 死亡回 AMR，锁未释放（自持死亡残留）。
    let mut amr = store
        .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &own.id)
        .expect("running own");
    amr.status = CodingAttemptStatus::AwaitingManualRecovery;
    amr.manual_recovery_reason = Some("coding_runner_failed_while_running".to_string());
    store.write_coding_attempt_for_test(&amr).expect("amr own");

    // 恢复锁复位必须只看 own 仓：自持残留释放+重绑成功，不再被兄弟仓
    // 预绑定锁误判为 already-running。
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    engine
        .ensure_issue_worktree_lock_for_resumed_attempt(&amr)
        .expect("multi-repo AMR self-held residue must rebind own repo lock");

    // own 仓：重绑回本 attempt。
    let own_record = lifecycle
        .get_repo_shared_worktree(PROJECT_ID, ISSUE_ID, own_repo)
        .expect("own record")
        .expect("own record present");
    assert_eq!(
        own_record.current_lock_owner_id.as_deref(),
        Some(own.id.as_str())
    );
    assert_eq!(
        own_record.current_active_work_item_id.as_deref(),
        Some(WORK_ITEM_ID)
    );
    // 兄弟仓锁事实原样保留。
    let sibling_record = lifecycle
        .get_repo_shared_worktree(PROJECT_ID, ISSUE_ID, sibling_repo)
        .expect("sibling record")
        .expect("sibling record present");
    assert_eq!(
        sibling_record.current_lock_owner_id.as_deref(),
        Some(sibling.id.as_str())
    );
    assert_eq!(
        sibling_record.current_active_work_item_id.as_deref(),
        Some(OTHER_WORK_ITEM_ID)
    );
}
