// P2 GAP-E/G（Task 0.1）：SC Evaluate 相位 reviewer 失败必须一次性收敛为
// durable Failed（phase+status+timeline 节点三面一致），供人工显式重驱；
// 不允许失败后被 finish_failed_run 重置回 Open 造成「无现场可恢复」。

/// reviewer 启动/运行失败在 SC Evaluate 相位必须落 durable 终态 Failed，
/// 且失败节点保留 Failed 身份（供 REST 显式重驱按节点认领）。
#[tokio::test]
async fn sc_evaluate_reviewer_failure_is_durably_failed_once() {
    use crate::cross_cutting::provider_adapter::ProviderAdapterError;

    let (_tmp, lifecycle, _plan, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    single_candidate_record(
        &lifecycle,
        &mut engine,
        SingleCandidatePhase::Evaluate,
        RunPolicy::Interactive,
    );
    engine.start_review().await;
    let failed_node_id = engine.active_node_id.clone().unwrap();
    let (_tx, rx) = tokio::sync::mpsc::channel(1);
    engine
        .drive_reviewer_provider_session(
            Err(ProviderAdapterError::provider_unavailable(
                "503 No available accounts",
            )),
            rx,
            crate::product::models::ProviderName::ClaudeCode,
        )
        .await;
    let durable = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .unwrap();
    assert_eq!(
        durable.single_candidate_phase,
        Some(SingleCandidatePhase::Failed)
    );
    assert_eq!(durable.status, WorkspaceSessionStatus::Failed);
    assert!(lifecycle
        .load_timeline_nodes_for_issue_session(&durable.project_id, &durable.issue_id, &durable.id)
        .unwrap()
        .iter()
        .any(|n| n.node_id == failed_node_id && n.status == TimelineNodeStatus::Failed));

    // 引擎内存投影与 durable 一致（重启/重放不漂移）。
    assert_eq!(
        engine.session().single_candidate_phase,
        Some(SingleCandidatePhase::Failed)
    );
    assert_eq!(engine.session().session_status, WorkspaceSessionStatus::Failed);

    // 非 SC flow（legacy）不受影响：reviewer 失败仍走旧 Open 语义。
    let (_tmp2, lifecycle2, _plan2, mut engine2) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let mut legacy = lifecycle2
        .get_workspace_session(&engine2.session().session_id)
        .unwrap();
    legacy.flow_kind = crate::product::work_item_plan_policy::WorkItemPlanFlowKind::Legacy;
    legacy.single_candidate_phase = None;
    crate::product::json_store::write_json(
        &lifecycle2
            .app_paths()
            .issue_root(&legacy.project_id, &legacy.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", legacy.id)),
        &legacy,
    )
    .expect("persist legacy session");
    engine2.session = crate::product::workspace_engine::WorkspaceSession::from_record(legacy);
    engine2.start_review().await;
    let (_tx2, rx2) = tokio::sync::mpsc::channel(1);
    engine2
        .drive_reviewer_provider_session(
            Err(ProviderAdapterError::provider_unavailable(
                "503 No available accounts",
            )),
            rx2,
            crate::product::models::ProviderName::ClaudeCode,
        )
        .await;
    let durable2 = lifecycle2
        .get_workspace_session(&engine2.session().session_id)
        .unwrap();
    assert_eq!(durable2.status, WorkspaceSessionStatus::Open);
    assert_eq!(durable2.single_candidate_phase, None);
}
