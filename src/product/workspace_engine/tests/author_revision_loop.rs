// spec-design-dialog-revision T2：provisional reviewer 快照持久化闭环
// serde 兼容（旧 record 无新字段反序列化）+ from_record 恢复用例
use crate::product::models::{ProviderName, WorkspaceSessionRecord};

#[test]
fn legacy_session_record_without_provisional_fields_deserializes() {
    let legacy = serde_json::json!({
        "id": "s1", "project_id": "p1", "issue_id": "i1", "entity_id": "e1",
        "workspace_type": "story", "status": "open",
        "author_provider": "claude_code", "reviewer_provider": "codex", "review_rounds": 1,
        "superpowers_enabled": true, "openspec_enabled": true,
        "messages": [], "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z"
    });
    let record: WorkspaceSessionRecord = serde_json::from_value(legacy).unwrap();
    assert_eq!(record.provisional_reviewer_provider, None);
    assert_eq!(record.reviewer_enabled_at_start, None);
}

#[test]
fn provisional_fields_roundtrip() {
    let record = WorkspaceSessionRecord {
        provisional_reviewer_provider: Some(ProviderName::Codex),
        reviewer_enabled_at_start: Some(false),
        ..serde_json::from_value(serde_json::json!({
            "id": "s2", "project_id": "p1", "issue_id": "i1", "entity_id": "e1",
            "workspace_type": "story", "status": "open",
            "author_provider": "claude_code", "reviewer_provider": "claude_code", "review_rounds": 1,
            "superpowers_enabled": true, "openspec_enabled": true,
            "messages": [], "created_at": "t", "updated_at": "t"
        })).unwrap()
    };
    let json = serde_json::to_value(&record).unwrap();
    assert_eq!(json["provisional_reviewer_provider"], "codex");
    assert_eq!(json["reviewer_enabled_at_start"], false);
    let back: WorkspaceSessionRecord = serde_json::from_value(json).unwrap();
    assert_eq!(
        back.provisional_reviewer_provider,
        Some(ProviderName::Codex)
    );
    assert_eq!(back.reviewer_enabled_at_start, Some(false));
}

#[test]
fn from_record_restores_provisional_fields() {
    use crate::product::workspace_engine::types::WorkspaceSession;
    let record: WorkspaceSessionRecord = serde_json::from_value(serde_json::json!({
        "id": "s3", "project_id": "p1", "issue_id": "i1", "entity_id": "e1",
        "workspace_type": "story", "status": "open",
        "author_provider": "claude_code", "reviewer_provider": "codex", "review_rounds": 1,
        "superpowers_enabled": true, "openspec_enabled": true,
        "messages": [], "created_at": "t", "updated_at": "t",
        "provisional_reviewer_provider": "codex",
        "reviewer_enabled_at_start": false
    }))
    .unwrap();
    let session = WorkspaceSession::from_record(record);
    assert_eq!(
        session.provisional_reviewer_provider,
        Some(ProviderName::Codex)
    );
    assert_eq!(session.reviewer_enabled_at_start, Some(false));
}
