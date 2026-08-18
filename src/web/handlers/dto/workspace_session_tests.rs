use crate::product::models::{
    ProviderName, SessionOrigin, WorkspaceSessionStatus, WorkspaceType,
};

#[test]
fn workspace_session_summary_dto_exposes_group_chat_origin() {
    let record = WorkspaceSessionSummaryRecord {
        id: "workspace_session_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        entity_id: "story_spec_0001".to_string(),
        workspace_type: WorkspaceType::Story,
        status: WorkspaceSessionStatus::Open,
        author_provider: ProviderName::ClaudeCode,
        reviewer_provider: ProviderName::Codex,
        review_rounds: 1,
        superpowers_enabled: true,
        openspec_enabled: true,
        origin: Some(SessionOrigin::GroupChat),
    };

    let dto = workspace_session_summary_dto(&record);

    assert_eq!(dto.origin, Some(SessionOrigin::GroupChat));
    assert_eq!(serde_json::to_value(dto).unwrap()["origin"], "group_chat");
}
