//! Code review triage gate regression tests.
//!
//! 这些测试覆盖 OpenSpec 变更 `open-code-review-triage-gate` 的 spec requirements：
//! - requirement 1: StopForHumanTriage 落 blocked gate（reason_code=
//!   `code_review_output_human_triage`）。
//! - requirement 2: RetryVerification 落 blocked gate（reason_code=
//!   `code_review_verification_incomplete`）。
//! - requirement 3: OpenOperationalGate 落 blocked gate（reason_code=
//!   `code_review_operational_blocker`）。
//! - requirement 4: 三类 gate 的动作集合均为
//!   `[retry_review, send_to_coder, manual_continue, abort]`。
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
        "code review triage gate 动作集合必须为四项",
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
        "code review triage gate 动作集合必须为四项",
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
        vec!["retry_review", "send_to_coder", "manual_continue", "abort"]
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
                assert!(reports[0]
                    .raw_provider_output_ref
                    .as_deref()
                    .is_some_and(|reference| !reference.is_empty()));
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
