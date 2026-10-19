//! P1-a（REQ-MTG-11 / 契约 A2）：plan 级「一键确认完成」后端。
//!
//! GET `.../coding-start` 的 `final_confirm` 投影 + 无 body POST
//! `/.../coding-final-confirm`。范围取当前 plan-run 冻结 targets（无 run 时
//! 按权威绑定派生已有 attempt 集，与 `build_plan_coding_progress` 同口径）：
//! 已 Completed 视为已确认；`WaitingForHuman ∧ FinalConfirm ∧ units 全
//! Completed ∧ readiness snapshot 身份匹配/Complete/空 diagnostics` 为就绪；
//! 其余进 `blocking`。GET 只看持久事实，不检查 git——owner/范围/git 仍由每
//! 项现有 `handle_final_confirm` 提交时校验（契约 A2：不宣称 GET 已检查
//! git）。POST 先对全部就绪 target 做只读 worktree clean 预检（无副作用，
//! 不落 manual gate），全过才逐个调用既有 `handle_final_confirm`（不改
//! 引擎）：按序跳过 Completed、首失败即停、其余 `not_attempted`。不做
//! 回执/防重放/跨仓事务（owner 已裁定剔除；原子性/半写/并发等已知风险
//! H-1~H-6 见块 1 设计 §11，批 1 不处理）。

use std::collections::BTreeSet;
use std::path::PathBuf;

use axum::Json;
use axum::extract::{Path, State};

use super::super::support::{
    coding_workspace_engine_with_dummy_events, product_app_paths, product_store_api_error,
};
use super::plan_start::{ensure_plan_confirmed, plan_live_admission};
use crate::product::advance_store::AdvanceTargetAttemptBinding;
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodingAttemptStatus, CodingExecutionAttempt, CodingExecutionStage, CodingExecutionUnit,
    CodingExecutionUnitStatus, GroupFinalReadinessStatus,
};
use crate::product::coding_plan_run::CodingPlanRunStore;
use crate::product::coding_workspace_engine::CodingWorkspaceEngineError;
use crate::product::git_workspace_service::GitWorkspaceService;
use crate::product::json_store::{ProductStoreError, validate_relative_id};
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::logical_codebase::LogicalRepositoryId;
use crate::web::error::{ApiError, ApiResult};
use crate::web::state::CodingAttemptRunKey;
use crate::web::state::WebAppState;
use crate::web::types::{
    PlanCodingFinalConfirmBlockingDto, PlanCodingFinalConfirmDto, PlanCodingFinalConfirmResultDto,
    PlanCodingFinalConfirmResultItemDto,
};

/// 单冻结 target 的聚合确认分类（GET 投影与 POST 预检共用）。
enum PlanFinalConfirmTargetState {
    /// 已人工确认（Completed）：跳过，不算失败。
    Confirmed,
    /// 持久事实齐备，可调用既有 `handle_final_confirm`。
    Ready,
    /// 未齐备：GET 进 `blocking`，POST 预检即 not_ready。
    Blocking {
        reason_code: String,
        message: String,
    },
}

struct PlanFinalConfirmTargetFacts {
    binding: AdvanceTargetAttemptBinding,
    /// `None` = 冻结成员 attempt 已被独立删除（REQ-CS-07 失效窗口）。
    attempt: Option<CodingExecutionAttempt>,
    units: Vec<CodingExecutionUnit>,
    state: PlanFinalConfirmTargetState,
}

fn blocking_state(reason_code: &str, message: String) -> PlanFinalConfirmTargetState {
    PlanFinalConfirmTargetState::Blocking {
        reason_code: reason_code.to_string(),
        message,
    }
}

/// 按持久事实分类单个 target（GET 就绪判据；与 `handle_final_confirm` 的
/// 前置同源，但不含 owner/lock/git——那些仍由引擎在提交时校验）。
fn classify_target_for_final_confirm(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    units: &[CodingExecutionUnit],
) -> Result<PlanFinalConfirmTargetState, ProductStoreError> {
    if attempt.status == CodingAttemptStatus::Completed {
        return Ok(PlanFinalConfirmTargetState::Confirmed);
    }
    if attempt.status != CodingAttemptStatus::WaitingForHuman {
        return Ok(blocking_state(
            "attempt_not_waiting_for_human",
            format!(
                "attempt {} is not waiting_for_human (stage={:?})",
                attempt.id, attempt.stage
            ),
        ));
    }
    if attempt.stage != CodingExecutionStage::FinalConfirm {
        return Ok(blocking_state(
            "attempt_not_in_final_confirm",
            format!(
                "attempt {} stage is not final_confirm (status=waiting_for_human)",
                attempt.id
            ),
        ));
    }
    if units.is_empty()
        || !units
            .iter()
            .all(|unit| unit.status == CodingExecutionUnitStatus::Completed)
    {
        return Ok(blocking_state(
            "units_not_completed",
            format!(
                "attempt {} has units that are not completed ({} units)",
                attempt.id,
                units.len()
            ),
        ));
    }
    let snapshot = match store.get_group_final_readiness_snapshot(attempt) {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => {
            return Ok(blocking_state(
                "readiness_snapshot_missing",
                format!(
                    "attempt {} has no group final readiness snapshot",
                    attempt.id
                ),
            ));
        }
        // 坏 snapshot（身份/记录校验失败）折叠为 blocking，不把畸形记录炸到
        // 整条 GET（与 handle_final_confirm 对外统一 FinalConfirmNotReady
        // 的诊断口径一致）；其余读取错误 fail-closed 上抛。
        Err(ProductStoreError::IdentityMismatch { .. })
        | Err(ProductStoreError::InvalidRecord { .. }) => {
            return Ok(blocking_state(
                "readiness_snapshot_invalid",
                format!(
                    "attempt {} group final readiness snapshot is invalid",
                    attempt.id
                ),
            ));
        }
        Err(error) => return Err(error),
    };
    if snapshot.attempt_id != attempt.id {
        return Ok(blocking_state(
            "readiness_snapshot_invalid",
            format!(
                "attempt {} readiness snapshot identity mismatch ({})",
                attempt.id, snapshot.attempt_id
            ),
        ));
    }
    if snapshot.status != GroupFinalReadinessStatus::Complete || !snapshot.diagnostics.is_empty() {
        return Ok(blocking_state(
            "readiness_incomplete",
            format!(
                "attempt {} readiness is not complete ({} diagnostics)",
                attempt.id,
                snapshot.diagnostics.len()
            ),
        ));
    }
    Ok(PlanFinalConfirmTargetState::Ready)
}

/// 冻结 target 集解析（与 `build_plan_coding_progress` 同口径）：run 冻结集
/// 优先；无 run 时读该 plan 的已有 attempt 集（Ready 集）。返回
/// `(plan_complete, facts)`——plan_complete 与 GET 现口径一致（全部权威 WI
/// 的 unit Completed，不含人工确认语义）。
fn plan_final_confirm_target_facts(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
) -> Result<(bool, Vec<PlanFinalConfirmTargetFacts>), ProductStoreError> {
    let store = CodingAttemptStore::new(paths.clone());
    let run = CodingPlanRunStore::new(paths.clone()).get(project_id, issue_id, plan_id)?;
    let authoritative = match &run {
        Some(run) => store.resolve_authoritative_group_plan_binding_for_revision(
            project_id,
            issue_id,
            plan_id,
            &run.bound_plan_revision_id,
        )?,
        None => store.resolve_authoritative_group_plan_binding(project_id, issue_id, plan_id)?,
    };

    let mut bindings = match run.filter(|run| !run.target_attempts.is_empty()) {
        Some(run) => run.target_attempts.clone(),
        None => {
            let targets: BTreeSet<LogicalRepositoryId> = authoritative
                .units
                .iter()
                .filter_map(|unit| unit.target_repository_id)
                .collect();
            let mut derived = Vec::new();
            for target in targets {
                if let Some(attempt) = store.get_attempt_for_work_item_group(
                    project_id,
                    issue_id,
                    plan_id,
                    Some(target),
                )? {
                    derived.push(AdvanceTargetAttemptBinding {
                        target_repository_id: target.0.to_string(),
                        attempt_id: attempt.id,
                    });
                }
            }
            derived
        }
    };
    bindings.sort_by(|left, right| {
        left.target_repository_id
            .cmp(&right.target_repository_id)
            .then_with(|| left.attempt_id.cmp(&right.attempt_id))
    });

    let mut facts = Vec::new();
    for binding in bindings {
        // REQ-CS-07：冻结成员被独立删除（NotFound）按失效 blocking 投影，
        // 不让单成员删除炸整条 GET；身份错配/IO/JSON 错误仍 fail-closed 上抛。
        let attempt = match store.get_attempt(project_id, issue_id, &binding.attempt_id) {
            Ok(attempt) => Some(attempt),
            Err(ProductStoreError::NotFound {
                kind: "coding_attempt",
                ..
            }) => None,
            Err(error) => return Err(error),
        };
        let units = match &attempt {
            Some(_) => store.list_coding_units(project_id, issue_id, &binding.attempt_id)?,
            None => Vec::new(),
        };
        let state = match &attempt {
            Some(attempt) => classify_target_for_final_confirm(&store, attempt, &units)?,
            None => blocking_state(
                "attempt_missing",
                format!(
                    "frozen target attempt {} is missing (independently deleted)",
                    binding.attempt_id
                ),
            ),
        };
        facts.push(PlanFinalConfirmTargetFacts {
            binding,
            attempt,
            units,
            state,
        });
    }

    let completed: BTreeSet<String> = facts
        .iter()
        .flat_map(|facts| {
            facts
                .units
                .iter()
                .filter(|unit| unit.status == CodingExecutionUnitStatus::Completed)
                .map(|unit| unit.logical_work_item_id.clone())
        })
        .collect();
    let plan_complete = !authoritative.units.is_empty()
        && authoritative
            .units
            .iter()
            .all(|unit| completed.contains(&unit.logical_work_item_id));
    Ok((plan_complete, facts))
}

/// 块 1 A2 共享纯函数：plan 级聚合最终确认投影（供 GET 接线与块 4
/// `coding_attention_info` 直接调用；只读持久事实，不构建整个 progress，
/// 不调用 attention/lifecycle，避免循环）。
pub(crate) fn build_plan_final_confirm(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
) -> Result<PlanCodingFinalConfirmDto, ProductStoreError> {
    let (plan_complete, facts) =
        plan_final_confirm_target_facts(paths, project_id, issue_id, plan_id)?;
    let mut dto = PlanCodingFinalConfirmDto::default();
    for facts in &facts {
        match &facts.state {
            PlanFinalConfirmTargetState::Confirmed => {
                dto.confirmed_attempt_ids
                    .push(facts.binding.attempt_id.clone());
            }
            PlanFinalConfirmTargetState::Ready => {
                dto.ready_attempt_ids.push(facts.binding.attempt_id.clone());
            }
            PlanFinalConfirmTargetState::Blocking {
                reason_code,
                message,
            } => dto.blocking.push(PlanCodingFinalConfirmBlockingDto {
                attempt_id: facts.binding.attempt_id.clone(),
                reason_code: reason_code.clone(),
                message: message.clone(),
            }),
        }
    }
    dto.state = final_confirm_state(plan_complete, &dto).to_string();
    Ok(dto)
}

/// state 判定：done=全已确认；ready=零确认且全就绪；partial=部分确认且剩余
/// 全就绪（blocking 空）；其余 not_ready（含 plan 未执行完成）。
fn final_confirm_state(plan_complete: bool, dto: &PlanCodingFinalConfirmDto) -> &'static str {
    if !dto.blocking.is_empty() {
        return "not_ready";
    }
    match (
        dto.confirmed_attempt_ids.is_empty(),
        dto.ready_attempt_ids.is_empty(),
        plan_complete,
    ) {
        (true, true, _) => "not_ready",
        (false, true, _) => "done",
        (true, false, true) => "ready",
        (false, false, true) => "partial",
        (_, false, false) => "not_ready",
    }
}

/// 只读 worktree clean 预检（无副作用，不落 manual gate）：与引擎
/// `ensure_issue_shared_worktree_clean` 同一物理路径——优先 attempt 自带
/// worktree，否则按 `target_snapshot` 路由仓维 shared worktree / legacy
/// issue shared worktree。读取失败按 unavailable fail-closed（宁可 not_ready
/// 也不在未知 git 状态下开始聚合确认）。
async fn attempt_worktree_clean(
    paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<(), (String, String)> {
    let Some(root) = attempt_worktree_root(paths, attempt) else {
        return Err((
            "worktree_missing".to_string(),
            format!("attempt {} has no worktree to check", attempt.id),
        ));
    };
    let git = GitWorkspaceService::new();
    match git.git_status(&root).await {
        Ok(status) if status.is_empty() => Ok(()),
        Ok(_) => Err((
            "worktree_dirty".to_string(),
            format!("worktree {} has uncommitted changes", root.display()),
        )),
        Err(error) => Err((
            "worktree_status_unavailable".to_string(),
            format!("git status failed for {}: {error}", root.display()),
        )),
    }
}

fn attempt_worktree_root(
    paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Option<PathBuf> {
    if let Some(worktree) = &attempt.worktree_path {
        return Some(worktree.clone());
    }
    let lifecycle = LifecycleStore::new(paths.clone());
    // shared worktree 记录读取错误折叠为「无可查 worktree」：预检 fail-closed
    // 走 worktree_missing，不静默放行。
    let shared = match attempt
        .target_snapshot
        .as_ref()
        .map(|snapshot| snapshot.logical_repository_id)
    {
        Some(repository_id) => lifecycle
            .get_repo_shared_worktree(&attempt.project_id, &attempt.issue_id, repository_id)
            .ok()
            .flatten(),
        None => lifecycle
            .get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)
            .ok()
            .flatten(),
    };
    shared.map(|shared| shared.worktree_path)
}

/// POST 失败项稳定码：优先引擎已知变体，未知兜底统一码（message 始终带
/// 引擎原文，失败仓原因可见）。
fn final_confirm_failure_code(error: &CodingWorkspaceEngineError) -> &'static str {
    match error {
        CodingWorkspaceEngineError::FinalConfirmNotReady(_) => "final_confirm_not_ready",
        CodingWorkspaceEngineError::SharedWorktreeDirtyManualGate(_) => {
            "shared_worktree_dirty_manual_gate"
        }
        CodingWorkspaceEngineError::CompletionCommitMissing(_) => "completion_commit_missing",
        CodingWorkspaceEngineError::WorkItemDiffScopeViolation(_) => {
            "work_item_diff_scope_violation"
        }
        CodingWorkspaceEngineError::CrossTargetDeliveryBlocked(_) => {
            "cross_target_delivery_blocked"
        }
        CodingWorkspaceEngineError::LegacySharedWorktreePresent(_) => {
            "legacy_shared_worktree_present"
        }
        _ => "coding_final_confirm_failed",
    }
}

fn result_item(
    binding: &AdvanceTargetAttemptBinding,
    status: &str,
    error_code: Option<String>,
    message: Option<String>,
) -> PlanCodingFinalConfirmResultItemDto {
    PlanCodingFinalConfirmResultItemDto {
        attempt_id: binding.attempt_id.clone(),
        target_repository_id: binding.target_repository_id.clone(),
        status: status.to_string(),
        error_code,
        message,
    }
}

/// 预检未过回执：blocking/plan 未完成/worktree 脏的 target 记 `failed`（带
/// 稳定码与原因），已 Completed 记 `already_completed`，其余未开始。
fn not_ready_result(
    plan_id: &str,
    facts: &[PlanFinalConfirmTargetFacts],
    failures: &[Option<(String, String)>],
) -> PlanCodingFinalConfirmResultDto {
    let results = facts
        .iter()
        .zip(failures.iter())
        .map(|(facts, failure)| match failure {
            Some((error_code, message)) => result_item(
                &facts.binding,
                "failed",
                Some(error_code.clone()),
                Some(message.clone()),
            ),
            None => match &facts.state {
                PlanFinalConfirmTargetState::Confirmed => {
                    result_item(&facts.binding, "already_completed", None, None)
                }
                PlanFinalConfirmTargetState::Ready
                | PlanFinalConfirmTargetState::Blocking { .. } => {
                    result_item(&facts.binding, "not_attempted", None, None)
                }
            },
        })
        .collect();
    PlanCodingFinalConfirmResultDto {
        plan_id: plan_id.to_string(),
        outcome: "not_ready".to_string(),
        results,
    }
}

/// 契约 A2：无 body POST——预检全部冻结 target（持久事实 + 只读 worktree
/// clean）全过才开始；按序跳过 Completed、逐个调用既有
/// `handle_final_confirm`，首失败即停、其余 not_attempted。
pub async fn post_plan_coding_final_confirm(
    State(state): State<WebAppState>,
    Path((project_id, issue_id, plan_id)): Path<(String, String, String)>,
) -> ApiResult<Json<PlanCodingFinalConfirmResultDto>> {
    for id in [&project_id, &issue_id, &plan_id] {
        validate_relative_id(id).map_err(|error| {
            ApiError::validation(
                "plan_coding_final_confirm_invalid_id",
                format!("invalid plan coding final confirm id: {error}"),
            )
        })?;
    }
    let app_paths = product_app_paths(&state);
    plan_live_admission(&app_paths, &project_id, &issue_id, &plan_id)
        .map_err(product_store_api_error)?;
    ensure_plan_confirmed(&app_paths, &project_id, &issue_id, &plan_id)?;

    let (plan_complete, facts) =
        plan_final_confirm_target_facts(&app_paths, &project_id, &issue_id, &plan_id)
            .map_err(product_store_api_error)?;

    // 预检一（持久事实，GET 同口径）：任一 target blocking → 不开始，逐仓
    // 说明未齐原因；plan 未执行完成时，事实就绪的 target 同样等待整体。
    let mut precheck_failures: Vec<Option<(String, String)>> = facts
        .iter()
        .map(|facts| match &facts.state {
            PlanFinalConfirmTargetState::Confirmed | PlanFinalConfirmTargetState::Ready => None,
            PlanFinalConfirmTargetState::Blocking {
                reason_code,
                message,
            } => Some((reason_code.clone(), message.clone())),
        })
        .collect();
    if !plan_complete {
        for (failure, facts) in precheck_failures.iter_mut().zip(facts.iter()) {
            if failure.is_none() && matches!(facts.state, PlanFinalConfirmTargetState::Ready) {
                *failure = Some((
                    "plan_not_complete".to_string(),
                    "plan coding execution is not complete".to_string(),
                ));
            }
        }
    }

    // 预检二（仅 POST，只读 git）：全部就绪 target 的 worktree clean——避免
    // 前仓确认成功、后仓因脏工作区半路失败。任一脏 → 整体 not_ready。
    if precheck_failures.iter().all(|failure| failure.is_none()) {
        for (index, facts) in facts.iter().enumerate() {
            if let (PlanFinalConfirmTargetState::Ready, Some(attempt)) =
                (&facts.state, facts.attempt.as_ref())
                && let Err((error_code, message)) =
                    attempt_worktree_clean(&app_paths, attempt).await
            {
                precheck_failures[index] = Some((error_code, message));
                break;
            }
        }
    }

    if precheck_failures.iter().any(|failure| failure.is_some()) {
        return Ok(Json(not_ready_result(&plan_id, &facts, &precheck_failures)));
    }

    // 执行：按序逐仓调用既有 handle_final_confirm（不改引擎；与 WS REST 同
    // 构造——dummy events，页面之后 GET 刷新真实状态，不依赖新事件协议）。
    let engine =
        coding_workspace_engine_with_dummy_events(CodingAttemptStore::new(app_paths.clone()));
    let mut results = Vec::with_capacity(facts.len());
    let mut stopped = false;
    for facts in &facts {
        let item = match (&facts.state, stopped) {
            (_, true) => result_item(&facts.binding, "not_attempted", None, None),
            (PlanFinalConfirmTargetState::Confirmed, _) => {
                result_item(&facts.binding, "already_completed", None, None)
            }
            (PlanFinalConfirmTargetState::Ready, _) => {
                // 与 WS/REST gate response 同款 mutation lease：逐仓持锁调用，
                // 防同 attempt 并发写。
                let attempt = facts.attempt.as_ref().expect("ready target has attempt");
                let attempt_key = CodingAttemptRunKey::from_attempt(attempt);
                let mutation_lease = state.coding_runs.lock_attempt_mutation(&attempt_key).await;
                let outcome = engine
                    .handle_final_confirm(&project_id, &issue_id, &facts.binding.attempt_id)
                    .await;
                drop(mutation_lease);
                match outcome {
                    Ok(_) => result_item(&facts.binding, "confirmed", None, None),
                    Err(error) => {
                        stopped = true;
                        result_item(
                            &facts.binding,
                            "failed",
                            Some(final_confirm_failure_code(&error).to_string()),
                            Some(error.to_string()),
                        )
                    }
                }
            }
            // 预检已保证 blocking 为空才进入执行；防御性视为未尝试并停止。
            (PlanFinalConfirmTargetState::Blocking { .. }, _) => {
                stopped = true;
                result_item(&facts.binding, "not_attempted", None, None)
            }
        };
        results.push(item);
    }

    let outcome = if results
        .iter()
        .all(|item| item.status == "confirmed" || item.status == "already_completed")
    {
        "all_confirmed"
    } else {
        "partially_confirmed"
    };
    Ok(Json(PlanCodingFinalConfirmResultDto {
        plan_id: plan_id.to_string(),
        outcome: outcome.to_string(),
        results,
    }))
}
