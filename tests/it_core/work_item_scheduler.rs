use cadence_aria::product::models::{
    LifecycleWorkItemRecord, WorkItemContextBudget, WorkItemExecutionPlanStatus, WorkItemKind,
    WorkItemPlanStatus, WorkItemStatus,
};
use cadence_aria::product::worktree_scheduler::{ReadyDecision, ready_work_items};

fn work_item(id: &str, depends_on: Vec<&str>, scope: Vec<&str>) -> LifecycleWorkItemRecord {
    LifecycleWorkItemRecord {
        id: id.to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        repository_id: "repo_0001".to_string(),
        story_spec_ids: vec!["story_spec_0001".to_string()],
        design_spec_ids: vec!["design_spec_0001".to_string()],
        title: id.to_string(),
        plan_status: WorkItemPlanStatus::Confirmed,
        execution_status: WorkItemStatus::Pending,
        worktree_path: None,
        work_item_set_id: Some("work_item_set_0001".to_string()),
        source_work_item_plan_id: None,
        source_outline_id: None,
        source_draft_id: None,
        planned_implementation_context: None,
        planned_handoff_summary: None,
        kind: WorkItemKind::Backend,
        sequence_hint: None,
        depends_on: depends_on.into_iter().map(str::to_string).collect(),
        exclusive_write_scopes: scope.into_iter().map(str::to_string).collect(),
        forbidden_write_scopes: Vec::new(),
        context_budget: WorkItemContextBudget::default(),
        required_handoff_from: Vec::new(),
        verification_plan_ref: None,
        require_execution_plan_confirm: false,
        execution_plan_status: WorkItemExecutionPlanStatus::NotStarted,
        handoff_summary_ref: None,
        completion_commit: None,
        completion_diff_summary_ref: None,
        created_at: "2026-06-16T00:00:00Z".to_string(),
        updated_at: "2026-06-16T00:00:00Z".to_string(),
    }
}

#[test]
fn blocks_items_with_unfinished_dependencies_and_overlapping_scope() {
    let items = vec![
        work_item("wi_001", vec![], vec!["src/auth/**"]),
        work_item("wi_002", vec!["wi_001"], vec!["src/api/**"]),
        work_item("wi_003", vec![], vec!["src/auth/login.rs"]),
    ];
    let decisions = ready_work_items(&items, &[], &["src/auth/**".to_string()]);

    assert_eq!(
        decisions.get("wi_001"),
        Some(&ReadyDecision::WaitingForScope)
    );
    assert_eq!(
        decisions.get("wi_002"),
        Some(&ReadyDecision::WaitingForDependency)
    );
    assert_eq!(
        decisions.get("wi_003"),
        Some(&ReadyDecision::WaitingForScope)
    );
}

#[test]
fn marks_pending_items_ready_when_dependencies_complete_and_scope_free() {
    let items = vec![
        work_item("wi_001", vec![], vec!["src/product/models.rs"]),
        work_item(
            "wi_002",
            vec!["wi_001"],
            vec!["src/product/worktree_scheduler.rs"],
        ),
    ];
    let decisions = ready_work_items(&items, &["wi_001".to_string()], &[]);

    assert_eq!(decisions.get("wi_001"), Some(&ReadyDecision::Ready));
    assert_eq!(decisions.get("wi_002"), Some(&ReadyDecision::Ready));
}

#[test]
fn non_pending_lifecycle_items_are_not_ready() {
    let mut item = work_item("wi_001", vec![], vec!["src/product/**"]);
    item.execution_status = WorkItemStatus::Coding;

    let decisions = ready_work_items(&[item], &[], &[]);

    assert_eq!(decisions.get("wi_001"), Some(&ReadyDecision::NotPending));
}
