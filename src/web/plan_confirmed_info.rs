//! P1 WIGA Task 8：成功 publication/compile + durable Confirmed 的只读确认
//! 信息投影（REQ-WIGA-07）。
//!
//! 不建通知表——事实只从 enrollment 精确绑定的 session/plan/compile 事务/
//! publication provenance 派生；approve 点击、compile Failed/RecoveryRequired、
//! 缺 provenance、未 Confirmed 一律 `None`，引用/文件不可读显式上抛错误，
//! 绝不报告成功。稳定 key `plan_confirmed:{plan_id}:{compile_id}`，时间取
//! 成功事务 `committed_at`（不取会变的 `updated_at`）。

use crate::product::app_paths::ProductAppPaths;
use crate::product::issue_automation_store::IssueAutomationStore;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::automation::IssueAutomationEnrollment;
use crate::product::models::lifecycle::IssueWorkItemPlanStatus;
use crate::product::models::outline::{WorkItemPlanCommitState, WorkItemPlanCompileStatus};
use crate::product::models::{SingleCandidatePhase, WorkspaceSessionStatus};
use crate::product::work_item_plan_policy::RunPolicy;
use crate::product::work_item_plan_source_store::{SourceStoreScope, WorkItemPlanSourceStore};
use crate::product::work_item_plan_store::WorkItemPlanStore;

/// 驾驶舱只读分区的固定文案：只称「Work Item Plan 已确认」，不称 coding
/// 已完成或全交付（P1 无 §3/§4 事实）。
pub const PLAN_CONFIRMED_INFO_TITLE: &str = "Work Item Plan 已确认";

/// 只读确认信息 DTO（`IssueLifecycleResponse.plan_confirmed_info` 条目）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanConfirmedInfoDto {
    pub key: String,
    pub plan_id: String,
    pub session_id: String,
    pub occurred_at: String,
    pub title: String,
}

/// 从 durable 事实派生绑定 plan 的成功确认信息；任一成功判据不满足返回
/// `None`，读取失败原样上抛（不吞为 off）。
pub fn plan_confirmed_info(
    paths: &ProductAppPaths,
    enrollment: &IssueAutomationEnrollment,
) -> Result<Option<PlanConfirmedInfoDto>, ProductStoreError> {
    // 仅当前 enabled 精确绑定 enrollment 派生；禁用不是假成功。
    if !enrollment.enabled {
        return Ok(None);
    }
    let (Some(plan_id), Some(session_id)) = (&enrollment.plan_id, &enrollment.session_id) else {
        return Ok(None);
    };
    let lifecycle = LifecycleStore::new(paths.clone());
    let session = lifecycle.get_workspace_session(session_id)?;
    if session.project_id != enrollment.project_id
        || session.issue_id != enrollment.issue_id
        || session.run_policy != RunPolicy::Interactive
        || session.status != WorkspaceSessionStatus::Confirmed
        || session.single_candidate_phase != Some(SingleCandidatePhase::Completed)
    {
        return Ok(None);
    }
    let plan = lifecycle.get_issue_work_item_plan(
        &enrollment.project_id,
        &enrollment.issue_id,
        plan_id,
    )?;
    if plan.status != IssueWorkItemPlanStatus::Confirmed {
        return Ok(None);
    }
    // 多成功事务时只认 session compile_reservation 精确绑定的那一条。
    let Some(reservation) = session.compile_reservation.as_ref() else {
        return Ok(None);
    };
    let compile_id = reservation.compile_id.clone();
    let tx = WorkItemPlanStore::new(paths.clone()).get_compile_transaction(
        &enrollment.project_id,
        &enrollment.issue_id,
        plan_id,
        &compile_id,
    )?;
    if tx.status != WorkItemPlanCompileStatus::Committed
        || tx.plan_commit_state != WorkItemPlanCommitState::Committed
        || tx.step_cursor != "committed"
        || tx.committed_at.is_none()
    {
        return Ok(None);
    }
    // tx 的 provenance ref 必须与 session 的 ref 一致，且源文件可加载验证；
    // provenance.id 即 reservation.compile_id（ref 尾段），与绑定事务互证。
    let (Some(tx_provenance_ref), Some(session_provenance_ref)) = (
        tx.publication_provenance_ref.as_deref(),
        session.publication_provenance_ref.as_deref(),
    ) else {
        return Ok(None);
    };
    if tx_provenance_ref != session_provenance_ref {
        return Ok(None);
    }
    let scope = SourceStoreScope {
        project_id: enrollment.project_id.clone(),
        issue_id: enrollment.issue_id.clone(),
        plan_id: plan_id.clone(),
    };
    let source_store = WorkItemPlanSourceStore::new(paths.clone());
    let provenance = source_store
        .get_publication_provenance(&scope, session_provenance_ref)
        .map_err(source_store_error)?;
    if provenance.plan_id != *plan_id || provenance.id != compile_id {
        return Ok(None);
    }
    Ok(Some(PlanConfirmedInfoDto {
        key: format!("plan_confirmed:{plan_id}:{compile_id}"),
        plan_id: plan_id.clone(),
        session_id: session_id.clone(),
        occurred_at: tx.committed_at.clone().unwrap_or_default(),
        title: PLAN_CONFIRMED_INFO_TITLE.to_string(),
    }))
}

/// issue lifecycle 的 additive 投影入口：只读派生当前 enabled 绑定 enrollment
/// 的确认信息（0 或 1 条）；enrollment 读取失败按 HTTP 显式错误传播。
pub fn issue_plan_confirmed_info(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
) -> Result<Vec<PlanConfirmedInfoDto>, ProductStoreError> {
    let enrollment = IssueAutomationStore::new(paths.clone()).get(project_id, issue_id)?;
    let Some(enrollment) = enrollment else {
        return Ok(Vec::new());
    };
    Ok(plan_confirmed_info(paths, &enrollment)?
        .into_iter()
        .collect())
}

// ---------------------------------------------------------------------------
// C1 Task 9（enrollment-recovery-surface）：统一 C1 恢复等待项投影。
// 只从 durable 事实派生（孤儿候选快照、lease 三态、Failed advance、intent
// 停等、换代历史）；通知/WS 失败不回滚业务事实，驾驶舱经 GET lifecycle
// 补读本投影。不建通知表，不以事件当权威（tasks.md §3.2/§4.1）。
// ---------------------------------------------------------------------------

/// C2 Task 12（REQ-CRO-06）：等待项操作上下文——与 DTO `actions` 字符串
/// 一一对应，携带稳定 `command_id` 与 expected 对象版本；REST／页面动作
/// 同源携带（旧页面过期版本 fail-closed 返回"请刷新"）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WaitingItemAction {
    pub action: String,
    pub command_id: String,
    pub expected_version: u64,
}

/// C1 恢复等待项 DTO（`IssueLifecycleResponse.c1_waiting_items` 条目）。
/// C2 Task 12 additive：`expected_version`／`action_context` 两字段
/// （`#[serde(default)]`，旧响应缺失按缺省解释）；C1 既有七种 kind 的
/// 投影内容零变化。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct C1WaitingItemDto {
    pub id: String,
    /// candidate_recovery | lease_wait | lease_takeover | lease_unknown |
    /// advance_retry_failed | intent_blocked | generation_history |
    /// C2：coding_completion_unconfirmed | coding_already_running |
    /// coding_takeover_required | coding_lease_unknown | coding_restart_available |
    /// reviewer_configuration_missing | verification_triage |
    /// policy_verification | instruction_claim_interrupted |
    /// large_candidate_blocked
    pub kind: String,
    pub reason: String,
    pub completed_steps: Vec<String>,
    pub target: Option<crate::product::logical_codebase::EnrollmentTarget>,
    pub plan_id: Option<String>,
    pub session_id: Option<String>,
    pub attempt_id: Option<String>,
    pub gate_id: Option<String>,
    pub possible_side_effect: Option<String>,
    /// recover_candidate | confirm_takeover | retry_initialization | rebind |
    /// C2：restart_coding | confirm_takeover | gate action_id（经 REST 作答）
    pub actions: Vec<String>,
    pub next_phase: Option<String>,
    /// C2 additive：操作目标对象的 expected durable 版本（无 REST 动作面
    /// 的条目为 `None`）。
    #[serde(default)]
    pub expected_version: Option<u64>,
    /// C2 additive：与 `actions` 一一对应的操作上下文（command_id＋版本）。
    #[serde(default)]
    pub action_context: Vec<WaitingItemAction>,
    /// C5 Task 6 additive：repository_initialization_failed 等待项指向的
    /// 稳定 operation id（其余 kind 为 `None`，wire 缺省）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    /// C5 Task 6 additive：初始化失败的结构化诊断（步骤/原因/provider/
    /// 变更路径/可重试），只在 `repository_initialization_failed` 在场。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<RepositoryInitializationFailureDiagnostics>,
    /// C5 Task 6 additive：project 级等待项必填；issue 级条目缺省。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// C5 Task 6 additive：issue 级关联（project 级条目缺省）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_id: Option<String>,
    /// C5 Task 6 additive：resume 链条的后继指回（最新失败链叶保留关联）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_operation_id: Option<String>,
    /// C5 Task 6 additive：被后继接续的原 Failed operation（只读关联投影）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
}

/// G1：candidate_recovery 引用的门是否已被处理（approve/close 后节点
/// 落终态）。读取失败按未处理返回（fail-safe：保持等待项可见，不吞错）。
fn candidate_recovery_gate_resolved(
    lifecycle: &crate::product::lifecycle_store::LifecycleStore,
    project_id: &str,
    issue_id: &str,
    session_id: &str,
    gate_id: &str,
) -> bool {
    use crate::web::workspace_ws_types::timeline::TimelineNodeStatus;

    let nodes = lifecycle
        .load_timeline_nodes_for_issue_session(project_id, issue_id, session_id)
        .unwrap_or_default();
    nodes.iter().any(|node| {
        node.node_id == gate_id
            && matches!(
                node.status,
                TimelineNodeStatus::Completed
                    | TimelineNodeStatus::Failed
                    | TimelineNodeStatus::Skipped
            )
    })
}

/// 从 durable enrollment/lease/advance/compile/gate 事实派生 issue 级 C1
/// 恢复等待项。仅当前 enabled 且已绑定 plan/session 的 enrollment 投影；
/// 读取失败显式上抛（不吞为空列表）。
pub fn list_c1_waiting_items(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
) -> Result<Vec<C1WaitingItemDto>, ProductStoreError> {
    use crate::product::advance_store::{AdvanceStatus, AdvanceStore};
    use crate::product::coding_attempt_store::CodingAttemptStore;
    use crate::product::coding_workspace_engine::CodingWorkspaceEngine;
    use crate::product::git_workspace_service::GitWorkspaceService;
    use crate::product::models::automation::{LeaseDecision, LeaseDisposition};
    use crate::product::models::outline::WorkItemPlanCompileStatus;
    use crate::product::work_item_plan_store::WorkItemPlanStore;

    let enrollment = IssueAutomationStore::new(paths.clone()).get(project_id, issue_id)?;
    let Some(enrollment) = enrollment else {
        return Ok(Vec::new());
    };
    if !enrollment.enabled {
        return Ok(Vec::new());
    }
    let binding_target = enrollment
        .binding_history
        .as_ref()
        .map(|history| history.current.target.clone());
    let plan_id = enrollment.plan_id.clone();
    let session_id = enrollment.session_id.clone();
    let mut items: Vec<C1WaitingItemDto> = Vec::new();

    // A07：当前 binding 指向的 plan session 上的孤儿候选快照（不猜最新
    // session）；快照不完整时 reason 逐项列出缺失事实。
    let lifecycle = LifecycleStore::new(paths.clone());
    if let Some(bound_session_id) = session_id.as_deref() {
        let bound_session = lifecycle.get_workspace_session(bound_session_id)?;
        if bound_session.workspace_type == crate::product::models::WorkspaceType::WorkItemPlan {
            if let Some(recovery) = bound_session
                .human_gate_snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.candidate_recovery.as_ref())
                // G1（终局关闸缺口）：approve/close 不回清快照——投影派生
                // 按当前门状态过滤：gate 节点已终态（Completed/Failed/
                // Skipped）即视为已处理，等待项消隐（现场
                // session_auto_bd69a84a/timeline_node_003 陈旧误报）；节点
                // 缺失（无法证明已处理）保持投影，Active/Paused 照常投影。
                && !candidate_recovery_gate_resolved(
                    &lifecycle,
                    project_id,
                    issue_id,
                    bound_session_id,
                    &recovery.gate_id,
                )
            {
                let reason = if recovery.complete {
                    "candidate snapshot complete; awaiting human recovery".to_string()
                } else {
                    format!(
                        "candidate snapshot incomplete: {}",
                        recovery.missing.join(",")
                    )
                };
                items.push(C1WaitingItemDto {
                    id: format!(
                        "c1:candidate_recovery:{}:{}",
                        bound_session.id, recovery.gate_id
                    ),
                    kind: "candidate_recovery".to_string(),
                    reason,
                    completed_steps: recovery.completed_steps.clone(),
                    target: binding_target.clone(),
                    plan_id: plan_id.clone(),
                    session_id: Some(bound_session.id.clone()),
                    attempt_id: None,
                    gate_id: Some(recovery.gate_id.clone()),
                    possible_side_effect: None,
                    actions: vec!["recover_candidate".to_string()],
                    next_phase: Some("candidate_recovered".to_string()),
                    expected_version: None,
                    action_context: Vec::new(),
                    operation_id: None,
                    diagnostics: None,
                    project_id: None,
                    issue_id: None,
                    parent_operation_id: None,
                    superseded_by: None,
                });
            }
        }
    }

    // A09：lease 三态（复用 Task 6 只读分类；链路未开始不制造噪声项）。
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(1);
    let engine = CodingWorkspaceEngine::new(
        CodingAttemptStore::new(paths.clone()),
        GitWorkspaceService::new(),
        event_tx,
    );
    let LeaseDecision {
        disposition,
        lease_id,
        last_activity_at,
        evidence,
    } = engine.classify_worktree_lease(project_id, issue_id);
    // 死亡租约的接管命令需要 expected attempt——只投影真实 terminal attempt
    // （fail-closed，不猜）；无 attempt 证据时留空，由确认接管 REST 校验拒绝。
    let terminal_attempt_id = if disposition == LeaseDisposition::DeadNeedsTakeover {
        CodingAttemptStore::new(paths.clone())
            .list_attempts_for_issue(project_id, issue_id)
            .ok()
            .and_then(|attempts| {
                attempts
                    .iter()
                    .filter(|attempt| !attempt.status.is_active())
                    .max_by(|left, right| left.updated_at.cmp(&right.updated_at))
                    .map(|attempt| attempt.id.clone())
            })
    } else {
        None
    };
    let lease_item = |kind: &str, reason: String, actions: Vec<&str>, next_phase: Option<&str>| {
        C1WaitingItemDto {
            id: format!("c1:{kind}:{issue_id}:{lease_id}"),
            kind: kind.to_string(),
            reason,
            completed_steps: Vec::new(),
            target: binding_target.clone(),
            plan_id: plan_id.clone(),
            session_id: session_id.clone(),
            attempt_id: terminal_attempt_id.clone(),
            gate_id: None,
            possible_side_effect: last_activity_at.clone(),
            actions: actions.into_iter().map(str::to_string).collect(),
            next_phase: next_phase.map(str::to_string),
            expected_version: None,
            action_context: Vec::new(),
            operation_id: None,
            diagnostics: None,
            project_id: None,
            issue_id: None,
            parent_operation_id: None,
            superseded_by: None,
        }
    };
    match disposition {
        LeaseDisposition::ActiveWait => items.push(lease_item(
            "lease_wait",
            format!("lease active; automation keeps waiting (lease {lease_id})"),
            vec![],
            None,
        )),
        LeaseDisposition::DeadNeedsTakeover => items.push(lease_item(
            "lease_takeover",
            format!(
                "lease dead; takeover requires human confirmation: {}",
                evidence.join("; ")
            ),
            vec!["confirm_takeover"],
            Some("takeover_confirmed"),
        )),
        LeaseDisposition::UnknownNeedsHuman => {
            let unstarted = evidence
                .iter()
                .any(|fact| fact.contains("worktree record not found"));
            if !unstarted {
                items.push(lease_item(
                    "lease_unknown",
                    format!(
                        "lease liveness unknown; stopped for human: {}",
                        evidence.join("; ")
                    ),
                    vec![],
                    Some("manual_triage_or_rebind"),
                ));
            }
        }
    }

    // A09：Failed advance → 显式 retry-initialization（原 Failed 记录只读）。
    // next_phase 携带 durable journal checkpoint（重试从该检查点续做）。
    if let Some(plan_id) = plan_id.as_deref() {
        let advance_store = AdvanceStore::new(paths.clone());
        if let Some(record) = advance_store.get_advance_for_plan(project_id, issue_id, plan_id)? {
            if record.status == AdvanceStatus::Failed {
                let journal_phase = advance_store
                    .get_advance_initialization(&record)?
                    .filter(|journal| journal.error.is_some())
                    .map(|journal| checkpoint_slug(journal.phase));
                items.push(C1WaitingItemDto {
                    id: format!("c1:advance_retry_failed:{}", record.id),
                    kind: "advance_retry_failed".to_string(),
                    reason: "advance initialization failed; original record stays failed"
                        .to_string(),
                    completed_steps: Vec::new(),
                    target: binding_target.clone(),
                    plan_id: Some(record.plan_id.clone()),
                    session_id: session_id.clone(),
                    attempt_id: record.attempt_id.clone(),
                    gate_id: None,
                    possible_side_effect: record.error.clone(),
                    actions: vec!["retry_initialization".to_string()],
                    next_phase: journal_phase.map(str::to_string),
                    expected_version: None,
                    action_context: Vec::new(),
                    operation_id: None,
                    diagnostics: None,
                    project_id: None,
                    issue_id: None,
                    parent_operation_id: None,
                    superseded_by: None,
                });
            }
        }

        // A12：intent 未声明/不能执行的 compile 停等（最新 Failed 事务）。
        let plan_store = WorkItemPlanStore::new(paths.clone());
        let mut transactions =
            plan_store.list_compile_transactions(project_id, issue_id, plan_id)?;
        transactions.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        let intent_finding = transactions
            .iter()
            .filter(|tx| tx.status == WorkItemPlanCompileStatus::Failed)
            .find_map(|tx| {
                tx.validator_findings
                    .iter()
                    .find(|finding| {
                        finding.code == "intent_undeclared" || finding.code == "intent_unexecutable"
                    })
                    .map(|finding| (tx.compile_id.clone(), finding))
            });
        if let Some((compile_id, finding)) = intent_finding {
            items.push(C1WaitingItemDto {
                id: format!("c1:intent_blocked:{compile_id}:{}", finding.code),
                kind: "intent_blocked".to_string(),
                reason: format!("{}: {}", finding.code, finding.message),
                completed_steps: Vec::new(),
                target: binding_target.clone(),
                plan_id: Some(plan_id.to_string()),
                session_id: session_id.clone(),
                attempt_id: None,
                gate_id: None,
                possible_side_effect: None,
                actions: Vec::new(),
                next_phase: Some("plan_revision".to_string()),
                expected_version: None,
                action_context: Vec::new(),
                operation_id: None,
                diagnostics: None,
                project_id: None,
                issue_id: None,
                parent_operation_id: None,
                superseded_by: None,
            });
        }
    }

    // A13：换代历史只读可查 + 显式 rebind 操作面。
    if let Some(history) = enrollment.binding_history.as_ref() {
        if !history.previous.is_empty() {
            items.push(C1WaitingItemDto {
                id: format!("c1:generation_history:{issue_id}"),
                kind: "generation_history".to_string(),
                reason: format!(
                    "{} previous binding generation(s) kept read-only",
                    history.previous.len()
                ),
                completed_steps: Vec::new(),
                target: binding_target.clone(),
                plan_id: plan_id.clone(),
                session_id: session_id.clone(),
                attempt_id: None,
                gate_id: None,
                possible_side_effect: None,
                actions: vec!["rebind".to_string()],
                next_phase: Some("rebind".to_string()),
                expected_version: None,
                action_context: Vec::new(),
                operation_id: None,
                diagnostics: None,
                project_id: None,
                issue_id: None,
                parent_operation_id: None,
                superseded_by: None,
            });
        }
    }
    // C2 Task 12（REQ-CRO-06）：coding 链十类 durable 等待事实 additive 投影
    //（不新建 DTO、不以事件当权威；读取失败显式上抛）。
    append_c2_waiting_items(
        paths,
        project_id,
        issue_id,
        plan_id.as_deref(),
        session_id.as_deref(),
        &binding_target,
        &mut items,
    )?;
    Ok(items)
}

// ---------------------------------------------------------------------------
// C5 Task 6（REQ-INIT-C5-RESUME）：project 级 repository 初始化失败等待项
// 投影。事实只从 operation store 派生；按 parent→后继链计算展示/消隐：
// 无后继的 Failed 链叶展示带 resume 动作的等待项，Completed 后继使原
// 等待项稳定消隐（记录只读可查），Failed 后继展示最新链叶并保留 parent
// 关联，Created/Running 后继显示运行中只读项（避免重复执行）。
// ---------------------------------------------------------------------------

/// C5 Task 6：Claude 初始化失败等待项 kind（前端 kind 宽 string 兼容）。
pub const WAITING_KIND_REPOSITORY_INITIALIZATION_FAILED: &str = "repository_initialization_failed";

/// C5 Task 6：初始化失败的结构化诊断（由 operation 冻结的 error 派生）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RepositoryInitializationFailureDiagnostics {
    pub failed_step: String,
    pub reason_code: String,
    pub provider: Option<String>,
    pub stderr_summary: Option<String>,
    pub changed_paths: Vec<String>,
    pub retryable: bool,
}

fn repository_initialization_step_slug(
    step: crate::product::repository_store::RepositoryInitializationStepKind,
) -> &'static str {
    use crate::product::repository_store::RepositoryInitializationStepKind;
    match step {
        RepositoryInitializationStepKind::CadenceSkills => "cadence_skills",
        RepositoryInitializationStepKind::PreCheck => "pre_check",
        RepositoryInitializationStepKind::RuleConfig => "rule_config",
        RepositoryInitializationStepKind::McpConfiguration => "mcp_configuration",
        RepositoryInitializationStepKind::ProjectRulesExamples => "project_rules_examples",
        RepositoryInitializationStepKind::GitFinalize => "git_finalize",
    }
}

fn repository_initialization_failure_item(
    project_id: &str,
    operation: &crate::product::repository_store::RepositoryInitializationOperation,
) -> C1WaitingItemDto {
    use crate::product::repository_store::RepositoryInitializationStepStatus;

    let completed_steps: Vec<String> = operation
        .steps
        .iter()
        .filter(|step| step.status == RepositoryInitializationStepStatus::Completed)
        .map(|step| repository_initialization_step_slug(step.step_id).to_string())
        .collect();
    let failed_step = operation
        .failed_step
        .map(repository_initialization_step_slug)
        .unwrap_or_default()
        .to_string();
    let diagnostics =
        operation
            .error
            .as_ref()
            .map(|error| RepositoryInitializationFailureDiagnostics {
                reason_code: error.reason_code.clone(),
                provider: error.provider.clone(),
                stderr_summary: error.stderr_summary.clone(),
                changed_paths: error.changed_paths.clone().unwrap_or_default(),
                retryable: error.retryable,
                failed_step: failed_step.clone(),
            });
    C1WaitingItemDto {
        id: format!(
            "c1:project:{project_id}:repository_init:{}",
            operation.operation_id
        ),
        kind: WAITING_KIND_REPOSITORY_INITIALIZATION_FAILED.to_string(),
        reason: format!(
            "repository initialization failed at {failed_step} ({}); awaiting gateway recovery",
            operation
                .error
                .as_ref()
                .map(|error| error.reason_code.as_str())
                .unwrap_or("unknown")
        ),
        completed_steps,
        target: None,
        plan_id: None,
        session_id: None,
        attempt_id: None,
        gate_id: None,
        possible_side_effect: None,
        actions: vec!["resume_repository_initialization".to_string()],
        next_phase: Some("repository_registered".to_string()),
        expected_version: None,
        action_context: Vec::new(),
        operation_id: Some(operation.operation_id.clone()),
        diagnostics,
        project_id: Some(project_id.to_string()),
        issue_id: None,
        parent_operation_id: operation.parent_operation_id.clone(),
        superseded_by: None,
    }
}

/// C5 Task 6：project 级初始化失败等待项（issue 级
/// [`list_c1_waiting_items`] 签名与消费零变化）。读取失败显式上抛。
pub fn list_project_c1_waiting_items(
    paths: &ProductAppPaths,
    project_id: &str,
) -> Result<Vec<C1WaitingItemDto>, ProductStoreError> {
    use crate::product::repository_store::RepositoryInitializationOperationStatus;
    use crate::product::repository_store::RepositoryInitializationOperationStore;
    use std::collections::HashSet;

    let operations = RepositoryInitializationOperationStore::new(paths.clone()).list(project_id)?;
    let superseded: HashSet<&str> = operations
        .iter()
        .filter_map(|operation| operation.parent_operation_id.as_deref())
        .collect();

    let mut items = Vec::new();
    for operation in &operations {
        let has_successor = superseded.contains(operation.operation_id.as_str());
        match operation.status {
            // 链叶 Failed：唯一可 resume 的展示项。
            RepositoryInitializationOperationStatus::Failed if !has_successor => {
                items.push(repository_initialization_failure_item(
                    project_id, operation,
                ));
            }
            // Created/Running 后继：运行中只读项（无动作、无诊断），避免重复执行。
            RepositoryInitializationOperationStatus::Created
            | RepositoryInitializationOperationStatus::Running
                if operation.parent_operation_id.is_some() =>
            {
                items.push(C1WaitingItemDto {
                    id: format!(
                        "c1:project:{project_id}:repository_init:{}",
                        operation.operation_id
                    ),
                    kind: WAITING_KIND_REPOSITORY_INITIALIZATION_FAILED.to_string(),
                    reason: "repository initialization resume is running; read-only until terminal"
                        .to_string(),
                    completed_steps: Vec::new(),
                    target: None,
                    plan_id: None,
                    session_id: None,
                    attempt_id: None,
                    gate_id: None,
                    possible_side_effect: None,
                    actions: Vec::new(),
                    next_phase: Some("repository_registered".to_string()),
                    expected_version: None,
                    action_context: Vec::new(),
                    operation_id: Some(operation.operation_id.clone()),
                    diagnostics: None,
                    project_id: Some(project_id.to_string()),
                    issue_id: None,
                    parent_operation_id: operation.parent_operation_id.clone(),
                    superseded_by: None,
                });
            }
            // Completed 后继：原等待项稳定消隐（记录仍可查）。
            _ => {}
        }
    }
    items.sort_by(|left, right| left.operation_id.cmp(&right.operation_id));
    Ok(items)
}

// ---------------------------------------------------------------------------
// C2 Task 12（REQ-CRO-06）：coding 链十类 durable 等待事实的 additive
// 投影。只从 durable store 派生（attempt 状态／诊断、Task 2 命令账本
// 停等记录、租约三态、blocked gate reason_code、验证处理记录、政策核验
// 等待事实、返修指令认领、SC 大候选停等）；通知／WS 投递失败不回滚
// 业务事实，驾驶舱经 GET 补读本投影。动作只暴露 REST 可达面
// （restart_coding／confirm_takeover／gate action_id）；无 REST 面的
// 等待项只读呈现（操作在既有 coding workspace 面）。
// ---------------------------------------------------------------------------

/// C2 十类 kind 常量（前端 kind 宽 string 直接兼容）。
pub const C2_KIND_CODING_COMPLETION_UNCONFIRMED: &str = "coding_completion_unconfirmed";
pub const C2_KIND_CODING_ALREADY_RUNNING: &str = "coding_already_running";
pub const C2_KIND_CODING_TAKEOVER_REQUIRED: &str = "coding_takeover_required";
pub const C2_KIND_CODING_LEASE_UNKNOWN: &str = "coding_lease_unknown";
pub const C2_KIND_CODING_RESTART_AVAILABLE: &str = "coding_restart_available";
pub const C2_KIND_REVIEWER_CONFIGURATION_MISSING: &str = "reviewer_configuration_missing";
pub const C2_KIND_VERIFICATION_TRIAGE: &str = "verification_triage";
pub const C2_KIND_POLICY_VERIFICATION: &str = "policy_verification";
pub const C2_KIND_INSTRUCTION_CLAIM_INTERRUPTED: &str = "instruction_claim_interrupted";
pub const C2_KIND_LARGE_CANDIDATE_BLOCKED: &str = "large_candidate_blocked";

fn append_c2_waiting_items(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    plan_id: Option<&str>,
    session_id: Option<&str>,
    binding_target: &Option<crate::product::logical_codebase::EnrollmentTarget>,
    items: &mut Vec<C1WaitingItemDto>,
) -> Result<(), ProductStoreError> {
    use crate::product::coding_models::CodingAttemptStatus;
    use crate::product::models::automation::{LeaseDisposition, OperationState};

    let store = crate::product::coding_attempt_store::CodingAttemptStore::new(paths.clone());
    let attempts = store.list_attempts_for_issue(project_id, issue_id)?;
    let base = |kind: &str, id: String| C1WaitingItemDto {
        id,
        kind: kind.to_string(),
        reason: String::new(),
        completed_steps: Vec::new(),
        target: binding_target.clone(),
        plan_id: plan_id.map(str::to_string),
        session_id: session_id.map(str::to_string),
        attempt_id: None,
        gate_id: None,
        possible_side_effect: None,
        actions: Vec::new(),
        next_phase: None,
        expected_version: None,
        action_context: Vec::new(),
        operation_id: None,
        diagnostics: None,
        project_id: None,
        issue_id: None,
        parent_operation_id: None,
        superseded_by: None,
    };

    for attempt in &attempts {
        // 完成状态待确认（A06）：AwaitingManualRecovery＋诊断 reason；恢复
        // 操作在 coding workspace 显式 RecoverCoding 面（驾驶舱只读呈现）。
        if attempt.status == CodingAttemptStatus::AwaitingManualRecovery {
            let reason = attempt
                .manual_recovery_reason
                .clone()
                .unwrap_or_else(|| "completion outcome unconfirmed".to_string());
            let mut item = base(
                C2_KIND_CODING_COMPLETION_UNCONFIRMED,
                format!("c2:coding_completion_unconfirmed:{}", attempt.id),
            );
            item.reason = format!(
                "coding run completion outcome unconfirmed ({reason}); \
                 awaiting manual recovery; provider side effects may exist"
            );
            item.attempt_id = Some(attempt.id.clone());
            item.possible_side_effect = Some(format!("provider outcome unconfirmed: {reason}"));
            item.expected_version = Some(attempt.version);
            item.next_phase = Some("manual_recovery".to_string());
            items.push(item);
        }

        // restart 可用（A08）：终态 attempt 显式 restart（REST 与 WS 同一
        // 应用服务；旧版本 Rejected"请刷新"）。
        if matches!(
            attempt.status,
            CodingAttemptStatus::Aborted | CodingAttemptStatus::Failed
        ) {
            let mut item = base(
                C2_KIND_CODING_RESTART_AVAILABLE,
                format!("c2:coding_restart_available:{}", attempt.id),
            );
            item.reason = format!(
                "coding attempt reached terminal state {:?}; explicit restart \
                 re-admits the attempt and spawns a new runner",
                attempt.status
            );
            item.attempt_id = Some(attempt.id.clone());
            item.actions = vec!["restart_coding".to_string()];
            item.expected_version = Some(attempt.version);
            item.action_context = vec![WaitingItemAction {
                action: "restart_coding".to_string(),
                command_id: format!("cmd-c2-restart-{}", attempt.id),
                expected_version: attempt.version,
            }];
            item.next_phase = Some("coding_restarted".to_string());
            items.push(item);
        }

        // reviewer 配置缺失（A10）：reason_code 定格的开放 blocked gate；
        // gate 动作经 gate-responses REST 作答（与 WS 同一应用服务）。
        for gate in store.list_open_blocked_gates(project_id, issue_id, &attempt.id)? {
            if gate.reason_code.as_deref() != Some("reviewer_configuration_missing") {
                continue;
            }
            let mut item = base(
                C2_KIND_REVIEWER_CONFIGURATION_MISSING,
                format!(
                    "c2:reviewer_configuration_missing:{}:{}",
                    attempt.id, gate.gate_id
                ),
            );
            item.reason = format!(
                "reviewer provider is missing ({}): {}; configuring a reviewer \
                 or a gate retry resumes the chain without falling back to author",
                gate.title, gate.description
            );
            item.attempt_id = Some(attempt.id.clone());
            item.gate_id = Some(gate.gate_id.clone());
            item.actions = gate
                .available_actions
                .iter()
                .map(|action| action.action_id.clone())
                .collect();
            item.expected_version = Some(attempt.version);
            item.action_context = gate
                .available_actions
                .iter()
                .map(|action| WaitingItemAction {
                    action: action.action_id.clone(),
                    command_id: format!("cmd-c2-gate-{}-{}", gate.gate_id, action.action_id),
                    expected_version: attempt.version,
                })
                .collect();
            item.next_phase = Some("reviewer_configured_or_gate_resolved".to_string());
            items.push(item);
        }

        // 验证处理未决（A11）：决定面在 coding workspace 验证处理面板
        //（三类结论均需用户明确批准），驾驶舱只读呈现。
        for triage in store.list_verification_triage_records(project_id, issue_id, &attempt.id)? {
            if triage.status
                != crate::product::coding_attempt_store::VerificationTriageStatus::Pending
            {
                continue;
            }
            let mut item = base(
                C2_KIND_VERIFICATION_TRIAGE,
                format!("c2:verification_triage:{}:{}", attempt.id, triage.triage_id),
            );
            item.reason = format!(
                "verification triage pending: check {} finding {} \
                 (plan revision {}); the decision requires explicit human approval",
                triage.check_id, triage.finding_id, triage.plan_revision_id
            );
            item.attempt_id = Some(attempt.id.clone());
            item.expected_version = Some(attempt.version);
            item.next_phase = Some("verification_triage_decided".to_string());
            items.push(item);
        }

        // 政策核验停等（A15）：fail-closed 等待事实直达；重新授权面随
        // C4 resolver 联验接线（MUST NOT 本地 fallback）。
        if let Some(record) =
            crate::product::logical_codebase::load_policy_verification_waiting_fact(paths, attempt)
                .map_err(|error| {
                    ProductStoreError::Io(format!("read policy verification waiting fact: {error}"))
                })?
        {
            let mut item = base(
                C2_KIND_POLICY_VERIFICATION,
                format!("c2:policy_verification:{}", attempt.id),
            );
            item.reason = format!("{}: {}", record.reason_code, record.detail);
            item.attempt_id = Some(attempt.id.clone());
            item.possible_side_effect = record.policy_digest.clone();
            item.expected_version = Some(attempt.version);
            item.next_phase = Some("policy_reauthorized".to_string());
            items.push(item);
        }

        // 指令消费中断（A15/C-1b）：恢复重驱对账落账的等待事实（claim
        // 存在而其 node 无 role run 输出——消费标记后、spawn 前中断）；
        // 下一次 coder 重驱以 claim.instruction_ids 强制入渲染（不重写
        // claim、不二次消费），重放完成后对账清除事实。
        for fact in
            store.list_instruction_claim_interrupted_facts(project_id, issue_id, &attempt.id)?
        {
            let mut item = base(
                C2_KIND_INSTRUCTION_CLAIM_INTERRUPTED,
                format!(
                    "c2:instruction_claim_interrupted:{}:{}",
                    attempt.id, fact.claim_id
                ),
            );
            item.reason = format!(
                "rework instruction claim {} consumed but never entered any \
                 prompt (instructions {:?}); the next coder re-drive \
                 force-renders the claimed instructions instead of \
                 consuming twice",
                fact.claim_id, fact.instruction_ids
            );
            item.attempt_id = Some(attempt.id.clone());
            item.expected_version = Some(attempt.version);
            item.next_phase = Some("claim_replayed".to_string());
            items.push(item);
        }
    }

    // 租约三态（A06／A09）：只在存在真实 admission 停等事实（Task 2 命令
    // 账本 NeedsHuman 记录）且停等者非租约持有者时投影 coding 视图；
    // 分类复用 C1 classify_worktree_lease（不新建判定），不抢占任何
    // 无法证明死亡的租约。
    let mut stop_wait_attempt_ids: Vec<String> = Vec::new();
    for attempt in &attempts {
        let records = store.list_attempt_command_records(project_id, issue_id, &attempt.id)?;
        if records
            .iter()
            .any(|record| record.state == OperationState::NeedsHuman)
        {
            stop_wait_attempt_ids.push(attempt.id.clone());
        }
    }
    if !stop_wait_attempt_ids.is_empty() {
        let lease = store.classify_worktree_lease(project_id, issue_id);
        let foreign_wait = stop_wait_attempt_ids
            .iter()
            .any(|attempt_id| Some(attempt_id.as_str()) != Some(lease.lease_id.as_str()));
        if foreign_wait {
            match lease.disposition {
                LeaseDisposition::ActiveWait => {
                    let mut item = base(
                        C2_KIND_CODING_ALREADY_RUNNING,
                        format!("c2:coding_already_running:{}:{}", issue_id, lease.lease_id),
                    );
                    item.reason = format!(
                        "coding run {} already holds the worktree lease; \
                         kicks for {:?} stay rejected; wait for it to settle \
                         or confirm takeover after it ends",
                        lease.lease_id, stop_wait_attempt_ids
                    );
                    item.attempt_id = Some(lease.lease_id.clone());
                    item.possible_side_effect = lease.last_activity_at.clone();
                    item.next_phase = Some("coding_run_settles".to_string());
                    items.push(item);
                }
                LeaseDisposition::DeadNeedsTakeover
                    if !lease.lease_id.is_empty()
                        && let Ok(holder) =
                            store.get_attempt(project_id, issue_id, &lease.lease_id) =>
                {
                    let mut item = base(
                        C2_KIND_CODING_TAKEOVER_REQUIRED,
                        format!(
                            "c2:coding_takeover_required:{}:{}",
                            issue_id, lease.lease_id
                        ),
                    );
                    item.reason = format!(
                        "coding lease {} is dead (holder terminal {:?}); \
                         takeover requires human confirmation: {}",
                        lease.lease_id,
                        holder.status,
                        lease.evidence.join("; ")
                    );
                    item.attempt_id = Some(holder.id.clone());
                    item.actions = vec!["confirm_takeover".to_string()];
                    item.expected_version = Some(holder.version);
                    item.action_context = vec![WaitingItemAction {
                        action: "confirm_takeover".to_string(),
                        command_id: format!("cmd-c2-takeover-{}-{}", issue_id, lease.lease_id),
                        expected_version: holder.version,
                    }];
                    item.next_phase = Some("takeover_confirmed".to_string());
                    items.push(item);
                }
                LeaseDisposition::UnknownNeedsHuman
                    if !lease
                        .evidence
                        .iter()
                        .any(|fact| fact.contains("worktree record not found")) =>
                {
                    let mut item = base(
                        C2_KIND_CODING_LEASE_UNKNOWN,
                        format!("c2:coding_lease_unknown:{}:{}", issue_id, lease.lease_id),
                    );
                    item.reason = format!(
                        "coding lease liveness unknown; stopped for human: {}",
                        lease.evidence.join("; ")
                    );
                    item.attempt_id = stop_wait_attempt_ids.first().cloned();
                    item.possible_side_effect = lease.last_activity_at.clone();
                    item.next_phase = Some("lease_clarified".to_string());
                    items.push(item);
                }
                _ => {}
            }
        }
    }

    // 大候选停等（A14）：绑定 plan session 分区的 SC 组装拒绝事实；
    // "分段返修／重试"在计划会话门面（用户点击后才开新回合）。
    if let Some(bound_session_id) = session_id {
        let lifecycle = LifecycleStore::new(paths.clone());
        if let Ok(bound_session) = lifecycle.get_workspace_session(bound_session_id)
            && bound_session.workspace_type
                == crate::product::models::WorkspaceType::WorkItemPlan
            && let Some(blocked) =
                crate::product::workspace_engine::conversational_gate::read_sc_revision_blocked_fact(
                    paths,
                    project_id,
                    issue_id,
                    bound_session_id,
                )?
        {
            let mut item = base(
                C2_KIND_LARGE_CANDIDATE_BLOCKED,
                format!("c2:large_candidate_blocked:{bound_session_id}"),
            );
            item.reason = format!(
                "{}: {} ({} bytes exceeds hard limit {}); segmented \
                 revision or retry opens a new turn after a human click",
                blocked.reason_code,
                blocked.detail,
                blocked.total_bytes,
                blocked.hard_limit_bytes
            );
            item.session_id = Some(bound_session_id.to_string());
            item.next_phase = Some("segmented_revision_or_retry".to_string());
            items.push(item);
        }
    }

    Ok(())
}
/// Advance journal checkpoint 的 wire slug（serde snake_case 同形）。
fn checkpoint_slug(
    phase: crate::product::advance_store::AdvanceInitializationPhase,
) -> &'static str {
    use crate::product::advance_store::AdvanceInitializationPhase;
    match phase {
        AdvanceInitializationPhase::RecordPersisted => "record_persisted",
        AdvanceInitializationPhase::JournalPrepared => "journal_prepared",
        AdvanceInitializationPhase::AttemptPersisted => "attempt_persisted",
        AdvanceInitializationPhase::WorktreeBound => "worktree_bound",
        AdvanceInitializationPhase::PlanBindingSaved => "plan_binding_saved",
        AdvanceInitializationPhase::UnitsMaterialized => "units_materialized",
        AdvanceInitializationPhase::Ready => "ready",
    }
}

/// SourceStoreError → ProductStoreError：NotFound 保留语义，其余以 Io 文本
/// 上抛（引用/文件不可读不得报告成功）。
fn source_store_error(
    error: crate::product::work_item_plan_source_store::SourceStoreError,
) -> ProductStoreError {
    ProductStoreError::Io(format!(
        "plan publication provenance unreadable: {}",
        error.code()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::wiga_gate_fixture::EnrolledGateFixture;
    use crate::web::wiga_gate_fixture::{ISSUE_ID, PROJECT_ID};
    use tower::ServiceExt;

    struct CompiledPlanFixture {
        gate: EnrolledGateFixture,
    }

    impl CompiledPlanFixture {
        async fn new() -> Self {
            Self {
                gate: EnrolledGateFixture::new().await,
            }
        }

        fn bound_enrollment(&self) -> IssueAutomationEnrollment {
            IssueAutomationStore::new(self.gate.inner.paths.clone())
                .get(PROJECT_ID, ISSUE_ID)
                .unwrap()
                .unwrap()
        }

        fn paths(&self) -> &ProductAppPaths {
            &self.gate.inner.paths
        }

        async fn approve_and_persist_failed_compile(&mut self) {
            self.gate.fail_compile_after_human_approve().await;
        }

        /// 人工 CompileRecovery Continue + Approve 关门（fixture 共享面）。
        async fn recover_and_commit_compile(&mut self) {
            self.gate.recover_and_confirm_compile().await;
        }
    }

    #[tokio::test]
    async fn plan_confirmed_info_requires_published_compile_and_confirmed_session() {
        let mut fixture = Box::new(CompiledPlanFixture::new().await);
        let enrollment = fixture.bound_enrollment();
        assert!(
            plan_confirmed_info(fixture.paths(), &enrollment)
                .unwrap()
                .is_none(),
            "未编译/未确认不得产生成功信息"
        );
        fixture.approve_and_persist_failed_compile().await;
        assert!(
            plan_confirmed_info(fixture.paths(), &enrollment)
                .unwrap()
                .is_none(),
            "failpoint 失败 compile（门保持开启）不得产生成功信息"
        );
        fixture.recover_and_commit_compile().await;
        let first = plan_confirmed_info(fixture.paths(), &enrollment)
            .unwrap()
            .expect("恢复并确认后必须有且仅有一条成功信息");
        let second = plan_confirmed_info(fixture.paths(), &fixture.bound_enrollment())
            .unwrap()
            .expect("重复读取幂等");
        assert_eq!(first.key, second.key);
        assert_eq!(first.occurred_at, second.occurred_at);
        assert!(first.key.contains(&first.plan_id));
        assert!(first.key.contains("plan_confirmed:"));
        assert_eq!(first.session_id, fixture.gate.session_id);
        assert_eq!(first.title, PLAN_CONFIRMED_INFO_TITLE);
        assert!(
            !first.occurred_at.is_empty(),
            "occurred_at 取 tx.committed_at"
        );
    }

    /// 禁用 enrollment 后历史成功不投影为当前信息（禁用不是假成功）。
    #[tokio::test]
    async fn plan_confirmed_info_hidden_after_enrollment_disabled() {
        let mut fixture = Box::new(CompiledPlanFixture::new().await);
        fixture.approve_and_persist_failed_compile().await;
        fixture.recover_and_commit_compile().await;
        let enrollment = fixture.bound_enrollment();
        assert!(
            plan_confirmed_info(fixture.paths(), &enrollment)
                .unwrap()
                .is_some()
        );
        let disabled = IssueAutomationEnrollment {
            enabled: false,
            ..enrollment
        };
        assert!(
            plan_confirmed_info(fixture.paths(), &disabled)
                .unwrap()
                .is_none(),
            "禁用后不得把历史成功当当前信息"
        );
    }
}
