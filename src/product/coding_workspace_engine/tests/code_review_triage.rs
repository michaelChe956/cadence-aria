//! Code review triage gate regression tests.
//!
//! 这些测试覆盖 OpenSpec 变更 `open-code-review-triage-gate` 的 spec requirements：
//! - requirement 1: StopForHumanTriage 落 blocked gate（reason_code=
//!   `code_review_output_human_triage`）。
//! - requirement 2: RetryVerification 落 blocked gate（reason_code=
//!   `code_review_verification_incomplete`）。
//! - requirement 3: OpenOperationalGate 落 blocked gate（reason_code=
//!   `code_review_operational_blocker`）。
//! - requirement 4（change `triage-gate-actions-by-verdict`）：三类 gate 的动作集合
//!   按该 attempt 最新一条 Code Review 结论的 verdict 派生——基础恒为
//!   `[retry_review, manual_continue, abort]`，仅当 verdict ∈
//!   {`request_changes`, `blocked`} 时额外含 `send_to_coder`（可返修结论下四项）；
//!   读不到 verdict 时按三项 fail-closed，门仍正常落地。
//! - requirement 6: 互斥——同一 stage 不得 double-gate。
//!
//! 生产实现按 `code_review_flow_decision` 的分诊结果创建门禁；这些测试确保
//! StopForHumanTriage、RetryVerification 与 OpenOperationalGate 不会回退为静默退出。

use super::*;

/// 构造一个「implementation_defect 携带非空 plan_defect_evidence」的 code review
/// provider 输出。
///
/// 依据 `validate_plan_defect_finding`（`plan_defect.rs`），`ImplementationDefect`
/// 一旦携带任何 plan_defect 字段（这里塞了非空 `plan_defect_evidence`，同时给出
/// `recommended_route=coder_rework`）即校验失败，整份 report 因此被
/// `code_review_flow_decision` 判定为 `StopForHumanTriage`。此路径不依赖
/// reviewer_projection 的 blocker_routing，可由 `running_attempt_with_worktree()`
/// （WorkItem scope，projection 为空）直接构造。
///
/// 注意：provider 输出的 finding 用 `evidence` 数组承载证据条目，
/// `RawReviewEvidence`（`review_parser.rs`）是 `untagged` enum，对象形式的条目
/// 会被反序列化为 `Canonical(PlanDefectEvidence)` 并填入 `ReviewFinding::
/// plan_defect_evidence`。直接写 `plan_defect_evidence` 字段名不会被解析
/// （`RawReviewFinding` 无该字段且不 `deny_unknown_fields`）。
fn implementation_finding_with_plan_defect_fields() -> serde_json::Value {
    serde_json::json!({
        "verdict": "request_changes",
        "summary": "需要返修",
        "findings": [{
            "severity": "error",
            "file_path": "src/lib.rs",
            "line": 1,
            "message": "实现缺少必需的错误处理",
            "required_action": "补齐错误处理",
            "source_stage": "code_review",
            "defect_class": "implementation_defect",
            "recommended_route": "coder_rework",
            "evidence": [{
                "kind": "manual_check",
                "source_ref": "src/lib.rs",
                "message": "错误分支未覆盖"
            }]
        }]
    })
}

/// 用给定 provider 输出跑一次 `execute_code_review_with_commands`，返回 store 与
/// 持久化后的 attempt。
///
/// 复用父级 `tests` 模块的 `running_attempt_with_worktree()` + `init_test_git_repo()`
/// 公共夹具；Provider 用 `CapturingProjectionProvider`（`provider_execution_context.rs`）
/// 直接吐出给定 JSON。
///
/// 注意：返回的 `tempfile::TempDir` 必须由调用方持有到测试结束——它绑定的是
/// `running_attempt_with_worktree()` 的临时根目录，一旦 drop 会清空 store 落盘的
/// 所有记录，导致后续断言读到空数据。
async fn run_code_review_with_provider_output(
    output: serde_json::Value,
) -> (
    tempfile::TempDir,
    crate::product::coding_attempt_store::CodingAttemptStore,
    CodingExecutionAttempt,
) {
    let (root, store, attempt) = running_attempt_with_worktree();
    init_test_git_repo(attempt.worktree_path.as_ref().unwrap());
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(
        store.clone(),
        crate::product::git_workspace_service::GitWorkspaceService::new(),
        tx,
    );
    let provider =
        super::provider_execution_context::CapturingProjectionProvider::new(output.to_string());
    let (_cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel(1);
    engine
        .execute_code_review_with_commands(&attempt, &provider, &mut cmd_rx)
        .await
        .unwrap();
    let persisted = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    (root, store, persisted)
}

#[tokio::test]
async fn code_review_accepts_routing_receipt_and_sentinel_payload() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    init_test_git_repo(attempt.worktree_path.as_ref().unwrap());
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(
        store.clone(),
        crate::product::git_workspace_service::GitWorkspaceService::new(),
        tx,
    );
    let provider =
        super::provider_execution_context::CapturingProjectionProvider::new_sentinel_payload(
            "工作流路由：阶段=只读代码审查；Change=本次改动；Plan=work_item_0001；必调 Skill=requesting-code-review。\n\
         {\"verdict\":\"approve\",\"summary\":\"sentinel review complete\",\"findings\":[]}",
        );
    let (_cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel(1);

    let report = engine
        .execute_code_review_with_commands(&attempt, &provider, &mut cmd_rx)
        .await
        .expect("sentinel payload should produce report");

    assert_eq!(report.verdict, ReviewVerdict::Approve);
    assert_eq!(report.summary, "sentinel review complete");
    let input = provider.input();
    let _contract = input
        .structured_output_contract
        .expect("code review structured output contract");
    assert!(input
        .prompt
        .ends_with("</ARIA_STRUCTURED_OUTPUT>\n- 不得输出 Markdown fence 包裹 JSON；最终结论的 JSON 必须是合法对象。\n"));
}

#[tokio::test]
async fn stop_for_human_triage_lands_blocked_gate_with_review_actions() {
    let (_root, store, attempt) =
        run_code_review_with_provider_output(implementation_finding_with_plan_defect_fields())
            .await;

    // 分诊决策必须使 attempt 进入 Blocked，并落下对应的 gate。
    assert_eq!(
        attempt.status,
        CodingAttemptStatus::Blocked,
        "StopForHumanTriage 必须把 attempt 转为 Blocked"
    );

    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    assert_eq!(gates.len(), 1, "StopForHumanTriage 必须且只能落一个 gate");
    assert_eq!(
        gates[0].reason_code.as_deref(),
        Some("code_review_output_human_triage"),
    );

    let mut action_ids: Vec<&str> = gates[0]
        .available_actions
        .iter()
        .map(|action| action.action_id.as_str())
        .collect();
    action_ids.sort();
    assert_eq!(
        action_ids,
        vec!["abort", "manual_continue", "retry_review", "send_to_coder"],
        "可返修结论（request_changes）下 code review triage gate 动作集合为四项",
    );
    assert_eq!(
        gates[0].latest_review_verdict,
        Some(ReviewVerdict::RequestChanges),
        "门必须暴露派生所用的最新 verdict 事实",
    );
}

/// `RetryVerification`（verification_incomplete）门禁测试。
///
/// verification_incomplete finding 必须通过 `validate_plan_defect_finding` 的
/// 完整契约校验，包括与 `reviewer_projection.blocker_routing` 对齐（reason_code +
/// recommended_route 匹配 blocker rule）。`running_attempt_with_worktree()` 构造的
/// WorkItem scope attempt 没有 plan lineage / projection bundle（projection 为空，
/// blocker_routing 为空），任何非 implementation finding 都会因找不到 blocker rule
/// 而 validate 失败、反而变成 StopForHumanTriage，无法触发 RetryVerification。
///
/// 因此本测试复用 `provider_execution_context.rs` 的完整 coding→projection 链路：
/// 先建 WorkItemGroup scope attempt + `seed_group_attempt_fixture`（预置含
/// `verification_incomplete` → `VerificationRetry` blocker rule 的 projection bundle），
/// 再 `execute_coding` 产出 unit run 并绑定 projection bundle，最后
/// `execute_code_review` 用 verification_incomplete finding 跑分诊。
#[tokio::test]
async fn retry_verification_lands_blocked_gate_with_review_actions() {
    let (root, store, coded) = run_group_attempt_through_coding().await;
    let worktree = root.path().join("worktree");
    std::fs::write(worktree.join("reviewed.rs"), "pub fn reviewed() {}\n").unwrap();

    let output = serde_json::json!({
        "verdict": "request_changes",
        "summary": "验证证据不完整",
        "findings": [{
            "severity": "error",
            "file_path": "src/lib.rs",
            "line": 1,
            "message": "缺少测试执行证据",
            "required_action": "补交红灯执行记录",
            "source_stage": "code_review",
            "defect_class": "verification_incomplete",
            "reason_code": "verification_incomplete",
            "recommended_route": "verification_retry",
            "confidence": "high",
            "evidence": [{
                "kind": "test_execution",
                "source_ref": "provider-managed-unit.log",
                "message": "验证证据缺失"
            }]
        }]
    })
    .to_string();
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(
        store.clone(),
        crate::product::git_workspace_service::GitWorkspaceService::new(),
        tx,
    );
    let provider = super::provider_execution_context::CapturingProjectionProvider::new(output);
    let (_cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel(1);
    engine
        .execute_code_review_with_commands(&coded, &provider, &mut cmd_rx)
        .await
        .unwrap();

    let attempt = store
        .get_attempt(&coded.project_id, &coded.issue_id, &coded.id)
        .unwrap();
    assert_eq!(
        attempt.status,
        CodingAttemptStatus::Blocked,
        "RetryVerification 必须把 attempt 转为 Blocked",
    );

    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    assert_eq!(gates.len(), 1, "RetryVerification 必须且只能落一个 gate");
    assert_eq!(
        gates[0].reason_code.as_deref(),
        Some("code_review_verification_incomplete"),
    );

    let mut action_ids: Vec<&str> = gates[0]
        .available_actions
        .iter()
        .map(|action| action.action_id.as_str())
        .collect();
    action_ids.sort();
    assert_eq!(
        action_ids,
        vec!["abort", "manual_continue", "retry_review", "send_to_coder"],
        "可返修结论（request_changes）下 code review triage gate 动作集合为四项",
    );
    assert_eq!(
        gates[0].latest_review_verdict,
        Some(ReviewVerdict::RequestChanges),
        "门必须暴露派生所用的最新 verdict 事实",
    );
}

/// 构造 WorkItemGroup scope attempt，走完整 coding→projection 链路，返回
/// (`TempDir`, `store`, `coded_attempt`)。
///
/// 复用 `provider_execution_context.rs` 验证过的夹具模式：`seed_group_attempt_fixture`
/// 预置含完整 blocker_routing（包括 `verification_incomplete` → `VerificationRetry`）
/// 的 projection bundle；`execute_coding` 用 `current_plan_defect_finding()` 拼出的
/// coder plan-defect 输出产出 unit run 并绑定 projection bundle，使后续
/// `execute_code_review` 能拿到非空的 `reviewer_projection.blocker_routing`。
///
/// 调用方必须持有返回的 `TempDir` 到测试结束。
async fn run_group_attempt_through_coding() -> (
    tempfile::TempDir,
    crate::product::coding_attempt_store::CodingAttemptStore,
    CodingExecutionAttempt,
) {
    let root = tempdir().expect("tempdir");
    let worktree = root.path().join("worktree");
    std::fs::create_dir_all(&worktree).expect("worktree dir");
    init_test_git_repo(&worktree);
    let head = git_stdout(&worktree, &["rev-parse", "HEAD"]);
    let store = crate::product::coding_attempt_store::CodingAttemptStore::new(
        crate::product::app_paths::ProductAppPaths::new(root.path().join(".aria")),
    );
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: head.clone(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree.clone()),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
            start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
        })
        .unwrap();
    seed_group_attempt_fixture(&store, &attempt, true, false);
    let mut attempt = store
        .seed_running_attempt_for_test(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    attempt.head_commit = Some(head.clone());
    attempt.stage = CodingExecutionStage::Coding;
    store.write_coding_attempt_for_test(&attempt).unwrap();
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(
        store.clone(),
        crate::product::git_workspace_service::GitWorkspaceService::new(),
        tx,
    );
    let coder_output = serde_json::json!({
        "plan_defect_findings": [
            super::provider_execution_context::current_plan_defect_finding()
        ]
    })
    .to_string();
    let coder = super::provider_execution_context::CapturingProjectionProvider::new(coder_output);
    let coded = engine
        .execute_coding(&attempt, &coder, &CodingExecutionContext::default())
        .await
        .unwrap();
    (root, store, coded)
}

/// `OpenOperationalGate`（operational_blocker）门禁测试。
///
/// 复用完整 coding→projection 链路，使 operational finding 能与权威
/// `operational_blocker` → `OperationalGate` blocker rule 对齐。
#[tokio::test]
async fn open_operational_gate_lands_blocked_gate_with_review_actions() {
    let (root, store, coded) = run_group_attempt_through_coding().await;
    let worktree = root.path().join("worktree");
    std::fs::write(worktree.join("reviewed.rs"), "pub fn reviewed() {}\n").unwrap();

    let output = serde_json::json!({
        "verdict": "request_changes",
        "summary": "运行环境阻塞",
        "findings": [{
            "severity": "error",
            "file_path": "src/lib.rs",
            "line": 1,
            "message": "所需 provider 当前不可用",
            "required_action": "恢复 provider 可用性后重试",
            "source_stage": "code_review",
            "defect_class": "operational_blocker",
            "reason_code": "operational_blocker",
            "recommended_route": "operational_gate",
            "confidence": "high",
            "evidence": []
        }]
    })
    .to_string();
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(
        store.clone(),
        crate::product::git_workspace_service::GitWorkspaceService::new(),
        tx,
    );
    let provider = super::provider_execution_context::CapturingProjectionProvider::new(output);
    let (_cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel(1);
    engine
        .execute_code_review_with_commands(&coded, &provider, &mut cmd_rx)
        .await
        .unwrap();

    let attempt = store
        .get_attempt(&coded.project_id, &coded.issue_id, &coded.id)
        .unwrap();
    assert_eq!(attempt.status, CodingAttemptStatus::Blocked);
    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    assert_eq!(gates.len(), 1);
    assert_eq!(
        gates[0].reason_code.as_deref(),
        Some("code_review_operational_blocker")
    );
    assert_eq!(
        gates[0]
            .available_actions
            .iter()
            .map(|action| action.action_id.as_str())
            .collect::<Vec<_>>(),
        vec!["retry_review", "send_to_coder", "manual_continue", "abort"],
        "可返修结论（request_changes）下动作顺序仍为 retry → send_to_coder → manual_continue → abort",
    );
    assert_eq!(
        gates[0].latest_review_verdict,
        Some(ReviewVerdict::RequestChanges),
        "门必须暴露派生所用的最新 verdict 事实",
    );
}

/// 互斥回归测试：`verdict=blocked` 且无可执行 finding 时，必须只落既有的
/// `code_review_blocked` gate，不得因为新的分诊 gate 逻辑而 double-gate。
///
/// 当前实现已覆盖此行为（`code_review.rs` 中 `Blocked && !actionable` 落
/// `code_review_blocked`），本测试作为回归保护，确保 Task 2 落地分诊 gate 时
/// 不会与既有 `code_review_blocked` 重复落 gate。
#[tokio::test]
async fn blocked_verdict_without_actionable_findings_lands_only_code_review_blocked_gate() {
    let (_root, store, attempt) = run_code_review_with_provider_output(serde_json::json!({
        "verdict": "blocked",
        "summary": "被阻塞",
        "findings": []
    }))
    .await;

    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    assert_eq!(gates.len(), 1, "不得 double-gate");
    assert_eq!(gates[0].reason_code.as_deref(), Some("code_review_blocked"),);
    assert!(
        gates
            .iter()
            .all(|gate| gate.reason_code.as_deref() != Some("code_review_output_human_triage")),
        "不得与 code_review_output_human_triage 分诊 gate 重复",
    );
}

/// 分诊门禁的 `send_to_coder` 必须走代码审查反馈返修路径（`send_code_review_
/// feedback_to_coder`），而不是审查轮次超限路径（`send_review_limit_feedback_
/// to_coder`，后者在 `rework_count < max_auto_rework` 时直接报错）。
///
/// 复用 `run_code_review_with_provider_output`（返回 `(TempDir, store, attempt)`，
/// TempDir 必须由调用方持有）落地 `code_review_output_human_triage` 门禁，再通过
/// 引擎 gate 响应入口 `handle_blocked_gate_response`（`gates.rs:479`，async，签名
/// `(project_id, issue_id, attempt_id, gate_id, action_id, extra_context)`）执行
/// `send_to_coder`（带 operator_context）。
///
/// 夹具的 provider 输出 `verdict=request_changes`（见
/// `implementation_finding_with_plan_defect_fields`），所以本测试同时覆盖
/// `rework.rs:456` 的 verdict 前置必须接受 `RequestChanges`。
#[tokio::test]
async fn triage_gate_send_to_coder_routes_request_changes_to_coder_rework() {
    let (_root, store, attempt) =
        run_code_review_with_provider_output(implementation_finding_with_plan_defect_fields())
            .await;
    let gate_id = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap()
        .into_iter()
        .next()
        .unwrap()
        .gate_id;
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(
        store.clone(),
        crate::product::git_workspace_service::GitWorkspaceService::new(),
        tx,
    );
    let updated = engine
        .handle_blocked_gate_response(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &gate_id,
            "send_to_coder",
            Some("请按审查结论返修".to_string()),
        )
        .await
        .unwrap();
    assert_eq!(updated.stage, CodingExecutionStage::Coding);
    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.rework_count, attempt.rework_count + 1);
}

/// 分诊门禁 `send_to_coder` 复用 `send_code_review_feedback_to_coder` 的不变量：
/// 必须提供非空 operator_context，否则拒绝并保持 Blocked。
///
/// 该拒绝语义来自 `send_code_review_feedback_to_coder`（`rework.rs:436`）对
/// `operator_context` 的强制要求；本 change 复用该不变量，不改它。
#[tokio::test]
async fn triage_gate_send_to_coder_without_operator_context_is_rejected() {
    let (_root, store, attempt) =
        run_code_review_with_provider_output(implementation_finding_with_plan_defect_fields())
            .await;
    let gate_id = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap()
        .into_iter()
        .next()
        .unwrap()
        .gate_id;
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(
        store.clone(),
        crate::product::git_workspace_service::GitWorkspaceService::new(),
        tx,
    );
    let result = engine
        .handle_blocked_gate_response(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &gate_id,
            "send_to_coder",
            None,
        )
        .await;
    assert!(
        result.is_err(),
        "must reject send_to_coder without operator context"
    );
    let persisted = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    assert_eq!(
        persisted.status,
        CodingAttemptStatus::Blocked,
        "must stay blocked"
    );
}

/// C2 Task 1（REQ-CRO-01）：完成收尾 durable-first。`CodeReviewComplete`
/// 观察事件必须在全部完成事实（artifact／report／snapshot／role run 状态／
/// timeline 完成）落盘**之后**才能发射——`complete_timeline_node` 先写
/// `update_timeline_node_status` 再发自己的事件，因此观察者先收到
/// `CodingTimelineNodeUpdated{Completed}` 即证明 durable 写面已成功。
///
/// 容量 1 的通道让引擎在每次 `reserve()` 处停等，观察者逐事件消费；当前
/// 实现（事件先于 `complete_timeline_node`/`update_role_run_status`）会在
/// `CodeReviewComplete` 到达时仍未见过 Completed 事件，断言失败。
#[tokio::test]
async fn code_review_complete_persists_facts_before_emitting_events() {
    let (root, store, attempt) = running_attempt_with_worktree();
    init_test_git_repo(attempt.worktree_path.as_ref().unwrap());
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let engine = CodingWorkspaceEngine::new(
        store.clone(),
        crate::product::git_workspace_service::GitWorkspaceService::new(),
        tx,
    );
    let provider = super::provider_execution_context::CapturingProjectionProvider::new(
        serde_json::json!({
            "verdict": "approve",
            "summary": "断连窗口补读",
            "findings": []
        })
        .to_string(),
    );
    let (_cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel(1);

    let store_for_observer = store.clone();
    let identity = (
        attempt.project_id.clone(),
        attempt.issue_id.clone(),
        attempt.id.clone(),
    );
    let handle = tokio::spawn(async move {
        engine
            .execute_code_review_with_commands(&attempt, &provider, &mut cmd_rx)
            .await
            .expect("review completes while observer drains slowly")
    });

    let mut saw_node_completed_before_complete = false;
    while let Some(event) = rx.recv().await {
        match event {
            CodingWsOutMessage::CodingTimelineNodeUpdated {
                node_id, status, ..
            } => {
                if status == CodingTimelineNodeStatus::Completed {
                    // 事件仅在 store 写面成功后发射；直接复核 durable 状态。
                    let nodes = store_for_observer
                        .get_timeline_nodes(&identity.0, &identity.1, &identity.2)
                        .expect("timeline nodes");
                    let node = nodes
                        .iter()
                        .find(|node| node.id == node_id)
                        .expect("persisted node");
                    assert_eq!(node.status, CodingTimelineNodeStatus::Completed);
                    saw_node_completed_before_complete = true;
                }
            }
            CodingWsOutMessage::CodeReviewComplete { .. } => {
                assert!(
                    saw_node_completed_before_complete,
                    "CodeReviewComplete 只能在 timeline 完成事实持久化之后发射"
                );
                // 观察点复核：role run 状态与 completion checkpoint（报告＋原始输出）
                // 均已 durable，驾驶舱重连可补读到同一结果。
                let run = store_for_observer
                    .latest_role_run(
                        &identity.0,
                        &identity.1,
                        &identity.2,
                        CodingExecutionStage::CodeReview,
                        CodingProviderRole::CodeReviewer,
                    )
                    .expect("role runs")
                    .expect("role run exists");
                assert_eq!(run.status, CodingRoleRunStatus::Completed);
                assert!(!run.raw_provider_output_refs.is_empty());
                let reports = store_for_observer
                    .list_code_review_reports(&identity.0, &identity.1, &identity.2)
                    .expect("reports");
                assert_eq!(
                    reports.len(),
                    1,
                    "completion checkpoint persisted exactly once"
                );
                assert!(
                    reports[0]
                        .raw_provider_output_ref
                        .as_deref()
                        .is_some_and(|reference| !reference.is_empty())
                );
            }
            _ => {}
        }
    }
    let report = handle.await.expect("engine task joins");
    assert_eq!(report.verdict, ReviewVerdict::Approve);

    // 全量复核：断连观察不改变业务事实，恰好一次 reviewer 运行。
    let runs = store
        .list_role_runs(&identity.0, &identity.1, &identity.2)
        .expect("role runs");
    assert_eq!(
        runs.iter()
            .filter(|run| run.role == CodingProviderRole::CodeReviewer)
            .count(),
        1,
        "disconnection must not spawn a second reviewer run"
    );
    drop(root);
}

/// C2 Task 1（REQ-CRO-01）：观察通道全失效（无任何 WS 订阅者，发送侧
/// reserve 立即失败）时，业务结果与 attempt 状态照常持久化——不产生失败
/// 或中止终态，驾驶舱经 GET 补读到同一结果。
#[tokio::test]
async fn code_review_completion_survives_disconnected_observers() {
    let (root, store, attempt) = running_attempt_with_worktree();
    init_test_git_repo(attempt.worktree_path.as_ref().unwrap());
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    drop(rx);
    let engine = CodingWorkspaceEngine::new(
        store.clone(),
        crate::product::git_workspace_service::GitWorkspaceService::new(),
        tx,
    );
    let provider = super::provider_execution_context::CapturingProjectionProvider::new(
        serde_json::json!({
            "verdict": "approve",
            "summary": "无订阅者补读",
            "findings": []
        })
        .to_string(),
    );
    let (_cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel(1);

    let report = engine
        .execute_code_review_with_commands(&attempt, &provider, &mut cmd_rx)
        .await
        .expect("business result must not depend on observers");

    let persisted = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("attempt persisted");
    assert_eq!(persisted.status, CodingAttemptStatus::Running);
    let reports = store
        .list_code_review_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("reports");
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].id, report.id);
    let run = store
        .latest_role_run(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
        )
        .expect("role runs")
        .expect("role run exists");
    assert_eq!(run.status, CodingRoleRunStatus::Completed);
    drop(root);
}

// ─── change `triage-gate-actions-by-verdict`：动作集合按最新 verdict 派生 ───

/// 直接落一条 Code Review 结论（派生动作集合的事实源），不跑 provider。
///
/// 派生只在建门时发生，读取口径与 `send_code_review_feedback_to_coder`
/// （`rework.rs`）一致：同一 attempt 结论集合的最后一条。本 helper 只负责把事实
/// 写进 store，读法由生产代码复用同一口径。
fn seed_review_conclusion(
    store: &crate::product::coding_attempt_store::CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    id: &str,
    verdict: ReviewVerdict,
) {
    store
        .save_code_review_report(
            attempt,
            &CodeReviewReport {
                id: id.to_string(),
                attempt_id: attempt.id.clone(),
                round: 1,
                verdict,
                findings: Vec::new(),
                tested_evidence_refs: Vec::new(),
                diff_refs: Vec::new(),
                summary: "seeded review conclusion".to_string(),
                created_at: "2026-10-25T00:00:00Z".to_string(),
                raw_provider_output_ref: None,
                role_run_id: None,
                run_no: None,
                unit_run_id: None,
                acceptance: None,
                review_window_id: None,
            },
        )
        .expect("seed code review report");
}

fn triage_test_engine(
    store: &crate::product::coding_attempt_store::CodingAttemptStore,
) -> CodingWorkspaceEngine {
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    CodingWorkspaceEngine::new(
        store.clone(),
        crate::product::git_workspace_service::GitWorkspaceService::new(),
        tx,
    )
}

/// 分诊门只可能在 CodeReview 阶段落地；把测试夹具的 attempt 置到该阶段。
fn attempt_at_code_review(
    store: &crate::product::coding_attempt_store::CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> CodingExecutionAttempt {
    store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("code review stage")
}

/// 走真实建门路径落一个 Code Review 分诊门。
async fn create_triage_gate(
    engine: &CodingWorkspaceEngine,
    attempt: &CodingExecutionAttempt,
    reason_code: &'static str,
    node_id: &str,
) -> CodingExecutionAttempt {
    engine
        .create_review_blocked_gate(ReviewBlockedGateInput {
            attempt,
            node_id,
            stage: CodingExecutionStage::CodeReview,
            role: CodingProviderRole::CodeReviewer,
            title: "Code Review 分诊".to_string(),
            description: "按最新 verdict 派生动作集合".to_string(),
            reason_code,
            evidence_refs: Vec::new(),
            raw_provider_output_ref: None,
        })
        .await
        .expect("create review blocked gate")
}

fn open_triage_gate(
    store: &crate::product::coding_attempt_store::CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> crate::product::coding_models::CodingGateRequired {
    let mut gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("open gates");
    assert_eq!(gates.len(), 1, "必须且只能落一个门");
    gates.remove(0)
}

fn gate_action_ids(gate: &crate::product::coding_models::CodingGateRequired) -> Vec<String> {
    gate.available_actions
        .iter()
        .map(|action| action.action_id.clone())
        .collect()
}

fn provider_stream_code(error: &CodingWorkspaceEngineError) -> String {
    match error {
        CodingWorkspaceEngineError::ProviderStream(code) => code.clone(),
        other => panic!("expected provider stream error, got {other:?}"),
    }
}

/// 最新 verdict 不在可返修集合（`approve`）时，三个分诊门各自派生恰好三项，
/// 且门 payload 暴露派生所用 verdict 事实；读路径（`normalize_blocked_gate`）
/// 不得把 `send_to_coder` 补回去。
#[tokio::test]
async fn triage_gates_derive_three_actions_when_latest_verdict_is_approve() {
    for reason_code in CODE_REVIEW_TRIAGE_REASON_CODES {
        let (_root, store, attempt) = running_attempt_with_worktree();
        let attempt = attempt_at_code_review(&store, &attempt);
        seed_review_conclusion(
            &store,
            &attempt,
            "code_review_report_0001",
            ReviewVerdict::Approve,
        );
        let engine = triage_test_engine(&store);
        let updated = create_triage_gate(&engine, &attempt, reason_code, "coding_node_0001").await;
        assert_eq!(
            updated.status,
            CodingAttemptStatus::Blocked,
            "{reason_code}: 门必须正常落地"
        );

        let gate = open_triage_gate(&store, &updated);
        assert_eq!(gate.reason_code.as_deref(), Some(reason_code));
        assert_eq!(
            gate_action_ids(&gate),
            vec!["retry_review", "manual_continue", "abort"],
            "{reason_code}: approve 只派生三项（不含 send_to_coder）",
        );
        assert_eq!(
            gate.latest_review_verdict,
            Some(ReviewVerdict::Approve),
            "{reason_code}: 门必须暴露派生所用 verdict 事实",
        );

        // 读路径只做归一，不重新派生：再读一次仍是三项。
        let reread = open_triage_gate(&store, &updated);
        assert_eq!(
            gate_action_ids(&reread),
            vec!["retry_review", "manual_continue", "abort"],
            "{reason_code}: 读路径不得补回 send_to_coder",
        );
    }
}

/// 该 attempt 没有任何 Code Review 结论时 fail-closed：动作集合恰好三项、不虚构
/// verdict 事实、门仍可操作（`retry_review` 能收口并回到 CodeReview）。
#[tokio::test]
async fn triage_gate_without_code_review_report_fails_closed_to_three_actions() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let attempt = attempt_at_code_review(&store, &attempt);
    let engine = triage_test_engine(&store);
    let updated = create_triage_gate(
        &engine,
        &attempt,
        "code_review_output_human_triage",
        "coding_node_0001",
    )
    .await;
    assert_eq!(updated.status, CodingAttemptStatus::Blocked);

    let gate = open_triage_gate(&store, &updated);
    assert_eq!(
        gate_action_ids(&gate),
        vec!["retry_review", "manual_continue", "abort"],
        "读不到结论必须按三项 fail-closed",
    );
    assert_eq!(
        gate.latest_review_verdict, None,
        "读不到结论时不得虚构 verdict 事实",
    );

    let resumed = engine
        .handle_blocked_gate_response(
            &updated.project_id,
            &updated.issue_id,
            &updated.id,
            &gate.gate_id,
            "retry_review",
            None,
        )
        .await
        .expect("三项门必须可收口");
    assert_eq!(resumed.status, CodingAttemptStatus::Running);
    assert_eq!(resumed.stage, CodingExecutionStage::CodeReview);
    assert!(
        store
            .list_open_blocked_gates(&updated.project_id, &updated.issue_id, &updated.id)
            .expect("open gates")
            .is_empty(),
        "retry_review 必须收口该门"
    );
}

/// 结论不可解析（损坏 JSON）等同读取失败：按三项 fail-closed，门仍正常落地。
#[tokio::test]
async fn triage_gate_with_unreadable_code_review_report_fails_closed_to_three_actions() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let attempt = attempt_at_code_review(&store, &attempt);
    let reports_dir = store
        .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .join("code-reviews");
    std::fs::create_dir_all(&reports_dir).expect("reports dir");
    std::fs::write(
        reports_dir.join("code_review_report_0001.json"),
        "{ not json",
    )
    .expect("corrupt report");
    assert!(
        store
            .list_code_review_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .is_err(),
        "损坏的结论文件必须表现为读取失败",
    );

    let engine = triage_test_engine(&store);
    let updated = create_triage_gate(
        &engine,
        &attempt,
        "code_review_verification_incomplete",
        "coding_node_0001",
    )
    .await;
    let gate = open_triage_gate(&store, &updated);
    assert_eq!(
        gate_action_ids(&gate),
        vec!["retry_review", "manual_continue", "abort"],
        "结论不可解析必须按三项 fail-closed",
    );
    assert_eq!(gate.latest_review_verdict, None);
}

/// `retry_review` 产生新结论后动作集合必须重新派生，且不存在持久禁用位：
/// `approve`（三项）→ retry_review → 新结论 `request_changes` → 新门四项。
#[tokio::test]
async fn retry_review_rederives_triage_actions_from_new_conclusion() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let attempt = attempt_at_code_review(&store, &attempt);
    seed_review_conclusion(
        &store,
        &attempt,
        "code_review_report_0001",
        ReviewVerdict::Approve,
    );
    let engine = triage_test_engine(&store);
    let gated = create_triage_gate(
        &engine,
        &attempt,
        "code_review_output_human_triage",
        "coding_node_0001",
    )
    .await;
    let first_gate = open_triage_gate(&store, &gated);
    assert_eq!(
        gate_action_ids(&first_gate),
        vec!["retry_review", "manual_continue", "abort"],
    );

    let resumed = engine
        .handle_blocked_gate_response(
            &gated.project_id,
            &gated.issue_id,
            &gated.id,
            &first_gate.gate_id,
            "retry_review",
            None,
        )
        .await
        .expect("retry_review 必须收口旧门");
    assert_eq!(resumed.status, CodingAttemptStatus::Running);

    // 新一轮 Code Review 落地后的结论（本用例固定派生输入；provider→报告的链路
    // 已由 `stop_for_human_triage_*` 等既有用例覆盖）。
    seed_review_conclusion(
        &store,
        &resumed,
        "code_review_report_0002",
        ReviewVerdict::RequestChanges,
    );
    let regated = create_triage_gate(
        &engine,
        &resumed,
        "code_review_output_human_triage",
        "coding_node_0002",
    )
    .await;
    let second_gate = open_triage_gate(&store, &regated);
    assert_eq!(
        gate_action_ids(&second_gate),
        vec!["retry_review", "send_to_coder", "manual_continue", "abort"],
        "新结论可返修时必须重新派生为四项（不得永久缺失该动作）",
    );
    assert_eq!(
        second_gate.latest_review_verdict,
        Some(ReviewVerdict::RequestChanges),
    );
}

/// 同 attempt／node／stage／reason_code 命中 store 去重路径时，动作集合与派生事实
/// 都按新结论覆盖（`create_blocked_gate_unlocked` 既有覆盖行为），不残留上一轮
/// verdict 或动作。
#[tokio::test]
async fn triage_gate_dedupe_path_rederives_actions_and_verdict() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let attempt = attempt_at_code_review(&store, &attempt);
    seed_review_conclusion(
        &store,
        &attempt,
        "code_review_report_0001",
        ReviewVerdict::Approve,
    );
    let engine = triage_test_engine(&store);
    let gated = create_triage_gate(
        &engine,
        &attempt,
        "code_review_output_human_triage",
        "coding_node_0001",
    )
    .await;
    let first = open_triage_gate(&store, &gated);
    assert_eq!(
        gate_action_ids(&first),
        vec!["retry_review", "manual_continue", "abort"],
    );

    seed_review_conclusion(
        &store,
        &attempt,
        "code_review_report_0002",
        ReviewVerdict::RequestChanges,
    );
    let regated = create_triage_gate(
        &engine,
        &attempt,
        "code_review_output_human_triage",
        "coding_node_0001",
    )
    .await;
    let second = open_triage_gate(&store, &regated);
    assert_eq!(second.gate_id, first.gate_id, "同键必须命中既有 open 门");
    assert_eq!(
        gate_action_ids(&second),
        vec!["retry_review", "send_to_coder", "manual_continue", "abort"],
        "去重路径必须按新结论覆盖动作集合",
    );
    assert_eq!(
        second.latest_review_verdict,
        Some(ReviewVerdict::RequestChanges),
        "去重路径必须覆盖派生事实，不残留上一轮 verdict",
    );
}

/// 集合不含 `send_to_coder` 时，脚本/陈旧快照直发该动作仍必须被服务端拒绝：
/// 可观察码 `send_to_coder_latest_review_not_actionable` 保留，不落 rework
/// instruction、不改 attempt 状态、不关门。
#[tokio::test]
async fn verdict_derived_triage_gate_rejects_direct_send_to_coder() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let attempt = attempt_at_code_review(&store, &attempt);
    seed_review_conclusion(
        &store,
        &attempt,
        "code_review_report_0001",
        ReviewVerdict::Approve,
    );
    let engine = triage_test_engine(&store);
    let gated = create_triage_gate(
        &engine,
        &attempt,
        "code_review_output_human_triage",
        "coding_node_0001",
    )
    .await;
    let gate = open_triage_gate(&store, &gated);
    assert!(!gate_action_ids(&gate).contains(&"send_to_coder".to_string()));

    let error = engine
        .handle_blocked_gate_response(
            &gated.project_id,
            &gated.issue_id,
            &gated.id,
            &gate.gate_id,
            "send_to_coder",
            Some("请按审查结论返修".to_string()),
        )
        .await
        .expect_err("集合不含该动作时必须拒绝");
    assert_eq!(
        provider_stream_code(&error),
        "send_to_coder_latest_review_not_actionable",
    );

    assert!(
        store
            .list_rework_instructions(&gated.project_id, &gated.issue_id, &gated.id)
            .expect("rework instructions")
            .is_empty(),
        "被拒绝的提交不得落地 rework instruction",
    );
    let persisted = store
        .get_attempt(&gated.project_id, &gated.issue_id, &gated.id)
        .expect("attempt");
    assert_eq!(persisted.status, CodingAttemptStatus::Blocked);
    assert_eq!(persisted.stage, CodingExecutionStage::CodeReview);
    assert_eq!(
        open_triage_gate(&store, &gated).gate_id,
        gate.gate_id,
        "拒绝不得关闭该门",
    );
}

/// 存量在途门（升级前落了四动作、不回写）：最新 verdict 不可返修时，直发
/// `send_to_coder` 仍被既有前置拒绝，语义与派生门一致。
#[tokio::test]
async fn legacy_four_action_triage_gate_still_rejects_send_to_coder() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let attempt = attempt_at_code_review(&store, &attempt);
    seed_review_conclusion(
        &store,
        &attempt,
        "code_review_report_0001",
        ReviewVerdict::Approve,
    );
    let blocked = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Blocked,
        )
        .expect("blocked");
    let gate = store
        .create_blocked_gate(
            &blocked,
            CreateBlockedGateInput {
                attempt_id: blocked.id.clone(),
                stage: CodingExecutionStage::CodeReview,
                node_id: Some("coding_node_0001".to_string()),
                role: Some(CodingProviderRole::CodeReviewer),
                title: "Code Review 结论需人工分诊".to_string(),
                description: "升级前落地的四动作门".to_string(),
                reason_code: Some("code_review_output_human_triage".to_string()),
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
        .expect("legacy four action gate");
    // 不回写：存量门没有派生事实字段，动作集合保持落地时的样子。
    assert_eq!(gate.latest_review_verdict, None);

    let engine = triage_test_engine(&store);
    let error = engine
        .handle_blocked_gate_response(
            &blocked.project_id,
            &blocked.issue_id,
            &blocked.id,
            &gate.gate_id,
            "send_to_coder",
            Some("请按审查结论返修".to_string()),
        )
        .await
        .expect_err("verdict 不可返修时存量门也必须拒绝");
    assert_eq!(
        provider_stream_code(&error),
        "send_to_coder_latest_review_not_actionable",
    );
    assert!(
        store
            .list_rework_instructions(&blocked.project_id, &blocked.issue_id, &blocked.id)
            .expect("rework instructions")
            .is_empty(),
    );
    let persisted = store
        .get_attempt(&blocked.project_id, &blocked.issue_id, &blocked.id)
        .expect("attempt");
    assert_eq!(persisted.status, CodingAttemptStatus::Blocked);
    assert_eq!(
        open_triage_gate(&store, &blocked),
        gate,
        "存量门保持原样（不回写、不关闭）",
    );
}

include!("verification_triage_entry.inc.rs");

include!("verification_triage_gates.rs");
