#[tokio::test]
async fn coding_runtime_handoff_revision_history_refresh_uses_real_run_and_handoff_artifacts() {
    let fixture = runtime_handoff_fixture(
        RuntimeContractChange::CompatibleExtension,
        CodingUnitRunStatus::AwaitingAmendment,
    );
    let lifecycle = LifecycleStore::new(fixture.store.paths());
    let session = lifecycle
        .create_workspace_session(
            crate::product::lifecycle_store::CreateWorkspaceSessionInput {
                project_id: fixture.attempt.project_id.clone(),
                issue_id: fixture.attempt.issue_id.clone(),
                entity_id: "work_item_plan_0001".to_string(),
                workspace_type: crate::product::models::WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Codex,
                reviewer_provider: ProviderName::ClaudeCode,
                review_rounds: 1,
                superpowers_enabled: true,
                openspec_enabled: true,
            },
        )
        .expect("plan workspace session");
    lifecycle
        .save_artifact_versions(
            &session.id,
            &[crate::web::workspace_ws_types::ArtifactVersion {
                version: 1,
                payload: crate::web::workspace_ws_types::ArtifactPayload::WorkItemRevisionHistory {
                    history: Box::new(
                        crate::web::workspace_ws_types::WorkItemRevisionHistoryDto {
                            entries: vec![
                                crate::web::workspace_ws_types::WorkItemHistoryEntryDto {
                                    kind: crate::web::workspace_ws_types::WorkItemHistoryEntryKind::WorkItemRevision,
                                    id: "work_item_revision_wi02_v1".to_string(),
                                    logical_work_item_id: "wi_registration".to_string(),
                                    related_revision_id: None,
                                    summary: "Compiled WorkItem revision".to_string(),
                                    created_at: "2026-07-20T00:00:00Z".to_string(),
                                },
                            ],
                        },
                    ),
                },
                generated_by: ProviderName::Codex,
                reviewed_by: None,
                review_verdict: None,
                confirmed_by: None,
                is_current: false,
                created_at: "2026-07-20T00:00:00Z".to_string(),
                source_node_id: "timeline_node_compile".to_string(),
            }],
        )
        .expect("initial history artifact");

    fixture
        .engine
        .apply_completed_handoff(&fixture.attempt, &fixture.next_handoff)
        .await
        .expect("runtime impact");
    let history = crate::web::workspace_ws_handler::refresh_coding_runtime_revision_history(
        &fixture.store.paths(),
        &fixture.attempt,
    )
    .expect("runtime history refresh");
    let replayed = crate::web::workspace_ws_handler::refresh_coding_runtime_revision_history(
        &fixture.store.paths(),
        &fixture.attempt,
    )
    .expect("runtime history replay");

    assert_eq!(history, replayed);
    assert!(history.entries.iter().any(|entry| {
        entry.kind == crate::web::workspace_ws_types::WorkItemHistoryEntryKind::UnitRun
            && entry.id.starts_with("coding_unit_run_")
            && entry.logical_work_item_id == "wi_registration"
            && entry.related_revision_id.as_deref() == Some("work_item_revision_wi02_v1")
    }));
    assert!(history.entries.iter().any(|entry| {
        entry.kind == crate::web::workspace_ws_types::WorkItemHistoryEntryKind::HandoffRevision
            && entry.id == "handoff_revision_0002"
            && entry.logical_work_item_id == "wi_core"
            && entry.related_revision_id.as_deref() == Some("work_item_revision_wi01_v2")
    }));
    let persisted = lifecycle
        .list_artifact_versions(&session.id)
        .expect("persisted history")
        .into_iter()
        .find_map(|version| match version.payload {
            crate::web::workspace_ws_types::ArtifactPayload::WorkItemRevisionHistory { history } => {
                Some(*history)
            }
            _ => None,
        })
        .expect("history payload");
    assert_eq!(persisted, history);
}
