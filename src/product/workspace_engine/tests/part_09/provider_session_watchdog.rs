#[tokio::test]
async fn drive_work_item_plan_provider_session_idle_watchdog_aborts_silent_provider() {
    // r29 问题2 根修锚点(r28 现场):SC 门修订 turn 的 provider 流式 1min 后
    // 静默挂死——本驱动循环此前缺 F-19 零活动看门狗(主驱动
    // drive_provider_session 有 600s),挂死 provider 永不收口,harness 侧
    // 90min 空等。修:同构补看门狗(test cfg 150ms)——静默超限即 Abort+
    // cancel+失败节点+Error 事件+finish_failed_run,有界返回 Err。
    let (_tmp, _checkpoint_store, _lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_idle_watchdog");
    engine.session.session_id = _lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .expect("workspace sessions")
        .into_iter()
        .find(|session| session.workspace_type == WorkspaceType::WorkItemPlan)
        .expect("work item plan session")
        .id;
    let node_id = engine.begin_work_item_plan_author_run().await;
    let (provider_event_tx, provider_event_rx) = mpsc::channel(8);
    let (provider_command_tx, _provider_command_rx) = mpsc::channel(8);
    // 一段真实流式后永久静默(事件通道保持打开=provider 未退出也未产出)。
    provider_event_tx
        .send(ProviderEvent::TextDelta {
            content: "partial draft then silence\n".to_string(),
        })
        .await
        .expect("send text delta");
    let mut command_rx = empty_provider_commands();

    let drive = engine.drive_work_item_plan_provider_session_to_output(
        Ok(ProviderSession {
            native_session_id: None,
            events: provider_event_rx,
            commands: provider_command_tx,
        }),
        &mut command_rx,
        node_id.clone(),
        ProviderName::ClaudeCode,
    );
    // 有界观测:修前裸 await 永挂(此处 5s 上界即 RED);修后看门狗
    // (test 150ms)触发,秒级返回带稳定原因码的 Err。
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), drive).await;
    let result = outcome.expect("看门狗必须在有界时间内收口(修前=永挂 90min)");
    assert!(result.is_err(), "静默 provider 必须按失败收口,不得返回输出");
    let message = result.expect_err("watchdog failure");
    assert!(
        message.contains("provider_idle_watchdog"),
        "失败原因必须携带稳定原因码 provider_idle_watchdog,got: {message}"
    );
}
