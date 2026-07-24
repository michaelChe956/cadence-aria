use super::*;

#[test]
fn first_group_code_reviewer_does_not_warn_about_missing_diff_base() {
    let (_tmp, paths, mut attempt) = group_attempt_with_two_work_items(false);
    attempt.stage = CodingExecutionStage::CodeReview;

    let pack = build_evaluation_context_pack(paths, &attempt, EvaluationContextRole::CodeReviewer)
        .expect("context pack");

    assert!(
        !pack
            .context_warnings
            .contains(&"code_review_diff_base_missing".to_string())
    );
}

#[test]
fn group_attempt_uses_current_work_item_as_execution_context() {
    let (_tmp, paths, attempt) = group_attempt_with_two_work_items(false);

    let pack = build_evaluation_context_pack(paths, &attempt, EvaluationContextRole::Coder)
        .expect("context pack");

    assert_eq!(pack.work_item.artifact_id, "work_item_0001");
    assert_eq!(
        pack.group_context.as_ref().expect("group").plan_id,
        "work_item_plan_0001"
    );
    assert_eq!(
        pack.group_context
            .as_ref()
            .expect("group")
            .sibling_work_item_ids,
        vec!["work_item_0001".to_string(), "work_item_0002".to_string()]
    );
}

#[test]
fn group_context_warns_when_current_work_item_is_not_in_plan() {
    let (_tmp, paths, mut attempt) = group_attempt_with_two_work_items(false);
    attempt.current_work_item_id = Some("work_item_outside".to_string());

    let pack = build_evaluation_context_pack(paths, &attempt, EvaluationContextRole::Coder)
        .expect("context pack");

    assert!(
        pack.context_warnings
            .contains(&"group_plan_mapping_mismatch".to_string())
    );
}

#[test]
fn group_context_includes_source_draft_mapping_when_compile_context_exists() {
    let (_tmp, paths, attempt) = group_attempt_with_two_work_items(true);

    let pack = build_evaluation_context_pack(paths, &attempt, EvaluationContextRole::Coder)
        .expect("context pack");
    let group_context = pack.group_context.expect("group context");

    assert_eq!(
        group_context.source_outline_id.as_deref(),
        Some("outline_backend")
    );
    assert_eq!(
        group_context.source_draft_id.as_deref(),
        Some("draft_backend")
    );
    assert!(
        pack.context_warnings
            .contains(&"group_draft_context_loaded".to_string())
    );
}

#[test]
fn group_context_prefers_work_item_source_fields_when_available() {
    let (_tmp, paths, attempt) = group_attempt_with_explicit_work_item_source();

    let pack = build_evaluation_context_pack(paths, &attempt, EvaluationContextRole::Coder)
        .expect("context pack");
    let group_context = pack.group_context.expect("group context");

    assert_eq!(
        group_context.source_outline_id.as_deref(),
        Some("outline_explicit")
    );
    assert_eq!(
        group_context.source_draft_id.as_deref(),
        Some("draft_explicit")
    );
    assert!(
        pack.context_warnings
            .contains(&"group_draft_context_loaded_from_work_item".to_string())
    );
}

#[test]
fn group_context_warns_when_compile_draft_mapping_is_unavailable() {
    let (_tmp, paths, attempt) = group_attempt_with_two_work_items(false);

    let pack = build_evaluation_context_pack(paths, &attempt, EvaluationContextRole::Coder)
        .expect("context pack");
    let group_context = pack.group_context.expect("group context");

    assert_eq!(group_context.source_outline_id, None);
    assert_eq!(group_context.source_draft_id, None);
    assert!(
        pack.context_warnings
            .contains(&"group_draft_context_unavailable".to_string())
    );
}

fn group_attempt_with_two_work_items(
    with_compile_context: bool,
) -> (TempDir, ProductAppPaths, CodingExecutionAttempt) {
    group_attempt_fixture(with_compile_context, false)
}

fn group_attempt_with_explicit_work_item_source()
-> (TempDir, ProductAppPaths, CodingExecutionAttempt) {
    group_attempt_fixture(false, true)
}

fn group_attempt_fixture(
    with_compile_context: bool,
    with_explicit_source: bool,
) -> (TempDir, ProductAppPaths, CodingExecutionAttempt) {
    let tmp = TempDir::new().expect("tmp");
    let paths = ProductAppPaths::new(tmp.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    let plan = create_group_plan_fixture(&lifecycle, with_explicit_source);
    if with_compile_context {
        save_compile_context_fixture(&paths, &plan);
    }

    let attempt = CodingExecutionAttempt {
        id: "coding_attempt_0001".to_string(),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        work_item_id: "work_item_0001".to_string(),
        attempt_no: 1,
        scope: crate::product::coding_models::CodingAttemptScope::WorkItemGroup,
        status: CodingAttemptStatus::Running,
        stage: CodingExecutionStage::Coding,
        base_branch: "main".to_string(),
        branch_name: "aria/issues/issue_0001".to_string(),
        worktree_path: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::Codex,
            reviewer: Some(ProviderName::ClaudeCode),
            review_rounds: 1,
        },
        rework_count: 0,
        max_auto_rework: 2,
        work_item_group_id: Some(plan.id),
        current_work_item_id: Some("work_item_0001".to_string()),
        active_unit_id: Some("coding_execution_unit_0001".to_string()),
        head_commit: None,
        pushed_remote: None,
        review_request_id: None,
        provider_conversations: Vec::new(),
        created_at: "2026-06-10T00:00:00Z".to_string(),
        updated_at: "2026-06-10T00:00:00Z".to_string(),
        completed_at: None,
    };
    (tmp, paths, attempt)
}

fn create_group_plan_fixture(
    lifecycle: &LifecycleStore,
    with_explicit_source: bool,
) -> IssueWorkItemPlan {
    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some("work_item_0001".to_string()),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            story_spec_ids: Vec::new(),
            design_spec_ids: Vec::new(),
            title: "Work Item 1".to_string(),
            work_item_set_id: Some("work_item_plan_0001".to_string()),
            source_work_item_plan_id: with_explicit_source
                .then(|| "work_item_plan_0001".to_string()),
            source_outline_id: with_explicit_source.then(|| "outline_explicit".to_string()),
            source_draft_id: with_explicit_source.then(|| "draft_explicit".to_string()),
            planned_implementation_context: with_explicit_source
                .then(|| "explicit planned context".to_string()),
            planned_handoff_summary: with_explicit_source
                .then(|| "explicit planned handoff".to_string()),
            kind: Default::default(),
            sequence_hint: Some(10),
            depends_on: Vec::new(),
            exclusive_write_scopes: Vec::new(),
            forbidden_write_scopes: Vec::new(),
            context_budget: Default::default(),
            required_handoff_from: Vec::new(),
            verification_plan_ref: None,
            require_execution_plan_confirm: false,
            plan_status: WorkItemPlanStatus::Confirmed,
        })
        .expect("create work item 1");
    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some("work_item_0002".to_string()),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            story_spec_ids: Vec::new(),
            design_spec_ids: Vec::new(),
            title: "Work Item 2".to_string(),
            work_item_set_id: Some("work_item_plan_0001".to_string()),
            source_work_item_plan_id: None,
            source_outline_id: None,
            source_draft_id: None,
            planned_implementation_context: None,
            planned_handoff_summary: None,
            kind: Default::default(),
            sequence_hint: Some(20),
            depends_on: vec!["work_item_0001".to_string()],
            exclusive_write_scopes: Vec::new(),
            forbidden_write_scopes: Vec::new(),
            context_budget: Default::default(),
            required_handoff_from: vec!["work_item_0001".to_string()],
            verification_plan_ref: None,
            require_execution_plan_confirm: false,
            plan_status: WorkItemPlanStatus::Confirmed,
        })
        .expect("create work item 2");
    lifecycle
        .update_work_item_handoff_summary(
            PROJECT_ID,
            ISSUE_ID,
            "work_item_0001",
            Some("handoff/work_item_0001.md".to_string()),
            None,
        )
        .expect("update handoff ref");
    lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some("work_item_plan_0001".to_string()),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            source_story_spec_ids: Vec::new(),
            source_design_spec_ids: Vec::new(),
            options: IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Confirmed,
            work_item_ids: vec!["work_item_0001".to_string(), "work_item_0002".to_string()],
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .expect("create plan")
}

fn save_compile_context_fixture(paths: &ProductAppPaths, plan: &IssueWorkItemPlan) {
    let store = WorkItemPlanStore::new(paths.clone());
    let tx = WorkItemPlanCompileTransaction {
        compile_id: "work_item_plan_compile_0001".to_string(),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        plan_id: plan.id.clone(),
        generation_round_id: "generation_round_0001".to_string(),
        outline_version_ref: "outline_version_0001".to_string(),
        active_draft_ids: vec!["draft_backend".to_string(), "draft_frontend".to_string()],
        status: WorkItemPlanCompileStatus::Committed,
        plan_commit_state: WorkItemPlanCommitState::Committed,
        step_cursor: "committed".to_string(),
        outline_to_work_item_id: std::collections::BTreeMap::from([
            ("outline_backend".to_string(), "work_item_0001".to_string()),
            ("outline_frontend".to_string(), "work_item_0002".to_string()),
        ]),
        outline_to_verification_plan_id: std::collections::BTreeMap::new(),
        created_work_item_ids: vec!["work_item_0001".to_string(), "work_item_0002".to_string()],
        created_verification_plan_ids: Vec::new(),
        child_session_ids: Vec::new(),
        validator_findings: Vec::new(),
        abort_requested_at: None,
        failure_reason: None,
        previous_plan_snapshot: plan.clone(),
        created_at: "2026-06-10T00:00:00Z".to_string(),
        updated_at: "2026-06-10T00:00:00Z".to_string(),
        committed_at: Some("2026-06-10T00:01:00Z".to_string()),
    };
    store.put_compile_transaction(&tx).expect("put compile tx");
    store
        .put_draft_record(&draft_record(
            &plan.id,
            "draft_backend",
            "outline_backend",
            "generation_round_0001",
        ))
        .expect("put backend draft");
    store
        .put_draft_record(&draft_record(
            &plan.id,
            "draft_frontend",
            "outline_frontend",
            "generation_round_0001",
        ))
        .expect("put frontend draft");
}

fn draft_record(
    plan_id: &str,
    draft_id: &str,
    outline_id: &str,
    generation_round_id: &str,
) -> WorkItemDraftRecord {
    WorkItemDraftRecord {
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        plan_id: plan_id.to_string(),
        draft_id: draft_id.to_string(),
        outline_id: outline_id.to_string(),
        generation_round_id: generation_round_id.to_string(),
        batch_id: None,
        attempt_index: 1,
        outline_version_ref: "outline_version_0001".to_string(),
        generation_mode: WorkItemGenerationMode::Serial,
        generation_diagnostics: None,
        candidate: {
            let logical_work_item_id = format!("wi_{outline_id}");
            let mut contract = crate::product::work_item_contract::canonical_contract_fixture(
                &logical_work_item_id,
            );
            contract.identity.title = format!("{outline_id} title");
            contract.identity.kind = "other".to_string();
            contract.goal.summary = format!("{outline_id} goal");
            contract.input_contracts.clear();
            contract.write_policy.exclusive_scopes.clear();
            contract.write_policy.forbidden_scopes.clear();
            WorkItemDraftCandidate {
                outline_id: outline_id.to_string(),
                logical_work_item_id,
                verification_plan: crate::product::models::WorkItemDraftVerificationPlan {
                    checks: contract.verification_checks.clone(),
                },
                canonical_contract_candidate: contract,
            }
        },
        status: WorkItemDraftStatus::Accepted,
        active: true,
        superseded_by_draft_id: None,
        supersede_reason: None,
        copied_from_draft_id: None,
        review_node_id: None,
        review_verdict_ref: None,
        generated_from_node_id: "author_run_0001".to_string(),
        accepted_at: Some("2026-06-10T00:00:00Z".to_string()),
        superseded_at: None,
        created_at: "2026-06-10T00:00:00Z".to_string(),
        updated_at: "2026-06-10T00:00:00Z".to_string(),
    }
}
