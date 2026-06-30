use cadence_aria::product::models::{
    LifecycleWorkItemRecord, WorkItemContextBudget, WorkItemExecutionPlanStatus, WorkItemKind,
    WorkItemPlanStatus, WorkItemStatus,
};

#[test]
fn lifecycle_work_item_deserializes_legacy_json_with_split_defaults() {
    let json = serde_json::json!({
        "id": "work_item_0001",
        "project_id": "project_0001",
        "issue_id": "issue_0001",
        "repository_id": "repo_0001",
        "story_spec_ids": ["story_spec_0001"],
        "design_spec_ids": ["design_spec_0001"],
        "title": "Implement backend API",
        "plan_status": "confirmed",
        "execution_status": "pending",
        "worktree_path": null,
        "created_at": "2026-06-16T00:00:00Z",
        "updated_at": "2026-06-16T00:00:00Z"
    });

    let record: LifecycleWorkItemRecord =
        serde_json::from_value(json).expect("legacy lifecycle work item should deserialize");

    assert_eq!(record.kind, WorkItemKind::Other);
    assert_eq!(record.work_item_set_id, None);
    assert_eq!(record.sequence_hint, None);
    assert!(record.depends_on.is_empty());
    assert!(record.exclusive_write_scopes.is_empty());
    assert!(record.forbidden_write_scopes.is_empty());
    assert_eq!(record.context_budget, WorkItemContextBudget::default());
    assert!(record.required_handoff_from.is_empty());
    assert_eq!(record.verification_plan_ref, None);
    assert!(!record.require_execution_plan_confirm);
    assert_eq!(
        record.execution_plan_status,
        WorkItemExecutionPlanStatus::NotStarted
    );
    assert_eq!(record.handoff_summary_ref, None);
    assert_eq!(record.completion_commit, None);
    assert_eq!(record.completion_diff_summary_ref, None);
}

#[test]
fn work_item_context_budget_defaults_to_single_session_budget_proxy() {
    let budget = WorkItemContextBudget::default();

    assert_eq!(budget.target_context_k, "30-50");
    assert_eq!(budget.max_summary_chars, 20_000);
    assert_eq!(budget.max_handoff_chars, 12_000);
    assert_eq!(budget.max_code_context_chars, 30_000);
    assert_eq!(budget.max_context_file_refs, 80);
    assert_eq!(budget.max_traceability_refs, 40);
    assert_eq!(budget.max_dependency_handoffs, 3);
}

#[test]
fn lifecycle_work_item_serializes_new_split_fields_as_snake_case() {
    let record = LifecycleWorkItemRecord {
        id: "work_item_0002".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        repository_id: "repo_0001".to_string(),
        story_spec_ids: vec!["story_spec_0001".to_string()],
        design_spec_ids: vec!["design_spec_0001".to_string()],
        title: "Backend API".to_string(),
        plan_status: WorkItemPlanStatus::Confirmed,
        execution_status: WorkItemStatus::Pending,
        worktree_path: None,
        work_item_set_id: Some("work_item_set_0001".to_string()),
        source_work_item_plan_id: Some("issue_work_item_plan_0001".to_string()),
        source_outline_id: Some("outline_backend".to_string()),
        source_draft_id: Some("draft_backend".to_string()),
        planned_implementation_context: Some("实现 Backend API".to_string()),
        planned_handoff_summary: Some("交付 Backend API contract".to_string()),
        kind: WorkItemKind::Backend,
        sequence_hint: Some(10),
        depends_on: vec!["work_item_0001".to_string()],
        exclusive_write_scopes: vec!["src/product/**".to_string()],
        forbidden_write_scopes: vec!["web/**".to_string()],
        context_budget: WorkItemContextBudget::default(),
        required_handoff_from: vec!["work_item_0001".to_string()],
        verification_plan_ref: Some("verification_plan_work_item_0002".to_string()),
        require_execution_plan_confirm: true,
        execution_plan_status: WorkItemExecutionPlanStatus::Draft,
        handoff_summary_ref: Some("handoffs/work_item_0001.json".to_string()),
        completion_commit: Some("abc123".to_string()),
        completion_diff_summary_ref: Some("diffs/work_item_0002.json".to_string()),
        created_at: "2026-06-16T00:00:00Z".to_string(),
        updated_at: "2026-06-16T00:00:00Z".to_string(),
    };

    let value = serde_json::to_value(record).expect("serialize lifecycle work item");

    assert_eq!(value["kind"], "backend");
    assert_eq!(value["execution_plan_status"], "draft");
    assert_eq!(
        value["verification_plan_ref"],
        "verification_plan_work_item_0002"
    );
    assert_eq!(value["work_item_set_id"], "work_item_set_0001");
    assert_eq!(
        value["source_work_item_plan_id"],
        "issue_work_item_plan_0001"
    );
    assert_eq!(value["source_outline_id"], "outline_backend");
    assert_eq!(value["source_draft_id"], "draft_backend");
    assert_eq!(value["planned_implementation_context"], "实现 Backend API");
    assert_eq!(
        value["planned_handoff_summary"],
        "交付 Backend API contract"
    );
    assert_eq!(value["depends_on"], serde_json::json!(["work_item_0001"]));
    assert_eq!(
        value["exclusive_write_scopes"],
        serde_json::json!(["src/product/**"])
    );
    assert_eq!(
        value["forbidden_write_scopes"],
        serde_json::json!(["web/**"])
    );
    assert_eq!(value["require_execution_plan_confirm"], true);
}
