use super::*;

#[test]
fn retry_interrupted_run_message_uses_stable_shape() {
    let message: WsInMessage = serde_json::from_value(serde_json::json!({
        "type": "retry_interrupted_run",
        "failed_node_id": "timeline_node_054"
    }))
    .expect("retry interrupted run message");

    assert!(matches!(
        message,
        WsInMessage::RetryInterruptedRun { failed_node_id }
            if failed_node_id == "timeline_node_054"
    ));
}
