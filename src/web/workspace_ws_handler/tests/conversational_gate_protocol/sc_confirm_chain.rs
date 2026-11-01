// REQ-WSC-14：SC confirm 真实链用例拆分（单文件行数门禁）。
//
// 7.2 SC 全链（冲突停等面）：真实 create→仲裁→SC Confirm 分流→真实
// registry 冲突 fail-closed——typed TrustWaiting（真实 reason+场景
// retry_action）、close CAS 计数锚零推进、durable 会话零变化、无
// HumanGateClosed/close 广播、用户冲突条目字节保持、失败 waiting/audit
// 落盘可观察（按设计），factory HOME 装配的只读 source 同一隔离 home。
#[tokio::test]
async fn ws_sc_confirm_real_manager_chain_waits_on_real_registry_conflict() {
    use crate::product::logical_codebase::{ProviderTrustHomeAdapter, ProviderTrustStore};
    use crate::web::state::WebAppState;
    use crate::web::workspace_session::{ConnectionRole, WorkspaceSessionManager};

    let (fixture, record) = t7_sc_confirm_chain_fixture();
    let root_path = fixture._root.path().to_path_buf();
    let app_paths = fixture.paths.clone();
    let lifecycle = fixture.lifecycle.clone();
    // 用户冲突条目（隔离 home=workspace root；冻结根=manifest 聚合根）。
    let canonical_root = std::fs::canonicalize(app_paths.root()).expect("canonical aggregate root");
    let codex_config = root_path.join(".codex").join("config.toml");
    std::fs::create_dir_all(codex_config.parent().expect("codex config dir"))
        .expect("codex config dir");
    std::fs::write(
        &codex_config,
        format!(
            "[projects.\"{}\"]\ntrust_level = \"untrusted\"\n",
            canonical_root.display()
        ),
    )
    .expect("seed user conflict");
    let conflict_bytes = std::fs::read(&codex_config).expect("conflict bytes");
    // legacy 别名 LC 作用域（issue 无显式 LC 归属——与 part_01 实证拓扑一致）。
    let lc_id = crate::product::logical_codebase::store::legacy_logical_codebase_id("project_0001");
    let trust_store = ProviderTrustStore::for_lc(app_paths.clone(), &lc_id);

    let _env = T7WsHomeEnvGuard::isolate(&root_path);
    let state = WebAppState::with_provider_registry(
        root_path.clone(),
        crate::web::runtime::WebRuntime::new_fake(root_path.clone()),
        ProviderRegistry::new(),
    );
    let manager = WorkspaceSessionManager::create(&state, &record.id)
        .await
        .expect("create real manager for SC session");

    // 铁律：manager 装配的确认上下文在（真实 create 注入），且 factory
    // HOME 装配的只读 source 指向同一隔离 home（冲突态给真实冲突指导）。
    {
        let engine_arc = manager.engine();
        let engine = engine_arc.lock().await;
        assert!(
            engine.confirmation_trust_context().is_some(),
            "真实 create 必须注入确认 trust 上下文"
        );
        let gateway = engine
            .logical_provider_gateway()
            .expect("LC 会话 gateway 在场");
        let waiting = gateway
            .read_only_generation_trust_waiting(&ProviderName::Codex)
            .expect("冲突态只读复验给等待面");
        assert_eq!(waiting.reason_code, "codex_trust_key_conflict");
        assert!(
            waiting
                .manual_guidance
                .contains(&codex_config.display().to_string()),
            "factory HOME 装配的 source 指向隔离 home 工件：{}",
            waiting.manual_guidance
        );
    }

    // 当前 driver 连接（真实 attachment 语义），仲裁放行。
    let (driver_tx, mut driver_rx) = mpsc::channel(16);
    manager.register_attachment("t7-sc-driver", driver_tx.clone());
    manager.bind_role(&driver_tx, ConnectionRole::Driver, None);
    manager.activate_attachment("t7-sc-driver");
    manager
        .arbitrate("t7-sc-driver", &WsInMessage::Confirm)
        .expect("driver 写面放行");

    let session_id = record.id.clone();
    let cas_before =
        crate::product::workspace_engine::conversational_gate::close_cas_attempts(&session_id);
    let durable_before = serde_json::to_vec(
        &lifecycle
            .get_workspace_session(&session_id)
            .expect("durable before"),
    )
    .expect("serialize before");

    let run_context = manager.provider_run_context(state.workspace_runs.clone());
    let context = WorkspaceInboundContext {
        app_state: state.clone(),
        engine: manager.engine().clone(),
        run_context,
        outbound_tx: driver_tx.clone(),
        session_id: session_id.clone(),
    };
    handle_workspace_inbound_message(context, WsInMessage::Confirm).await;

    // SC 分流 → Approve → 真实 registry 冲突停等：typed ProtocolError。
    let mut waiting_frame: Option<serde_json::Value> = None;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while let Some(control) = tokio::time::timeout(
        deadline.saturating_duration_since(std::time::Instant::now()),
        driver_rx.recv(),
    )
    .await
    .expect("SC 真实链确认必须回执（含恢复 Error 帧在内的帧流内找到停等帧）")
    {
        let OutboundControl::Text(json) = control else {
            continue;
        };
        let value: serde_json::Value = serde_json::from_str(&json).expect("frame json");
        if value["type"] == "protocol_error" {
            waiting_frame = Some(value);
            break;
        }
    }
    let frame = waiting_frame.expect("必须出现 protocol_error 停等帧");
    assert_eq!(
        frame["code"],
        serde_json::json!("workspace_confirmation_trust_waiting")
    );
    let message = frame["message"].as_str().expect("message");
    assert!(
        message.contains("codex_trust_key_conflict"),
        "真实冲突 reason 保留：{message}"
    );
    assert!(
        message.contains("重新确认"),
        "确认场景 retry_action：{message}"
    );
    assert!(
        message.contains("门保持未关闭"),
        "门未关闭提示在场：{message}"
    );

    // close CAS 零推进、durable 会话零变化、无 close 广播。
    assert_eq!(
        crate::product::workspace_engine::conversational_gate::close_cas_attempts(&session_id),
        cas_before,
        "准备失败不推进原 close CAS"
    );
    assert_eq!(
        serde_json::to_vec(
            &lifecycle
                .get_workspace_session(&session_id)
                .expect("durable after")
        )
        .expect("serialize after"),
        durable_before,
        "停等前后 durable 会话记录完全不变"
    );
    let frames = t7_sc_drain_outbound(&mut driver_rx, std::time::Duration::from_millis(200)).await;
    assert!(
        !frames.iter().any(|frame| {
            (frame["type"] == "stage_change" && frame["stage"] == "completed")
                || frame["type"] == "human_gate_closed"
        }),
        "停等不产生 close/关门广播：{frames:?}"
    );

    // 用户冲突条目字节保持；失败 waiting/audit 可观察；零 ownership。
    assert_eq!(
        std::fs::read(&codex_config).expect("conflict bytes after"),
        conflict_bytes,
        "用户冲突条目不得被覆盖"
    );
    let waiting = trust_store.list_waiting("project_0001").unwrap();
    assert!(
        waiting
            .iter()
            .any(|record| record.reason_code == "codex_trust_key_conflict"),
        "失败 waiting 落盘可观察：{waiting:?}"
    );
    let expected_operation = format!("workspace_confirm:{}", session_id);
    let audits = trust_store.list_audit("project_0001").unwrap();
    assert!(
        audits.iter().any(|audit| {
            audit.action == "ensure"
                && audit.outcome == "conflict"
                && audit.operation_id.as_deref() == Some(expected_operation.as_str())
        }),
        "冲突审计落盘且关联本次确认（operation_id=workspace_confirm:<session>）：{audits:?}"
    );
    let codex_trust_key = crate::product::logical_codebase::CodexTrustAdapter::for_home(&root_path)
        .trust_key(&canonical_root);
    assert!(
        trust_store
            .load_ownership("project_0001", "codex", &codex_trust_key)
            .expect("ownership lookup")
            .is_none(),
        "冲突未登记成功，零 ownership"
    );
}
