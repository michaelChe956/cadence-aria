use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::cross_cutting::streaming_provider::{
    ProviderSession, StreamChunk, StreamingProviderAdapter, StreamingProviderInput,
};
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::{
    CodingAttemptStore, CreateBlockedGateInput, CreateCodingAttemptInput,
};
use crate::product::coding_models::{
    CodeReviewReport, CodingAgentRole, CodingAttemptScope, CodingAttemptStatus,
    CodingExecutionStage, CodingGateAction, CodingGateActionType, CodingProviderPermissionMode,
    CodingRolePermissionModes, CodingRoleProviderConfigSnapshot, CodingTimelineNode,
    CodingTimelineNodeStatus, FindingSeverity, GroupFinalReadinessSnapshot,
    GroupFinalReadinessStatus, GroupFinalReadinessUnit, ReviewFinding, ReviewVerdict,
};
use crate::product::coding_work_item_context::select_work_item_markdown;
use crate::product::coding_workspace_engine::CodingWorkspaceEngine;
use crate::product::coding_workspace_runner::CodingRunnerCommand;
use crate::product::git_workspace_service::GitWorkspaceService;
use crate::product::lifecycle_store::{
    CreateVerificationPlanInput, CreateWorkItemInput, CreateWorkspaceSessionInput, LifecycleStore,
};
use crate::product::models::{
    ProviderName, RepositoryProfileConfidence, VerificationCommand, VerificationCommandSafety,
    VerificationCommandSource, VerificationFallbackPolicy, VerificationScope,
    WorkItemDraftCandidate, WorkItemDraftRecord, WorkItemDraftStatus, WorkItemGenerationMode,
    WorkItemKind, WorkItemPlanStatus, WorkspaceMessageRecord, WorkspaceSessionRecord,
    WorkspaceSessionStatus, WorkspaceType,
};
use crate::product::test_executor::planned_test_commands_from_markdown;
use crate::product::work_item_plan_store::WorkItemPlanStore;
use crate::protocol::contracts::AdapterInput;
use crate::web::runtime::WebRuntime;
use crate::web::state::WebAppState;
use crate::web::workspace_ws_types::{ArtifactPayload, ArtifactVersion};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::{
    CodeReviewFlowDecision, CodingExecutionAttempt, CodingWsInMessage, CodingWsOutMessage,
    ProviderConfigSnapshot, build_coding_session_state, code_review_flow_decision,
    coding_execution_context, is_coding_ws_message_allowed,
    should_emit_coding_runner_protocol_error, should_resume_runner_after_gate_response,
};

mod code_review_router;
mod failed_review_recovery;
mod plan_repair;
mod runner_cleanup;

#[tokio::test]
async fn coding_pi_start_failure_does_not_start_registered_alternate_provider() {
    let root = tempfile::tempdir().expect("root");
    let worktree = root.path().join("worktree");
    std::fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Pi,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("create attempt");
    let attempt = store
        .seed_running_attempt_for_test(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("running attempt");
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::Coding,
        )
        .expect("coding stage");
    store
        .update_role_provider_config_snapshot(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingRoleProviderConfigSnapshot {
                coder: ProviderName::Pi,
                code_reviewer: ProviderName::ClaudeCode,
                internal_reviewer: ProviderName::ClaudeCode,
                review_rounds: 1,
                permission_modes: CodingRolePermissionModes {
                    coder: CodingProviderPermissionMode::Supervised,
                    code_reviewer: CodingProviderPermissionMode::Auto,
                    internal_reviewer: CodingProviderPermissionMode::Auto,
                },
            },
        )
        .expect("set Pi role config");

    let pi_starts = Arc::new(AtomicUsize::new(0));
    let alternate_starts = Arc::new(AtomicUsize::new(0));
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Pi,
        Arc::new(FailingCodingProvider {
            starts: Arc::clone(&pi_starts),
        }),
    );
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(AlternateCodingProvider {
            starts: Arc::clone(&alternate_starts),
        }),
    );
    let mut state = WebAppState::with_provider_registry(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        registry,
    );
    state.test_provider_enabled = true;
    let (event_tx, _event_rx) = mpsc::channel(16);
    let engine =
        CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx.clone());
    let (command_tx, command_rx) = mpsc::channel(1);
    command_tx
        .send(CodingRunnerCommand::StageGateConfirm {
            stage: CodingExecutionStage::Coding,
        })
        .await
        .expect("confirm coding stage gate");

    let result =
        super::execute_start_coding_flow(&state, &store, &engine, &event_tx, command_rx, &attempt)
            .await;

    assert!(
        result.is_err(),
        "Pi startup failure must terminate the coding run"
    );
    assert_eq!(pi_starts.load(Ordering::SeqCst), 1, "Pi starts once");
    assert_eq!(
        alternate_starts.load(Ordering::SeqCst),
        0,
        "Pi failure must not fall back to a registered alternate provider"
    );
}

struct FailingCodingProvider {
    starts: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for FailingCodingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "Pi start failed",
            0,
        ))
    }

    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        unreachable!("coding must use start")
    }
}

struct AlternateCodingProvider {
    starts: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for AlternateCodingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        unreachable!("Pi failure must not start the alternate provider")
    }

    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        unreachable!("coding must use start")
    }
}

#[test]
fn falls_back_to_assistant_artifact_when_persisted_markdown_lacks_commands() {
    let session = WorkspaceSessionRecord {
        id: "workspace_session_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        entity_id: "work_item_0001".to_string(),
        workspace_type: WorkspaceType::WorkItem,
        status: WorkspaceSessionStatus::Confirmed,
        author_provider: ProviderName::Codex,
        reviewer_provider: ProviderName::ClaudeCode,
        review_rounds: 1,
        permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
        provisional_reviewer_provider: None,
        reviewer_enabled_at_start: None,
        superpowers_enabled: true,
        openspec_enabled: true,
        work_item_runtime_binding: None,
        provider_conversations: Vec::new(),
        messages: vec![WorkspaceMessageRecord {
            role: "assistant".to_string(),
            content: "```artifact\n# Work Item\n\n## 验证命令\n\n```bash\nuv run python -m unittest discover -s tests -v\n```\n```"
                .to_string(),
            created_at: "2026-05-28T00:00:00Z".to_string(),
        }],
        created_at: "2026-05-28T00:00:00Z".to_string(),
        updated_at: "2026-05-28T00:00:00Z".to_string(),
        flow_kind: crate::product::work_item_plan_policy::WorkItemPlanFlowKind::Legacy,
        run_policy: crate::product::work_item_plan_policy::RunPolicy::Interactive,
        run_history: crate::product::work_item_plan_policy::RunHistory::default(),
        review_invocation_scope: None,
        human_gate_snapshot: None,
        repair_reservation: None,
        human_gate_reservation: None,
        policy_diagnostics: Vec::new(),
        provider_start_ledger: Vec::new(),
        single_candidate_phase: None,
        work_item_plan_source_revision_ref: None,
        plan_candidate_ir_ref: None,
        mechanical_report_ref: None,
        publication_provenance_ref: None,
        approval_attempt_id: None,
        approved_at: None,
        compile_reservation: None,
    };

    let selected = select_work_item_markdown(
        Some("# Work Item\n\n## 验证命令\n\n首选无第三方测试依赖命令：".to_string()),
        &session,
    )
    .expect("selected markdown");

    assert!(selected.contains("uv run python -m unittest discover -s tests -v"));
    assert_eq!(
        planned_test_commands_from_markdown(&selected)[0].command,
        vec![
            "uv", "run", "python", "-m", "unittest", "discover", "-s", "tests", "-v"
        ]
    );
}

#[test]
fn coding_execution_context_uses_final_compile_work_item_when_workspace_artifact_missing() {
    let (_tmp, app_paths, attempt) = seed_compiled_work_item_fixture();

    let context = coding_execution_context(&app_paths, &attempt).expect("coding context");

    let markdown = context.work_item_markdown.expect("work item markdown");
    assert!(markdown.contains("# Final Compile Work Item"));
    assert!(markdown.contains("work_item_compile_20260702063721302_001"));
    assert!(markdown.contains("Final Compile title"));
    assert!(markdown.contains("source_work_item_plan_id: issue_work_item_plan_0001"));
    assert!(markdown.contains("source_outline_id: outline_backend"));
    assert!(markdown.contains("source_draft_id: draft_backend"));
    assert!(markdown.contains("planned implementation context for coder"));
    assert!(markdown.contains("src/web/coding_ws_handler/context.rs"));
    assert!(markdown.contains("forbidden/path"));
    assert!(markdown.contains("verification_plan_compile_20260702063721302_001"));
    assert!(markdown.contains("cargo test --locked --lib coding_execution_context"));
    assert_eq!(
        context.verification_commands,
        vec!["cargo test --locked --lib coding_execution_context".to_string()]
    );
}

#[test]
fn code_review_flow_decision_routes_reviewer_verdicts() {
    let projection = code_review_router::reviewer_projection_fixture();
    assert_eq!(
        code_review_flow_decision(
            &code_review_report_with(ReviewVerdict::RequestChanges, Vec::new()),
            &projection
        ),
        CodeReviewFlowDecision::RunCoderFix
    );
    assert_eq!(
        code_review_flow_decision(
            &code_review_report_with(
                ReviewVerdict::Blocked,
                vec![code_review_router::implementation_finding()]
            ),
            &projection
        ),
        CodeReviewFlowDecision::RunCoderFix
    );
    assert_eq!(
        code_review_flow_decision(
            &code_review_report_with(ReviewVerdict::Blocked, Vec::new()),
            &projection,
        ),
        CodeReviewFlowDecision::StopForHumanTriage
    );
    assert_eq!(
        code_review_flow_decision(
            &code_review_report_with(ReviewVerdict::Approve, Vec::new()),
            &projection,
        ),
        CodeReviewFlowDecision::ContinueAfterApprove
    );
}

fn code_review_report_with(
    verdict: ReviewVerdict,
    findings: Vec<ReviewFinding>,
) -> CodeReviewReport {
    CodeReviewReport {
        id: "code_review_report_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        round: 1,
        verdict,
        findings,
        tested_evidence_refs: Vec::new(),
        diff_refs: Vec::new(),
        summary: "summary".to_string(),
        created_at: "2026-07-07T00:00:00Z".to_string(),
        raw_provider_output_ref: None,
        role_run_id: None,
        run_no: None,
        unit_run_id: None,
    }
}

#[test]
fn coding_execution_context_supplements_source_draft_when_final_compile_context_is_missing() {
    let (_tmp, app_paths, mut attempt) = seed_compiled_work_item_fixture();
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let sparse_work_item_id = "work_item_compile_sparse_001";
    let plan_id = "issue_work_item_plan_0001";
    let draft_id = "draft_sparse_backend";

    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some(sparse_work_item_id.to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            title: "Sparse final compile title".to_string(),
            source_work_item_plan_id: Some(plan_id.to_string()),
            source_outline_id: Some("outline_sparse_backend".to_string()),
            source_draft_id: Some(draft_id.to_string()),
            planned_implementation_context: None,
            verification_plan_ref: None,
            plan_status: WorkItemPlanStatus::Confirmed,
            ..Default::default()
        })
        .expect("create sparse work item");

    WorkItemPlanStore::new(app_paths.clone())
        .put_draft_record(&WorkItemDraftRecord {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: plan_id.to_string(),
            draft_id: draft_id.to_string(),
            outline_id: "outline_sparse_backend".to_string(),
            generation_round_id: "round_001".to_string(),
            batch_id: None,
            attempt_index: 1,
            outline_version_ref: "outline_version_001".to_string(),
            generation_mode: WorkItemGenerationMode::Serial,
            generation_diagnostics: None,
            candidate: {
                let mut contract = crate::product::work_item_contract::canonical_contract_fixture(
                    "wi_sparse_backend",
                );
                contract.identity.title = "Draft sparse backend".to_string();
                contract.identity.kind = "backend".to_string();
                contract.goal.summary = "restore draft context".to_string();
                contract.input_contracts.clear();
                contract.write_policy.exclusive_scopes =
                    vec!["src/web/coding_ws_handler/context.rs".to_string()];
                contract.write_policy.forbidden_scopes = vec!["forbidden/draft/path".to_string()];
                contract.verification_checks[0].command = Some("cargo check --locked".to_string());
                WorkItemDraftCandidate {
                    target_repository_id: None,
                    outline_id: "outline_sparse_backend".to_string(),
                    logical_work_item_id: "wi_sparse_backend".to_string(),
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
            generated_from_node_id: "node_draft_author".to_string(),
            accepted_at: Some("2026-07-02T00:00:00Z".to_string()),
            superseded_at: None,
            created_at: "2026-07-02T00:00:00Z".to_string(),
            updated_at: "2026-07-02T00:00:00Z".to_string(),
        })
        .expect("put draft record");

    attempt.work_item_id = sparse_work_item_id.to_string();
    attempt.current_work_item_id = Some(sparse_work_item_id.to_string());

    let context = coding_execution_context(&app_paths, &attempt).expect("coding context");
    let markdown = context.work_item_markdown.expect("work item markdown");

    assert!(markdown.contains("# Final Compile Work Item"));
    assert!(markdown.contains("Sparse final compile title"));
    assert!(markdown.contains("## Source Draft Supplement"));
    assert!(markdown.contains("Draft Canonical Contract Candidate JSON"));
    assert!(markdown.contains("restore draft context"));
    assert!(markdown.contains("src/web/coding_ws_handler/context.rs"));
    assert!(!markdown.contains("implementation_context"));
}

#[test]
fn coding_execution_context_prefers_final_compile_over_workspace_artifact() {
    let (_tmp, app_paths, attempt) = seed_compiled_work_item_fixture();
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "work_item_compile_20260702063721302_001".to_string(),
            workspace_type: WorkspaceType::WorkItem,
            author_provider: ProviderName::Fake,
            reviewer_provider: ProviderName::Fake,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
            work_item_plan_options: None,
        })
        .expect("create workspace session");

    lifecycle
        .append_artifact_version(
            &session.id,
            ArtifactVersion {
                version: 1,
                payload: ArtifactPayload::Markdown {
                    markdown:
                        "# Workspace Work Item\n\n## 验证命令\n\n```bash\ncargo check --locked\n```"
                            .to_string(),
                    diff: None,
                },
                generated_by: ProviderName::Fake,
                reviewed_by: Some(ProviderName::Fake),
                review_verdict: None,
                confirmed_by: Some("user".to_string()),
                is_current: true,
                created_at: "2026-07-02T00:00:00Z".to_string(),
                source_node_id: "node_0001".to_string(),
            },
        )
        .expect("append artifact version");

    let context = coding_execution_context(&app_paths, &attempt).expect("coding context");
    let markdown = context.work_item_markdown.expect("work item markdown");
    assert!(markdown.contains("# Final Compile Work Item"));
    assert!(markdown.contains("planned implementation context for coder"));
    assert!(!markdown.contains("## Workspace Artifact Snapshot"));
    assert!(!markdown.contains("# Workspace Work Item"));
    assert_eq!(
        context.verification_commands,
        vec!["cargo test --locked --lib coding_execution_context".to_string()]
    );
}

#[test]
fn coding_execution_context_uses_workspace_artifact_when_final_compile_is_missing() {
    let (_tmp, app_paths, mut attempt) = seed_compiled_work_item_fixture();
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let legacy_work_item_id = "legacy_work_item_from_workspace";
    attempt.work_item_id = legacy_work_item_id.to_string();
    attempt.current_work_item_id = Some(legacy_work_item_id.to_string());

    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: legacy_work_item_id.to_string(),
            workspace_type: WorkspaceType::WorkItem,
            author_provider: ProviderName::Fake,
            reviewer_provider: ProviderName::Fake,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
            work_item_plan_options: None,
        })
        .expect("create workspace session");

    lifecycle
        .append_artifact_version(
            &session.id,
            ArtifactVersion {
                version: 1,
                payload: ArtifactPayload::Markdown {
                    markdown:
                        "# Workspace Work Item\n\n## 验证命令\n\n```bash\ncargo check --locked\n```"
                            .to_string(),
                    diff: None,
                },
                generated_by: ProviderName::Fake,
                reviewed_by: Some(ProviderName::Fake),
                review_verdict: None,
                confirmed_by: Some("user".to_string()),
                is_current: true,
                created_at: "2026-07-02T00:00:00Z".to_string(),
                source_node_id: "node_0001".to_string(),
            },
        )
        .expect("append artifact version");

    let context = coding_execution_context(&app_paths, &attempt).expect("coding context");
    let markdown = context.work_item_markdown.expect("work item markdown");

    assert!(markdown.contains("# Workspace Work Item"));
    assert!(!markdown.contains("# Final Compile Work Item"));
    assert_eq!(context.verification_commands, vec!["cargo check --locked"]);
}

#[test]
fn coding_session_state_includes_group_final_readiness_snapshot() {
    let (_tmp, app_paths, attempt) = seed_compiled_work_item_fixture();
    let coding_store = CodingAttemptStore::new(app_paths);
    coding_store
        .write_coding_attempt_for_test(&attempt)
        .expect("save coding attempt");
    let readiness = GroupFinalReadinessSnapshot {
        attempt_id: attempt.id.clone(),
        status: GroupFinalReadinessStatus::Complete,
        units: vec![GroupFinalReadinessUnit {
            unit_id: "coding_unit_0001".to_string(),
            logical_work_item_id: attempt.work_item_id.clone(),
            unit_run_id: Some("coding_unit_run_0001".to_string()),
            start_commit: Some("BASE".to_string()),
            completion_commit: Some("C2".to_string()),
            commit_shas: vec!["C1".to_string(), "C2".to_string()],
            diff_ref: "diffs/coding_unit_0001.patch".to_string(),
            empty_observation: false,
            code_review_report_id: Some("code_review_0001".to_string()),
            review_verdict: Some(ReviewVerdict::Approve),
            review_summary: Some("review ok".to_string()),
            review_findings: Some(Vec::new()),
            review_raw_provider_output_ref: None,
            handoff_revision_id: Some("handoff_revision_0001".to_string()),
            plan_revision_id: Some("plan_revision_0001".to_string()),
        }],
        diagnostics: Vec::new(),
        created_at: "2026-08-07T00:00:00Z".to_string(),
    };
    coding_store
        .write_group_final_readiness_snapshot(&attempt, &readiness)
        .expect("write group final readiness");

    let state = build_coding_session_state(&coding_store, attempt).expect("coding session state");
    let CodingWsOutMessage::CodingSessionState {
        group_final_readiness,
        ..
    } = state
    else {
        panic!("expected coding session state");
    };

    let readiness = group_final_readiness
        .as_ref()
        .as_ref()
        .expect("group final readiness must be included");
    assert_eq!(readiness.attempt_id, "coding_attempt_0001");
    assert_eq!(readiness.status, GroupFinalReadinessStatus::Complete);
    assert_eq!(readiness.units[0].commit_shas, vec!["C1", "C2"]);
}

#[test]
fn coding_session_state_omits_stale_blocked_gate_for_inactive_stage() {
    let (_tmp, app_paths, mut attempt) = seed_compiled_work_item_fixture();
    attempt.status = CodingAttemptStatus::Running;
    attempt.stage = CodingExecutionStage::CodeReview;

    let coding_store = CodingAttemptStore::new(app_paths);
    coding_store
        .write_coding_attempt_for_test(&attempt)
        .expect("save coding attempt");
    coding_store
        .create_blocked_gate(
            &attempt,
            CreateBlockedGateInput {
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::FinalConfirm,
                node_id: None,
                role: None,
                title: "Shared worktree has uncommitted changes".to_string(),
                description: "Issue shared worktree has uncommitted changes".to_string(),
                reason_code: Some("shared_worktree_dirty_manual_gate".to_string()),
                evidence_refs: Vec::new(),
                raw_provider_output_ref: None,
                available_actions: vec![CodingGateAction {
                    action_id: "manual_continue".to_string(),
                    label: "人工继续".to_string(),
                    action_type: CodingGateActionType::ManualContinue,
                }],
            },
        )
        .expect("create blocked gate");

    let state = build_coding_session_state(&coding_store, attempt).expect("coding session state");
    let CodingWsOutMessage::CodingSessionState { pending_gates, .. } = state else {
        panic!("expected coding session state");
    };

    assert!(
        pending_gates
            .iter()
            .all(|gate| gate.reason_code.as_deref() != Some("shared_worktree_dirty_manual_gate")),
        "stale final_confirm blocked gate must not be exposed while attempt is running code_review"
    );
}

#[test]
fn coding_session_state_keeps_final_confirm_blocked_gate_for_current_stage() {
    let (_tmp, app_paths, mut attempt) = seed_compiled_work_item_fixture();
    attempt.status = CodingAttemptStatus::Running;
    attempt.stage = CodingExecutionStage::FinalConfirm;

    let coding_store = CodingAttemptStore::new(app_paths);
    coding_store
        .write_coding_attempt_for_test(&attempt)
        .expect("save coding attempt");
    coding_store
        .create_blocked_gate(
            &attempt,
            CreateBlockedGateInput {
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::FinalConfirm,
                node_id: None,
                role: None,
                title: "Shared worktree has uncommitted changes".to_string(),
                description: "Issue shared worktree has uncommitted changes".to_string(),
                reason_code: Some("shared_worktree_dirty_manual_gate".to_string()),
                evidence_refs: Vec::new(),
                raw_provider_output_ref: None,
                available_actions: vec![CodingGateAction {
                    action_id: "manual_continue".to_string(),
                    label: "人工继续".to_string(),
                    action_type: CodingGateActionType::ManualContinue,
                }],
            },
        )
        .expect("create blocked gate");

    let state = build_coding_session_state(&coding_store, attempt).expect("coding session state");
    let CodingWsOutMessage::CodingSessionState { pending_gates, .. } = state else {
        panic!("expected coding session state");
    };

    assert!(pending_gates.iter().any(|gate| {
        gate.reason_code.as_deref() == Some("shared_worktree_dirty_manual_gate")
            && gate.stage.as_ref() == Some(&CodingExecutionStage::FinalConfirm)
    }));
}

#[test]
fn coding_session_state_does_not_reactivate_historical_blocked_node() {
    let (_tmp, app_paths, attempt) = seed_compiled_work_item_fixture();
    let coding_store = CodingAttemptStore::new(app_paths);
    coding_store
        .write_coding_attempt_for_test(&attempt)
        .expect("save coding attempt");
    coding_store
        .save_timeline_node(
            &attempt,
            CodingTimelineNode {
                id: "coding_node_0001".to_string(),
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::CodeReview,
                title: "代码审查".to_string(),
                status: CodingTimelineNodeStatus::Blocked,
                agent_role: Some(CodingAgentRole::Reviewer),
                summary: Some("code review 被阻塞".to_string()),
                started_at: "2026-07-13T00:00:00Z".to_string(),
                completed_at: Some("2026-07-13T00:01:00Z".to_string()),
                artifact_refs: Vec::new(),
            },
        )
        .expect("save blocked node");
    coding_store
        .save_timeline_node(
            &attempt,
            CodingTimelineNode {
                id: "coding_node_0002".to_string(),
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::CodeReview,
                title: "代码审查".to_string(),
                status: CodingTimelineNodeStatus::Completed,
                agent_role: Some(CodingAgentRole::Reviewer),
                summary: Some("code review 通过".to_string()),
                started_at: "2026-07-13T00:02:00Z".to_string(),
                completed_at: Some("2026-07-13T00:03:00Z".to_string()),
                artifact_refs: Vec::new(),
            },
        )
        .expect("save completed retry node");

    let state = build_coding_session_state(&coding_store, attempt).expect("coding session state");
    let CodingWsOutMessage::CodingSessionState { active_node_id, .. } = state else {
        panic!("expected coding session state");
    };

    assert!(active_node_id.is_none());
}

#[test]
fn blocked_attempt_allows_gate_response_messages() {
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::Blocked,
        &CodingExecutionStage::CodeReview,
        &CodingWsInMessage::GateResponse {
            gate_id: "coding_blocked_gate_0001".to_string(),
            action_id: "retry_review".to_string(),
            extra_context: None,
        },
    ));
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::Blocked,
        &CodingExecutionStage::CodeReview,
        &CodingWsInMessage::AbortAttempt,
    ));
}

#[test]
fn awaiting_manual_recovery_attempt_allows_only_abort_message() {
    // AbortAttempt 在任何 stage 下都应放行（该状态唯一可达的终态路径）。
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::AwaitingManualRecovery,
        &CodingExecutionStage::FinalConfirm,
        &CodingWsInMessage::AbortAttempt,
    ));
    // 其余消息一律拒绝，即使 stage 维度本会放行（FinalConfirm stage 放行 FinalConfirm/GateResponse）。
    assert!(!is_coding_ws_message_allowed(
        &CodingAttemptStatus::AwaitingManualRecovery,
        &CodingExecutionStage::FinalConfirm,
        &CodingWsInMessage::FinalConfirm,
    ));
    assert!(!is_coding_ws_message_allowed(
        &CodingAttemptStatus::AwaitingManualRecovery,
        &CodingExecutionStage::FinalConfirm,
        &CodingWsInMessage::GateResponse {
            gate_id: "coding_blocked_gate_0001".to_string(),
            action_id: "manual_continue".to_string(),
            extra_context: None,
        },
    ));
    assert!(!is_coding_ws_message_allowed(
        &CodingAttemptStatus::AwaitingManualRecovery,
        &CodingExecutionStage::CodeReview,
        &CodingWsInMessage::StartCoding,
    ));
    assert!(!is_coding_ws_message_allowed(
        &CodingAttemptStatus::AwaitingManualRecovery,
        &CodingExecutionStage::CodeReview,
        &CodingWsInMessage::ProviderSelect {
            role: "coder".to_string(),
            provider: ProviderName::Fake,
        },
    ));
    assert!(!is_coding_ws_message_allowed(
        &CodingAttemptStatus::AwaitingManualRecovery,
        &CodingExecutionStage::CodeReview,
        &CodingWsInMessage::ContextNote {
            content: "manual fix".to_string(),
        },
    ));
}

#[test]
fn manual_continue_gate_response_does_not_auto_resume_runner() {
    let mut attempt = CodingExecutionAttempt {
        id: "coding_attempt_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        work_item_id: "work_item_0001".to_string(),
        attempt_no: 1,
        scope: crate::product::coding_models::CodingAttemptScope::WorkItem,
        status: CodingAttemptStatus::Blocked,
        version: 0,
        manual_recovery_reason: None,
        admission_ticket_consumed_at: None,
        admission_kind: crate::product::coding_models::CodingAdmissionKind::LegacyGroup,
        stage: CodingExecutionStage::CodeReview,
        base_branch: "HEAD".to_string(),
        branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
        worktree_path: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::Fake,
            reviewer: Some(ProviderName::Fake),
            review_rounds: 1,
            permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
        },
        provider_conversations: Vec::new(),
        rework_count: 2,
        max_auto_rework: 2,
        work_item_group_id: None,
        current_work_item_id: Some("work_item_0001".to_string()),
        active_unit_id: None,
        head_commit: None,
        pushed_remote: None,
        review_request_id: None,
        created_at: "2026-06-12T00:00:00Z".to_string(),
        updated_at: "2026-06-12T00:00:00Z".to_string(),
        target_snapshot: None,
        completed_at: None,
    };

    assert!(!should_resume_runner_after_gate_response(
        "manual_continue",
        &attempt
    ));
    assert!(!should_resume_runner_after_gate_response(
        "accept_risk",
        &attempt
    ));
    assert!(!should_resume_runner_after_gate_response(
        "retry_test_plan",
        &attempt
    ));
    assert!(should_resume_runner_after_gate_response(
        "retry_internal_review",
        &attempt
    ));
    assert!(should_resume_runner_after_gate_response(
        "send_to_coder",
        &attempt
    ));
    assert!(!should_resume_runner_after_gate_response(
        "accept_testing_result",
        &attempt
    ));
    assert!(should_resume_runner_after_gate_response(
        "retry_coding",
        &attempt
    ));

    attempt.status = CodingAttemptStatus::Running;
    assert!(!should_resume_runner_after_gate_response(
        "retry_test_plan",
        &attempt
    ));
}

#[test]
fn recoverable_attempt_status_suppresses_coding_start_failed() {
    assert!(!should_emit_coding_runner_protocol_error(
        &CodingAttemptStatus::Blocked
    ));
    assert!(!should_emit_coding_runner_protocol_error(
        &CodingAttemptStatus::WaitingForHuman
    ));
    assert!(should_emit_coding_runner_protocol_error(
        &CodingAttemptStatus::Failed
    ));
}

#[test]
fn waiting_attempt_allows_gate_response_for_coder_feedback() {
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::WaitingForHuman,
        &CodingExecutionStage::CodeReview,
        &CodingWsInMessage::GateResponse {
            gate_id: "coding_blocked_gate_0001".to_string(),
            action_id: "send_to_coder".to_string(),
            extra_context: Some("人工修复意见".to_string()),
        },
    ));
}

fn seed_compiled_work_item_fixture() -> (TempDir, ProductAppPaths, CodingExecutionAttempt) {
    let tmp = TempDir::new().expect("temp dir");
    let app_paths = ProductAppPaths::new(tmp.path().join(".aria"));
    crate::product::project_store::ProjectStore::new(app_paths.clone())
        .create(crate::product::project_store::CreateProjectInput {
            name: "compiled work item fixture".to_string(),
            description: None,
        })
        .expect("create project");
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let work_item_id = "work_item_compile_20260702063721302_001";
    let verification_plan_id = "verification_plan_compile_20260702063721302_001";

    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some(work_item_id.to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            design_spec_ids: vec!["design_spec_0001".to_string()],
            title: "Final Compile title".to_string(),
            source_work_item_plan_id: Some("issue_work_item_plan_0001".to_string()),
            source_outline_id: Some("outline_backend".to_string()),
            source_draft_id: Some("draft_backend".to_string()),
            planned_implementation_context: Some(
                "planned implementation context for coder\n- touch src/web/coding_ws_handler/context.rs"
                    .to_string(),
            ),
            kind: WorkItemKind::Backend,
            sequence_hint: Some(1),
            depends_on: vec!["work_item_compile_dependency_001".to_string()],
            exclusive_write_scopes: vec!["src/web/coding_ws_handler/context.rs".to_string()],
            forbidden_write_scopes: vec!["forbidden/path".to_string()],
            verification_plan_ref: Some(verification_plan_id.to_string()),
            plan_status: WorkItemPlanStatus::Confirmed,
            ..Default::default()
        })
        .expect("create final compile work item");

    lifecycle
        .create_verification_plan(CreateVerificationPlanInput {
            id: Some(verification_plan_id.to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: work_item_id.to_string(),
            repository_profile_ref: None,
            provider_run_ref: None,
            scope: VerificationScope::Unit,
            commands: vec![VerificationCommand {
                id: "cmd_001".to_string(),
                label: "context unit test".to_string(),
                command: "cargo test --locked --lib coding_execution_context".to_string(),
                cwd: ".".to_string(),
                purpose: "verify coding context uses final compile work item".to_string(),
                required: true,
                timeout_seconds: 120,
                source: VerificationCommandSource::Provider,
                safety: VerificationCommandSafety::Approved,
            }],
            manual_checks: Vec::new(),
            required_gates: vec!["cargo fmt --check".to_string()],
            risk_notes: vec!["provider prompt must include final compile context".to_string()],
            confidence: RepositoryProfileConfidence::High,
            fallback_policy: VerificationFallbackPolicy::ManualGate,
        })
        .expect("create verification plan");

    let attempt = CodingExecutionAttempt {
        id: "coding_attempt_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        work_item_id: work_item_id.to_string(),
        attempt_no: 1,
        scope: CodingAttemptScope::WorkItemGroup,
        status: CodingAttemptStatus::Running,
        version: 0,
        manual_recovery_reason: None,
        admission_ticket_consumed_at: None,
        admission_kind: crate::product::coding_models::CodingAdmissionKind::LegacyGroup,
        stage: CodingExecutionStage::Coding,
        base_branch: "main".to_string(),
        branch_name: "aria/issues/issue_0001".to_string(),
        worktree_path: Some(tmp.path().join("coding-worktree")),
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::Fake,
            reviewer: Some(ProviderName::Fake),
            review_rounds: 1,
            permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
        },
        provider_conversations: Vec::new(),
        rework_count: 0,
        max_auto_rework: 2,
        work_item_group_id: None,
        current_work_item_id: Some(work_item_id.to_string()),
        active_unit_id: None,
        head_commit: None,
        pushed_remote: None,
        review_request_id: None,
        created_at: "2026-07-02T00:00:00Z".to_string(),
        updated_at: "2026-07-02T00:00:00Z".to_string(),
        target_snapshot: None,
        completed_at: None,
    };

    (tmp, app_paths, attempt)
}
