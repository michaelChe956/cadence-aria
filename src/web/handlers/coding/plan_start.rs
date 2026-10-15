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

use super::super::dto::{coding_execution_stage_text, push_status_text};
use super::super::support::*;
use super::super::*;
use crate::product::advance_store::{
    AdvanceInitializationPhase, AdvanceInput, AdvanceOutcome, AdvanceStatus, AdvanceStore,
    AdvanceTargetAttemptBinding,
};
use crate::product::coding_attempt_store::{
    AuthoritativeCodingUnitBinding, CodingGroupInitializationPhase,
};
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
use crate::product::issue_store::IssueStore;
use crate::product::json_store::ProductStoreError;
use crate::product::logical_codebase::LogicalRepositoryId;
use crate::product::models::automation::OperationState;
use crate::product::project_store::ProjectStore;
use crate::product::repository_store::RepositoryStore;
use crate::web::advance_plan::{AdvancePlanOrigin, advance_plan};
use crate::web::coding_start::{StartCodingCommand, StartCodingOutcome, start_coding_once};
use crate::web::state::CodingAttemptRunKey;
use crate::web::types::{
    PlanCodingProgressDto, PlanCodingStartRequest, PlanCodingStartResultDto, PlanCodingTargetDto,
    PlanCodingUnitProgressDto, PlanCodingWorkspaceDeletionQuery,
    PlanCodingWorkspaceDeletionResultDto, PlanWorkspaceLifecycleDto,
};

/// POST 推进结果分类：区分“本次新冻结的首启回执 / 无 ready 等待 / 已有
/// 事实按原样返回 / 续跑推进了新就绪 unit”，用于 `operation_state` 投影。
enum PostFlow {
    Recorded(CodingPlanStartOutcome),
    WaitingBound,
    Existing(CodingPlanRunPhase),
    /// S2a 续跑：本请求推动了至少一个新就绪 unit（start 或跨桶桥 resume）。
    Continued,
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

    // H4（设计 A 状态表/C 节）：开始动作先收敛半残执行轮——clearing 在案以
    // `clearing.command_id` 身份续清（H2a 语义：同 command 续清/异 command 拒，
    // 开始入口是钩子中断态的可达续清出口）；零成员且旧执行引用已证断裂
    // （旧 Ready/journal 指向已亡全集）则走 H2a 零成员闭包完整重置后正常
    // 新建；正常初始化前缀与有成员现场零动作（N2/prepare 权威语义不变）。
    converge_stale_plan_coding_round(
        &app_paths,
        &project_id,
        &issue_id,
        &plan_id,
        &request.command_id,
    )?;
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

    // S2a 续跑全自动化（owner 2026-10-14）：回执相位上的再点击 → 全局
    // 就绪集重估推进（就绪=直接依赖全部 Completed 且未启动；就绪集>1
    // 并行 start；失败链停、独立链继续）。仅当已存在终态 unit（完成/失败
    // 等）才推进——纯重放/首启窗口保持零新启（REQ-CS-01 单选首启冻结，
    // n3 族回归）。本请求内完成的首启不重估。
    let (final_run, flow) = match flow {
        PostFlow::Existing(phase) => {
            if plan_has_terminal_coding_unit(&app_paths, &final_run)
                .map_err(product_store_api_error)?
            {
                let drive =
                    drive_ready_units_for_plan(&state, &project_id, &issue_id, &plan_id, None)
                        .await?;
                if drive.started_attempt_ids.is_empty() && drive.resumed_attempt_ids.is_empty() {
                    (final_run, PostFlow::Existing(phase))
                } else {
                    let reloaded = run_store
                        .get(&project_id, &issue_id, &plan_id)
                        .map_err(product_store_api_error)?;
                    (reloaded.unwrap_or(final_run), PostFlow::Continued)
                }
            } else {
                (final_run, PostFlow::Existing(phase))
            }
        }
        other => (final_run, other),
    };

    let operation_state = match &flow {
        PostFlow::Recorded(
            CodingPlanStartOutcome::Started { .. } | CodingPlanStartOutcome::AlreadyStarted { .. },
        ) => OperationState::Accepted,
        PostFlow::Recorded(CodingPlanStartOutcome::NeedsHuman { .. }) => OperationState::NeedsHuman,
        PostFlow::WaitingBound => OperationState::Accepted,
        PostFlow::Existing(CodingPlanRunPhase::NeedsHuman) => OperationState::NeedsHuman,
        PostFlow::Existing(_) => OperationState::Replayed,
        PostFlow::Continued => OperationState::Accepted,
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

/// H4（设计 2026-10-14 C/D 节）：plan-scoped 一键整组删除——服务端一个受
/// 保护操作（非浏览器四次单删），范围权威是服务端按 `(project,issue,plan)`
/// 枚举的真实 group attempts，不收前端 attempt ID 列表。
///
/// 顺序：删除准入（Plan 真删除互斥，闭包 own 本清理）→ 组串行键 →
/// clearing 在案以 `clearing.command_id` 身份续清（删除动作是中断态的产品
/// 续清出口；pending 冻结清单即本次回执 deleted 集）→ 同 command 回执且其
/// 轮次仍为当前轮次的幂等重放（Replayed，不扫下一代刚创建的 attempts；
/// 更旧回执不判重放，按当前状态走分类门禁）→ 服务端分类确认
/// （无对象无残留零弹窗；浅/强恰一次 409 `deletion_confirmation_required`，
/// 文案权威）→ `expected_generation` 互证（≠当前 409，零写，晚到删除不删
/// 新轮）→ 逐成员复用 N7 清理（abort runner→mutation lease→Git 终止协调→
/// worktree 锁释放→workspace/handoff 清理→delete_attempt→N7 run 失效→
/// 锁残留）→ H2a 零成员闭包收口（本 command 记纯 ID 回执：generation 单调
/// +1、退役围栏、run/advance/journal/专属资源清，保留 Plan/WI/revisions）。
pub async fn delete_plan_coding_workspaces(
    State(state): State<WebAppState>,
    Path((project_id, issue_id, plan_id)): Path<(String, String, String)>,
    Query(query): Query<PlanCodingWorkspaceDeletionQuery>,
) -> ApiResult<Json<PlanCodingWorkspaceDeletionResultDto>> {
    for id in [
        project_id.as_str(),
        issue_id.as_str(),
        plan_id.as_str(),
        query.command_id.as_str(),
    ] {
        validate_relative_id(id).map_err(|error| {
            ApiError::validation(
                "plan_coding_workspace_delete_invalid_id",
                format!("invalid plan coding workspace delete id: {error}"),
            )
        })?;
    }
    let app_paths = product_app_paths(&state);
    plan_live_admission(&app_paths, &project_id, &issue_id, &plan_id)
        .map_err(product_store_api_error)?;

    // 与 POST/组创建同一进程内串行键，避免并写 group attempt 初始化。
    let group_lock_key = format!("work_item_group:{project_id}:{issue_id}:{plan_id}");
    let _group_guard = state.coding_runs.lock_named(&group_lock_key).await;

    let run_store = CodingPlanRunStore::new(app_paths.clone());
    let lifecycle = run_store
        .get_workspace_lifecycle(&project_id, &issue_id, &plan_id)
        .map_err(product_store_api_error)?;
    let current_generation = lifecycle.as_ref().map_or(1, |metadata| metadata.generation);
    let mut operation_state = "accepted";
    let mut deleted_attempt_ids: Vec<String> = Vec::new();
    if let Some(clearing) = lifecycle
        .as_ref()
        .and_then(|metadata| metadata.clearing.as_ref())
    {
        // 清理中断重入：同一删除动作续清（H2a：同 command 续清/异 command 拒
        // ——此处按记录身份 `clearing.command_id` 续清，幂等 missing=OK）。
        deleted_attempt_ids = clearing.pending_attempt_ids.clone();
        close_plan_coding_workspace_round(
            &app_paths,
            &project_id,
            &issue_id,
            &plan_id,
            &clearing.command_id,
            None,
            false,
        )?;
    } else if lifecycle.as_ref().is_some_and(|metadata| {
        metadata.receipts.iter().any(|receipt| {
            receipt.command_id == query.command_id
                    // Replayed 仅当回执轮次仍是当前轮次（其后无新轮切换）；
                    // 更旧的回执只描述旧轮事实——不按旧 command 判重放，
                    // 继续按当前状态走分类路径（新成员按新操作门禁，不删新轮）。
                    && receipt.to_generation == current_generation
        })
    }) {
        // 同 command 已完整删除过的幂等重放：不再次清、不扫下一代 attempts。
        operation_state = "replayed";
    } else {
        let coding_store = CodingAttemptStore::new(app_paths.clone());
        let members = coding_store
            .list_attempts_for_plan(&project_id, &issue_id, &plan_id)
            .map_err(product_store_api_error)?;
        let run = run_store
            .get(&project_id, &issue_id, &plan_id)
            .map_err(product_store_api_error)?;
        let references = AdvanceStore::new(app_paths.clone())
            .plan_execution_references(&project_id, &issue_id, &plan_id)
            .map_err(product_store_api_error)?;
        let journals = coding_store
            .list_group_initialization_journals_for_plan(&project_id, &issue_id, &plan_id)
            .map_err(product_store_api_error)?;
        let residue = run.is_some()
            || !references.records.is_empty()
            || references.initialization_journal.is_some()
            || !journals.is_empty();
        if !members.is_empty() || residue {
            // 互证先行（零写拒绝）：confirmed=true 不是跨 generation 授权——
            // expected_generation 漂移即拒；本 command 存在旧轮回执
            // （to_generation < 当前，其后已发生轮次切换）同样不构成对当前
            // 轮次成员的授权，与互证同返 409（陈旧回执不判重放也不得整删
            // 新轮）。零成员 fall-through（清残留/幂等收尾）不进本分支。
            let stale_receipt = lifecycle.as_ref().is_some_and(|metadata| {
                metadata.receipts.iter().any(|receipt| {
                    receipt.command_id == query.command_id
                        && receipt.to_generation < current_generation
                })
            });
            if stale_receipt
                || query
                    .expected_generation
                    .is_some_and(|generation| generation != current_generation)
            {
                return Err(ApiError::runtime(
                    "coding_plan_workspace_generation_conflict",
                    "coding workspace 轮次已切换，本次删除未执行；请刷新后按当前状态操作",
                    json!({ "plan_id": plan_id }),
                ));
            }
            if query.confirmed != Some(true) {
                return Err(plan_coding_deletion_confirmation_required(
                    &plan_id,
                    &members,
                    run.as_ref(),
                ));
            }
            // 逐成员复用 N7 清理（服务端一个操作内完成，非浏览器四次单删）。
            for attempt in members {
                let attempt_id = attempt.id.clone();
                teardown_plan_coding_member(&state, &app_paths, attempt).await?;
                deleted_attempt_ids.push(attempt_id);
            }
            // H2a 零成员闭包收口：本 command 记回执（同 command 重试 Replayed）。
            close_plan_coding_workspace_round(
                &app_paths,
                &project_id,
                &issue_id,
                &plan_id,
                &query.command_id,
                None,
                false,
            )?;
        }
        // members 空 && 无残留：幂等成功，零弹窗零变更（Clean）。
    }
    // Bug1(owner 2026-10-16):删除动作自身具备残留清扫幂等——无论本次走
    // 续清/重放/成员拆卸/零成员哪条路径,收尾统一清扫仓侧 issue worktree
    // 残留目录(注册已删、目录留下的存量也一并清干净;存活引用受保护)。
    sweep_issue_worktree_residue(&app_paths, &project_id, &issue_id).await?;
    deleted_attempt_ids.sort();
    deleted_attempt_ids.dedup();
    // remaining 报权威事实（设计 C：前端不假装全删/失败展示剩余数）——服务端
    // 枚举当前存活集；Replayed 于新轮成员在场时亦如实报告（新内容非本
    // command 范围，不删不瞒）。
    let mut remaining_attempt_ids: Vec<String> = CodingAttemptStore::new(app_paths.clone())
        .list_attempts_for_plan(&project_id, &issue_id, &plan_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .map(|attempt| attempt.id)
        .collect();
    remaining_attempt_ids.sort();
    remaining_attempt_ids.dedup();
    let run = run_store
        .get(&project_id, &issue_id, &plan_id)
        .map_err(product_store_api_error)?;
    let workspace_lifecycle = plan_workspace_lifecycle_projection(
        &app_paths,
        &project_id,
        &issue_id,
        &plan_id,
        run.as_ref(),
    )
    .map_err(product_store_api_error)?;
    let generation = workspace_lifecycle.generation;
    Ok(Json(PlanCodingWorkspaceDeletionResultDto {
        plan_id,
        command_id: query.command_id,
        operation_state: operation_state.to_string(),
        generation,
        deleted_attempt_ids,
        remaining_attempt_ids,
        workspace_lifecycle,
    }))
}

/// 服务端三分类（设计 C 节确认分级）→ 稳定 409 `deletion_confirmation_required`
///（复用 D2.3 已铺面）：message 为浅/强分级文案权威（前端恰一次 window.confirm
/// 消费）；强=任一成员已开始执行（status≠Created/有 start claim）或 run 已冻结
/// 首启事实（selection/start_outcome）；浅=整组 Created 零执行内容或仅无成员
/// 技术残留；无对象无残留不弹窗（幂等成功）。
fn plan_coding_deletion_confirmation_required(
    plan_id: &str,
    members: &[crate::product::coding_models::CodingExecutionAttempt],
    run: Option<&CodingPlanRun>,
) -> ApiError {
    let strong = members.iter().any(|attempt| {
        attempt.status != CodingAttemptStatus::Created || attempt.start_claim.is_some()
    }) || run
        .is_some_and(|run| run.selection.is_some() || run.start_outcome.is_some());
    let (level, message) = if members.is_empty() {
        (
            "shallow",
            "本计划 Coding 工作区已删除完毕，仍存在残留执行引用：本次操作将清理残留并重置执行轮，保留计划与工作项。"
                .to_string(),
        )
    } else if strong {
        (
            "strong",
            format!(
                "将终止并删除本计划全部 {} 个 Coding 工作区：运行中的 Coding 会先终止；日志、测试与 worktree 内容不可恢复；计划与工作项保留。",
                members.len()
            ),
        )
    } else {
        (
            "shallow",
            format!(
                "将删除本计划 {} 个 Coding 初始化工作区：仅删除 Coding 工作区与初始化内容，保留计划与工作项。",
                members.len()
            ),
        )
    };
    ApiError::runtime(
        "deletion_confirmation_required",
        message,
        json!({
            "level": level,
            "member_count": members.len(),
            "plan_id": plan_id,
        }),
    )
}

/// 组删单成员清理：复用单删端点（N7）的同一清理链——abort runner→mutation
/// lease→Git 终止协调→组初始化 arbitration→worktree 锁释放（含 owner 幂等
/// 释放）→workspace/branch 清理→handoff revision 清理→`delete_attempt`→
/// N7 run 失效→锁残留。与 `finalize_coding_attempt_deletion` 的差别：不接
/// 最后成员钩子（闭包由本编排以 DELETE command 收口，回执/续清按本 command
/// 幂等），仓维/issue 维 shared worktree 清理并入 H2a 闭包的 manifest 清理。
#[allow(clippy::too_many_arguments)]
async fn teardown_plan_coding_member(
    state: &WebAppState,
    app_paths: &ProductAppPaths,
    attempt: crate::product::coding_models::CodingExecutionAttempt,
) -> ApiResult<()> {
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let attempt_key = CodingAttemptRunKey::from_attempt(&attempt);
    state.coding_runs.abort_attempt(&attempt_key).await;
    let _mutation_lease = state.coding_runs.lock_attempt_mutation(&attempt_key).await;
    let attempt = coding_store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .map_err(product_store_api_error)?;
    let attempt = coding_workspace_engine_with_dummy_events(coding_store.clone())
        .reconcile_coding_git_operation_for_termination(&attempt)
        .await
        .map_err(coding_termination_reconcile_api_error)?;
    let _group_initialization_guard = if matches!(attempt.scope, CodingAttemptScope::WorkItemGroup)
    {
        Some(
            coding_store
                .acquire_group_initialization_arbitration_async(
                    &attempt.project_id,
                    &attempt.issue_id,
                )
                .await
                .map_err(product_store_api_error)?,
        )
    } else {
        None
    };
    let active_work_item_id = attempt
        .current_work_item_id
        .as_deref()
        .unwrap_or(&attempt.work_item_id);
    let repository = super::resolve_attempt_repository(app_paths, &attempt)?;
    if let Ok(Some(shared)) =
        lifecycle.get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)
        && shared.current_active_work_item_id.as_deref() == Some(active_work_item_id)
    {
        coding_workspace_engine_with_dummy_events(coding_store.clone())
            .handle_delete_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .await
            .map_err(coding_workspace_api_error)?;
    } else if let Ok(Some(shared)) =
        lifecycle.get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)
        && shared.current_lock_owner_id.as_deref() == Some(attempt.id.as_str())
    {
        let _ = lifecycle.release_issue_worktree_lock_by_owner(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        );
    }
    cleanup_coding_attempt_workspace(&repository, &attempt).await?;
    super::cleanup_attempt_handoff_revisions(app_paths, &coding_store, &attempt)
        .map_err(product_store_api_error)?;
    let attempt_work_item_ids = collect_attempt_work_item_ids(&coding_store, &attempt);
    coding_store
        .delete_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .map_err(product_store_api_error)?;
    // N7：成员删除即整 run 失效（部分删除现场不得再被旧授权首启/收养）。
    CodingPlanRunStore::new(app_paths.clone())
        .invalidate_for_deleted_attempt(&attempt)
        .map_err(product_store_api_error)?;
    purge_coding_attempt_lock_residue(app_paths, &attempt, &attempt_work_item_ids);
    Ok(())
}

/// H4（设计 A 状态表/C 节）：开始动作的半残现场收敛——只在 Plan 准入＋组
/// 串行键内做（检测、耐久 clearing、重新准入是同一受保护操作）：
///
/// - clearing 在案：以 `clearing.command_id` 身份续清（H2a 硬约束：同 command
///   续清/异 command 拒），完成后按请求正常新建；
/// - 零成员且旧执行引用已证断裂：H2a 零成员闭包完整重置（`require_broken_
///   proof=true`，正常初始化前缀不重置），回执记在请求 command 下（同
///   command 重试返回/续做这一次操作，不再次清新轮）；
/// - 同 command 承接（设计 C 行135）：现场若恰有请求 command 的空 Prepared
///   前缀 run（无 claim/provider 事实），先清该前缀再闭包，避免闭包退役
///   请求 command 后同动作自拒；原已 Started/Invalidated run 一律退役；
/// - 有成员现场零动作（N2/prepare 权威语义不变）。
fn converge_stale_plan_coding_round(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
    request_command_id: &str,
) -> ApiResult<()> {
    let run_store = CodingPlanRunStore::new(app_paths.clone());
    let lifecycle = run_store
        .get_workspace_lifecycle(project_id, issue_id, plan_id)
        .map_err(product_store_api_error)?;
    if let Some(clearing) = lifecycle
        .as_ref()
        .and_then(|metadata| metadata.clearing.as_ref())
    {
        close_plan_coding_workspace_round(
            app_paths,
            project_id,
            issue_id,
            plan_id,
            &clearing.command_id,
            None,
            true,
        )?;
        return Ok(());
    }
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    if !coding_store
        .list_attempts_for_plan(project_id, issue_id, plan_id)
        .map_err(product_store_api_error)?
        .is_empty()
    {
        return Ok(());
    }
    let run = run_store
        .get(project_id, issue_id, plan_id)
        .map_err(product_store_api_error)?;
    if let Some(run) = &run
        && run.command_id == request_command_id
        && run.phase == CodingPlanRunPhase::Prepared
        && run.target_attempts.is_empty()
        && run.selection.is_none()
        && run.start_outcome.is_none()
    {
        run_store
            .purge_plan(project_id, issue_id, plan_id)
            .map_err(product_store_api_error)?;
    }
    // Reset=半残已收敛（新轮由随后的 prepare 新建）；InitializationPrefix/
    // Clean=零动作（正常初始化前缀续原协议/正常新建）。
    close_plan_coding_workspace_round(
        app_paths,
        project_id,
        issue_id,
        plan_id,
        request_command_id,
        None,
        true,
    )?;
    Ok(())
}

/// H4（设计 C 节）：workspace_lifecycle 事实投影——generation/现存/缺失成员
/// 计数与状态。数据来自 run 冻结集 ∪ Ready record 绑定 ∪（已越
/// AttemptPersisted 的）外层 journal/per-target journal 绑定与真实存活集的
/// 交叉核验（设计 A 节判据），不从 attempt_status=aborted 猜物理存在性；
/// 未越 AttemptPersisted 的绑定是初始化前缀，不计缺失。
fn plan_workspace_lifecycle_projection(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
    run: Option<&CodingPlanRun>,
) -> Result<PlanWorkspaceLifecycleDto, ProductStoreError> {
    let generation = CodingPlanRunStore::new(paths.clone())
        .current_workspace_generation(project_id, issue_id, plan_id)?;
    let existing: BTreeSet<String> = CodingAttemptStore::new(paths.clone())
        .list_attempts_for_plan(project_id, issue_id, plan_id)?
        .into_iter()
        .map(|attempt| attempt.id)
        .collect();
    let references = AdvanceStore::new(paths.clone())
        .plan_execution_references(project_id, issue_id, plan_id)?;
    let mut referenced: BTreeSet<String> = BTreeSet::new();
    if let Some(run) = run {
        for binding in &run.target_attempts {
            referenced.insert(binding.attempt_id.clone());
        }
        if let Some(selection) = &run.selection {
            referenced.insert(selection.attempt_id.clone());
        }
    }
    referenced.extend(references.ready_bound_attempt_ids());
    let journal_beyond_attempt_persisted =
        references
            .initialization_journal
            .as_ref()
            .is_some_and(|journal| {
                journal.phase.order_for_engine()
                    >= AdvanceInitializationPhase::AttemptPersisted.order_for_engine()
            });
    if journal_beyond_attempt_persisted {
        referenced.extend(references.journal_bound_attempt_ids());
    }
    let journals = CodingAttemptStore::new(paths.clone())
        .list_group_initialization_journals_for_plan(project_id, issue_id, plan_id)?;
    for journal in &journals {
        if journal
            .phase
            .has_reached(CodingGroupInitializationPhase::AttemptPersisted)
        {
            referenced.insert(journal.attempt.id.clone());
        }
    }
    let missing = referenced.difference(&existing).count();
    let clearing = CodingPlanRunStore::new(paths.clone())
        .get_workspace_lifecycle(project_id, issue_id, plan_id)?
        .is_some_and(|metadata| metadata.clearing.is_some());
    let state = if clearing {
        "clearing"
    } else if missing > 0 {
        "incomplete"
    } else if !existing.is_empty() {
        "complete"
    } else {
        "clean"
    };
    Ok(PlanWorkspaceLifecycleDto {
        generation,
        existing_member_count: existing.len(),
        missing_member_count: missing,
        state: state.to_string(),
    })
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
        // S2a（owner 2026-10-14）语义反转：Paused 不再 409 not_enabled——
        // resume 回执相位后按既有事实返回，续跑推进由入口统一核心重估。
        CodingPlanRunPhase::Paused => {
            let resumed = run_store
                .resume_from_pause(&run)
                .map_err(product_store_api_error)?;
            let phase = resumed.phase;
            Ok((resumed, PostFlow::Existing(phase)))
        }
        // prepare 已拒绝 Invalidated（coding_plan_run_not_active）。
        CodingPlanRunPhase::Invalidated => {
            Err(product_store_api_error(ProductStoreError::Conflict {
                kind: "coding_plan_run_not_active",
                id: run.command_id.clone(),
            }))
        }
    }
}

/// S2a 续跑驱动结果：本轮全局重估后的推进事实（纯 ID，供入口/引擎
/// 挂点观察；不做第二调度账本——启动事实权威仍是 attempt claim/runner）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PlanContinuationDriveOutcome {
    /// 本轮新 start 的 target attempt（冻结确定性 command 首次消费）。
    pub started_attempt_ids: Vec<String>,
    /// 本轮跨桶桥续跑（物化就绪 unit + 重启 runner）的 attempt。
    pub resumed_attempt_ids: Vec<String>,
    /// 本轮就地续跑(runner 在册命中 in_place:仅物化就绪 unit,不重复
    /// 拉起 runner——在册 runner 重读 attempt 后继续排程)的 attempt。
    pub continued_in_place_attempt_ids: Vec<String>,
    /// 未推进的 attempt：忙（已启动/在册 runner/claim 已消费）、人控或
    /// 终态事实（失败链按事实停，不误报为错误）。
    pub busy_attempt_ids: Vec<String>,
}

/// S2a 续跑全自动化核心（owner 2026-10-14 直批，自动化编排器引擎种子）：
/// 按权威 plan binding 全 units + 各 attempt 完成事实做**全局**就绪重估
///（就绪=直接依赖全部 Completed 且未启动；与 `select_first_ready_unit`
/// 同源的全图完成证据），就绪集>1 并行 start；幂等（已启动跳过）；失败
/// 链（依赖未 Completed）不启动，独立链继续。
///
/// 跨桶桥：桶内 SC 依赖门只见本 attempt units（桶外上游恒 Waiting），
/// 本核心以全图完成事实补全该视角——Created attempt 的根 unit 就绪即开桶
///（`start_coding_once` 单发幂等，command=冻结 run.command_id 的确定性
/// 派生）；Running 无 active unit 的 parked attempt 物化确定性首个就绪
/// Pending unit 并重启 runner。人控/终态 attempt 零动作。
///
/// 作用域与锁：按单 (project,issue,plan) 设计，无跨 plan 全局串行假设；
/// **本函数不自取** per-plan 串行键 `work_item_group:{project}:{issue}:
/// {plan}`（非重入）——HTTP 入口已持有；engine 完成链挂点（S2b）调用前
/// 须先取同键。相位语义：无 run/首启窗口（Prepared/ReadyBound/
/// StartRequested）零动作（首启编排归 POST 重放链）；Paused 先 resume；
/// Invalidated fail-closed。
pub(crate) async fn drive_ready_units_for_plan(
    state: &WebAppState,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
    in_place_attempt_id: Option<&str>,
) -> Result<PlanContinuationDriveOutcome, ApiError> {
    let app_paths = product_app_paths(state);
    let run_store = CodingPlanRunStore::new(app_paths.clone());
    let Some(mut run) = run_store
        .get(project_id, issue_id, plan_id)
        .map_err(product_store_api_error)?
    else {
        // 无授权 run：零动作（completion 挂点对未走 plan 入口的组零推进）。
        return Ok(PlanContinuationDriveOutcome::default());
    };
    if matches!(run.phase, CodingPlanRunPhase::Invalidated) {
        return Err(product_store_api_error(ProductStoreError::Conflict {
            kind: "coding_plan_run_not_active",
            id: run.command_id.clone(),
        }));
    }
    if matches!(run.phase, CodingPlanRunPhase::Paused) {
        run = run_store
            .resume_from_pause(&run)
            .map_err(product_store_api_error)?;
    }
    if !matches!(
        run.phase,
        CodingPlanRunPhase::Started
            | CodingPlanRunPhase::AlreadyStarted
            | CodingPlanRunPhase::NeedsHuman
    ) {
        // 首启窗口（含无回执 Paused 的 fail-closed 残留）：零动作。
        return Ok(PlanContinuationDriveOutcome::default());
    }

    let store = CodingAttemptStore::new(app_paths.clone());
    let authoritative = store
        .resolve_authoritative_group_plan_binding_for_revision(
            project_id,
            issue_id,
            plan_id,
            &run.bound_plan_revision_id,
        )
        .map_err(product_store_api_error)?;
    let ranks = global_topological_ranks(&authoritative.units);

    // 冻结绑定集事实采集（成员删除场景 run 已 Invalidated，此处 NotFound
    // 为防御性跳过）；全局完成集跨桶累计。
    let bindings = sorted_target_bindings(run.target_attempts.clone());
    let mut facts = Vec::new();
    let mut completed: BTreeSet<String> = BTreeSet::new();
    for binding in &bindings {
        let attempt = match store.get_attempt(project_id, issue_id, &binding.attempt_id) {
            Ok(attempt) => attempt,
            Err(ProductStoreError::NotFound { .. }) => continue,
            Err(error) => return Err(product_store_api_error(error)),
        };
        let units = store
            .list_coding_units(project_id, issue_id, &binding.attempt_id)
            .map_err(product_store_api_error)?;
        for unit in &units {
            if unit.status == CodingExecutionUnitStatus::Completed {
                completed.insert(unit.logical_work_item_id.clone());
            }
        }
        facts.push((binding.clone(), attempt, units));
    }

    let mut outcome = PlanContinuationDriveOutcome::default();
    let mut rotated: Option<(usize, CodingPlanSelection)> = None;
    for (binding, attempt, units) in facts {
        let attempt_key = CodingAttemptRunKey::from_attempt(&attempt);
        let runner_live = state
            .coding_runs
            .attempt_is_reserved_or_running(&attempt_key);
        // 就绪判定（全图）：非终态且直接依赖全部 Completed。
        let unit_globally_ready = |unit: &CodingExecutionUnit| {
            !matches!(
                unit.status,
                CodingExecutionUnitStatus::Completed
                    | CodingExecutionUnitStatus::Failed
                    | CodingExecutionUnitStatus::Superseded
                    | CodingExecutionUnitStatus::Skipped
            ) && unit
                .dependency_logical_work_item_ids
                .iter()
                .all(|dependency| completed.contains(dependency))
        };
        let rank_of = |unit: &CodingExecutionUnit| {
            ranks
                .get(&unit.logical_work_item_id)
                .copied()
                .unwrap_or(usize::MAX)
        };
        match attempt.status {
            CodingAttemptStatus::Created => {
                // 开桶条件：桶根（建组物化的 order_index 0）全局就绪。
                let Some(root) = units.iter().min_by_key(|unit| unit.order_index) else {
                    continue;
                };
                if !unit_globally_ready(root) {
                    continue;
                }
                let command_id = CodingPlanSelection::deterministic_start_command_id(
                    project_id,
                    issue_id,
                    plan_id,
                    &run.command_id,
                    &binding.target_repository_id,
                );
                match start_coding_once(
                    state,
                    project_id,
                    issue_id,
                    StartCodingCommand {
                        attempt_id: attempt.id.clone(),
                        command_id,
                        origin: CodingStartOrigin::Manual,
                    },
                )
                .await
                {
                    Ok(StartCodingOutcome::Started { attempt_id }) => {
                        outcome.started_attempt_ids.push(attempt_id);
                        let selection = CodingPlanSelection {
                            unit_id: root.id.clone(),
                            logical_work_item_id: root.logical_work_item_id.clone(),
                            attempt_id: binding.attempt_id.clone(),
                            target_repository_id: binding.target_repository_id.clone(),
                            start_command_id: CodingPlanSelection::deterministic_start_command_id(
                                project_id,
                                issue_id,
                                plan_id,
                                &run.command_id,
                                &binding.target_repository_id,
                            ),
                        };
                        rotated = Some(better_rotation(rotated, (rank_of(root), selection)));
                    }
                    // 已启动/人控事实：幂等跳过，不阻断其余链。
                    Ok(
                        StartCodingOutcome::AlreadyStarted { attempt_id }
                        | StartCodingOutcome::NeedsHuman { attempt_id, .. },
                    ) => {
                        outcome.busy_attempt_ids.push(attempt_id);
                    }
                    Err(error) => {
                        return Err(ApiError::runtime(
                            "plan_coding_continuation_start_failed",
                            format!("continuation start for plan coding failed: {error}"),
                            serde_json::json!({ "code": error.code() }),
                        ));
                    }
                }
            }
            CodingAttemptStatus::Running => {
                // S2 切片B:完成链挂点的在册 runner 自驱——in_place 命中本
                // attempt 时不判 runner_live busy(下方只物化不 spawn,由该
                // 在册 runner 重读后继续排程)。
                let continues_in_place =
                    runner_live && in_place_attempt_id == Some(attempt.id.as_str());
                if (runner_live && !continues_in_place) || attempt.active_unit_id.is_some() {
                    // 忙：桶内推进归在册 runner（single-active-unit 不变量）。
                    outcome.busy_attempt_ids.push(attempt.id.clone());
                    continue;
                }
                // 跨桶 parked：物化确定性首个就绪 Pending unit 并重启 runner。
                let candidate = units
                    .iter()
                    .filter(|unit| {
                        unit.status == CodingExecutionUnitStatus::Pending
                            && unit_globally_ready(unit)
                    })
                    .min_by(|left, right| {
                        rank_of(left)
                            .cmp(&rank_of(right))
                            .then_with(|| left.order_index.cmp(&right.order_index))
                            .then_with(|| left.id.cmp(&right.id))
                    });
                let Some(unit) = candidate else {
                    continue;
                };
                if continues_in_place {
                    // 在册 runner 现场:只物化(状态/索引/锁转移),不重复
                    // 拉起 runner、不轮换 selection——在册 runner 重读
                    // attempt 后按 PrepareContext 继续内循环排程。
                    materialize_parked_plan_attempt(state, &app_paths, &store, &attempt, unit)
                        .await?;
                    outcome
                        .continued_in_place_attempt_ids
                        .push(attempt.id.clone());
                    continue;
                }
                if resume_parked_plan_attempt(state, &app_paths, &store, &attempt, unit).await? {
                    outcome.resumed_attempt_ids.push(attempt.id.clone());
                    let selection = CodingPlanSelection {
                        unit_id: unit.id.clone(),
                        logical_work_item_id: unit.logical_work_item_id.clone(),
                        attempt_id: binding.attempt_id.clone(),
                        target_repository_id: binding.target_repository_id.clone(),
                        start_command_id: CodingPlanSelection::deterministic_start_command_id(
                            project_id,
                            issue_id,
                            plan_id,
                            &run.command_id,
                            &binding.target_repository_id,
                        ),
                    };
                    rotated = Some(better_rotation(rotated, (rank_of(unit), selection)));
                }
            }
            // Blocked/WaitingForHuman/AwaitingManualRecovery/终态：人工/终局
            // 事实，零动作（失败链按事实停）。
            _ => outcome.busy_attempt_ids.push(attempt.id.clone()),
        }
    }
    if let Some((_rank, selection)) = rotated {
        run = run_store
            .rotate_selection(&run, selection)
            .map_err(product_store_api_error)?;
    }
    Ok(outcome)
}

/// 确定性 selection 轮换：取本轮新启 unit 中（全图拓扑位置 → logical
/// WI ID → target UUID）最小者（多桶并行启动时的稳定投影）。
fn better_rotation(
    current: Option<(usize, CodingPlanSelection)>,
    candidate: (usize, CodingPlanSelection),
) -> (usize, CodingPlanSelection) {
    match current {
        None => candidate,
        Some(current) => {
            let candidate_key = (
                candidate.0,
                candidate.1.logical_work_item_id.clone(),
                candidate.1.target_repository_id.clone(),
            );
            let current_key = (
                current.0,
                current.1.logical_work_item_id.clone(),
                current.1.target_repository_id.clone(),
            );
            if candidate_key < current_key {
                candidate
            } else {
                current
            }
        }
    }
}

/// 入口续跑门：冻结绑定集内是否已存在终态 unit（Completed/Failed/
/// Superseded/Skipped）——有推进诱因（完成/失败）才重估，纯重放/首启
/// 窗口保持零新启（n3 族单选首启冻结）。
fn plan_has_terminal_coding_unit(
    paths: &ProductAppPaths,
    run: &CodingPlanRun,
) -> Result<bool, ProductStoreError> {
    let store = CodingAttemptStore::new(paths.clone());
    for binding in &run.target_attempts {
        for unit in store.list_coding_units(&run.project_id, &run.issue_id, &binding.attempt_id)? {
            if matches!(
                unit.status,
                CodingExecutionUnitStatus::Completed
                    | CodingExecutionUnitStatus::Failed
                    | CodingExecutionUnitStatus::Superseded
                    | CodingExecutionUnitStatus::Skipped
            ) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// 跨桶桥 parked 物化(S2 切片B 拆段):就绪 unit 的状态/索引修复——unit
/// →Running、current/active 指针对齐、stage 回 PrepareContext、worktree
/// 锁转移。与 `advance_to_next_group_unit` 的 next 分支同源,但就绪判定按
/// 全图完成事实;不含 runner 拉起(in_place 续跑只物化,由在册 runner
/// 重读 attempt 后继续排程)。
#[allow(clippy::too_many_arguments)]
async fn materialize_parked_plan_attempt(
    state: &WebAppState,
    app_paths: &ProductAppPaths,
    store: &CodingAttemptStore,
    attempt: &crate::product::coding_models::CodingExecutionAttempt,
    unit: &CodingExecutionUnit,
) -> Result<crate::product::coding_models::CodingExecutionAttempt, ApiError> {
    // 锁转移 from 取**推进前**身份（与引擎 advance_to_next_group_unit 同款
    // 先例）：park 形状的 current 指针仍是上一完成 unit（真实锁记录的
    // active 端）；先物化再读会把 from 同步成新 unit（from==to 自转移），
    // 撞真实记录的 repo/issue_worktree_active。缺 current 时回落桶根
    // work_item_id。
    let from_work_item_id = attempt
        .current_work_item_id
        .clone()
        .unwrap_or_else(|| attempt.work_item_id.clone());
    let unit = store
        .update_coding_unit_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &unit.id,
            CodingExecutionUnitStatus::Running,
            Some("续跑：进入下一个 Work Item".to_string()),
        )
        .map_err(product_store_api_error)?;
    let mut updated = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .map_err(product_store_api_error)?;
    updated.current_work_item_id = Some(unit.logical_work_item_id.clone());
    updated.active_unit_id = Some(unit.id.clone());
    updated.stage = crate::product::coding_models::CodingExecutionStage::PrepareContext;
    updated.updated_at = chrono::Utc::now().to_rfc3339();
    store
        .update_attempt_non_status_fields(&updated)
        .map_err(product_store_api_error)?;

    // worktree 锁转移（与 advance_to_next_group_unit 同源；锁事实在
    // repo 维三元键，缺转移会让后续 completion 的 owner 校验 fail-closed）。
    let attempt_key = CodingAttemptRunKey::from_attempt(&updated);
    let event_tx = state.coding_sockets.hub_sender(&attempt_key);
    let engine = crate::product::coding_workspace_engine::CodingWorkspaceEngine::new(
        store.clone(),
        crate::product::git_workspace_service::GitWorkspaceService::new(),
        event_tx,
    );
    let route = engine
        .route_issue_shared_worktree(&updated)
        .map_err(|error| {
            ApiError::runtime(
                "plan_coding_continuation_resume_failed",
                format!("route shared worktree for continuation resume failed: {error}"),
                serde_json::json!({}),
            )
        })?;
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let transfer_result: Result<(), ProductStoreError> = match route {
        crate::product::coding_workspace_engine::IssueSharedWorktreeRoute::Legacy => {
            if lifecycle
                .get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)
                .map_err(product_store_api_error)?
                .is_some()
            {
                lifecycle
                    .transfer_issue_worktree_lock(
                        &attempt.project_id,
                        &attempt.issue_id,
                        &from_work_item_id,
                        &unit.logical_work_item_id,
                        &attempt.id,
                    )
                    .map(|_| ())
            } else {
                Ok(())
            }
        }
        crate::product::coding_workspace_engine::IssueSharedWorktreeRoute::Repository {
            repository_id,
        } => lifecycle
            .transfer_repo_worktree_lock(
                &attempt.project_id,
                &attempt.issue_id,
                repository_id,
                &from_work_item_id,
                &unit.logical_work_item_id,
                &attempt.id,
            )
            .map(|_| ()),
    };
    transfer_result.map_err(|error| {
        ApiError::runtime(
            "plan_coding_continuation_resume_failed",
            format!("transfer shared worktree lock for continuation resume failed: {error}"),
            serde_json::json!({}),
        )
    })?;
    Ok(updated)
}

/// 跨桶桥 parked 续跑（Running 且无 active unit 的 attempt）：物化就绪
/// unit（`materialize_parked_plan_attempt`：Running+指针对齐、stage 回
/// PrepareContext、worktree 锁转移）、重启 runner——与
/// `advance_to_next_group_unit` 的 next 分支同源，但就绪判定按全图完成
/// 事实。返回 false=registry 拒绝（已有 runner，幂等）。
#[allow(clippy::too_many_arguments)]
async fn resume_parked_plan_attempt(
    state: &WebAppState,
    app_paths: &ProductAppPaths,
    store: &CodingAttemptStore,
    attempt: &crate::product::coding_models::CodingExecutionAttempt,
    unit: &CodingExecutionUnit,
) -> Result<bool, ApiError> {
    let updated = materialize_parked_plan_attempt(state, app_paths, store, attempt, unit).await?;
    let event_tx = state
        .coding_sockets
        .hub_sender(&CodingAttemptRunKey::from_attempt(&updated));
    Ok(crate::web::coding_ws_handler::spawn_coding_runner(
        state.clone(),
        store.clone(),
        event_tx,
        updated,
    )
    .is_some())
}

/// plan scope 删除准入：try-shared 争用/墓碑一律立即
/// `Conflict{entity_deleting}`，零阻塞等待（沿 EntityOperationStore 现语义）。
pub(super) fn plan_live_admission(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
) -> Result<(), ProductStoreError> {
    let scope = EntityScope::new(project_id, issue_id, DeletionEntityKind::Plan, plan_id);
    scope.validate()?;
    EntityOperationStore::new(paths.clone()).with_live_write(&scope, || Ok(()))
}

pub(super) fn ensure_plan_confirmed(
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

/// S3（Q5）：plan target 仓展示名（`resolve_plan_target_repository_name`
/// 同款解析）：logical id 按 issue 归属 codebase 走 strict/per-LC 权威解析，
/// 取 checkout 路径末段目录名；解析不出末段回落 logical id 字符串。
fn plan_target_repository_display_name(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_lc_id: Option<&str>,
    target_repository_id: &str,
) -> Result<String, ProductStoreError> {
    let Ok(repository_id) = uuid::Uuid::parse_str(target_repository_id) else {
        return Ok(target_repository_id.to_string());
    };
    let project = ProjectStore::new(paths.clone()).get(project_id)?;
    let (_, checkout, _) = RepositoryStore::for_project(paths.clone(), &project)
        .resolve_logical_repository_for_issue_codebase(
            project_id,
            issue_lc_id,
            LogicalRepositoryId(repository_id),
        )?;
    Ok(checkout
        .canonical_path
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| target_repository_id.to_string()))
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
    // S3（Q5）：仓展示名解析按 issue 归属 codebase（同 plan_group_projection
    // 口径，逐 target 只读派生）。
    let issue = IssueStore::new(paths.clone()).get(project_id, issue_id)?;
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
        // S3（Q5 逐仓 PR 面板）：attempt 面事实 + 最新 ReviewRequest 推送面；
        // 成员已删（attempt None）或无对应事实时逐字段为 None。
        let repository_name = plan_target_repository_display_name(
            paths,
            project_id,
            issue.logical_codebase_id.as_deref(),
            &facts.binding.target_repository_id,
        )?;
        let latest_review_request = match &facts.attempt {
            Some(attempt) => store
                .list_review_requests(project_id, issue_id, &attempt.id)?
                .into_iter()
                .next_back(),
            None => None,
        };
        let (branch_name, base_branch, stage, head_commit, manual_recovery_reason) =
            match &facts.attempt {
                Some(attempt) => (
                    Some(attempt.branch_name.clone()),
                    Some(attempt.base_branch.clone()),
                    Some(coding_execution_stage_text(&attempt.stage).to_string()),
                    attempt.head_commit.clone(),
                    attempt.manual_recovery_reason.clone(),
                ),
                None => (None, None, None, None, None),
            };
        let (push_status, push_error, review_request_id) = match latest_review_request.as_ref() {
            Some(review) => (
                Some(push_status_text(&review.push_status).to_string()),
                review.push_error.clone(),
                Some(review.id.clone()),
            ),
            None => (None, None, None),
        };
        target_attempts.push(PlanCodingTargetDto {
            target_repository_id: facts.binding.target_repository_id.clone(),
            attempt_id: facts.binding.attempt_id.clone(),
            attempt_status,
            worktree_path,
            lease_state,
            repository_name: Some(repository_name),
            branch_name,
            base_branch,
            stage,
            head_commit,
            push_status,
            push_error,
            review_request_id,
            manual_recovery_reason,
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
        // Slice B（仪表盘跨仓依赖投影）：全量直接依赖取权威绑定（跨桶边
        // 不因本 attempt 桶缺席而丢失），与全图拓扑序一并供前端重建 plan
        // 全图拓扑/依赖链。
        let mut dependency_logical_work_item_ids =
            auth_unit.dependency_logical_work_item_ids.clone();
        dependency_logical_work_item_ids.sort();
        // provider 事实（durable claim 相位 ≥ ProviderMayHaveStarted + active
        // 指针 + unit Running）与初始化桶首 unit Running（物化事实）严格分开
        //（REQ-CS-04）：Claimed/RunnerRegistered 回落 initialized。
        // S3（Q3 投影真实化）：provider running 还需 attempt.status==Running——
        // AMR/人控/终态现场的 unit 仍 Running、claim 仍 ≥Provider，但 attempt
        // 已不在执行（POST 回执窗口 admission 亦未跨），不得投影 running。
        let attempt_status = facts.attempt.as_ref().map(|attempt| attempt.status.clone());
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
            && attempt_status == Some(CodingAttemptStatus::Running)
            && unit.status == CodingExecutionUnitStatus::Running;
        let (execution_state, waiting_reason): (&str, Option<&str>) =
            if unit.status == CodingExecutionUnitStatus::Completed {
                // 完成事实不因授权失效回滚（REQ-CS-05/07）。
                ("completed", None)
            } else if invalidated {
                // run Invalidated：整体失效文案（成员删除授权失效）。
                ("invalidated", None)
            } else if attempt_status == Some(CodingAttemptStatus::AwaitingManualRecovery) {
                // S3（Q3）：attempt 级屏幕优先于 unit 级等待/执行投影——AMR
                // 现场 unit 仍 Running（恢复复用，不改写底层事实），真实事实
                // 是等待人工恢复；waiting_reason 原样透传 manual_recovery_reason
                //（开放值集）。
                (
                    "awaiting_manual_recovery",
                    facts
                        .attempt
                        .as_ref()
                        .and_then(|attempt| attempt.manual_recovery_reason.as_deref()),
                )
            } else if attempt_status == Some(CodingAttemptStatus::Failed) {
                ("failed", None)
            } else if attempt_status == Some(CodingAttemptStatus::Aborted) {
                ("aborted", None)
            } else if matches!(
                attempt_status,
                Some(CodingAttemptStatus::Blocked | CodingAttemptStatus::WaitingForHuman)
            ) {
                ("waiting_human", None)
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
            dependency_logical_work_item_ids,
            order_index: ranks
                .get(&auth_unit.logical_work_item_id)
                .copied()
                .unwrap_or(usize::MAX),
            summary: unit.summary.clone(),
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
        // P1-a（REQ-MTG-11 / 契约 A2）：plan 级聚合最终确认投影（共享纯
        // 函数只读持久事实，供块 4 attention 同源复用）。
        final_confirm: super::plan_final_confirm::build_plan_final_confirm(
            paths, project_id, issue_id, plan_id,
        )?,
        workspace_lifecycle: plan_workspace_lifecycle_projection(
            paths, project_id, issue_id, plan_id, run,
        )?,
        // P4-c（契约 A1 / 3* 第 4 段）：本 plan 同源待处理事项——多 target
        // plan 的 plan_final_confirm 聚一条，逐仓 final_confirm 抑制；无
        // plan 归属的事项（手动单仓等）不进本 plan 视图。
        attention_items: crate::web::coding_attention_info::issue_coding_attention_items(
            paths, project_id, issue_id,
        )?
        .into_iter()
        .filter(|item| item.plan_id.as_deref() == Some(plan_id))
        .collect(),
    })
}
