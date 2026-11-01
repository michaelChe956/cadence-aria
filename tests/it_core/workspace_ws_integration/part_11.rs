// ── change human-gate-termination-reliability（C3 Task 1）─────────────────────
// REQ-HTR-01 abort 诚实化的 WS 集成面：
// - 场景 1：人工门开态（SC human_confirm，无 active run）发送 Abort → 必须
//   得到携带指路信息的 ProtocolError（门级操作=反馈/确认/终止此门），会话
//   durable 状态零变化，连接保持可用（非阻塞校验，socket 读循环不被楔死）。
// - 场景 2：无 active run 的 run 族阶段（prepare_context）发送 Abort →
//   必须得到稳定码 abort_no_active_run 的诚实回执（不再静默零回执）。
// 既有「run 态 Abort → Aborted」语义由 part_06b 的 lease/observer 用例持续回归。

/// REQ-HTR-01 场景 1：门开态 Abort 被矩阵拒收并指路，会话零变化。
#[tokio::test]
async fn workspace_ws_abort_at_human_confirm_gate_is_rejected_with_guidance() {
    let root = tempdir().expect("root");
    // 复用 part_10 的 SC 门夹具：WorkItemPlan + single_candidate + waiting_for_human
    // + active human_confirm 节点（引擎重建后 stage=human_confirm）。
    let fixture = create_sc_compile_recovery_fixture(
        &root,
        vec![sc_recovery_node(
            SC_RECOVERY_NODE_ID,
            TimelineNodeType::HumanConfirm,
            cadence_aria::web::workspace_ws_types::WorkspaceStage::HumanConfirm,
            TimelineNodeStatus::Active,
        )],
        cadence_aria::product::models::WorkspaceSessionStatus::WaitingForHuman,
        false,
    )
    .await;
    let (mut ws, server) = sc_recovery_ws_harness(&root, &fixture.session_id).await;

    // 零变化基线：诊断流（lease/degraded）不属会话状态，比较时排除。
    let before = session_state_snapshot(&root, &fixture.session_id);
    let durable_before = durable_tree_without_diagnostics(&root);

    send_json(&mut ws, &WsInMessage::Abort).await;
    let outcome = sc_recovery_await_outcome(&mut ws, true).await;
    match &outcome {
        WsOutMessage::ProtocolError {
            code,
            message,
            context,
        } => {
            assert_eq!(
                code, "WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID",
                "门开态 Abort 必须以稳定码拒收: {message}"
            );
            assert!(
                message.contains("abandon_human_gate")
                    && message.contains("human_gate_feedback"),
                "拒收必须携带门级动作指路（反馈/确认/终止此门）: {message}"
            );
            let context = context.as_ref().expect("rejection context");
            assert_eq!(context["stage"].as_str(), Some("human_confirm"));
            assert_eq!(context["received"].as_str(), Some("abort"));
        }
        other => panic!("门开态 Abort 必须被拒收，got {other:?}"),
    }

    // 连接健康：拒收路径不得等待 engine 锁/阻塞读循环——Ping 立即回 Pong。
    send_json(&mut ws, &WsInMessage::Ping).await;
    let mut ponged = false;
    for _ in 0..20 {
        if matches!(recv_json(&mut ws).await, WsOutMessage::Pong) {
            ponged = true;
            break;
        }
    }
    assert!(ponged, "拒收后连接必须保持响应（无锁等待楔死）");

    // 会话状态零变化：record/timeline/门快照原样（F-53 定案的「点了没反应」
    // 根因面——本变更后是「明确报错」，且报错不产生任何副作用）。
    let after = session_state_snapshot(&root, &fixture.session_id);
    assert_eq!(before, after, "门开态 Abort 拒收后会话 durable 状态零变化");
    assert_eq!(
        durable_tree_without_diagnostics(&root),
        durable_before,
        "durable 树（除诊断流）零变化"
    );

    drop(ws);
    server.abort();
}

/// REQ-HTR-01 场景 2：无 active run 时 Abort 得到诚实回执（abort_no_active_run）。
#[tokio::test]
async fn workspace_ws_abort_without_active_run_gets_honest_receipt() {
    let root = tempdir().expect("root");
    create_workspace_session_fixture(&root).await;
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
    let url = format!("ws://{addr}/api/workspace-sessions/workspace_session_0001/ws");

    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let initial = recv_json(&mut ws).await;
    assert!(matches!(initial, WsOutMessage::SessionState { .. }));

    let durable_before = durable_tree_without_diagnostics(&root);
    send_json(&mut ws, &WsInMessage::Abort).await;
    let outcome = sc_recovery_await_outcome(&mut ws, true).await;
    match &outcome {
        WsOutMessage::ProtocolError { code, context, .. } => {
            assert_eq!(code, "abort_no_active_run", "无 run 回执必须用稳定码");
            let context = context.as_ref().expect("abort receipt context");
            assert_eq!(
                context["stage"].as_str(),
                Some("prepare_context"),
                "回执必须携带当前阶段"
            );
        }
        other => panic!("无 active run 的 Abort 必须回执 ProtocolError，got {other:?}"),
    }

    send_json(&mut ws, &WsInMessage::Ping).await;
    let mut ponged = false;
    for _ in 0..20 {
        if matches!(recv_json(&mut ws).await, WsOutMessage::Pong) {
            ponged = true;
            break;
        }
    }
    assert!(ponged, "诚实回执后连接必须保持响应");
    assert_eq!(
        durable_tree_without_diagnostics(&root),
        durable_before,
        "无 run Abort 回执不得产生 durable 副作用"
    );

    drop(ws);
    server.abort();
}

/// 会话状态摘要（record 关键字段 + timeline 节点）——零变化断言的比较面。
fn session_state_snapshot(root: &TempDir, session_id: &str) -> serde_json::Value {
    let lifecycle = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let record = lifecycle
        .get_workspace_session(session_id)
        .expect("load session record");
    let nodes = lifecycle.load_timeline_nodes(session_id).expect("load nodes");
    serde_json::json!({
        "status": record.status,
        "single_candidate_phase": record.single_candidate_phase,
        "human_gate_snapshot": record.human_gate_snapshot,
        "updated_at": record.updated_at,
        "nodes": nodes
            .iter()
            .map(|node| {
                serde_json::json!({
                    "node_id": node.node_id,
                    "node_type": node.node_type,
                    "status": node.status,
                })
            })
            .collect::<Vec<_>>(),
    })
}

/// durable 树快照，排除诊断流（lease/degraded 打点属可观测面，不属会话状态）。
fn durable_tree_without_diagnostics(root: &TempDir) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    durable_tree_snapshot(root.path())
        .into_iter()
        .filter(|(path, _)| {
            !path
                .to_string_lossy()
                .contains("lease-diagnostics.jsonl")
                && !path
                    .to_string_lossy()
                    .contains("degraded-diagnostics.jsonl")
        })
        .collect()
}
