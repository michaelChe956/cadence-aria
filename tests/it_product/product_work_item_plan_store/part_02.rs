fn sample_active_index() -> WorkItemPlanDraftActiveIndex {
    WorkItemPlanDraftActiveIndex {
        project_id: "project_1".to_string(),
        issue_id: "issue_1".to_string(),
        plan_id: "plan_1".to_string(),
        current_generation_round_id: "round_001".to_string(),
        outline_state: "confirmed".to_string(),
        active_outline_id: Some("outline_001".to_string()),
        outline_to_current_draft_id: BTreeMap::from([(
            "outline_001".to_string(),
            "draft_001".to_string(),
        )]),
        draft_statuses: BTreeMap::from([("draft_001".to_string(), WorkItemDraftStatus::Draft)]),
        batches: vec![WorkItemBatchRecord {
            batch_id: "batch_20260622_001".to_string(),
            generation_round_id: "round_001".to_string(),
            mode: WorkItemGenerationMode::Batch,
            item_draft_ids: vec!["draft_001".to_string()],
            status: WorkItemBatchStatus::Completed,
            validation_failed_ids: vec![],
            created_at: "2026-06-22T10:00:00Z".to_string(),
        }],
        updated_at: "2026-06-22T10:00:00Z".to_string(),
    }
}

fn sample_compile_transaction() -> WorkItemPlanCompileTransaction {
    WorkItemPlanCompileTransaction {
        compile_id: "compile_001".to_string(),
        project_id: "project_1".to_string(),
        issue_id: "issue_1".to_string(),
        plan_id: "plan_1".to_string(),
        generation_round_id: "round_001".to_string(),
        outline_version_ref: "artifact://outline/1".to_string(),
        active_draft_ids: vec!["draft_001".to_string()],
        status: WorkItemPlanCompileStatus::Preparing,
        plan_commit_state: WorkItemPlanCommitState::NotStarted,
        step_cursor: "start".to_string(),
        outline_to_work_item_id: BTreeMap::new(),
        outline_to_verification_plan_id: BTreeMap::new(),
        created_work_item_ids: vec![],
        created_verification_plan_ids: vec![],
        child_session_ids: vec![],
        validator_findings: vec![],
        abort_requested_at: None,
        failure_reason: None,
        previous_plan_snapshot: sample_plan(),
        created_at: "2026-06-22T10:00:00Z".to_string(),
        updated_at: "2026-06-22T10:00:00Z".to_string(),
        committed_at: None,
    }
}

fn sample_context_index() -> OutlineContextIndex {
    OutlineContextIndex {
        project_id: "project_1".to_string(),
        issue_id: "issue_1".to_string(),
        plan_id: "plan_1".to_string(),
        generation_round_id: "round_001".to_string(),
        blocker_resolutions: vec![OutlineContextBlockerResolution {
            blocker_node_id: "node_blocker_1".to_string(),
            resolution_node_id: "node_resolution_1".to_string(),
            resolution_artifact_ref:
                "context_blocker_resolution://node_blocker_1/node_resolution_1".to_string(),
            estimated_tokens: 120,
            created_at: "2026-06-22T10:00:00Z".to_string(),
            summary: None,
            merged_count: None,
        }],
        design_context_gaps: vec!["missing_test_strategy".to_string()],
        design_context_capabilities: DesignContextCapabilities {
            has_architecture: true,
            has_module_breakdown: true,
            has_tech_stack: true,
            has_test_strategy: false,
            has_key_paths: false,
        },
        updated_at: "2026-06-22T10:00:00Z".to_string(),
    }
}

fn sample_context_index_with_resolution_count(
    count: usize,
    estimated_tokens: u32,
) -> OutlineContextIndex {
    let mut index = sample_context_index();
    index.blocker_resolutions = (1..=count)
        .map(|number| OutlineContextBlockerResolution {
            blocker_node_id: format!("node_blocker_{number:03}"),
            resolution_node_id: format!("node_resolution_{number:03}"),
            resolution_artifact_ref: format!(
                "context_blocker_resolution://node_blocker_{number:03}/node_resolution_{number:03}"
            ),
            estimated_tokens,
            created_at: format!("2026-06-22T10:{number:02}:00Z"),
            summary: None,
            merged_count: None,
        })
        .collect();
    index
}

fn sample_plan() -> IssueWorkItemPlan {
    IssueWorkItemPlan {
        id: "plan_1".to_string(),
        project_id: "project_1".to_string(),
        issue_id: "issue_1".to_string(),
        source_story_spec_ids: vec!["story_1".to_string()],
        source_design_spec_ids: vec!["design_1".to_string()],
        options: IssueWorkItemPlanOptions {
            include_integration_tests: true,
            include_e2e_tests: false,
            force_frontend_backend_split: false,
            require_execution_plan_confirm: true,
        },
        status: IssueWorkItemPlanStatus::Draft,
        work_item_ids: vec![],
        repository_profile_ref: None,
        verification_plan_ids: vec![],
        dependency_graph: vec![],
        created_from_provider_run: None,
        validator_findings: vec![],
        review_summary: None,
        created_at: "2026-06-22T10:00:00Z".to_string(),
        updated_at: "2026-06-22T10:00:00Z".to_string(),
    }
}
