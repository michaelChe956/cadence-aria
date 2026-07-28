fn create_required_verification_plan(
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
        .expect("create verification plan");
}

fn save_minimal_unit_handoff(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    unit_id: &str,
    work_item_id: &str,
) {
    store
        .save_coding_unit_handoff(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            unit_id,
            &WorkItemHandoff {
                id: format!("work_item_handoff_{unit_id}"),
                project_id: attempt.project_id.clone(),
                issue_id: attempt.issue_id.clone(),
                work_item_id: work_item_id.to_string(),
                attempt_id: attempt.id.clone(),
                provider_run_ref: None,
                summary: format!("handoff summary for {work_item_id}"),
                files_changed: Vec::new(),
                commit_sha: Some(format!("{work_item_id}-sha")),
                diff_summary: String::new(),
                tests_run: vec!["cargo test --locked --lib unit".to_string()],
                test_result_summary: "passed".to_string(),
                review_summary: None,
                api_or_contract_changes: Vec::new(),
                open_risks: Vec::new(),
                next_work_item_notes: Vec::new(),
                created_at: "2026-06-27T00:00:00Z".to_string(),
            },
        )
        .expect("save unit handoff");
}
