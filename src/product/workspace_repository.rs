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

/// E（fix-planning-session-anchor-recovery REQ-PLN-08）：同一 resolver 一次读取
/// manifest/selection/实体后派生的两种用途输出，不造第二 resolver
/// （REQ-PLN-07 唯一 resolver／同 snapshot 派生）。
///
/// - `native_launch_anchor`：provider native launch 锚。LC 规划会话
///   （Story/Design/WorkItemPlan，PlanningReadOnly）恒 LC 聚合根视图，产物
///   involved/focus/change_order 回写不迁移锚；Legacy/direct 与 WorkItem/Coding
///   与业务同锚（原目录/原 member）。selection 失效/focus 越界等共同守卫
///   fail-closed，native launch 不得绕过。
/// - `business_repository`：现行 `workspace_repository_for_session` 语义逐项
///   保留（业务材料/compile target：成员选仓、多 target 无唯一 focus 已回
///   root 的材料行为、无 target/未知成员错误均不变）。
///
/// 共享读取（routing/实体/runtime binding）失败对外为 `Err`（无共同基线即无
/// 上下文，两 getter 同错）；仅单用途派生的失败落在对应字段。聚合视图 ID 是
/// 合成身份（lc_aggregate_root_view_*），不得流入业务选仓/WorkItemPlan IR
/// target（design §3.2）。
#[derive(Debug)]
pub struct WorkspaceRepositoryContext {
    pub native_launch_anchor: Result<RepositoryRecord, ProductStoreError>,
    pub business_repository: Result<RepositoryRecord, ProductStoreError>,
}

pub fn workspace_repository_for_session(
    app_paths: &ProductAppPaths,
    lifecycle: &LifecycleStore,
    session: &WorkspaceSessionRecord,
) -> Result<RepositoryRecord, ProductStoreError> {
    resolve_workspace_repository_context(app_paths, lifecycle, session)
        .and_then(|context| context.business_repository)
}

/// E（REQ-PLN-08）：native launch anchor getter——共享 resolver 结果的字段
/// 选择，非第二 resolver；native launch caller 逐个迁移，不得以 API 名称
/// 模糊两用途。
pub fn workspace_native_launch_anchor_for_session(
    app_paths: &ProductAppPaths,
    lifecycle: &LifecycleStore,
    session: &WorkspaceSessionRecord,
) -> Result<RepositoryRecord, ProductStoreError> {
    resolve_workspace_repository_context(app_paths, lifecycle, session)
        .and_then(|context| context.native_launch_anchor)
}

pub fn resolve_workspace_repository_context(
    app_paths: &ProductAppPaths,
    lifecycle: &LifecycleStore,
    session: &WorkspaceSessionRecord,
) -> Result<WorkspaceRepositoryContext, ProductStoreError> {
    // WorkItem 运行态解析消费 record 专属字段(runtime binding)——保留
    // record 专臂;规划三型(Story/Design/WorkItemPlan)走实体四事实入口。
    if session.workspace_type == WorkspaceType::WorkItem {
        return resolve_work_item_repository_context(app_paths, lifecycle, session);
    }
    resolve_workspace_repository_context_for_entity(
        app_paths,
        lifecycle,
        &session.project_id,
        &session.issue_id,
        &session.entity_id,
        session.workspace_type.clone(),
    )
}

/// WorkItem(record 专臂:runtime binding 解析是 record 字段消费,不进
/// 实体四事实入口)。
fn resolve_work_item_repository_context(
    app_paths: &ProductAppPaths,
    lifecycle: &LifecycleStore,
    session: &WorkspaceSessionRecord,
) -> Result<WorkspaceRepositoryContext, ProductStoreError> {
    WorkItemRuntimeReader::new(app_paths.clone()).resolve_workspace(session)?;
    match RepositoryRouting::load_for_issue(app_paths, &session.project_id, &session.issue_id)? {
        RepositoryRouting::Legacy { .. } => {
            let repository = resolve_issue_repository_record(app_paths, session)?;
            Ok(same_anchor_context(repository))
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
            let repository = resolve_selected_logical_repository(
                app_paths,
                &session.project_id,
                &session.issue_id,
                logical_id,
                &manifest,
                &selection,
            )?;
            Ok(same_anchor_context(repository))
        }
        RepositoryRouting::FailClosed { code, reason } => Err(routing_error(code, reason)),
    }
}

fn resolve_issue_repository_record(
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

/// 批2 Task 20b 修复轮 1(F1):实体四事实(engine `WorkspaceSession` 与
/// durable record 共用同一 resolver——engine 侧规划审查窗口目标解析
/// 消费,不复制选仓规则;字段名与 record 对齐,body 零改动)。
struct EntityFacts<'a> {
    project_id: &'a str,
    issue_id: &'a str,
    entity_id: &'a str,
    workspace_type: WorkspaceType,
}

pub fn resolve_workspace_repository_context_for_entity(
    app_paths: &ProductAppPaths,
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    entity_id: &str,
    workspace_type: WorkspaceType,
) -> Result<WorkspaceRepositoryContext, ProductStoreError> {
    let routing = RepositoryRouting::load_for_issue(app_paths, project_id, issue_id)?;
    let session = EntityFacts {
        project_id,
        issue_id,
        entity_id,
        workspace_type,
    };
    match session.workspace_type {
        WorkspaceType::Story => {
            let story = lifecycle
                .list_story_specs(session.project_id, session.issue_id)?
                .into_iter()
                .find(|story| story.id == session.entity_id)
                .ok_or_else(|| ProductStoreError::NotFound {
                    kind: "story_spec",
                    id: session.entity_id.to_string(),
                })?;
            match routing {
                RepositoryRouting::Legacy { .. } => {
                    let repository = resolve_legacy_physical_repository(
                        app_paths,
                        session.project_id,
                        &story.repository_id,
                    )?;
                    Ok(same_anchor_context(repository))
                }
                RepositoryRouting::Logical {
                    manifest,
                    selection,
                } => Ok(WorkspaceRepositoryContext {
                    // E（REQ-PLN-08）：native anchor 与业务 target 同 resolver
                    // 分离派生——native 恒聚合根，业务按现行规则逐项保留。
                    native_launch_anchor: planning_native_launch_anchor(&manifest, &selection),
                    business_repository: match story.focus_repository_id {
                        Some(logical_id) => resolve_selected_logical_repository(
                            app_paths,
                            session.project_id,
                            session.issue_id,
                            logical_id,
                            &manifest,
                            &selection,
                        ),
                        // 方案X草稿态（缺陷 #2 修法 A,controller 2026-10-02）+
                        // story 焦点仓路由修复（2026-10-11,story_spec_0001 现场）:
                        // focus=None 时锚定 LC 聚合根本身（ENV-10/11 cwd≠target 合法
                        // 形态;author_root_launch 的 PolicyTarget::aggregate_root 锚
                        // 同口径）。原「involved 非空仍 fail-closed」臂是缺陷 #2 时
                        // 对「回写异常半态」的防放宽锁——但 structured output schema
                        // 允许 AI 输出 focus=null（多仓均衡纵切无单一焦点）,该态是
                        // AI 自决流合法可达的正常态,fail-closed 造成修订轮死区
                        // （现场 involved=4/focus=None 恒 TargetMissing,不可重试）。
                        // 与 plan S6（多 target + focus 非唯一回落聚合根,5a217fa8）
                        // 同构:①锚与首轮草稿态连续 → 指纹可比对 → 原生恢复可达
                        // （r23 根修目标）;②cwd 恒 canonical root,author 视野零变化
                        // ;③design change_order 首仓（r62）家族先例同款。AI 显式
                        // 回写 focus 后走上方成员锚臂（r23 语义零变化,锚迁移
                        // supersede 语义正确）。
                        None => Ok(aggregate_root_view(&manifest)),
                    },
                }),
                RepositoryRouting::FailClosed { code, reason } => Err(routing_error(code, reason)),
            }
        }
        WorkspaceType::Design => {
            let design = lifecycle
                .list_design_specs(session.project_id, session.issue_id)?
                .into_iter()
                .find(|design| design.id == session.entity_id)
                .ok_or_else(|| ProductStoreError::NotFound {
                    kind: "design_spec",
                    id: session.entity_id.to_string(),
                })?;
            match routing {
                RepositoryRouting::Legacy { .. } => {
                    let repository = resolve_issue_repository(app_paths, &session)?;
                    Ok(same_anchor_context(repository))
                }
                RepositoryRouting::Logical {
                    manifest,
                    selection,
                } => Ok(WorkspaceRepositoryContext {
                    native_launch_anchor: planning_native_launch_anchor(&manifest, &selection),
                    business_repository: resolve_logical_design_repository(
                        app_paths,
                        session.project_id,
                        session.issue_id,
                        &design,
                        &manifest,
                        &selection,
                    ),
                }),
                RepositoryRouting::FailClosed { code, reason } => Err(routing_error(code, reason)),
            }
        }
        WorkspaceType::WorkItemPlan => {
            let plan = lifecycle.get_issue_work_item_plan(
                session.project_id,
                session.issue_id,
                session.entity_id,
            )?;
            match routing {
                RepositoryRouting::Legacy { .. } => {
                    let repository = resolve_issue_repository(app_paths, &session)?;
                    Ok(same_anchor_context(repository))
                }
                RepositoryRouting::Logical {
                    manifest,
                    selection,
                } => Ok(WorkspaceRepositoryContext {
                    native_launch_anchor: planning_native_launch_anchor(&manifest, &selection),
                    business_repository: resolve_logical_plan_repository(
                        app_paths, lifecycle, &session, &plan, &manifest, &selection,
                    ),
                }),
                RepositoryRouting::FailClosed { code, reason } => Err(routing_error(code, reason)),
            }
        }
        WorkspaceType::WorkItem => {
            // 实体四事实入口不承载 WorkItem 运行态(runtime binding 是
            // record 字段消费,见 record 专臂);规划审查窗口不驱动该型。
            Err(ProductStoreError::Io(
                "entity_facts_entry_unsupported_for_work_item_runtime".to_string(),
            ))
        }
    }
}

/// Legacy/direct 与 WorkItem/Coding 的双用途同锚：native launch 与业务解析是
/// 同一计算（原目录/原 member/worktree），一次派生两字段共享同一结果；
/// 失败对两 getter 同错（共享派生失败，outer Err）。
fn same_anchor_context(repository: RepositoryRecord) -> WorkspaceRepositoryContext {
    WorkspaceRepositoryContext {
        native_launch_anchor: Ok(repository.clone()),
        business_repository: Ok(repository),
    }
}

/// 规划会话（Logical 路由，Story/Design/WorkItemPlan）的 native launch
/// anchor（REQ-PLN-08）：共同 selection 守卫通过即恒 LC 聚合根视图——
/// scope 回写（involved/focus/change_order/target）不迁移锚；守卫失败
/// fail-closed，native launch 不得绕过。业务选仓歧义（TargetAmbiguous/
/// TargetUnknown/无唯一 focus）不阻断 native-only launch——业务校验是
/// `business_repository` 字段的职责（design §3.1：native-only launch 不被
/// Design 草稿缺 change_order 的选仓歧义阻断，但产物/确认/compile 校验
/// 仍保留）。
fn planning_native_launch_anchor(
    manifest: &crate::product::logical_codebase::LogicalCodebaseManifest,
    selection: &crate::product::logical_codebase::IssueCodebaseSelection,
) -> Result<RepositoryRecord, ProductStoreError> {
    selection_context_guards(selection)?;
    Ok(aggregate_root_view(manifest))
}

/// LC Design 业务选仓（business_repository 用途；E 前原语义逐项保留）。
fn resolve_logical_design_repository(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    design: &crate::product::models::DesignSpecRecord,
    manifest: &crate::product::logical_codebase::LogicalCodebaseManifest,
    selection: &crate::product::logical_codebase::IssueCodebaseSelection,
) -> Result<RepositoryRecord, ProductStoreError> {
    let target_ids = unique_ids(design.involved_repository_ids.clone());
    // 方案X草稿态（缺陷 #5，同缺陷 #2 修法 A 口径）：involved 空 =
    // AI 自决之前，目标锚定 LC 聚合根视图；回写 involved 后恢复
    // 唯一成员解析。
    if target_ids.is_empty() {
        return Ok(aggregate_root_view(manifest));
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
        app_paths, project_id, issue_id, logical_id, manifest, selection,
    )
}

/// LC WorkItemPlan 业务选仓（business_repository 用途；E 前原语义逐项保留）。
fn resolve_logical_plan_repository(
    app_paths: &ProductAppPaths,
    lifecycle: &LifecycleStore,
    session: &EntityFacts<'_>,
    plan: &crate::product::models::IssueWorkItemPlan,
    manifest: &crate::product::logical_codebase::LogicalCodebaseManifest,
    selection: &crate::product::logical_codebase::IssueCodebaseSelection,
) -> Result<RepositoryRecord, ProductStoreError> {
    let targets = resolve_logical_work_item_plan_repository_targets(lifecycle, plan)
        .map_err(|reason| routing_error_for_target_error(&reason))?;
    let mut target_ids = targets
        .unwrap_or_default()
        .keys()
        .copied()
        .collect::<BTreeSet<LogicalRepositoryId>>();
    // 缺陷 #7（同族）：聚合 plan 会话 target 以源 Design involved 集
    // 过滤（LC selection 恒 all_members，不过滤恒 Ambiguous）；无聚合
    // 视野（involved 空）保持原 target 集不变。
    let design_involved = load_involved_from_confirmed_design(lifecycle, plan)
        .map_err(|reason| routing_error_for_target_error(&reason))?;
    if !design_involved.is_empty() {
        let involved: std::collections::BTreeSet<LogicalRepositoryId> =
            design_involved.into_iter().collect();
        target_ids.retain(|id| involved.contains(id));
    }
    match plan_session_repository_target(target_ids, &plan.id, selection)? {
        PlanSessionRepositoryTarget::Member(logical_id) => resolve_selected_logical_repository(
            app_paths,
            session.project_id,
            session.issue_id,
            logical_id,
            manifest,
            selection,
        ),
        // S6 方案 B：plan 会话=orchestrator 角色，多 target 且
        // focus 非唯一时锚定聚合根视图（同 story 草稿态口径；
        // 视图仅供 attach/launch 锚定，不落盘、不参与写根授权）。
        PlanSessionRepositoryTarget::AggregateRoot => Ok(aggregate_root_view(manifest)),
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
    selection_context_guards(selection)?;
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

/// 规划上下文共同 selection 守卫（REQ-PLN-08 由 resolve_selected_logical_repository
/// 原内联检查迁移同源）：selection 失效与 focus ⊄ include 在业务成员解析与
/// native launch anchor 两用途共享 fail-closed，native anchor 不得绕过；成员
/// 归属（TargetUnknown/TargetAmbiguous）仍只在业务选仓面判定。
fn selection_context_guards(
    selection: &crate::product::logical_codebase::IssueCodebaseSelection,
) -> Result<(), ProductStoreError> {
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
    })
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

/// B1 铁律：聚合根视图 id 前缀。视图仅供 attach/launch 锚定用途，
/// 不得作 compile/IR target（多仓 plan 的 IR/WI 目标按每 WI 成员归属
/// 落成员物理 ID；lowering 守卫同源引用本前缀）。
pub const AGGREGATE_ROOT_VIEW_ID_PREFIX: &str = "lc_aggregate_root_view_";

/// 会话解析出的业务仓记录是否为聚合根视图（写点判定与 lowering 守卫
/// 共用；聚合视图会话的 IR/compile 目标必须走 per-WI 成员归属解析）。
pub fn is_aggregate_root_view_repository(repository: &RepositoryRecord) -> bool {
    repository.id.starts_with(AGGREGATE_ROOT_VIEW_ID_PREFIX)
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
        id: format!(
            "{AGGREGATE_ROOT_VIEW_ID_PREFIX}{}",
            manifest.logical_codebase_id
        ),
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
    session: &EntityFacts<'_>,
) -> Result<RepositoryRecord, ProductStoreError> {
    let physical_repository_id = IssueStore::new(app_paths.clone())
        .get(session.project_id, session.issue_id)?
        .repo_id
        .ok_or_else(|| ProductStoreError::NotFound {
            kind: "repository",
            id: format!("issue:{}:repo_id", session.issue_id),
        })?;
    resolve_legacy_physical_repository(app_paths, session.project_id, &physical_repository_id)
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
