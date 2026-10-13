//! N3（REQ-CS-01/02/06/08）：多仓 confirmed plan 的一键显式 Coding 启动
//! POST 与 plan 聚合进度 GET。
//!
//! POST 顺序固定：confirmed/revision/delete 准入 → `prepare`（N2 run 冻结）
//! → 现有 Manual `advance_plan` 到 durable Ready → Ready 集互证 + 全图
//! ready 稳定选一 → `bind_ready`/`mark_start_requested` → 现 Manual
//! `start_coding_once`（用冻结 `selection.start_command_id`）→ `record_start_
//! outcome`。每个中断窗口重放读同一 selection；已有 run
//! Started/AlreadyStarted/NeedsHuman 按事实返回，不再 start。
//!
//! GET/POST 共用 `build_plan_coding_progress` 聚合读函数：按冻结 target/桶
//! units 覆盖核对，初始化 unit Running 与真正 provider 执行分开
//! （`execution_state`），全局 unsatisfied 依赖/waiting_slot/pause 可观察；
//! 未创建 run 时只读 confirmed authoritative plan 与已有 Ready 集，仍返回
//! bound_plan_revision_id，不持久化、不启动。

use std::collections::{BTreeMap, BTreeSet};

use super::super::support::*;
use super::super::*;
use crate::product::advance_store::{
    AdvanceInput, AdvanceOutcome, AdvanceStatus, AdvanceTargetAttemptBinding,
};
use crate::product::coding_attempt_store::AuthoritativeCodingUnitBinding;
use crate::product::coding_models::{
    CodingAttemptStatus, CodingExecutionUnit, CodingExecutionUnitStatus, CodingStartOrigin,
    CodingStartPhase,
};
use crate::product::coding_plan_run::{
    CodingPlanRun, CodingPlanRunPhase, CodingPlanRunStore, CodingPlanSelection,
    CodingPlanStartOutcome, PLAN_CODING_CONTINUATION_NOT_ENABLED,
};
use crate::product::entity_deletion_store::{DeletionEntityKind, EntityScope};
use crate::product::entity_operation_store::EntityOperationStore;
use crate::product::json_store::ProductStoreError;
use crate::product::logical_codebase::LogicalRepositoryId;
use crate::product::models::automation::OperationState;
use crate::web::advance_plan::{AdvancePlanOrigin, advance_plan};
use crate::web::coding_start::{StartCodingCommand, StartCodingOutcome, start_coding_once};
use crate::web::types::{
    PlanCodingProgressDto, PlanCodingStartRequest, PlanCodingStartResultDto, PlanCodingTargetDto,
    PlanCodingUnitProgressDto,
};

/// POST 推进结果分类：区分“本次新冻结的首启回执 / 无 ready 等待 / 已有
/// 事实按原样返回”，用于 `operation_state` 投影。
enum PostFlow {
    Recorded(CodingPlanStartOutcome),
    WaitingBound,
    Existing(CodingPlanRunPhase),
}

/// REQ-CS-01：多仓 plan 一键显式首启。稳定 `command_id` +
/// `expected_plan_revision_id`（与 GET 聚合返回的 bound revision 互证）。
pub async fn post_plan_coding_start(
    State(state): State<WebAppState>,
    Path((project_id, issue_id, plan_id)): Path<(String, String, String)>,
    Json(request): Json<PlanCodingStartRequest>,
) -> ApiResult<Json<PlanCodingStartResultDto>> {
    for id in [
        project_id.as_str(),
        issue_id.as_str(),
        plan_id.as_str(),
        request.command_id.as_str(),
        request.expected_plan_revision_id.as_str(),
    ] {
        validate_relative_id(id).map_err(|error| {
            ApiError::validation(
                "plan_coding_start_invalid_id",
                format!("invalid plan coding start id: {error}"),
            )
        })?;
    }
    let app_paths = product_app_paths(&state);
    // 删除准入：plan scope 删除进行中即 fail-closed（沿现 entity_deleting 映射）。
    plan_live_admission(&app_paths, &project_id, &issue_id, &plan_id)
        .map_err(product_store_api_error)?;
    ensure_plan_confirmed(&app_paths, &project_id, &issue_id, &plan_id)?;

    // 与旧 group 创建面同一进程内串行键，避免并写 group attempt 初始化。
    let group_lock_key = format!("work_item_group:{project_id}:{issue_id}:{plan_id}");
    let _group_guard = state.coding_runs.lock_named(&group_lock_key).await;

    let run_store = CodingPlanRunStore::new(app_paths.clone());
    let run = run_store
        .prepare(
            &project_id,
            &issue_id,
            &plan_id,
            &request.command_id,
            &request.expected_plan_revision_id,
        )
        .map_err(product_store_api_error)?;
    let (final_run, flow) = drive_plan_coding_start(&state, &run_store, &app_paths, run).await?;

    let operation_state = match &flow {
        PostFlow::Recorded(
            CodingPlanStartOutcome::Started { .. } | CodingPlanStartOutcome::AlreadyStarted { .. },
        ) => OperationState::Accepted,
        PostFlow::Recorded(CodingPlanStartOutcome::NeedsHuman { .. }) => OperationState::NeedsHuman,
        PostFlow::WaitingBound => OperationState::Accepted,
        PostFlow::Existing(CodingPlanRunPhase::NeedsHuman) => OperationState::NeedsHuman,
        PostFlow::Existing(_) => OperationState::Replayed,
    };

    let progress = build_plan_coding_progress(
        &app_paths,
        &project_id,
        &issue_id,
        &plan_id,
        Some(&final_run),
    )
    .map_err(product_store_api_error)?;
    let waiting_units = progress
        .units
        .iter()
        .filter(|unit| {
            matches!(
                unit.execution_state.as_str(),
                "waiting_dependency" | "waiting_slot" | "paused"
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    Ok(Json(PlanCodingStartResultDto {
        plan_id: plan_id.clone(),
        bound_plan_revision_id: final_run.bound_plan_revision_id.clone(),
        command_id: final_run.command_id.clone(),
        operation_state,
        primary_attempt_id: progress.primary_attempt_id.clone(),
        target_attempts: progress.target_attempts.clone(),
        selected_unit_id: final_run
            .selection
            .as_ref()
            .map(|selection| selection.unit_id.clone()),
        selected_attempt_id: final_run
            .selection
            .as_ref()
            .map(|selection| selection.attempt_id.clone()),
        start_outcome: final_run.start_outcome.clone(),
        waiting_units,
        progress,
    }))
}

/// REQ-CS-06：plan 聚合进度 GET。未创建 run 时只读 confirmed authoritative
/// plan + 已有 Ready 集（不持久化/不启动），仍返回 bound revision。
pub async fn get_plan_coding_progress(
    State(state): State<WebAppState>,
    Path((project_id, issue_id, plan_id)): Path<(String, String, String)>,
) -> ApiResult<Json<PlanCodingProgressDto>> {
    for id in [&project_id, &issue_id, &plan_id] {
        validate_relative_id(id).map_err(|error| {
            ApiError::validation(
                "plan_coding_start_invalid_id",
                format!("invalid plan coding start id: {error}"),
            )
        })?;
    }
    let app_paths = product_app_paths(&state);
    plan_live_admission(&app_paths, &project_id, &issue_id, &plan_id)
        .map_err(product_store_api_error)?;
    ensure_plan_confirmed(&app_paths, &project_id, &issue_id, &plan_id)?;

    let run = CodingPlanRunStore::new(app_paths.clone())
        .get(&project_id, &issue_id, &plan_id)
        .map_err(product_store_api_error)?;
    let progress =
        build_plan_coding_progress(&app_paths, &project_id, &issue_id, &plan_id, run.as_ref())
            .map_err(product_store_api_error)?;
    Ok(Json(progress))
}

/// 已有终态回执/首启事实按事实返回（不再 start）；在途相位沿同一冻结
/// selection 重放推进。
async fn drive_plan_coding_start(
    state: &WebAppState,
    run_store: &CodingPlanRunStore,
    app_paths: &ProductAppPaths,
    run: CodingPlanRun,
) -> ApiResult<(CodingPlanRun, PostFlow)> {
    match run.phase {
        CodingPlanRunPhase::Started
        | CodingPlanRunPhase::AlreadyStarted
        | CodingPlanRunPhase::NeedsHuman => {
            let phase = run.phase;
            Ok((run, PostFlow::Existing(phase)))
        }
        // Prepared/ReadyBound/StartRequested：中断窗口重放。
        CodingPlanRunPhase::Prepared
        | CodingPlanRunPhase::ReadyBound
        | CodingPlanRunPhase::StartRequested => {
            // ① Manual advance 到 durable Ready（advance 按计划幂等重放）。
            // AdvanceInput.command_id 必须用 prepare 返回的 run.command_id
            //（第二 command 命中时不是新的 request.command_id）。
            let outcome = advance_plan(
                state,
                AdvanceInput {
                    command_id: run.command_id.clone(),
                    project_id: run.project_id.clone(),
                    issue_id: run.issue_id.clone(),
                    plan_id: run.plan_id.clone(),
                },
                AdvancePlanOrigin::Manual,
            )
            .await
            .map_err(|error| {
                ApiError::runtime(
                    "plan_coding_advance_failed",
                    format!("manual advance for plan coding start failed: {error}"),
                    serde_json::json!({}),
                )
            })?;
            let bindings = sorted_target_bindings(advance_target_bindings(outcome)?);

            // ② 全图 ready 稳定选一：已有冻结 selection 直接复用（不重选）。
            let selection = match run.selection.clone() {
                Some(frozen) => Some(frozen),
                None => select_first_ready_unit(
                    app_paths,
                    &run.project_id,
                    &run.issue_id,
                    &run,
                    &bindings,
                )
                .map_err(product_store_api_error)?,
            };

            // ③ Ready 集互证 + selection 冻结（N2 锁内重读核身份/集合漂移）。
            let bound = run_store
                .bind_ready(&run, &bindings, selection)
                .map_err(product_store_api_error)?;
            let Some(selection) = bound.selection.clone() else {
                // 无 ready：durable 绑定全集保持 ready_bound 等待，零首启。
                return Ok((bound, PostFlow::WaitingBound));
            };

            // ④ 首启调用前持久化 start request（不能发空首启）。
            let requested = run_store
                .mark_start_requested(&bound)
                .map_err(product_store_api_error)?;

            // ⑤ 现 Manual start_coding_once：首启用已落盘
            //    selection.start_command_id，durable claim 单发幂等；副作用
            //    不明由其内部保持 NeedsHuman（不用重试隐式清除）。
            let outcome = start_coding_once(
                state,
                &run.project_id,
                &run.issue_id,
                StartCodingCommand {
                    attempt_id: selection.attempt_id.clone(),
                    command_id: selection.start_command_id.clone(),
                    origin: CodingStartOrigin::Manual,
                },
            )
            .await
            .map_err(|error| {
                ApiError::runtime(
                    "plan_coding_first_start_failed",
                    format!("plan coding first start failed: {error}"),
                    serde_json::json!({ "code": error.code() }),
                )
            })?;
            let outcome = match outcome {
                StartCodingOutcome::Started { attempt_id } => {
                    CodingPlanStartOutcome::Started { attempt_id }
                }
                StartCodingOutcome::AlreadyStarted { attempt_id } => {
                    CodingPlanStartOutcome::AlreadyStarted { attempt_id }
                }
                StartCodingOutcome::NeedsHuman { attempt_id, reason } => {
                    CodingPlanStartOutcome::NeedsHuman { attempt_id, reason }
                }
            };

            // ⑥ record 真实 outcome（回执冻结后不再漂移；同值重放幂等）。
            let recorded = run_store
                .record_start_outcome(&requested, outcome.clone())
                .map_err(product_store_api_error)?;
            Ok((recorded, PostFlow::Recorded(outcome)))
        }
        // prepare 已拒绝 Paused/Invalidated（coding_plan_run_not_active）。
        CodingPlanRunPhase::Paused | CodingPlanRunPhase::Invalidated => {
            Err(product_store_api_error(ProductStoreError::Conflict {
                kind: "coding_plan_run_not_active",
                id: run.command_id.clone(),
            }))
        }
    }
}

/// plan scope 删除准入：try-shared 争用/墓碑一律立即
/// `Conflict{entity_deleting}`，零阻塞等待（沿 EntityOperationStore 现语义）。
fn plan_live_admission(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
) -> Result<(), ProductStoreError> {
    let scope = EntityScope::new(project_id, issue_id, DeletionEntityKind::Plan, plan_id);
    scope.validate()?;
    EntityOperationStore::new(paths.clone()).with_live_write(&scope, || Ok(()))
}

fn ensure_plan_confirmed(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
) -> Result<(), ApiError> {
    let plan = LifecycleStore::new(paths.clone())
        .get_issue_work_item_plan(project_id, issue_id, plan_id)
        .map_err(product_store_api_error)?;
    if plan.status != IssueWorkItemPlanStatus::Confirmed {
        return Err(ApiError::validation(
            "work_item_plan_not_confirmed",
            "work item plan must be confirmed before plan coding start",
        ));
    }
    Ok(())
}

/// advance 回执中提取 per-target Ready 绑定集：Rejected/未 Ready/空集合
/// （单仓或未归属计划不走本入口）都 fail-closed，不建 legacy fallback。
fn advance_target_bindings(
    outcome: AdvanceOutcome,
) -> Result<Vec<AdvanceTargetAttemptBinding>, ApiError> {
    match outcome {
        AdvanceOutcome::Completed {
            target_attempts, ..
        } => {
            if target_attempts.is_empty() {
                Err(ApiError::validation(
                    "plan_coding_start_target_set_missing",
                    "plan coding start requires a multi-target attempt binding set",
                ))
            } else {
                Ok(target_attempts)
            }
        }
        AdvanceOutcome::Replayed { record } => {
            if record.status != AdvanceStatus::Ready {
                Err(ApiError::runtime(
                    "plan_coding_advance_not_ready",
                    format!(
                        "advance record {} is in durable status {:?}",
                        record.id, record.status
                    ),
                    serde_json::json!({ "advance_id": record.id }),
                ))
            } else if record.target_attempts.is_empty() {
                Err(ApiError::validation(
                    "plan_coding_start_target_set_missing",
                    "plan coding start requires a multi-target attempt binding set",
                ))
            } else {
                Ok(record.target_attempts)
            }
        }
        AdvanceOutcome::Rejected { code, reason, .. } => Err(ApiError::runtime(
            "plan_coding_advance_rejected",
            format!("manual advance was rejected: {reason}"),
            serde_json::json!({ "code": code }),
        )),
    }
}

fn sorted_target_bindings(
    mut bindings: Vec<AdvanceTargetAttemptBinding>,
) -> Vec<AdvanceTargetAttemptBinding> {
    bindings.sort_by(|left, right| {
        left.target_repository_id
            .cmp(&right.target_repository_id)
            .then_with(|| left.attempt_id.cmp(&right.attempt_id))
    });
    bindings
}

/// 全图拓扑位置（Kahn，字典序 tie-break）——稳定选择的排序基础。
/// 图合法性已由 N1 冻结校验；此处仅排序，未入 rank 的环成员不参与候选。
fn global_topological_ranks(units: &[AuthoritativeCodingUnitBinding]) -> BTreeMap<String, usize> {
    let mut indegree: BTreeMap<&str, usize> = units
        .iter()
        .map(|unit| (unit.logical_work_item_id.as_str(), 0usize))
        .collect();
    let mut dependents: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for unit in units {
        for dependency in &unit.dependency_logical_work_item_ids {
            let dependency_known = indegree.contains_key(dependency.as_str());
            if dependency_known
                && let Some(entry) = indegree.get_mut(unit.logical_work_item_id.as_str())
            {
                *entry += 1;
                dependents
                    .entry(dependency.as_str())
                    .or_default()
                    .insert(unit.logical_work_item_id.as_str());
            }
        }
    }
    let mut ready: BTreeSet<&str> = indegree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(id, _)| *id)
        .collect();
    let mut ranks = BTreeMap::new();
    while let Some(id) = ready.pop_first() {
        ranks.insert(id.to_string(), ranks.len());
        if let Some(downstream) = dependents.get(id) {
            for next in downstream.clone() {
                if let Some(degree) = indegree.get_mut(next) {
                    *degree -= 1;
                    if *degree == 0 {
                        ready.insert(next);
                    }
                }
            }
        }
    }
    ranks
}

/// 全图 ready 稳定选一（REQ-CS-01/02）：候选限本冻结集合内 Created attempt
/// 的非终态 unit，直接依赖全部有完成证据；排序键（全图拓扑位置 →
/// logical WI ID → target UUID）不硬编码首项。无 ready 返回 None（run/结果
/// 等待且零首启）。确定性首启 command 由
/// `CodingPlanSelection::deterministic_start_command_id` 唯一派生。
fn select_first_ready_unit(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    run: &CodingPlanRun,
    bindings: &[AdvanceTargetAttemptBinding],
) -> Result<Option<CodingPlanSelection>, ProductStoreError> {
    let store = CodingAttemptStore::new(paths.clone());
    let authoritative = store.resolve_authoritative_group_plan_binding_for_revision(
        project_id,
        issue_id,
        &run.plan_id,
        &run.bound_plan_revision_id,
    )?;
    let ranks = global_topological_ranks(&authoritative.units);

    // 完成事实集（跨桶全局）：unit Completed 即完成证据。
    let mut completed: BTreeSet<String> = BTreeSet::new();
    for binding in bindings {
        for unit in store.list_coding_units(project_id, issue_id, &binding.attempt_id)? {
            if unit.status == CodingExecutionUnitStatus::Completed {
                completed.insert(unit.logical_work_item_id.clone());
            }
        }
    }

    let mut best: Option<(usize, String, String)> = None;
    let mut best_selection: Option<CodingPlanSelection> = None;
    for binding in bindings {
        let attempt = store.get_attempt(project_id, issue_id, &binding.attempt_id)?;
        // 只在未首启集合中选择（与 bind_ready 的不收养边界同源）。
        if attempt.status != CodingAttemptStatus::Created {
            continue;
        }
        for unit in store.list_coding_units(project_id, issue_id, &binding.attempt_id)? {
            if matches!(
                unit.status,
                CodingExecutionUnitStatus::Completed
                    | CodingExecutionUnitStatus::Failed
                    | CodingExecutionUnitStatus::Superseded
                    | CodingExecutionUnitStatus::Skipped
            ) {
                continue;
            }
            let Some(auth_unit) = authoritative
                .units
                .iter()
                .find(|candidate| candidate.logical_work_item_id == unit.logical_work_item_id)
            else {
                continue;
            };
            let unsatisfied = auth_unit
                .dependency_logical_work_item_ids
                .iter()
                .any(|dependency| !completed.contains(dependency));
            if unsatisfied {
                continue;
            }
            let rank = ranks
                .get(&unit.logical_work_item_id)
                .copied()
                .unwrap_or(usize::MAX);
            let key = (
                rank,
                unit.logical_work_item_id.clone(),
                binding.target_repository_id.clone(),
            );
            if best.as_ref().is_none_or(|current| &key < current) {
                best = Some(key);
                best_selection = Some(CodingPlanSelection {
                    unit_id: unit.id.clone(),
                    logical_work_item_id: unit.logical_work_item_id.clone(),
                    attempt_id: binding.attempt_id.clone(),
                    target_repository_id: binding.target_repository_id.clone(),
                    start_command_id: CodingPlanSelection::deterministic_start_command_id(
                        project_id,
                        issue_id,
                        &run.plan_id,
                        &run.command_id,
                        &binding.target_repository_id,
                    ),
                });
            }
        }
    }
    Ok(best_selection)
}

fn coding_plan_phase_text(phase: CodingPlanRunPhase) -> &'static str {
    match phase {
        CodingPlanRunPhase::Prepared => "prepared",
        CodingPlanRunPhase::ReadyBound => "ready_bound",
        CodingPlanRunPhase::StartRequested => "start_requested",
        CodingPlanRunPhase::Started => "started",
        CodingPlanRunPhase::AlreadyStarted => "already_started",
        CodingPlanRunPhase::NeedsHuman => "needs_human",
        CodingPlanRunPhase::Paused => "paused",
        CodingPlanRunPhase::Invalidated => "invalidated",
    }
}

/// repo worktree lock 事实投影：无记录/无 owner → `absent`；owner 为本绑定
/// attempt → `held`；owner 为他人（含未绑定 attempt 的 lease）→ `contested`。
/// 不造 provider_running 猜测（REQ-CS-04：资源占用按真实 lease 展示）。
fn repo_lease_state(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    target_repository_id: &str,
    attempt_id: &str,
) -> Result<String, ProductStoreError> {
    let Ok(repository_id) = uuid::Uuid::parse_str(target_repository_id) else {
        return Err(ProductStoreError::InvalidRecord {
            kind: "coding_plan_run_target",
            reason: format!("target repository id is not a logical UUID: {target_repository_id}"),
        });
    };
    match lifecycle.get_repo_shared_worktree(
        project_id,
        issue_id,
        LogicalRepositoryId(repository_id),
    )? {
        None => Ok("absent".to_string()),
        Some(record) => match record.current_lock_owner_id.as_deref() {
            None => Ok("absent".to_string()),
            Some(owner) if owner == attempt_id => Ok("held".to_string()),
            Some(_) => Ok("contested".to_string()),
        },
    }
}

struct PlanCodingTargetFacts {
    binding: AdvanceTargetAttemptBinding,
    /// `None` = 冻结成员 attempt 已被独立删除（REQ-CS-07 失效窗口）。
    attempt: Option<crate::product::coding_models::CodingExecutionAttempt>,
    units: Vec<CodingExecutionUnit>,
}

/// GET/POST 共用聚合读函数：按冻结 target（无 run 时按已有 Ready 集）/
/// 桶 units 覆盖核对，产出 REQ-CS-06 的 plan 进度投影。
pub(crate) fn build_plan_coding_progress(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
    run: Option<&CodingPlanRun>,
) -> Result<PlanCodingProgressDto, ProductStoreError> {
    let store = CodingAttemptStore::new(paths.clone());
    let authoritative = match run {
        Some(run) => store.resolve_authoritative_group_plan_binding_for_revision(
            project_id,
            issue_id,
            plan_id,
            &run.bound_plan_revision_id,
        )?,
        None => store.resolve_authoritative_group_plan_binding(project_id, issue_id, plan_id)?,
    };
    let bound_plan_revision_id = run
        .map(|run| run.bound_plan_revision_id.clone())
        .unwrap_or_else(|| authoritative.plan_revision_id.clone());

    // 绑定集：run 冻结集优先；无 run 时读该 plan 的已有 attempt 集（Ready 集）。
    let bindings = match run.filter(|run| !run.target_attempts.is_empty()) {
        Some(run) => run.target_attempts.clone(),
        None => {
            let targets: BTreeSet<LogicalRepositoryId> = authoritative
                .units
                .iter()
                .filter_map(|unit| unit.target_repository_id)
                .collect();
            let mut derived = Vec::new();
            for target in targets.into_iter() {
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
    let bindings = sorted_target_bindings(bindings);

    let mut facts: Vec<PlanCodingTargetFacts> = Vec::new();
    for binding in &bindings {
        // REQ-CS-07：冻结成员被独立删除（NotFound）时按 run 相位投影该
        // target 为失效态，不让单成员删除炸整条 GET；身份错配/IO/JSON
        // 错误仍 fail-closed 上抛。
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
        facts.push(PlanCodingTargetFacts {
            binding: binding.clone(),
            attempt,
            units,
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
    let ranks = global_topological_ranks(&authoritative.units);
    let selection = run.and_then(|run| run.selection.clone());
    let paused = run.is_some_and(|run| run.phase == CodingPlanRunPhase::Paused);
    let invalidated = run.is_some_and(|run| run.phase == CodingPlanRunPhase::Invalidated);

    let lifecycle = LifecycleStore::new(paths.clone());
    let mut target_attempts = Vec::new();
    for facts in &facts {
        let lease_state = repo_lease_state(
            &lifecycle,
            project_id,
            issue_id,
            &facts.binding.target_repository_id,
            &facts.binding.attempt_id,
        )?;
        let (attempt_status, worktree_path) = match &facts.attempt {
            Some(attempt) => (attempt.status.clone(), attempt.worktree_path.clone()),
            // 成员已删除：授权失效的终态投影（repo lock/worktree 归 N7/D2.2
            // 删除闭包按真实事实清理，此处不猜测资源）。
            None => (CodingAttemptStatus::Aborted, None),
        };
        target_attempts.push(PlanCodingTargetDto {
            target_repository_id: facts.binding.target_repository_id.clone(),
            attempt_id: facts.binding.attempt_id.clone(),
            attempt_status,
            worktree_path,
            lease_state,
        });
    }

    let mut units = Vec::new();
    for auth_unit in &authoritative.units {
        let Some(target) = auth_unit.target_repository_id else {
            continue;
        };
        let Some(facts) = facts
            .iter()
            .find(|facts| facts.binding.target_repository_id == target.0.to_string())
        else {
            continue;
        };
        let Some(unit) = facts
            .units
            .iter()
            .find(|unit| unit.logical_work_item_id == auth_unit.logical_work_item_id)
        else {
            continue;
        };
        let mut unsatisfied_dependency_ids: Vec<String> = auth_unit
            .dependency_logical_work_item_ids
            .iter()
            .filter(|dependency| !completed.contains(*dependency))
            .cloned()
            .collect();
        unsatisfied_dependency_ids.sort();
        // provider 事实（durable claim 相位 ≥ ProviderMayHaveStarted + active
        // 指针 + unit Running）与初始化桶首 unit Running（物化事实）严格分开
        //（REQ-CS-04）：Claimed/RunnerRegistered 回落 initialized。
        let provider_started = facts
            .attempt
            .as_ref()
            .and_then(|attempt| attempt.start_claim.as_ref())
            .is_some_and(|claim| {
                matches!(
                    claim.phase,
                    CodingStartPhase::ProviderMayHaveStarted | CodingStartPhase::NeedsHuman
                )
            })
            && facts
                .attempt
                .as_ref()
                .is_some_and(|attempt| attempt.active_unit_id.as_deref() == Some(unit.id.as_str()))
            && unit.status == CodingExecutionUnitStatus::Running;
        let (execution_state, waiting_reason): (&str, Option<&str>) =
            if unit.status == CodingExecutionUnitStatus::Completed {
                // 完成事实不因授权失效回滚（REQ-CS-05/07）。
                ("completed", None)
            } else if invalidated {
                // run Invalidated：整体失效文案（成员删除授权失效）。
                ("invalidated", None)
            } else if !unsatisfied_dependency_ids.is_empty() {
                ("waiting_dependency", Some("dependency_not_ready"))
            } else if provider_started {
                ("running", None)
            } else if paused {
                // 首项停点：下一项编排尚未启用（REQ-CS-05/06）。
                ("paused", Some(PLAN_CODING_CONTINUATION_NOT_ENABLED))
            } else if unit.status == CodingExecutionUnitStatus::Running {
                ("initialized", None)
            } else if selection.is_some() {
                ("waiting_slot", Some("execution_slot_occupied"))
            } else {
                ("waiting_slot", None)
            };
        units.push(PlanCodingUnitProgressDto {
            unit_id: unit.id.clone(),
            logical_work_item_id: unit.logical_work_item_id.clone(),
            target_repository_id: facts.binding.target_repository_id.clone(),
            attempt_id: facts.binding.attempt_id.clone(),
            unit_status: unit.status.clone(),
            execution_state: execution_state.to_string(),
            unsatisfied_dependency_ids,
            waiting_reason: waiting_reason.map(str::to_string),
        });
    }
    units.sort_by(|left, right| {
        let left_rank = ranks
            .get(&left.logical_work_item_id)
            .copied()
            .unwrap_or(usize::MAX);
        let right_rank = ranks
            .get(&right.logical_work_item_id)
            .copied()
            .unwrap_or(usize::MAX);
        left_rank
            .cmp(&right_rank)
            .then_with(|| left.logical_work_item_id.cmp(&right.logical_work_item_id))
            .then_with(|| left.target_repository_id.cmp(&right.target_repository_id))
    });

    let plan_complete = !authoritative.units.is_empty()
        && authoritative
            .units
            .iter()
            .all(|unit| completed.contains(&unit.logical_work_item_id));
    // primary 仅表示打开入口（REQ-CS-06）：优先冻结 selection，否则确定性
    // 首个绑定；绝不代表整 plan 执行或完成。
    let primary_attempt_id = selection
        .as_ref()
        .map(|selection| selection.attempt_id.clone())
        .or_else(|| bindings.first().map(|binding| binding.attempt_id.clone()));

    Ok(PlanCodingProgressDto {
        plan_id: plan_id.to_string(),
        bound_plan_revision_id,
        command_id: run.map(|run| run.command_id.clone()),
        mode: run.map(|run| run.mode.clone()),
        phase: run.map(|run| coding_plan_phase_text(run.phase).to_string()),
        primary_attempt_id,
        selected_unit_id: selection
            .as_ref()
            .map(|selection| selection.unit_id.clone()),
        selected_attempt_id: selection
            .as_ref()
            .map(|selection| selection.attempt_id.clone()),
        target_attempts,
        units,
        plan_complete,
    })
}
