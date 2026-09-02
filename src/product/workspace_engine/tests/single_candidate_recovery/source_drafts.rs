// 8.3 review Major-1 修复的恢复幂等用例(经 `include!` 内联进父模块,
// 复用父模块的全部 fixture/failpoint 辅助函数)。

/// SC 编译轮次草稿的字节级快照(文件名→内容,排序稳定),用于恢复重放幂等断言。
fn single_candidate_recovery_sc_draft_snapshot(
    lifecycle: &LifecycleStore,
    plan_id: &str,
    compile_id: &str,
) -> Vec<(String, Vec<u8>)> {
    let round_dir = lifecycle
        .app_paths()
        .issue_root("project_0001", "issue_0001")
        .join("work_item_plan_drafts")
        .join(plan_id)
        .join(format!("single_candidate_{compile_id}"));
    let mut snapshot = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&round_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
                let name = path.file_name().unwrap().to_string_lossy().to_string();
                snapshot.push((name, std::fs::read(&path).expect("draft bytes")));
            }
        }
    }
    snapshot.sort_by(|left, right| left.0.cmp(&right.0));
    snapshot
}

/// SC 编译 source-draft 落盘的恢复幂等(8.3 review Major-1 修复):
/// publication 中断 crash → Continue 恢复重放(再次中断)→ 再 Continue 完成,
/// 草稿库恰一套 SC draft:每个 `tx.active_draft_ids` 恰好一条、Accepted/active,
/// 且每次重放前后字节完全不变(draft_id/generation_round_id 由 durable
/// reservation 确定性派生,重放为同路径覆盖写,不重复不缺失)。
#[tokio::test(flavor = "current_thread")]
async fn single_candidate_recovery_replays_source_drafts_idempotently() {
    let _serial = single_candidate_recovery_failpoint_lock().await;
    let (_tmp, lifecycle, plan_id, mut engine) =
        single_candidate_recovery_prepare_approval().await;
    let compile_id =
        single_candidate_recovery_prepare_after_provenance_boundary(&mut engine, &lifecycle).await;
    let mut restarted = single_candidate_recovery_restart(&engine, &lifecycle);
    let publication_failpoint = restarted
        .revision_store()
        .register_initial_plan_publication_failpoint(
            "project_0001",
            "issue_0001",
            &plan_id,
            &compile_id,
            InitialPlanPublicationCheckpoint::PlanArtifactsWritten,
        );
    let error = restarted
        .run_work_item_plan_compile()
        .await
        .expect_err("publication checkpoint 必须中断");
    drop(publication_failpoint);
    assert!(error.contains("PlanArtifactsWritten"));

    // crash 时 drafts 已在编译提交段落盘(先于 publication),不缺失。
    let store = restarted.work_item_plan_store().expect("plan store");
    let interrupted_tx = store
        .get_compile_transaction("project_0001", "issue_0001", &plan_id, &compile_id)
        .expect("interrupted transaction");
    let expected_draft_ids = interrupted_tx
        .active_draft_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    assert_eq!(expected_draft_ids.len(), 2, "fixture 两个 SC 候选");
    let list_sc_drafts = |lifecycle: &LifecycleStore| {
        crate::product::work_item_plan_store::WorkItemPlanStore::new(lifecycle.app_paths())
            .list_draft_records("project_0001", "issue_0001", &plan_id)
            .expect("draft records")
            .into_iter()
            .filter(|record| record.generation_round_id == format!("single_candidate_{compile_id}"))
            .collect::<Vec<_>>()
    };
    let drafts_after_crash = list_sc_drafts(&lifecycle);
    let ids_after_crash = drafts_after_crash
        .iter()
        .map(|record| record.draft_id.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        ids_after_crash, expected_draft_ids,
        "编译提交段必须把全部 active draft 落盘"
    );
    assert!(
        drafts_after_crash.iter().all(|record| {
            record.status == crate::product::models::WorkItemDraftStatus::Accepted && record.active
        }),
        "SC source draft 必须是 accepted+active"
    );
    let bytes_after_crash =
        single_candidate_recovery_sc_draft_snapshot(&lifecycle, &plan_id, &compile_id);
    assert_eq!(bytes_after_crash.len(), 2);
    let total_after_crash =
        crate::product::work_item_plan_store::WorkItemPlanStore::new(lifecycle.app_paths())
            .list_draft_records("project_0001", "issue_0001", &plan_id)
            .expect("draft records")
            .len();
    assert_eq!(
        total_after_crash, 4,
        "fixture 基座 2 条 legacy + SC 恰 2 条"
    );

    // 第一次恢复重放:resume 在 publication 重放前重写同一套 draft,再次 crash。
    single_candidate_recovery_mark_transaction_recovery(
        &mut restarted,
        &plan_id,
        &compile_id,
        "publication crash PlanArtifactsWritten",
    )
    .await;
    let mut resumed_once = single_candidate_recovery_restart(&restarted, &lifecycle);
    let replay_failpoint = resumed_once
        .revision_store()
        .register_initial_plan_publication_failpoint(
            "project_0001",
            "issue_0001",
            &plan_id,
            &compile_id,
            InitialPlanPublicationCheckpoint::PlanActivated,
        );
    let replay_error = resumed_once
        .handle_work_item_plan_compile_recovery_action(
            WorkItemPlanCompileRecoveryActionDto::Continue,
            None,
        )
        .await
        .expect_err("恢复重放必须再次中断");
    drop(replay_failpoint);
    assert!(replay_error.contains("PlanActivated"));
    assert_eq!(
        single_candidate_recovery_sc_draft_snapshot(&lifecycle, &plan_id, &compile_id),
        bytes_after_crash,
        "第一次重放后草稿字节必须不变(同 draft_id 覆盖写,不重复)"
    );

    // 第二次恢复重放:完成到 committed,草稿仍恰一套。
    single_candidate_recovery_mark_transaction_recovery(
        &mut resumed_once,
        &plan_id,
        &compile_id,
        "publication replay crash PlanActivated",
    )
    .await;
    let mut recovered = single_candidate_recovery_restart(&resumed_once, &lifecycle);
    let outcome = recovered
        .handle_work_item_plan_compile_recovery_action(
            WorkItemPlanCompileRecoveryActionDto::Continue,
            None,
        )
        .await
        .expect("第二次恢复重放必须完成");
    assert_eq!(outcome, WorkItemPlanCompileRecoveryOutcome::HumanConfirm);
    assert_eq!(
        single_candidate_recovery_sc_draft_snapshot(&lifecycle, &plan_id, &compile_id),
        bytes_after_crash,
        "最终恢复后草稿字节仍不变(恰一套,不重复不缺失)"
    );
    let final_tx =
        crate::product::work_item_plan_store::WorkItemPlanStore::new(lifecycle.app_paths())
            .get_compile_transaction("project_0001", "issue_0001", &plan_id, &compile_id)
            .expect("final transaction");
    assert_eq!(final_tx.status, WorkItemPlanCompileStatus::Committed);
    assert_eq!(
        list_sc_drafts(&lifecycle)
            .iter()
            .map(|record| record.draft_id.clone())
            .collect::<BTreeSet<_>>(),
        expected_draft_ids,
        "恢复完成后 SC 草稿仍与 active_draft_ids 一一对应"
    );
}
