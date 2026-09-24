// ---------------------------------------------------------------------------
// REQ-DLS-04：attach 对租约零效应（driver-lease-self-healing）
// ---------------------------------------------------------------------------

/// REQ-DLS-04 场景一：observer 纯 attach 不偷租约——attach 不产生任何获取/
/// 转移/快照效应；无论 observer 是否随后 hello，现任 driver 的写面不受影响。
#[tokio::test]
async fn attach_zero_effect_observer_attach_does_not_steal_lease() {
    use crate::web::workspace_session::ConnectionRole;
    use crate::web::workspace_ws_types::WsInMessage;

    let manager = WorkspaceSessionManager::test_fixture("session_dls04_observer_attach");
    let (driver_tx, _driver_rx) = mpsc::channel::<OutboundControl>(1);
    manager.register_attachment("driver", driver_tx.clone());
    manager.bind_role(&driver_tx, ConnectionRole::Driver, None);

    // observer 纯 attach（无 hello）：不得触碰租约。
    let (observer_tx, _observer_rx) = mpsc::channel::<OutboundControl>(1);
    manager.register_attachment("observer_pending", observer_tx.clone());
    let snapshot = manager.lease_diagnostics_snapshot();
    assert_eq!(
        snapshot["holder"], "driver",
        "纯 attach 不得改变租约持有者（REQ-DLS-04），got {snapshot}"
    );
    assert!(
        manager.arbitrate("driver", &WsInMessage::Abort).is_ok(),
        "observer attach 后现任 driver 写面必须不受影响（REQ-DLS-04）"
    );

    // 随后显式 hello(Observer) 同样不偷（REQ-WCR-02/F1 语义保持）。
    manager.bind_role(&observer_tx, ConnectionRole::Observer, None);
    assert!(
        manager.arbitrate("driver", &WsInMessage::Abort).is_ok(),
        "observer hello 不得接管 driver lease"
    );
}

/// REQ-DLS-04 场景二：无 hello 首写经自愈放行——attach 后租约仍悬空
/// （holder=None），首条写消息必须经 REQ-DLS-01 自愈授予而非 attach 预取；
/// self_heal 打点是「经自愈而非 attach 抢占」的可定案证据。
#[tokio::test]
async fn attach_zero_effect_no_hello_first_write_grants_via_self_heal() {
    use crate::web::workspace_ws_types::WsInMessage;

    let session_id = "session_dls04_no_hello_self_heal";
    let diagnostics_path = std::env::temp_dir()
        .join(session_id)
        .join("lease-diagnostics.jsonl");
    let _ = std::fs::remove_file(&diagnostics_path);

    let manager = WorkspaceSessionManager::test_fixture(session_id);
    let (conn_tx, _conn_rx) = mpsc::channel::<OutboundControl>(1);
    manager.register_attachment("no_hello_connection", conn_tx);

    // attach 零效应：新会话首连后租约仍悬空。
    let snapshot = manager.lease_diagnostics_snapshot();
    assert!(
        snapshot["holder"].is_null(),
        "attach 不得预取租约（REQ-DLS-04），got {snapshot}"
    );

    // 无 hello 首写：经 REQ-DLS-01 自愈放行；重复写不因 epoch 漂移被拒。
    assert!(
        manager
            .arbitrate("no_hello_connection", &WsInMessage::Abort)
            .is_ok(),
        "holder=None 时无 hello 首写必须经自愈放行（REQ-DLS-01×04）"
    );
    assert!(
        manager
            .arbitrate("no_hello_connection", &WsInMessage::Abort)
            .is_ok(),
        "自愈授予后重复写不得被拒（lease_epoch 必须同步刷新）"
    );

    // 授予必须来自自愈而非 attach：self_heal 打点恰一次。
    let content = std::fs::read_to_string(&diagnostics_path).unwrap_or_default();
    let self_heal_count = content
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|event| {
            event["event"] == "self_heal" && event["connection_id"] == "no_hello_connection"
        })
        .count();
    assert_eq!(
        self_heal_count, 1,
        "首写授予必须经 REQ-DLS-01 自愈（self_heal 打点恰一次），实际 jsonl：{content}"
    );
}

/// REQ-DLS-04 场景三：无 hello 且他人持有时拒——attach 不再抢占，因此无
/// hello 连接在 holder=Some(其他活跃连接) 时的写面必须按既有语义拒绝
/// STALE（Err(Driver)），不产生连接争抢。
#[tokio::test]
async fn attach_zero_effect_no_hello_write_rejected_while_other_holds() {
    use crate::web::workspace_session::ConnectionRole;
    use crate::web::workspace_ws_types::WsInMessage;

    let manager = WorkspaceSessionManager::test_fixture("session_dls04_other_holds");
    let (driver_tx, _driver_rx) = mpsc::channel::<OutboundControl>(1);
    manager.register_attachment("current_driver", driver_tx.clone());
    manager.bind_role(&driver_tx, ConnectionRole::Driver, None);

    let (intruder_tx, _intruder_rx) = mpsc::channel::<OutboundControl>(1);
    manager.register_attachment("no_hello_intruder", intruder_tx);

    assert!(
        matches!(
            manager.arbitrate("no_hello_intruder", &WsInMessage::Abort),
            Err(ConnectionRole::Driver)
        ),
        "无 hello 连接不得经 attach 抢占：他人持有时写面必须拒绝 STALE（REQ-DLS-04）"
    );
    // 现任 driver 写面不受无 hello attach 影响。
    assert!(
        manager
            .arbitrate("current_driver", &WsInMessage::Abort)
            .is_ok(),
        "现任 holder 写面必须不受无 hello attach 影响（REQ-DLS-04）"
    );
}

/// REQ-DLS-04 场景四（不变性）：多 tab 驾驶切换——hello(driver) 仍是唯一的
/// 显式接管路径且无条件生效：接管后旧 holder 迟到写拒绝 STALE，新 holder
/// 写面放行（REQ-WCR-02 语义保持）。
#[tokio::test]
async fn attach_zero_effect_hello_driver_takes_over_unconditionally() {
    use crate::web::workspace_session::ConnectionRole;
    use crate::web::workspace_ws_types::WsInMessage;

    let manager = WorkspaceSessionManager::test_fixture("session_dls04_hello_takeover");
    let (first_tx, _first_rx) = mpsc::channel::<OutboundControl>(1);
    manager.register_attachment("first_tab", first_tx.clone());
    manager.bind_role(&first_tx, ConnectionRole::Driver, None);

    let (second_tx, _second_rx) = mpsc::channel::<OutboundControl>(1);
    manager.register_attachment("second_tab", second_tx.clone());
    manager.bind_role(&second_tx, ConnectionRole::Driver, None);

    let snapshot = manager.lease_diagnostics_snapshot();
    assert_eq!(
        snapshot["holder"], "second_tab",
        "hello(driver) 必须无条件接管（多 tab 驾驶切换），got {snapshot}"
    );
    assert!(
        matches!(
            manager.arbitrate("first_tab", &WsInMessage::Abort),
            Err(ConnectionRole::Driver)
        ),
        "被接管旧 holder 的迟到写必须拒绝 STALE"
    );
    assert!(
        manager.arbitrate("second_tab", &WsInMessage::Abort).is_ok(),
        "新 holder 写面必须放行"
    );
}

/// REQ-DLS-04 场景五（归一不变性）：无 role hello 保归一——wire 缺席 role
/// 经 normalize 归一为 Driver 并显式获取租约；不声明 role 的旧客户端在
/// attach 零效应形态下仍是合法接管路径。
#[tokio::test]
async fn attach_zero_effect_roleless_hello_normalizes_to_driver() {
    use crate::web::workspace_session::ConnectionRole;
    use crate::web::workspace_ws_types::WsInMessage;

    let manager = WorkspaceSessionManager::test_fixture("session_dls04_roleless_hello");
    let (incumbent_tx, _incumbent_rx) = mpsc::channel::<OutboundControl>(1);
    manager.register_attachment("incumbent", incumbent_tx.clone());
    manager.bind_role(&incumbent_tx, ConnectionRole::Driver, None);

    let (legacy_tx, _legacy_rx) = mpsc::channel::<OutboundControl>(1);
    manager.register_attachment("legacy_client", legacy_tx.clone());
    // wire 入口同款归一：role 缺席 → Driver（inbound.rs Hello 分支）。
    manager.bind_role(&legacy_tx, ConnectionRole::normalize(None), None);

    let snapshot = manager.lease_diagnostics_snapshot();
    assert_eq!(
        snapshot["holder"], "legacy_client",
        "无 role hello 必须归一为 Driver 并接管租约，got {snapshot}"
    );
    assert!(
        manager
            .arbitrate("legacy_client", &WsInMessage::Abort)
            .is_ok(),
        "归一接管后新 holder 写面必须放行"
    );
    assert!(
        matches!(
            manager.arbitrate("incumbent", &WsInMessage::Abort),
            Err(ConnectionRole::Driver)
        ),
        "被归一接管取代的旧 holder 写面必须拒绝 STALE"
    );
}
