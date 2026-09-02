use crate::product::coding_models::{RemoteKind, ReviewRequest, ReviewRequestKind, ReviewRequestOwnerKind};

fn manual_recovery_attempt_fixture() -> CodingExecutionAttempt {
    CodingExecutionAttempt {
        id: "coding_attempt_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        work_item_id: "work_item_0001".to_string(),
        attempt_no: 1,
        scope: CodingAttemptScope::WorkItem,
        status: CodingAttemptStatus::AwaitingManualRecovery,
        version: 0,
        manual_recovery_reason: Some("code_review_blocked".to_string()),
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
            permission_modes: WorkspaceRolePermissionModes::default(),
        },
        provider_conversations: Vec::new(),
        rework_count: 0,
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
    }
}

fn store_fixture() -> (tempfile::TempDir, CodingAttemptStore) {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CodingAttemptStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    (tmp, store)
}

fn failed_review_request_fixture(attempt: &CodingExecutionAttempt) -> ReviewRequest {
    ReviewRequest {
        id: "review_request_0001".to_string(),
        attempt_id: attempt.id.clone(),
        kind: ReviewRequestKind::GitBranchOnly,
        remote_kind: RemoteKind::GenericGit,
        remote: "origin".to_string(),
        base_branch: "main".to_string(),
        branch_name: attempt.branch_name.clone(),
        commit_sha: "sha111".to_string(),
        push_status: PushStatus::Failed,
        external_url: None,
        manual_instructions: Vec::new(),
        push_error: Some("push rejected".to_string()),
        owner_kind: ReviewRequestOwnerKind::Attempt,
        pointer_publication_id: None,
        revoked: false,
        created_at: "2026-06-12T00:00:00Z".to_string(),
        updated_at: "2026-06-12T00:00:00Z".to_string(),
    }
}

#[test]
fn coding_attempt_dto_exposes_manual_recovery_reason() {
    let (_tmp, store) = store_fixture();
    let dto = coding_attempt_dto(&store, &manual_recovery_attempt_fixture()).unwrap();
    assert_eq!(dto.status, "awaiting_manual_recovery");
    assert_eq!(
        dto.manual_recovery_reason,
        Some("code_review_blocked".to_string())
    );
}

#[test]
fn coding_attempt_dto_manual_recovery_reason_is_none_when_absent() {
    let (_tmp, store) = store_fixture();
    let mut attempt = manual_recovery_attempt_fixture();
    attempt.manual_recovery_reason = None;
    let dto = coding_attempt_dto(&store, &attempt).unwrap();
    assert_eq!(dto.status, "awaiting_manual_recovery");
    assert_eq!(dto.manual_recovery_reason, None);
}

#[test]
fn coding_attempt_dto_projects_failed_push_status_from_review_request() {
    let (_tmp, store) = store_fixture();
    let mut attempt = manual_recovery_attempt_fixture();
    attempt.status = CodingAttemptStatus::Completed;
    attempt.head_commit = Some("sha111".to_string());
    store.write_coding_attempt_for_test(&attempt).unwrap();
    store
        .save_review_request(&attempt, &failed_review_request_fixture(&attempt))
        .unwrap();

    let dto = coding_attempt_dto(&store, &attempt).unwrap();

    assert_eq!(dto.push_status.as_deref(), Some("failed"));
}

#[test]
fn coding_attempt_dto_falls_back_to_pushed_remote_without_review_request() {
    let (_tmp, store) = store_fixture();
    let mut attempt = manual_recovery_attempt_fixture();
    attempt.pushed_remote = Some("origin".to_string());
    store.write_coding_attempt_for_test(&attempt).unwrap();

    let dto = coding_attempt_dto(&store, &attempt).unwrap();

    assert_eq!(dto.push_status.as_deref(), Some("pushed"));
}

// —— 阶段 3 Task 8.3b —— group progress/result 投影 campaign 用例 ——
//
// fixture 覆盖 Pending/Running/Completed/Blocked 四态:Running/Pending/
// Completed 为第一份 durable 快照;随后经真实引擎路径
// (`handle_group_unit_failure` NonRetryable)把 active unit 推到 Blocked,
// 读取第二份快照。HTTP(与 GET coding-attempts/{id} handler 同源的
// assembler)与 coding WS(`build_coding_session_state`)两次都按 logical WI
// 读取 status/stage/current+final commit/review/handoff/reason/plan binding
// 与 group aggregate;篡改 SC per-WI child session 后输出不变——来源链只含
// group attempt/unit/run/handoff 事实,不读 workspace session。
#[tokio::test]
async fn campaign_stage3_group_projection_reads_every_work_item_result() {
    use crate::product::coding_models::{
        CodeReviewReport, CodingAttemptStatus, CodingExecutionStage, CodingExecutionUnitStatus,
        CodingUnitRun, CodingUnitRunStatus, ReviewVerdict,
    };
    use crate::product::coding_workspace_engine::readiness_fixture;
    use crate::product::lifecycle_store::workspace_session_read_spy::{
        reset_workspace_session_read_spy, set_workspace_session_read_panic,
        workspace_session_read_count,
    };
    use crate::product::models::{HandoffRevision, WorkspaceSessionStatus, WorkspaceType};
    use crate::product::work_item_revision_store::WorkItemRevisionStore;
    use crate::web::types::WorkItemCodingProgressDto;

    fn by_logical<'a>(
        progress: &'a [WorkItemCodingProgressDto],
        logical: &str,
    ) -> &'a WorkItemCodingProgressDto {
        progress
            .iter()
            .find(|item| item.logical_work_item_id == logical)
            .unwrap_or_else(|| panic!("progress for {logical}"))
    }

    let fixture = readiness_fixture();
    let store = &fixture.store;
    let mut attempt = store
        .get_attempt(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("durable attempt");
    let units = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("units");
    assert_eq!(units.len(), 3);
    let unit_by_logical = |logical: &str| {
        units
            .iter()
            .find(|unit| unit.logical_work_item_id == logical)
            .unwrap_or_else(|| panic!("unit {logical}"))
            .clone()
    };
    let running = unit_by_logical("work_item_0001");
    let pending = unit_by_logical("work_item_0002");
    let completing = unit_by_logical("work_item_0003");

    // Completed 态:completed run + 匹配 handoff + code review 报告。
    let revision_store = WorkItemRevisionStore::new(store.paths());
    let lineage = revision_store
        .get_plan_lineage(&attempt.project_id, &attempt.issue_id, "work_item_plan_0001")
        .expect("lineage");
    let revision = revision_store
        .get_work_item_revision(
            &lineage,
            &completing.logical_work_item_id,
            &completing.work_item_revision_id,
        )
        .expect("revision");
    let bundle = revision_store
        .get_work_item_projection_bundle(&lineage, &revision.work_item_projection_bundle_id)
        .expect("bundle");
    let completed_run = CodingUnitRun {
        id: "coding_unit_run_completed".to_string(),
        unit_id: completing.id.clone(),
        execution_no: 1,
        work_item_revision_id: revision.id.clone(),
        resolved_handoff_revision_ids: Vec::new(),
        canonical_contract_hash: bundle.canonical_contract_hash.clone(),
        projection_bundle_id: bundle.id.clone(),
        projection_compiler_version: bundle.compiler_version.clone(),
        coder_provider_renderer_version: "test-renderer-v1".to_string(),
        reviewer_provider_renderer_version: "test-renderer-v1".to_string(),
        internal_reviewer_provider_renderer_version: None,
        coder_projection_hash: bundle.coder_projection_hash.clone(),
        reviewer_projection_hash: bundle.reviewer_projection_hash.clone(),
        coder_execution_context_hash: None,
        reviewer_execution_context_hash: None,
        internal_reviewer_execution_context_hash: None,
        status: CodingUnitRunStatus::Completed,
        unit_rework_count: 0,
        verification_retry_count: 0,
        operational_retry_count: 0,
        plan_repair_count: 0,
        start_commit: Some(fixture.start_commit.clone()),
        completion_commit: Some("commit_completed_0003".to_string()),
        created_at: "2026-08-31T00:00:00Z".to_string(),
        updated_at: "2026-08-31T00:00:00Z".to_string(),
    };
    store
        .create_coding_unit_run(&attempt, &completed_run)
        .expect("completed run");
    let handoff = HandoffRevision {
        id: "handoff_campaign_0003".to_string(),
        logical_work_item_id: completing.logical_work_item_id.clone(),
        work_item_revision_id: revision.id.clone(),
        coding_unit_run_id: completed_run.id.clone(),
        provided_contracts: Vec::new(),
        provided_capabilities: Default::default(),
        contract_hash: "contract_hash_campaign".to_string(),
        commit_sha: "commit_completed_0003".to_string(),
        created_at: "2026-08-31T00:00:00Z".to_string(),
    };
    revision_store
        .put_handoff_revision(&lineage, &handoff)
        .expect("handoff");
    store
        .update_coding_unit_latest_handoff_revision_id(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &completing.id,
            Some(handoff.id.clone()),
        )
        .expect("handoff pointer");
    store
        .update_coding_unit_completion_commit(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &completing.id,
            Some("commit_completed_0003".to_string()),
        )
        .expect("completion commit");
    store
        .update_coding_unit_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &completing.id,
            CodingExecutionUnitStatus::Completed,
            Some("completed campaign unit".to_string()),
        )
        .expect("completed unit");
    store
        .save_code_review_report(
            &attempt,
            &CodeReviewReport {
                id: "review_campaign_0003".to_string(),
                attempt_id: attempt.id.clone(),
                round: 1,
                verdict: ReviewVerdict::Approve,
                findings: Vec::new(),
                tested_evidence_refs: Vec::new(),
                diff_refs: Vec::new(),
                summary: "campaign review approved".to_string(),
                created_at: "2026-08-31T00:00:01Z".to_string(),
                raw_provider_output_ref: None,
                role_run_id: None,
                run_no: None,
                unit_run_id: Some(completed_run.id.clone()),
            },
        )
        .expect("code review report");

    // Running 态:active unit 的 running run + attempt 进入 Coding/Running。
    let running_revision = revision_store
        .get_work_item_revision(
            &lineage,
            &running.logical_work_item_id,
            &running.work_item_revision_id,
        )
        .expect("running revision");
    let running_bundle = revision_store
        .get_work_item_projection_bundle(
            &lineage,
            &running_revision.work_item_projection_bundle_id,
        )
        .expect("running bundle");
    let running_run = CodingUnitRun {
        id: "coding_unit_run_running".to_string(),
        unit_id: running.id.clone(),
        execution_no: 1,
        work_item_revision_id: running_revision.id.clone(),
        resolved_handoff_revision_ids: Vec::new(),
        canonical_contract_hash: running_bundle.canonical_contract_hash.clone(),
        projection_bundle_id: running_bundle.id.clone(),
        projection_compiler_version: running_bundle.compiler_version.clone(),
        coder_provider_renderer_version: "test-renderer-v1".to_string(),
        reviewer_provider_renderer_version: "test-renderer-v1".to_string(),
        internal_reviewer_provider_renderer_version: None,
        coder_projection_hash: running_bundle.coder_projection_hash.clone(),
        reviewer_projection_hash: running_bundle.reviewer_projection_hash.clone(),
        coder_execution_context_hash: None,
        reviewer_execution_context_hash: None,
        internal_reviewer_execution_context_hash: None,
        status: CodingUnitRunStatus::Running,
        unit_rework_count: 0,
        verification_retry_count: 0,
        operational_retry_count: 0,
        plan_repair_count: 0,
        start_commit: Some(fixture.start_commit.clone()),
        completion_commit: None,
        created_at: "2026-08-31T00:00:00Z".to_string(),
        updated_at: "2026-08-31T00:00:00Z".to_string(),
    };
    store
        .create_coding_unit_run(&attempt, &running_run)
        .expect("running run");
    attempt.stage = CodingExecutionStage::Coding;
    attempt.status = CodingAttemptStatus::Running;
    attempt.head_commit = Some("commit_running_head".to_string());
    store
        .write_coding_attempt_for_test(&attempt)
        .expect("running attempt");

    let binding_revision = store
        .get_plan_binding(&attempt)
        .expect("plan binding")
        .bound_plan_revision_id;

    // 读取 helper:HTTP(assembler)+ WS(真实 session-state 构建器)。
    let read_projection = |attempt: &crate::product::coding_models::CodingExecutionAttempt| {
        let http = crate::web::handlers::build_group_work_item_progress(store, attempt)
            .expect("HTTP group progress");
        let ws = crate::web::coding_ws_handler::build_coding_session_state(store, attempt.clone())
            .expect("coding WS session state");
        (http, ws)
    };

    // —— 第一份快照:Running + Pending + Completed ——
    let (http, ws) = read_projection(&attempt);
    let http_progress = &http.0;
    let http_aggregate = &http.1;

    let item_pending = by_logical(http_progress, &pending.logical_work_item_id);
    assert_eq!(item_pending.status, "pending");
    assert_eq!(item_pending.stage, None);
    assert_eq!(item_pending.current_commit, None);
    assert_eq!(item_pending.final_commit, None);
    assert_eq!(item_pending.handoff_revision_id, None);
    assert_eq!(item_pending.code_review, None);
    assert_eq!(item_pending.failure_or_blocked_reason, None);
    assert_eq!(item_pending.plan_revision_id, binding_revision);

    let item_running = by_logical(http_progress, &running.logical_work_item_id);
    assert_eq!(item_running.status, "running");
    assert_eq!(
        item_running.stage.as_deref(),
        Some(coding_execution_stage_text(&attempt.stage))
    );
    assert_eq!(item_running.current_commit.as_deref(), Some("commit_running_head"));
    assert_eq!(item_running.final_commit, None);

    let item_completed = by_logical(http_progress, &completing.logical_work_item_id);
    assert_eq!(item_completed.status, "completed");
    assert_eq!(
        item_completed.final_commit.as_deref(),
        Some("commit_completed_0003")
    );
    assert_eq!(
        item_completed.current_commit.as_deref(),
        Some("commit_completed_0003")
    );
    assert_eq!(
        item_completed.handoff_revision_id.as_deref(),
        Some("handoff_campaign_0003")
    );
    let review = item_completed.code_review.as_ref().expect("review projected");
    assert_eq!(review.id, "review_campaign_0003");
    assert_eq!(item_completed.plan_revision_id, binding_revision);

    assert_eq!(http_aggregate.total, 3);
    assert_eq!(http_aggregate.pending, 1);
    assert_eq!(http_aggregate.active, 1);
    assert_eq!(http_aggregate.completed, 1);
    assert_eq!(http_aggregate.failed_or_blocked, 0);

    // —— 篡改 SC per-WI child session:投影输出必须逐字节不变,且来源链
    // 完全不读 workspace session(读则 panic 的 spy 直接证明)。——
    let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(store.paths());
    let child = lifecycle
        .create_workspace_session(
            crate::product::lifecycle_store::CreateWorkspaceSessionInput {
                project_id: attempt.project_id.clone(),
                issue_id: attempt.issue_id.clone(),
                entity_id: pending.logical_work_item_id.clone(),
                workspace_type: WorkspaceType::WorkItem,
                author_provider: crate::product::models::ProviderName::Fake,
                reviewer_provider: crate::product::models::ProviderName::Fake,
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            },
        )
        .expect("SC child session");
    lifecycle
        .update_workspace_session_status(&child.id, WorkspaceSessionStatus::Failed)
        .expect("contradictory child status");
    lifecycle
        .append_workspace_message(
            &child.id,
            "assistant".to_string(),
            "stage=completed commit=SC_CHILD_FAKE_COMMIT".to_string(),
        )
        .expect("contaminating child payload");

    reset_workspace_session_read_spy();
    set_workspace_session_read_panic(true);
    let (tampered, _) = read_projection(&attempt);
    set_workspace_session_read_panic(false);
    assert_eq!(workspace_session_read_count(), 0, "来源链不读 workspace session");
    assert_eq!(tampered.0, http.0, "篡改后 per-WI 输出逐字节不变");
    assert_eq!(tampered.1, http.1, "篡改后 group aggregate 不变");
    for item in &tampered.0 {
        assert_ne!(
            item.current_commit.as_deref(),
            Some("SC_CHILD_FAKE_COMMIT"),
            "child session 的伪造 commit 不得泄入投影"
        );
    }

    // WS 面(HTTP/WS 按 logical WI 一致)。
    let crate::web::coding_ws_handler::CodingWsOutMessage::CodingSessionState {
        group_coding_progress,
        group_progress,
        units: ws_units,
        ..
    } = ws
    else {
        panic!("expected coding session state");
    };
    let ws_progress = group_coding_progress.expect("WS per-WI progress");
    assert_eq!(
        ws_progress
            .iter()
            .map(|item| item.logical_work_item_id.as_str())
            .collect::<Vec<_>>(),
        ["work_item_0001", "work_item_0002", "work_item_0003"],
        "WS units 覆盖全部 logical WI"
    );
    assert_eq!(ws_units.len(), ws_progress.len());
    for (ws_item, http_item) in ws_progress.iter().zip(http_progress.iter()) {
        assert_eq!(ws_item, http_item, "HTTP 与 WS 按 logical WI 输出一致");
    }
    assert_eq!(group_progress.expect("WS aggregate"), *http_aggregate);

    // —— Blocked 态:真实引擎路径(NonRetryable failure classification)——
    let outcome = fixture
        .engine
        .handle_group_unit_failure(
            &attempt,
            &running.id,
            crate::product::coding_workspace_engine::ProviderFailureClassification::NonRetryable {
                reason_code: "campaign_non_retryable".to_string(),
                interaction_wait: false,
            },
        )
        .await
        .expect("real blocked transition");
    assert!(
        matches!(
            outcome,
            crate::product::coding_workspace_engine::GroupUnitFailureOutcome::AwaitingManualRecovery { ref reason_code, .. }
                if reason_code == "campaign_non_retryable"
        ),
        "NonRetryable 必须经真实路径把 unit 推到 Blocked"
    );
    let blocked_attempt = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("blocked attempt");

    let (blocked_http, blocked_ws) = read_projection(&blocked_attempt);
    let blocked_progress = &blocked_http.0;
    let item_blocked = by_logical(blocked_progress, &running.logical_work_item_id);
    assert_eq!(item_blocked.status, "blocked");
    assert!(
        item_blocked
            .failure_or_blocked_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("campaign_non_retryable")),
        "blocked reason 落到 durable 事实: {:?}",
        item_blocked.failure_or_blocked_reason
    );
    // Blocked 单元成为唯一 active 单元:stage 投影指向 attempt stage。
    assert!(item_blocked.stage.is_some());
    assert_eq!(
        by_logical(blocked_progress, &pending.logical_work_item_id).status,
        "pending"
    );
    assert_eq!(
        by_logical(blocked_progress, &completing.logical_work_item_id).status,
        "completed"
    );
    let blocked_aggregate = &blocked_http.1;
    assert_eq!(blocked_aggregate.total, 3);
    assert_eq!(blocked_aggregate.pending, 1);
    assert_eq!(blocked_aggregate.active, 0, "blocked 归入 failed_or_blocked 桶,active 归零");
    assert_eq!(blocked_aggregate.completed, 1);
    assert_eq!(blocked_aggregate.failed_or_blocked, 1);

    let crate::web::coding_ws_handler::CodingWsOutMessage::CodingSessionState {
        group_coding_progress: blocked_ws_progress,
        group_progress: blocked_ws_aggregate,
        ..
    } = blocked_ws
    else {
        panic!("expected blocked coding session state");
    };
    let blocked_ws_progress = blocked_ws_progress.expect("WS blocked progress");
    for (ws_item, http_item) in blocked_ws_progress.iter().zip(blocked_progress.iter()) {
        assert_eq!(ws_item, http_item, "Blocked 后 HTTP 与 WS 仍逐 WI 一致");
    }
    assert_eq!(
        blocked_ws_aggregate.expect("WS blocked aggregate"),
        *blocked_aggregate
    );

    // 既有 per-attempt DTO 断言不放宽:group attempt 主体仍可读。
    let dto = coding_attempt_dto(store, &blocked_attempt).unwrap();
    assert_eq!(dto.attempt_scope, "work_item_group");
    assert_eq!(dto.status, "blocked");
}
