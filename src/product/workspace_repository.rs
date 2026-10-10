use std::collections::BTreeSet;

use crate::product::app_paths::ProductAppPaths;
use crate::product::issue_store::IssueStore;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::logical_codebase::{
    LogicalRepositoryId, RepositoryRouting, RepositoryRoutingErrorCode, SelectionPolicy,
};
use crate::product::models::{RepositoryRecord, WorkspaceSessionRecord, WorkspaceType};
use crate::product::project_store::ProjectStore;
use crate::product::repository_store::RepositoryStore;
use crate::product::work_item_runtime_reader::WorkItemRuntimeReader;
use crate::product::workspace_engine::draft_batch::compile_support::{
    load_involved_from_confirmed_design, resolve_logical_work_item_plan_repository_targets,
};

pub fn workspace_repository_for_session(
    app_paths: &ProductAppPaths,
    lifecycle: &LifecycleStore,
    session: &WorkspaceSessionRecord,
) -> Result<RepositoryRecord, ProductStoreError> {
    workspace_repository(app_paths, lifecycle, session)
}

fn workspace_repository(
    app_paths: &ProductAppPaths,
    lifecycle: &LifecycleStore,
    session: &WorkspaceSessionRecord,
) -> Result<RepositoryRecord, ProductStoreError> {
    let routing =
        RepositoryRouting::load_for_issue(app_paths, &session.project_id, &session.issue_id)?;
    match session.workspace_type {
        WorkspaceType::Story => {
            let story = lifecycle
                .list_story_specs(&session.project_id, &session.issue_id)?
                .into_iter()
                .find(|story| story.id == session.entity_id)
                .ok_or_else(|| ProductStoreError::NotFound {
                    kind: "story_spec",
                    id: session.entity_id.clone(),
                })?;
            match routing {
                RepositoryRouting::Legacy { .. } => resolve_legacy_physical_repository(
                    app_paths,
                    &session.project_id,
                    &story.repository_id,
                ),
                RepositoryRouting::Logical {
                    manifest,
                    selection,
                } => {
                    match story.focus_repository_id {
                        Some(logical_id) => resolve_selected_logical_repository(
                            app_paths,
                            &session.project_id,
                            &session.issue_id,
                            logical_id,
                            &manifest,
                            &selection,
                        ),
                        // 方案X草稿态（缺陷 #2 修法 A，controller 裁决 2026-10-02）：
                        // focus=None ∧ involved 空 = AI 自决 involved 之前，目标即
                        // LC 聚合根本身（ENV-10/11 cwd≠target 合法形态；author_root_launch
                        // 的 PolicyTarget::aggregate_root 锚同口径）。回写 focus 后
                        // 恢复成员解析；involved 非空仍 fail-closed（下方 None 臂）。
                        None if story.involved_repository_ids.is_empty() => {
                            Ok(aggregate_root_view(&manifest))
                        }
                        None => Err(routing_error(
                            RepositoryRoutingErrorCode::TargetMissing,
                            format!("story {} has no focus repository", story.id),
                        )),
                    }
                }
                RepositoryRouting::FailClosed { code, reason } => Err(routing_error(code, reason)),
            }
        }
        WorkspaceType::Design => {
            let design = lifecycle
                .list_design_specs(&session.project_id, &session.issue_id)?
                .into_iter()
                .find(|design| design.id == session.entity_id)
                .ok_or_else(|| ProductStoreError::NotFound {
                    kind: "design_spec",
                    id: session.entity_id.clone(),
                })?;
            match routing {
                RepositoryRouting::Legacy { .. } => resolve_issue_repository(app_paths, session),
                RepositoryRouting::Logical {
                    manifest,
                    selection,
                } => {
                    let target_ids = unique_ids(design.involved_repository_ids);
                    // 方案X草稿态（缺陷 #5，同缺陷 #2 修法 A 口径）：involved 空 =
                    // AI 自决之前，目标锚定 LC 聚合根视图；回写 involved 后恢复
                    // 唯一成员解析。
                    if target_ids.is_empty() {
                        return Ok(aggregate_root_view(&manifest));
                    }
                    // r62(codex-6 design resume 现场):确认面已合法化多仓 Design
                    // (REQ-PLN-05 决策 3b:involved>1 + change_order 通过确认
                    // gate 落盘),路由面若对同一数据 TargetAmbiguous,则确认成功
                    // 后修订轮永远 fail-closed(现场 design_spec_0001 involved=
                    // [alpha,beta] 修订即拒)。≥2 involved 且 change_order 非空
                    // 时按首成员确定性锚定(change_order 是该形态的确定性实施
                    // 顺序,首仓=author 修订目标;与 plan 会话 selection.focus
                    // 回落先例 REQ-COD-04 对称);change_order 空(未回写完成/
                    // 中间态)保持 TargetAmbiguous fail-closed 不放宽。
                    let logical_id = if target_ids.len() >= 2 {
                        match design.change_order.first() {
                            Some(first) if target_ids.contains(first) => *first,
                            _ => unique_target(target_ids, &design.id)?,
                        }
                    } else {
                        unique_target(target_ids, &design.id)?
                    };
                    resolve_selected_logical_repository(
                        app_paths,
                        &session.project_id,
                        &session.issue_id,
                        logical_id,
                        &manifest,
                        &selection,
                    )
                }
                RepositoryRouting::FailClosed { code, reason } => Err(routing_error(code, reason)),
            }
        }
        WorkspaceType::WorkItemPlan => {
            let plan = lifecycle.get_issue_work_item_plan(
                &session.project_id,
                &session.issue_id,
                &session.entity_id,
            )?;
            match routing {
                RepositoryRouting::Legacy { .. } => resolve_issue_repository(app_paths, session),
                RepositoryRouting::Logical {
                    manifest,
                    selection,
                } => {
                    let targets =
                        resolve_logical_work_item_plan_repository_targets(lifecycle, &plan)
                            .map_err(|reason| routing_error_for_target_error(&reason))?;
                    let mut target_ids = targets
                        .unwrap_or_default()
                        .keys()
                        .copied()
                        .collect::<BTreeSet<LogicalRepositoryId>>();
                    // 缺陷 #7（同族）：聚合 plan 会话 target 以源 Design involved 集
                    // 过滤（LC selection 恒 all_members，不过滤恒 Ambiguous）；无聚合
                    // 视野（involved 空）保持原 target 集不变。
                    let design_involved = load_involved_from_confirmed_design(lifecycle, &plan)
                        .map_err(|reason| routing_error_for_target_error(&reason))?;
                    if !design_involved.is_empty() {
                        let involved: std::collections::BTreeSet<LogicalRepositoryId> =
                            design_involved.into_iter().collect();
                        target_ids.retain(|id| involved.contains(id));
                    }
                    match plan_session_repository_target(target_ids, &plan.id, &selection)? {
                        PlanSessionRepositoryTarget::Member(logical_id) => {
                            resolve_selected_logical_repository(
                                app_paths,
                                &session.project_id,
                                &session.issue_id,
                                logical_id,
                                &manifest,
                                &selection,
                            )
                        }
                        // S6 方案 B：plan 会话=orchestrator 角色，多 target 且
                        // focus 非唯一时锚定聚合根视图（同 story 草稿态口径；
                        // 视图仅供 attach/launch 锚定，不落盘、不参与写根授权）。
                        PlanSessionRepositoryTarget::AggregateRoot => {
                            Ok(aggregate_root_view(&manifest))
                        }
                    }
                }
                RepositoryRouting::FailClosed { code, reason } => Err(routing_error(code, reason)),
            }
        }
        WorkspaceType::WorkItem => {
            WorkItemRuntimeReader::new(app_paths.clone()).resolve_workspace(session)?;
            match routing {
                RepositoryRouting::Legacy { .. } => {
                    let physical_repository_id = lifecycle
                        .list_work_items(&session.project_id, &session.issue_id)?
                        .into_iter()
                        .find(|work_item| work_item.id == session.entity_id)
                        .map(|work_item| work_item.repository_id)
                        .or_else(|| {
                            IssueStore::new(app_paths.clone())
                                .get(&session.project_id, &session.issue_id)
                                .ok()
                                .and_then(|issue| issue.repo_id)
                        })
                        .ok_or_else(|| ProductStoreError::NotFound {
                            kind: "work_item",
                            id: session.entity_id.clone(),
                        })?;
                    resolve_legacy_physical_repository(
                        app_paths,
                        &session.project_id,
                        &physical_repository_id,
                    )
                }
                RepositoryRouting::Logical {
                    manifest,
                    selection,
                } => {
                    let work_item = lifecycle
                        .list_work_items(&session.project_id, &session.issue_id)?
                        .into_iter()
                        .find(|work_item| work_item.id == session.entity_id)
                        .ok_or_else(|| ProductStoreError::NotFound {
                            kind: "work_item",
                            id: session.entity_id.clone(),
                        })?;
                    let logical_id = work_item.target_repository_id.ok_or_else(|| {
                        routing_error(
                            RepositoryRoutingErrorCode::TargetMissing,
                            format!("work item {} has no target repository", work_item.id),
                        )
                    })?;
                    resolve_selected_logical_repository(
                        app_paths,
                        &session.project_id,
                        &session.issue_id,
                        logical_id,
                        &manifest,
                        &selection,
                    )
                }
                RepositoryRouting::FailClosed { code, reason } => Err(routing_error(code, reason)),
            }
        }
    }
}

fn resolve_selected_logical_repository(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    logical_id: LogicalRepositoryId,
    manifest: &crate::product::logical_codebase::LogicalCodebaseManifest,
    selection: &crate::product::logical_codebase::IssueCodebaseSelection,
) -> Result<RepositoryRecord, ProductStoreError> {
    if selection.invalidation.is_some() {
        return Err(routing_error(
            RepositoryRoutingErrorCode::SelectionInvalidated,
            "issue codebase selection has been invalidated",
        ));
    }
    selection.validate_focus_subset().map_err(|error| {
        routing_error(
            RepositoryRoutingErrorCode::Inconsistent,
            format!("invalid issue codebase selection: {error}"),
        )
    })?;
    let selected_ids: BTreeSet<LogicalRepositoryId> = match selection.selection_policy {
        SelectionPolicy::AllMembers => manifest.member_ids.iter().copied().collect(),
        SelectionPolicy::Explicit => selection.resolve_effective_members().into_iter().collect(),
    };
    if !selected_ids.contains(&logical_id) {
        return Err(routing_error(
            RepositoryRoutingErrorCode::TargetUnknown,
            format!("logical repository target {logical_id:?} is not in the effective selection"),
        ));
    }
    // 缺陷 #6（2026-10-02 E2E）：per-LC（v1.3 布局）成员/checkouts 位于
    // logical-codebases/{lc}/ 子树且不写 legacy repos.json 投影，直接 strict
    // 解析恒 IdentityMismatch（design 会话回写 involved 后 WS 重连即撞）。
    // 按 issue 持久化 lc_id 走 for_lc 权威解析（合成投影），legacy LC 语义
    // 由 for_issue_codebase 内部分流保持不变。
    let lc_id = crate::product::logical_codebase::resolve_issue_logical_codebase_id(
        app_paths, project_id, issue_id,
    )?;
    let project = ProjectStore::new(app_paths.clone()).get(project_id)?;
    RepositoryStore::for_project(app_paths.clone(), &project)
        .resolve_logical_repository_for_issue_codebase(project_id, lc_id.as_deref(), logical_id)
        .map(|(_, _, repository)| repository)
}

fn resolve_legacy_physical_repository(
    app_paths: &ProductAppPaths,
    project_id: &str,
    physical_repository_id: &str,
) -> Result<RepositoryRecord, ProductStoreError> {
    let project = ProjectStore::new(app_paths.clone()).get(project_id)?;
    let store = RepositoryStore::for_project(app_paths.clone(), &project);
    if let Ok((_, _, repository)) =
        store.resolve_legacy_physical_repository_if_dual(project_id, physical_repository_id)
    {
        return Ok(repository);
    }
    store
        .list(project_id)?
        .into_iter()
        .find(|repository| repository.id == physical_repository_id)
        .ok_or_else(|| ProductStoreError::NotFound {
            kind: "repository",
            id: physical_repository_id.to_string(),
        })
}

/// 草稿 Story（Logical 路由、focus 未定、involved 空）的聚合根锚视图：
/// cwd 仍由唯一 authority resolver 冻结为 canonical root；target 锚定
/// `PolicyTarget::aggregate_root(provider_context_root)`（primary_checkout_id
/// =None → author_root_launch 走 aggregate_root 臂）。视图仅供 attach/launch
/// 目标锚定，不落盘、不参与写根授权（PlanningReadOnly 空 writable_roots
/// 不变）；logical_repository_id 取 manifest 逻辑身份（确定性，仅作 LC
/// 会话 gateway 注入谓词之用，不进入 PolicyTarget/成员解析）。
fn aggregate_root_view(
    manifest: &crate::product::logical_codebase::LogicalCodebaseManifest,
) -> RepositoryRecord {
    let root = manifest.provider_context_root.clone();
    RepositoryRecord {
        id: format!("lc_aggregate_root_view_{}", manifest.logical_codebase_id),
        project_id: manifest.project_id.clone(),
        name: "lc-aggregate-root".to_string(),
        path: root.clone(),
        repo_hash: String::new(),
        runtime_root: root,
        default_policy_preset: String::new(),
        default_provider_mode: String::new(),
        created_at: String::new(),
        logical_repository_id: Some(LogicalRepositoryId(manifest.logical_codebase_id)),
        primary_checkout_id: None,
        identity_schema_version: 0,
        updated_at: String::new(),
    }
}

fn resolve_issue_repository(
    app_paths: &ProductAppPaths,
    session: &WorkspaceSessionRecord,
) -> Result<RepositoryRecord, ProductStoreError> {
    let physical_repository_id = IssueStore::new(app_paths.clone())
        .get(&session.project_id, &session.issue_id)?
        .repo_id
        .ok_or_else(|| ProductStoreError::NotFound {
            kind: "repository",
            id: format!("issue:{}:repo_id", session.issue_id),
        })?;
    resolve_legacy_physical_repository(app_paths, &session.project_id, &physical_repository_id)
}

fn unique_ids(ids: Vec<LogicalRepositoryId>) -> BTreeSet<LogicalRepositoryId> {
    ids.into_iter().collect()
}

fn unique_target(
    target_ids: BTreeSet<LogicalRepositoryId>,
    entity_id: &str,
) -> Result<LogicalRepositoryId, ProductStoreError> {
    match target_ids.len() {
        0 => Err(routing_error(
            RepositoryRoutingErrorCode::TargetMissing,
            format!("{entity_id} has no unique logical repository target"),
        )),
        1 => Ok(*target_ids.first().expect("one target exists")),
        _ => Err(routing_error(
            RepositoryRoutingErrorCode::TargetAmbiguous,
            format!("{entity_id} has multiple logical repository targets"),
        )),
    }
}

/// plan 会话路由面的 target 解析结果（S6 方案 B，controller 2026-10-10）。
///
/// - `Member`：锚定单一逻辑成员（唯一 target，或多 target + 唯一 focus 回落）；
/// - `AggregateRoot`：锚定 LC 聚合根视图——plan 会话=orchestrator 角色
///   （durable system 消息 adapter_role=orchestrator），多 target 且 focus
///   非唯一时不再 fail-closed。
#[derive(Debug, PartialEq, Eq)]
enum PlanSessionRepositoryTarget {
    Member(LogicalRepositoryId),
    AggregateRoot,
}

/// REQ-COD-04（WP1 分流化）+ S6 方案 B：plan 会话路由面的 target 解析。
///
/// - 0 target → TargetMissing fail-closed（保持现行，即使 focus 唯一也不回落）；
/// - 1 target → 唯一 target（现行语义零变化）；
/// - ≥2 target + 唯一 focus → 该 focus 成员（REQ-COD-04 回落语义保留，
///   与创建面 0-target focus 语义对称）；
/// - ≥2 target + focus 非唯一（0 或 ≥2）→ 聚合根视图（S6 方案 B：plan 会话
///   本就是 orchestrator 角色，多仓纵切下 selection 恒 all_members/全 focus，
///   单一 focus 逃生门不足以解除 TargetAmbiguous——与 story 草稿态
///   aggregate_root_view 同款、design 臂 change_order 首仓兜底对称参照。
///   S6 现场：四仓 explicit included=4/focus=4，WS create 在本函数
///   TargetAmbiguous，前端停留「正在连接工作区…」重连循环）。
fn plan_session_repository_target(
    target_ids: BTreeSet<LogicalRepositoryId>,
    entity_id: &str,
    selection: &crate::product::logical_codebase::IssueCodebaseSelection,
) -> Result<PlanSessionRepositoryTarget, ProductStoreError> {
    if target_ids.len() >= 2 {
        if let [focus_repository_id] = selection.focus_repository_ids.as_slice() {
            return Ok(PlanSessionRepositoryTarget::Member(*focus_repository_id));
        }
        return Ok(PlanSessionRepositoryTarget::AggregateRoot);
    }
    Ok(PlanSessionRepositoryTarget::Member(unique_target(
        target_ids, entity_id,
    )?))
}

fn routing_error_for_target_error(reason: &str) -> ProductStoreError {
    let code = if reason.contains("target_member_removed") || reason.contains("invalid members") {
        RepositoryRoutingErrorCode::Inconsistent
    } else if reason.contains("cannot resolve target") {
        RepositoryRoutingErrorCode::TargetUnknown
    } else {
        RepositoryRoutingErrorCode::TargetMissing
    };
    routing_error(code, reason)
}

fn routing_error(code: RepositoryRoutingErrorCode, reason: impl Into<String>) -> ProductStoreError {
    let stable_code = code.stable_code();
    ProductStoreError::InvalidRecord {
        kind: "repository_routing",
        reason: format!("{stable_code}: {}", reason.into()),
    }
}

#[cfg(test)]
mod tests {
    // large_file_guard（1200 行上限）：测试体拆到同目录文件，在本模块
    // 上下文 include 展开——`use super::*` 与私有项可见性不变（同
    // coding_attempt_store/admission_tests.rs 的 include 惯例）。
    include!("workspace_repository_tests.rs");
}
