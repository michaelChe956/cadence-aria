use super::*;
use crate::product::coding_models::{CodingUnitRun, CodingUnitRunStatus};
use crate::product::work_item_contract::{BlockerRoute, BlockerRule, WorkItemWritePolicy};
use crate::product::work_item_projection::ReviewerWorkItemProjection;

#[test]
fn coding_plan_repair_entrypoints_internal_and_group_review_use_group_reviewer_route() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let role_run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::InternalPrReview,
            CodingProviderRole::InternalReviewer,
            CodingRoleRunTrigger::Initial,
            None,
        )
        .unwrap();
    let request = ReviewRequest {
        id: "review_request_0001".to_string(),
        attempt_id: attempt.id.clone(),
        kind: ReviewRequestKind::GitBranchOnly,
        remote_kind: RemoteKind::GenericGit,
        remote: "origin".to_string(),
        base_branch: attempt.base_branch.clone(),
        branch_name: attempt.branch_name.clone(),
        commit_sha: "commit_0001".to_string(),
        push_status: PushStatus::Pushed,
        external_url: None,
        manual_instructions: Vec::new(),
        created_at: "2026-07-18T00:00:00Z".to_string(),
        updated_at: "2026-07-18T00:00:00Z".to_string(),
        push_error: None,
    };
    let projection = reviewer_projection_fixture();

    for source_stage in ["internal_pr_review", "group_final_review"] {
        let output = serde_json::json!({
            "verdict": "blocked",
            "findings": [{
                "source_stage": source_stage,
                "severity": "error",
                "defect_class": "current_work_item_invalid",
                "reason_code": "current_work_item_contract_invalid",
                "message": "current contract invalid",
                "contract_refs": [],
                "capability_refs": [],
                "repair_target": {
                    "kind": "current_work_item",
                    "logical_work_item_ids": ["work_item_0001"],
                    "work_item_revision_ids": ["work_item_revision_0001"]
                },
                "recommended_route": "plan_repair",
                "confidence": "high",
                "evidence": []
            }]
        })
        .to_string();
        let review = engine
            .build_internal_pr_review(&attempt, &request, &output, None, &role_run)
            .unwrap();

        assert_eq!(
            internal_review_flow_decision(&review, &projection),
            CodeReviewFlowDecision::StartPlanRepair
        );
    }
}

#[tokio::test]
async fn coding_plan_repair_entrypoints_internal_review_execution_persists_safe_route() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    init_test_git_repo(attempt.worktree_path.as_ref().unwrap());
    let request = ReviewRequest {
        id: "review_request_0001".to_string(),
        attempt_id: attempt.id.clone(),
        kind: ReviewRequestKind::GitBranchOnly,
        remote_kind: RemoteKind::GenericGit,
        remote: "origin".to_string(),
        base_branch: attempt.base_branch.clone(),
        branch_name: attempt.branch_name.clone(),
        commit_sha: git_stdout(
            attempt.worktree_path.as_ref().unwrap(),
            &["rev-parse", "HEAD"],
        ),
        push_status: PushStatus::Pushed,
        external_url: None,
        manual_instructions: Vec::new(),
        created_at: "2026-07-18T00:00:00Z".to_string(),
        updated_at: "2026-07-18T00:00:00Z".to_string(),
        push_error: None,
    };
    store.save_review_request(&attempt, &request).unwrap();
    let (tx, _rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = super::provider_execution_context::CapturingProjectionProvider::new(
        super::provider_execution_context::review_plan_defect_output(),
    );

    let review = engine
        .execute_internal_pr_review(&attempt, &provider)
        .await
        .unwrap();

    assert_eq!(review.verdict, ReviewVerdict::Blocked);
    let entry = store
        .list_chat_entries(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap()
        .into_iter()
        .find(|entry| {
            entry
                .metadata
                .as_ref()
                .is_some_and(|metadata| metadata["source"] == "internal_pr_review")
        })
        .expect("internal review chat entry");
    assert_eq!(
        entry.metadata.as_ref().unwrap()["plan_defect_route"],
        "stop_for_human_triage"
    );
    let persisted = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("persisted attempt");
    assert_eq!(persisted.status, CodingAttemptStatus::Blocked);
    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("blocked gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(
        gates[0].reason_code.as_deref(),
        Some("internal_review_human_triage")
    );
    assert_eq!(gates[0].title, "Internal PR review requires human triage");
    assert!(
        gates[0]
            .available_actions
            .iter()
            .all(|action| action.action_id != "send_to_coder"),
        "human triage gate must not expose coder rework",
    );
}

#[test]
fn coding_plan_repair_group_reviewer_routes_no_target_story_and_design_findings() {
    for (class, route, blocker_route, expected) in [
        (
            crate::product::models::PlanDefectClass::StoryAmendmentRequired,
            crate::product::models::PlanDefectRoute::StoryAmendment,
            BlockerRoute::StoryAmendment,
            CodeReviewFlowDecision::StartStoryAmendment,
        ),
        (
            crate::product::models::PlanDefectClass::DesignAmendmentRequired,
            crate::product::models::PlanDefectRoute::DesignAmendment,
            BlockerRoute::DesignAmendment,
            CodeReviewFlowDecision::StartDesignAmendment,
        ),
    ] {
        let review = internal_review_with_finding(no_target_finding(class, route));
        let bindings = vec![
            reviewer_binding("work_item_0001", blocker_route.clone(), Vec::new()),
            reviewer_binding("work_item_0002", blocker_route, Vec::new()),
        ];

        assert_eq!(
            internal_review_flow_decision_with_bindings(&review, &bindings),
            expected,
        );
    }
}

#[test]
fn coding_plan_repair_group_reviewer_no_target_projection_match_is_fail_closed() {
    let review = internal_review_with_finding(no_target_finding(
        crate::product::models::PlanDefectClass::StoryAmendmentRequired,
        crate::product::models::PlanDefectRoute::StoryAmendment,
    ));
    let no_match = vec![reviewer_binding(
        "work_item_0001",
        BlockerRoute::DesignAmendment,
        Vec::new(),
    )];
    let ambiguous = vec![
        reviewer_binding(
            "work_item_0001",
            BlockerRoute::StoryAmendment,
            vec!["contract_a".to_string()],
        ),
        reviewer_binding(
            "work_item_0002",
            BlockerRoute::StoryAmendment,
            vec!["contract_b".to_string()],
        ),
    ];

    assert_eq!(
        internal_review_flow_decision_with_bindings(&review, &no_match),
        CodeReviewFlowDecision::StopForHumanTriage,
    );
    assert_eq!(
        internal_review_flow_decision_with_bindings(&review, &ambiguous),
        CodeReviewFlowDecision::StopForHumanTriage,
    );
}

#[test]
fn coding_plan_repair_group_reviewer_loads_authoritative_no_target_projections() {
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
            worktree_path: Some(root.path().join("worktree")),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("group attempt");
    seed_group_attempt_fixture(&store, &attempt, true, false);
    complete_group_units_with_authoritative_runs(&store, &attempt);
    let mut attempt = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("attempt");
    attempt.status = CodingAttemptStatus::Running;
    attempt.stage = CodingExecutionStage::InternalPrReview;
    store.save_coding_attempt(&attempt).expect("save attempt");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let review = internal_review_with_finding(no_target_finding(
        crate::product::models::PlanDefectClass::StoryAmendmentRequired,
        crate::product::models::PlanDefectRoute::StoryAmendment,
    ));

    assert_eq!(
        engine
            .internal_review_flow_decision_for_attempt(&attempt, &review)
            .expect("authoritative group decision"),
        CodeReviewFlowDecision::StartStoryAmendment,
    );
}

#[test]
fn coding_plan_repair_group_reviewer_rejects_stale_completed_run_when_latest_is_running() {
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
            worktree_path: Some(root.path().join("worktree")),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("group attempt");
    seed_group_attempt_fixture(&store, &attempt, true, false);
    complete_group_units_with_authoritative_runs(&store, &attempt);
    let unit = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("units")
        .into_iter()
        .min_by_key(|unit| unit.order_index)
        .expect("first unit");
    let completed = store
        .list_coding_unit_runs(&attempt, &unit.id)
        .expect("runs")
        .into_iter()
        .max_by_key(|run| run.execution_no)
        .expect("completed run");
    let mut latest = completed;
    latest.id = "coding_unit_run_latest_running".to_string();
    latest.execution_no += 1;
    latest.status = CodingUnitRunStatus::Running;
    latest.completion_commit = None;
    store
        .create_coding_unit_run(&attempt, &latest)
        .expect("latest running run");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let review = internal_review_with_finding(no_target_finding(
        crate::product::models::PlanDefectClass::StoryAmendmentRequired,
        crate::product::models::PlanDefectRoute::StoryAmendment,
    ));

    let error = engine
        .internal_review_flow_decision_for_attempt(&attempt, &review)
        .expect_err("latest authoritative UnitRun must be completed");

    assert!(
        error
            .to_string()
            .contains("group_review_unit_run_not_completed")
    );
}

#[test]
fn coding_plan_repair_internal_review_gate_reasons_follow_one_decision_mapping() {
    for decision in [
        CodeReviewFlowDecision::StartPlanRepair,
        CodeReviewFlowDecision::StartStoryAmendment,
        CodeReviewFlowDecision::StartDesignAmendment,
        CodeReviewFlowDecision::ContinueAfterApprove,
    ] {
        assert_eq!(internal_review_blocked_gate_reason(decision, true), None);
    }
    assert_eq!(
        internal_review_blocked_gate_reason(CodeReviewFlowDecision::RetryVerification, true),
        Some("internal_review_verification_incomplete")
    );
    assert_eq!(
        internal_review_blocked_gate_reason(CodeReviewFlowDecision::RunCoderFix, true),
        Some("group_final_review_blocked")
    );
    assert_eq!(
        internal_review_blocked_gate_reason(CodeReviewFlowDecision::RunCoderFix, false),
        Some("internal_review_change_requested")
    );
    assert_eq!(
        internal_review_blocked_gate_reason(CodeReviewFlowDecision::StopForHumanTriage, true),
        Some("internal_review_human_triage")
    );
    assert_eq!(
        internal_review_blocked_gate_reason(CodeReviewFlowDecision::OpenOperationalGate, true),
        Some("internal_review_operational_blocker")
    );

    let distinct_reasons = [
        internal_review_blocked_gate_reason(CodeReviewFlowDecision::RunCoderFix, true).unwrap(),
        internal_review_blocked_gate_reason(CodeReviewFlowDecision::RetryVerification, true)
            .unwrap(),
        internal_review_blocked_gate_reason(CodeReviewFlowDecision::StopForHumanTriage, true)
            .unwrap(),
        internal_review_blocked_gate_reason(CodeReviewFlowDecision::OpenOperationalGate, true)
            .unwrap(),
    ]
    .into_iter()
    .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(distinct_reasons.len(), 4);
}

#[tokio::test]
async fn coding_plan_repair_group_review_non_gate_routes_and_verification_gate_stay_distinct() {
    let (_root, store, attempt, engine, _event_rx) = prepared_group_review_fixture();

    for (
        defect_class,
        reason_code,
        route,
        repair_target,
        expected,
        expected_gate_reason,
        expected_gate_title,
    ) in [
        (
            "story_amendment_required",
            "story_scope_invalid",
            "story_amendment",
            serde_json::Value::Null,
            CodeReviewFlowDecision::StartStoryAmendment,
            None,
            None,
        ),
        (
            "design_amendment_required",
            "design_constraint_invalid",
            "design_amendment",
            serde_json::Value::Null,
            CodeReviewFlowDecision::StartDesignAmendment,
            None,
            None,
        ),
        (
            "current_work_item_invalid",
            "current_work_item_contract_invalid",
            "plan_repair",
            serde_json::json!({
                "kind": "current_work_item",
                "logical_work_item_ids": ["work_item_0001"],
                "work_item_revision_ids": ["work_item_revision_0001"]
            }),
            CodeReviewFlowDecision::StartPlanRepair,
            None,
            None,
        ),
        (
            "verification_incomplete",
            "verification_incomplete",
            "verification_retry",
            serde_json::Value::Null,
            CodeReviewFlowDecision::RetryVerification,
            Some("internal_review_verification_incomplete"),
            Some("GroupFinalReview verification incomplete"),
        ),
    ] {
        let provider = super::provider_execution_context::CapturingProjectionProvider::new(
            serde_json::json!({
                "verdict": "blocked",
                "summary": "safe stop",
                "findings": [{
                    "source_stage": "group_final_review",
                    "severity": "error",
                    "defect_class": defect_class,
                    "reason_code": reason_code,
                    "message": "repair outside coding",
                    "contract_refs": [],
                    "capability_refs": [],
                    "repair_target": repair_target,
                    "recommended_route": route,
                    "confidence": "high",
                    "evidence": []
                }],
                "impact_scope": [],
                "pr_description": "",
                "commit_message_suggestion": ""
            })
            .to_string(),
        );

        let review = engine
            .execute_internal_pr_review(&attempt, &provider)
            .await
            .expect("group review route");

        assert_eq!(review.verdict, ReviewVerdict::Blocked);
        let persisted = store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("persisted attempt");
        assert_eq!(persisted.stage, CodingExecutionStage::InternalPrReview);
        let gates = store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("blocked gates");
        match expected_gate_reason {
            Some(expected_gate_reason) => {
                assert_eq!(persisted.status, CodingAttemptStatus::Blocked);
                assert_eq!(gates.len(), 1);
                assert_eq!(gates[0].reason_code.as_deref(), Some(expected_gate_reason));
                assert_eq!(gates[0].title, expected_gate_title.unwrap());
            }
            None => {
                assert_eq!(persisted.status, CodingAttemptStatus::Running);
                assert!(gates.is_empty());
            }
        }
        let entry = store
            .list_chat_entries(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("chat entries")
            .into_iter()
            .rfind(|entry| {
                entry.metadata.as_ref().is_some_and(|metadata| {
                    metadata.get("source").and_then(|value| value.as_str())
                        == Some("internal_pr_review")
                })
            })
            .expect("group review chat entry");
        assert_eq!(
            entry.metadata.as_ref().expect("metadata")["plan_defect_route"],
            expected.label()
        );
    }
}

pub(super) fn prepared_group_review_fixture() -> (
    tempfile::TempDir,
    CodingAttemptStore,
    CodingExecutionAttempt,
    CodingWorkspaceEngine,
    mpsc::Receiver<CodingWsOutMessage>,
) {
    let root = tempdir().expect("tempdir");
    let worktree = root.path().join("worktree");
    std::fs::create_dir_all(&worktree).expect("worktree");
    init_test_git_repo(&worktree);
    let head = git_stdout(&worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let mut attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: head.clone(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("group attempt");
    seed_group_attempt_fixture(&store, &attempt, true, false);
    complete_group_units_with_authoritative_runs(&store, &attempt);
    attempt.status = CodingAttemptStatus::Running;
    attempt.stage = CodingExecutionStage::InternalPrReview;
    attempt.head_commit = Some(head.clone());
    store.save_coding_attempt(&attempt).expect("save attempt");
    store
        .save_review_request(
            &attempt,
            &ReviewRequest {
                id: "review_request_0001".to_string(),
                attempt_id: attempt.id.clone(),
                kind: ReviewRequestKind::GitBranchOnly,
                remote_kind: RemoteKind::GenericGit,
                remote: "origin".to_string(),
                base_branch: attempt.base_branch.clone(),
                branch_name: attempt.branch_name.clone(),
                commit_sha: head,
                push_status: PushStatus::Pushed,
                external_url: None,
                manual_instructions: Vec::new(),
                created_at: "2026-07-19T00:00:00Z".to_string(),
                updated_at: "2026-07-19T00:00:00Z".to_string(),
                push_error: None,
            },
        )
        .expect("review request");
    let (tx, _rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    (root, store, attempt, engine, _rx)
}

fn complete_group_units_with_authoritative_runs(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) {
    let revision_store = WorkItemRevisionStore::new(store.paths());
    let lineage = revision_store
        .get_plan_lineage(
            &attempt.project_id,
            &attempt.issue_id,
            "work_item_plan_0001",
        )
        .expect("lineage");
    let units = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("units");
    for unit in units {
        let revision = revision_store
            .get_work_item_revision(
                &lineage,
                &unit.logical_work_item_id,
                &unit.work_item_revision_id,
            )
            .expect("revision");
        let bundle = revision_store
            .get_work_item_projection_bundle(&lineage, &revision.work_item_projection_bundle_id)
            .expect("projection bundle");
        store
            .create_coding_unit_run(
                attempt,
                &CodingUnitRun {
                    id: format!("coding_unit_run_{}", unit.order_index + 1),
                    unit_id: unit.id.clone(),
                    execution_no: 1,
                    work_item_revision_id: unit.work_item_revision_id.clone(),
                    resolved_handoff_revision_ids: Vec::new(),
                    canonical_contract_hash: bundle.canonical_contract_hash,
                    projection_bundle_id: bundle.id,
                    projection_compiler_version: bundle.compiler_version,
                    coder_provider_renderer_version:
                        crate::product::work_item_projection::renderer_for(&ProviderName::Codex)
                            .renderer_version()
                            .to_string(),
                    reviewer_provider_renderer_version:
                        crate::product::work_item_projection::renderer_for(
                            &ProviderName::ClaudeCode,
                        )
                        .renderer_version()
                        .to_string(),
                    internal_reviewer_provider_renderer_version: None,
                    coder_projection_hash: bundle.coder_projection_hash,
                    reviewer_projection_hash: bundle.reviewer_projection_hash,
                    coder_execution_context_hash: None,
                    reviewer_execution_context_hash: None,
                    internal_reviewer_execution_context_hash: None,
                    status: CodingUnitRunStatus::Completed,
                    unit_rework_count: 0,
                    verification_retry_count: 0,
                    operational_retry_count: 0,
                    plan_repair_count: 0,
                    start_commit: None,
                    completion_commit: Some("commit_0001".to_string()),
                    created_at: "2026-07-19T00:00:00Z".to_string(),
                    updated_at: "2026-07-19T00:00:00Z".to_string(),
                },
            )
            .expect("unit run");
        store
            .update_coding_unit_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &unit.id,
                CodingExecutionUnitStatus::Completed,
                None,
            )
            .expect("complete unit");
    }
}

fn internal_review_with_finding(finding: ReviewFinding) -> InternalPrReview {
    InternalPrReview {
        id: "internal_review_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        review_request_id: "review_request_0001".to_string(),
        verdict: ReviewVerdict::Blocked,
        findings: vec![finding],
        impact_scope: Vec::new(),
        pr_description: String::new(),
        commit_message_suggestion: String::new(),
        tested_evidence_refs: Vec::new(),
        diff_refs: Vec::new(),
        summary: "plan defect".to_string(),
        created_at: "2026-07-19T00:00:00Z".to_string(),
        raw_provider_output_ref: None,
        role_run_id: None,
        run_no: None,
    }
}

fn no_target_finding(
    defect_class: crate::product::models::PlanDefectClass,
    recommended_route: crate::product::models::PlanDefectRoute,
) -> ReviewFinding {
    let reason_code = match defect_class {
        crate::product::models::PlanDefectClass::StoryAmendmentRequired => "story_scope_invalid",
        crate::product::models::PlanDefectClass::DesignAmendmentRequired => {
            "design_constraint_invalid"
        }
        _ => "group_specification_invalid",
    };
    ReviewFinding {
        severity: FindingSeverity::Error,
        file_path: None,
        line: None,
        message: "upstream specification must change".to_string(),
        required_action: None,
        source_stage: CodingExecutionStage::InternalPrReview,
        evidence: Vec::new(),
        plan_defect_evidence: Vec::new(),
        related_requirements: Vec::new(),
        related_design_constraints: Vec::new(),
        related_work_item_tasks: Vec::new(),
        defect_class,
        reason_code: Some(reason_code.to_string()),
        contract_refs: Vec::new(),
        capability_refs: Vec::new(),
        repair_target: None,
        recommended_route,
        confidence: Some(crate::product::plan_repair::PlanDefectConfidence::High),
    }
}

fn reviewer_binding(
    logical_work_item_id: &str,
    route: BlockerRoute,
    target_contract_refs: Vec<String>,
) -> GroupReviewerProjectionBinding {
    let reason_code = match &route {
        BlockerRoute::StoryAmendment => "story_scope_invalid",
        BlockerRoute::DesignAmendment => "design_constraint_invalid",
        _ => "group_specification_invalid",
    };
    GroupReviewerProjectionBinding {
        logical_work_item_id: logical_work_item_id.to_string(),
        projection: ReviewerWorkItemProjection {
            work_item_revision_id: format!("{logical_work_item_id}_revision"),
            criterion_refs: Vec::new(),
            requirement_matrix: Vec::new(),
            scope_policy: WorkItemWritePolicy {
                exclusive_scopes: Vec::new(),
                forbidden_scopes: Vec::new(),
            },
            input_contract_checks: Vec::new(),
            output_contract_checks: Vec::new(),
            verification_evidence_rules: Vec::new(),
            blocker_routing: vec![BlockerRule {
                reason_code: reason_code.to_string(),
                route,
                target_contract_refs,
            }],
        },
    }
}

fn reviewer_projection_fixture() -> ReviewerWorkItemProjection {
    ReviewerWorkItemProjection {
        work_item_revision_id: "work_item_revision_0001".to_string(),
        criterion_refs: Vec::new(),
        requirement_matrix: Vec::new(),
        scope_policy: WorkItemWritePolicy {
            exclusive_scopes: Vec::new(),
            forbidden_scopes: Vec::new(),
        },
        input_contract_checks: Vec::new(),
        output_contract_checks: Vec::new(),
        verification_evidence_rules: Vec::new(),
        blocker_routing: vec![BlockerRule {
            reason_code: "current_work_item_contract_invalid".to_string(),
            route: BlockerRoute::PlanRepairCurrent,
            target_contract_refs: Vec::new(),
        }],
    }
}
