// ─── fix-cascade-deletion Task D2.1d＋H3：advance 及 retry 真实入口 lease 与旧重放拒收 ───
//
// REQ-DEL-06/07（plan D2.1d Step 1）＋设计 2026-10-14 H3：advance 入口在
// engine/arbitration 锁外持 owning Plan run lease——准入在持期间删除独占
// 立即 Busy（零墓碑零破坏）；purging/deleted 身份守卫先于 command_id/
// plan_id 旧幂等命中：旧 Ready record 不回放（不产生 AdvanceCompleted 旧
// 入口），同/新 command、retry 一律稳定拒绝、零 attempt/retry JSON 新增；
// Ready 判定 false；WS 面经 map_advance_outcome 仍真实 Rejected。
#[tokio::test]
async fn cascade_d2_1d_deleted_advance_cannot_replay() {
    use crate::product::entity_deletion_store::{
        DeletionEntityKind, DeletionPhase, EntityDeletionStore, EntityScope,
    };
    use crate::product::entity_operation_store::{EntityOperationError, EntityOperationStore};
    use crate::product::models::automation::RetryInitializationRequest;
    use crate::product::workspace_engine::AdvancePlanAdmission;

    let root = tempfile::tempdir().unwrap();
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = AdvanceStore::new(app_paths.clone());
    let record = stored_record(COMMAND_ID);
    store.put_record(&record).unwrap();
    let lifecycle = LifecycleStore::new(app_paths.clone());
    // engine fixture 先于任何墓碑建立（session 创建自身受 D1.3a 准入约束）。
    let mut engine = engine_fixture(&root, lifecycle);
    let operations = EntityOperationStore::new(app_paths.clone());
    let deletions = EntityDeletionStore::new(app_paths.clone());
    let scope = EntityScope::new(PROJECT_ID, ISSUE_ID, DeletionEntityKind::Plan, PLAN_ID);

    // (a) 入口准入在持（advance_plan/retry HTTP 同一获取面）：删除独占立即
    //     Busy，零墓碑零 pending（零破坏拒绝）。
    let admission = AdvancePlanAdmission::acquire(&app_paths, PROJECT_ID, ISSUE_ID, PLAN_ID)
        .expect("plan run admission for a live plan");
    match operations.try_acquire_delete(&scope) {
        Err(EntityOperationError::Busy) => {}
        Err(other) => panic!("expected Busy, got: {other:?}"),
        Ok(_) => panic!("delete must be busy while the advance admission is held"),
    }
    assert!(
        deletions.get(&scope).unwrap().is_none(),
        "busy rejection must leave zero tombstone"
    );
    drop(admission);

    // (b) purging 墓碑先于旧幂等命中：同 command（Ready record 命中位）与
    //     新 command 都稳定 Rejected，绝不 Replayed→AdvanceCompleted。
    let delete_lease = operations
        .try_acquire_delete(&scope)
        .expect("delete exclusive after the admission closed");
    deletions.begin(&delete_lease).expect("purging tombstone");
    drop(delete_lease);
    assert_eq!(
        deletions.get(&scope).unwrap().map(|tomb| tomb.phase),
        Some(DeletionPhase::Purging)
    );

    for command in [COMMAND_ID, "command_d21d_late"] {
        let outcome = engine.handle_advance(input(command)).await.unwrap();
        match &outcome {
            AdvanceOutcome::Rejected { code, reason, .. } => {
                assert_eq!(code, "ADVANCE_PLAN_DELETING", "{command}: {reason}");
            }
            other => panic!("purging plan must reject {command}, got {other:?}"),
        }
        // WS 面仍真实 Rejected（不发 AdvanceCompleted 旧入口）。
        assert!(matches!(
            crate::web::handlers::map_advance_outcome(command.to_string(), outcome),
            crate::web::workspace_ws_types::WsOutMessage::AdvanceRejected { .. }
        ));
    }

    // retry 面：身份守卫先于 Failed 前置/命令账本——稳定拒绝且零 retry 事实。
    let binding: crate::product::models::automation::EnrollmentBindingIdentity =
        serde_json::from_value(serde_json::json!({
            "binding_version": 1,
            "enrollment_id": "enrollment_d21d",
            "plan_id": PLAN_ID,
            "session_id": "session_d21d",
            "source": { "stories": [], "designs": [] },
            "target": { "kind": "single_repository", "repository_id": "repository_0001" },
            "author_provider": "fake",
            "reviewer_provider": "fake"
        }))
        .expect("binding identity");
    let retry_error = engine
        .retry_initialization(&RetryInitializationRequest {
            command_id: "retry_d21d_0001".to_string(),
            expected_binding: binding,
            expected_attempt_id: "attempt_advance".to_string(),
            expected_checkpoint: AdvanceInitializationPhase::JournalPrepared,
            confirm_unknown_side_effect: true,
        })
        .await
        .expect_err("retry must reject the purging plan");
    assert!(
        retry_error.contains("entity_deleting"),
        "retry rejection must carry the stable deletion code: {retry_error}"
    );
    assert!(
        AdvanceStore::new(app_paths.clone())
            .get_retry_initialization(PROJECT_ID, ISSUE_ID, "retry_d21d_0001")
            .unwrap()
            .is_none(),
        "rejected retry must leave zero retry facts"
    );

    // 零新增：advance record 仍是原一条，coding attempts 面零新建。
    assert_eq!(
        AdvanceStore::new(app_paths.clone())
            .list_plan_artifacts(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap()
            .advance_ids
            .len(),
        1,
        "rejected entries must not add advance records"
    );
    assert!(
        crate::product::coding_attempt_store::CodingAttemptStore::new(app_paths.clone())
            .list_attempts_for_issue(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .is_empty(),
        "rejected entries must leave zero coding attempts"
    );
    // Ready 判定 false（删除事实优先，D1.2c 面在此一并钉住）。
    assert!(
        !AdvanceStore::new(app_paths.clone())
            .advance_is_ready_for_attempt(PROJECT_ID, ISSUE_ID, PLAN_ID, "attempt_advance")
            .unwrap()
    );

    // (c) Deleted 完成态同样稳定拒绝（不复活旧轮）。
    let delete_lease = operations
        .try_acquire_delete(&scope)
        .expect("purging reentry lease");
    deletions
        .complete(&delete_lease)
        .expect("complete tombstone");
    drop(delete_lease);
    let outcome = engine.handle_advance(input(COMMAND_ID)).await.unwrap();
    match &outcome {
        AdvanceOutcome::Rejected { code, .. } => assert_eq!(code, "ADVANCE_PLAN_DELETED"),
        other => panic!("deleted tombstone must reject, got {other:?}"),
    }
}

// ─── D2.1d＋H3：执行轮退役围栏先于旧 Ready 回放（不回放旧 attempt）───
//
// 设计 2026-10-14 A/H3 节：clearing 窗口内任何 advance 入口拒绝；轮次切换
// 完成后退役 command（旧 Ready record 的原 command）不再回放；新 command
// 不被退役围栏拦截，沿正常前置检查进入新轮。
#[tokio::test]
async fn cascade_d2_1d_retired_round_fences_old_replay_and_new_round() {
    use crate::product::coding_plan_run::{CodingPlanRunStore, CodingPlanWorkspaceResetChecklist};
    use crate::product::entity_deletion_store::{DeletionEntityKind, EntityScope};
    use crate::product::entity_operation_store::EntityOperationStore;

    let root = tempfile::tempdir().unwrap();
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = AdvanceStore::new(app_paths.clone());
    store.put_record(&stored_record(COMMAND_ID)).unwrap();
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let operations = EntityOperationStore::new(app_paths.clone());
    let run_store = CodingPlanRunStore::new(app_paths.clone());
    let scope = EntityScope::new(PROJECT_ID, ISSUE_ID, DeletionEntityKind::Plan, PLAN_ID);

    let checklist = CodingPlanWorkspaceResetChecklist {
        command_id: "close_d21d_0001".to_string(),
        retired_command_ids: vec![COMMAND_ID.to_string()],
        retired_attempt_ids: vec!["attempt_advance".to_string()],
        retired_advance_ids: vec![format!("advance_{COMMAND_ID}")],
        pending_attempt_ids: Vec::new(),
        pending_advance_ids: Vec::new(),
        pending_retry_command_ids: Vec::new(),
        pending_target_repository_ids: Vec::new(),
    };
    let delete_lease = operations.try_acquire_delete(&scope).expect("retire lease");
    run_store
        .retire_workspace_round_with_lease(&delete_lease, &checklist)
        .expect("freeze clearing fence");
    drop(delete_lease);

    // clearing 窗口：旧 command 与新 command 都拒绝，不回放旧 Ready。
    let mut engine = engine_fixture(&root, lifecycle);
    for command in [COMMAND_ID, "command_d21d_next_round"] {
        let outcome = engine.handle_advance(input(command)).await.unwrap();
        match &outcome {
            AdvanceOutcome::Rejected { code, reason, .. } => {
                assert_eq!(
                    code, "ADVANCE_PLAN_WORKSPACE_CLEARING",
                    "{command}: {reason}"
                );
            }
            other => panic!("clearing window must reject {command}, got {other:?}"),
        }
    }

    // 轮次切换收口（clearing 清空、退役事实保留）：旧 command 拒绝且不
    // 回放；新 command 穿透围栏进入正常前置（本 fixture 无 plan 记录 →
    // 原 ADVANCE_PLAN_NOT_FOUND，证围栏未扩大化）。
    let delete_lease = operations
        .try_acquire_delete(&scope)
        .expect("complete lease");
    run_store
        .complete_workspace_reset_with_lease(&delete_lease, "close_d21d_0001")
        .expect("complete the round switch");
    drop(delete_lease);

    let outcome = engine.handle_advance(input(COMMAND_ID)).await.unwrap();
    match &outcome {
        AdvanceOutcome::Rejected { code, reason, .. } => {
            assert_eq!(code, "ADVANCE_PLAN_RUN_RETIRED", "{reason}");
        }
        other => panic!("retired command must not replay, got {other:?}"),
    }
    let outcome = engine
        .handle_advance(input("command_d21d_next_round"))
        .await
        .unwrap();
    match &outcome {
        AdvanceOutcome::Replayed { record } => {
            assert_eq!(record.command_id, COMMAND_ID);
        }
        other => panic!(
            "new command must pass the fence into the normal plan-level replay path, got {other:?}"
        ),
    }
}
