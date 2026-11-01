// ---------------------------------------------------------------------------
// REQ-DLS-01：写时自愈（driver-lease-self-healing）
// ---------------------------------------------------------------------------

/// 悬空自愈：租约 holder=None（原 holder 因连接断开悬空）时，Driver 连接的
/// 首条写消息必须放行并原子重授；重复写不因 epoch 漂移被拒（R2 钉子：
/// 自愈放款必须同步刷新 attachment.lease_epoch，否则后续写仍 stale）。
#[tokio::test]
async fn self_heal_regrants_dangling_lease_on_driver_first_write() {
    use crate::web::workspace_session::ConnectionRole;
    use crate::web::workspace_ws_types::WsInMessage;

    let manager = WorkspaceSessionManager::test_fixture("session_self_heal_dangling");
    let (original_tx, _original_rx) = mpsc::channel::<OutboundControl>(1);
    manager.register_attachment("original_driver", original_tx.clone());
    manager.bind_role(&original_tx, ConnectionRole::Driver, None);
    // 偷窃连接悬空：占用者断开后租约回到无人持有。
    let (thief_tx, _thief_rx) = mpsc::channel::<OutboundControl>(1);
    manager.register_attachment("thief_connection", thief_tx);
    manager.handle_connection_closed("thief_connection").await;

    // 原驾驶连接首写：holder=None 必须放行并重授（当前实现直接拒绝 → 红）。
    assert!(
        manager
            .arbitrate("original_driver", &WsInMessage::Abort)
            .is_ok(),
        "悬空租约下 Driver 首写必须自愈放行（REQ-DLS-01）"
    );
    // R2 联合断言：第二次写仍放行——若实现漏刷 attachment.lease_epoch，
    // 重授后的 epoch 与 attachment 记录不一致，第二写必被拒。
    assert!(
        manager
            .arbitrate("original_driver", &WsInMessage::Abort)
            .is_ok(),
        "自愈授予后同连接重复写不得被拒（lease_epoch 必须同步刷新）"
    );

    // 自愈后的持有关系可被显式接管打断，且再次悬空后仍可自愈（F-50-1 死路闭环）。
    let (taker_tx, _taker_rx) = mpsc::channel::<OutboundControl>(1);
    manager.register_attachment("explicit_taker", taker_tx.clone());
    manager.bind_role(&taker_tx, ConnectionRole::Driver, None);
    assert!(
        matches!(
            manager.arbitrate("original_driver", &WsInMessage::Abort),
            Err(ConnectionRole::Driver)
        ),
        "holder=Some(活跃连接) 时写面仍按既有语义拒绝（自愈不引入争抢）"
    );
    manager.handle_connection_closed("explicit_taker").await;
    assert!(
        manager
            .arbitrate("original_driver", &WsInMessage::Abort)
            .is_ok(),
        "再次悬空后必须再次自愈"
    );
}

/// REQ-DLS-01 场景二：活跃偷窃者不被抢——holder 为另一条活跃连接时，
/// 写消息仍按既有语义拒绝 STALE（Err(Driver)）。
#[tokio::test]
async fn self_heal_never_grants_when_another_active_connection_holds_lease() {
    use crate::web::workspace_session::ConnectionRole;
    use crate::web::workspace_ws_types::WsInMessage;

    let manager = WorkspaceSessionManager::test_fixture("session_self_heal_active_holder");
    let (first_tx, _first_rx) = mpsc::channel::<OutboundControl>(1);
    manager.register_attachment("first_driver", first_tx.clone());
    manager.bind_role(&first_tx, ConnectionRole::Driver, None);
    let (second_tx, _second_rx) = mpsc::channel::<OutboundControl>(1);
    manager.register_attachment("second_driver", second_tx.clone());
    manager.bind_role(&second_tx, ConnectionRole::Driver, None);

    assert!(
        matches!(
            manager.arbitrate("first_driver", &WsInMessage::Abort),
            Err(ConnectionRole::Driver)
        ),
        "holder=Some(活跃偷窃者) 时写面必须拒绝，自愈只覆盖悬空态"
    );
}

/// REQ-DLS-01 场景三：observer 写拒绝不变——即使租约悬空（自愈窗口打开），
/// observer 角色的写消息仍必须按既有语义拒绝（Err(Observer)）。
#[tokio::test]
async fn self_heal_window_does_not_relax_observer_write_rejection() {
    use crate::web::workspace_session::ConnectionRole;
    use crate::web::workspace_ws_types::WsInMessage;

    let manager = WorkspaceSessionManager::test_fixture("session_self_heal_observer");
    let (observer_tx, _observer_rx) = mpsc::channel::<OutboundControl>(1);
    manager.register_attachment("observer_connection", observer_tx.clone());
    manager.bind_role(&observer_tx, ConnectionRole::Observer, None);

    assert!(
        matches!(
            manager.arbitrate("observer_connection", &WsInMessage::Abort),
            Err(ConnectionRole::Observer)
        ),
        "悬空窗口对 observer 不适用：写拒绝语义不得放宽"
    );
}

/// REQ-DLS-03：五类租约事件（hold/self_heal/release/write_rejected_stale/
/// write_rejected_observer）以 append-only 单行 JSON 落会话级
/// `lease-diagnostics.jsonl`；自愈授予幂等（同连接重复写不重复打点）。
#[tokio::test]
async fn lease_diagnostics_records_five_event_kinds_to_session_jsonl() {
    use crate::web::workspace_session::ConnectionRole;
    use crate::web::workspace_ws_types::WsInMessage;

    let session_id = "session_lease_diag_events";
    let diagnostics_path = std::env::temp_dir()
        .join(session_id)
        .join("lease-diagnostics.jsonl");
    let _ = std::fs::remove_file(&diagnostics_path);

    let manager = WorkspaceSessionManager::test_fixture(session_id);
    let (a_tx, _a_rx) = mpsc::channel::<OutboundControl>(1);
    manager.register_attachment("conn_a", a_tx.clone());
    manager.bind_role(&a_tx, ConnectionRole::Driver, None);
    let (b_tx, _b_rx) = mpsc::channel::<OutboundControl>(1);
    manager.register_attachment("conn_b", b_tx.clone());
    manager.bind_role(&b_tx, ConnectionRole::Driver, None);
    // b 活跃持有：a 写被拒 → write_rejected_stale
    assert!(matches!(
        manager.arbitrate("conn_a", &WsInMessage::Abort),
        Err(ConnectionRole::Driver)
    ));
    // b 断开释放 → release；a 首写自愈 → self_heal（重复写不再打点）
    manager.handle_connection_closed("conn_b").await;
    assert!(manager.arbitrate("conn_a", &WsInMessage::Abort).is_ok());
    assert!(manager.arbitrate("conn_a", &WsInMessage::Abort).is_ok());
    // observer 写拒 → write_rejected_observer
    let (o_tx, _o_rx) = mpsc::channel::<OutboundControl>(1);
    manager.register_attachment("conn_observer", o_tx.clone());
    manager.bind_role(&o_tx, ConnectionRole::Observer, None);
    assert!(matches!(
        manager.arbitrate("conn_observer", &WsInMessage::Abort),
        Err(ConnectionRole::Observer)
    ));

    let content = std::fs::read_to_string(&diagnostics_path)
        .expect("lease-diagnostics.jsonl must exist after lease transitions");
    let events: Vec<serde_json::Value> = content
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    let kinds: Vec<&str> = events
        .iter()
        .map(|event| event["event"].as_str().unwrap_or_default())
        .collect();
    for expected in [
        "hold",
        "self_heal",
        "release",
        "write_rejected_stale",
        "write_rejected_observer",
    ] {
        assert!(
            kinds.contains(&expected),
            "lease-diagnostics.jsonl 缺少 {expected} 事件，现有：{kinds:?}"
        );
    }
    assert_eq!(
        kinds.iter().filter(|kind| **kind == "self_heal").count(),
        1,
        "同连接重复写不得重复授予/重复打点（自愈幂等）"
    );
    // release 先于 self_heal：悬空是自愈的前提，序列必须可定案。
    let release_at = kinds
        .iter()
        .position(|kind| *kind == "release")
        .expect("release event");
    let heal_at = kinds
        .iter()
        .position(|kind| *kind == "self_heal")
        .expect("self_heal event");
    assert!(release_at < heal_at);
    // 字段完整性：时刻、连接标识、角色、epoch。
    let heal = events
        .iter()
        .find(|event| event["event"] == "self_heal")
        .expect("self_heal record");
    assert_eq!(heal["connection_id"], "conn_a");
    assert_eq!(heal["role"], "driver");
    assert!(
        heal["recorded_at"]
            .as_str()
            .is_some_and(|at| !at.is_empty())
    );
    assert!(heal["epoch"].as_u64().is_some());
}

