use std::collections::BTreeSet;
use std::sync::{Arc, Barrier};
use std::time::Duration;

use super::*;
use crate::product::coding_attempt_store::locking::{
    ExclusiveFileLock, register_lock_attempt_hook,
};
use crate::product::json_store::ProductStoreError;

#[test]
fn single_work_item_attempt_creation_is_serialized_across_store_instances() {
    let (tmp, first_store) = setup_store();
    let second_store = CodingAttemptStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    let lock_target = first_store
        .coding_attempts_root(PROJECT_ID, ISSUE_ID)
        .join("work-item-attempt-locks")
        .join(WORK_ITEM_ID);
    let held_lock = ExclusiveFileLock::acquire(&lock_target).expect("hold attempt creation lock");
    let (_hook, lock_attempts) = register_lock_attempt_hook(&lock_target);
    let start = Arc::new(Barrier::new(3));

    let spawn_create = |store: CodingAttemptStore, branch_name: &'static str| {
        let start = Arc::clone(&start);
        std::thread::spawn(move || {
            start.wait();
            store.create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: WORK_ITEM_ID.to_string(),
                base_branch: "main".to_string(),
                branch_name: branch_name.to_string(),
                worktree_path: None,
                provider_config_snapshot: provider_snapshot(),
                target_snapshot: None,
                max_auto_rework: 2,
            })
        })
    };
    let first = spawn_create(
        first_store.clone(),
        "aria/work-items/work_item_0001/attempt-a",
    );
    let second = spawn_create(
        second_store.clone(),
        "aria/work-items/work_item_0001/attempt-b",
    );
    start.wait();

    lock_attempts
        .recv_timeout(Duration::from_secs(2))
        .expect("first store must contend on the cross-process lock");
    lock_attempts
        .recv_timeout(Duration::from_secs(2))
        .expect("second store must contend on the same cross-process lock");
    drop(held_lock);

    let results = [
        first.join().expect("first create thread"),
        second.join().expect("second create thread"),
    ];
    let attempts = results
        .iter()
        .filter_map(|result| result.as_ref().ok())
        .collect::<Vec<_>>();
    let conflicts = results
        .iter()
        .filter_map(|result| result.as_ref().err())
        .collect::<Vec<_>>();
    assert_eq!(
        attempts.len(),
        1,
        "at most one active attempt may be created"
    );
    assert_eq!(
        conflicts.len(),
        1,
        "the losing request must return a typed conflict"
    );
    assert!(matches!(
        conflicts[0],
        ProductStoreError::Conflict {
            kind: "active_coding_attempt",
            id,
        } if id == &attempts[0].id
    ));

    let persisted = first_store
        .list_attempts_for_work_item(PROJECT_ID, ISSUE_ID, WORK_ITEM_ID)
        .expect("persisted attempts");
    assert_eq!(persisted.len(), 1);
    assert_eq!(persisted[0].attempt_no, 1);
    assert_eq!(
        persisted
            .iter()
            .map(|attempt| attempt.attempt_no)
            .collect::<BTreeSet<_>>()
            .len(),
        persisted.len()
    );
    assert!(
        first_store
            .role_provider_config_path(PROJECT_ID, ISSUE_ID, &persisted[0].id)
            .is_file()
    );
    let provider_configs =
        std::fs::read_dir(first_store.coding_attempts_root(PROJECT_ID, ISSUE_ID))
            .expect("coding attempts root")
            .filter_map(Result::ok)
            .filter(|entry| entry.path().join("role-provider-config.json").is_file())
            .count();
    assert_eq!(
        provider_configs, 1,
        "losing request must not leave provider config"
    );
}

#[test]
fn work_item_creation_guard_rejects_same_ids_from_different_store_root() {
    let (_first_tmp, first_store) = setup_store();
    let (_second_tmp, second_store) = setup_store();
    let guard = first_store
        .acquire_work_item_attempt_creation(PROJECT_ID, ISSUE_ID, WORK_ITEM_ID)
        .expect("first root creation guard");

    let error = second_store
        .create_attempt_with_guard(
            CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: WORK_ITEM_ID.to_string(),
                base_branch: "main".to_string(),
                branch_name: "aria/work-items/work_item_0001/cross-root".to_string(),
                worktree_path: None,
                provider_config_snapshot: provider_snapshot(),
                target_snapshot: None,
                max_auto_rework: 2,
            },
            &guard,
        )
        .expect_err("a guard from another Store root must be rejected");

    assert!(matches!(
        error,
        ProductStoreError::IdentityMismatch {
            kind: "work_item_attempt_creation_guard",
            ..
        }
    ));
    assert!(
        second_store
            .list_attempts_for_work_item(PROJECT_ID, ISSUE_ID, WORK_ITEM_ID)
            .expect("second root attempts")
            .is_empty()
    );
}

// ─── fix-cascade-deletion Task D2.1c：coding attempt 创建与删除门禁不竞态 ───
//
// REQ-DEL-05/06：创建入口在原 creation/arbitration/object 锁外获得 owning
// Plan scope 共享准入并持有到 attempt 持久化——创建在途时删除独占立即
// entity_busy（零墓碑零破坏）；删除先立墓碑后 direct/group/journal 恢复
// 创建一律稳定拒绝、attempt 数 0；其他 Plan 创建不受影响；guard 已取旧
// arbitration 锁路径也不得绕墓碑。
#[test]
fn cascade_d2_1c_coding_create_delete_race() {
    use crate::product::entity_deletion_store::{
        DeletionEntityKind, DeletionPhase, EntityDeletionStore, EntityScope,
    };
    use crate::product::entity_operation_store::{EntityOperationError, EntityOperationStore};
    use crate::product::lifecycle_store::{CreateWorkItemInput, LifecycleStore};

    fn conflict_kind(error: &ProductStoreError) -> &str {
        match error {
            ProductStoreError::Conflict { kind, .. } => kind,
            other => panic!("expected Conflict, got: {other}"),
        }
    }

    fn work_item_input(id: &str, plan_id: &str) -> CreateWorkItemInput {
        CreateWorkItemInput {
            id: Some(id.to_string()),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: "repository_main".to_string(),
            title: format!("work item {id}"),
            source_work_item_plan_id: Some(plan_id.to_string()),
            ..CreateWorkItemInput::default()
        }
    }

    fn direct_input(
        work_item_id: &str,
        target: Option<LogicalRepositoryId>,
    ) -> CreateCodingAttemptInput {
        CreateCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: work_item_id.to_string(),
            base_branch: "main".to_string(),
            branch_name: format!("aria/work-items/{work_item_id}/d21c"),
            worktree_path: None,
            provider_config_snapshot: provider_snapshot(),
            target_snapshot: target.map(d21c_target_snapshot),
            max_auto_rework: 2,
        }
    }

    fn group_input(
        plan_id: &str,
        work_item_id: &str,
        target: Option<LogicalRepositoryId>,
    ) -> CreateGroupCodingAttemptInput {
        CreateGroupCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: plan_id.to_string(),
            current_work_item_id: work_item_id.to_string(),
            base_branch: "main".to_string(),
            branch_name: format!("aria/issues/{ISSUE_ID}/{plan_id}"),
            worktree_path: None,
            provider_config_snapshot: provider_snapshot(),
            target_snapshot: target.map(d21c_target_snapshot),
            max_auto_rework: 2,
            start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
        }
    }

    // per-(issue,target) 单 active 语义（既有行为）：各创建用例分到互异
    // target 桶，避免用例间互相顶撞 active 检查。
    const TARGET_1: LogicalRepositoryId = LogicalRepositoryId(uuid::Uuid::from_u64_pair(1, 1));
    const TARGET_2: LogicalRepositoryId = LogicalRepositoryId(uuid::Uuid::from_u64_pair(2, 2));
    const TARGET_3: LogicalRepositoryId = LogicalRepositoryId(uuid::Uuid::from_u64_pair(3, 3));

    fn d21c_target_snapshot(repo: LogicalRepositoryId) -> AttemptTargetSnapshot {
        AttemptTargetSnapshot {
            logical_repository_id: repo,
            checkout_id: RepositoryCheckoutId(uuid::Uuid::new_v4()),
            physical_repository_id: format!("repository_d21c_{}", repo.0),
            canonical_path: std::path::PathBuf::from(format!("/tmp/repository_d21c_{}", repo.0)),
            git_dir_identity: format!("sha256:d21c-{}", repo.0),
            revision: Some("d21c-revision".to_string()),
            policy_digest: "d21c-policy".to_string(),
            membership_revision: 1,
            captured_at: "2026-10-14T00:00:00Z".to_string(),
            capture_source: "test".to_string(),
        }
    }

    fn journal_unit(logical_id: &str) -> AuthoritativeCodingUnitBinding {
        AuthoritativeCodingUnitBinding {
            logical_work_item_id: logical_id.to_string(),
            work_item_revision_id: format!("revision_{logical_id}"),
            verification_plan_revision_id: format!("verification_{logical_id}"),
            projection_bundle_id: format!("projection_{logical_id}"),
            target_repository_id: None,
            source_draft_error: None,
            dependency_logical_work_item_ids: Vec::new(),
        }
    }

    let tmp = tempfile::TempDir::new().unwrap();
    let paths = ProductAppPaths::new(tmp.path().join(".aria"));
    let store = CodingAttemptStore::new(paths.clone());
    let lifecycle = LifecycleStore::new(paths.clone());
    let operations = EntityOperationStore::new(paths.clone());
    let deletions = EntityDeletionStore::new(paths.clone());

    const PLAN_A: &str = "work_item_plan_d21c_a";
    const PLAN_B: &str = "work_item_plan_d21c_b";
    const PLAN_C: &str = "work_item_plan_d21c_c";
    const WI_A: &str = "work_item_d21c_a";
    const WI_B: &str = "work_item_d21c_b";
    const WI_C: &str = "work_item_d21c_c";
    // 无 lifecycle 记录的 legacy WI：guard 归属解析为 None（保持原行为），
    // journal 恢复路径必须靠 journal.plan_id 自身的准入兜住墓碑。
    const WI_LEGACY: &str = "work_item_d21c_legacy";

    lifecycle
        .create_work_item(work_item_input(WI_A, PLAN_A))
        .unwrap();
    lifecycle
        .create_work_item(work_item_input(WI_B, PLAN_B))
        .unwrap();
    lifecycle
        .create_work_item(work_item_input(WI_C, PLAN_C))
        .unwrap();

    let scope_a = EntityScope::new(PROJECT_ID, ISSUE_ID, DeletionEntityKind::Plan, PLAN_A);
    let scope_b = EntityScope::new(PROJECT_ID, ISSUE_ID, DeletionEntityKind::Plan, PLAN_B);

    // ── (a) 创建先得 lease：guard 持 owning Plan 共享准入 → 删除独占立即
    //       Busy，且零墓碑零 pending（零破坏拒绝）。
    let guard = store
        .acquire_work_item_attempt_creation(PROJECT_ID, ISSUE_ID, WI_A)
        .expect("creation guard under live plan");
    match operations.try_acquire_delete(&scope_a) {
        Err(EntityOperationError::Busy) => {}
        Err(other) => panic!("expected Busy, got: {other:?}"),
        Ok(_) => panic!("delete must be busy while creation holds the plan admission"),
    }
    assert!(
        deletions.get(&scope_a).unwrap().is_none(),
        "busy rejection must leave zero tombstone"
    );
    drop(guard);

    // ── (b) attempt 落盘后（准入收口）：删除独占可得，coding 门禁判据
    //       （REQ-DEL-05 attempt 记录存在即拒）在场。
    store
        .create_attempt(direct_input(WI_A, Some(TARGET_1)))
        .expect("direct create under live plan");
    let delete_lease = operations
        .try_acquire_delete(&scope_a)
        .expect("delete exclusive is available after creation closed");
    assert!(
        !store
            .list_attempts_for_work_item(PROJECT_ID, ISSUE_ID, WI_A)
            .unwrap()
            .is_empty(),
        "persisted attempt is the coding_workspace_exists gate input"
    );
    drop(delete_lease);

    // ── (c) 删除先赢（purging 墓碑）：direct / group / journal 恢复创建
    //       全拒、attempt 数 0。
    let journal_b = store
        .prepare_group_initialization(
            &group_input(PLAN_B, WI_LEGACY, None),
            "plan_revision_d21c_b",
            &[journal_unit(WI_LEGACY)],
        )
        .expect("prepare journal before the deletion window");
    let lease_b = operations
        .try_acquire_delete(&scope_b)
        .expect("delete exclusive for plan B");
    deletions.begin(&lease_b).expect("begin purging tombstone");
    drop(lease_b);
    assert_eq!(
        deletions.get(&scope_b).unwrap().map(|record| record.phase),
        Some(DeletionPhase::Purging)
    );

    let direct_error = store
        .create_attempt(direct_input(WI_B, None))
        .expect_err("direct create must reject the purging plan");
    assert_eq!(conflict_kind(&direct_error), "entity_deleting");

    let group_error = store
        .create_group_attempt(group_input(PLAN_B, WI_B, None))
        .expect_err("group create must reject the purging plan");
    assert_eq!(conflict_kind(&group_error), "entity_deleting");

    // journal 恢复路径一：lock WI 属于被删 Plan——guard 获取即拒（墓碑
    // 复核在 creation lock 外层）。
    let guard_error = match store.acquire_work_item_attempt_creation(PROJECT_ID, ISSUE_ID, WI_B) {
        Err(error) => error,
        Ok(_) => panic!("guard acquisition must reject the purging owning plan"),
    };
    assert_eq!(conflict_kind(&guard_error), "entity_deleting");

    // journal 恢复路径二：lock WI 无 lifecycle 归属（guard 准入 None）——
    // ensure 必须以 journal.plan_id 自身的准入拒绝墓碑后的 attempt 持久化。
    let legacy_guard = store
        .acquire_work_item_attempt_creation(PROJECT_ID, ISSUE_ID, WI_LEGACY)
        .expect("legacy unowned guard keeps original behavior");
    let ensure_error = store
        .ensure_group_initialization_attempt(&journal_b, &legacy_guard)
        .expect_err("journal recovery must reject the purging plan");
    assert_eq!(conflict_kind(&ensure_error), "entity_deleting");
    drop(legacy_guard);

    // journal 再准备（新入口）同样拒绝，不得绕过墓碑重建可恢复 journal。
    let prepare_error = store
        .prepare_group_initialization(
            &group_input(PLAN_B, WI_LEGACY, None),
            "plan_revision_d21c_b",
            &[journal_unit(WI_LEGACY)],
        )
        .expect_err("journal prepare must reject the purging plan");
    assert_eq!(conflict_kind(&prepare_error), "entity_deleting");

    assert!(
        store
            .list_attempts_for_work_item(PROJECT_ID, ISSUE_ID, WI_B)
            .unwrap()
            .is_empty()
            && store
                .list_attempts_for_work_item_group(PROJECT_ID, ISSUE_ID, PLAN_B)
                .unwrap()
                .is_empty(),
        "rejected creations must leave zero attempts for the purging plan"
    );

    // ── (d) guard 已取旧 arbitration 锁路径（advance 编排在 arbitration 内
    //       取 creation guard）：墓碑在位时同样拒绝，不得绕过。
    let _arbitration = store
        .acquire_group_initialization_arbitration(PROJECT_ID, ISSUE_ID)
        .expect("old arbitration path");
    let arbitration_guard_error =
        match store.acquire_work_item_attempt_creation(PROJECT_ID, ISSUE_ID, WI_B) {
            Err(error) => error,
            Ok(_) => panic!("guard under arbitration must still reject the tombstone"),
        };
    assert_eq!(conflict_kind(&arbitration_guard_error), "entity_deleting");
    drop(_arbitration);

    // ── (e) Deleted 墓碑（完成态）同样拒绝创建（稳定拒绝，不复活）。
    let lease_b = operations
        .try_acquire_delete(&scope_b)
        .expect("purging reentry lease");
    deletions.complete(&lease_b).expect("complete tombstone");
    drop(lease_b);
    let deleted_error = store
        .create_group_attempt(group_input(PLAN_B, WI_B, None))
        .expect_err("group create must reject the deleted plan");
    // 与 coding_plan_run/advance_store 惯例同族：删除窗口（purging 与
    // deleted）一律稳定 entity_deleting，不区分相位。
    assert_eq!(conflict_kind(&deleted_error), "entity_deleting");

    // ── (f) 其他 Plan 创建正常：删除窗口只收窄到 owning Plan scope。
    store
        .create_attempt(direct_input(WI_C, Some(TARGET_2)))
        .expect("other plan direct create is unaffected");
    store
        .create_group_attempt(group_input(PLAN_C, WI_C, Some(TARGET_3)))
        .expect("other plan group create is unaffected");
    assert_eq!(
        store
            .list_attempts_for_work_item(PROJECT_ID, ISSUE_ID, WI_C)
            .unwrap()
            .len(),
        2,
        "one direct attempt and one group attempt for the live plan"
    );
}
