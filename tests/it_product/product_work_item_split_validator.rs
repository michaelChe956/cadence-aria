use cadence_aria::product::models::{
    IssueWorkItemDependencyEdge, IssueWorkItemPlan, IssueWorkItemPlanOptions,
    IssueWorkItemPlanStatus, LifecycleWorkItemRecord, RepositoryProfile,
    RepositoryProfileConfidence, VerificationCommand, VerificationCommandSafety,
    VerificationCommandSource, VerificationFallbackPolicy, VerificationPlan, VerificationScope,
    WorkItemContextBudget, WorkItemExecutionPlanStatus, WorkItemKind, WorkItemPlanStatus,
    WorkItemSplitFindingSeverity, WorkItemStatus,
};
use cadence_aria::product::work_item_split_validator::WorkItemSplitValidator;

fn work_item(
    id: &str,
    kind: WorkItemKind,
    depends_on: Vec<&str>,
    scope: Vec<&str>,
) -> LifecycleWorkItemRecord {
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
        kind,
        sequence_hint: None,
        depends_on: depends_on.into_iter().map(str::to_string).collect(),
        exclusive_write_scopes: scope.into_iter().map(str::to_string).collect(),
        forbidden_write_scopes: Vec::new(),
        context_budget: WorkItemContextBudget::default(),
        required_handoff_from: Vec::new(),
        verification_plan_ref: Some(format!("verification_plan_{id}")),
        require_execution_plan_confirm: false,
        execution_plan_status: WorkItemExecutionPlanStatus::NotStarted,
        handoff_summary_ref: None,
        completion_commit: None,
        completion_diff_summary_ref: None,
        created_at: "2026-06-16T00:00:00Z".to_string(),
        updated_at: "2026-06-16T00:00:00Z".to_string(),
    }
}

fn split_plan(ids: Vec<&str>, edges: Vec<(&str, &str)>) -> IssueWorkItemPlan {
    let work_item_ids: Vec<String> = ids.into_iter().map(str::to_string).collect();
    let verification_plan_ids: Vec<String> = work_item_ids
        .iter()
        .map(|id| format!("verification_plan_{id}"))
        .collect();
    IssueWorkItemPlan {
        id: "work_item_set_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        source_story_spec_ids: vec!["story_spec_0001".to_string()],
        source_design_spec_ids: vec!["design_spec_0001".to_string()],
        options: IssueWorkItemPlanOptions {
            include_integration_tests: false,
            include_e2e_tests: false,
            force_frontend_backend_split: false,
            require_execution_plan_confirm: false,
        },
        status: IssueWorkItemPlanStatus::Draft,
        work_item_ids,
        repository_profile_ref: Some("repository_profile_0001".to_string()),
        verification_plan_ids,
        dependency_graph: edges
            .into_iter()
            .map(|(from, to)| IssueWorkItemDependencyEdge {
                from_work_item_id: from.to_string(),
                to_work_item_id: to.to_string(),
            })
            .collect(),
        created_from_provider_run: None,
        validator_findings: Vec::new(),
        review_summary: None,
        created_at: "2026-06-16T00:00:00Z".to_string(),
        updated_at: "2026-06-16T00:00:00Z".to_string(),
    }
}

fn repository_profile() -> RepositoryProfile {
    RepositoryProfile {
        id: "repository_profile_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        repository_id: "repo_0001".to_string(),
        provider_run_ref: Some("provider_run_0001".to_string()),
        languages: vec!["rust".to_string(), "typescript".to_string()],
        frameworks: vec!["axum".to_string(), "react".to_string()],
        package_managers: vec!["cargo".to_string(), "pnpm".to_string()],
        test_frameworks: vec!["cargo-test".to_string(), "vitest".to_string()],
        build_systems: vec!["cargo".to_string(), "vite".to_string()],
        verification_capabilities: vec!["unit".to_string(), "integration".to_string()],
        detected_layers: vec!["backend".to_string(), "frontend".to_string()],
        split_recommendation: "frontend_backend".to_string(),
        confidence: RepositoryProfileConfidence::High,
        uncertainties: Vec::new(),
        created_at: "2026-06-16T00:00:00Z".to_string(),
        updated_at: "2026-06-16T00:00:00Z".to_string(),
    }
}

fn verification_plan_for(item_id: &str) -> VerificationPlan {
    VerificationPlan {
        id: format!("verification_plan_{item_id}"),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        work_item_id: item_id.to_string(),
        repository_profile_ref: Some("repository_profile_0001".to_string()),
        provider_run_ref: Some("provider_run_0001".to_string()),
        scope: VerificationScope::Unit,
        commands: vec![VerificationCommand {
            id: "verify_unit".to_string(),
            label: "provider unit verification".to_string(),
            command: "custom-verify unit".to_string(),
            cwd: ".".to_string(),
            purpose: "unit".to_string(),
            required: true,
            timeout_seconds: 120,
            source: VerificationCommandSource::Provider,
            safety: VerificationCommandSafety::Approved,
        }],
        manual_checks: Vec::new(),
        required_gates: vec!["verify_unit".to_string()],
        risk_notes: Vec::new(),
        confidence: RepositoryProfileConfidence::High,
        fallback_policy: VerificationFallbackPolicy::ManualGate,
        created_at: "2026-06-16T00:00:00Z".to_string(),
        updated_at: "2026-06-16T00:00:00Z".to_string(),
    }
}

fn validate_for_test(
    plan: &IssueWorkItemPlan,
    items: &[LifecycleWorkItemRecord],
) -> cadence_aria::product::work_item_split_validator::WorkItemSplitValidationReport {
    let profile = repository_profile();
    let verification_plans: Vec<VerificationPlan> = items
        .iter()
        .map(|item| verification_plan_for(&item.id))
        .collect();
    WorkItemSplitValidator::validate(plan, items, Some(&profile), &verification_plans)
}

#[test]
fn issue_work_item_plan_serializes_options_and_dependency_graph_as_snake_case() {
    let plan = IssueWorkItemPlan {
        id: "work_item_set_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        source_story_spec_ids: vec!["story_spec_0001".to_string()],
        source_design_spec_ids: vec!["design_spec_0001".to_string()],
        options: IssueWorkItemPlanOptions {
            include_integration_tests: true,
            include_e2e_tests: false,
            force_frontend_backend_split: true,
            require_execution_plan_confirm: false,
        },
        status: IssueWorkItemPlanStatus::Draft,
        work_item_ids: vec!["work_item_0001".to_string(), "work_item_0002".to_string()],
        repository_profile_ref: Some("repository_profile_0001".to_string()),
        verification_plan_ids: vec![
            "verification_plan_work_item_0001".to_string(),
            "verification_plan_work_item_0002".to_string(),
        ],
        dependency_graph: vec![IssueWorkItemDependencyEdge {
            from_work_item_id: "work_item_0001".to_string(),
            to_work_item_id: "work_item_0002".to_string(),
        }],
        created_from_provider_run: Some("provider_run_0001".to_string()),
        validator_findings: Vec::new(),
        review_summary: Some("backend first, frontend second".to_string()),
        created_at: "2026-06-16T00:00:00Z".to_string(),
        updated_at: "2026-06-16T00:00:00Z".to_string(),
    };

    let value = serde_json::to_value(plan).expect("serialize plan");

    assert_eq!(value["status"], "draft");
    assert_eq!(value["options"]["include_integration_tests"], true);
    assert_eq!(value["options"]["include_e2e_tests"], false);
    assert_eq!(
        value["dependency_graph"][0]["from_work_item_id"],
        "work_item_0001"
    );
    assert_eq!(
        value["dependency_graph"][0]["to_work_item_id"],
        "work_item_0002"
    );
}

#[test]
fn split_finding_severity_serializes_as_snake_case() {
    assert_eq!(
        serde_json::to_value(WorkItemSplitFindingSeverity::Error).unwrap(),
        serde_json::json!("error")
    );
    assert_eq!(
        serde_json::to_value(WorkItemSplitFindingSeverity::Warning).unwrap(),
        serde_json::json!("warning")
    );
}

#[test]
fn repository_profile_and_verification_plan_serialize_as_provider_output() {
    let profile = RepositoryProfile {
        id: "repository_profile_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        repository_id: "repo_0001".to_string(),
        provider_run_ref: Some("provider_run_0001".to_string()),
        languages: vec!["rust".to_string(), "typescript".to_string()],
        frameworks: vec!["axum".to_string(), "react".to_string()],
        package_managers: vec!["cargo".to_string(), "pnpm".to_string()],
        test_frameworks: vec!["cargo-test".to_string(), "vitest".to_string()],
        build_systems: vec!["cargo".to_string(), "vite".to_string()],
        verification_capabilities: vec!["unit".to_string(), "integration".to_string()],
        detected_layers: vec!["backend".to_string(), "frontend".to_string()],
        split_recommendation: "frontend_backend".to_string(),
        confidence: RepositoryProfileConfidence::High,
        uncertainties: Vec::new(),
        created_at: "2026-06-16T00:00:00Z".to_string(),
        updated_at: "2026-06-16T00:00:00Z".to_string(),
    };
    let plan = VerificationPlan {
        id: "verification_plan_work_item_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        work_item_id: "work_item_0001".to_string(),
        repository_profile_ref: Some("repository_profile_0001".to_string()),
        provider_run_ref: Some("provider_run_0001".to_string()),
        scope: VerificationScope::Unit,
        commands: vec![VerificationCommand {
            id: "verify_backend_unit".to_string(),
            label: "backend unit tests".to_string(),
            command: "custom-verify backend-api".to_string(),
            cwd: ".".to_string(),
            purpose: "unit".to_string(),
            required: true,
            timeout_seconds: 120,
            source: VerificationCommandSource::Provider,
            safety: VerificationCommandSafety::Approved,
        }],
        manual_checks: Vec::new(),
        required_gates: vec!["verify_backend_unit".to_string()],
        risk_notes: Vec::new(),
        confidence: RepositoryProfileConfidence::High,
        fallback_policy: VerificationFallbackPolicy::ManualGate,
        created_at: "2026-06-16T00:00:00Z".to_string(),
        updated_at: "2026-06-16T00:00:00Z".to_string(),
    };

    let value = serde_json::json!({
        "repository_profile": profile,
        "verification_plan": plan,
    });

    assert_eq!(value["repository_profile"]["confidence"], "high");
    assert_eq!(
        value["verification_plan"]["commands"][0]["source"],
        "provider"
    );
    assert_eq!(value["verification_plan"]["fallback_policy"], "manual_gate");
}

#[test]
fn validator_rejects_dependency_cycles() {
    let plan = split_plan(
        vec!["work_item_0001", "work_item_0002"],
        vec![
            ("work_item_0001", "work_item_0002"),
            ("work_item_0002", "work_item_0001"),
        ],
    );
    let items = vec![
        work_item(
            "work_item_0001",
            WorkItemKind::Backend,
            vec!["work_item_0002"],
            vec!["src/**"],
        ),
        work_item(
            "work_item_0002",
            WorkItemKind::Frontend,
            vec!["work_item_0001"],
            vec!["web/src/**"],
        ),
    ];

    let report = validate_for_test(&plan, &items);

    assert!(report.has_errors());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "dependency_cycle")
    );
}

#[test]
fn validator_rejects_dependency_outside_same_issue() {
    let plan = split_plan(
        vec!["work_item_0001"],
        vec![("work_item_0001", "work_item_9999")],
    );
    let items = vec![work_item(
        "work_item_0001",
        WorkItemKind::Backend,
        vec!["work_item_9999"],
        vec!["src/**"],
    )];

    let report = validate_for_test(&plan, &items);

    assert!(report.has_errors());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "dependency_not_in_plan")
    );
}

#[test]
fn validator_rejects_parallel_overlapping_write_scopes() {
    let plan = split_plan(vec!["work_item_0001", "work_item_0002"], vec![]);
    let items = vec![
        work_item(
            "work_item_0001",
            WorkItemKind::Backend,
            vec![],
            vec!["src/product/**"],
        ),
        work_item(
            "work_item_0002",
            WorkItemKind::Backend,
            vec![],
            vec!["src/**"],
        ),
    ];

    let report = validate_for_test(&plan, &items);

    assert!(report.has_errors());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "parallel_scope_overlap")
    );
}

#[test]
fn validator_allows_overlapping_write_scopes_when_dependency_orders_items() {
    let plan = split_plan(
        vec!["work_item_0001", "work_item_0002"],
        vec![("work_item_0001", "work_item_0002")],
    );
    let items = vec![
        work_item(
            "work_item_0001",
            WorkItemKind::Backend,
            vec![],
            vec!["src/product/**"],
        ),
        work_item(
            "work_item_0002",
            WorkItemKind::Backend,
            vec!["work_item_0001"],
            vec!["src/**"],
        ),
    ];

    let report = validate_for_test(&plan, &items);

    assert!(!report.has_errors());
}

#[test]
fn validator_rejects_context_budget_over_proxy_limits() {
    let plan = split_plan(vec!["work_item_0001"], vec![]);
    let mut item = work_item(
        "work_item_0001",
        WorkItemKind::Backend,
        vec![],
        vec!["src/**"],
    );
    item.context_budget.max_summary_chars = 100_001;
    item.context_budget.max_context_file_refs = 500;

    let report = validate_for_test(&plan, &[item]);

    assert!(report.has_errors());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "context_budget_over_limit")
    );
}

#[test]
fn validator_requires_backend_and_frontend_when_force_split_is_enabled() {
    let mut plan = split_plan(vec!["work_item_0001"], vec![]);
    plan.options.force_frontend_backend_split = true;
    let items = vec![work_item(
        "work_item_0001",
        WorkItemKind::Backend,
        vec![],
        vec!["src/**"],
    )];

    let report = validate_for_test(&plan, &items);

    assert!(report.has_errors());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "frontend_backend_split_required")
    );
}

#[test]
fn validator_requires_integration_item_when_option_enabled() {
    let mut plan = split_plan(vec!["work_item_0001", "work_item_0002"], vec![]);
    plan.options.include_integration_tests = true;
    let items = vec![
        work_item(
            "work_item_0001",
            WorkItemKind::Backend,
            vec![],
            vec!["src/**"],
        ),
        work_item(
            "work_item_0002",
            WorkItemKind::Frontend,
            vec!["work_item_0001"],
            vec!["web/src/**"],
        ),
    ];

    let report = validate_for_test(&plan, &items);

    assert!(report.has_errors());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "integration_work_item_required")
    );
}

#[test]
fn validator_requires_traceability_refs_on_every_work_item() {
    let plan = split_plan(vec!["work_item_0001"], vec![]);
    let mut item = work_item(
        "work_item_0001",
        WorkItemKind::Backend,
        vec![],
        vec!["src/**"],
    );
    item.story_spec_ids.clear();
    item.design_spec_ids.clear();

    let report = validate_for_test(&plan, &[item]);

    assert!(report.has_errors());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "traceability_refs_required")
    );
}

#[test]
fn validator_requires_non_empty_exclusive_write_scopes() {
    let plan = split_plan(vec!["work_item_0001"], vec![]);
    let item = work_item("work_item_0001", WorkItemKind::Backend, vec![], vec![]);

    let report = validate_for_test(&plan, &[item]);

    assert!(report.has_errors());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "write_scope_required")
    );
}

#[test]
fn validator_records_risk_when_integration_or_e2e_skipped() {
    let mut plan = split_plan(
        vec!["work_item_0001", "work_item_0002"],
        vec![("work_item_0001", "work_item_0002")],
    );
    plan.options.include_integration_tests = false;
    plan.options.include_e2e_tests = false;
    let items = vec![
        work_item(
            "work_item_0001",
            WorkItemKind::Backend,
            vec![],
            vec!["src/**"],
        ),
        work_item(
            "work_item_0002",
            WorkItemKind::Frontend,
            vec!["work_item_0001"],
            vec!["web/src/**"],
        ),
    ];

    let report = validate_for_test(&plan, &items);

    assert!(!report.has_errors());
    assert!(report.findings.iter().any(|finding| {
        finding.code == "integration_or_e2e_skipped_risk"
            && finding.severity == WorkItemSplitFindingSeverity::Warning
    }));
}

#[test]
fn validator_rejects_missing_verification_plan_for_work_item() {
    let plan = split_plan(vec!["work_item_0001"], vec![]);
    let items = vec![work_item(
        "work_item_0001",
        WorkItemKind::Backend,
        vec![],
        vec!["src/**"],
    )];
    let profile = repository_profile();

    let report = WorkItemSplitValidator::validate(&plan, &items, Some(&profile), &[]);

    assert!(report.has_errors());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "verification_plan_missing")
    );
}

#[test]
fn validator_rejects_verification_command_cwd_outside_repository() {
    let plan = split_plan(vec!["work_item_0001"], vec![]);
    let items = vec![work_item(
        "work_item_0001",
        WorkItemKind::Backend,
        vec![],
        vec!["src/**"],
    )];
    let profile = repository_profile();
    let mut verification = verification_plan_for("work_item_0001");
    verification.commands[0].cwd = "../outside".to_string();

    let report = WorkItemSplitValidator::validate(&plan, &items, Some(&profile), &[verification]);

    assert!(report.has_errors());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "verification_command_cwd_outside_repository")
    );
}

#[test]
fn validator_rejects_unsafe_verification_command() {
    let plan = split_plan(vec!["work_item_0001"], vec![]);
    let items = vec![work_item(
        "work_item_0001",
        WorkItemKind::Backend,
        vec![],
        vec!["src/**"],
    )];
    let profile = repository_profile();
    let mut verification = verification_plan_for("work_item_0001");
    verification.commands[0].command = "git reset --hard && git clean -fdx".to_string();

    let report = WorkItemSplitValidator::validate(&plan, &items, Some(&profile), &[verification]);

    assert!(report.has_errors());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "verification_command_unsafe")
    );
}

#[test]
fn validator_rejects_dependency_graph_missing_declared_edge() {
    let plan = split_plan(vec!["work_item_0001", "work_item_0002"], vec![]);
    let items = vec![
        work_item(
            "work_item_0001",
            WorkItemKind::Backend,
            vec![],
            vec!["src/**"],
        ),
        work_item(
            "work_item_0002",
            WorkItemKind::Frontend,
            vec!["work_item_0001"],
            vec!["web/src/**"],
        ),
    ];

    let report = validate_for_test(&plan, &items);

    assert!(report.has_errors());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "dependency_graph_mismatch")
    );
}

#[test]
fn validator_rejects_dependency_graph_undeclared_edge() {
    let plan = split_plan(
        vec!["work_item_0001", "work_item_0002"],
        vec![("work_item_0001", "work_item_0002")],
    );
    let items = vec![
        work_item(
            "work_item_0001",
            WorkItemKind::Backend,
            vec![],
            vec!["src/**"],
        ),
        work_item(
            "work_item_0002",
            WorkItemKind::Frontend,
            vec![],
            vec!["web/src/**"],
        ),
    ];

    let report = validate_for_test(&plan, &items);

    assert!(report.has_errors());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "dependency_graph_mismatch")
    );
}

#[test]
fn validator_rejects_extra_verification_plan_not_referenced() {
    let plan = split_plan(vec!["work_item_0001"], vec![]);
    let items = vec![work_item(
        "work_item_0001",
        WorkItemKind::Backend,
        vec![],
        vec!["src/**"],
    )];
    let profile = repository_profile();
    let verification_plans = vec![
        verification_plan_for("work_item_0001"),
        verification_plan_for("work_item_0002"),
    ];

    let report =
        WorkItemSplitValidator::validate(&plan, &items, Some(&profile), &verification_plans);

    assert!(report.has_errors());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "verification_plan_mismatch")
    );
}

#[test]
fn validator_reports_only_missing_for_unreferenced_verification_plan() {
    let mut plan = split_plan(vec!["work_item_0001"], vec![]);
    plan.verification_plan_ids.clear();
    let mut item = work_item(
        "work_item_0001",
        WorkItemKind::Backend,
        vec![],
        vec!["src/**"],
    );
    item.verification_plan_ref = Some("verification_plan_nonexistent".to_string());
    let profile = repository_profile();

    let report = WorkItemSplitValidator::validate(&plan, &[item], Some(&profile), &[]);

    assert!(report.has_errors());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "verification_plan_missing")
    );
    assert!(
        report
            .findings
            .iter()
            .all(|finding| finding.code != "verification_plan_mismatch")
    );
}
