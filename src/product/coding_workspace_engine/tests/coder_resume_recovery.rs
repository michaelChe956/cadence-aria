use super::*;
use crate::product::coding_models::FindingSeverity;
use std::sync::Mutex;

#[derive(Default)]
struct ResumeStallThenFreshSuccessProvider {
    inputs: Mutex<Vec<StreamingProviderInput>>,
}

#[derive(Default)]
struct AlwaysResumeStallProvider {
    inputs: Mutex<Vec<StreamingProviderInput>>,
}

impl ResumeStallThenFreshSuccessProvider {
    fn recorded_inputs(&self) -> Vec<StreamingProviderInput> {
        self.inputs.lock().expect("inputs").clone()
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ResumeStallThenFreshSuccessProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let resumed = input.resume_provider_session_id.is_some();
        self.inputs.lock().expect("inputs").push(input);
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(4);
        tokio::spawn(async move {
            if resumed {
                let _ = event_tx
                    .send(ProviderEvent::Failed {
                        message:
                            "Codex resume stalled before provider progress for thread stale-thread"
                                .to_string(),
                    })
                    .await;
            } else {
                let _ = event_tx
                    .send(ProviderEvent::Completed(
                        crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                            "fresh coder completed".to_string(),
                            Some("fresh-thread".to_string()),
                        ),
                    ))
                    .await;
            }
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for AlwaysResumeStallProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.inputs.lock().expect("inputs").push(input);
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(4);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::Failed {
                    message: "Codex resume stalled before provider progress".to_string(),
                })
                .await;
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }
}

#[tokio::test]
async fn coder_resume_stall_maps_to_retryable_transport_without_same_role_run_restart() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let attempt = seed_stale_coder_conversation(&store, &attempt);
    let role_run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Coding,
            CodingProviderRole::Coder,
            CodingRoleRunTrigger::Initial,
            Some("coding_resume_invocation".to_string()),
        )
        .expect("role run");
    let (tx, _rx) = mpsc::channel(16);
    let provider = std::sync::Arc::new(ResumeStallThenFreshSuccessProvider::default());
    let engine = super::provider_gateway_validated_input::engine_with_fixture_gateway(
        &store,
        tx,
        "project_0001",
        provider.clone(),
    );
    let (_command_tx, mut command_rx) = mpsc::channel(1);
    let worktree = attempt.worktree_path.clone().expect("worktree path");
    let legacy_input = AdapterInput {
        env_vars: Default::default(),
        working_directory: None,
        provider_type: ProviderType::Codex,
        role: AdapterRole::Executor,
        worktree_path: Some(worktree.to_string_lossy().to_string()),
        provider_stream_log_dir: None,
        prompt: "resume coder".to_string(),
        context_files: Vec::new(),
        output_schema: "coding_workspace_markdown".to_string(),
        timeout: 30,
        max_retries: 0,
    };
    let mut input = streaming_input_from_adapter(
        &legacy_input,
        worktree,
        crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
    );
    input.resume_provider_session_id = Some("stale-thread".to_string());
    // 与生产 LC Coder launch 同源：有效 cwd 显式取 gateway authority root
    // （`.aria` 的父目录），spawn 前复验以该字段与 envelope 冻结 cwd 比对。
    input.working_directory = Some(
        store
            .paths()
            .root()
            .parent()
            .expect("workspace root")
            .to_path_buf(),
    );
    let provider_name = ProviderName::Codex;
    // REQ-ROUTE-01 两态化：有快照的 attempt 其 provider run 唯一经 gateway
    // validated 通道；本用例直调 stream 入口，需先取与生产同源的 validated
    // launch（gateway 未注入/旧路径 → None，行为不变）。
    let working_dir = input.working_dir.clone();
    let validated_input = engine
        .prepare_streaming_launch_for_role(
            &attempt,
            CodingProviderRole::Coder,
            &working_dir,
            input.clone(),
        )
        .expect("prepare validated launch");

    let outcome = engine
        .run_provider_stream_invocation(CodingProviderStreamRun {
            attempt: &attempt,
            node_id: "coding_resume_invocation",
            role_run: Some(&role_run),
            provider: provider.as_ref(),
            legacy_input: &legacy_input,
            input,
            provider_name: &provider_name,
            provider_role: CodingProviderRole::Coder,
            command_rx: &mut command_rx,
            allow_legacy_stream_fallback: true,
            timeout: None,
            timeout_reason_code: None,
            suppress_failure_side_effects: false,
            validated_input,
        })
        .await;

    assert!(matches!(
        outcome,
        ProviderInvocationOutcome::RetryableTransport {
            failure: RetryableProviderFailure::ConnectionInterrupted,
            ref reason_code,
            ref message,
            ref partial_output,
        } if reason_code == "provider_connection_interrupted"
            && message.contains("Codex resume stalled")
            && partial_output.is_empty()
    ));
    assert_eq!(provider.recorded_inputs().len(), 1);
    let persisted = store
        .get_role_run(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
        )
        .expect("persisted role run");
    assert_eq!(persisted.raw_provider_output_refs.len(), 1);
    assert!(
        store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("open gates")
            .is_empty()
    );
}

#[tokio::test]
async fn codex_fresh_session_recovery_consumes_one_of_three_attempts() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let attempt = seed_stale_coder_conversation(&store, &attempt);
    let worktree = attempt.worktree_path.clone().expect("worktree");
    let (tx, _rx) = mpsc::channel(16);
    let provider = std::sync::Arc::new(ResumeStallThenFreshSuccessProvider::default());
    let engine = super::provider_gateway_validated_input::engine_with_fixture_gateway(
        &store,
        tx,
        "project_0001",
        provider.clone(),
    );
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let completed = engine
        .execute_coding_with_commands(
            &attempt,
            provider.as_ref(),
            &coding_context(),
            &mut command_rx,
        )
        .await
        .expect("fresh Codex retry succeeds inside the bounded cycle");

    assert_eq!(completed.status, CodingAttemptStatus::Running);
    let inputs = provider.recorded_inputs();
    assert_eq!(inputs.len(), 2);
    assert_eq!(
        inputs[0].resume_provider_session_id.as_deref(),
        Some("stale-thread")
    );
    assert_eq!(inputs[1].resume_provider_session_id, None);
    assert_eq!(inputs[0].working_dir, worktree);
    assert_eq!(inputs[1].working_dir, inputs[0].working_dir);
    assert!(inputs[1].prompt.contains("Provider 恢复问题"));
    let runs = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("coder runs");
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Failed);
    assert_eq!(runs[1].status, CodingRoleRunStatus::Completed);
    assert_eq!(runs[1].trigger, CodingRoleRunTrigger::AutomaticRetry);
    assert_eq!(runs[0].retry_metadata.as_ref().unwrap().attempt_no, 1);
    assert_eq!(runs[1].retry_metadata.as_ref().unwrap().attempt_no, 2);
}

#[tokio::test]
async fn successful_retry_then_reviewer_finding_starts_rework_without_incrementing_retry_as_rework()
{
    let (_root, store, attempt) = running_attempt_with_worktree();
    let attempt = seed_stale_coder_conversation(&store, &attempt);
    let (tx, _rx) = mpsc::channel(32);
    let retrying_provider = std::sync::Arc::new(ResumeStallThenFreshSuccessProvider::default());
    let engine = super::provider_gateway_validated_input::engine_with_fixture_gateway(
        &store,
        tx,
        "project_0001",
        retrying_provider.clone(),
    );
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let coded = engine
        .execute_coding_with_commands(
            &attempt,
            retrying_provider.as_ref(),
            &coding_context(),
            &mut command_rx,
        )
        .await
        .expect("automatic transport retry succeeds");
    assert_eq!(coded.rework_count, 0);
    assert!(
        store
            .list_rework_instructions(&coded.project_id, &coded.issue_id, &coded.id)
            .expect("instructions after retry")
            .is_empty()
    );

    let code_review = store
        .update_attempt_stage(
            &coded.project_id,
            &coded.issue_id,
            &coded.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("code review stage");
    let rework_provider =
        std::sync::Arc::new(super::provider_driven::ReviewerDrivenReworkProvider::default());
    let (rework_tx, _rework_rx) = mpsc::channel(32);
    let engine = super::provider_gateway_validated_input::engine_with_fixture_gateway(
        &store,
        rework_tx,
        "project_0001",
        rework_provider.clone(),
    );
    let updated = engine
        .execute_coder_fix_from_review(
            &code_review,
            &review_report_requesting_changes(&code_review),
            &coding_context(),
            rework_provider.as_ref(),
            &mut command_rx,
        )
        .await
        .expect("reviewer finding starts one normal rework");

    assert_eq!(updated.rework_count, 1);
    let instructions = store
        .list_rework_instructions(&updated.project_id, &updated.issue_id, &updated.id)
        .expect("rework instructions");
    assert_eq!(instructions.len(), 1);
    assert_eq!(instructions[0].rework_round, 1);
    let runs = store
        .list_role_runs(&updated.project_id, &updated.issue_id, &updated.id)
        .expect("all coder runs");
    assert_eq!(runs.len(), 3);
    assert_eq!(runs[0].retry_metadata.as_ref().unwrap().attempt_no, 1);
    assert_eq!(runs[1].retry_metadata.as_ref().unwrap().attempt_no, 2);
    assert_eq!(runs[2].retry_metadata.as_ref().unwrap().attempt_no, 1);
    assert_ne!(
        runs[1].retry_metadata.as_ref().unwrap().cycle_id,
        runs[2].retry_metadata.as_ref().unwrap().cycle_id
    );
}

#[tokio::test]
async fn coder_rework_resume_stall_retries_in_a_new_role_run() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let attempt = seed_stale_coder_conversation(&store, &attempt);
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("code review stage");
    let (tx, _rx) = mpsc::channel(16);
    let provider = std::sync::Arc::new(ResumeStallThenFreshSuccessProvider::default());
    let engine = super::provider_gateway_validated_input::engine_with_fixture_gateway(
        &store,
        tx,
        "project_0001",
        provider.clone(),
    );
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let completed = engine
        .execute_coder_fix_from_review(
            &attempt,
            &review_report_requesting_changes(&attempt),
            &coding_context(),
            provider.as_ref(),
            &mut command_rx,
        )
        .await
        .expect("outer retry coordinator starts a fresh rework invocation");

    assert_eq!(completed.rework_count, 1);
    assert_eq!(completed.stage, CodingExecutionStage::CodeReview);
    let inputs = provider.recorded_inputs();
    assert_eq!(inputs.len(), 2);
    assert_eq!(
        inputs[0].resume_provider_session_id.as_deref(),
        Some("stale-thread")
    );
    assert_eq!(inputs[1].resume_provider_session_id, None);
    let runs = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("rework role runs");
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Failed);
    assert_eq!(runs[1].status, CodingRoleRunStatus::Completed);
}

#[tokio::test]
async fn fresh_coder_transport_failure_uses_bounded_retry_cycle() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let (tx, _rx) = mpsc::channel(16);
    let provider = std::sync::Arc::new(AlwaysResumeStallProvider::default());
    let engine = super::provider_gateway_validated_input::engine_with_fixture_gateway(
        &store,
        tx,
        "project_0001",
        provider.clone(),
    );
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let error = engine
        .execute_coding_with_commands(
            &attempt,
            provider.as_ref(),
            &coding_context(),
            &mut command_rx,
        )
        .await
        .expect_err("three fresh transport failures exhaust the retry cycle");

    assert!(error.to_string().contains("Codex resume stalled"));
    assert_eq!(provider.inputs.lock().expect("inputs").len(), 3);
    let runs = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("coder role runs");
    assert_eq!(runs.len(), 3);
    assert!(
        runs.iter()
            .all(|run| run.status == CodingRoleRunStatus::Failed)
    );
}

#[tokio::test]
async fn non_codex_transport_failure_uses_the_same_bounded_retry_cycle() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let mut provider_config = store
        .get_role_provider_config_snapshot(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role provider config");
    provider_config.coder = ProviderName::ClaudeCode;
    store
        .update_role_provider_config_snapshot(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            provider_config,
        )
        .expect("update coder provider");
    let attempt = store
        .replace_attempt_provider_conversations(
            &attempt,
            vec![ProviderConversationRef {
                role: ProviderConversationRole::Coder,
                provider: ProviderName::ClaudeCode,
                provider_session_id: "claude-thread".to_string(),
                updated_at: "2026-07-13T00:00:00Z".to_string(),
                last_node_id: Some("coding_node_0001".to_string()),
            }],
        )
        .expect("seed claude coder conversation");
    let (tx, _rx) = mpsc::channel(16);
    let provider = std::sync::Arc::new(AlwaysResumeStallProvider::default());
    let engine = super::provider_gateway_validated_input::engine_with_fixture_gateway(
        &store,
        tx,
        "project_0001",
        provider.clone(),
    );
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let error = engine
        .execute_coding_with_commands(
            &attempt,
            provider.as_ref(),
            &coding_context(),
            &mut command_rx,
        )
        .await
        .expect_err("transport retry policy is provider-independent");

    assert!(error.to_string().contains("Codex resume stalled"));
    assert_eq!(provider.inputs.lock().expect("inputs").len(), 3);
    let runs = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("coder role runs");
    assert_eq!(runs.len(), 3);
    assert_eq!(runs[1].trigger, CodingRoleRunTrigger::AutomaticRetry);
    assert_eq!(runs[2].trigger, CodingRoleRunTrigger::AutomaticRetry);
}

#[tokio::test]
async fn repeated_coder_failure_blocks_with_retry_gate_and_preserves_worktree() {
    let root = tempdir().expect("tempdir");
    let shared_worktree = root.path().join("shared-worktree");
    fs::create_dir_all(&shared_worktree).expect("shared worktree dir");
    init_test_git_repo(&shared_worktree);
    fs::write(shared_worktree.join("dirty.txt"), "uncommitted\n").expect("dirty worktree");

    let paths = ProductAppPaths::new(root.path().join(".aria"));
    // REQ-ROUTE-01 两态化：快照 attempt 的租约只按仓维分类；issue 维记录会被
    // 判 `legacy_shared_worktree_present` fail-closed。LC 权威（issue + 成员 +
    // 主 checkout + selection + policy）与冻结快照同源。
    let logical = crate::product::logical_codebase::logical_test_fixture::seed_complete(
        &paths,
        root.path(),
        "project_0001",
        "issue_0001",
    );
    let lifecycle = LifecycleStore::new(paths.clone());
    crate::product::logical_codebase::logical_test_fixture::upsert_repo_worktree(
        &lifecycle,
        "project_0001",
        "issue_0001",
        logical.member_id,
        &shared_worktree,
    );

    let store = CodingAttemptStore::new(paths);
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(shared_worktree.clone()),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: Some(logical.snapshot(&store.paths())),
            max_auto_rework: 2,
            start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
        })
        .expect("group attempt");
    crate::product::logical_codebase::logical_test_fixture::bind_repo_lock_to_attempt(
        &lifecycle,
        "project_0001",
        "issue_0001",
        logical.member_id,
        "work_item_0001",
        &attempt.id,
    );
    seed_group_attempt_fixture(&store, &attempt, true, false);
    // 单仓语义在 LC 模型下 = 全部 work item 归属唯一成员仓：
    // `validate_group_single_target` 对 Logical 路由要求每个权威 unit 带
    // `target_repository_id` 且与 attempt 快照一致；共享夹具写入的 draft 仍
    // 无归属，这里按同一不变量补齐（不改变任何断言事实）。
    let plan_store = WorkItemPlanStore::new(store.paths());
    for mut draft in plan_store
        .list_draft_records("project_0001", "issue_0001", "work_item_plan_0001")
        .expect("fixture draft records")
    {
        if draft.candidate.target_repository_id.is_none() {
            draft.candidate.target_repository_id = Some(logical.member_id);
            plan_store
                .put_draft_record(&draft)
                .expect("fixture draft target");
        }
    }
    let attempt = store
        .seed_running_attempt_for_test(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("running attempt");
    let prior_run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Coding,
            CodingProviderRole::Coder,
            CodingRoleRunTrigger::Initial,
            None,
        )
        .expect("prior coder role run");
    store
        .update_role_run_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &prior_run.id,
            CodingRoleRunStatus::Completed,
            None,
        )
        .expect("complete prior coder role run");
    let attempt = seed_stale_coder_conversation(&store, &attempt);
    let (tx, _rx) = mpsc::channel(16);
    let provider = std::sync::Arc::new(AlwaysResumeStallProvider::default());
    let engine = super::provider_gateway_validated_input::engine_with_fixture_gateway(
        &store,
        tx,
        "project_0001",
        provider.clone(),
    );
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let error = engine
        .execute_coding_with_commands(
            &attempt,
            provider.as_ref(),
            &coding_context(),
            &mut command_rx,
        )
        .await
        .expect_err("repeated provider failure remains surfaced");

    assert!(
        matches!(
            error,
            CodingWorkspaceEngineError::ProviderStream(ref message)
                if message.contains("Codex resume stalled")
        ),
        "unexpected error: {error:?}"
    );
    assert_eq!(provider.inputs.lock().expect("inputs").len(), 3);
    let persisted = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("persisted attempt");
    assert_eq!(persisted.status, CodingAttemptStatus::Blocked);
    assert_eq!(persisted.stage, CodingExecutionStage::Coding);
    let role_run = store
        .latest_role_run(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::Coding,
            CodingProviderRole::Coder,
        )
        .expect("latest coder role run")
        .expect("coder role run");
    assert_eq!(role_run.status, CodingRoleRunStatus::Failed);
    assert_eq!(
        role_run.reason_code.as_deref(),
        Some("provider_connection_interrupted")
    );
    let cycle_runs = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("all coder runs")
        .into_iter()
        .filter(|run| run.retry_metadata.is_some())
        .collect::<Vec<_>>();
    assert_eq!(cycle_runs.len(), 4);
    let latest_cycle_id = cycle_runs[1]
        .retry_metadata
        .as_ref()
        .expect("cycle metadata")
        .cycle_id
        .clone();
    let automatic_cycle_runs = cycle_runs
        .iter()
        .filter(|run| {
            run.retry_metadata
                .as_ref()
                .is_some_and(|retry| retry.cycle_id == latest_cycle_id)
        })
        .collect::<Vec<_>>();
    assert_eq!(automatic_cycle_runs.len(), 3);
    assert_eq!(
        automatic_cycle_runs
            .iter()
            .map(|run| run.retry_metadata.as_ref().unwrap().attempt_no)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    let open_gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("open gates");
    let gate = open_gates
        .iter()
        .find(|gate| gate.reason_code.as_deref() == Some("coder_provider_interrupted"))
        .expect("coder recovery gate");
    assert_eq!(
        gate.available_actions
            .iter()
            .map(|action| action.action_id.as_str())
            .collect::<Vec<_>>(),
        vec!["retry_coding", "abort"]
    );
    assert!(
        open_gates.iter().all(|gate| {
            gate.reason_code.as_deref() != Some("shared_worktree_dirty_manual_gate")
        })
    );
    assert!(!git_stdout(&shared_worktree, &["status", "--porcelain"]).is_empty());
}

fn seed_stale_coder_conversation(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> CodingExecutionAttempt {
    store
        .replace_attempt_provider_conversations(
            attempt,
            vec![ProviderConversationRef {
                role: ProviderConversationRole::Coder,
                provider: ProviderName::Codex,
                provider_session_id: "stale-thread".to_string(),
                updated_at: "2026-07-13T00:00:00Z".to_string(),
                last_node_id: Some("coding_node_0001".to_string()),
            }],
        )
        .expect("seed stale coder conversation")
}

fn coding_context() -> CodingExecutionContext {
    CodingExecutionContext {
        work_item_markdown: Some("# Draft 4\n\n- 修复 Provider 恢复问题".to_string()),
        verification_commands: vec!["cargo test --locked --lib coder_resume_recovery".to_string()],
    }
}

fn review_report_requesting_changes(attempt: &CodingExecutionAttempt) -> CodeReviewReport {
    CodeReviewReport {
        id: "code_review_report_0001".to_string(),
        attempt_id: attempt.id.clone(),
        round: 1,
        verdict: ReviewVerdict::RequestChanges,
        findings: vec![ReviewFinding {
            severity: FindingSeverity::Error,
            file_path: Some("src/lib.rs".to_string()),
            line: Some(42),
            message: "missing validation".to_string(),
            required_action: Some("add validation".to_string()),
            source_stage: CodingExecutionStage::CodeReview,
            evidence: vec!["review-output.log".to_string()],
            plan_defect_evidence: Vec::new(),
            related_requirements: Vec::new(),
            related_design_constraints: Vec::new(),
            related_work_item_tasks: Vec::new(),
            defect_class: crate::product::models::PlanDefectClass::ImplementationDefect,
            reason_code: None,
            contract_refs: Vec::new(),
            capability_refs: Vec::new(),
            repair_target: None,
            recommended_route: crate::product::models::PlanDefectRoute::CoderRework,
            confidence: None,
        }],
        tested_evidence_refs: Vec::new(),
        diff_refs: Vec::new(),
        summary: "reviewer requested changes".to_string(),
        created_at: "2026-07-13T00:00:00Z".to_string(),
        raw_provider_output_ref: None,
        role_run_id: None,
        run_no: None,
        unit_run_id: None,
        acceptance: None,
        review_window_id: None,
    }
}
