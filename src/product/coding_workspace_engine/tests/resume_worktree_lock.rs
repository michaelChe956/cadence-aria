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
use crate::product::lifecycle_store::UpsertIssueSharedWorktreeInput;

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_0001";
const WORK_ITEM_ID: &str = "work_item_0001";
const OTHER_WORK_ITEM_ID: &str = "work_item_0002";

struct ResumeLockFixture {
    _root: tempfile::TempDir,
    lifecycle: LifecycleStore,
    store: CodingAttemptStore,
    attempt: CodingExecutionAttempt,
}

fn seed_running_attempt_with_bound_lock() -> ResumeLockFixture {
    let root = tempdir().expect("tempdir");
    let worktree = root.path().join("shared-worktree");
    std::fs::create_dir_all(&worktree).expect("shared worktree dir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: worktree.clone(),
            base_branch: "HEAD".to_string(),
        })
        .expect("shared worktree");
    lifecycle
        .try_acquire_issue_worktree_lock(
            PROJECT_ID,
            ISSUE_ID,
            WORK_ITEM_ID,
            "issue_worktree_lease_seed",
        )
        .expect("seed lease");
    let store = CodingAttemptStore::new(paths);
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: WORK_ITEM_ID.to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("attempt");
    lifecycle
        .bind_issue_worktree_lock_to_attempt(PROJECT_ID, ISSUE_ID, WORK_ITEM_ID, &attempt.id)
        .expect("bind owner");
    let attempt = store
        .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &attempt.id)
        .expect("running attempt");
    ResumeLockFixture {
        _root: root,
        lifecycle,
        store,
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
        .get_issue_shared_worktree(PROJECT_ID, ISSUE_ID)
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
        .release_issue_worktree_lock_by_owner(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
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
            kind: "issue_worktree_lock_owner",
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
        .release_issue_worktree_lock_by_owner(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
        .expect("clear owner");
    fixture
        .lifecycle
        .try_acquire_issue_worktree_lock(
            PROJECT_ID,
            ISSUE_ID,
            WORK_ITEM_ID,
            "issue_worktree_lease_transient_residue",
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
        .release_issue_worktree_lock_by_owner(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
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
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("other attempt");
    fixture
        .lifecycle
        .try_acquire_issue_worktree_lock(
            PROJECT_ID,
            ISSUE_ID,
            OTHER_WORK_ITEM_ID,
            "issue_worktree_lease_b",
        )
        .expect("foreign lease");
    fixture
        .lifecycle
        .bind_issue_worktree_lock_to_attempt(PROJECT_ID, ISSUE_ID, OTHER_WORK_ITEM_ID, &other.id)
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
        .release_issue_worktree_lock_by_owner(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
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
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("other attempt");
    fixture
        .lifecycle
        .try_acquire_issue_worktree_lock(
            PROJECT_ID,
            ISSUE_ID,
            OTHER_WORK_ITEM_ID,
            "issue_worktree_lease_b",
        )
        .expect("foreign lease");
    fixture
        .lifecycle
        .bind_issue_worktree_lock_to_attempt(PROJECT_ID, ISSUE_ID, OTHER_WORK_ITEM_ID, &other.id)
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
fn resume_without_worktree_record_is_noop_for_legacy_attempt() {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
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
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("attempt");
    let attempt = store
        .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &attempt.id)
        .expect("running");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    // 无租约事实（未启锁的 legacy attempt）：不拦截（C2 同语义）。
    engine
        .ensure_issue_worktree_lock_for_resumed_attempt(&attempt)
        .expect("legacy attempt without lock record");
}
