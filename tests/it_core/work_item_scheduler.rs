use cadence_aria::product::models::{ExecutionMode, WorkItemRecord, WorkItemStatus};
use cadence_aria::product::worktree_scheduler::{ReadyDecision, ready_work_items};

fn work_item(id: &str, depends_on: Vec<&str>, scope: Vec<&str>) -> WorkItemRecord {
    WorkItemRecord {
        id: id.to_string(),
        issue_id: "issue_0001".to_string(),
        repo_id: "repo_0001".to_string(),
        title: id.to_string(),
        allowed_write_scope: scope.into_iter().map(str::to_string).collect(),
        depends_on: depends_on.into_iter().map(str::to_string).collect(),
        execution_mode: ExecutionMode::Agent,
        status: WorkItemStatus::Pending,
        worktree_path: None,
        worktree_branch: None,
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
