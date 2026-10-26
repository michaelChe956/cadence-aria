//! terminated 记录的人工重新开启（REQ-WIGA-09 修订／REQ-WPS-01/02/06）：
//! Enable 前的收口门与旧 plan 代际取代删除。
//!
//! 从 `automation_enrollment.rs` 净搬出（该文件已在 large_file_guard 登记表内
//! 且登记上限已超，禁止继续增长）；纯移动，语义与错误码逐字不变。

use crate::product::issue_automation_store::IssueAutomationStore;
use crate::product::models::automation::EnrollmentWriteCommand;
use crate::web::error::{ApiError, ApiResult};
use crate::web::handlers::support::{product_app_paths, product_store_api_error};
use crate::web::state::WebAppState;

/// terminated 重新开启时的旧 plan 代际取代删除（REQ-WPS-01/REQ-WPS-02/REQ-WPS-06）。
///
/// 删除 issue 下全部存活 plan（re-enable 后新代 plan 由新 intent 派生，旧代
/// 一律作废）：先收口未终结 attempt，再按同一闭包物理删除 attempts/worktree/
/// shared-worktree/reviewer 资源与 plan 根，只留一条无内容删除事实。
///
/// 失败 → 既有 `automation_plan_cleanup_required` 语义（收口失败零副作用）。
pub(crate) async fn supersede_terminated_plan_generation(
    state: &WebAppState,
    project_id: &str,
    issue_id: &str,
) -> ApiResult<()> {
    let app_paths = product_app_paths(state);
    let superseded_plan_id = IssueAutomationStore::new(app_paths.clone())
        .get(project_id, issue_id)
        .map_err(product_store_api_error)?
        .and_then(|enrollment| enrollment.plan_id);
    // 单 plan 不变量（REQ-WPS-01）：取代删除与新代建立落在同一 issue 级
    // plan-slot 互斥区（重新开启后的新代 plan 由 `ensure_enrolled_plan` 在
    // 同一互斥区内建立）。
    let plan_slot = crate::product::plan_generation_supersede::acquire_issue_plan_slot(
        &app_paths, project_id, issue_id,
    )
    .await
    .map_err(|error| {
        ApiError::validation_with_details(
            "automation_plan_cleanup_required",
            "automation plan still holds unfinished attempts or unclosed run facts; retry the \
             termination to finish cleanup before re-enabling the automation",
            serde_json::json!({
                "plan_id": superseded_plan_id.clone().unwrap_or_default(),
                "reason": "plan_generation_supersede_failed",
                "message": format!("{error:?}"),
            }),
        )
    })?;
    let report = crate::product::plan_generation_supersede::ensure_single_plan_generation(
        &app_paths,
        &plan_slot,
        project_id,
        issue_id,
        &[],
        crate::product::entity_deletion_cascade::PlanSupersedeReason::ReEnablePrepareIntentRotation,
        Some(state.workspace_root.as_path()),
    )
    .await
    .map_err(|error| {
        ApiError::validation_with_details(
            "automation_plan_cleanup_required",
            "automation plan still holds unfinished attempts or unclosed run facts; retry the \
             termination to finish cleanup before re-enabling the automation",
            serde_json::json!({
                "plan_id": superseded_plan_id.unwrap_or_default(),
                "reason": "plan_generation_supersede_failed",
                "message": format!("{error:?}"),
            }),
        )
    })?;
    crate::web::handlers::lifecycle::deletion::evict_workspace_session_managers(
        state,
        &report.evicted_session_ids,
    )
    .await;
    Ok(())
}

/// REQ-WIGA-09 修订：terminated 人工重新开启（Enable）的收口门。判据（顺序
/// 固定）：①绑定实体已删（墓碑，plan 记录已不存在）→ 无可收口之物，放行取代
/// （不因清理任务失败本身永久阻断）；②绑定实体仍在，但 terminated 记录仍持有
/// 未收口运行事实（终止清理失败写入的 `automation_failed(stage=termination)`
/// 等，resolve 后即消失）→ 409。
///
/// REQ-WPS-06 修订：绑定 plan 存在**未终结 attempt** 不再在此阻断——取代删除
/// 路径（`supersede_terminated_plan_generation`）先按既有终止收口语义把 attempt
/// 收口到终态，收口失败返回既有 `automation_plan_cleanup_required` 语义且零
/// 副作用（不得以「仍有 attempt」永久阻断取代）。全部 409 零副作用（store CAS
/// 在其后才执行）。
///
/// 预检与 CAS 之间无写入者：terminated 已阻断全部自动路径（reconcile/
/// ensure/provider run），人工 attempt 认领同样要求 enabled enrollment。
pub(crate) fn ensure_terminated_reenable_cleanup_closed(
    state: &WebAppState,
    project_id: &str,
    issue_id: &str,
    command: &EnrollmentWriteCommand,
) -> ApiResult<()> {
    if !matches!(command, EnrollmentWriteCommand::Enable { .. }) {
        return Ok(());
    }
    let paths = product_app_paths(state);
    let Some(enrollment) = IssueAutomationStore::new(paths.clone())
        .get(project_id, issue_id)
        .map_err(product_store_api_error)?
    else {
        return Ok(());
    };
    if enrollment.terminated_at.is_none() {
        return Ok(());
    }
    let Some(plan_id) = enrollment.plan_id.clone() else {
        // 从未绑定 plan → 无可收口之物（直接放行取代）。
        return Ok(());
    };
    // 绑定实体已删（墓碑）：plan 记录不在 → 无可收口之物，放行取代。
    let bound_entity_present = crate::product::lifecycle_store::LifecycleStore::new(paths)
        .list_issue_work_item_plans(project_id, issue_id)
        .map_err(product_store_api_error)?
        .iter()
        .any(|plan| plan.id == plan_id);
    if !bound_entity_present {
        return Ok(());
    }
    // 未收口的运行事实（terminated 记录上仍挂着待处理事实＝终止清理未收口）。
    if !enrollment.attention_facts.is_empty() {
        return Err(cleanup_required_error(&plan_id, "unclosed_run_facts"));
    }
    Ok(())
}

fn cleanup_required_error(plan_id: &str, reason: &str) -> ApiError {
    ApiError::validation_with_details(
        "automation_plan_cleanup_required",
        "automation plan still holds unfinished attempts or unclosed run facts; retry the \
         termination to finish cleanup before re-enabling the automation",
        serde_json::json!({ "plan_id": plan_id, "reason": reason }),
    )
}
