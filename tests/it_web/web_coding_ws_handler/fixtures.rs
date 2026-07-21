fn seed_group_revision_history_fixture(lifecycle: &LifecycleStore, session_id: &str) {
    let entries = [
        ("work_item_0001", "work_item_revision_0001"),
        ("work_item_0002", "work_item_revision_0002"),
    ]
    .into_iter()
    .map(|(logical_work_item_id, revision_id)| WorkItemHistoryEntryDto {
        kind: WorkItemHistoryEntryKind::WorkItemRevision,
        id: revision_id.to_string(),
        logical_work_item_id: logical_work_item_id.to_string(),
        related_revision_id: Some(format!("draft_{revision_id}")),
        summary: format!("Compiled WorkItem revision from draft_{revision_id}"),
        created_at: "2026-07-18T00:00:00Z".to_string(),
    })
    .collect();
    lifecycle
        .save_artifact_versions(
            session_id,
            &[ArtifactVersion {
                version: 1,
                payload: ArtifactPayload::WorkItemRevisionHistory {
                    history: Box::new(WorkItemRevisionHistoryDto { entries }),
                },
                generated_by: ProviderName::Fake,
                reviewed_by: None,
                review_verdict: None,
                confirmed_by: None,
                is_current: true,
                created_at: "2026-07-18T00:00:00Z".to_string(),
                source_node_id: "timeline_node_compile".to_string(),
            }],
        )
        .expect("save work item revision history");
}
