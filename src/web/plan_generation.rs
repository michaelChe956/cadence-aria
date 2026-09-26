//! P1 WIGA Task 5：plan 生成动作与真实 manager run 的协调（REQ-WIGA-03）。
//!
//! `start_plan_generation_once` 只做：活 run 观察（AlreadyActive，绝不
//! supersede）→ durable session 准入（Prepare/Interactive/无人工停点）→
//! enrollment 锁内认领生成检查点（`PlanGenerationIntent`）→ 共用非
//! superseding provider 启动（`spawn_provider_run_claiming_idle`）。检查点
//! 在 `EngineStarted`/`ProviderDispatched` 之后无法证明外部 provider 未被
//! 触达时持久 `NeedsHuman`，只供人显式恢复——绝不宣称 exactly-once。

use crate::product::issue_automation_store::IssueAutomationStore;
use crate::product::models::automation::{
    IssueAutomationEnrollment, PlanGenerationPhase, PlanGenerationIntent,
};
use crate::product::models::{SingleCandidatePhase, WorkspaceSessionRecord, WorkspaceSessionStatus, WorkspaceType};
use crate::product::work_item_plan_policy::RunPolicy;
use crate::web::state::WebAppState;
use crate::web::workspace_session::WorkspaceSessionManager;
use crate::web::workspace_ws_handler::ProviderRunKind;

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

    // durable session 准入：Prepare/Interactive/无人工或终态停点才可启动。
    let app_paths =
        crate::product::app_paths::ProductAppPaths::new(state.workspace_root.join(".aria"));
    let store = IssueAutomationStore::new(app_paths.clone());
    let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(app_paths.clone());
    let session = lifecycle
        .get_workspace_session(&session_id)
        .map_err(|error| format!("bound session unreadable: {error}"))?;
    match admission_outcome(&session) {
        Some(outcome) => return Ok(outcome),
        None => {}
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
        PlanGenerationPhase::NeedsHuman => return Ok(PlanGenerationOutcome::NeedsHuman),
        PlanGenerationPhase::EngineStarted | PlanGenerationPhase::ProviderDispatched => {
            // 无活 run 却已越过 EngineStarted：SC ledger/外部 provider 是否已被
            // 触达不可证明——持久 NeedsHuman，绝不重复 spawn。
            let _ = store.mark_plan_generation_phase(
                &enrollment.project_id,
                &enrollment.issue_id,
                &enrollment.enrollment_id,
                PlanGenerationPhase::NeedsHuman,
            );
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
/// Interactive 的 WorkItemPlan session 可启动；停点显式分诊，其余 NeedsHuman。
fn admission_outcome(session: &WorkspaceSessionRecord) -> Option<PlanGenerationOutcome> {
    if session.workspace_type != WorkspaceType::WorkItemPlan
        || session.run_policy != RunPolicy::Interactive
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

/// 检查点只读投影（编排器/诊断用）。
pub fn generation_intent(
    state: &WebAppState,
    enrollment: &IssueAutomationEnrollment,
) -> Result<Option<PlanGenerationIntent>, String> {
    let paths =
        crate::product::app_paths::ProductAppPaths::new(state.workspace_root.join(".aria"));
    let intent_path = paths
        .issue_root(&enrollment.project_id, &enrollment.issue_id)
        .join("automation-generation-intent.json");
    if intent_path.metadata().is_err() {
        return Ok(None);
    }
    crate::product::json_store::read_json::<PlanGenerationIntent>(&intent_path)
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
        assert_eq!(admission_outcome(&session), None);

        session.status = WorkspaceSessionStatus::WaitingForHuman;
        assert_eq!(
            admission_outcome(&session),
            Some(PlanGenerationOutcome::WaitingForHuman)
        );
        session.status = WorkspaceSessionStatus::StoppedNeedsHuman;
        assert_eq!(
            admission_outcome(&session),
            Some(PlanGenerationOutcome::WaitingForHuman)
        );
        session.status = WorkspaceSessionStatus::Failed;
        assert_eq!(
            admission_outcome(&session),
            Some(PlanGenerationOutcome::NeedsHuman)
        );
        session.status = WorkspaceSessionStatus::Confirmed;
        assert_eq!(
            admission_outcome(&session),
            Some(PlanGenerationOutcome::NeedsHuman)
        );

        // SC ledger 已预留（Generate）→ 不再自动启动。
        session.status = WorkspaceSessionStatus::Open;
        session.single_candidate_phase = Some(SingleCandidatePhase::Generate);
        assert_eq!(
            admission_outcome(&session),
            Some(PlanGenerationOutcome::WaitingForHuman)
        );
        // 非 Interactive 不代表授权（REQ-WIGA-02）。
        session.single_candidate_phase = Some(SingleCandidatePhase::Prepare);
        session.run_policy = RunPolicy::AutoIfValid;
        assert_eq!(
            admission_outcome(&session),
            Some(PlanGenerationOutcome::NeedsHuman)
        );
    }
}
