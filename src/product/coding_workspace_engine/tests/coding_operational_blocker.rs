// P2-b（REQ-COD-08、REQ-CRO-09/10）：coder 路径运行环境阻塞闭环的行为面测试。
//
// 覆盖：
// - 校验放宽：`operational_blocker`+`operational_gate` 且 reason_code 不在计划契约
//   blocker_rules（空 blocker_routing）的 coder finding → 决策 `OpenOperationalGate`
//   （WI-002 同构现场；此前恒为 `StopForHumanTriage`）。其余 defect_class 保持
//   原严格校验（plan 契约背书，缺 rule 仍 `StopForHumanTriage`）。
// - 落门：coder 路径（首次 coding 与 rework，A-9 分支顺序）`OpenOperationalGate`
//   落 `coding_operational_blocker` 门，attempt `Blocked`，动作恰为
//   `[retry_coding, retry_coding_with_context, abort]`。
// - 新动作：`retry_coding_with_context` 先写 context note 再等同 retry_coding
//   （回 Running/Coding），下一轮 coder prompt 含「本轮补充上下文」；空/全空白 →
//   `coding_gate_extra_context_required`；>2000 → `coding_gate_extra_context_too_long`；
//   失败时 gate 保持 Open、attempt 保持 Blocked（可重试）。
// - X2 回归：旧门 `coding_output_human_triage` 的动作集合保持恰
//   `[retry_coding, abort]` 不变。
use super::*;
use crate::product::coding_models::CodingGateKind;
use crate::product::plan_repair::{
    PlanDefectClass, PlanDefectConfidence, PlanDefectEvidence, PlanDefectFinding, PlanDefectRoute,
    PlanDefectSeverity,
};

const OPERATIONAL_BLOCKER_GATE: &str = "coding_operational_blocker";
const RETRY_WITH_CONTEXT: &str = "retry_coding_with_context";

/// WI-002 同构 finding：operational_blocker + operational_gate + coder 自造
/// reason_code（任何 blocker_rules 都未声明）+ 空投影 blocker_routing。
fn wi002_isomorphic_operational_finding() -> PlanDefectFinding {
    PlanDefectFinding {
        finding_id: "PDF-CODER-0001".to_string(),
        severity: PlanDefectSeverity::Error,
        defect_class: PlanDefectClass::OperationalBlocker,
        reason_code: "upstream_deliverable_absent_in_runtime".to_string(),
        message: "CT-001 上游交付物缺失：busi 成员仓（WI-001）在运行环境中完全不存在"
            .to_string(),
        evidence: vec![PlanDefectEvidence {
            kind: "filesystem_observation".to_string(),
            source_ref: "/tmp/notify-center".to_string(),
            message: "聚合根仅含 api/.git 骨架；busi 成员仓不存在".to_string(),
        }],
        contract_refs: vec!["CT-001".to_string()],
        capability_refs: Vec::new(),
        repair_target: None,
        recommended_route: PlanDefectRoute::OperationalGate,
        confidence: PlanDefectConfidence::High,
    }
}

/// 与 `wi002_isomorphic_operational_finding` 同构的 coder 原始输出（结论 JSON）。
fn wi002_operational_output() -> String {
    serde_json::json!({
        "plan_defect_findings": [{
            "finding_id": "PDF-CODER-0001",
            "severity": "error",
            "defect_class": "operational_blocker",
            "reason_code": "upstream_deliverable_absent_in_runtime",
            "message": "CT-001 上游交付物缺失：busi 成员仓（WI-001）在运行环境中完全不存在",
            "evidence": [{
                "kind": "filesystem_observation",
                "source_ref": "/tmp/notify-center",
                "message": "聚合根仅含 api/.git 骨架；busi 成员仓不存在"
            }],
            "contract_refs": ["CT-001"],
            "capability_refs": [],
            "repair_target": null,
            "recommended_route": "operational_gate",
            "confidence": "high"
        }]
    })
    .to_string()
}

/// 校验放宽：operational_blocker+operational_gate 的 coder finding 不再要求
/// reviewer projection blocker rule——空 blocker_routing（单仓恒空、现场
/// WI-002 bundle 亦空）时仍判 `OpenOperationalGate`，而非 `StopForHumanTriage`。
#[test]
fn operational_blocker_finding_without_blocker_rule_decides_open_operational_gate() {
    let report = ExecutionPlanDefectReport {
        source: PlanDefectSource::Coder,
        findings: vec![wi002_isomorphic_operational_finding()],
    };
    let decision = execution_plan_defect_flow_decision(&report, &empty_reviewer_projection());
    assert_eq!(
        decision,
        CodeReviewFlowDecision::OpenOperationalGate,
        "operational_blocker+operational_gate 的 coder finding 在无 blocker rule 时必须判 OpenOperationalGate"
    );
}

/// 其余 defect_class（plan repair 类）保持原严格校验：缺 blocker rule 仍
/// `StopForHumanTriage`——它们触发会改写计划的重动作，必须有 plan 契约背书。
#[test]
fn non_operational_plan_defect_still_requires_blocker_rule() {
    let report = ExecutionPlanDefectReport {
        source: PlanDefectSource::Coder,
        findings: vec![PlanDefectFinding {
            finding_id: "PDF-CODER-0002".to_string(),
            severity: PlanDefectSeverity::Error,
            defect_class: PlanDefectClass::CurrentWorkItemInvalid,
            reason_code: "current_work_item_contract_invalid".to_string(),
            message: "当前 work item 契约不可实现".to_string(),
            evidence: vec![PlanDefectEvidence {
                kind: "test_execution".to_string(),
                source_ref: "provider-managed-unit.log".to_string(),
                message: "当前契约无法被执行".to_string(),
            }],
            contract_refs: Vec::new(),
            capability_refs: Vec::new(),
            repair_target: None,
            recommended_route: PlanDefectRoute::PlanRepair,
            confidence: PlanDefectConfidence::High,
        }],
    };
    let decision = execution_plan_defect_flow_decision(&report, &empty_reviewer_projection());
    assert_eq!(
        decision,
        CodeReviewFlowDecision::StopForHumanTriage,
        "plan repair 类 finding 缺 blocker rule 时必须保持 StopForHumanTriage"
    );
}

/// 首次 coding 路径：`OpenOperationalGate` 决策落 `coding_operational_blocker`
/// 门（此前无落门分支），attempt `Blocked`，动作恰三按钮、顺序固定。
#[tokio::test]
async fn coder_operational_gate_lands_operational_blocker_gate_with_three_actions() {
    let (root, store, attempt) = running_attempt_with_worktree();
    init_test_git_repo(attempt.worktree_path.as_ref().unwrap());
    let (tx, _rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider =
        super::provider_execution_context::CapturingProjectionProvider::new(wi002_operational_output());
    let (_cmd_tx, mut cmd_rx) = mpsc::channel(1);
    engine
        .execute_coding_with_commands(
            &attempt,
            &provider,
            &CodingExecutionContext::default(),
            &mut cmd_rx,
        )
        .await
        .expect("execute coding");

    let persisted = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("persisted attempt");
    assert_eq!(
        persisted.status,
        CodingAttemptStatus::Blocked,
        "OpenOperationalGate 必须把 attempt 转为 Blocked"
    );

    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("open gates");
    assert_eq!(gates.len(), 1, "必须且只能落一个 gate");
    let gate = &gates[0];
    assert_eq!(gate.reason_code.as_deref(), Some(OPERATIONAL_BLOCKER_GATE));
    assert_eq!(gate.title, "运行环境阻塞");
    assert_eq!(gate.kind, CodingGateKind::Blocked);
    assert_eq!(gate.stage, Some(CodingExecutionStage::Coding));
    assert_eq!(gate.role, Some(CodingProviderRole::Coder));
    assert!(
        gate.raw_provider_output_ref.is_some(),
        "门必须携带 coder 原始输出引用"
    );
    assert_eq!(
        gate.available_actions
            .iter()
            .map(|action| action.action_id.as_str())
            .collect::<Vec<_>>(),
        vec!["retry_coding", RETRY_WITH_CONTEXT, "abort"],
        "运行环境阻塞门动作恰为三按钮且顺序固定"
    );
    assert_eq!(
        gate.available_actions[1].label, "我来说明情况，让 AI 继续",
        "第二按钮 label 固定"
    );
    drop(root);
}

/// rework 路径（A-9）：`OpenOperationalGate` 必须先于
/// `else if decision != RunCoderFix` 的 updated 分支被匹配——不得被吞入
/// 「原样返回 Running/Coding 无门」的静默路径。
#[tokio::test]
async fn rework_operational_gate_lands_operational_blocker_gate() {
    let (root, store, attempt) = running_attempt_with_worktree();
    init_test_git_repo(attempt.worktree_path.as_ref().unwrap());
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("code review stage");
    let (tx, _rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider =
        super::provider_execution_context::CapturingProjectionProvider::new(wi002_operational_output());
    let (_cmd_tx, mut cmd_rx) = mpsc::channel(1);
    engine
        .execute_coder_fix_from_review(
            &attempt,
            &super::provider_driven::review_report_requesting_changes(&attempt),
            &CodingExecutionContext::default(),
            &provider,
            &mut cmd_rx,
        )
        .await
        .expect("coder fix from review");

    let persisted = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("persisted attempt");
    assert_eq!(
        persisted.status,
        CodingAttemptStatus::Blocked,
        "rework 后 OpenOperationalGate 必须把 attempt 转为 Blocked，不得静默停在 Running/Coding"
    );
    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("open gates");
    assert_eq!(gates.len(), 1, "rework 路径必须落一个 gate");
    assert_eq!(gates[0].reason_code.as_deref(), Some(OPERATIONAL_BLOCKER_GATE));
    assert_eq!(
        gates[0]
            .available_actions
            .iter()
            .map(|action| action.action_id.as_str())
            .collect::<Vec<_>>(),
        vec!["retry_coding", RETRY_WITH_CONTEXT, "abort"]
    );
    drop(root);
}

/// X2 回归：coder 输出解析失败（教学重驱后仍失败）→ `StopForHumanTriage`
/// → 旧门 `coding_output_human_triage` 动作集合保持恰 `[retry_coding, abort]`。
#[tokio::test]
async fn parse_failure_still_lands_human_triage_gate_with_two_actions() {
    let (root, store, attempt) = running_attempt_with_worktree();
    init_test_git_repo(attempt.worktree_path.as_ref().unwrap());
    let (tx, _rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    // 声明了 plan_defect_findings 但结构非法：Vec 反序列化失败 → parse Err；
    // 教学重驱输出同样非法 → 仍 Err → StopForHumanTriage → 旧门。
    let provider = super::provider_execution_context::CapturingProjectionProvider::new(
        "{\"plan_defect_findings\": \"not-an-array\"}",
    );
    let (_cmd_tx, mut cmd_rx) = mpsc::channel(1);
    engine
        .execute_coding_with_commands(
            &attempt,
            &provider,
            &CodingExecutionContext::default(),
            &mut cmd_rx,
        )
        .await
        .expect("execute coding");

    let persisted = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("persisted attempt");
    assert_eq!(persisted.status, CodingAttemptStatus::Blocked);
    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("open gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(
        gates[0].reason_code.as_deref(),
        Some("coding_output_human_triage")
    );
    assert_eq!(
        gates[0]
            .available_actions
            .iter()
            .map(|action| action.action_id.as_str())
            .collect::<Vec<_>>(),
        vec!["retry_coding", "abort"],
        "X2：旧门动作集合保持恰 retry_coding+abort"
    );
    drop(root);
}

/// 跑一轮 coding 落下 `coding_operational_blocker` 门，返回 (gate_id, store)。
async fn land_operational_blocker_gate(
    root: tempfile::TempDir,
    store: CodingAttemptStore,
    attempt: CodingExecutionAttempt,
) -> (tempfile::TempDir, CodingAttemptStore, String) {
    let _ = root;
    let (tx, _rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider =
        super::provider_execution_context::CapturingProjectionProvider::new(wi002_operational_output());
    let (_cmd_tx, mut cmd_rx) = mpsc::channel(1);
    engine
        .execute_coding_with_commands(
            &attempt,
            &provider,
            &CodingExecutionContext::default(),
            &mut cmd_rx,
        )
        .await
        .expect("execute coding");
    let gate_id = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("open gates")
        .into_iter()
        .find(|gate| gate.reason_code.as_deref() == Some(OPERATIONAL_BLOCKER_GATE))
        .expect("operational blocker gate")
        .gate_id;
    (root, store, gate_id)
}

/// `retry_coding_with_context`：先写 context note、再等同 retry_coding 回
/// Running/Coding、门 resolve；下一轮 coder prompt 含「本轮补充上下文」段。
#[tokio::test]
async fn retry_coding_with_context_writes_note_and_resumes_coding() {
    let (root, store, attempt) = running_attempt_with_worktree();
    init_test_git_repo(attempt.worktree_path.as_ref().unwrap());
    let (root, store, gate_id) = land_operational_blocker_gate(root, store, attempt.clone()).await;
    let (tx, _rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let updated = engine
        .handle_blocked_gate_response(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &gate_id,
            RETRY_WITH_CONTEXT,
            Some("busi 仓已恢复并重新物化，请按原计划重跑 CHECK-002".to_string()),
        )
        .await
        .expect("retry coding with context");

    assert_eq!(
        updated.status,
        CodingAttemptStatus::Running,
        "说明情况后等同 retry_coding：attempt 回 Running"
    );
    assert_eq!(updated.stage, CodingExecutionStage::Coding);

    let notes = store
        .list_context_notes(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("context notes");
    assert_eq!(notes.len(), 1, "必须先写一条 context note");
    assert_eq!(
        notes[0].content, "busi 仓已恢复并重新物化，请按原计划重跑 CHECK-002"
    );

    assert!(
        store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("open gates")
            .is_empty(),
        "动作成功后门必须 resolve"
    );

    // 下一轮 coder prompt 带出「本轮补充上下文」段与 note 内容。
    let provider = super::provider_execution_context::CapturingProjectionProvider::new(
        "{\"plan_defect_findings\": []}",
    );
    let (_cmd_tx, mut cmd_rx) = mpsc::channel(1);
    engine
        .execute_coding_with_commands(
            &updated,
            &provider,
            &CodingExecutionContext::default(),
            &mut cmd_rx,
        )
        .await
        .expect("rerun coding");
    let prompt = provider.input().prompt;
    assert!(
        prompt.contains("本轮补充上下文"),
        "下一轮 coder prompt 必须含「本轮补充上下文」段"
    );
    assert!(
        prompt.contains("busi 仓已恢复并重新物化"),
        "补充说明内容必须进入下一轮 prompt"
    );
    drop(root);
}

/// 空/全空白 `extra_context` → `coding_gate_extra_context_required`，
/// 门保持 Open、attempt 保持 Blocked（可重试，不产生半执行状态）。
#[tokio::test]
async fn retry_coding_with_context_requires_non_empty_context() {
    let (root, store, attempt) = running_attempt_with_worktree();
    init_test_git_repo(attempt.worktree_path.as_ref().unwrap());
    let (root, store, gate_id) = land_operational_blocker_gate(root, store, attempt.clone()).await;
    let (tx, _rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    for empty in [None, Some("   \n\t ".to_string())] {
        let error = engine
            .handle_blocked_gate_response(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &gate_id,
                RETRY_WITH_CONTEXT,
                empty,
            )
            .await
            .expect_err("empty context must be rejected");
        assert!(
            matches!(
                &error,
                CodingWorkspaceEngineError::ProviderStream(code)
                    if code == "coding_gate_extra_context_required"
            ),
            "空 context 必须报 coding_gate_extra_context_required，实际: {error:?}"
        );
    }

    assert_eq!(
        store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("open gates")
            .len(),
        1,
        "失败后门必须保持 Open"
    );
    assert_eq!(
        store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("attempt")
            .status,
        CodingAttemptStatus::Blocked,
        "失败后 attempt 必须保持 Blocked"
    );
    drop(root);
}

/// `extra_context` >2000 字符 → `coding_gate_extra_context_too_long`，不写
/// note；恰 2000 字符放行（前端同一上限的双重校验）。
#[tokio::test]
async fn retry_coding_with_context_rejects_overlong_context() {
    let (root, store, attempt) = running_attempt_with_worktree();
    init_test_git_repo(attempt.worktree_path.as_ref().unwrap());
    let (root, store, gate_id) = land_operational_blocker_gate(root, store, attempt.clone()).await;
    let (tx, _rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let error = engine
        .handle_blocked_gate_response(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &gate_id,
            RETRY_WITH_CONTEXT,
            Some("长".repeat(2001)),
        )
        .await
        .expect_err("overlong context must be rejected");
    assert!(
        matches!(
            &error,
            CodingWorkspaceEngineError::ProviderStream(code)
                if code == "coding_gate_extra_context_too_long"
        ),
        "超长 context 必须报 coding_gate_extra_context_too_long，实际: {error:?}"
    );
    assert!(
        store
            .list_context_notes(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("context notes")
            .is_empty(),
        "超长拒绝不得写 note"
    );
    assert_eq!(
        store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("open gates")
            .len(),
        1,
        "超长拒绝后门必须保持 Open"
    );

    // 恰 2000 字符放行（边界含端点）。
    engine
        .handle_blocked_gate_response(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &gate_id,
            RETRY_WITH_CONTEXT,
            Some("好".repeat(2000)),
        )
        .await
        .expect("boundary 2000 chars must pass");
    drop(root);
}

/// 动作注册表：`coding_gate_action_for_id` 必须有 `retry_coding_with_context`
/// 臂（label 固定），且不影响既有动作。
#[test]
fn coding_gate_action_for_id_registers_retry_with_context() {
    let action = coding_gate_action_for_id(RETRY_WITH_CONTEXT).expect("action arm");
    assert_eq!(action.action_id, RETRY_WITH_CONTEXT);
    assert_eq!(action.label, "我来说明情况，让 AI 继续");
    assert_eq!(
        serde_json::to_string(&action.action_type).expect("serialized action type"),
        "\"retry_coding_with_context\""
    );
    // 既有臂不受影响。
    assert!(coding_gate_action_for_id("retry_coding").is_some());
    assert!(coding_gate_action_for_id("abort").is_some());
    assert!(coding_gate_action_for_id("provide_context").is_some());
}
