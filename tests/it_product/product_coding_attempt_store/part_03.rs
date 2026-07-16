#[test]
fn role_run_retry_diagnostic_summary_preserves_refs_when_inline_detail_is_long() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");
    let run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0005".to_string()),
        )
        .expect("role run");
    let long_detail = format!("{}DETAIL_SHOULD_BE_TRUNCATED", "x".repeat(10_000));
    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "title": "Long diagnostic detail",
                "status": "blocked",
                "detail": long_detail
            }),
        )
        .expect("event");
    store
        .update_role_run_refs(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &run.id,
            vec!["provider-raw/testing/long_detail_0001.txt".to_string()],
            vec!["artifacts/role-run-events/coding_role_run_0001/0001_detail.txt".to_string()],
        )
        .expect("refs");
    store
        .update_role_run_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &run.id,
            CodingRoleRunStatus::Blocked,
            Some("long_detail_blocked".to_string()),
        )
        .expect("blocked");

    let summary = store
        .role_run_retry_diagnostic_summary("project_0001", "issue_0001", &attempt.id, &run.id)
        .expect("summary")
        .expect("summary text");

    assert!(summary.contains("Long diagnostic detail"));
    assert!(summary.contains("reason_code: long_detail_blocked"));
    assert!(summary.contains("provider-raw/testing/long_detail_0001.txt"));
    assert!(summary.contains("artifacts/role-run-events/coding_role_run_0001/0001_detail.txt"));
    assert!(!summary.contains("DETAIL_SHOULD_BE_TRUNCATED"));
    assert!(
        summary.len() <= 8_000,
        "retry diagnostic summary must stay prompt-safe"
    );
}

#[test]
fn saves_and_loads_work_item_execution_plan() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let plan = WorkItemExecutionPlan {
        id: "work_item_execution_plan_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        work_item_id: "work_item_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        status: WorkItemExecutionPlanStatus::Draft,
        goal: "实现后端 API".to_string(),
        allowed_write_scopes: vec!["src/product/**".to_string()],
        forbidden_write_scopes: vec!["web/**".to_string()],
        dependency_handoffs: Vec::new(),
        story_refs: vec!["story_spec_0001".to_string()],
        design_refs: vec!["design_spec_0001".to_string()],
        openspec_refs: vec!["REQ-001".to_string()],
        superpowers_contract: "use superpowers:test-driven-development".to_string(),
        tdd_contract: "先写失败测试，再写实现".to_string(),
        verification_plan_ref: Some("verification_plan_work_item_0001".to_string()),
        verification_summary: Some(
            "provider supplied required gate verify_backend_unit".to_string(),
        ),
        risk_notes: Vec::new(),
        created_at: "2026-06-16T00:00:00Z".to_string(),
        updated_at: "2026-06-16T00:00:00Z".to_string(),
    };

    store
        .save_work_item_execution_plan(&plan)
        .expect("save execution plan");

    let loaded = store
        .get_work_item_execution_plan("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("load execution plan")
        .expect("plan exists");
    assert_eq!(loaded.goal, "实现后端 API");
    assert_eq!(loaded.status, WorkItemExecutionPlanStatus::Draft);
}

#[test]
fn saves_and_loads_work_item_handoff() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let handoff = WorkItemHandoff {
        id: "work_item_handoff_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        work_item_id: "work_item_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        provider_run_ref: Some("provider-raw/handoff/work_item_0001.txt".to_string()),
        summary: "后端 API 已完成，前端可调用 /api/session".to_string(),
        files_changed: vec!["src/web/handlers.rs".to_string()],
        commit_sha: Some("abc123".to_string()),
        diff_summary: "新增 session API".to_string(),
        tests_run: vec!["provider gate verify_session_api passed".to_string()],
        test_result_summary: "全部通过".to_string(),
        review_summary: Some("无阻塞问题".to_string()),
        api_or_contract_changes: vec!["GET /api/session".to_string()],
        open_risks: Vec::new(),
        next_work_item_notes: vec!["前端处理 401".to_string()],
        created_at: "2026-06-16T00:00:00Z".to_string(),
    };

    store
        .save_work_item_handoff(&handoff)
        .expect("save handoff");

    let loaded = store
        .get_work_item_handoff("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("load handoff")
        .expect("handoff exists");
    assert_eq!(loaded.summary, handoff.summary);
}

#[test]
fn saves_and_loads_group_unit_work_item_handoff() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("create group attempt");
    store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: attempt.id.clone(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            order_index: 0,
            status: CodingExecutionUnitStatus::Running,
        })
        .expect("create unit");
    let handoff = WorkItemHandoff {
        id: "work_item_handoff_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        work_item_id: "work_item_0001".to_string(),
        attempt_id: attempt.id.clone(),
        provider_run_ref: None,
        summary: "group unit handoff".to_string(),
        files_changed: vec!["src/lib.rs".to_string()],
        commit_sha: Some("abc123".to_string()),
        diff_summary: "group unit diff".to_string(),
        tests_run: vec!["cargo test".to_string()],
        test_result_summary: "passed".to_string(),
        review_summary: Some("ok".to_string()),
        api_or_contract_changes: Vec::new(),
        open_risks: Vec::new(),
        next_work_item_notes: vec!["next".to_string()],
        created_at: "2026-06-27T00:00:00Z".to_string(),
    };

    store
        .save_coding_unit_handoff("project_0001", "issue_0001", &attempt.id, "coding_unit_0001", &handoff)
        .expect("save unit handoff");

    let loaded = store
        .get_coding_unit_handoff("project_0001", "issue_0001", &attempt.id, "coding_unit_0001")
        .expect("load unit handoff")
        .expect("unit handoff exists");
    assert_eq!(loaded.summary, handoff.summary);
    assert!(store
        .get_work_item_handoff("project_0001", "issue_0001", &attempt.id)
        .expect("attempt handoff")
        .is_none());
}

#[test]
fn scoped_writes_target_only_exact_legacy_attempt_identity() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let mut target = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create target attempt");
    target.id = "coding_attempt_0001".to_string();
    store
        .save_coding_attempt(&target)
        .expect("save legacy target attempt");
    let mut other = target.clone();
    other.issue_id = "issue_0002".to_string();
    store
        .save_coding_attempt(&other)
        .expect("save duplicate legacy attempt");

    store
        .create_stage_gate(
            &target,
            CodingExecutionStage::Coding,
            CodingProviderRole::Coder,
            "2026-07-16T00:00:05Z".to_string(),
            CodingRoleProviderConfigSnapshot::from(ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
            }),
        )
        .expect("target stage gate");
    store
        .create_blocked_gate(
            &target,
            CreateBlockedGateInput {
                attempt_id: target.id.clone(),
                stage: CodingExecutionStage::Coding,
                node_id: None,
                role: Some(CodingProviderRole::Coder),
                title: "target blocked gate".to_string(),
                description: "target only".to_string(),
                reason_code: Some("target_only".to_string()),
                evidence_refs: Vec::new(),
                raw_provider_output_ref: None,
                available_actions: Vec::new(),
            },
        )
        .expect("target blocked gate");
    store
        .create_choice_gate(
            &target,
            CreateChoiceGateInput {
                attempt_id: target.id.clone(),
                choice_id: "choice_0001".to_string(),
                stage: CodingExecutionStage::Coding,
                node_id: None,
                role: CodingProviderRole::Coder,
                provider: ProviderName::Fake,
                source: "target".to_string(),
                prompt: "target choice".to_string(),
                options: vec![CodingChoiceOption {
                    id: "yes".to_string(),
                    label: "Yes".to_string(),
                    description: None,
                }],
                allow_multiple: false,
                allow_free_text: false,
            },
        )
        .expect("target choice gate");
    store
        .create_quality_bypass_audit(
            &target,
            CreateQualityBypassAuditInput {
                attempt_id: target.id.clone(),
                gate_id: "coding_stage_gate_0001".to_string(),
                stage: CodingExecutionStage::Coding,
                reason_code: Some("accepted_risk".to_string()),
                skipped_required_steps: Vec::new(),
                operator_context: "target audit".to_string(),
            },
        )
        .expect("target quality audit");

    let raw_ref = store
        .save_provider_raw_output(
            &target,
            CodingExecutionStage::Coding,
            "coder_output",
            "target raw output",
        )
        .expect("target raw output");
    store
        .save_test_plan(
            &target,
            &TestPlan {
                id: "test_plan_0001".to_string(),
                attempt_id: target.id.clone(),
                role_run_id: None,
                run_no: None,
                summary: "target plan".to_string(),
                context_warnings: Vec::new(),
                assumptions: Vec::new(),
                steps: Vec::new(),
                created_at: "2026-07-16T00:00:00Z".to_string(),
                raw_provider_output_ref: Some(raw_ref.clone()),
            },
        )
        .expect("target test plan");
    store
        .save_testing_report(&target, &sample_testing_report(&target.id))
        .expect("target testing report");
    store
        .save_code_review_report(&target, &sample_code_review_report(&target.id))
        .expect("target code review");
    let review_request = sample_review_request(&target.id);
    store
        .save_review_request(&target, &review_request)
        .expect("target review request");
    store
        .save_internal_pr_review(
            &target,
            &sample_internal_review(&target.id, &review_request.id),
        )
        .expect("target internal review");
    store
        .replace_attempt_provider_conversations(
            &target,
            vec![ProviderConversationRef {
                role: ProviderConversationRole::Coder,
                provider: ProviderName::Fake,
                provider_session_id: "target-session".to_string(),
                updated_at: "2026-07-16T00:00:00Z".to_string(),
                last_node_id: Some("coding_node_0001".to_string()),
            }],
        )
        .expect("target conversation");

    assert_eq!(
        store
            .read_attempt_artifact_text(&target, &raw_ref)
            .expect("target raw output text"),
        "target raw output"
    );
    assert_eq!(
        store
            .list_open_stage_gates("project_0001", "issue_0001", &target.id)
            .expect("target stage gates")
            .len(),
        1
    );
    assert_eq!(
        store
            .list_open_blocked_gates("project_0001", "issue_0001", &target.id)
            .expect("target blocked gates")
            .len(),
        1
    );
    assert_eq!(
        store
            .list_open_choice_gates("project_0001", "issue_0001", &target.id)
            .expect("target choice gates")
            .len(),
        1
    );
    assert_eq!(
        store
            .list_quality_bypass_audits("project_0001", "issue_0001", &target.id)
            .expect("target audits")
            .len(),
        1
    );
    assert_eq!(
        store
            .list_test_plans("project_0001", "issue_0001", &target.id)
            .expect("target test plans")
            .len(),
        1
    );
    assert_eq!(
        store
            .list_testing_reports("project_0001", "issue_0001", &target.id)
            .expect("target testing reports")
            .len(),
        1
    );
    assert_eq!(
        store
            .list_code_review_reports("project_0001", "issue_0001", &target.id)
            .expect("target code reviews")
            .len(),
        1
    );
    assert_eq!(
        store
            .list_review_requests("project_0001", "issue_0001", &target.id)
            .expect("target review requests")
            .len(),
        1
    );
    assert_eq!(
        store
            .list_internal_pr_reviews("project_0001", "issue_0001", &target.id)
            .expect("target internal reviews")
            .len(),
        1
    );
    assert!(
        store
            .list_open_stage_gates("project_0001", "issue_0002", &target.id)
            .expect("other stage gates")
            .is_empty()
    );
    assert!(
        store
            .list_open_blocked_gates("project_0001", "issue_0002", &target.id)
            .expect("other blocked gates")
            .is_empty()
    );
    assert!(
        store
            .list_open_choice_gates("project_0001", "issue_0002", &target.id)
            .expect("other choice gates")
            .is_empty()
    );
    assert!(
        store
            .list_testing_reports("project_0001", "issue_0002", &target.id)
            .expect("other testing reports")
            .is_empty()
    );
    assert!(
        store
            .list_code_review_reports("project_0001", "issue_0002", &target.id)
            .expect("other code reviews")
            .is_empty()
    );
    assert!(
        store
            .get_attempt("project_0001", "issue_0002", &target.id)
            .expect("other attempt")
            .provider_conversations
            .is_empty()
    );
}
