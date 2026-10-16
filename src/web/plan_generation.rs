//! P1 WIGA Task 5：plan 生成动作与真实 manager run 的协调（REQ-WIGA-03）。
//!
//! `start_plan_generation_once` 只做：活 run 观察（AlreadyActive，绝不
//! supersede）→ durable session 准入（Prepare/冻结期望 run policy/无人工
//! 停点；批 2 P2 起按 enrollment 冻结 plan_approval 互证）→ enrollment
//! 锁内认领生成检查点（`PlanGenerationIntent`）→ 共用非
//! superseding provider 启动（`spawn_provider_run_claiming_idle`）。检查点
//! 在 `EngineStarted`/`ProviderDispatched` 之后无法证明外部 provider 未被
//! 触达时持久 `NeedsHuman`，只供人显式恢复——绝不宣称 exactly-once。

use crate::product::issue_automation_store::IssueAutomationStore;
use crate::product::models::automation::{IssueAutomationEnrollment, PlanGenerationPhase};
use crate::product::models::{
    SingleCandidatePhase, WorkspaceSessionRecord, WorkspaceSessionStatus, WorkspaceType,
};
use crate::product::work_item_plan_policy::RunPolicy;
use crate::web::state::WebAppState;
use crate::web::workspace_session::WorkspaceSessionManager;
use crate::web::workspace_ws_handler::ProviderRunKind;

// ─── 批 2 P4（REQ-WIGA-12）：plan 阶段 attention 事实的稳定 key 与写/清 ───

/// 生成检查点派发前失败（intent NeedsHuman / EngineStarted/ProviderDispatched
/// 无活 run 转 NeedsHuman）的稳定事实 key：与 `PlanGenerationIntent::action_key`
/// 同源，重复停点不增项。
pub(crate) fn plan_generation_failed_fact_key(enrollment_id: &str, plan_id: &str) -> String {
    format!(
        "{}:needs_human",
        crate::product::models::automation::PlanGenerationIntent::action_key_for(
            enrollment_id,
            plan_id
        )
    )
}

/// plan 人工门（StoppedNeedsHuman/WaitingForHuman 的 HumanConfirm 门）事实 key。
pub(crate) fn plan_gate_fact_key(session_id: &str) -> String {
    format!("plan_gate:{session_id}:human_confirm")
}

/// plan 阶段挂起 choice 的事实 key（逐 choice 一条）。
pub(crate) fn plan_choice_fact_key(session_id: &str, choice_id: &str) -> String {
    format!("plan_choice:{session_id}:{choice_id}")
}

/// plan 会话 durable Failed（provider 失败收敛）的稳定事实 key：单会话一条，
/// 重复停点 upsert 不增项、`occurred_at` 首次冻结。
pub(crate) fn plan_failed_node_fact_key(session_id: &str) -> String {
    format!("plan_failed_node:{session_id}")
}

/// 批 2 P4（REQ-WIGA-12）：plan 事实写/清的精确绑定互证。仅当 enrollment
/// 存在、enabled、冻结 auto 授权且当前代绑定精确指向该 session 时才可写
///（手动/未绑定/禁用/旧代零写入）；绑定读取复用 advance 的
/// `load_current_enrollment_binding`（漂移 fail-closed）。
pub(crate) fn bound_auto_enrollment_for_plan_session(
    app_paths: &crate::product::app_paths::ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    session_id: &str,
) -> Result<
    Option<(
        IssueAutomationEnrollment,
        crate::product::models::automation::EnrollmentBindingIdentity,
    )>,
    String,
> {
    let enrollment = IssueAutomationStore::new(app_paths.clone())
        .get(project_id, issue_id)
        .map_err(|error| format!("automation enrollment unreadable: {error}"))?;
    let Some(enrollment) = enrollment else {
        return Ok(None);
    };
    if !enrollment.enabled
        || !matches!(
            enrollment.options.plan_approval,
            crate::product::models::automation::PlanApprovalPolicy::Auto
        )
    {
        return Ok(None);
    }
    if enrollment.session_id.as_deref() != Some(session_id) {
        return Ok(None);
    }
    let binding = crate::web::advance_plan::load_current_enrollment_binding(&enrollment)?;
    if binding.session_id != session_id {
        return Ok(None);
    }
    Ok(Some((enrollment, binding)))
}

/// 批 2 P4（REQ-WIGA-12）：按精确绑定 upsert plan 阶段事实；写失败原样上抛
///（不假成功）。时间戳由 store 首写冻结（重入保留）。
pub(crate) fn upsert_plan_attention_fact(
    app_paths: &crate::product::app_paths::ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    binding: &crate::product::models::automation::EnrollmentBindingIdentity,
    fact: crate::product::models::automation::AutomationAttentionFact,
) -> Result<(), String> {
    IssueAutomationStore::new(app_paths.clone())
        .upsert_automation_attention_fact(project_id, issue_id, binding, fact)
        .map_err(|error| format!("plan attention fact upsert failed: {error}"))
}

/// 批 2 P4（REQ-WIGA-12）：按稳定 key 清除 plan 阶段事实（继续成功/关门
/// resolve）；key 不存在幂等成功；写失败原样上抛。
pub(crate) fn resolve_plan_attention_fact(
    app_paths: &crate::product::app_paths::ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    binding: &crate::product::models::automation::EnrollmentBindingIdentity,
    key: &str,
) -> Result<(), String> {
    IssueAutomationStore::new(app_paths.clone())
        .resolve_automation_attention_fact(project_id, issue_id, binding, key)
        .map_err(|error| format!("plan attention fact resolve failed: {error}"))
}

/// 批 2 P4（REQ-WIGA-12）：`start_plan_generation_once` 的 intent NeedsHuman /
/// EngineStarted/ProviderDispatched 无活 run 两分支写 `automation_failed`
/// 事实（稳定 key：`{action_key}:needs_human`，重复停点不增项、零重复
/// provider）。仅精确绑定的 auto 授权写（绑定读取失败/手动零写入，不阻断
/// NeedsHuman 停等本身）；事实写入失败原样上抛（不假成功）。
fn record_generation_needs_human_fact(
    app_paths: &crate::product::app_paths::ProductAppPaths,
    enrollment: &IssueAutomationEnrollment,
    reason_code: &str,
) -> Result<(), String> {
    if !matches!(
        enrollment.options.plan_approval,
        crate::product::models::automation::PlanApprovalPolicy::Auto
    ) || !enrollment.enabled
    {
        return Ok(());
    }
    let Ok(binding) = crate::web::advance_plan::load_current_enrollment_binding(enrollment) else {
        return Ok(());
    };
    let fact = crate::product::models::automation::AutomationAttentionFact {
        key: plan_generation_failed_fact_key(&enrollment.enrollment_id, &binding.plan_id),
        kind: crate::product::models::automation::AutomationAttentionKind::AutomationFailed,
        stage: crate::product::models::automation::AutomationAttentionStage::Plan,
        plan_id: Some(binding.plan_id.clone()),
        session_id: Some(binding.session_id.clone()),
        attempt_id: None,
        reason_code: reason_code.to_string(),
        occurred_at: chrono::Utc::now().to_rfc3339(),
        choice_id: None,
        gate_id: None,
    };
    upsert_plan_attention_fact(
        app_paths,
        &enrollment.project_id,
        &enrollment.issue_id,
        &binding,
        fact,
    )
}

/// 生成动作的观察结果；状态只从 durable 事实与 manager 临界区快照推导。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanGenerationOutcome {
    /// provider run 已由本调用驱动（或此前已驱动且检查点已 Dispatched）。
    Running,
    /// 活 run 已存在（任何来源）：只观察，不 supersede、不重复 spawn。
    AlreadyActive,
    /// durable 停点（WaitingForHuman / compile recovery / 门开启）：等待人工。
    WaitingForHuman,
    /// 无法安全恢复（不可证明的 provider 分诊 / 损坏 / 授权漂移）：停等人。
    NeedsHuman,
}

/// 绑定 plan session 的唯一生成入口。事件只是 wake hint；每次调用从
/// durable enrollment/session/检查点事实重推导。
pub async fn start_plan_generation_once(
    state: &WebAppState,
    enrollment: &IssueAutomationEnrollment,
) -> Result<PlanGenerationOutcome, String> {
    let Some(session_id) = enrollment.session_id.clone() else {
        return Ok(PlanGenerationOutcome::NeedsHuman);
    };
    let manager = state
        .workspace_sessions
        .get_or_create(&session_id, || {
            WorkspaceSessionManager::create(state, &session_id)
        })
        .await?;

    // 活 run 观察（REQ-WIGA-03）：任何来源的活 run 都不 supersede。
    if manager.is_active_run() {
        return Ok(PlanGenerationOutcome::AlreadyActive);
    }

    // durable session 准入：Prepare+冻结期望 run policy+无人工或终态停点才可启动。
    let app_paths =
        crate::product::app_paths::ProductAppPaths::new(state.workspace_root.join(".aria"));
    let store = IssueAutomationStore::new(app_paths.clone());
    let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(app_paths.clone());
    let session = lifecycle
        .get_workspace_session(&session_id)
        .map_err(|error| format!("bound session unreadable: {error}"))?;
    // 批 2 P2（REQ-WIGA-02 修订）：会话 run policy 必须与 enrollment 冻结
    // plan_approval 期望一致（auto → AutoIfValid、manual → Interactive）才可
    // 驱动——错绑/降级一律 NeedsHuman；该互证先于 G8 委托返修重驱，任何
    // 路径都不得以错绑会话启动 provider。
    let expected_run_policy = enrollment.options.plan_approval.expected_run_policy();
    if session.workspace_type != WorkspaceType::WorkItemPlan
        || session.run_policy != expected_run_policy
    {
        return Ok(PlanGenerationOutcome::NeedsHuman);
    }
    // G8（终局关闸缺口）：generate 相位委托返修接力孤儿——review 裁决返修
    // 后 TriggerAggregateRepair 已原子预领（repair_reservation=Reserved＋
    // ledger 追加），但接力 spawn 未落地（进程重启→manager 重建回落
    // generate 相位人工门，无快照无轮次；REST/WS 批准面按相位门语义
    // fail-closed 正确拒绝）。Reserved 未被接力消费＝provider 可证明未启动：
    // enrolled 自动链在此直接重驱 WorkItemPlanSingleCandidateAuthor（唯一
    // 接续路径），预留 CAS（Reserved→ProviderStarted 恰一次）保证不二次
    // 启动；已消费（ProviderStarted/Committed/Released）不重驱，交由
    // admission 停等分诊。
    if delegated_rerun_orphan(&session) {
        let mut run_context = manager.provider_run_context(state.workspace_runs.clone());
        run_context.connection_id = None;
        let (outbound_tx, _dropped_outbound_rx) = tokio::sync::mpsc::channel(1);
        let started = crate::web::workspace_ws_handler::spawn_provider_run_claiming_idle(
            run_context,
            ProviderRunKind::WorkItemPlanSingleCandidateAuthor,
            outbound_tx,
        )
        .await?;
        return Ok(if started {
            PlanGenerationOutcome::Running
        } else {
            PlanGenerationOutcome::AlreadyActive
        });
    }
    if let Some(outcome) = admission_outcome(&session, expected_run_policy) {
        return Ok(outcome);
    }

    // enrollment 锁内认领稳定检查点；按 phase 分诊。
    let intent = store
        .claim_plan_generation(
            &enrollment.project_id,
            &enrollment.issue_id,
            &enrollment.enrollment_id,
        )
        .map_err(|error| format!("plan generation claim failed: {error}"))?;
    match intent.phase {
        PlanGenerationPhase::NeedsHuman => {
            // 批 2 P4（REQ-WIGA-12）：派发前失败/停等落 durable 事实——无活
            // 窗口仍可通知（N1 只读投影）；精确绑定才写、手动零写入、写失败
            // 上抛不假成功。
            record_generation_needs_human_fact(
                &app_paths,
                enrollment,
                "plan_generation_needs_human",
            )?;
            return Ok(PlanGenerationOutcome::NeedsHuman);
        }
        PlanGenerationPhase::EngineStarted | PlanGenerationPhase::ProviderDispatched => {
            // 无活 run 却已越过 EngineStarted：SC ledger/外部 provider 是否已被
            // 触达不可证明——持久 NeedsHuman，绝不重复 spawn。
            let _ = store.mark_plan_generation_phase(
                &enrollment.project_id,
                &enrollment.issue_id,
                &enrollment.enrollment_id,
                PlanGenerationPhase::NeedsHuman,
            );
            record_generation_needs_human_fact(
                &app_paths,
                enrollment,
                "plan_generation_dispatch_unproven",
            )?;
            return Ok(PlanGenerationOutcome::NeedsHuman);
        }
        PlanGenerationPhase::Claimed => {}
    }
    // Claimed 之外的安全网：durable phase 已 Generate（SC ledger 已预留）而检查
    // 点仍 Claimed（崩溃于预留之后、推进之前）——同样不可证明，停等人。
    if session.single_candidate_phase == Some(SingleCandidatePhase::Generate) {
        let _ = store.mark_plan_generation_phase(
            &enrollment.project_id,
            &enrollment.issue_id,
            &enrollment.enrollment_id,
            PlanGenerationPhase::NeedsHuman,
        );
        return Ok(PlanGenerationOutcome::NeedsHuman);
    }

    store
        .mark_plan_generation_phase(
            &enrollment.project_id,
            &enrollment.issue_id,
            &enrollment.enrollment_id,
            PlanGenerationPhase::EngineStarted,
        )
        .map_err(|error| format!("plan generation engine-start mark failed: {error}"))?;

    // P1 WIGA Task 10：测试注入的 EngineStarted 中窗（provider 派发之前）：
    // 检查点已越过 EngineStarted 而进程消失，重启后无法证明 provider 未被
    // 触达——必须走 NeedsHuman 人工分诊，绝不重发 provider。
    #[cfg(test)]
    if crate::product::issue_automation_store::automation_crash_window::fire_once(
        crate::product::issue_automation_store::automation_crash_window::CrashWindow::AfterEngineStarted,
    ) {
        return Err("automation_crash_window: interrupted after engine started".to_string());
    }

    // 共用非 superseding provider drive：manager 临界区内只认领空闲 session。
    let mut run_context = manager.provider_run_context(state.workspace_runs.clone());
    run_context.connection_id = None;
    let (outbound_tx, _detached_outbound_rx) = tokio::sync::mpsc::channel(1);
    let started = crate::web::workspace_ws_handler::spawn_provider_run_claiming_idle(
        run_context,
        ProviderRunKind::WorkItemPlanSingleCandidateAuthor,
        outbound_tx,
    )
    .await?;
    if !started {
        return Ok(PlanGenerationOutcome::AlreadyActive);
    }
    store
        .mark_plan_generation_phase(
            &enrollment.project_id,
            &enrollment.issue_id,
            &enrollment.enrollment_id,
            PlanGenerationPhase::ProviderDispatched,
        )
        .map_err(|error| format!("plan generation dispatch mark failed: {error}"))?;
    Ok(PlanGenerationOutcome::Running)
}

/// durable 准入：仅 Prepare+Open（或 Running 但无活 run 的早期窗口）且
/// run policy 与 enrollment 冻结期望一致的 WorkItemPlan session 可启动；
/// 停点显式分诊，其余 NeedsHuman（错绑/降级不获自动执行权）。
fn admission_outcome(
    session: &WorkspaceSessionRecord,
    expected_run_policy: RunPolicy,
) -> Option<PlanGenerationOutcome> {
    if session.workspace_type != WorkspaceType::WorkItemPlan
        || session.run_policy != expected_run_policy
    {
        return Some(PlanGenerationOutcome::NeedsHuman);
    }
    match session.status {
        WorkspaceSessionStatus::WaitingForHuman | WorkspaceSessionStatus::StoppedNeedsHuman => {
            Some(PlanGenerationOutcome::WaitingForHuman)
        }
        WorkspaceSessionStatus::Failed
        | WorkspaceSessionStatus::Terminated
        | WorkspaceSessionStatus::Confirmed
        | WorkspaceSessionStatus::ChangeRequested
        | WorkspaceSessionStatus::BlockedProviderUnavailable => {
            Some(PlanGenerationOutcome::NeedsHuman)
        }
        WorkspaceSessionStatus::Open | WorkspaceSessionStatus::Running => {
            if session.single_candidate_phase == Some(SingleCandidatePhase::Prepare) {
                None
            } else {
                // Generate/Evaluate/Approval/Completed/Failed：不在初始生成位。
                Some(PlanGenerationOutcome::WaitingForHuman)
            }
        }
    }
}

/// G8（终局关闸缺口）：generate 相位委托返修接力孤儿判据。TriggerAggregateRepair
/// 为 SC 返修原子预领 `repair_reservation`（Reserved＋ledger 追加），接力 run 的
/// reserve 恰一次消费它（Reserved→ProviderStarted）。Reserved 残留且无活 run
///（调用方已观察）＝接力 spawn 从未落地＝外部 provider 可证明未启动，重驱安全；
/// 其余状态（已启动/已提交/已释放）均不重驱。
fn delegated_rerun_orphan(session: &WorkspaceSessionRecord) -> bool {
    session.single_candidate_phase == Some(SingleCandidatePhase::Generate)
        && session
            .repair_reservation
            .as_ref()
            .is_some_and(|reservation| {
                matches!(
                    reservation.state,
                    crate::product::work_item_plan_policy::RepairReservationState::Reserved
                )
            })
}

/// 检查点只读投影（编排器/诊断用）。C5 Task 1：经版本化读侧——旧格式
/// 检查点（仅含已废弃 `logical_repository_id`、无 `target`）读入为
/// `LegacyUnbound`（旧代无绑定），调用者据此停止自动链；不因 serde 缺
/// 字段直接反序列化失败。
pub fn generation_intent(
    state: &WebAppState,
    enrollment: &IssueAutomationEnrollment,
) -> Result<Option<crate::product::issue_automation_store::PlanGenerationIntentRead>, String> {
    let paths = crate::product::app_paths::ProductAppPaths::new(state.workspace_root.join(".aria"));
    let intent_path = paths
        .issue_root(&enrollment.project_id, &enrollment.issue_id)
        .join("automation-generation-intent.json");
    if intent_path.metadata().is_err() {
        return Ok(None);
    }
    crate::product::issue_automation_store::read_plan_generation_intent(&intent_path)
        .map(Some)
        .map_err(|error| format!("plan generation checkpoint unreadable: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admission_stops_at_human_and_terminal_states() {
        let mut session = crate::web::workspace_session::test_session_record("s");
        session.workspace_type = WorkspaceType::WorkItemPlan;
        session.run_policy = RunPolicy::Interactive;
        session.single_candidate_phase = Some(SingleCandidatePhase::Prepare);
        session.status = WorkspaceSessionStatus::Open;
        assert_eq!(admission_outcome(&session, RunPolicy::Interactive), None);

        session.status = WorkspaceSessionStatus::WaitingForHuman;
        assert_eq!(
            admission_outcome(&session, RunPolicy::Interactive),
            Some(PlanGenerationOutcome::WaitingForHuman)
        );
        session.status = WorkspaceSessionStatus::StoppedNeedsHuman;
        assert_eq!(
            admission_outcome(&session, RunPolicy::Interactive),
            Some(PlanGenerationOutcome::WaitingForHuman)
        );
        session.status = WorkspaceSessionStatus::Failed;
        assert_eq!(
            admission_outcome(&session, RunPolicy::Interactive),
            Some(PlanGenerationOutcome::NeedsHuman)
        );
        session.status = WorkspaceSessionStatus::Confirmed;
        assert_eq!(
            admission_outcome(&session, RunPolicy::Interactive),
            Some(PlanGenerationOutcome::NeedsHuman)
        );

        // SC ledger 已预留（Generate）→ 不再自动启动。
        session.status = WorkspaceSessionStatus::Open;
        session.single_candidate_phase = Some(SingleCandidatePhase::Generate);
        assert_eq!(
            admission_outcome(&session, RunPolicy::Interactive),
            Some(PlanGenerationOutcome::WaitingForHuman)
        );
        // 错绑：manual 冻结（期望 Interactive）对 AutoIfValid 会话不获准
        //（REQ-WIGA-02 修订——AutoIfValid 只对 auto 冻结放行）。
        session.single_candidate_phase = Some(SingleCandidatePhase::Prepare);
        session.run_policy = RunPolicy::AutoIfValid;
        assert_eq!(
            admission_outcome(&session, RunPolicy::Interactive),
            Some(PlanGenerationOutcome::NeedsHuman)
        );
    }

    // 批 2 P2（REQ-WIGA-02 修订）：生成准入按 enrollment 冻结策略互证——
    // auto 冻结放行 AutoIfValid 会话进入生成；错绑/降级（实际与期望
    // 不一致）一律 NeedsHuman，仅客户端传 AutoIfValid 不获自动执行权。
    #[test]
    fn admission_follows_enrollment_expected_run_policy() {
        let mut session = crate::web::workspace_session::test_session_record("s");
        session.workspace_type = WorkspaceType::WorkItemPlan;
        session.single_candidate_phase = Some(SingleCandidatePhase::Prepare);
        session.status = WorkspaceSessionStatus::Open;

        // auto 冻结 + AutoIfValid 会话：准入放行（None = 继续生成链）。
        session.run_policy = RunPolicy::AutoIfValid;
        assert_eq!(admission_outcome(&session, RunPolicy::AutoIfValid), None);

        // 错绑：期望 Interactive 而会话 AutoIfValid → 拒绝。
        assert_eq!(
            admission_outcome(&session, RunPolicy::Interactive),
            Some(PlanGenerationOutcome::NeedsHuman)
        );

        // 降级：auto 冻结对 Interactive 会话不代为执行 → 拒绝。
        session.run_policy = RunPolicy::Interactive;
        assert_eq!(
            admission_outcome(&session, RunPolicy::AutoIfValid),
            Some(PlanGenerationOutcome::NeedsHuman)
        );

        // manual 回归：Interactive 会话 + Interactive 期望 → 准入。
        assert_eq!(admission_outcome(&session, RunPolicy::Interactive), None);
    }

    // G8：委托返修接力孤儿判据——仅 Generate 相位＋Reserved 预领未被消费
    //（provider 可证明未启动）才可重驱；已消费/已释放与其余相位不重驱。
    #[test]
    fn delegated_rerun_orphan_requires_generate_phase_with_unconsumed_reservation() {
        use crate::product::work_item_plan_policy::{RepairReservation, RepairReservationState};

        let mut session = crate::web::workspace_session::test_session_record("s");
        session.workspace_type = WorkspaceType::WorkItemPlan;
        session.run_policy = RunPolicy::Interactive;
        session.single_candidate_phase = Some(SingleCandidatePhase::Generate);
        let reservation = |state| {
            Some(RepairReservation {
                token: "single_candidate_author_repair:s:1".to_string(),
                owner_session_id: "s".to_string(),
                owner_run_id: "review_scope_v1:test".to_string(),
                provider_start_idempotency_key: "single_candidate_author:s:1".to_string(),
                state,
                commit_id: None,
            })
        };

        session.repair_reservation = reservation(RepairReservationState::Reserved);
        assert!(delegated_rerun_orphan(&session));

        session.repair_reservation = reservation(RepairReservationState::ProviderStarted);
        assert!(!delegated_rerun_orphan(&session));
        session.repair_reservation = reservation(RepairReservationState::Released);
        assert!(!delegated_rerun_orphan(&session));
        session.repair_reservation = None;
        assert!(!delegated_rerun_orphan(&session));

        session.single_candidate_phase = Some(SingleCandidatePhase::Prepare);
        session.repair_reservation = reservation(RepairReservationState::Reserved);
        assert!(!delegated_rerun_orphan(&session));
    }
}
