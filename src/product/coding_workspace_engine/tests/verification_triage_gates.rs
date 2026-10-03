// ─── C2 Task 8（#19／BYPASS-19，REQ-CVT-03/04）：独立验证处理与受限豁免 ───

use crate::product::coding_attempt_store::{
    CreateGroupCodingAttemptInput, EnterVerificationTriageInput, VerificationTriageConclusion,
    VerificationTriageStatus,
};
use crate::product::models::WorkspaceRolePermissionModes;
use crate::product::work_item_contract::VerificationCheck;

const TRIAGE_CHECK_NON_ZERO: &str = "check_nonzero";
const TRIAGE_CHECK_PLAIN: &str = "check_plain";
const TRIAGE_FINDING_REF: &str = "code_review_report_0001#0";

fn verification_triage_group_fixture() -> (
    tempfile::TempDir,
    crate::product::coding_attempt_store::CodingAttemptStore,
    CodingWorkspaceEngine,
    CodingExecutionAttempt,
) {
    let root = tempdir().expect("tempdir");
    let worktree = root.path().join("worktree");
    std::fs::create_dir_all(&worktree).expect("worktree");
    init_test_git_repo(&worktree);
    let original_head = git_stdout(&worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    let store = crate::product::coding_attempt_store::CodingAttemptStore::new(ProductAppPaths::new(
        root.path().join(".aria"),
    ));
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: original_head,
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
            start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
        })
        .expect("group attempt");
    let checks = vec![
        VerificationCheck {
            check_id: TRIAGE_CHECK_NON_ZERO.to_string(),
            command: Some("pnpm -C web exec vitest run src/lib.test.ts".to_string()),
            manual_instruction: None,
            required: true,
            non_zero_test_execution_required: true,
        },
        VerificationCheck {
            check_id: TRIAGE_CHECK_PLAIN.to_string(),
            command: Some("cargo test --lib".to_string()),
            manual_instruction: None,
            required: true,
            non_zero_test_execution_required: false,
        },
    ];
    super::seed_group_attempt_fixture_with_legacy_work_items(
        &store, &attempt, true, false, true, &checks, false,
    );
    let attempt = store
        .seed_running_attempt_for_test(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("running attempt");
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("code review stage");
    let (tx, _rx) = tokio::sync::mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    (root, store, engine, attempt)
}

fn active_plan_revision_id(
    store: &crate::product::coding_attempt_store::CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> String {
    store
        .get_active_coding_unit(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("units")
        .expect("active unit")
        .work_item_revision_id
}

fn seed_verification_incomplete_gate(
    store: &crate::product::coding_attempt_store::CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> crate::product::coding_models::CodingGateRequired {
    let blocked = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Blocked,
        )
        .expect("blocked");
    store
        .create_blocked_gate(
            &blocked,
            CreateBlockedGateInput {
                attempt_id: blocked.id.clone(),
                stage: CodingExecutionStage::CodeReview,
                node_id: None,
                role: Some(CodingProviderRole::CodeReviewer),
                title: "验证证据不完整".to_string(),
                description: "code review 验证不完整，等待人工处理".to_string(),
                reason_code: Some("code_review_verification_incomplete".to_string()),
                evidence_refs: Vec::new(),
                raw_provider_output_ref: None,
                available_actions: vec![
                    coding_gate_action_for_id("retry_review").expect("retry review action"),
                    coding_gate_action_for_id("send_to_coder").expect("send to coder action"),
                    coding_gate_action_for_id("manual_continue").expect("manual continue action"),
                    coding_gate_action_for_id("abort").expect("abort action"),
                ],
            },
        )
        .expect("verification incomplete gate")
}

fn seed_code_review_report_with_finding(
    store: &crate::product::coding_attempt_store::CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) {
    store
        .save_code_review_report(
            attempt,
            &CodeReviewReport {
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
                    evidence: Vec::new(),
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
                created_at: "2026-07-01T00:00:00Z".to_string(),
                raw_provider_output_ref: None,
                role_run_id: None,
                run_no: None,
                unit_run_id: None,
            },
        )
        .expect("code review report");
}

fn equivalent_evidence_entry(
    check_id: &str,
    test_execution_count: Option<u64>,
) -> crate::product::coding_workspace_engine::VerificationTriageEntryRequest {
    crate::product::coding_workspace_engine::VerificationTriageEntryRequest {
        finding_id: TRIAGE_FINDING_REF.to_string(),
        check_id: check_id.to_string(),
        original_command: Some("pnpm exec vitest run src/other.test.ts".to_string()),
        alternative_command: Some("pnpm -C web exec vitest run src/lib.test.ts".to_string()),
        cwd: Some("/repo".to_string()),
        outcome: Some("3 passed".to_string()),
        test_execution_count,
        environment: Some("linux".to_string()),
    }
}

/// 从 coder 输出门与 Code Review 验证不完整门分别转入：创建唯一验证处理
/// 记录并绑定全部字段；原门动作集合保持恰 retry_coding＋abort（coder 输出
/// 门）与恰四动作（CR 门）；门与 finding 不被关闭或改写；同键重入返回既有
/// 记录，不创建第二条。
#[tokio::test]
async fn verification_triage_entry_keeps_gate_action_sets_unchanged() {
    let (_root, store, engine, attempt) = verification_triage_group_fixture();
    seed_code_review_report_with_finding(&store, &attempt);
    let gate = seed_verification_incomplete_gate(&store, &attempt);

    let record = engine
        .enter_verification_triage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            equivalent_evidence_entry(TRIAGE_CHECK_PLAIN, Some(3)),
        )
        .await
        .expect("enter verification triage");

    assert!(record.triage_id.starts_with("verification_triage_"));
    assert_eq!(record.attempt_id, attempt.id);
    assert_eq!(record.finding_id, TRIAGE_FINDING_REF);
    assert_eq!(record.check_id, TRIAGE_CHECK_PLAIN);
    assert_eq!(
        record.plan_revision_id,
        active_plan_revision_id(&store, &attempt)
    );
    assert_eq!(
        record.original_command.as_deref(),
        Some("pnpm exec vitest run src/other.test.ts")
    );
    assert_eq!(
        record.alternative_command.as_deref(),
        Some("pnpm -C web exec vitest run src/lib.test.ts")
    );
    assert_eq!(record.cwd.as_deref(), Some("/repo"));
    assert_eq!(record.outcome.as_deref(), Some("3 passed"));
    assert_eq!(record.test_execution_count, Some(3));
    assert_eq!(record.environment.as_deref(), Some("linux"));
    assert_eq!(record.scope, vec![TRIAGE_CHECK_PLAIN.to_string()]);
    assert_eq!(record.status, VerificationTriageStatus::Pending);
    assert_eq!(record.conclusion, None);
    assert!(chrono::DateTime::parse_from_rfc3339(&record.expires_at).is_ok());

    // 原门保持开放且动作集合恰四动作，finding 报告不被改写。
    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("open gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].gate_id, gate.gate_id);
    assert_eq!(
        gates[0]
            .available_actions
            .iter()
            .map(|action| action.action_id.as_str())
            .collect::<Vec<_>>(),
        vec!["retry_review", "send_to_coder", "manual_continue", "abort"]
    );
    let reports = store
        .list_code_review_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("reports");
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].findings.len(), 1);
    assert_eq!(reports[0].findings[0].message, "missing validation");

    // 同 finding/check/plan revision 未决再转入：返回既有记录。
    let replay = engine
        .enter_verification_triage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            equivalent_evidence_entry(TRIAGE_CHECK_PLAIN, Some(3)),
        )
        .await
        .expect("re-enter verification triage");
    assert_eq!(replay.triage_id, record.triage_id);
    assert_eq!(
        store
            .list_verification_triage_records(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("triage records")
            .len(),
        1
    );

    // coder 输出门路径：动作集合保持恰 retry_coding＋abort，finding 引用绑定
    // 该门的 plan defect finding（无 report#idx 持久面）。
    let (_root2, store2, engine2, attempt2) = verification_triage_group_fixture();
    engine2
        .open_coding_output_human_triage_gate(
            &attempt2,
            "coding_node_0001",
            None,
            Some("structured output parse failed"),
            None,
        )
        .expect("coder output gate");
    let coder_record = engine2
        .enter_verification_triage(
            &attempt2.project_id,
            &attempt2.issue_id,
            &attempt2.id,
            crate::product::coding_workspace_engine::VerificationTriageEntryRequest {
                finding_id: "plan_defect_finding_0001".to_string(),
                check_id: TRIAGE_CHECK_PLAIN.to_string(),
                original_command: None,
                alternative_command: None,
                cwd: None,
                outcome: None,
                test_execution_count: None,
                environment: None,
            },
        )
        .await
        .expect("enter from coder output gate");
    assert_eq!(coder_record.finding_id, "plan_defect_finding_0001");
    assert_eq!(
        coder_record.original_command.as_deref(),
        Some("cargo test --lib"),
        "原命令缺失时按绑定 check 的计划合同字面命令回填"
    );
    let gates2 = store2
        .list_open_blocked_gates(&attempt2.project_id, &attempt2.issue_id, &attempt2.id)
        .expect("open gates");
    assert_eq!(gates2.len(), 1);
    assert_eq!(
        gates2[0]
            .available_actions
            .iter()
            .map(|action| action.action_id.as_str())
            .collect::<Vec<_>>(),
        vec!["retry_coding", "abort"]
    );
}

/// 拒绝条件全部 fail-closed：无用户批准上下文、非零测试要求而证据测试量
/// 为零、证据不完整、豁免 scope 宽于绑定 check、plan revision 过期；记录
/// 保持 Pending，原链零推进。
#[tokio::test]
async fn verification_triage_decisions_fail_closed_without_evidence_or_approval() {
    let (_root, store, engine, attempt) = verification_triage_group_fixture();
    seed_code_review_report_with_finding(&store, &attempt);
    seed_verification_incomplete_gate(&store, &attempt);

    // check 要求非零测试，但证据测试执行数量为零。
    let nonzero = engine
        .enter_verification_triage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            equivalent_evidence_entry(TRIAGE_CHECK_NON_ZERO, Some(0)),
        )
        .await
        .expect("enter non-zero triage");

    let mut decision = crate::product::coding_workspace_engine::VerificationTriageDecisionRequest {
        triage_id: nonzero.triage_id.clone(),
        conclusion: VerificationTriageConclusion::AcceptEquivalentEvidence,
        decided_by: "operator-1".to_string(),
        reason: "等价证据成立".to_string(),
        exemption_scope: Vec::new(),
    };
    let rejected = engine
        .decide_verification_triage(&attempt.project_id, &attempt.issue_id, &attempt.id, decision.clone())
        .await
        .expect_err("zero test evidence must fail closed");
    assert!(
        rejected.to_string().contains("verification_triage_non_zero_test_required"),
        "{rejected}"
    );
    assert!(
        store
            .list_verification_triage_records(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("records")[0]
            .status
            == VerificationTriageStatus::Pending,
        "拒绝后记录保持 Pending"
    );

    // 无用户批准上下文（decided_by 为空）。
    let unapproved = engine
        .decide_verification_triage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            crate::product::coding_workspace_engine::VerificationTriageDecisionRequest {
                triage_id: nonzero.triage_id.clone(),
                conclusion: VerificationTriageConclusion::AcceptEquivalentEvidence,
                decided_by: "  ".to_string(),
                reason: "等价证据成立".to_string(),
                exemption_scope: Vec::new(),
            },
        )
        .await
        .expect_err("missing approval context must fail closed");
    assert!(
        unapproved
            .to_string()
            .contains("verification_triage_approval_context_required"),
        "{unapproved}"
    );

    // 证据不完整（无替代命令）。
    let incomplete = engine
        .enter_verification_triage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            crate::product::coding_workspace_engine::VerificationTriageEntryRequest {
                finding_id: TRIAGE_FINDING_REF.to_string(),
                check_id: TRIAGE_CHECK_PLAIN.to_string(),
                original_command: None,
                alternative_command: None,
                cwd: None,
                outcome: None,
                test_execution_count: None,
                environment: None,
            },
        )
        .await
        .expect("enter incomplete triage");
    decision.triage_id = incomplete.triage_id.clone();
    let evidence_rejected = engine
        .decide_verification_triage(&attempt.project_id, &attempt.issue_id, &attempt.id, decision.clone())
        .await
        .expect_err("incomplete evidence must fail closed");
    assert!(
        evidence_rejected
            .to_string()
            .contains("verification_triage_evidence_incomplete"),
        "{evidence_rejected}"
    );

    // 豁免 scope 宽于绑定 check。
    let mut scoped = decision.clone();
    scoped.triage_id = nonzero.triage_id.clone();
    scoped.conclusion = VerificationTriageConclusion::GrantScopedEnvironmentException;
    scoped.exemption_scope = vec!["check_other".to_string()];
    let scope_rejected = engine
        .decide_verification_triage(&attempt.project_id, &attempt.issue_id, &attempt.id, scoped)
        .await
        .expect_err("scope wider than bound check must fail closed");
    assert!(
        scope_rejected
            .to_string()
            .contains("verification_triage_scope_exceeds_bound_check"),
        "{scope_rejected}"
    );

    // plan revision 过期（store 级直接落一条绑定旧 revision 的记录）。
    store
        .enter_verification_triage(
            &attempt,
            EnterVerificationTriageInput {
                attempt_id: attempt.id.clone(),
                finding_id: TRIAGE_FINDING_REF.to_string(),
                check_id: TRIAGE_CHECK_PLAIN.to_string(),
                plan_revision_id: "work_item_revision_0002".to_string(),
                original_command: Some("cargo test --lib".to_string()),
                alternative_command: Some("cargo test --lib alt".to_string()),
                cwd: Some("/repo".to_string()),
                outcome: Some("passed".to_string()),
                test_execution_count: Some(2),
                environment: None,
                scope: vec![TRIAGE_CHECK_PLAIN.to_string()],
                expires_at: (chrono::Utc::now() + chrono::Duration::days(7))
                    .to_rfc3339(),
            },
        )
        .expect("stale triage record");
    let records = store
        .list_verification_triage_records(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("records");
    let stale = records
        .iter()
        .find(|record| record.plan_revision_id == "work_item_revision_0002")
        .expect("stale record");
    decision.triage_id = stale.triage_id.clone();
    decision.conclusion = VerificationTriageConclusion::AcceptEquivalentEvidence;
    let stale_rejected = engine
        .decide_verification_triage(&attempt.project_id, &attempt.issue_id, &attempt.id, decision)
        .await
        .expect_err("stale plan revision must fail closed");
    assert!(
        stale_rejected
            .to_string()
            .contains("verification_triage_plan_revision_expired"),
        "{stale_rejected}"
    );

    // 原链零推进：门保持开放，attempt 保持 Blocked。
    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("open gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(
        store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("attempt")
            .status,
        CodingAttemptStatus::Blocked
    );
}

/// 批准"等价证据"后：原门按 manual_continue 语义续跑（复用 Task 4），finding
/// 保留并标注"已由验证处理覆盖"，审计（操作者／时间／理由）落账；重复决定
/// 幂等返回首次结果。
#[tokio::test]
async fn approved_equivalent_evidence_annotates_finding_and_resumes_gate() {
    let (_root, store, engine, attempt) = verification_triage_group_fixture();
    seed_code_review_report_with_finding(&store, &attempt);
    seed_verification_incomplete_gate(&store, &attempt);

    let record = engine
        .enter_verification_triage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            equivalent_evidence_entry(TRIAGE_CHECK_NON_ZERO, Some(3)),
        )
        .await
        .expect("enter triage");

    let decision = crate::product::coding_workspace_engine::VerificationTriageDecisionRequest {
        triage_id: record.triage_id.clone(),
        conclusion: VerificationTriageConclusion::AcceptEquivalentEvidence,
        decided_by: "operator-1".to_string(),
        reason: "替代命令在同一测试面通过".to_string(),
        exemption_scope: Vec::new(),
    };
    let approved = engine
        .decide_verification_triage(&attempt.project_id, &attempt.issue_id, &attempt.id, decision)
        .await
        .expect("approve equivalent evidence");
    assert_eq!(approved.status, VerificationTriageStatus::Approved);
    assert_eq!(
        approved.conclusion,
        Some(VerificationTriageConclusion::AcceptEquivalentEvidence)
    );
    assert_eq!(approved.decided_by.as_deref(), Some("operator-1"));
    assert_eq!(approved.reason.as_deref(), Some("替代命令在同一测试面通过"));
    assert!(approved.decided_at.is_some());

    // 原门按 manual_continue 语义续跑：门关闭、attempt 回 Running、审计落账。
    assert!(
        store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("open gates")
            .is_empty()
    );
    assert_eq!(
        store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("attempt")
            .status,
        CodingAttemptStatus::Running
    );
    let audits = store
        .list_quality_bypass_audits(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("audits");
    assert_eq!(audits.len(), 1);
    assert!(audits[0].operator_context.contains(&record.triage_id));

    // finding 保留并标注"已由验证处理覆盖"，不清空、不改写。
    let reports = store
        .list_code_review_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("reports");
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].findings.len(), 1);
    assert_eq!(reports[0].findings[0].message, "missing validation");
    let annotation = store
        .verification_triage_annotation_for_finding(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            TRIAGE_FINDING_REF,
        )
        .expect("annotation");
    assert_eq!(
        annotation,
        Some(format!("已由验证处理覆盖（{}）", record.triage_id))
    );

    // 同决定重复提交：幂等返回首次结果，不重复审计。
    let replay = engine
        .decide_verification_triage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            crate::product::coding_workspace_engine::VerificationTriageDecisionRequest {
                triage_id: record.triage_id.clone(),
                conclusion: VerificationTriageConclusion::AcceptEquivalentEvidence,
                decided_by: "operator-1".to_string(),
                reason: "替代命令在同一测试面通过".to_string(),
                exemption_scope: Vec::new(),
            },
        )
        .await
        .expect("replay decision");
    assert_eq!(replay, approved);
    assert_eq!(
        store
            .list_quality_bypass_audits(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("audits")
            .len(),
        1
    );
}

/// 批准"计划修订"：记录 Approved，attempt 转入 AwaitingPlanAmendment 由既有
/// amendment 链接管（无 linked repair 时不强造修订），原门不被该结论关闭。
#[tokio::test]
async fn approved_plan_revision_routes_into_amendment_chain() {
    let (_root, store, engine, attempt) = verification_triage_group_fixture();
    seed_code_review_report_with_finding(&store, &attempt);
    seed_verification_incomplete_gate(&store, &attempt);
    let record = engine
        .enter_verification_triage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            equivalent_evidence_entry(TRIAGE_CHECK_PLAIN, Some(3)),
        )
        .await
        .expect("enter triage");

    let approved = engine
        .decide_verification_triage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            crate::product::coding_workspace_engine::VerificationTriageDecisionRequest {
                triage_id: record.triage_id.clone(),
                conclusion: VerificationTriageConclusion::ApprovePlanRevision,
                decided_by: "operator-1".to_string(),
                reason: "计划命令不可满足，需修订".to_string(),
                exemption_scope: Vec::new(),
            },
        )
        .await
        .expect("approve plan revision");
    assert_eq!(approved.status, VerificationTriageStatus::Approved);
    assert_eq!(
        approved.conclusion,
        Some(VerificationTriageConclusion::ApprovePlanRevision)
    );
    assert_eq!(
        store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("attempt")
            .status,
        CodingAttemptStatus::AwaitingPlanAmendment
    );
    // 计划修订结论不关闭原门：门的解决由 amendment 链收口。
    assert_eq!(
        store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("open gates")
            .len(),
        1
    );
    // 计划修订结论不标注 finding 覆盖（finding 走修订链处理）。
    assert_eq!(
        store.verification_triage_annotation_for_finding(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            TRIAGE_FINDING_REF
        )
        .expect("annotation"),
        None
    );
}

// ─── C2 Task 9（#2/#9，REQ-CVT-01/02/05）：计划命令与实际命令并列证据、
// 重跑原计划命令与三类文案 ───

/// 计划命令合法、coder 在不同参数下执行失败：并列显示计划 check 命令与
/// 实际命令、cwd、退出码，标注"实际执行命令与计划不一致"；无对应命令
/// 记录时实际命令栏"未记录"（None），不推断补写。
#[tokio::test]
async fn verification_evidence_panel_marks_actual_command_mismatch() {
    let (_root, store, _engine, attempt) = verification_triage_group_fixture();
    let checks = [
        VerificationCheck {
            check_id: TRIAGE_CHECK_PLAIN.to_string(),
            command: Some("cargo test --lib".to_string()),
            manual_instruction: None,
            required: true,
            non_zero_test_execution_required: false,
        },
        VerificationCheck {
            check_id: "check_manual".to_string(),
            command: None,
            manual_instruction: Some("人工核对迁移脚本输出".to_string()),
            required: false,
            non_zero_test_execution_required: false,
        },
    ];
    let plain = checks[0].clone();

    // 无任何 role-run 命令记录：实际命令"未记录"（None），不推断补写。
    let unrecorded = store
        .verification_command_evidence(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &plain,
        )
        .expect("evidence without runs");
    assert_eq!(unrecorded.planned_command.as_deref(), Some("cargo test --lib"));
    assert_eq!(unrecorded.actual_command, None, "未记录不得推断补写");
    assert_eq!(unrecorded.actual_cwd, None);
    assert_eq!(unrecorded.exit_code, None);
    assert!(!unrecorded.mismatch);

    // coder 在不同参数执行失败：role-run JSONL 的 ExecutionEvent 派生并列证据。
    let role_run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Coding,
            CodingProviderRole::Coder,
            CodingRoleRunTrigger::Initial,
            None,
        )
        .expect("role run");
    store
        .append_role_run_event(
            &attempt,
            &role_run,
            crate::product::coding_models::CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "event_id": "exec_0001",
                "kind": "Command",
                "status": "Failed",
                "title": "cargo test",
                "command": "cargo test --lib --features strict",
                "cwd": "/repo/worktree",
                "output": "1 failed",
                "exit_code": 1
            }),
        )
        .expect("execution event");

    let evidence = store
        .verification_command_evidence(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &plain,
        )
        .expect("evidence");
    assert_eq!(evidence.check_id, TRIAGE_CHECK_PLAIN);
    assert_eq!(evidence.planned_command.as_deref(), Some("cargo test --lib"));
    assert_eq!(
        evidence.actual_command.as_deref(),
        Some("cargo test --lib --features strict")
    );
    assert_eq!(evidence.actual_cwd.as_deref(), Some("/repo/worktree"));
    assert_eq!(evidence.exit_code, Some(1));
    assert!(evidence.mismatch, "不同参数执行必须标注与计划不一致");

    // 无命令的 manual check：planned_manual_instruction 并列，不误标 mismatch。
    let manual = checks[1].clone();
    let manual_evidence = store
        .verification_command_evidence(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &manual,
        )
        .expect("manual evidence");
    assert_eq!(manual_evidence.planned_command, None);
    assert_eq!(
        manual_evidence.planned_manual_instruction.as_deref(),
        Some("人工核对迁移脚本输出")
    );
    assert!(!manual_evidence.mismatch);

    // 三类文案：全新中文口径，指向对应操作；MUST NOT 建议升级运行时版本、
    // 泛化重试或"忽略 finding"。
    let mismatch_copy = crate::product::coding_attempt_store::verification_evidence_copy(
        "actual_command_mismatch",
    )
    .expect("mismatch copy");
    assert!(mismatch_copy.contains("实际执行命令与计划不一致"));
    assert!(mismatch_copy.contains("重跑原计划命令"));
    let undeclared_copy = crate::product::coding_attempt_store::verification_evidence_copy(
        "plan_undeclared_path_or_command",
    )
    .expect("undeclared copy");
    assert!(undeclared_copy.contains("计划未声明"));
    assert!(undeclared_copy.contains("计划反馈"));
    let unexecutable_copy = crate::product::coding_attempt_store::verification_evidence_copy(
        "plan_path_unexecutable",
    )
    .expect("unexecutable copy");
    assert!(unexecutable_copy.contains("计划路径不可执行"));
    assert!(unexecutable_copy.contains("计划修订") || unexecutable_copy.contains("验证处理"));
    for copy in [mismatch_copy, undeclared_copy, unexecutable_copy] {
        assert!(!copy.contains("升级"), "文案不得建议升级运行时版本：{copy}");
        assert!(!copy.contains("重试一切"), "文案不得建议泛化重试：{copy}");
        assert!(!copy.contains("忽略"), "文案不得建议忽略 finding：{copy}");
    }
    assert!(
        crate::product::coding_attempt_store::verification_evidence_copy("unknown_reason")
            .is_none()
    );
}

/// 重跑原计划命令：以计划合同字面命令与 cwd 作明确返修指令走既有 rework
/// 落地面（后续 coder run 经 Task 7 事务消费）；同 command_id 重复点击只
/// 触发一次返修并返回同一结果；错 expected 版本 fail-closed；不改写计划
/// 合同、不清 finding。
#[tokio::test]
async fn rerun_planned_command_routes_via_rework_once_per_command() {
    let (_root, store, engine, attempt) = verification_triage_group_fixture();
    seed_code_review_report_with_finding(&store, &attempt);
    seed_verification_incomplete_gate(&store, &attempt);

    let request = crate::product::coding_workspace_engine::RerunPlannedCommandRequest {
        command_id: "rerun-cmd-0001".to_string(),
        gate_id: store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("gates")[0]
            .gate_id
            .clone(),
        check_id: TRIAGE_CHECK_PLAIN.to_string(),
        expected_version: attempt.rework_count as u64,
    };
    let outcome = engine
        .rerun_planned_command(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &request,
        )
        .await
        .expect("rerun planned command");
    assert!(!outcome.replayed, "首次执行");
    assert_eq!(outcome.attempt.status, CodingAttemptStatus::Running);
    assert_eq!(outcome.attempt.stage, CodingExecutionStage::Coding);

    // 返修指令携带计划合同字面命令全文与 worktree cwd，尚未被消费
    //（消费发生在下一次 coder run 的 Task 7 事务）。
    let instructions = store
        .list_rework_instructions(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("instructions");
    assert_eq!(instructions.len(), 1);
    assert!(instructions[0].consumed_at.is_none());
    assert!(
        instructions[0]
            .fix_hints
            .iter()
            .any(|hint| hint.contains("cargo test --lib")),
        "计划字面命令必须全文进入返修指令：{:?}",
        instructions[0].fix_hints
    );

    // 同 command_id 重复点击：幂等返回同一结果，不产生第二条指令。
    let replay = engine
        .rerun_planned_command(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &request,
        )
        .await
        .expect("replay rerun");
    assert!(replay.replayed, "同 command 同 payload 必须重放首次结果");
    assert_eq!(replay.instruction_id, outcome.instruction_id);
    assert_eq!(
        store
            .list_rework_instructions(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("instructions")
            .len(),
        1
    );

    // 错 expected 版本 fail-closed：提示刷新，不触发返修、不启动 provider。
    let conflict = engine
        .rerun_planned_command(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &crate::product::coding_workspace_engine::RerunPlannedCommandRequest {
                command_id: "rerun-cmd-0002".to_string(),
                gate_id: request.gate_id.clone(),
                check_id: TRIAGE_CHECK_PLAIN.to_string(),
                expected_version: 99,
            },
        )
        .await
        .expect_err("stale version must fail closed");
    assert!(
        conflict.to_string().contains("coding_rerun_version_conflict"),
        "{conflict}"
    );
    assert_eq!(
        store
            .list_rework_instructions(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("instructions")
            .len(),
        1
    );

    // 不改写计划合同、不清 finding：code review report findings 保持原样。
    let reports = store
        .list_code_review_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("reports");
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].findings.len(), 1);
    assert_eq!(reports[0].findings[0].message, "missing validation");
}
