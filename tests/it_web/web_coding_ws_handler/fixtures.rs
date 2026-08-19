/// 旧单仓 coding fixture 的生产路径现在会通过 `RepositoryStore::for_project`
/// 读取项目记录。各 fixture 会先写入 repository / issue 等旧数据，故不能依赖
/// `ProjectStore::create` 的目录计数；这里直接为既有 `project_0001` 路径持久化
/// 最小 ProjectRecord，确保旧数据路径仍明确为单仓。
fn seed_legacy_project_fixture(paths: &ProductAppPaths) {
    let project_path = paths.project_root("project_0001").join("project.json");
    if project_path.exists() {
        let _project: cadence_aria::product::models::ProjectRecord =
            cadence_aria::product::json_store::read_json(&project_path)
                .expect("read legacy coding fixture project");
        assert!(
            !cadence_aria::product::logical_codebase::LogicalCodebaseStore::new(paths.clone())
                .has_any_storage("project_0001")
                .expect("probe legacy coding fixture storage"),
            "legacy coding fixture project must remain single-repository"
        );
        return;
    }

    cadence_aria::product::json_store::write_json(
        &project_path,
        &cadence_aria::product::models::ProjectRecord {
            id: "project_0001".to_string(),
            name: "legacy coding websocket fixture".to_string(),
            description: None,
            created_at: "2026-08-18T00:00:00Z".to_string(),
            updated_at: "2026-08-18T00:00:00Z".to_string(),
            last_opened_at: None,
        },
    )
    .expect("persist legacy coding fixture project");
}

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
