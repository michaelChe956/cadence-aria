fn sample_code_review_report(attempt_id: &str) -> CodeReviewReport {
    CodeReviewReport {
        id: "code_review_0001".to_string(),
        attempt_id: attempt_id.to_string(),
        round: 1,
        verdict: ReviewVerdict::Approve,
        findings: vec![sample_finding()],
        tested_evidence_refs: vec!["code_review_command.log".to_string()],
        diff_refs: vec!["diff_0001".to_string()],
        summary: "基础 code review 通过".to_string(),
        created_at: "2026-05-23T00:01:00Z".to_string(),
        raw_provider_output_ref: None,
        role_run_id: None,
        run_no: None,
        unit_run_id: None,
    }
}

fn sample_review_request(attempt_id: &str) -> ReviewRequest {
    ReviewRequest {
        id: "review_request_0001".to_string(),
        attempt_id: attempt_id.to_string(),
        kind: ReviewRequestKind::GitBranchOnly,
        remote_kind: RemoteKind::GenericGit,
        remote: "origin".to_string(),
        base_branch: "main".to_string(),
        branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
        commit_sha: "abc123".to_string(),
        push_status: PushStatus::Pushed,
        external_url: None,
        manual_instructions: vec!["打开远端分支发起审查".to_string()],
        created_at: "2026-05-23T00:02:00Z".to_string(),
        updated_at: "2026-05-23T00:02:00Z".to_string(),
        push_error: None,
        owner_kind: ReviewRequestOwnerKind::Attempt,
        pointer_publication_id: None,
        revoked: false,
    }
}

fn sample_internal_review(attempt_id: &str, review_request_id: &str) -> InternalPrReview {
    InternalPrReview {
        id: "internal_review_0001".to_string(),
        attempt_id: attempt_id.to_string(),
        review_request_id: review_request_id.to_string(),
        verdict: ReviewVerdict::Approve,
        findings: vec![sample_finding()],
        impact_scope: vec!["src/lib.rs".to_string()],
        pr_description: "实现 work item".to_string(),
        commit_message_suggestion: "feat: implement work item".to_string(),
        tested_evidence_refs: vec!["internal_review_command.log".to_string()],
        diff_refs: vec!["diff_0001".to_string()],
        summary: "最终审查通过".to_string(),
        created_at: "2026-05-23T00:03:00Z".to_string(),
        raw_provider_output_ref: None,
        role_run_id: None,
        run_no: None,
    }
}

fn sample_finding() -> ReviewFinding {
    ReviewFinding {
        severity: FindingSeverity::Info,
        file_path: Some("src/lib.rs".to_string()),
        line: Some(1),
        message: "ok".to_string(),
        required_action: None,
        source_stage: CodingExecutionStage::CodeReview,
        evidence: Vec::new(),
        plan_defect_evidence: Vec::new(),
        related_requirements: Vec::new(),
        related_design_constraints: Vec::new(),
        related_work_item_tasks: Vec::new(),
        defect_class: cadence_aria::product::models::PlanDefectClass::ImplementationDefect,
        reason_code: None,
        contract_refs: Vec::new(),
        capability_refs: Vec::new(),
        repair_target: None,
        recommended_route: cadence_aria::product::models::PlanDefectRoute::CoderRework,
        confidence: None,
    }
}

fn sample_completed_node(attempt_id: &str) -> CodingTimelineNode {
    CodingTimelineNode {
        id: "coding_node_0001".to_string(),
        attempt_id: attempt_id.to_string(),
        stage: CodingExecutionStage::Coding,
        title: "代码编写".to_string(),
        status: CodingTimelineNodeStatus::Completed,
        agent_role: Some(CodingAgentRole::Author),
        summary: Some("代码编写完成".to_string()),
        started_at: "2026-05-23T00:01:00Z".to_string(),
        completed_at: Some("2026-05-23T00:02:00Z".to_string()),
        artifact_refs: vec!["coding_output_0001".to_string()],
    }
}

fn sample_running_node(attempt_id: &str) -> CodingTimelineNode {
    CodingTimelineNode {
        id: "coding_node_0002".to_string(),
        attempt_id: attempt_id.to_string(),
        stage: CodingExecutionStage::CodeReview,
        title: "Code Review".to_string(),
        status: CodingTimelineNodeStatus::Running,
        agent_role: Some(CodingAgentRole::Reviewer),
        summary: None,
        started_at: "2026-05-23T00:02:00Z".to_string(),
        completed_at: None,
        artifact_refs: vec![],
    }
}

#[tokio::test]
async fn coding_attempt_snapshot_includes_generated_work_item_execution_plan() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item(app.clone(), repo.path()).await;

    let (_status, attempt) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    let attempt_id = assert_global_attempt_id(&attempt);

    let (status, snapshot) = request_json(
        app,
        Method::GET,
        &scoped_attempt_uri(&attempt_id, ""),
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        snapshot["work_item_execution_plan"]["work_item_id"],
        "work_item_0001"
    );
    assert_eq!(snapshot["work_item_execution_plan"]["status"], "draft");
    assert!(
        snapshot["work_item_execution_plan"]["verification_plan_ref"]
            .as_str()
            .unwrap()
            .starts_with("verification_plan_")
    );
    assert!(
        snapshot["work_item_execution_plan"]["verification_summary"]
            .as_str()
            .unwrap()
            .contains("provider supplied required gate")
    );
}

#[tokio::test]
async fn single_repo_lifecycle_create_repair_delete() {
    // 单仓（无 manifest、无 selection）端到端：创建 → repair（plan-repair 恢复路径）→ 删除。
    // 创建 200 OK；repair 200 OK 且复用同一 attempt（无 500）；删除 204 NO_CONTENT。
    // repair 不是「任意重复 POST」：这里先把既有 attempt 置为 plan-repair 暂停态
    // （AwaitingPlanAmendment），再对同一计划重新 POST 走 plan-repair 恢复路径。
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let path = "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts";

    // 创建 → 200 OK
    let (status, body) = request_json(app.clone(), Method::POST, path, json!({})).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "single-repo group coding attempt creation must succeed: {body}"
    );
    let attempt_id = assert_global_attempt_id(&body);

    // repair：置为 plan-repair 暂停态后对同一计划重新 POST → 200 OK，恢复同一 attempt
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    pause_attempt_for_repair(&store, "project_0001", "issue_0001", &attempt_id);
    let (status, body) = request_json(app.clone(), Method::POST, path, json!({})).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "plan-repair resume must not return 500: {body}"
    );
    assert_eq!(
        assert_global_attempt_id(&body),
        attempt_id.as_str(),
        "plan-repair resume must reuse the existing attempt"
    );

    // 删除 → 204 NO_CONTENT
    let (status, _) = request_json(
        app,
        Method::DELETE,
        &format!("/api/projects/project_0001/issues/issue_0001/coding-attempts/{attempt_id}"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

/// 将既有 attempt 置为 plan-repair 暂停态（AwaitingPlanAmendment），
/// 供 `single_repo_lifecycle_create_repair_delete` 的 repair 恢复路径构造。
/// 仅改状态、不改 active pointers，保证 `validate_group_attempt_integrity` 通过。
fn pause_attempt_for_repair(
    store: &CodingAttemptStore,
    project_id: &str,
    issue_id: &str,
    attempt_id: &str,
) {
    let mut attempt = store
        .get_attempt(project_id, issue_id, attempt_id)
        .expect("load attempt for repair pause");
    attempt.status = CodingAttemptStatus::AwaitingPlanAmendment;
    store
        .update_attempt_non_status_fields(&attempt)
        .expect("persist paused attempt for plan repair");
}
