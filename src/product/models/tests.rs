use crate::product::models::{
    AgentRole, ArtifactRef, NodeDetail, PermissionEvent, ProviderSnapshot,
};
use crate::web::workspace_ws_types::{TimelineNodeStatus, TimelineNodeType};

#[test]
fn node_detail_roundtrip() {
    let detail = NodeDetail {
        node_id: "node-1".to_string(),
        session_id: "sess-1".to_string(),
        node_type: TimelineNodeType::AuthorRun,
        status: TimelineNodeStatus::Completed,
        agent_role: Some(AgentRole::Author),
        provider: Some(ProviderSnapshot {
            name: "claude_code".to_string(),
            model: "claude-opus-4-7".to_string(),
        }),
        prompt: Some("Workspace 类型: Story Spec".to_string()),
        messages: vec![],
        streaming_content: "输出内容".to_string(),
        execution_events: vec![],
        permission_events: vec![PermissionEvent {
            request_id: "perm-1".to_string(),
            request: serde_json::json!({"tool": "shell"}),
            response: Some(serde_json::json!({"approved": true})),
            ts: "2026-05-20T14:35:00Z".to_string(),
        }],
        verdict: None,
        artifact_ref: Some(ArtifactRef {
            artifact_id: "art-1".to_string(),
            version: 2,
        }),
        is_revision: false,
        base_artifact_ref: None,
        started_at: "2026-05-20T14:30:00Z".to_string(),
        ended_at: Some("2026-05-20T14:35:00Z".to_string()),
    };

    let json = serde_json::to_value(&detail).unwrap();
    let back: NodeDetail = serde_json::from_value(json).unwrap();

    assert_eq!(back.node_id, detail.node_id);
    assert_eq!(back.prompt, detail.prompt);
    assert_eq!(back.permission_events.len(), 1);
}
