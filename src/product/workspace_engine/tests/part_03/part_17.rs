// F2/R2 回归：原地重编译确认（`RecompileConfirm`）必须清旧代 **advance
// engine checkpoint**。
//
// 现场：`purge_plan_runtime_resources` 的 checkpoint 清理分支受
// `if let Some(workspace_root)` 保护，而 finalizer 是 `RecompileConfirm` 的
// 唯一调用点且传 `None` → 该清理从未生效：新代仍可能从旧代同一 session 的
// checkpoint 恢复。修法：finalizer 传真实 workspace root（构造
// `ProductAppPaths` 时的仓工作区根，与 evidence 注入同口径＝`<aria>` 的父目录）。

/// 编译确认链（`run_work_item_plan_compile` → finalize）必须清掉
/// `<workspace_root>/advance-engine-checkpoints/workspace-sessions/<owned session>`，
/// 且不得误删他 session 的 checkpoint。
#[tokio::test]
async fn compile_confirm_clears_old_generation_advance_engine_checkpoints() {
    let (tmp, _lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    // fixture 的 `ProductAppPaths` 根＝`<tmp>/.aria`，故 workspace root＝`<tmp>`。
    let workspace_root = tmp.path();
    let session_id = engine.session.session_id.clone();
    let sessions_root = workspace_root
        .join("advance-engine-checkpoints")
        .join("workspace-sessions");
    let checkpoint_session = sessions_root.join(&session_id);
    std::fs::create_dir_all(checkpoint_session.join("checkpoints")).expect("checkpoint dir");
    std::fs::write(
        checkpoint_session.join("checkpoints").join("cp_001.json"),
        b"{}",
    )
    .expect("checkpoint file");
    // 他 session 的 checkpoint：按 session 精确删，不得整删共享根。
    let other_session = sessions_root.join("workspace_session_other");
    std::fs::create_dir_all(&other_session).expect("other session dir");

    engine
        .run_work_item_plan_compile()
        .await
        .expect("initial plan compile");

    assert!(
        !checkpoint_session.exists(),
        "编译确认必须清旧代 advance engine checkpoint（否则新代可从旧代同 session 恢复）"
    );
    assert!(
        other_session.exists(),
        "checkpoint 清理按 session 精确删除，他 session 必须保留"
    );
    // 原地演进：plan 根保留。
    assert!(
        _lifecycle
            .get_issue_work_item_plan("project_0001", "issue_0001", &plan_id)
            .is_ok(),
        "原地重编译确认不得删除本代 plan 根"
    );
}
