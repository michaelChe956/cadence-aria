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

// spec-design-dialog-revision T3：AuthorConfirm 对话式修订循环 + 确认双出口决策用例。
// 引擎构造：Story/AuthorConfirm + artifact + 不同 provisional / enabled_at_start / rounds 组合。

async fn author_confirm_engine() -> WorkspaceEngine {
    // reviewer_provider=None, rounds=0, provisional=Some(Codex), enabled_at_start=Some(false)
    let (_tmp, store) = setup();
    let (tx, _rx) = mpsc::channel(64);
    let mut session = make_session("sess_author_confirm");
    session.stage = WorkspaceStage::AuthorConfirm;
    session.workspace_type = WorkspaceType::Story;
    session.artifact = Some(ArtifactPayload::Markdown {
        markdown: "# Story Spec\n\n候选内容".to_string(),
        diff: None,
    });
    session.reviewer_provider = None;
    session.review_rounds = 0;
    session.provisional_reviewer_provider = Some(ProviderName::Codex);
    session.reviewer_enabled_at_start = Some(false);
    WorkspaceEngine::new(store, tx, session)
}

async fn author_confirm_engine_no_provisional() -> WorkspaceEngine {
    // 与 author_confirm_engine 相同，但 provisional=None（创建时未保留 reviewer 选择）。
    let (_tmp, store) = setup();
    let (tx, _rx) = mpsc::channel(64);
    let mut session = make_session("sess_author_confirm_no_provisional");
    session.stage = WorkspaceStage::AuthorConfirm;
    session.workspace_type = WorkspaceType::Story;
    session.artifact = Some(ArtifactPayload::Markdown {
        markdown: "# Story Spec\n\n候选内容".to_string(),
        diff: None,
    });
    session.reviewer_provider = None;
    session.review_rounds = 0;
    session.provisional_reviewer_provider = None;
    session.reviewer_enabled_at_start = Some(false);
    WorkspaceEngine::new(store, tx, session)
}

async fn author_confirm_engine_provisional_restored() -> WorkspaceEngine {
    // reviewer_enabled_at_start=Some(false) + provisional 已恢复（rounds=1）→ Accept 仍定稿。
    let (_tmp, store) = setup();
    let (tx, _rx) = mpsc::channel(64);
    let mut session = make_session("sess_author_confirm_provisional_restored");
    session.stage = WorkspaceStage::AuthorConfirm;
    session.workspace_type = WorkspaceType::Story;
    session.artifact = Some(ArtifactPayload::Markdown {
        markdown: "# Story Spec\n\n候选内容".to_string(),
        diff: None,
    });
    session.reviewer_provider = Some(ProviderName::Codex);
    session.review_rounds = 1;
    session.provisional_reviewer_provider = Some(ProviderName::Codex);
    session.reviewer_enabled_at_start = Some(false);
    WorkspaceEngine::new(store, tx, session)
}

async fn author_confirm_engine_legacy_record() -> WorkspaceEngine {
    // 旧记录：reviewer_enabled_at_start=None（未落盘）→ 按有效态判定。
    let (_tmp, store) = setup();
    let (tx, _rx) = mpsc::channel(64);
    let mut session = make_session("sess_author_confirm_legacy");
    session.stage = WorkspaceStage::AuthorConfirm;
    session.workspace_type = WorkspaceType::Story;
    session.artifact = Some(ArtifactPayload::Markdown {
        markdown: "# Story Spec\n\n候选内容".to_string(),
        diff: None,
    });
    session.reviewer_provider = Some(ProviderName::Codex);
    session.review_rounds = 1;
    session.provisional_reviewer_provider = None;
    session.reviewer_enabled_at_start = None;
    WorkspaceEngine::new(store, tx, session)
}

#[tokio::test]
async fn revise_with_feedback_transitions_to_revision() {
    let mut engine = author_confirm_engine().await;
    let outcome = engine
        .handle_author_decision(AuthorDecision::Revise {
            feedback: "补充异常场景".into(),
        })
        .await
        .unwrap();
    assert_eq!(
        outcome,
        AuthorDecisionOutcome::StartRevision {
            feedback: "补充异常场景".into()
        }
    );
    assert_eq!(engine.session().stage, WorkspaceStage::Revision);
    assert_eq!(
        engine.pending_revision_context.as_deref(),
        Some("补充异常场景")
    );
    assert!(engine.session().artifact.is_some(), "反馈修订不得清空产物");
}

#[tokio::test]
async fn revise_with_blank_feedback_rejected() {
    let mut engine = author_confirm_engine().await;
    let err = engine
        .handle_author_decision(AuthorDecision::Revise { feedback: "  ".into() })
        .await
        .unwrap_err();
    assert!(err.contains("feedback"));
    assert_eq!(engine.session().stage, WorkspaceStage::AuthorConfirm);
}

#[tokio::test]
async fn reject_returns_guidance_error_without_reset() {
    let mut engine = author_confirm_engine().await;
    let err = engine
        .handle_author_decision(AuthorDecision::Reject)
        .await
        .unwrap_err();
    assert!(err.contains("反馈"), "引导改用反馈修订: {err}");
    assert_eq!(engine.session().stage, WorkspaceStage::AuthorConfirm);
    assert!(engine.session().artifact.is_some());
}

#[tokio::test]
async fn accept_finalize_completes_workspace() {
    let mut engine = author_confirm_engine().await;
    let outcome = engine
        .handle_author_decision(AuthorDecision::AcceptFinalize)
        .await
        .unwrap();
    assert_eq!(outcome, AuthorDecisionOutcome::Finalized);
    assert_eq!(engine.session().stage, WorkspaceStage::Completed);
}

#[tokio::test]
async fn accept_with_review_restores_provisional_when_disabled() {
    // reviewer_provider=None, rounds=0, provisional=Some(Codex)
    let mut engine = author_confirm_engine().await;
    let outcome = engine
        .handle_author_decision(AuthorDecision::AcceptWithReview)
        .await
        .unwrap();
    assert!(matches!(outcome, AuthorDecisionOutcome::StartReview));
    assert_eq!(engine.session().reviewer_provider, Some(ProviderName::Codex));
    assert_eq!(engine.session().review_rounds, 1);
    assert_eq!(engine.session().stage, WorkspaceStage::CrossReview);
}

#[tokio::test]
async fn accept_with_review_errors_without_provisional() {
    let mut engine = author_confirm_engine_no_provisional().await;
    let err = engine
        .handle_author_decision(AuthorDecision::AcceptWithReview)
        .await
        .unwrap_err();
    assert!(err.contains("reviewer"), "{err}");
    assert_eq!(engine.session().stage, WorkspaceStage::AuthorConfirm);
}

#[tokio::test]
async fn legacy_accept_routes_by_enabled_at_start() {
    // reviewer_enabled_at_start=Some(false) + provisional 已恢复(rounds=1) → Accept 仍定稿（按创建默认值）
    let mut engine = author_confirm_engine_provisional_restored().await;
    let outcome = engine
        .handle_author_decision(AuthorDecision::Accept)
        .await
        .unwrap();
    assert_eq!(outcome, AuthorDecisionOutcome::Finalized);
    // 旧记录（None）按有效态：rounds>0 && reviewer.is_some() → StartReview
    let mut legacy = author_confirm_engine_legacy_record().await;
    let outcome2 = legacy
        .handle_author_decision(AuthorDecision::Accept)
        .await
        .unwrap();
    assert!(matches!(outcome2, AuthorDecisionOutcome::StartReview));
}
