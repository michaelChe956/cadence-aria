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
fn recover_coding_message_allowed_only_in_awaiting_manual_recovery() {
    // F-16：人工恢复态的显式恢复通道——RecoverCoding 是 AbortAttempt 之外唯一
    // 放行的入站消息（重走 admission CAS 回到 Running + 重启 runner）。
    for stage in [
        CodingExecutionStage::Coding,
        CodingExecutionStage::WorktreePrepare,
        CodingExecutionStage::FinalConfirm,
    ] {
        assert!(
            is_coding_ws_message_allowed(
                &CodingAttemptStatus::AwaitingManualRecovery,
                &stage,
                &CodingWsInMessage::RecoverCoding,
            ),
            "AwaitingManualRecovery 必须放行显式恢复动作（stage={stage:?}）"
        );
    }
    // 其余状态一律拒绝：恢复通道是人工恢复态专属，不得成为绕过状态机的旁路。
    assert!(!is_coding_ws_message_allowed(
        &CodingAttemptStatus::Running,
        &CodingExecutionStage::Coding,
        &CodingWsInMessage::RecoverCoding,
    ));
    assert!(!is_coding_ws_message_allowed(
        &CodingAttemptStatus::Blocked,
        &CodingExecutionStage::CodeReview,
        &CodingWsInMessage::RecoverCoding,
    ));
    assert!(!is_coding_ws_message_allowed(
        &CodingAttemptStatus::WaitingForHuman,
        &CodingExecutionStage::FinalConfirm,
        &CodingWsInMessage::RecoverCoding,
    ));
    assert!(!is_coding_ws_message_allowed(
        &CodingAttemptStatus::Created,
        &CodingExecutionStage::PrepareContext,
        &CodingWsInMessage::RecoverCoding,
    ));
    assert!(!is_coding_ws_message_allowed(
        &CodingAttemptStatus::Aborted,
        &CodingExecutionStage::Coding,
        &CodingWsInMessage::RecoverCoding,
    ));
}

#[test]
fn manual_continue_resumes_runner_from_original_stage() {
    // C2 Task 4（REQ-CRO-04，A08）：Code Review 门 manual_continue／accept_risk
    // 后必须续跑（runner 从原阶段继续推进，不依赖提交动作的连接保持打开）；
    // 状态仍限 Blocked｜WaitingForHuman；retry 类动作既有续跑语义零变化。
    let mut attempt = CodingExecutionAttempt {
        id: "coding_attempt_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        work_item_id: "work_item_0001".to_string(),
        attempt_no: 1,
        scope: crate::product::coding_models::CodingAttemptScope::WorkItem,
        status: CodingAttemptStatus::Blocked,
        version: 3,
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
        start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
        start_claim: None,
    };

    // manual_continue／accept_risk：Blocked 与 WaitingForHuman 均续跑。
    assert!(should_resume_runner_after_gate_response(
        "manual_continue",
        &attempt
    ));
    assert!(should_resume_runner_after_gate_response(
        "accept_risk",
        &attempt
    ));
    attempt.status = CodingAttemptStatus::WaitingForHuman;
    assert!(should_resume_runner_after_gate_response(
        "manual_continue",
        &attempt
    ));

    // 既有白名单零变化：retry 类照常续跑。
    assert!(should_resume_runner_after_gate_response(
        "retry_internal_review",
        &attempt
    ));
    assert!(should_resume_runner_after_gate_response(
        "send_to_coder",
        &attempt
    ));
    assert!(should_resume_runner_after_gate_response(
        "retry_coding",
        &attempt
    ));
    // 非续跑动作照旧不续跑。
    assert!(!should_resume_runner_after_gate_response(
        "retry_test_plan",
        &attempt
    ));
    assert!(!should_resume_runner_after_gate_response(
        "accept_testing_result",
        &attempt
    ));

    // Running（已续跑）不得重复续跑。
    attempt.status = CodingAttemptStatus::Running;
    assert!(!should_resume_runner_after_gate_response(
        "manual_continue",
        &attempt
    ));
    assert!(!should_resume_runner_after_gate_response(
        "retry_test_plan",
        &attempt
    ));
}

#[test]
fn manual_continue_resolved_gate_skips_completed_reviewer() {
    // C2 Task 4（REQ-CRO-04）：manual_continue／accept_risk 已解决的分诊门
    // 引用最新报告时，续跑必须复用已持久化结论（不重跑 Code Reviewer）；
    // retry_review 解决的门不构成跳过依据（新评审轮次是预期行为）。
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::coding_attempt_store::{
        CodingAttemptStore, CreateBlockedGateInput, CreateCodingAttemptInput,
    };
    use crate::product::coding_models::{CodeReviewReport, ReviewVerdict};
    use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;

    let root = tempfile::tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = CodingAttemptStore::new(paths.clone());
    IssueStore::new(paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: Some("repository_0001".to_string()),
            logical_codebase_id: None,
            title: "gate issue".to_string(),
            description: None,
            change_id: None,
            base_branch: None,
        })
        .expect("issue");
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/skip-review".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: crate::product::models::ProviderName::Fake,
                reviewer: None,
                review_rounds: 0,
                permission_modes: Default::default(),
            },
            target_snapshot: None,
            max_auto_rework: 0,
        })
        .expect("attempt");

    let report = CodeReviewReport {
        id: "code_review_0001".to_string(),
        attempt_id: attempt.id.clone(),
        round: 1,
        verdict: ReviewVerdict::RequestChanges,
        findings: Vec::new(),
        tested_evidence_refs: Vec::new(),
        diff_refs: Vec::new(),
        summary: "需要人工分诊".to_string(),
        created_at: "2026-09-29T00:00:00Z".to_string(),
        raw_provider_output_ref: None,
        role_run_id: None,
        run_no: None,
        unit_run_id: None,
    };
    store
        .save_code_review_report(&attempt, &report)
        .expect("report");

    // 无已解决门：不跳过（默认新评审轮次）。
    let mut current = attempt.clone();
    current.stage = CodingExecutionStage::CodeReview;
    assert!(!super::runner::reviewer_conclusion_already_confirmed(&store, &current)
        .expect("check"));

    // retry_review 解决的门：不跳过（重跑评审是预期）——先落 retry 解决，
    // 再落 manual_continue 解决，分别断言。
    let gate_retry = store
        .create_blocked_gate(
            &attempt,
            CreateBlockedGateInput {
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::CodeReview,
                node_id: None,
                role: Some(crate::product::coding_models::CodingProviderRole::CodeReviewer),
                title: "Code Review 验证证据不完整".to_string(),
                description: report.summary.clone(),
                reason_code: Some("code_review_verification_incomplete".to_string()),
                evidence_refs: vec![report.id.clone()],
                raw_provider_output_ref: None,
                available_actions: Vec::new(),
            },
        )
        .expect("gate_retry");
    store
        .resolve_blocked_gate_with_action(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &gate_retry.gate_id,
            Some("retry_review"),
        )
        .expect("resolve retry");
    assert!(
        !super::runner::reviewer_conclusion_already_confirmed(&store, &current).expect("check"),
        "retry_review resolution must not skip the reviewer"
    );

    // manual_continue 解决、引用最新报告：跳过（复用持久化结论）。
    let gate = store
        .create_blocked_gate(
            &attempt,
            CreateBlockedGateInput {
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::CodeReview,
                node_id: None,
                role: Some(crate::product::coding_models::CodingProviderRole::CodeReviewer),
                title: "Code Review 结论需人工分诊".to_string(),
                description: report.summary.clone(),
                reason_code: Some("code_review_output_human_triage".to_string()),
                evidence_refs: vec![report.id.clone()],
                raw_provider_output_ref: None,
                available_actions: Vec::new(),
            },
        )
        .expect("gate");
    store
        .resolve_blocked_gate_with_action(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &gate.gate_id,
            Some("manual_continue"),
        )
        .expect("resolve with action");
    assert!(super::runner::reviewer_conclusion_already_confirmed(&store, &current)
        .expect("check"));
}

