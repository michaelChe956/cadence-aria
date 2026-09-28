// ============================================================================
// C1 Task 4 行 —— 孤儿候选门恢复（REQ-C1-GATE-01/02）。
// relay/WS consumer 缺席（fixture 无任何 WS attachment）：durable 候选事实
// 先落盘，REST CandidateRecovery 恢复原门；同 command 幂等、异 payload
// 409、旧 gate id 409；恢复后原门 approve 走完整 compile→Confirmed。
// 深层引擎语义（缺事实 fail-closed approve/预算不动）由引擎级
// orphaned_candidate_snapshot_requires_recovery_before_approve 持有。
// ============================================================================

/// C1（REQ-C1-GATE-02）：候选恢复命令幂等重放返回同一 durable 结果；
/// 同 command 异 payload fail-closed；旧 gate id 零副作用拒绝。
#[tokio::test]
async fn campaign_stage3_recovery_matrix_candidate_row_orphaned_snapshot_recovery() {
    let fixture = super::campaign_stage3_interactive::workspace_human_action_http_fixture(
        2,
        Vec::new(),
    )
    .await;
    let gate_id = fixture.active_gate_id().await.expect("durable gate node");

    // 恢复运行：accepted + 完整事实。
    let (status, body) = fixture
        .post_human_action(serde_json::json!({
            "type": "candidate_recovery",
            "command_id": "cmd-candidate-row-recover",
            "expected_gate_id": gate_id,
            "action": "recover",
        }))
        .await;
    assert_eq!(status, axum::http::StatusCode::OK, "body: {body}");
    assert_eq!(body["state"], "accepted", "body: {body}");
    assert_eq!(body["complete"], true, "body: {body}");
    assert_eq!(body["gate_id"], serde_json::json!(gate_id));
    assert_eq!(body["missing"], serde_json::json!([]));

    // durable：预算/ledger/turn 零增量（恢复动作只读评估 + label 落盘）。
    let record = fixture.session_record().await;
    assert_eq!(
        record
            .human_gate_snapshot
            .as_ref()
            .unwrap()
            .manual_repairs_remaining,
        2
    );
    assert!(record.provider_start_ledger.is_empty());
    assert!(record.status == WorkspaceSessionStatus::WaitingForHuman);

    // 同 command 同 payload 重放：同一 durable 结果（replayed）。
    let (status, body) = fixture
        .post_human_action(serde_json::json!({
            "type": "candidate_recovery",
            "command_id": "cmd-candidate-row-recover",
            "expected_gate_id": gate_id,
            "action": "recover",
        }))
        .await;
    assert_eq!(status, axum::http::StatusCode::OK, "body: {body}");
    assert_eq!(body["state"], "replayed", "body: {body}");
    assert_eq!(body["complete"], true, "body: {body}");

    // 同 command 异 payload：fail-closed 409。
    let (status, body) = fixture
        .post_human_action(serde_json::json!({
            "type": "candidate_recovery",
            "command_id": "cmd-candidate-row-recover",
            "expected_gate_id": gate_id,
            "action": "rebuild",
        }))
        .await;
    assert_eq!(status, axum::http::StatusCode::CONFLICT, "body: {body}");

    // 旧 gate id：门身份比对 409 零副作用。
    let (status, body) = fixture
        .post_human_action(serde_json::json!({
            "type": "candidate_recovery",
            "command_id": "cmd-candidate-row-stale-gate",
            "expected_gate_id": "timeline_node_stale",
            "action": "recover",
        }))
        .await;
    assert_eq!(status, axum::http::StatusCode::CONFLICT, "body: {body}");
    let unchanged = fixture.session_record().await;
    assert_eq!(unchanged.status, WorkspaceSessionStatus::WaitingForHuman);

    // 恢复后原门仍可用既有 approve：deterministic compile → Confirmed。
    let (status, body) = fixture
        .post_human_action(serde_json::json!({
            "type": "approve",
            "command_id": "cmd-candidate-row-approve",
            "expected_gate_id": gate_id,
        }))
        .await;
    assert_eq!(status, axum::http::StatusCode::OK, "body: {body}");
    let confirmed = fixture
        .await_session_status(WorkspaceSessionStatus::Confirmed)
        .await;
    assert_eq!(confirmed.status, WorkspaceSessionStatus::Confirmed);
}
