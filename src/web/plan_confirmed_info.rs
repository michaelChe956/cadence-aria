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
use crate::product::models::outline::{WorkItemPlanCompileStatus, WorkItemPlanCommitState};
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
    Ok(plan_confirmed_info(paths, &enrollment)?.into_iter().collect())
}

// ---------------------------------------------------------------------------
// C1 Task 9（enrollment-recovery-surface）：统一 C1 恢复等待项投影。
// 只从 durable 事实派生（孤儿候选快照、lease 三态、Failed advance、intent
// 停等、换代历史）；通知/WS 失败不回滚业务事实，驾驶舱经 GET lifecycle
// 补读本投影。不建通知表，不以事件当权威（tasks.md §3.2/§4.1）。
// ---------------------------------------------------------------------------

/// C1 恢复等待项 DTO（`IssueLifecycleResponse.c1_waiting_items` 条目）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct C1WaitingItemDto {
    pub id: String,
    /// candidate_recovery | lease_wait | lease_takeover | lease_unknown |
    /// advance_retry_failed | intent_blocked | generation_history
    pub kind: String,
    pub reason: String,
    pub completed_steps: Vec<String>,
    pub target: Option<crate::product::logical_codebase::EnrollmentTarget>,
    pub plan_id: Option<String>,
    pub session_id: Option<String>,
    pub attempt_id: Option<String>,
    pub gate_id: Option<String>,
    pub possible_side_effect: Option<String>,
    /// recover_candidate | confirm_takeover | retry_initialization | rebind
    pub actions: Vec<String>,
    pub next_phase: Option<String>,
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
    use crate::product::models::automation::{LeaseDisposition, LeaseDecision};
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
        if bound_session.workspace_type
            == crate::product::models::WorkspaceType::WorkItemPlan
        {
            if let Some(recovery) = bound_session
                .human_gate_snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.candidate_recovery.as_ref())
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
        if let Some(record) =
            advance_store.get_advance_for_plan(project_id, issue_id, plan_id)?
        {
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
                        finding.code == "intent_undeclared"
                            || finding.code == "intent_unexecutable"
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
            });
        }
    }

    Ok(items)
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
    ProductStoreError::Io(format!("plan publication provenance unreadable: {}", error.code()))
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
        assert!(!first.occurred_at.is_empty(), "occurred_at 取 tx.committed_at");
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
