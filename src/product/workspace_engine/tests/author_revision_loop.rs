// spec-design-dialog-revision T2：provisional reviewer 快照持久化闭环
// serde 兼容（旧 record 无新字段反序列化）+ from_record 恢复用例 + start_generation 写入路径用例（T2 fix1）
use super::*;
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

// T2 fix1：start_generation 写入路径断言。
// reviewer_enabled=false 时：provisional 必须保留快照【原始】reviewer 选择（Critical-1），
// 同时 reviewer_provider/review_rounds 仍被清空（默认行为不变）。
#[tokio::test]
async fn start_generation_disabled_review_keeps_provisional_snapshot() {
    let (_tmp, store) = setup();
    let (tx, _rx) = mpsc::channel(64);
    let session = make_session("sess_provisional_disabled");
    let mut engine = WorkspaceEngine::new(store, tx, session);
    let snapshot = ProviderConfigSnapshot {
        author: ProviderName::Codex,
        reviewer: Some(ProviderName::Codex),
        review_rounds: 1,
        permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
    };

    engine
        .start_generation(snapshot, false)
        .await
        .expect("start generation");

    let session = engine.session();
    assert_eq!(
        session.provisional_reviewer_provider,
        Some(ProviderName::Codex),
        "provisional must retain the raw snapshot reviewer even when review is disabled"
    );
    assert_eq!(
        session.reviewer_provider, None,
        "disabled review must still clear reviewer_provider"
    );
    assert_eq!(
        session.review_rounds, 0,
        "disabled review must still zero review_rounds"
    );
    assert_eq!(session.reviewer_enabled_at_start, Some(false));
}

// T2 fix1 对照：reviewer_enabled=true 时 provisional == reviewer_provider == 快照 reviewer。
#[tokio::test]
async fn start_generation_enabled_review_sets_provisional_and_reviewer() {
    let (_tmp, store) = setup();
    let (tx, _rx) = mpsc::channel(64);
    let session = make_session("sess_provisional_enabled");
    let mut engine = WorkspaceEngine::new(store, tx, session);
    let snapshot = ProviderConfigSnapshot {
        author: ProviderName::Codex,
        reviewer: Some(ProviderName::Codex),
        review_rounds: 1,
        permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
    };

    engine
        .start_generation(snapshot, true)
        .await
        .expect("start generation");

    let session = engine.session();
    assert_eq!(
        session.provisional_reviewer_provider,
        Some(ProviderName::Codex),
        "provisional must equal the snapshot reviewer"
    );
    assert_eq!(
        session.reviewer_provider,
        Some(ProviderName::Codex),
        "enabled review keeps reviewer_provider"
    );
    assert_eq!(session.review_rounds, 1);
    assert_eq!(session.reviewer_enabled_at_start, Some(true));
}
