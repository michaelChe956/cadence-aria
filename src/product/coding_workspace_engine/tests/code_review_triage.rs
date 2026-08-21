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
