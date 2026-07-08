use super::*;

#[test]
fn tester_execution_context_uses_refs_not_full_spec_markdown() {
    let tmp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(tmp.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());

    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            title: "Story".to_string(),
        })
        .unwrap();
    lifecycle
        .append_version(AppendSpecVersionInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            entity_id: story.id.clone(),
            markdown: "# Story\n\n完整 Story Spec 正文\n\n[REQ-001]".to_string(),
            provider_run_refs: vec!["author_run_story".to_string()],
            review_refs: Vec::new(),
            confirmed_by: Some("user".to_string()),
        })
        .unwrap();

    let design = lifecycle
        .create_design_spec(CreateDesignSpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_ids: vec![story.id.clone()],
            title: "Design".to_string(),
        })
        .unwrap();
    lifecycle
        .append_version(AppendSpecVersionInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            entity_id: design.id.clone(),
            markdown: "# Design\n\n完整 Design Spec 正文\n\n[DEC-001]".to_string(),
            provider_run_refs: vec!["author_run_design".to_string()],
            review_refs: Vec::new(),
            confirmed_by: Some("user".to_string()),
        })
        .unwrap();

    let work_item = lifecycle
        .create_work_item(CreateWorkItemInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            story_spec_ids: vec![story.id.clone()],
            design_spec_ids: vec![design.id.clone()],
            title: "Work Item".to_string(),
            ..Default::default()
        })
        .unwrap();

    let attempt = CodingExecutionAttempt {
        id: "coding_attempt_0001".to_string(),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        work_item_id: work_item.id.clone(),
        attempt_no: 1,
        scope: crate::product::coding_models::CodingAttemptScope::WorkItem,
        status: CodingAttemptStatus::Running,
        stage: CodingExecutionStage::Testing,
        base_branch: "main".to_string(),
        branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
        worktree_path: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::Codex,
            reviewer: Some(ProviderName::ClaudeCode),
            review_rounds: 1,
        },
        rework_count: 0,
        max_auto_rework: 2,
        work_item_group_id: None,
        current_work_item_id: Some(work_item.id),
        active_unit_id: None,
        head_commit: None,
        pushed_remote: None,
        review_request_id: None,
        provider_conversations: Vec::new(),
        created_at: "2026-06-10T00:00:00Z".to_string(),
        updated_at: "2026-06-10T00:00:00Z".to_string(),
        completed_at: None,
    };

    let pack = build_tester_execution_context_pack(paths, &attempt).expect("context");
    let json = serde_json::to_string_pretty(&pack).expect("json");

    assert!(json.contains(&story.id));
    assert!(json.contains(&design.id));
    assert!(json.contains("source_artifacts"));
    assert!(json.contains("changed_files"));
    assert!(!json.contains("raw_markdown_or_sections"));
    assert!(!json.contains("完整 Story Spec 正文"));
    assert!(!json.contains("完整 Design Spec 正文"));
}
