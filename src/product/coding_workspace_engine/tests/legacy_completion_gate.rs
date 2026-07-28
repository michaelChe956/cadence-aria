use super::*;
use crate::product::lifecycle_store::CreateVerificationPlanInput;
use crate::product::models::{
    RepositoryProfileConfidence, VerificationCommand, VerificationCommandSafety,
    VerificationCommandSource, VerificationFallbackPolicy, VerificationScope,
};

fn create_required_legacy_verification_plan(
    lifecycle: &LifecycleStore,
    work_item_id: &str,
    plan_id: &str,
) {
    lifecycle
        .create_verification_plan(CreateVerificationPlanInput {
            id: Some(plan_id.to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: work_item_id.to_string(),
            repository_profile_ref: None,
            provider_run_ref: None,
            scope: VerificationScope::Unit,
            commands: vec![VerificationCommand {
                id: "unit_tests".to_string(),
                label: "Unit tests".to_string(),
                command: "cargo test --locked --lib unit".to_string(),
                cwd: ".".to_string(),
                purpose: "unit tests".to_string(),
                required: true,
                timeout_seconds: 120,
                source: VerificationCommandSource::Provider,
                safety: VerificationCommandSafety::Approved,
            }],
            manual_checks: Vec::new(),
            required_gates: vec!["unit_tests".to_string()],
            risk_notes: Vec::new(),
            confidence: RepositoryProfileConfidence::High,
            fallback_policy: VerificationFallbackPolicy::ManualGate,
        })
        .expect("create required verification plan");
}

#[tokio::test]
async fn legacy_group_completion_gates_ignore_non_passed_testing_reports() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("group attempt");
    let lifecycle = LifecycleStore::new(store.paths());

    for (index, (work_item_id, plan_id)) in [
        ("work_item_0001", "verification_plan_0001"),
        ("work_item_0002", "verification_plan_0002"),
    ]
    .into_iter()
    .enumerate()
    {
        create_required_legacy_verification_plan(&lifecycle, work_item_id, plan_id);
        lifecycle
            .create_work_item(CreateWorkItemInput {
                id: Some(work_item_id.to_string()),
                project_id: attempt.project_id.clone(),
                issue_id: attempt.issue_id.clone(),
                repository_id: "repository_0001".to_string(),
                title: work_item_id.to_string(),
                verification_plan_ref: Some(plan_id.to_string()),
                ..Default::default()
            })
            .expect("legacy work item");
        let unit = store
            .create_coding_unit(CreateCodingExecutionUnitInput {
                attempt_id: attempt.id.clone(),
                project_id: attempt.project_id.clone(),
                issue_id: attempt.issue_id.clone(),
                plan_id: "work_item_plan_0001".to_string(),
                logical_work_item_id: work_item_id.to_string(),
                work_item_revision_id: format!("work_item_revision_{:04}", index + 1),
                dependency_logical_work_item_ids: Vec::new(),
                order_index: index as u32,
                status: CodingExecutionUnitStatus::Completed,
            })
            .expect("completed coding unit");
        store
            .save_coding_unit_handoff(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &unit.id,
                &WorkItemHandoff {
                    id: format!("work_item_handoff_{:04}", index + 1),
                    project_id: attempt.project_id.clone(),
                    issue_id: attempt.issue_id.clone(),
                    work_item_id: work_item_id.to_string(),
                    attempt_id: attempt.id.clone(),
                    provider_run_ref: None,
                    summary: format!("handoff for {work_item_id}"),
                    files_changed: Vec::new(),
                    commit_sha: Some("deadbeef".to_string()),
                    diff_summary: String::new(),
                    tests_run: Vec::new(),
                    test_result_summary: String::new(),
                    review_summary: None,
                    api_or_contract_changes: Vec::new(),
                    open_risks: Vec::new(),
                    next_work_item_notes: Vec::new(),
                    created_at: "2026-07-27T00:00:00Z".to_string(),
                },
            )
            .expect("legacy handoff");
        store
            .update_coding_unit_completion_commit(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &unit.id,
                Some("deadbeef".to_string()),
            )
            .expect("completion commit");
    }

    let attempt = store
        .update_attempt_head_commit(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            Some("deadbeef".to_string()),
        )
        .expect("attempt head commit");
    assert!(
        WorkItemRevisionStore::new(store.paths())
            .get_plan_lineage(
                &attempt.project_id,
                &attempt.issue_id,
                "work_item_plan_0001",
            )
            .is_err()
    );

    let mut blocked_report = blocked_report_with(Vec::new(), Vec::new());
    blocked_report.id = "testing_report_blocked".to_string();
    blocked_report.attempt_id = attempt.id.clone();
    blocked_report.plan_id = Some("verification_plan_0001".to_string());
    store
        .save_testing_report(&attempt, &blocked_report)
        .expect("blocked testing report");

    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    engine
        .run_group_completion_gates(&attempt)
        .await
        .expect("legacy completion gate must ignore testing report status");

    assert_eq!(
        store
            .list_testing_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("testing reports"),
        vec![blocked_report]
    );
}
