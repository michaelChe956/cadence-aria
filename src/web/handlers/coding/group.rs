use std::collections::{BTreeMap, BTreeSet};

use super::super::dto::*;
use super::super::support::*;
use super::super::*;
use super::{
    RuntimeBindingProviderConfigInput, coding_provider_config_snapshot_for_runtime_binding,
};
use crate::product::coding_attempt_store::target_snapshot::build_attempt_target_snapshot;
use crate::product::coding_attempt_store::{
    AuthoritativeGroupPlanBinding, CodingGroupInitializationPhase,
};
use crate::product::coding_models::AttemptTargetSnapshot;
use crate::product::coding_models::CodingAdmissionKind;
use crate::product::issue_store::IssueStore;
use crate::product::logical_codebase::{
    LogicalRepositoryId, RepositoryRouting, RepositoryRoutingErrorCode, SelectionPolicy,
};
use crate::product::work_item_revision_store::WorkItemRevisionStore;

pub async fn create_group_coding_attempt(
    State(state): State<WebAppState>,
    Path((project_id, issue_id, plan_id)): Path<(String, String, String)>,
) -> ApiResult<Json<CodingAttemptDto>> {
    let app_paths = product_app_paths(&state);
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let plan = lifecycle
        .get_issue_work_item_plan(&project_id, &issue_id, &plan_id)
        .map_err(product_store_api_error)?;
    if plan.status != IssueWorkItemPlanStatus::Confirmed {
        return Err(ApiError::validation(
            "work_item_plan_not_confirmed",
            "work item plan must be confirmed before group coding",
        ));
    }

    let group_lock_key = format!("work_item_group:{project_id}:{issue_id}:{plan_id}");
    let _group_guard = state.coding_runs.lock_named(&group_lock_key).await;
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    let pending_journal =
        match coding_store.get_group_initialization(&project_id, &issue_id, &plan_id) {
            Ok(journal) => Some(journal),
            Err(ProductStoreError::NotFound {
                kind: "coding_group_initialization_journal",
                ..
            }) => None,
            Err(error) => return Err(coding_group_attempt_incomplete_api_error(error)),
        };
    if let Some(journal) = pending_journal.as_ref()
        && journal.phase != CodingGroupInitializationPhase::Completed
    {
        let active_revision_id = WorkItemRevisionStore::new(app_paths.clone())
            .get_plan_lineage(&project_id, &issue_id, &plan_id)
            .map_err(product_store_api_error)?
            .active_revision_id;
        if active_revision_id.as_deref()
            != Some(journal.plan_binding.bound_plan_revision_id.as_str())
        {
            return Err(coding_group_attempt_incomplete_api_error(
                ProductStoreError::IdentityMismatch {
                    kind: "coding_group_initialization_plan_revision",
                    id: journal.attempt.id.clone(),
                },
            ));
        }
    }
    let authoritative = coding_store
        .resolve_authoritative_group_plan_binding(&project_id, &issue_id, &plan_id)
        .map_err(coding_plan_revision_binding_api_error)?;
    let current_unit = authoritative.units.first().ok_or_else(|| {
        coding_plan_revision_binding_api_error(ProductStoreError::IdentityMismatch {
            kind: "coding_group_order",
            id: plan_id.clone(),
        })
    })?;
    let repository = resolve_group_repository(&app_paths, &project_id, &issue_id, &authoritative)?;
    if !is_git_repo(&repository.path) {
        return Err(ApiError::validation(
            "repository_path_not_git_repo",
            "repository path must point to a git work tree",
        ));
    }
    // 已存在 journal 是本次初始化的权威重放输入：快照在 Prepared 时已冻结，
    // 不能重新 capture（captured_at 会变化）。仍由 journal_matches_request 对此值作
    // 全等校验，确保后续持久化的 attempt 与 journal 一致。
    // 准入身份同理：journal 可能由 sc_advance 入口创建（admission_kind=ScAdvance、
    // worktree_path=Some(...)），create 端点必须按 journal 冻结值重放收养，而非拿
    // 新建请求的 LegacyGroup/None 硬对；无 journal 时保持既有新建语义不变。
    // provider 快照同理（接缝修复轮 2）：journal 冻结的是 plan 会话选定的 provider
    // （advance_provider_config(&session, unit)，如 pi），而按仓库默认重算会回退到
    // repository_default_provider（如 codex），plan provider ≠ 默认时全等必败 →
    // 400 coding_group_attempt_incomplete。journal 存在时必须按冻结值重放；无 journal
    // 的新建路径保持既有重算语义不变。
    let target_snapshot = match pending_journal.as_ref() {
        Some(journal) => journal.attempt.target_snapshot.clone(),
        None => group_target_snapshot(&app_paths, &project_id, &issue_id, &authoritative)?,
    };
    let replay_admission_kind = pending_journal
        .as_ref()
        .map(|journal| journal.attempt.admission_kind)
        .unwrap_or(CodingAdmissionKind::LegacyGroup);
    let replay_worktree_path = pending_journal
        .as_ref()
        .and_then(|journal| journal.attempt.worktree_path.clone());
    let branch_name = format!("aria/issues/{issue_id}");
    let base_branch = current_git_branch(&repository.path).unwrap_or_else(|| "HEAD".to_string());
    let shared_worktree_path = repository
        .path
        .join(".worktrees")
        .join("aria-issues")
        .join(&issue_id);
    let provider_config_snapshot = match pending_journal.as_ref() {
        Some(journal) => journal.attempt.provider_config_snapshot.clone(),
        None => coding_provider_config_snapshot_for_runtime_binding(
            &lifecycle,
            RuntimeBindingProviderConfigInput {
                project_id: &project_id,
                issue_id: &issue_id,
                plan_id: &plan_id,
                plan_revision_id: &authoritative.plan_revision_id,
                unit: current_unit,
                repository_default_provider: &repository.default_provider_mode,
            },
            &*state.provider_availability,
        )?,
    };
    let initialization_input = CreateGroupCodingAttemptInput {
        project_id: project_id.clone(),
        issue_id: issue_id.clone(),
        plan_id: plan_id.clone(),
        current_work_item_id: current_unit.logical_work_item_id.clone(),
        base_branch: base_branch.clone(),
        branch_name: branch_name.clone(),
        worktree_path: replay_worktree_path,
        provider_config_snapshot,
        target_snapshot,
        max_auto_rework: 2,
    };

    let _initialization_guard = coding_store
        .acquire_group_initialization_arbitration_async(&project_id, &issue_id)
        .await
        .map_err(product_store_api_error)?;
    let creation_guard = coding_store
        .acquire_work_item_attempt_creation_async(
            &project_id,
            &issue_id,
            &current_unit.logical_work_item_id,
        )
        .await
        .map_err(product_store_api_error)?;
    let mut journal = coding_store
        .prepare_group_initialization_with_admission(
            &initialization_input,
            &authoritative.plan_revision_id,
            &authoritative.units,
            replay_admission_kind,
        )
        .map_err(group_initialization_api_error)?;
    maybe_interrupt_group_initialization(
        &state,
        crate::web::test_controls::GroupAttemptInitializationCheckpoint::PreparedBeforeAttemptPersisted,
    )?;
    if journal.phase == CodingGroupInitializationPhase::Completed {
        let existing = coding_store
            .get_attempt(&project_id, &issue_id, &journal.attempt.id)
            .map_err(coding_group_attempt_incomplete_api_error)?;
        coding_store
            .validate_group_attempt_integrity(&existing)
            .map_err(coding_group_attempt_incomplete_api_error)?;
        return Ok(Json(coding_attempt_dto(&coding_store, &existing)?));
    }

    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: project_id.clone(),
            issue_id: issue_id.clone(),
            repository_id: repository.id.clone(),
            branch_name,
            worktree_path: shared_worktree_path,
            base_branch,
        })
        .map_err(product_store_api_error)?;
    let worktree_lease = lifecycle
        .try_acquire_issue_worktree_lock(
            &project_id,
            &issue_id,
            &journal.lock_work_item_id,
            &journal.worktree_lease_id,
        )
        .map_err(issue_worktree_active_api_error)?;
    let replay_already_bound = journal
        .phase
        .has_reached(CodingGroupInitializationPhase::AttemptPersisted)
        && worktree_lease.worktree.current_lock_owner_id.as_deref()
            == Some(journal.attempt.id.as_str());
    if !worktree_lease.acquired && !replay_already_bound {
        return Err(coding_group_attempt_incomplete_api_error(
            ProductStoreError::IdentityMismatch {
                kind: "coding_group_worktree_lease",
                id: journal.attempt.id.clone(),
            },
        ));
    }
    if !worktree_lease.acquired {
        coding_store
            .validate_materialized_group_initialization_attempt(&journal, &creation_guard)
            .map_err(coding_group_attempt_incomplete_api_error)?;
    }
    state
        .test_controls
        .pause_group_attempt_after_worktree_acquire_if_configured()
        .await;

    let attempt = coding_store
        .ensure_group_initialization_attempt(&journal, &creation_guard)
        .map_err(coding_group_attempt_incomplete_api_error)?;
    journal = coding_store
        .advance_group_initialization_phase(
            &journal,
            CodingGroupInitializationPhase::AttemptPersisted,
        )
        .map_err(coding_group_attempt_incomplete_api_error)?;
    maybe_interrupt_group_initialization(
        &state,
        crate::web::test_controls::GroupAttemptInitializationCheckpoint::PersistedBeforeBind,
    )?;

    lifecycle
        .bind_issue_worktree_lock_to_attempt(
            &project_id,
            &issue_id,
            &journal.lock_work_item_id,
            &attempt.id,
        )
        .map_err(product_store_api_error)?;
    maybe_interrupt_group_initialization(
        &state,
        crate::web::test_controls::GroupAttemptInitializationCheckpoint::BoundBeforePhaseAdvance,
    )?;
    journal = coding_store
        .advance_group_initialization_phase(&journal, CodingGroupInitializationPhase::WorktreeBound)
        .map_err(coding_group_attempt_incomplete_api_error)?;
    maybe_interrupt_group_initialization(
        &state,
        crate::web::test_controls::GroupAttemptInitializationCheckpoint::BoundBeforePlanBinding,
    )?;

    coding_store
        .ensure_group_initialization_plan_binding(&journal)
        .map_err(coding_group_attempt_incomplete_api_error)?;
    journal = coding_store
        .advance_group_initialization_phase(
            &journal,
            CodingGroupInitializationPhase::PlanBindingSaved,
        )
        .map_err(coding_group_attempt_incomplete_api_error)?;
    for index in 0..journal.units.len() {
        coding_store
            .ensure_group_initialization_unit(&journal, index)
            .map_err(coding_group_attempt_incomplete_api_error)?;
        if index == 0 {
            maybe_interrupt_group_initialization(
                &state,
                crate::web::test_controls::GroupAttemptInitializationCheckpoint::FirstUnitPersisted,
            )?;
        }
    }
    journal = coding_store
        .advance_group_initialization_phase(
            &journal,
            CodingGroupInitializationPhase::UnitsMaterialized,
        )
        .map_err(coding_group_attempt_incomplete_api_error)?;

    let persisted_attempt = coding_store
        .get_attempt(&project_id, &issue_id, &attempt.id)
        .map_err(product_store_api_error)?;
    coding_store
        .validate_group_attempt_integrity(&persisted_attempt)
        .map_err(coding_group_attempt_incomplete_api_error)?;
    coding_store
        .advance_group_initialization_phase(&journal, CodingGroupInitializationPhase::Completed)
        .map_err(coding_group_attempt_incomplete_api_error)?;
    Ok(Json(coding_attempt_dto(&coding_store, &persisted_attempt)?))
}

fn group_target_snapshot(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    authoritative: &AuthoritativeGroupPlanBinding,
) -> ApiResult<Option<AttemptTargetSnapshot>> {
    // v1.3：按 issue 所属 lc_id 寻址（R9）；单仓/无 LC 回退 legacy project 级路径。
    let lc_id = crate::product::logical_codebase::resolve_issue_logical_codebase_id(
        app_paths, project_id, issue_id,
    )
    .map_err(product_store_api_error)?;
    match RepositoryRouting::load_for_issue(app_paths, project_id, issue_id)
        .map_err(product_store_api_error)?
    {
        RepositoryRouting::Legacy { .. } => Ok(None),
        RepositoryRouting::Logical {
            manifest,
            selection,
        } => {
            if let Some(reason) = authoritative
                .units
                .iter()
                .find_map(|unit| unit.source_draft_error.as_deref())
            {
                return Err(routing_api_error(
                    RepositoryRoutingErrorCode::Inconsistent,
                    reason,
                ));
            }
            let selected_ids = validate_logical_group_selection(
                app_paths,
                lc_id.as_deref(),
                project_id,
                &manifest,
                &selection,
            )?;
            let target_ids: BTreeSet<LogicalRepositoryId> = authoritative
                .units
                .iter()
                .filter_map(|unit| unit.target_repository_id)
                .collect();
            let logical_repository_id = match target_ids.len() {
                1 => *target_ids.first().expect("one group target exists"),
                0 => {
                    let [focus_repository_id] = selection.focus_repository_ids.as_slice() else {
                        return Err(routing_api_error(
                            RepositoryRoutingErrorCode::TargetMissing,
                            "group has no unique target repository and selection focus is not unique",
                        ));
                    };
                    *focus_repository_id
                }
                _ => {
                    return Err(routing_api_error(
                        RepositoryRoutingErrorCode::TargetAmbiguous,
                        "group has multiple target repositories",
                    ));
                }
            };
            if !selected_ids.contains(&logical_repository_id) {
                return Err(routing_api_error(
                    RepositoryRoutingErrorCode::TargetUnknown,
                    "group target repository is not in the effective selection",
                ));
            }
            build_attempt_target_snapshot(
                app_paths,
                project_id,
                logical_repository_id,
                lc_id.as_deref(),
            )
            .map(Some)
            .map_err(target_snapshot_api_error)
        }
        RepositoryRouting::FailClosed { code, reason } => Err(routing_api_error(code, &reason)),
    }
}

fn target_snapshot_api_error(
    error: crate::product::coding_attempt_store::target_snapshot::TargetSnapshotError,
) -> ApiError {
    ApiError::runtime(
        "product_store_error",
        "coding attempt target snapshot capture failed",
        json!({ "reason": error.to_string() }),
    )
}

fn resolve_group_repository(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    authoritative: &AuthoritativeGroupPlanBinding,
) -> ApiResult<RepositoryRecord> {
    // v1.3：按 issue 所属 lc_id 寻址（R9）；单仓/无 LC 回退 legacy project 级路径。
    let lc_id = crate::product::logical_codebase::resolve_issue_logical_codebase_id(
        app_paths, project_id, issue_id,
    )
    .map_err(product_store_api_error)?;
    match RepositoryRouting::load_for_issue(app_paths, project_id, issue_id)
        .map_err(product_store_api_error)?
    {
        RepositoryRouting::Legacy { .. } => {
            let repository_id = IssueStore::new(app_paths.clone())
                .get(project_id, issue_id)
                .map_err(product_store_api_error)?
                .repo_id
                .ok_or_else(|| {
                    product_store_api_error(ProductStoreError::NotFound {
                        kind: "repository",
                        id: format!("issue:{issue_id}:repo_id"),
                    })
                })?;
            resolve_legacy_group_repository(app_paths, project_id, &repository_id)
        }
        RepositoryRouting::Logical {
            manifest,
            selection,
        } => {
            if let Some(reason) = authoritative
                .units
                .iter()
                .find_map(|unit| unit.source_draft_error.as_deref())
            {
                return Err(routing_api_error(
                    RepositoryRoutingErrorCode::Inconsistent,
                    reason,
                ));
            }
            let selected_ids = validate_logical_group_selection(
                app_paths,
                lc_id.as_deref(),
                project_id,
                &manifest,
                &selection,
            )?;
            let target_ids: BTreeSet<LogicalRepositoryId> = authoritative
                .units
                .iter()
                .filter_map(|unit| unit.target_repository_id)
                .collect();
            let logical_repository_id = match target_ids.len() {
                1 => *target_ids.first().expect("one group target exists"),
                0 => {
                    let [focus_repository_id] = selection.focus_repository_ids.as_slice() else {
                        return Err(routing_api_error(
                            RepositoryRoutingErrorCode::TargetMissing,
                            "group has no unique target repository and selection focus is not unique",
                        ));
                    };
                    *focus_repository_id
                }
                _ => {
                    return Err(routing_api_error(
                        RepositoryRoutingErrorCode::TargetAmbiguous,
                        "group has multiple target repositories",
                    ));
                }
            };
            if !selected_ids.contains(&logical_repository_id) {
                return Err(routing_api_error(
                    RepositoryRoutingErrorCode::TargetUnknown,
                    "group target repository is not in the effective selection",
                ));
            }
            let store = match lc_id.as_deref() {
                Some(_) => RepositoryStore::new(app_paths.clone()),
                None => {
                    let project = ProjectStore::new(app_paths.clone())
                        .get(project_id)
                        .map_err(product_store_api_error)?;
                    RepositoryStore::for_project(app_paths.clone(), &project)
                }
            };
            store
                .resolve_logical_repository_for_issue_codebase(
                    project_id,
                    lc_id.as_deref(),
                    logical_repository_id,
                )
                .map(|(_, _, repository)| repository)
                .map_err(product_store_api_error)
        }
        RepositoryRouting::FailClosed { code, reason } => Err(routing_api_error(code, &reason)),
    }
}

/// REQ-COD-04（WP1 分流化，REQ-MTG-01）：split 解析面的公共前置——routing 三态
/// 判定 + fail-closed 校验（source_draft_error/selection）+ units target 收敛 +
/// 0-target focus 唯一回落（与单值面同语义）。多 target 不再构成拒绝理由，
/// 而是逐 target 进入分流解析。
enum SplitGroupTargets {
    Legacy,
    Logical {
        lc_id: Option<String>,
        selected_ids: BTreeSet<LogicalRepositoryId>,
        target_ids: Vec<LogicalRepositoryId>,
    },
}

fn split_group_targets(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    authoritative: &AuthoritativeGroupPlanBinding,
) -> ApiResult<SplitGroupTargets> {
    // v1.3：按 issue 所属 lc_id 寻址（R9）；单仓/无 LC 回退 legacy project 级路径。
    let lc_id = crate::product::logical_codebase::resolve_issue_logical_codebase_id(
        app_paths, project_id, issue_id,
    )
    .map_err(product_store_api_error)?;
    match RepositoryRouting::load_for_issue(app_paths, project_id, issue_id)
        .map_err(product_store_api_error)?
    {
        RepositoryRouting::Legacy { .. } => Ok(SplitGroupTargets::Legacy),
        RepositoryRouting::Logical {
            manifest,
            selection,
        } => {
            if let Some(reason) = authoritative
                .units
                .iter()
                .find_map(|unit| unit.source_draft_error.as_deref())
            {
                return Err(routing_api_error(
                    RepositoryRoutingErrorCode::Inconsistent,
                    reason,
                ));
            }
            let selected_ids = validate_logical_group_selection(
                app_paths,
                lc_id.as_deref(),
                project_id,
                &manifest,
                &selection,
            )?;
            let target_ids: BTreeSet<LogicalRepositoryId> = authoritative
                .units
                .iter()
                .filter_map(|unit| unit.target_repository_id)
                .collect();
            let target_ids = if target_ids.is_empty() {
                // 0-target focus 唯一回落语义原样保留（「单 target 路径」的一部分，
                // 与单值面一致）；focus 不唯一 → TargetMissing fail-closed。
                let [focus_repository_id] = selection.focus_repository_ids.as_slice() else {
                    return Err(routing_api_error(
                        RepositoryRoutingErrorCode::TargetMissing,
                        "group has no unique target repository and selection focus is not unique",
                    ));
                };
                vec![*focus_repository_id]
            } else {
                target_ids.into_iter().collect()
            };
            Ok(SplitGroupTargets::Logical {
                lc_id,
                selected_ids,
                target_ids,
            })
        }
        RepositoryRouting::FailClosed { code, reason } => Err(routing_api_error(code, &reason)),
    }
}

/// REQ-COD-04（WP1 分流化）：mixed-target group 的多值快照解析面。与
/// `group_target_snapshot` 共享前置与单值语义（含 0-target focus 唯一回落、
/// TargetUnknown/Inconsistent fail-closed）；差异仅在 `target_ids.len() >= 2`
/// 不再拒绝——按 unit target 逐仓产出冻结快照。单 target 退化为单条目（语义与
/// 单值面零变化）。Legacy 路由 → `None`（无逻辑仓可分流，走单值面既有语义）。
///
/// 注：WP2 分流创建循环落在 engine 侧（`workspace_engine::advance::
/// initialize_advance_split`，product 层同语义实现：`units_by_target` 分桶 +
/// `build_attempt_target_snapshot` 逐仓）；本函数保持为 web 创建面的多值解析
/// 库存（API 面后续拆分采用时接线，测试族已钉死语义）。
#[allow(dead_code)] // web 创建面尚未消费（engine 侧分流循环用 product 层实现）。
fn group_target_snapshots(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    authoritative: &AuthoritativeGroupPlanBinding,
) -> ApiResult<Option<BTreeMap<LogicalRepositoryId, AttemptTargetSnapshot>>> {
    let SplitGroupTargets::Logical {
        lc_id,
        selected_ids,
        target_ids,
    } = split_group_targets(app_paths, project_id, issue_id, authoritative)?
    else {
        return Ok(None);
    };
    let mut snapshots = BTreeMap::new();
    for logical_repository_id in target_ids {
        if !selected_ids.contains(&logical_repository_id) {
            return Err(routing_api_error(
                RepositoryRoutingErrorCode::TargetUnknown,
                "group target repository is not in the effective selection",
            ));
        }
        let snapshot = build_attempt_target_snapshot(
            app_paths,
            project_id,
            logical_repository_id,
            lc_id.as_deref(),
        )
        .map_err(target_snapshot_api_error)?;
        snapshots.insert(logical_repository_id, snapshot);
    }
    Ok(Some(snapshots))
}

/// REQ-COD-04（WP1 分流化）：`resolve_group_repository` 的多值同构变体——按
/// unit target 逐仓解析 RepositoryRecord；前置/错误码语义与
/// `group_target_snapshots` 一致。Legacy 路由 → `None`。同上注：engine 侧分流
/// 循环用 product 层实现，本函数为 web 创建面多值解析库存。
#[allow(dead_code)] // web 创建面尚未消费（engine 侧分流循环用 product 层实现）。
fn resolve_group_repositories(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    authoritative: &AuthoritativeGroupPlanBinding,
) -> ApiResult<Option<BTreeMap<LogicalRepositoryId, RepositoryRecord>>> {
    let SplitGroupTargets::Logical {
        lc_id,
        selected_ids,
        target_ids,
    } = split_group_targets(app_paths, project_id, issue_id, authoritative)?
    else {
        return Ok(None);
    };
    let store = match lc_id.as_deref() {
        Some(_) => RepositoryStore::new(app_paths.clone()),
        None => {
            let project = ProjectStore::new(app_paths.clone())
                .get(project_id)
                .map_err(product_store_api_error)?;
            RepositoryStore::for_project(app_paths.clone(), &project)
        }
    };
    let mut repositories = BTreeMap::new();
    for logical_repository_id in target_ids {
        if !selected_ids.contains(&logical_repository_id) {
            return Err(routing_api_error(
                RepositoryRoutingErrorCode::TargetUnknown,
                "group target repository is not in the effective selection",
            ));
        }
        let repository = store
            .resolve_logical_repository_for_issue_codebase(
                project_id,
                lc_id.as_deref(),
                logical_repository_id,
            )
            .map(|(_, _, repository)| repository)
            .map_err(product_store_api_error)?;
        repositories.insert(logical_repository_id, repository);
    }
    Ok(Some(repositories))
}

fn resolve_legacy_group_repository(
    app_paths: &ProductAppPaths,
    project_id: &str,
    physical_repository_id: &str,
) -> ApiResult<RepositoryRecord> {
    let project = ProjectStore::new(app_paths.clone())
        .get(project_id)
        .map_err(product_store_api_error)?;
    let store = RepositoryStore::for_project(app_paths.clone(), &project);
    match store.resolve_legacy_physical_repository_if_dual(project_id, physical_repository_id) {
        Ok((_, _, repository)) => return Ok(repository),
        Err(ProductStoreError::NotFound {
            kind: "logical_repository",
            ..
        }) => {}
        Err(error) => return Err(product_store_api_error(error)),
    }
    store
        .list(project_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .find(|repository| repository.id == physical_repository_id)
        .ok_or_else(|| {
            product_store_api_error(ProductStoreError::NotFound {
                kind: "repository",
                id: physical_repository_id.to_string(),
            })
        })
}

fn validate_logical_group_selection(
    app_paths: &ProductAppPaths,
    lc_id: Option<&str>,
    project_id: &str,
    manifest: &crate::product::logical_codebase::LogicalCodebaseManifest,
    selection: &crate::product::logical_codebase::IssueCodebaseSelection,
) -> ApiResult<BTreeSet<LogicalRepositoryId>> {
    if selection.invalidation.is_some() {
        return Err(routing_api_error(
            RepositoryRoutingErrorCode::SelectionInvalidated,
            "issue codebase selection has been invalidated",
        ));
    }
    let authority = match lc_id {
        Some(lc_id) => {
            crate::product::logical_codebase::LogicalCodebaseStore::for_lc(app_paths.clone(), lc_id)
        }
        None => crate::product::logical_codebase::LogicalCodebaseStore::new(app_paths.clone()),
    };
    let active_members: BTreeSet<LogicalRepositoryId> = authority
        .list_members(project_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .filter(|member| member.status == crate::product::logical_codebase::MemberStatus::Active)
        .map(|member| member.logical_repository_id)
        .collect();
    if manifest
        .member_ids
        .iter()
        .any(|id| !active_members.contains(id))
    {
        return Err(routing_api_error(
            RepositoryRoutingErrorCode::MemberRemoved,
            "logical codebase manifest references a missing or inactive member",
        ));
    }
    match selection.selection_policy {
        SelectionPolicy::AllMembers => {
            if selection
                .focus_repository_ids
                .iter()
                .any(|id| !manifest.member_ids.contains(id))
            {
                return Err(routing_api_error(
                    RepositoryRoutingErrorCode::Inconsistent,
                    "issue codebase selection focus is outside the manifest",
                ));
            }
            Ok(manifest.member_ids.iter().copied().collect())
        }
        SelectionPolicy::Explicit => {
            selection.validate_focus_subset().map_err(|error| {
                routing_api_error(
                    RepositoryRoutingErrorCode::Inconsistent,
                    &format!("invalid issue codebase selection: {error}"),
                )
            })?;
            let selected_ids: BTreeSet<LogicalRepositoryId> =
                selection.resolve_effective_members().into_iter().collect();
            if selected_ids
                .iter()
                .any(|id| !manifest.member_ids.contains(id))
            {
                return Err(routing_api_error(
                    RepositoryRoutingErrorCode::Inconsistent,
                    "issue codebase selection references a member absent from the manifest",
                ));
            }
            Ok(selected_ids)
        }
    }
}

fn issue_worktree_active_api_error(error: ProductStoreError) -> ApiError {
    match error {
        ProductStoreError::Io(message) if message.contains("issue_worktree_active") => {
            ApiError::runtime(
                "issue_worktree_active",
                "another work item is already active on the issue shared worktree",
                json!({}),
            )
        }
        other => product_store_api_error(other),
    }
}

fn group_initialization_api_error(error: ProductStoreError) -> ApiError {
    match error {
        ProductStoreError::Io(message) if message.starts_with("active_coding_attempt_exists:") => {
            ApiError::runtime(
                "issue_worktree_active",
                "another work item is already active on the issue shared worktree",
                json!({}),
            )
        }
        other => coding_group_attempt_incomplete_api_error(other),
    }
}

fn maybe_interrupt_group_initialization(
    state: &WebAppState,
    checkpoint: crate::web::test_controls::GroupAttemptInitializationCheckpoint,
) -> ApiResult<()> {
    if !state
        .test_controls
        .consume_group_attempt_initialization_failure(checkpoint)
    {
        return Ok(());
    }
    Err(ApiError::runtime(
        "coding_group_initialization_interrupted",
        "group coding attempt initialization interrupted",
        json!({ "checkpoint": format!("{checkpoint:?}") }),
    ))
}

fn coding_plan_revision_binding_api_error(error: ProductStoreError) -> ApiError {
    if is_group_business_validation_error(&error) {
        ApiError::validation(
            "coding_plan_revision_binding_missing",
            "group coding requires complete authoritative plan revision bindings",
        )
    } else {
        product_store_api_error(error)
    }
}

fn coding_group_attempt_incomplete_api_error(error: ProductStoreError) -> ApiError {
    if is_group_business_validation_error(&error) {
        ApiError::validation(
            "coding_group_attempt_incomplete",
            "existing group coding attempt is only partially initialized or inconsistent",
        )
    } else {
        product_store_api_error(error)
    }
}
#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::process::Command;

    use super::*;
    use crate::product::coding_attempt_store::AuthoritativeCodingUnitBinding;
    use crate::product::issue_store::CreateProductIssueInput;
    use crate::product::logical_codebase::{
        AggregatePolicyArtifactStore, IssueCodebaseSelection, IssueCodebaseSelectionStore,
        LogicalCodebaseFeature, LogicalCodebaseStore,
    };
    use crate::product::project_store::CreateProjectInput;
    use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};

    struct SplitResolutionFixture {
        _root: tempfile::TempDir,
        paths: ProductAppPaths,
        project_id: String,
        issue_id: String,
        targets: Vec<LogicalRepositoryId>,
    }

    impl SplitResolutionFixture {
        /// 重写 issue selection（focus 形态由用例自定）。
        fn save_selection(
            &self,
            included: Vec<LogicalRepositoryId>,
            focus: Vec<LogicalRepositoryId>,
        ) {
            IssueCodebaseSelectionStore::new(self.paths.clone())
                .save(&IssueCodebaseSelection::explicit(
                    &self.project_id,
                    &self.issue_id,
                    included,
                    Vec::new(),
                    focus,
                    None,
                ))
                .unwrap();
        }
    }

    fn split_resolution_fixture() -> SplitResolutionFixture {
        let root = tempfile::tempdir().expect("temporary product root");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let project = ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "split resolution".to_string(),
                description: None,
            })
            .unwrap();
        let mut targets = Vec::new();
        for name in ["api", "web"] {
            let canonical_path = root.path().join(name);
            fs::create_dir_all(&canonical_path).unwrap();
            run_git(&canonical_path, &["init", "--quiet"]);
            run_git(
                &canonical_path,
                &["config", "user.email", "split@example.test"],
            );
            run_git(
                &canonical_path,
                &["config", "user.name", "Split Resolution"],
            );
            fs::write(canonical_path.join("README.md"), format!("# {name}\n")).unwrap();
            run_git(&canonical_path, &["add", "README.md"]);
            run_git(
                &canonical_path,
                &["commit", "--quiet", "-m", "initial commit"],
            );
            let repository = RepositoryStore::with_logical_codebase_feature(
                paths.clone(),
                LogicalCodebaseFeature::enabled(),
            )
            .create(CreateRepositoryInput {
                project_id: project.id.clone(),
                name: name.to_string(),
                path: canonical_path,
                default_policy_preset: None,
                default_provider_mode: None,
                idempotency_key: format!("split-resolution-{name}"),
            })
            .unwrap();
            targets.push(
                repository
                    .logical_repository_id
                    .expect("logical repository ID"),
            );
        }
        let manifest = LogicalCodebaseStore::new(paths.clone())
            .load_manifest(&project.id)
            .unwrap()
            .expect("manifest");
        AggregatePolicyArtifactStore::new(paths.clone())
            .ensure_bootstrap(&manifest)
            .unwrap();
        let issue = IssueStore::new(paths.clone())
            .create(CreateProductIssueInput {
                project_id: project.id.clone(),
                repo_id: None,
                logical_codebase_id: None,
                title: "mixed-target group".to_string(),
                description: None,
                change_id: None,
                base_branch: None,
            })
            .unwrap();
        let fixture = SplitResolutionFixture {
            _root: root,
            paths,
            project_id: project.id,
            issue_id: issue.id,
            targets,
        };
        fixture.save_selection(fixture.targets.clone(), vec![fixture.targets[0]]);
        fixture
    }

    fn run_git(cwd: &std::path::Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .status()
            .expect("start git");
        assert!(status.success(), "git {} failed", args.join(" "));
    }

    fn unit_binding(target: Option<LogicalRepositoryId>) -> AuthoritativeCodingUnitBinding {
        AuthoritativeCodingUnitBinding {
            logical_work_item_id: "work_item_0001".to_string(),
            work_item_revision_id: "revision_0001".to_string(),
            verification_plan_revision_id: "verification_0001".to_string(),
            projection_bundle_id: "projection_0001".to_string(),
            target_repository_id: target,
            source_draft_error: None,
            dependency_logical_work_item_ids: Vec::new(),
        }
    }

    fn authoritative(units: Vec<AuthoritativeCodingUnitBinding>) -> AuthoritativeGroupPlanBinding {
        AuthoritativeGroupPlanBinding {
            plan_revision_id: "plan_revision_0001".to_string(),
            dependency_graph_revision_id: "dependency_graph_0001".to_string(),
            plan_projection_bundle_id: "plan_projection_0001".to_string(),
            units,
        }
    }

    #[test]
    fn group_target_snapshots_resolves_mixed_targets_per_target() {
        // REQ-COD-04（WP1 分流化）：mixed-target 不再构成拒绝理由——按 unit target
        // 分组逐仓产出冻结快照（REQ-MTG-01 scenario「尝试 mixed-target group」解析面）。
        let fixture = split_resolution_fixture();
        let [api, web] = fixture.targets.as_slice() else {
            panic!("fixture must register two targets");
        };
        let binding = authoritative(vec![unit_binding(Some(*api)), unit_binding(Some(*web))]);

        let snapshots: BTreeMap<LogicalRepositoryId, AttemptTargetSnapshot> =
            group_target_snapshots(
                &fixture.paths,
                &fixture.project_id,
                &fixture.issue_id,
                &binding,
            )
            .expect("mixed-target split resolution must succeed")
            .expect("logical routing must yield a target map");

        assert_eq!(snapshots.len(), 2, "one snapshot per target");
        assert_eq!(snapshots[api].logical_repository_id, *api);
        assert_eq!(snapshots[web].logical_repository_id, *web);
        assert!(snapshots[api].revision.is_some(), "git HEAD captured");
        assert!(snapshots[web].revision.is_some(), "git HEAD captured");
    }

    #[test]
    fn resolve_group_repositories_resolves_mixed_targets_per_target() {
        let fixture = split_resolution_fixture();
        let [api, web] = fixture.targets.as_slice() else {
            panic!("fixture must register two targets");
        };
        let binding = authoritative(vec![unit_binding(Some(*api)), unit_binding(Some(*web))]);

        let repositories = resolve_group_repositories(
            &fixture.paths,
            &fixture.project_id,
            &fixture.issue_id,
            &binding,
        )
        .expect("mixed-target split resolution must succeed")
        .expect("logical routing must yield a repository map");

        assert_eq!(repositories.len(), 2, "one repository per target");
        assert_eq!(repositories[api].logical_repository_id, Some(*api));
        assert_eq!(repositories[web].logical_repository_id, Some(*web));
    }

    #[test]
    fn group_target_snapshots_single_target_keeps_single_entry() {
        // 单 target 语义零变化：多值面退化为单条目，与现行单值面一致。
        let fixture = split_resolution_fixture();
        let [api, _] = fixture.targets.as_slice() else {
            panic!("fixture must register two targets");
        };
        let binding = authoritative(vec![unit_binding(Some(*api)), unit_binding(Some(*api))]);

        let snapshots = group_target_snapshots(
            &fixture.paths,
            &fixture.project_id,
            &fixture.issue_id,
            &binding,
        )
        .unwrap()
        .expect("logical routing");

        assert_eq!(snapshots.len(), 1);
        assert!(snapshots.contains_key(api));
    }

    #[test]
    fn group_target_snapshots_zero_target_unique_focus_falls_back_to_focus() {
        // 0-target focus 唯一回落语义原样保留（「单 target 路径」的一部分）。
        let fixture = split_resolution_fixture();
        let [api, _] = fixture.targets.as_slice() else {
            panic!("fixture must register two targets");
        };
        let binding = authoritative(vec![unit_binding(None), unit_binding(None)]);

        let snapshots = group_target_snapshots(
            &fixture.paths,
            &fixture.project_id,
            &fixture.issue_id,
            &binding,
        )
        .unwrap()
        .expect("logical routing");

        assert_eq!(snapshots.len(), 1);
        assert!(snapshots.contains_key(api));
    }

    #[test]
    fn group_target_snapshots_zero_target_without_unique_focus_fails_closed() {
        // REQ-COD-04 scenario「无唯一 target 归属仍 fail-closed」：units 均无
        // target 且 focus 多点 → TargetMissing 稳定码，不静默选 target。
        let fixture = split_resolution_fixture();
        fixture.save_selection(fixture.targets.clone(), fixture.targets.clone());
        let binding = authoritative(vec![unit_binding(None), unit_binding(None)]);

        let error = group_target_snapshots(
            &fixture.paths,
            &fixture.project_id,
            &fixture.issue_id,
            &binding,
        )
        .unwrap_err();

        assert_eq!(error.code, "repository_routing_target_missing");
    }

    #[test]
    fn group_target_snapshots_target_outside_selection_fails_closed() {
        // 有效 selection 之外的 target 保持 TargetUnknown fail-closed。
        let fixture = split_resolution_fixture();
        let [api, web] = fixture.targets.as_slice() else {
            panic!("fixture must register two targets");
        };
        fixture.save_selection(vec![*api], vec![*api]);
        let binding = authoritative(vec![unit_binding(Some(*api)), unit_binding(Some(*web))]);

        let error = group_target_snapshots(
            &fixture.paths,
            &fixture.project_id,
            &fixture.issue_id,
            &binding,
        )
        .unwrap_err();

        assert_eq!(error.code, "repository_routing_target_unknown");
    }

    #[test]
    fn group_target_snapshots_source_draft_error_stays_inconsistent() {
        // source_draft_error fail-closed 前置保留（溯源断链不进分流解析）。
        let fixture = split_resolution_fixture();
        let [api, web] = fixture.targets.as_slice() else {
            panic!("fixture must register two targets");
        };
        let mut broken = unit_binding(Some(*api));
        broken.source_draft_error = Some("source draft missing".to_string());
        let binding = authoritative(vec![broken, unit_binding(Some(*web))]);

        let error = group_target_snapshots(
            &fixture.paths,
            &fixture.project_id,
            &fixture.issue_id,
            &binding,
        )
        .unwrap_err();

        assert_eq!(error.code, "repository_routing_inconsistent");
    }
}
