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
/// `None`，真实 IO/JSON 读取失败原样上抛（不吞为 off）。绑定 plan/session
/// 已被删除（NotFound：实体级联删除清盘后 enrollment 仍指向旧身份）时降级
/// 为 `None`——已删会话不是成功事实，也不得把已删 id 抛成 NotFound 让
/// 首页报错。
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
    let session = match lifecycle.get_workspace_session(session_id) {
        Ok(session) => session,
        Err(ProductStoreError::NotFound { .. }) => return Ok(None),
        Err(other) => return Err(other),
    };
    if session.project_id != enrollment.project_id
        || session.issue_id != enrollment.issue_id
        // 批 2 P2（REQ-WIGA-02 修订）：按冻结 plan_approval 期望互证——auto
        // 冻结认 AutoIfValid、manual 只认 Interactive；错绑不投影成功。
        || session.run_policy != enrollment.options.plan_approval.expected_run_policy()
        || session.status != WorkspaceSessionStatus::Confirmed
        || session.single_candidate_phase != Some(SingleCandidatePhase::Completed)
    {
        return Ok(None);
    }
    let plan = match lifecycle.get_issue_work_item_plan(
        &enrollment.project_id,
        &enrollment.issue_id,
        plan_id,
    ) {
        Ok(plan) => plan,
        Err(ProductStoreError::NotFound { .. }) => return Ok(None),
        Err(other) => return Err(other),
    };
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
    /// F-A additive（owner 2026-10-25 裁定，cadence/notes/2026-10-25_UI复验发现）：等待项
    /// 可操作性分类。false = 自动等待/只读信息（到期自动恢复或纯审计痕迹，前端
    /// 归「进度信息」分组，不进「需要人工处理」与待处理计数）；true = 需要人工
    /// 决策。旧响应缺失该字段按 true 解释（fail-safe：旧数据保持旧展示，不因
    /// 缺字段静默降级真待办）。
    #[serde(default = "c1_waiting_item_requires_human_by_default")]
    pub requires_human: bool,
}

/// [`C1WaitingItemDto::requires_human`] 的反序列化缺省（旧响应 → true）。
fn c1_waiting_item_requires_human_by_default() -> bool {
    true
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
    // session）；快照不完整时 reason 逐项列出缺失事实。绑定会话已被删除
    //（实体级联删除清盘，NotFound）→ 降级为跳过本段，不把已删 id 抛成
    // NotFound 让首页报错；真实 IO/JSON 错误仍上抛。
    let lifecycle = LifecycleStore::new(paths.clone());
    let bound_session = match session_id.as_deref() {
        None => None,
        Some(bound_session_id) => match lifecycle.get_workspace_session(bound_session_id) {
            Ok(session) => Some(session),
            // 绑定会话已被删除（实体级联删除清盘）→ 降级为没有会话，
            // 跳过本段派生；真实 IO/JSON 错误仍上抛。
            Err(ProductStoreError::NotFound { .. }) => None,
            Err(other) => return Err(other),
        },
    };
    if let (Some(bound_session_id), Some(bound_session)) =
        (session_id.as_deref(), bound_session.as_ref())
        && bound_session.workspace_type == crate::product::models::WorkspaceType::WorkItemPlan
        && let Some(recovery) = bound_session
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
            requires_human: true,
        });
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
    let lease_item = |kind: &str,
                      reason: String,
                      actions: Vec<&str>,
                      next_phase: Option<&str>,
                      requires_human: bool| {
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
            requires_human,
        }
    };
    match disposition {
        // F-A（owner 2026-10-25 裁定）：租约活跃是自动等待（到期自动恢复），
        // 不需要人动手——requires_human=false，前端归「进度信息」不进
        // 「需要人工处理」与计数；死亡租约仍需人确认接管。
        LeaseDisposition::ActiveWait => items.push(lease_item(
            "lease_wait",
            format!("lease active; automation keeps waiting (lease {lease_id})"),
            vec![],
            None,
            false,
        )),
        LeaseDisposition::DeadNeedsTakeover => items.push(lease_item(
            "lease_takeover",
            format!(
                "lease dead; takeover requires human confirmation: {}",
                evidence.join("; ")
            ),
            vec!["confirm_takeover"],
            Some("takeover_confirmed"),
            true,
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
                    true,
                ));
            }
        }
    }

    // A09：Failed advance → 显式 retry-initialization（原 Failed 记录只读）。
    // next_phase 携带 durable journal checkpoint（重试从该检查点续做）。
    if let Some(plan_id) = plan_id.as_deref() {
        let advance_store = AdvanceStore::new(paths.clone());
        if let Some(record) = advance_store.get_advance_for_plan(project_id, issue_id, plan_id)?
            && record.status == AdvanceStatus::Failed
        {
            let journal_phase = advance_store
                .get_advance_initialization(&record)?
                .filter(|journal| journal.error.is_some())
                .map(|journal| checkpoint_slug(journal.phase));
            items.push(C1WaitingItemDto {
                id: format!("c1:advance_retry_failed:{}", record.id),
                kind: "advance_retry_failed".to_string(),
                reason: "advance initialization failed; original record stays failed".to_string(),
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
                requires_human: true,
            });
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
                requires_human: true,
            });
        }
    }

    // A13：换代历史只读可查 + 显式 rebind 操作面。
    // F-A（owner 2026-10-25 裁定）：当前绑定健在（plan 记录与 session 均存在）时，
    // 换代历史只是只读审计信息——不给 rebind 动作、requires_human=false（前端归
    // 「进度信息」，不进「需要人工处理」与待处理计数）；仅当前绑定失效（单 plan
    // 级联删除/悬空绑定，plan 或 session 任一缺失）才升级为换代待办 +「去换代」。
    // plan 存在性经目录列表判定（目录缺失=空列表，真实 IO/JSON 错误仍上抛；
    // 单文件读取把缺文件当 Io 上抛，无法与真 IO 失败区分）。
    let bound_plan_live = match plan_id.as_deref() {
        None => false,
        Some(bound_plan_id) => lifecycle
            .list_issue_work_item_plans(project_id, issue_id)?
            .iter()
            .any(|plan| plan.id == bound_plan_id),
    };
    let bound_session_live = bound_session.is_some();
    if let Some(history) = enrollment.binding_history.as_ref()
        && !history.previous.is_empty()
    {
        let (reason, actions, next_phase, requires_human) = if bound_plan_live && bound_session_live
        {
            (
                format!(
                    "{} previous binding generation(s) kept read-only; current binding active",
                    history.previous.len()
                ),
                Vec::new(),
                None,
                false,
            )
        } else {
            let mut gaps: Vec<&str> = Vec::new();
            if !bound_plan_live {
                gaps.push("plan");
            }
            if !bound_session_live {
                gaps.push("session");
            }
            (
                format!(
                    "{} previous binding generation(s) kept read-only; current binding dangling ({})",
                    history.previous.len(),
                    gaps.join("+")
                ),
                vec!["rebind".to_string()],
                Some("rebind".to_string()),
                true,
            )
        };
        items.push(C1WaitingItemDto {
            id: format!("c1:generation_history:{issue_id}"),
            kind: "generation_history".to_string(),
            reason,
            completed_steps: Vec::new(),
            target: binding_target.clone(),
            plan_id: plan_id.clone(),
            session_id: session_id.clone(),
            attempt_id: None,
            gate_id: None,
            possible_side_effect: None,
            actions,
            next_phase,
            expected_version: None,
            action_context: Vec::new(),
            operation_id: None,
            diagnostics: None,
            project_id: None,
            issue_id: None,
            parent_operation_id: None,
            superseded_by: None,
            requires_human,
        });
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
        requires_human: true,
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
                    requires_human: true,
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
        requires_human: true,
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

    // ── F-A（owner 2026-10-25 裁定，cadence/notes/2026-10-25_UI复验发现）：
    //    等待项可操作性分类——lease_wait 归自动等待，generation_history 按
    //    当前绑定有效性分流。──

    /// F-A 夹具：project/issue + enabled enrollment 绑定 plan/session 并
    /// rebind 一次制造 `history.previous`；`with_plan_record` 控制当前绑定
    /// 指向的 plan 记录是否落盘（false = 悬空绑定，如单 plan 级联删除后）。
    fn f_a_waiting_fixture(
        with_plan_record: bool,
    ) -> (tempfile::TempDir, ProductAppPaths, String, String) {
        use crate::product::issue_automation_store::IssueAutomationStore;
        use crate::product::json_store::write_json;
        use crate::product::lifecycle_store::{CreateWorkspaceSessionInput, LifecycleStore};
        use crate::product::logical_codebase::EnrollmentTarget;
        use crate::product::models::automation::{
            EnrollmentBindingIdentityInput, EnrollmentOptions, EnrollmentRebindRequest,
            EnrollmentSource, EnrollmentWriteCommand, PlanApprovalPolicy,
        };
        use crate::product::models::lifecycle::{
            IssueWorkItemPlan, IssueWorkItemPlanOptions, IssueWorkItemPlanStatus,
        };
        use crate::product::models::provider::ProviderName;
        use crate::product::models::{IssueRecord, IssueWorkItemDependencyEdge, WorkspaceType};
        use crate::product::project_store::{CreateProjectInput, ProjectStore};

        let tmp = tempfile::TempDir::new().unwrap();
        let paths = ProductAppPaths::new(tmp.path().join(".aria"));
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "f-a waiting fixture".to_string(),
                description: None,
            })
            .unwrap();
        let lifecycle = LifecycleStore::new(paths.clone());
        let now = chrono::Utc::now().to_rfc3339();
        write_json(
            &paths.issue_root(PROJECT_ID, ISSUE_ID).join("issue.json"),
            &IssueRecord {
                id: ISSUE_ID.to_string(),
                project_id: PROJECT_ID.to_string(),
                repo_id: None,
                logical_codebase_id: None,
                title: "f-a waiting fixture".to_string(),
                description: None,
                change_id: "f_a_waiting".to_string(),
                phase: crate::product::models::IssuePhase::Clarification,
                status: crate::product::models::IssueStatus::Draft,
                active_binding_id: None,
                created_at: now.clone(),
                updated_at: now,
                base_branch: None,
            },
        )
        .unwrap();

        let plan_id = "plan_fa_0001".to_string();
        let session = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                entity_id: plan_id.clone(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Fake,
                reviewer_provider: Some(ProviderName::Fake),
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            })
            .unwrap();
        let session_id = session.id.clone();

        let target = EnrollmentTarget::SingleRepository {
            repository_id: "repo_physical_fa".to_string(),
        };
        let source = EnrollmentSource {
            stories: vec![],
            designs: vec![],
        };
        let options = EnrollmentOptions {
            author_provider: ProviderName::Fake,
            reviewer_provider: ProviderName::Fake,
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            plan_approval: PlanApprovalPolicy::Manual,
            plan_options: IssueWorkItemPlanOptions {
                include_integration_tests: true,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
        };
        let automation = IssueAutomationStore::new(paths.clone());
        automation
            .compare_and_set(
                PROJECT_ID,
                ISSUE_ID,
                None,
                EnrollmentWriteCommand::Enable {
                    command_id: None,
                    selection_key: "f_a_waiting_fixture".to_string(),
                    source: source.clone(),
                    options: options.clone(),
                    target: target.clone(),
                },
            )
            .unwrap();
        automation
            .bind_plan(PROJECT_ID, ISSUE_ID, 1, &plan_id, &session_id)
            .unwrap();
        // rebind 一次：同 plan/session 换代，history.previous 恰有一条。
        automation
            .rebind(
                PROJECT_ID,
                ISSUE_ID,
                EnrollmentRebindRequest {
                    command_id: "cmd-f-a-rebind-v2".to_string(),
                    expected_policy_revision: 2,
                    expected_binding_version: 1,
                    binding: EnrollmentBindingIdentityInput {
                        plan_id: plan_id.clone(),
                        session_id: session_id.clone(),
                        source,
                        target,
                        author_provider: ProviderName::Codex,
                        reviewer_provider: ProviderName::Fake,
                    },
                    reason: "f-a fixture rebind".to_string(),
                },
            )
            .unwrap();

        if with_plan_record {
            write_json(
                &paths
                    .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
                    .join("issue-work-item-plans")
                    .join(format!("{plan_id}.json")),
                &IssueWorkItemPlan {
                    id: plan_id.clone(),
                    project_id: PROJECT_ID.to_string(),
                    issue_id: ISSUE_ID.to_string(),
                    source_story_spec_ids: vec![],
                    source_design_spec_ids: vec![],
                    options: IssueWorkItemPlanOptions {
                        include_integration_tests: true,
                        include_e2e_tests: false,
                        force_frontend_backend_split: false,
                        require_execution_plan_confirm: false,
                    },
                    status: IssueWorkItemPlanStatus::Draft,
                    work_item_ids: vec![],
                    repository_profile_ref: None,
                    verification_plan_ids: vec![],
                    dependency_graph: vec![IssueWorkItemDependencyEdge {
                        from_work_item_id: "a".to_string(),
                        to_work_item_id: "b".to_string(),
                    }],
                    created_from_provider_run: None,
                    validator_findings: vec![],
                    review_summary: None,
                    ever_confirmed: None,
                    created_at: "2026-10-25T00:00:00Z".to_string(),
                    updated_at: "2026-10-25T00:00:00Z".to_string(),
                },
            )
            .unwrap();
        }
        (tmp, paths, plan_id, session_id)
    }

    /// F-A：当前绑定健在（plan 记录与 session 均存在）时，换代历史降级为
    /// 只读信息——无 rebind 动作、requires_human=false（不进「需要人工
    /// 处理」与待处理计数）。
    #[test]
    fn f_a_generation_history_demotes_to_read_only_info_while_binding_live() {
        let (_tmp, paths, plan_id, session_id) = f_a_waiting_fixture(true);
        let items = list_c1_waiting_items(&paths, PROJECT_ID, ISSUE_ID).unwrap();
        let generation = items
            .iter()
            .find(|item| item.kind == "generation_history")
            .expect("generation history stays queryable");
        assert!(
            generation.reason.contains("current binding active"),
            "reason must state the live binding: {generation:?}"
        );
        assert!(
            generation.actions.is_empty(),
            "live binding must not offer rebind: {generation:?}"
        );
        assert_eq!(generation.next_phase, None);
        assert!(!generation.requires_human);
        assert_eq!(generation.plan_id.as_deref(), Some(plan_id.as_str()));
        assert_eq!(generation.session_id.as_deref(), Some(session_id.as_str()));
    }

    /// F-A：当前绑定失效（plan 记录缺失，如单 plan 级联删除）时，换代历史
    /// 保持换代待办——rebind 动作 + requires_human=true。
    #[test]
    fn f_a_generation_history_stays_actionable_when_binding_dangling() {
        let (_tmp, paths, _plan_id, _session_id) = f_a_waiting_fixture(false);
        let items = list_c1_waiting_items(&paths, PROJECT_ID, ISSUE_ID).unwrap();
        let generation = items
            .iter()
            .find(|item| item.kind == "generation_history")
            .expect("generation history stays queryable");
        assert!(
            generation
                .reason
                .contains("current binding dangling (plan)"),
            "reason must name the dangling leg: {generation:?}"
        );
        assert_eq!(generation.actions, vec!["rebind".to_string()]);
        assert_eq!(generation.next_phase.as_deref(), Some("rebind"));
        assert!(generation.requires_human);
    }

    /// F-A：租约活跃（自动链到期自动恢复）是自动等待——requires_human=false，
    /// 无动作；死租约的接管项不在此夹具出现（见 c1 测试既有覆盖）。
    #[test]
    fn f_a_lease_wait_is_auto_waiting_not_needs_human() {
        use crate::product::coding_attempt_store::{CodingAttemptStore, CreateCodingAttemptInput};
        use crate::product::coding_models::CodingAttemptStatus;
        use crate::product::json_store::write_json;
        use crate::product::models::lifecycle::IssueSharedWorktree;
        use crate::product::models::project::IssueSharedWorktreeStatus;
        use crate::product::models::provider::ProviderName;
        use crate::web::workspace_ws_types::ProviderConfigSnapshot;

        let (_tmp, paths, _plan_id, _session_id) = f_a_waiting_fixture(true);
        let store = CodingAttemptStore::new(paths.clone());
        let attempt = store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: "work_item_fa_0001".to_string(),
                base_branch: "main".to_string(),
                branch_name: "aria/fa-running".to_string(),
                worktree_path: None,
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: ProviderName::Fake,
                    reviewer: None,
                    review_rounds: 0,
                    permission_modes: Default::default(),
                },
                target_snapshot: None,
                max_auto_rework: 0,
            })
            .unwrap();
        let mut running = attempt.clone();
        running.status = CodingAttemptStatus::Running;
        store.write_coding_attempt_for_test(&running).unwrap();
        write_json(
            &paths
                .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
                .join("issue-shared-worktree.json"),
            &IssueSharedWorktree {
                id: "issue_shared_worktree_fa".to_string(),
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                repository_id: "repo_physical_fa".to_string(),
                target_repository_id: None,
                checkout_id: None,
                path_schema_version: 0,
                branch_name: "aria/issue-shared-fa".to_string(),
                worktree_path: std::path::PathBuf::from("/tmp/fa-shared-worktree"),
                base_branch: "main".to_string(),
                status: IssueSharedWorktreeStatus::Running,
                current_active_work_item_id: Some("work_item_fa_0001".to_string()),
                current_lock_owner_id: Some(running.id.clone()),
                last_completed_work_item_id: None,
                created_at: "2026-10-25T00:00:00Z".to_string(),
                updated_at: "2026-10-25T00:00:00Z".to_string(),
            },
        )
        .unwrap();

        let items = list_c1_waiting_items(&paths, PROJECT_ID, ISSUE_ID).unwrap();
        let lease_wait = items
            .iter()
            .find(|item| item.kind == "lease_wait")
            .expect("active lease must project lease_wait");
        assert!(
            lease_wait.reason.contains("lease active"),
            "reason must carry the holder lease: {lease_wait:?}"
        );
        assert!(lease_wait.actions.is_empty());
        assert!(!lease_wait.requires_human);
        assert!(
            !items
                .iter()
                .any(|item| item.kind == "lease_takeover" || item.kind == "lease_unknown"),
            "active lease must not project takeover/unknown noise: {items:?}"
        );
    }

    /// F-A additive 契约：旧响应缺 `requires_human` 字段按需要人工解释
    /// （fail-safe 不静默降级真待办）；新响应显式携带该字段。
    #[test]
    fn f_a_requires_human_defaults_true_for_old_responses() {
        let old_response = r#"{
            "id": "c1:lease_wait:issue_0001:lease_0001",
            "kind": "lease_wait",
            "reason": "lease active; automation keeps waiting",
            "completed_steps": [],
            "target": null,
            "plan_id": null,
            "session_id": null,
            "attempt_id": null,
            "gate_id": null,
            "possible_side_effect": null,
            "actions": [],
            "next_phase": null
        }"#;
        let parsed: C1WaitingItemDto = serde_json::from_str(old_response).unwrap();
        assert!(parsed.requires_human, "旧响应缺省按需要人工解释");
        let serialized = serde_json::to_value(&parsed).unwrap();
        assert_eq!(serialized["requires_human"], serde_json::json!(true));
    }
}
