#[tokio::test]
async fn complete_work_item_plan_author_errors_trigger_auto_revision_then_human_confirm() {
    use crate::product::lifecycle_store::{
        CreateDesignSpecInput, CreateIssueWorkItemPlanInput, CreateStorySpecInput,
    };
    use crate::product::models::{
        IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, LifecycleWorkItemRecord,
        RepositoryProfile, RepositoryProfileConfidence, VerificationCommand,
        VerificationCommandSafety, VerificationCommandSource, VerificationFallbackPolicy,
        VerificationManualCheck, VerificationPlan, VerificationScope, WorkItemContextBudget,
        WorkItemExecutionPlanStatus, WorkItemKind, WorkItemPlanStatus,
        WorkItemSplitFindingSeverity, WorkItemStatus,
    };
    use crate::product::work_item_split_engine::WorkItemSplitProviderOutput;

    let (_tmp, checkpoint_store) = setup();
    let app_root = tempfile::tempdir().expect("app root");
    let lifecycle_store = LifecycleStore::new(ProductAppPaths::new(app_root.path().join(".aria")));
    let project_id = "project_0001";
    let issue_id = "issue_0001";
    let repository_id = "repo_0001";

    let story = lifecycle_store
        .create_story_spec(CreateStorySpecInput {
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            repository_id: repository_id.to_string(),
            title: "Story".to_string(),
        })
        .unwrap();
    let design = lifecycle_store
        .create_design_spec(CreateDesignSpecInput {
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            story_spec_ids: vec![story.id.clone()],
            title: "Design".to_string(),
        })
        .unwrap();

    let plan = lifecycle_store
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some("issue_work_item_plan_0001".to_string()),
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            source_story_spec_ids: vec![story.id.clone()],
            source_design_spec_ids: vec![design.id.clone()],
            options: IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Draft,
            work_item_ids: vec![],
            repository_profile_ref: None,
            verification_plan_ids: vec![],
            dependency_graph: vec![],
            created_from_provider_run: None,
            validator_findings: vec![],
        })
        .unwrap();

    let session_record = lifecycle_store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            entity_id: plan.id.clone(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 0,
            superpowers_enabled: false,
            openspec_enabled: false,
        })
        .unwrap();

    let session = WorkspaceSession::from_record(session_record);
    let (tx, _rx) = mpsc::channel(64);
    let mut engine =
        WorkspaceEngine::new_persistent(checkpoint_store, lifecycle_store.clone(), tx, session);

    fn make_error_output(story_id: &str, design_id: &str) -> WorkItemSplitProviderOutput {
        let now = chrono::Utc::now().to_rfc3339();
        let repository_profile = RepositoryProfile {
            id: "repo_profile_0001".to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repo_0001".to_string(),
            provider_run_ref: None,
            languages: vec!["rust".to_string()],
            frameworks: vec![],
            package_managers: vec!["cargo".to_string()],
            test_frameworks: vec![],
            build_systems: vec!["cargo".to_string()],
            verification_capabilities: vec![],
            detected_layers: vec!["backend".to_string()],
            split_recommendation: "backend_only".to_string(),
            confidence: RepositoryProfileConfidence::High,
            uncertainties: vec![],
            created_at: now.clone(),
            updated_at: now.clone(),
        };

        let work_items = vec![LifecycleWorkItemRecord {
            id: "wi_err_001".to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repo_0001".to_string(),
            story_spec_ids: vec![story_id.to_string()],
            design_spec_ids: vec![design_id.to_string()],
            title: "Error work item".to_string(),
            plan_status: WorkItemPlanStatus::Draft,
            execution_status: WorkItemStatus::Pending,
            worktree_path: None,
            work_item_set_id: None,
            source_work_item_plan_id: None,
            source_outline_id: None,
            source_draft_id: None,
            planned_implementation_context: None,
            planned_handoff_summary: None,
            kind: WorkItemKind::Backend,
            sequence_hint: None,
            depends_on: vec![],
            exclusive_write_scopes: vec![],
            forbidden_write_scopes: vec![],
            context_budget: WorkItemContextBudget::default(),
            required_handoff_from: vec![],
            verification_plan_ref: Some("vp_err_001".to_string()),
            require_execution_plan_confirm: false,
            execution_plan_status: WorkItemExecutionPlanStatus::NotStarted,
            handoff_summary_ref: None,
            completion_commit: None,
            completion_diff_summary_ref: None,
            created_at: now.clone(),
            updated_at: now.clone(),
        }];

        let verification_plans = vec![VerificationPlan {
            id: "vp_err_001".to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "wi_err_001".to_string(),
            repository_profile_ref: Some("repo_profile_0001".to_string()),
            provider_run_ref: None,
            scope: VerificationScope::Unit,
            commands: vec![VerificationCommand {
                id: "cmd_001".to_string(),
                label: "cargo test".to_string(),
                command: "cargo test".to_string(),
                cwd: "".to_string(),
                purpose: "unit tests".to_string(),
                required: true,
                timeout_seconds: 120,
                source: VerificationCommandSource::Provider,
                safety: VerificationCommandSafety::Approved,
            }],
            manual_checks: vec![VerificationManualCheck {
                id: "check_001".to_string(),
                label: "manual".to_string(),
                instructions: "check".to_string(),
                required: false,
            }],
            required_gates: vec!["cmd_001".to_string()],
            risk_notes: vec![],
            confidence: RepositoryProfileConfidence::High,
            fallback_policy: VerificationFallbackPolicy::ManualGate,
            created_at: now.clone(),
            updated_at: now.clone(),
        }];

        let plan_record = crate::product::models::IssueWorkItemPlan {
            id: "issue_work_item_plan_0001".to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            source_story_spec_ids: vec![story_id.to_string()],
            source_design_spec_ids: vec![design_id.to_string()],
            options: IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Draft,
            work_item_ids: vec!["wi_err_001".to_string()],
            repository_profile_ref: Some("repo_profile_0001".to_string()),
            verification_plan_ids: vec!["vp_err_001".to_string()],
            dependency_graph: vec![],
            created_from_provider_run: None,
            validator_findings: vec![],
            review_summary: None,
            created_at: now.clone(),
            updated_at: now.clone(),
        };

        WorkItemSplitProviderOutput {
            repository_profile,
            plan: plan_record,
            work_items,
            verification_plans,
        }
    }

    let outcome = engine
        .complete_work_item_plan_author(make_error_output(&story.id, &design.id))
        .await
        .expect("first call should succeed");
    assert!(
        matches!(outcome, WorkItemPlanAuthorOutcome::AutoRevision { .. }),
        "first error should trigger auto revision, got {outcome:?}"
    );
    assert_eq!(engine.work_item_plan_author_retry_count, 1);

    let outcome = engine
        .complete_work_item_plan_author(make_error_output(&story.id, &design.id))
        .await
        .expect("second call should succeed");
    assert!(
        matches!(outcome, WorkItemPlanAuthorOutcome::AutoRevision { .. }),
        "second error should trigger auto revision, got {outcome:?}"
    );
    assert_eq!(engine.work_item_plan_author_retry_count, 2);

    let outcome = engine
        .complete_work_item_plan_author(make_error_output(&story.id, &design.id))
        .await
        .expect("third call should succeed");
    assert!(
        matches!(outcome, WorkItemPlanAuthorOutcome::HumanConfirm { .. }),
        "third error should escalate to human confirm, got {outcome:?}"
    );
    assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);

    let persisted = lifecycle_store
        .get_issue_work_item_plan(project_id, issue_id, &plan.id)
        .unwrap();
    assert!(
        persisted
            .validator_findings
            .iter()
            .any(|f| f.severity == WorkItemSplitFindingSeverity::Error)
    );
}
