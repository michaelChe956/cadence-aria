use super::*;
use crate::product::logical_codebase::repository_routing::RepositoryRouting;
use crate::product::logical_codebase::{
    IssueCodebaseSelectionStore, LogicalCodebaseFeature, LogicalCodebaseStore, LogicalRepositoryId,
    MemberStatus,
};
use serde::{Deserialize, Serialize};

use std::collections::BTreeSet;

use crate::product::planning_failure::{
    PlanningCycleWitnessHop, PlanningEdgeSource, PlanningFailure, PlanningFailureEvidence,
    PlanningFailureKind, PlanningFailureOrigin, PlanningFailureSet, PlanningStage,
};
use crate::product::repository_store::RepositoryStore;
use crate::product::work_item_contract::RequiredInputContract;

impl WorkspaceEngine {
    pub(crate) fn is_current_work_item_plan_batch_mode(&self) -> bool {
        let Ok(store) = self.work_item_plan_store() else {
            return false;
        };
        store
            .load_active_index(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .ok()
            .flatten()
            .map(|index| {
                index.batches.iter().any(|batch| {
                    batch.generation_round_id == index.current_generation_round_id
                        && batch.mode == WorkItemGenerationMode::Batch
                })
            })
            .unwrap_or(false)
    }

    pub(crate) fn logical_work_item_plan_repository_targets(
        &self,
        lifecycle: &LifecycleStore,
        plan: &IssueWorkItemPlan,
    ) -> Result<Option<std::collections::BTreeMap<LogicalRepositoryId, String>>, String> {
        resolve_logical_work_item_plan_repository_targets(lifecycle, plan)
    }
}

/// 解析 issue 的 logical work item plan 仓库 target 映射（REQ-PLN-02 消费点）。
/// 独立为自由函数以便无 engine 的单元测试直接验证 target 有效性校验。
/// 成对判定复用 RepositoryRouting（Task 11），避免与 resume 入口漂移：
/// Legacy → Ok(None)；Logical → 既有 target map 构建逻辑；
/// FailClosed → Err 带稳定错误码（repository_routing_*，B3）供 HTTP 映射识别。
pub(crate) fn resolve_logical_work_item_plan_repository_targets(
    lifecycle: &LifecycleStore,
    plan: &IssueWorkItemPlan,
) -> Result<Option<std::collections::BTreeMap<LogicalRepositoryId, String>>, String> {
    let paths = lifecycle.app_paths();
    // C4 Task 2：issue 身份先经唯一 authority resolver 冻结（kind/来源/legacy
    // 布局冲突 fail-closed）；无归属且无 legacy 别名 record 的旧数据保留
    // project 级兼容分支。
    let lc_id = crate::product::logical_codebase::RepositoryAuthorityResolver::new(paths.clone())
        .resolve_for_issue(&plan.project_id, &plan.issue_id)
        .map_err(|error| format!("resolve issue logical codebase failed: {error}"))?
        .and_then(|resolution| resolution.target.logical_codebase_id);
    let (logical_store, selection_store) = match lc_id.as_deref() {
        Some(lc_id) => (
            LogicalCodebaseStore::for_lc(paths.clone(), lc_id),
            IssueCodebaseSelectionStore::for_lc(paths.clone(), lc_id),
        ),
        None => (
            LogicalCodebaseStore::new(paths.clone()),
            IssueCodebaseSelectionStore::new(paths.clone()),
        ),
    };
    let manifest = logical_store
        .load_manifest(&plan.project_id)
        .map_err(|error| format!("load logical codebase manifest failed: {error}"))?;
    let selection = selection_store
        .load(&plan.project_id, &plan.issue_id)
        .map_err(|error| format!("load issue codebase selection failed: {error}"))?;
    let (manifest, _selection) = match RepositoryRouting::classify(manifest, selection) {
        // (None, None)：无 manifest 且无 selection → 传统单仓，target map 为空。
        RepositoryRouting::Legacy { .. } => return Ok(None),
        // (Some, Some)：逻辑代码库 → 继续按既有 target map 构建逻辑。
        RepositoryRouting::Logical {
            manifest,
            selection,
        } => (manifest, *selection),
        // 其余不成对状态 → fail-closed，稳定错误码（B3），绝不静默回退物理仓库。
        RepositoryRouting::FailClosed { code, .. } => {
            return Err(format!(
                "{}: logical codebase manifest and issue selection must both exist",
                code.stable_code()
            ));
        }
    };
    // REQ-PLN-02：有效成员 = manifest.member_ids ∩ active_member_ids（manifest 外的
    // active member 不算逻辑代码库成员）；manifest 中非 active 成员（删除/停用）进入
    // 失效集合，selection 据此失效并阻断指向它们的新工作项。
    let active_set: std::collections::BTreeSet<LogicalRepositoryId> = logical_store
        .list_members(&plan.project_id)
        .map_err(|error| format!("list logical codebase members failed: {error}"))?
        .into_iter()
        .filter(|member| member.status == MemberStatus::Active)
        .map(|member| member.logical_repository_id)
        .collect();
    let effective_active_member_ids: Vec<LogicalRepositoryId> = manifest
        .member_ids
        .iter()
        .copied()
        .filter(|id| active_set.contains(id))
        .collect();
    let stale_manifest_members: Vec<LogicalRepositoryId> = manifest
        .member_ids
        .iter()
        .copied()
        .filter(|id| !active_set.contains(id))
        .collect();
    let resolution = selection_store
        .resolve_effective_members(
            &plan.project_id,
            &plan.issue_id,
            &effective_active_member_ids,
        )
        .map_err(|error| format!("resolve issue codebase selection failed: {error}"))?;
    // REQ-PLN-02：manifest 不一致（删除成员仍留在 manifest.member_ids）→ 显式标记
    // selection 失效（resolve_effective_members 仅在 selection 引用失效成员时自动标记）。
    if !stale_manifest_members.is_empty() && resolution.selection.invalidation.is_none() {
        selection_store
            .mark_invalidated(&plan.project_id, &plan.issue_id, "member_removed")
            .map_err(|error| {
                format!("mark issue codebase selection invalidated failed: {error}")
            })?;
    }
    // REQ-PLN-02：目标指向已删除/停用成员时 blocker。manifest 中非 active 成员直接
    // 判定为删除；selection 引用的失效成员若 on-disk 非 Active 也判为删除。
    let mut removed_targets = stale_manifest_members;
    let on_disk_removed: Vec<LogicalRepositoryId> = resolution
        .effective_member_ids
        .iter()
        .chain(resolution.invalid_member_ids.iter())
        .copied()
        .filter(|id| {
            logical_store
                .load_member(&plan.project_id, *id)
                .ok()
                .flatten()
                .is_some_and(|member| member.status != MemberStatus::Active)
        })
        .collect();
    for id in on_disk_removed {
        if !removed_targets.contains(&id) {
            removed_targets.push(id);
        }
    }
    if !removed_targets.is_empty() {
        return Err(format!(
            "work_item_target_missing: target_member_removed: {:?}",
            removed_targets
        ));
    }
    if !resolution.invalid_member_ids.is_empty() {
        return Err(format!(
            "work_item_target_missing: issue codebase selection has invalid members: {:?}",
            resolution.invalid_member_ids
        ));
    }

    let repository_store =
        RepositoryStore::with_logical_codebase_feature(paths, LogicalCodebaseFeature::enabled());
    // 缺陷 #6（同族第三处）：per-LC（v1.3）成员/checkouts 在 LC 子树，无 legacy
    // repos.json 投影；`resolve_logical_repository` 走 legacy scope 权威恒
    // NotFound/IdentityMismatch。按 issue lc_id 走 for_issue_codebase 分流
    // （for_lc 合成 / legacy 语义不变）。
    resolution
        .effective_member_ids
        .into_iter()
        .map(|target_repository_id| {
            repository_store
                .resolve_logical_repository_for_issue_codebase(
                    &plan.project_id,
                    lc_id.as_deref(),
                    target_repository_id,
                )
                .map(|(_, _, repository)| (target_repository_id, repository.id))
                .map_err(|error| {
                    format!(
                        "work_item_target_missing: cannot resolve target repository `{target_repository_id:?}`: {error}"
                    )
                })
        })
        .collect::<Result<_, _>>()
        .map(Some)
}

/// B1 铁律（多仓 plan 聚合视图缺口，session_0003 现场）：plan 会话
/// IR/compile 写入点的 per-WI 成员归属目标表。
/// - Legacy（单仓，target map 为空）→ `Ok(None)`：既有单 target 语义零变化；
/// - LC → `Ok(Some)`：`physical_by_logical` 与 Final Compile 的 logical_targets
///   同源（`resolve_logical_work_item_plan_repository_targets`）；确定性回落
///   `fallback_physical_id` 按既有规则——成员锚定会话（唯一 target/唯一
///   focus）= 会话业务成员物理仓；聚合视图会话（多 target 无唯一 focus）=
///   confirmed design `change_order` 首成员物理仓（r62 design 首仓兜底先例），
///   change_order 空 → None（无归属 WI 由 lowering 语义错 fail-closed）。
///   聚合视图 ID 不得作 compile target（Final Compile 以
///   work_item_target_missing 拒收）。
pub(crate) fn plan_compile_member_targets(
    lifecycle: &LifecycleStore,
    plan: &IssueWorkItemPlan,
    session_repository: &crate::product::models::RepositoryRecord,
) -> Result<Option<crate::product::work_item_plan_compiler::PlanMemberTargets>, String> {
    let Some(physical_by_logical) =
        resolve_logical_work_item_plan_repository_targets(lifecycle, plan)?
    else {
        return Ok(None);
    };
    let fallback_physical_id =
        if crate::product::workspace_repository::is_aggregate_root_view_repository(
            session_repository,
        ) {
            load_change_order_from_confirmed_design(lifecycle, plan)?
                .iter()
                .find_map(|logical| physical_by_logical.get(logical).cloned())
        } else {
            Some(session_repository.id.clone())
        };
    Ok(Some(
        crate::product::work_item_plan_compiler::PlanMemberTargets {
            physical_by_logical,
            fallback_physical_id,
        },
    ))
}

/// 加载 issue 的 confirmed Design 的 change_order（仓级执行顺序，REQ-TGT-04 消费点）。
/// compile 入口经 lifecycle + plan.source_design_spec_ids 加载 confirmed design，取
/// DesignSpecRecord.change_order。单仓/无 aggregate scope（无 confirmed design）→ 返回空
/// （不映射），compile 行为不变（红线）。
pub(crate) fn load_change_order_from_confirmed_design(
    lifecycle: &LifecycleStore,
    plan: &IssueWorkItemPlan,
) -> Result<Vec<LogicalRepositoryId>, String> {
    let designs = lifecycle
        .list_design_specs(&plan.project_id, &plan.issue_id)
        .map_err(|error| format!("list design specs failed: {error}"))?;
    // 只取 plan.source_design_spec_ids 指向且已 Confirmed 的 design（Task 6 confirm gate 产物）。
    let confirmed = designs.iter().find(|design| {
        plan.source_design_spec_ids.contains(&design.id)
            && design.confirmation_status == LifecycleConfirmationStatus::Confirmed
    });
    Ok(confirmed
        .map(|design| design.change_order.clone())
        .unwrap_or_default())
}

/// 加载 plan 源 confirmed Design 的 involved 集（缺陷 #7 同族：聚合 plan 的
/// 会话 target 以此过滤——LC selection 恒 all_members，不过滤则 plan 会话
/// 路由恒 TargetAmbiguous）。无聚合视野/无 confirmed design → 空。
pub(crate) fn load_involved_from_confirmed_design(
    lifecycle: &LifecycleStore,
    plan: &IssueWorkItemPlan,
) -> Result<Vec<LogicalRepositoryId>, String> {
    let designs = lifecycle
        .list_design_specs(&plan.project_id, &plan.issue_id)
        .map_err(|error| format!("list design specs failed: {error}"))?;
    let confirmed = designs.iter().find(|design| {
        plan.source_design_spec_ids.contains(&design.id)
            && design.confirmation_status == LifecycleConfirmationStatus::Confirmed
    });
    Ok(confirmed
        .map(|design| design.involved_repository_ids.clone())
        .unwrap_or_default())
}

// ===================== SG1 单源有效图纯核心（REQ-PLN-09/10、REQ-SG-02）=====================
//
// 本节是 plan §0.5 / 设计 §4.1 的纯输入输出核心：外层负责读取并核授权
// （Confirmed Design/member/selection/options/baseline/intent 事实一次装
// 载），纯函数只消费 `PlanningStructureContext` 里的已确认事实——
// 0 selection writes、0 provider I/O、0 transaction。有效图
// E = canonical Inputs provider→consumer 边 ∪ change_order 自动边，
// 方向恒为前置→后置；change_order 规则与 Final Compile 合并路径共用
// `change_order_auto_edges`（单一规则源，不复制边算法）。

/// SG1 有效图源字段定位（纯事实）：`field` 为 canonical 源内字段路径，
/// `line` 为 1-based 行号；不携带任何 store/provider 引用。
/// 尚未接线：SG2/SG3a 消费前仅测试引用。
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PlanningStructureSpan {
    pub field: String,
    pub line: usize,
}

/// SG1 纯核心上下文（plan §0.5 / 设计 §4.1）：外层一次加载并核授权后的
/// 只读事实快照。refs/hash/digest 绑定 source/IR/编译器/规则与
/// Design/member/selection/options/baseline/intent 摘要；本类型不构成新
/// 入站许可——缺失/失效/多份矛盾 Design 由外层装载时按原
/// operational/业务门阻断，不在这里随便选一个。
/// 尚未接线：SG2/SG3a 消费前仅测试引用。
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PlanningStructureContext {
    pub source_revision_ref: String,
    pub plan_candidate_ir_ref: String,
    pub source_hash: String,
    pub compiler_version: String,
    pub rule_version: String,
    pub context_digest: String,
    /// plan 绑定的 Confirmed Design ID 及内容摘要（未绑定为 None，不猜）。
    pub confirmed_design_id: Option<String>,
    pub confirmed_design_digest: Option<String>,
    /// 成员表/选定成员快照摘要与授权逻辑成员→物理仓 target（一次加载，
    /// 与 Final Compile logical_targets 同源）。
    pub member_table_digest: Option<String>,
    pub selection_digest: Option<String>,
    pub physical_by_logical: BTreeMap<LogicalRepositoryId, String>,
    /// confirmed Design 的仓级执行顺序（change_order 自动边唯一来源）。
    pub change_order: Vec<LogicalRepositoryId>,
    pub options_digest: String,
    pub baseline_tree_digest: Option<String>,
    pub intent_digest: Option<String>,
}

/// SG1 canonical Input 连同源定位：契约原样保留（provider、contract_id、
/// required_capabilities、compatibility_policy），`span` 指向声明字段。
/// 尚未接线：SG2/SG3a 消费前仅测试引用。
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PlanningStructureCanonicalInput {
    pub contract: RequiredInputContract,
    pub span: PlanningStructureSpan,
}

/// SG1 纯核心输入项（设计 §4.1）：逻辑 WI 身份/短编号/title、原成员声明、
/// 解析后逻辑/物理 target、canonical Inputs、声明依赖与源定位。
/// `declared_depends_on`（Dependencies.depends_on）单独保留、按既有规则
/// 验证一致性——它不是有效图边，未参与当前编译的声明边不得误称实际边。
/// 尚未接线：SG2/SG3a 消费前仅测试引用。
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PlanningStructureItem {
    pub logical_work_item_id: String,
    pub short_id: String,
    pub title: String,
    /// 原成员声明（未解析的声明归属）。
    pub declared_member: Option<String>,
    /// 解析后逻辑成员 target（授权成员 ID；legacy 单仓为 None）。
    pub logical_target: Option<LogicalRepositoryId>,
    /// 解析后物理 target（与授权成员表解析结果核同构；None = 未声明）。
    pub physical_target: Option<String>,
    pub canonical_inputs: Vec<PlanningStructureCanonicalInput>,
    pub declared_depends_on: Vec<String>,
    pub source_spans: Vec<PlanningStructureSpan>,
}

/// SG1 有效图边（plan §0.5）：`from`→`to` 恒为前置→后置。`sources` 保留
/// 全部来源（固定枚举序 CanonicalInput→ChangeOrder）——同一端点两种来源
/// 都保，删一来源不等于删除有效边；`contract_refs`/`spans` 属
/// canonical_input 来源，`order_indices` 属 change_order 来源
/// （[前仓索引, 后仓索引]，无该来源时为空）。
/// 尚未接线：SG2/SG3a 消费前仅测试引用。
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EffectivePlanningEdge {
    pub from: String,
    pub to: String,
    pub sources: Vec<PlanningEdgeSource>,
    pub contract_refs: Vec<String>,
    pub spans: Vec<PlanningStructureSpan>,
    pub order_indices: Vec<usize>,
}

/// 声明依赖事实（与有效边分别保留，设计 §4.1「声明不一致」呈现）：
/// `from`=被依赖方、`to`=声明方（与 IssueWorkItemDependencyEdge 同向）。
/// `resolvable`=被引用 ID 在本批 items 内；`effective`=该端点对在有效图
/// 中确有边（来源不限）。声明边不是有效图边。
/// 尚未接线：SG2/SG3a 消费前仅测试引用。
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PlanningStructureDeclaredDependencyFact {
    pub from: String,
    pub to: String,
    pub resolvable: bool,
    pub effective: bool,
}

/// SG1 有效图投影报告（plan §0.5 / 设计 §4.1）：有效边（含全部来源）、
/// 可排序结果、Kahn 残余组、logical→物理 target 映射、声明依赖事实与
/// typed failures（真实有向 witness 在 DependencyCycle payload 内）。
/// 残余组只是 Kahn 分组事实，不是有向 witness，禁止画成 A→B→A
/// （REQ-PLN-10）。纯输出，不含 store/provider I/O。
/// 尚未接线：SG2/SG3a 消费前仅测试引用。
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PlanningStructureReport {
    pub source_hash: String,
    pub context_digest: String,
    pub rule_version: String,
    pub compiler_version: String,
    /// 按 (from,to) 稳定排序的有效边全集。
    pub effective_edges: Vec<EffectivePlanningEdge>,
    /// 无环时的稳定拓扑序（Kahn FIFO，按 ID 稳定序播种）；有环为 None。
    pub topological_order: Option<Vec<String>>,
    /// Kahn 残余节点组（稳定排序）——不可排序节点集合，不是 witness。
    pub residual_nodes: Vec<String>,
    /// 逻辑 WI → 物理仓 target 映射（授权成员表单源解析；legacy 单仓
    /// item 不参与）。
    pub physical_targets: BTreeMap<String, String>,
    pub declared_dependency_facts: Vec<PlanningStructureDeclaredDependencyFact>,
    pub failures: PlanningFailureSet,
    /// 合法报告不含未解决阻断；空有效图 + 空 failures 是合法 DAG。
    pub structurally_valid: bool,
}

/// 以逻辑 WI ID 构造单源有效图并全量校验（SG1 纯投影）：
/// ① 逻辑 WI 身份唯一（重复 = Operational，不驱 author）；
/// ② target×成员归属核验（授权成员表解析，logical/物理映射同构）；
/// ③ canonical Inputs 数据流边（provider 不在本批 → typed
///    UnknownProvider，真实 Inputs 引用不因检出被删除）；
/// ④ change_order 自动边（同仓无自动边、空顺序不映射——单仓语义不变）；
/// ⑤ 现 Kahn 核心判可排序性 + 确定性 DFS 提取真实有向 witness
///    （每一跳确在有效图中并标来源）。
/// 尚未接线：SG2/SG3a 消费前仅测试引用。
#[allow(dead_code)]
pub(crate) fn project_effective_planning_graph(
    items: &[PlanningStructureItem],
    context: &PlanningStructureContext,
) -> PlanningStructureReport {
    let mut failures: Vec<PlanningFailure> = Vec::new();

    // ① 逻辑 WI 身份唯一：重复是装载/投影内部错（Operational，不驱
    // author）。首次出现继续参与投影，阻断来自 typed 失败，不虚构结构环。
    let mut known_ids: BTreeSet<&str> = BTreeSet::new();
    let mut unique: Vec<&PlanningStructureItem> = Vec::with_capacity(items.len());
    for item in items {
        if known_ids.insert(item.logical_work_item_id.as_str()) {
            unique.push(item);
        } else {
            failures.push(structure_failure(
                PlanningFailureKind::Operational {
                    stable_code: "duplicate_logical_work_item_identity".to_string(),
                    cause: format!(
                        "logical work item identity `{}` appears more than once",
                        item.logical_work_item_id
                    ),
                },
                PlanningFailureOrigin::System,
                std::slice::from_ref(&item.logical_work_item_id),
                context,
            ));
        }
    }

    // ② target×成员归属核验：授权成员表（与 Final Compile logical_targets
    // 同源）单源解析物理 target；声明物理 target 不一致或成员未授权 →
    // typed TargetMismatch（真实 Inputs/AC 不因检出被删除）。
    let mut physical_targets: BTreeMap<String, String> = BTreeMap::new();
    for item in &unique {
        let Some(logical_target) = &item.logical_target else {
            continue; // legacy 单仓 item 不参与映射（单仓语义不变）。
        };
        let mismatch = match context.physical_by_logical.get(logical_target) {
            Some(authorized) => {
                physical_targets.insert(item.logical_work_item_id.clone(), authorized.clone());
                // 声明物理 target 缺失 = 未声明，不算不一致（类型合同）。
                item.physical_target
                    .as_deref()
                    .is_some_and(|declared| declared != authorized.as_str())
                    .then(|| PlanningFailureKind::TargetMismatch {
                        logical_work_item_id: item.logical_work_item_id.clone(),
                        declared_target: item.physical_target.clone(),
                        resolved_target: authorized.clone(),
                    })
            }
            None => Some(PlanningFailureKind::TargetMismatch {
                logical_work_item_id: item.logical_work_item_id.clone(),
                declared_target: item.physical_target.clone(),
                resolved_target: format!("unauthorized_logical_target:{logical_target:?}"),
            }),
        };
        if let Some(kind) = mismatch {
            // 呈现事实（短编号/title/源定位）进入 details，供 SG2 指令渲染。
            let span = item
                .source_spans
                .first()
                .map(|span| format!("{}:{}", span.field, span.line))
                .unwrap_or_default();
            let failure = PlanningFailure {
                evidence: PlanningFailureEvidence {
                    affected_ids: vec![item.logical_work_item_id.clone()],
                    details: BTreeMap::from([
                        ("short_id".to_string(), item.short_id.clone()),
                        ("title".to_string(), item.title.clone()),
                        ("item_source_span".to_string(), span),
                    ]),
                },
                ..structure_failure(kind, PlanningFailureOrigin::TargetAttribution, &[], context)
            };
            failures.push(failure);
        }
    }

    // ③ canonical Inputs 数据流边（provider→consumer）：provider 不在本批
    // → typed UnknownProvider，不虚构边；真实 Inputs 引用保留在 payload。
    let mut edges: BTreeMap<(&str, &str), EffectiveEdgeAccumulator> = BTreeMap::new();
    for item in &unique {
        for input in &item.canonical_inputs {
            let provider = input.contract.provider_logical_work_item_id.as_str();
            if !known_ids.contains(provider) {
                failures.push(structure_failure(
                    PlanningFailureKind::UnknownProvider {
                        provider: provider.to_string(),
                        consumer: item.logical_work_item_id.clone(),
                        contract_id: input.contract.contract_id.clone(),
                    },
                    PlanningFailureOrigin::EffectiveGraph,
                    &[provider.to_string(), item.logical_work_item_id.clone()],
                    context,
                ));
                continue;
            }
            let entry = edges
                .entry((provider, item.logical_work_item_id.as_str()))
                .or_default();
            entry.canonical = true;
            entry.contract_refs.push(input.contract.contract_id.clone());
            entry.spans.push(input.span.clone());
        }
    }

    // ④ change_order 自动边（前仓全部 WI→后仓全部 WI）：与 Final Compile
    // 合并路径共用 `change_order_pairs` 单一规则源；同仓/空顺序天然无对。
    let mut by_target: BTreeMap<&LogicalRepositoryId, Vec<&str>> = BTreeMap::new();
    for item in &unique {
        if let Some(target) = &item.logical_target {
            by_target
                .entry(target)
                .or_default()
                .push(item.logical_work_item_id.as_str());
        }
    }
    for (earlier_index, later_index) in change_order_pairs(&context.change_order) {
        let earlier_items = by_target.get(&context.change_order[earlier_index]);
        let later_items = by_target.get(&context.change_order[later_index]);
        if let (Some(earlier_items), Some(later_items)) = (earlier_items, later_items) {
            for later_item in later_items {
                for earlier_item in earlier_items {
                    let entry = edges.entry((earlier_item, later_item)).or_default();
                    if entry.order_indices.is_empty() {
                        entry.order_indices = vec![earlier_index, later_index];
                    }
                    entry.change_order = true;
                }
            }
        }
    }

    // 同端点多来源合并（固定枚举序 CanonicalInput→ChangeOrder），按
    // (from,to) 稳定排序输出。
    let effective_edges: Vec<EffectivePlanningEdge> = edges
        .iter()
        .map(|((from, to), entry)| EffectivePlanningEdge {
            from: (*from).to_string(),
            to: (*to).to_string(),
            sources: match (entry.canonical, entry.change_order) {
                (true, true) => vec![
                    PlanningEdgeSource::CanonicalInput,
                    PlanningEdgeSource::ChangeOrder,
                ],
                (true, false) => vec![PlanningEdgeSource::CanonicalInput],
                (false, true) => vec![PlanningEdgeSource::ChangeOrder],
                (false, false) => Vec::new(),
            },
            contract_refs: entry.contract_refs.clone(),
            spans: entry.spans.clone(),
            order_indices: entry.order_indices.clone(),
        })
        .collect();

    // ⑤ 现 Kahn 核心判可排序性（FIFO，按 ID 稳定序播种）+ 确定性 DFS
    // 提取真实有向 witness（固定 ID/邻接序，只取首个环，不枚举全部环）。
    let mut adjacency: BTreeMap<&str, std::collections::BTreeSet<&str>> = BTreeMap::new();
    let mut indegree: BTreeMap<&str, usize> = known_ids.iter().map(|id| (*id, 0usize)).collect();
    for (from, to) in edges.keys() {
        adjacency.entry(from).or_default().insert(to);
        *indegree.get_mut(to).expect("edge endpoint in known ids") += 1;
    }
    let mut queue: VecDeque<&str> = indegree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(id, _)| *id)
        .collect();
    let mut remaining = indegree.clone();
    let mut order: Vec<String> = Vec::with_capacity(unique.len());
    while let Some(id) = queue.pop_front() {
        order.push(id.to_string());
        if let Some(next_ids) = adjacency.get(id) {
            for next_id in next_ids {
                let degree = remaining.get_mut(next_id).expect("next id in remaining");
                *degree -= 1;
                if *degree == 0 {
                    queue.push_back(next_id);
                }
            }
        }
    }
    let sortable = order.len() == unique.len();
    let residual_nodes: Vec<String> = remaining
        .iter()
        .filter(|(_, degree)| **degree > 0)
        .map(|(id, _)| (*id).to_string())
        .collect();

    if !sortable && let Some(path) = find_cycle_witness(&known_ids, &adjacency) {
        let witness: Vec<PlanningCycleWitnessHop> = path
            .iter()
            .map(|(from, to)| {
                let entry = &edges[&(*from, *to)];
                let mut sources = Vec::new();
                if entry.canonical {
                    sources.push(PlanningEdgeSource::CanonicalInput);
                }
                if entry.change_order {
                    sources.push(PlanningEdgeSource::ChangeOrder);
                }
                PlanningCycleWitnessHop {
                    from: (*from).to_string(),
                    to: (*to).to_string(),
                    sources,
                }
            })
            .collect();
        // affected_ids 保持 witness 节点序（首跳 from + 各跳 to 去重）。
        let mut affected_ids: Vec<String> = Vec::with_capacity(witness.len() + 1);
        for (index, hop) in witness.iter().enumerate() {
            if index == 0 {
                affected_ids.push(hop.from.clone());
            }
            if !affected_ids.contains(&hop.to) {
                affected_ids.push(hop.to.clone());
            }
        }
        failures.push(structure_failure(
            PlanningFailureKind::DependencyCycle { witness },
            PlanningFailureOrigin::EffectiveGraph,
            &affected_ids,
            context,
        ));
    }

    // 声明依赖单独保留（不是有效图边）：resolvable=被引用 ID 在本批；
    // effective=该端点对在有效图中确有边（来源不限）。
    let mut declared_dependency_facts: Vec<PlanningStructureDeclaredDependencyFact> = Vec::new();
    for item in &unique {
        for dependency in &item.declared_depends_on {
            declared_dependency_facts.push(PlanningStructureDeclaredDependencyFact {
                from: dependency.clone(),
                to: item.logical_work_item_id.clone(),
                resolvable: known_ids.contains(dependency.as_str()),
                effective: edges
                    .contains_key(&(dependency.as_str(), item.logical_work_item_id.as_str())),
            });
        }
    }

    PlanningStructureReport {
        source_hash: context.source_hash.clone(),
        context_digest: context.context_digest.clone(),
        rule_version: context.rule_version.clone(),
        compiler_version: context.compiler_version.clone(),
        effective_edges,
        topological_order: sortable.then_some(order),
        residual_nodes,
        physical_targets,
        declared_dependency_facts,
        structurally_valid: failures.is_empty(),
        failures: PlanningFailureSet { failures },
    }
}

/// 有效边来源累加器：同端点 canonical/change_order 贡献分别保留，
/// 输出按固定枚举序合并（删一来源不等于删除有效边）。
// 尚未接线：SG2/SG3a 消费前仅测试引用。
#[allow(dead_code)]
#[derive(Default)]
struct EffectiveEdgeAccumulator {
    canonical: bool,
    change_order: bool,
    order_indices: Vec<usize>,
    contract_refs: Vec<String>,
    spans: Vec<PlanningStructureSpan>,
}

/// 构造带 context 身份的 typed 结构失败（stage 恒 Plan；diagnostic_ref
/// 由 source_hash+稳定码单源派生）。
// 尚未接线：SG2/SG3a 消费前仅测试引用。
#[allow(dead_code)]
fn structure_failure(
    kind: PlanningFailureKind,
    origin: PlanningFailureOrigin,
    affected_ids: &[String],
    context: &PlanningStructureContext,
) -> PlanningFailure {
    let diagnostic_ref = format!(
        "planning-structure/{}/{}",
        context.source_hash,
        kind.stable_code()
    );
    PlanningFailure {
        kind,
        origin,
        stage: PlanningStage::Plan,
        diagnostic_ref,
        source_ref: context.source_revision_ref.clone(),
        source_hash: context.source_hash.clone(),
        evidence: PlanningFailureEvidence {
            affected_ids: affected_ids.to_vec(),
            details: BTreeMap::new(),
        },
    }
}

/// 确定性 DFS 提取有向环 witness（固定 ID/邻接序）：遇到指向栈内节点的
/// 回边即返回栈片段+回边；每一跳都确在有效图邻接中。只取首个环，
/// 不枚举全部环，也不把 Kahn 残余节点集合画成环（REQ-PLN-10）。
// 尚未接线：SG2/SG3a 消费前仅测试引用。
#[allow(dead_code)]
fn find_cycle_witness<'a>(
    sorted_ids: &BTreeSet<&'a str>,
    adjacency: &BTreeMap<&'a str, BTreeSet<&'a str>>,
) -> Option<Vec<(&'a str, &'a str)>> {
    fn dfs<'a>(
        adjacency: &BTreeMap<&'a str, BTreeSet<&'a str>>,
        on_stack: &mut Vec<&'a str>,
        visited: &mut BTreeSet<&'a str>,
        node: &'a str,
    ) -> Option<Vec<(&'a str, &'a str)>> {
        visited.insert(node);
        on_stack.push(node);
        if let Some(neighbors) = adjacency.get(node) {
            for next in neighbors {
                if let Some(position) = on_stack.iter().position(|id| id == next) {
                    // 回边 node→next：从栈中 next 到 node 的栈片段即真实
                    // 有向路径，末跳回边闭合。
                    let mut path: Vec<(&'a str, &'a str)> = on_stack[position..]
                        .windows(2)
                        .map(|window| (window[0], window[1]))
                        .collect();
                    path.push((node, *next));
                    return Some(path);
                }
                if !visited.contains(next)
                    && let Some(found) = dfs(adjacency, on_stack, visited, next)
                {
                    return Some(found);
                }
            }
        }
        on_stack.pop();
        None
    }
    let mut on_stack: Vec<&'a str> = Vec::new();
    let mut visited: BTreeSet<&'a str> = BTreeSet::new();
    for id in sorted_ids {
        if !visited.contains(id)
            && let Some(found) = dfs(adjacency, &mut on_stack, &mut visited, id)
        {
            return Some(found);
        }
    }
    None
}

/// 把 Design 的 change_order（仓级执行顺序）映射成跨仓 WorkItem depends_on（REQ-TGT-04）。
/// 规则：
/// ① 两个 WorkItem target 仓 tA≠tB 且 change_order 中 tA 在 tB 之前 → B depends_on A；
/// ② 同 target 仓的多 item 不因 change_order 建跨仓依赖（同仓顺序由既有 WorkItem 级
///    depends_on/sequence_hint 决定）；
/// ③ 合并既有 WorkItem 级 depends_on（candidate input_contracts 提取）；
/// ④ 环检测 → blocker dependency_cycle。
fn apply_change_order_cross_repo_depends_on(
    work_items: &mut [LifecycleWorkItemRecord],
    change_order: &[LogicalRepositoryId],
) -> Result<(), String> {
    if change_order.is_empty() {
        // 单仓/无 aggregate scope → 空，不映射（红线：单仓 compile 行为不变）。
        return Ok(());
    }
    // 按 target 仓聚合 work item id（仅 Logical 分支有 target_repository_id）。
    let mut by_target: BTreeMap<LogicalRepositoryId, Vec<String>> = BTreeMap::new();
    for item in work_items.iter() {
        if let Some(target) = item.target_repository_id {
            by_target.entry(target).or_default().push(item.id.clone());
        }
    }
    // 对每对 (earlier_repo, later_repo)：所有 target=later_repo 的 item depends_on 所有
    // target=earlier_repo 的 item（规则①②）。仓序对展开走 `change_order_pairs`
    // 单一规则源——SG1 纯核心有效图与 Final Compile 合并路径不复制边算法。
    for (earlier_index, later_index) in change_order_pairs(change_order) {
        let earlier_items = by_target.get(&change_order[earlier_index]);
        let later_items = by_target.get(&change_order[later_index]);
        if let (Some(earlier_items), Some(later_items)) = (earlier_items, later_items) {
            for later_item in later_items {
                for earlier_item in earlier_items {
                    let item = work_items
                        .iter_mut()
                        .find(|it| &it.id == later_item)
                        .expect("later item id must exist in work_items");
                    // 规则③：合并既有 WorkItem 级 depends_on，去重。
                    if !item.depends_on.contains(earlier_item) {
                        item.depends_on.push(earlier_item.clone());
                    }
                }
            }
        }
    }
    // 规则④：环检测 → blocker dependency_cycle。
    if let Some(cycle_ids) = detect_dependency_cycle(work_items) {
        return Err(format!(
            "dependency_cycle: change_order 与既有 WorkItem 依赖构成环: {}",
            cycle_ids.join(", ")
        ));
    }
    Ok(())
}

/// change_order 仓序自动边的唯一规则源（REQ-PLN-09）：产出全部
/// (前仓索引, 后仓索引) 对（索引 i<j 天然跨仓；同仓/单仓/空顺序无对，
/// 单仓 compile 行为不变）。Final Compile 合并路径与 SG1 纯核心共用。
fn change_order_pairs(change_order: &[LogicalRepositoryId]) -> Vec<(usize, usize)> {
    let mut pairs = Vec::new();
    for earlier_index in 0..change_order.len() {
        for later_index in (earlier_index + 1)..change_order.len() {
            pairs.push((earlier_index, later_index));
        }
    }
    pairs
}

/// 环检测（Kahn）：对 work_items 的 depends_on（限定在本批 item id 内的边）做拓扑排序，
/// 有环则返回环内 work item id 列表，无环返回 None。
fn detect_dependency_cycle(work_items: &[LifecycleWorkItemRecord]) -> Option<Vec<String>> {
    let ids: HashSet<&str> = work_items.iter().map(|item| item.id.as_str()).collect();

    let mut indegree: HashMap<&str, usize> = work_items
        .iter()
        .map(|item| (item.id.as_str(), 0usize))
        .collect();
    let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();
    for item in work_items {
        for dependency_id in &item.depends_on {
            if ids.contains(dependency_id.as_str()) && dependency_id != &item.id {
                adjacency
                    .entry(dependency_id.as_str())
                    .or_default()
                    .push(item.id.as_str());
                *indegree.entry(item.id.as_str()).or_default() += 1;
            }
        }
    }
    let mut queue: VecDeque<&str> = indegree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(id, _)| *id)
        .collect();
    let mut visited = 0usize;
    while let Some(id) = queue.pop_front() {
        visited += 1;
        if let Some(next_ids) = adjacency.get(id) {
            for &next_id in next_ids {
                let degree = indegree.get_mut(next_id).expect("next id in indegree");
                *degree -= 1;
                if *degree == 0 {
                    queue.push_back(next_id);
                }
            }
        }
    }
    if visited == ids.len() {
        None
    } else {
        Some(
            indegree
                .iter()
                .filter(|(_, degree)| **degree > 0)
                .map(|(id, _)| (*id).to_string())
                .collect(),
        )
    }
}

impl WorkspaceEngine {
    pub(crate) fn work_item_plan_repository_id(
        &self,
        lifecycle: &LifecycleStore,
        plan: &IssueWorkItemPlan,
    ) -> Result<String, String> {
        let story_specs = lifecycle
            .list_story_specs(&plan.project_id, &plan.issue_id)
            .map_err(|error| format!("list story specs failed: {error}"))?;
        for story_id in &plan.source_story_spec_ids {
            if let Some(story) = story_specs.iter().find(|story| &story.id == story_id) {
                return Ok(story.repository_id.clone());
            }
        }
        Err("cannot resolve repository_id for WorkItemPlan compile".to_string())
    }

    #[cfg(test)]
    pub(crate) fn project_work_item_plan_drafts_for_compile(
        &self,
        previous_plan: &IssueWorkItemPlan,
        draft_records: &[WorkItemDraftRecord],
        context: WorkItemPlanCompileProjectionContext<'_>,
        change_order: &[LogicalRepositoryId],
    ) -> Result<
        (
            IssueWorkItemPlan,
            Vec<LifecycleWorkItemRecord>,
            Vec<VerificationPlan>,
        ),
        String,
    > {
        project_work_item_plan_drafts_for_compile(
            previous_plan,
            draft_records,
            context,
            change_order,
        )
    }
}

pub(crate) fn project_work_item_plan_drafts_for_compile(
    previous_plan: &IssueWorkItemPlan,
    draft_records: &[WorkItemDraftRecord],
    context: WorkItemPlanCompileProjectionContext<'_>,
    change_order: &[LogicalRepositoryId],
) -> Result<
    (
        IssueWorkItemPlan,
        Vec<LifecycleWorkItemRecord>,
        Vec<VerificationPlan>,
    ),
    String,
> {
    let outline_order = context.outline_order;
    let outline_to_work_item_id = context.outline_to_work_item_id;
    let outline_to_verification_plan_id = context.outline_to_verification_plan_id;
    let logical_targets = context.logical_targets;
    let now = context.now;
    let draft_by_outline: HashMap<&str, &WorkItemDraftRecord> = draft_records
        .iter()
        .map(|record| (record.outline_id.as_str(), record))
        .collect();
    let mut logical_to_outline_id = HashMap::new();
    for record in draft_records {
        if logical_to_outline_id
            .insert(
                record.candidate.logical_work_item_id.as_str(),
                record.outline_id.as_str(),
            )
            .is_some()
        {
            return Err(format!(
                "duplicate logical work item identity `{}` during compile",
                record.candidate.logical_work_item_id
            ));
        }
    }
    let mut work_items = Vec::with_capacity(outline_order.len());
    let mut verification_plans = Vec::with_capacity(outline_order.len());
    if let Some(logical_targets) = logical_targets {
        for outline_id in outline_order {
            let record = draft_by_outline
                .get(outline_id.as_str())
                .ok_or_else(|| format!("accepted draft for outline `{outline_id}` missing"))?;
            let target_repository_id = record.candidate.target_repository_id.ok_or_else(|| {
                    format!(
                        "work_item_target_missing: target_repository_id_missing for outline `{outline_id}`"
                    )
                })?;
            if !logical_targets.contains_key(&target_repository_id) {
                return Err(format!(
                    "work_item_target_missing: target_repository_id_not_effective for outline `{outline_id}`"
                ));
            }
        }
    }
    for (index, outline_id) in outline_order.iter().enumerate() {
        let record = draft_by_outline
            .get(outline_id.as_str())
            .ok_or_else(|| format!("accepted draft for outline `{outline_id}` missing"))?;
        let candidate = &record.candidate;
        let work_item_id = outline_to_work_item_id
            .get(outline_id)
            .cloned()
            .ok_or_else(|| format!("work item id for outline `{outline_id}` missing"))?;
        let verification_plan_id = outline_to_verification_plan_id
            .get(outline_id)
            .cloned()
            .ok_or_else(|| format!("verification plan id for outline `{outline_id}` missing"))?;
        let (repository_id, target_repository_id) = match logical_targets {
            Some(logical_targets) => {
                let target_repository_id = candidate.target_repository_id.expect(
                    "logical target prevalidation must ensure target_repository_id is present",
                );
                let repository_id = logical_targets.get(&target_repository_id).expect(
                    "logical target prevalidation must ensure target_repository_id is effective",
                );
                (repository_id.as_str(), Some(target_repository_id))
            }
            None => (context.repository_id, None),
        };
        let depends_on = candidate
                .canonical_contract_candidate
                .input_contracts
                .iter()
                .map(|input| {
                    let dependency_outline_id = logical_to_outline_id
                        .get(input.provider_logical_work_item_id.as_str())
                        .ok_or_else(|| {
                            format!(
                                "provider logical identity `{}` for `{outline_id}` missing",
                                input.provider_logical_work_item_id
                            )
                        })?;
                    outline_to_work_item_id
                        .get(*dependency_outline_id)
                        .cloned()
                        .ok_or_else(|| {
                            format!(
                                "dependency outline `{dependency_outline_id}` for `{outline_id}` missing"
                            )
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
        work_items.push(LifecycleWorkItemRecord {
            id: work_item_id.clone(),
            project_id: previous_plan.project_id.clone(),
            issue_id: previous_plan.issue_id.clone(),
            repository_id: repository_id.to_string(),
            target_repository_id,
            story_spec_ids: previous_plan.source_story_spec_ids.clone(),
            design_spec_ids: previous_plan.source_design_spec_ids.clone(),
            title: candidate
                .canonical_contract_candidate
                .identity
                .title
                .clone(),
            plan_status: WorkItemPlanStatus::Confirmed,
            execution_status: crate::product::models::WorkItemStatus::Pending,
            worktree_path: None,
            work_item_set_id: Some(previous_plan.id.clone()),
            source_work_item_plan_id: Some(previous_plan.id.clone()),
            source_outline_id: Some(record.outline_id.clone()),
            source_draft_id: Some(record.draft_id.clone()),
            planned_implementation_context: None,
            kind: crate::product::work_item_split_engine::types::parse_work_item_kind(
                &candidate.canonical_contract_candidate.identity.kind,
            ),
            sequence_hint: Some((index + 1) as u32),
            depends_on,
            exclusive_write_scopes: candidate
                .canonical_contract_candidate
                .write_policy
                .exclusive_scopes
                .clone(),
            forbidden_write_scopes: candidate
                .canonical_contract_candidate
                .write_policy
                .forbidden_scopes
                .clone(),
            context_budget: crate::product::models::WorkItemContextBudget::default(),
            verification_plan_ref: Some(verification_plan_id.clone()),
            require_execution_plan_confirm: previous_plan.options.require_execution_plan_confirm,
            execution_plan_status: crate::product::models::WorkItemExecutionPlanStatus::NotStarted,
            completion_commit: None,
            completion_diff_summary_ref: None,
            created_at: now.to_string(),
            updated_at: now.to_string(),
        });
        verification_plans.push(parse_compile_verification_plan(
            &candidate.verification_plan,
            verification_plan_id,
            previous_plan.project_id.clone(),
            previous_plan.issue_id.clone(),
            work_item_id,
            now.to_string(),
        ));
    }
    // Task 8：把 Design 的 change_order（仓级执行顺序）映射成跨仓 WorkItem depends_on。
    // 单仓/无 aggregate scope → change_order 为空 → 不映射，compile 行为不变（红线）。
    apply_change_order_cross_repo_depends_on(&mut work_items, change_order)?;
    let work_item_ids: Vec<String> = outline_order
        .iter()
        .filter_map(|outline_id| outline_to_work_item_id.get(outline_id).cloned())
        .collect();
    let verification_plan_ids: Vec<String> = outline_order
        .iter()
        .filter_map(|outline_id| outline_to_verification_plan_id.get(outline_id).cloned())
        .collect();
    let dependency_graph = work_items
        .iter()
        .flat_map(|work_item| {
            work_item
                .depends_on
                .iter()
                .map(|dependency_id| IssueWorkItemDependencyEdge {
                    from_work_item_id: dependency_id.clone(),
                    to_work_item_id: work_item.id.clone(),
                })
        })
        .collect();
    let mut compiled_plan = previous_plan.clone();
    compiled_plan.status = crate::product::models::IssueWorkItemPlanStatus::Confirmed;
    compiled_plan.work_item_ids = work_item_ids;
    compiled_plan.verification_plan_ids = verification_plan_ids;
    compiled_plan.repository_profile_ref = None;
    compiled_plan.dependency_graph = dependency_graph;
    compiled_plan.validator_findings = Vec::new();
    compiled_plan.updated_at = now.to_string();
    Ok((compiled_plan, work_items, verification_plans))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::repository_routing::RepositoryRoutingErrorCode;
    use crate::product::logical_codebase::{
        CodebaseMemberRecord, IssueCodebaseSelection, LogicalCodebaseManifest, MemberStatus,
        RepositoryCheckoutId, RepositorySourceIdentity, RepositoryType,
    };
    use crate::product::models::{IssueWorkItemPlanOptions, IssueWorkItemPlanStatus};
    use crate::product::work_item_contract::ContractCompatibilityPolicy;
    use std::collections::BTreeSet;
    use tempfile::TempDir;
    use uuid::Uuid;

    /// 稳定 UUID：禁止运行时随机，保证测试可复现。
    const fn stable_uuid(seed: u16) -> Uuid {
        let mut bytes = [0u8; 16];
        bytes[14] = (seed >> 8) as u8;
        bytes[15] = seed as u8;
        // version 7 + variant 10xx，满足 Uuid::from_bytes 的合法构造。
        bytes[6] = 0x70;
        bytes[8] = 0x80;
        Uuid::from_bytes(bytes)
    }

    struct CompileInvalidationFixture {
        _temp: TempDir,
        lifecycle: LifecycleStore,
    }

    impl CompileInvalidationFixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let lifecycle = LifecycleStore::new(ProductAppPaths::new(temp.path()));
            Self {
                _temp: temp,
                lifecycle,
            }
        }

        fn aggregate_root(&self) -> std::path::PathBuf {
            self._temp.path().join("aggregate-root")
        }

        /// 写入含单个 active 成员的 manifest + selection（include 该成员）。
        fn write_selection_with_member(&self, member_id: LogicalRepositoryId, alias: &str) {
            let store = LogicalCodebaseStore::new(self.lifecycle.app_paths());
            let manifest = LogicalCodebaseManifest::new(
                "project_0001",
                self.aggregate_root(),
                vec![member_id],
            );
            store.save_manifest("project_0001", &manifest).unwrap();
            store
                .save_member(
                    "project_0001",
                    &self.member_record(member_id, alias, MemberStatus::Active),
                )
                .unwrap();
            let selection = IssueCodebaseSelection::explicit(
                "project_0001",
                "issue_0001",
                vec![member_id],
                Vec::new(),
                Vec::new(),
                None,
            );
            IssueCodebaseSelectionStore::new(self.lifecycle.app_paths())
                .save(&selection)
                .unwrap();
        }

        /// 删除成员（tombstone 语义）：从 manifest.member_ids 移除并推进 revision，
        /// member record 保留但 status=Removed。
        fn delete_member(&self, member_id: LogicalRepositoryId, alias: &str) {
            let store = LogicalCodebaseStore::new(self.lifecycle.app_paths());
            let mut manifest = store.load_manifest("project_0001").unwrap().unwrap();
            manifest.member_ids.retain(|id| *id != member_id);
            manifest.membership_revision += 1;
            store.save_manifest("project_0001", &manifest).unwrap();
            store
                .save_member(
                    "project_0001",
                    &self.member_record(member_id, alias, MemberStatus::Removed),
                )
                .unwrap();
        }

        /// 写入含多个 active 成员的 manifest + member records（不写 selection）。
        fn write_manifest_with_members(
            &self,
            member_ids: &[LogicalRepositoryId],
            aliases: &[&str],
        ) {
            let store = LogicalCodebaseStore::new(self.lifecycle.app_paths());
            let manifest = LogicalCodebaseManifest::new(
                "project_0001",
                self.aggregate_root(),
                member_ids.to_vec(),
            );
            store.save_manifest("project_0001", &manifest).unwrap();
            for (id, alias) in member_ids.iter().zip(aliases) {
                store
                    .save_member(
                        "project_0001",
                        &self.member_record(*id, alias, MemberStatus::Active),
                    )
                    .unwrap();
            }
        }

        /// 写入含多个 active 成员的 manifest + selection（include 全部成员）。
        fn write_selection_with_members(
            &self,
            member_ids: &[LogicalRepositoryId],
            aliases: &[&str],
        ) {
            self.write_manifest_with_members(member_ids, aliases);
            let selection = IssueCodebaseSelection::explicit(
                "project_0001",
                "issue_0001",
                member_ids.to_vec(),
                Vec::new(),
                Vec::new(),
                None,
            );
            IssueCodebaseSelectionStore::new(self.lifecycle.app_paths())
                .save(&selection)
                .unwrap();
        }

        /// 真实 tombstone 语义（Plan 1 `apply_delete_tombstone`）：manifest.member_ids
        /// 不变，仅 member.status → Tombstoned。
        fn tombstone_member(&self, member_id: LogicalRepositoryId, alias: &str) {
            let store = LogicalCodebaseStore::new(self.lifecycle.app_paths());
            store
                .save_member(
                    "project_0001",
                    &self.member_record(member_id, alias, MemberStatus::Tombstoned),
                )
                .unwrap();
        }

        fn member_record(
            &self,
            id: LogicalRepositoryId,
            alias: &str,
            status: MemberStatus,
        ) -> CodebaseMemberRecord {
            let now = "2026-08-10T00:00:00Z".to_string();
            let checkout_path = self.aggregate_root().join(alias);
            CodebaseMemberRecord {
                logical_repository_id: id,
                physical_repository_id: format!("repository_{alias}"),
                alias: alias.to_string(),
                role: "service".to_string(),
                ordinal: 1,
                source_identity: RepositorySourceIdentity::from_git_parts(
                    &checkout_path,
                    checkout_path.join(".git"),
                    Some(format!("ssh://git@example.test/acme/{alias}.git")),
                ),
                repo_type: RepositoryType::Backend,
                tech_stack: vec!["rust".to_string()],
                owner: None,
                tags: Vec::new(),
                default_ref: None,
                checkout_ids: vec![RepositoryCheckoutId(Uuid::nil())],
                status,
                created_at: now.clone(),
                updated_at: now,
            }
        }

        fn plan(&self) -> IssueWorkItemPlan {
            IssueWorkItemPlan {
                id: "issue_work_item_plan_0001".to_string(),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                source_story_spec_ids: Vec::new(),
                source_design_spec_ids: Vec::new(),
                options: IssueWorkItemPlanOptions {
                    include_integration_tests: false,
                    include_e2e_tests: false,
                    force_frontend_backend_split: false,
                    require_execution_plan_confirm: false,
                },
                status: IssueWorkItemPlanStatus::Draft,
                work_item_ids: Vec::new(),
                repository_profile_ref: None,
                verification_plan_ids: Vec::new(),
                dependency_graph: Vec::new(),
                created_from_provider_run: None,
                validator_findings: Vec::new(),
                review_summary: None,
                ever_confirmed: None,
                created_at: "2026-08-10T00:00:00Z".to_string(),
                updated_at: "2026-08-10T00:00:00Z".to_string(),
            }
        }
    }

    #[test]
    fn compile_blocks_new_work_item_targeting_deleted_member() {
        let fixture = CompileInvalidationFixture::new();
        let api = LogicalRepositoryId(stable_uuid(0x0001));
        fixture.write_selection_with_member(api, "api");
        fixture.delete_member(api, "api");

        // compile 校验触发 resolve_effective_members，阻断指向已删除成员的新工作项。
        let compile =
            resolve_logical_work_item_plan_repository_targets(&fixture.lifecycle, &fixture.plan());
        assert!(matches!(
            compile,
            Err(ref reason) if reason.contains("target_member_removed")
        ));

        // resolve 过程中 selection 失效标记已自动写入（REQ-PLN-02）。
        let selection_store = IssueCodebaseSelectionStore::new(fixture.lifecycle.app_paths());
        assert!(
            selection_store
                .is_invalidated("project_0001", "issue_0001")
                .unwrap()
        );
    }

    #[test]
    fn compile_blocks_work_item_targeting_tombstoned_member_still_in_manifest() {
        // REQ-PLN-02 fix round 1：真实 tombstone 语义（apply_delete_tombstone 只置
        // status=Tombstoned、不改 manifest.member_ids）→ 仍被 target_member_removed 阻塞，
        // 且 selection 自动失效。
        let fixture = CompileInvalidationFixture::new();
        let api = LogicalRepositoryId(stable_uuid(0x0001));
        let web = LogicalRepositoryId(stable_uuid(0x0002));
        fixture.write_selection_with_members(&[api, web], &["api", "web"]);
        // tombstone web：manifest 仍含 web，仅 status → Tombstoned。
        fixture.tombstone_member(web, "web");

        // compile 校验触发 resolve_effective_members（active 集合），阻断指向已删除成员的新工作项。
        let compile =
            resolve_logical_work_item_plan_repository_targets(&fixture.lifecycle, &fixture.plan());
        assert!(matches!(
            compile,
            Err(ref reason) if reason.contains("target_member_removed")
        ));

        // resolve 过程中 selection 失效标记已自动写入（REQ-PLN-02）。
        let selection_store = IssueCodebaseSelectionStore::new(fixture.lifecycle.app_paths());
        assert!(
            selection_store
                .is_invalidated("project_0001", "issue_0001")
                .unwrap()
        );
    }

    #[test]
    fn compile_blocks_and_invalidates_all_members_selection_when_member_tombstoned() {
        // REQ-PLN-02 fix round 2：AllMembers selection 不含已删除成员，compile 仍须
        // 返回 target_member_removed 且 selection 被标记失效（manifest 不一致兑底）。
        let fixture = CompileInvalidationFixture::new();
        let api = LogicalRepositoryId(stable_uuid(0x0001));
        let web = LogicalRepositoryId(stable_uuid(0x0002));
        fixture.write_manifest_with_members(&[api, web], &["api", "web"]);
        let selection = IssueCodebaseSelection::all_members("project_0001", "issue_0001", None);
        IssueCodebaseSelectionStore::new(fixture.lifecycle.app_paths())
            .save(&selection)
            .unwrap();
        fixture.tombstone_member(web, "web");

        let compile =
            resolve_logical_work_item_plan_repository_targets(&fixture.lifecycle, &fixture.plan());
        assert!(matches!(
            compile,
            Err(ref reason) if reason.contains("target_member_removed")
        ));

        // selection 已被标记失效（manifest 不一致兑底，selection 自身不含删除成员）。
        let selection_store = IssueCodebaseSelectionStore::new(fixture.lifecycle.app_paths());
        assert!(
            selection_store
                .is_invalidated("project_0001", "issue_0001")
                .unwrap()
        );
    }

    #[test]
    fn compile_blocks_and_invalidates_when_deleted_member_absent_from_explicit_selection() {
        // REQ-PLN-02 fix round 2：selection 未包含已删除成员，compile 仍须返回
        // target_member_removed 且 selection 被标记失效（manifest 不一致兑底）。
        let fixture = CompileInvalidationFixture::new();
        let api = LogicalRepositoryId(stable_uuid(0x0001));
        let web = LogicalRepositoryId(stable_uuid(0x0002));
        fixture.write_manifest_with_members(&[api, web], &["api", "web"]);
        // selection 只 include api（不含已删除的 web）。
        let selection = IssueCodebaseSelection::explicit(
            "project_0001",
            "issue_0001",
            vec![api],
            Vec::new(),
            Vec::new(),
            None,
        );
        IssueCodebaseSelectionStore::new(fixture.lifecycle.app_paths())
            .save(&selection)
            .unwrap();
        fixture.tombstone_member(web, "web");

        let compile =
            resolve_logical_work_item_plan_repository_targets(&fixture.lifecycle, &fixture.plan());
        assert!(matches!(
            compile,
            Err(ref reason) if reason.contains("target_member_removed")
        ));

        // selection 已被标记失效（manifest 不一致兑底，selection 自身不含删除成员）。
        let selection_store = IssueCodebaseSelectionStore::new(fixture.lifecycle.app_paths());
        assert!(
            selection_store
                .is_invalidated("project_0001", "issue_0001")
                .unwrap()
        );
    }

    #[test]
    fn compile_unpaired_state_uses_repository_routing_stable_error() {
        // 评审 Warning：Task 11 必须 TDD——先断言 compile 的不成对错误是稳定错误码
        // （repository_routing_target_missing）。当前 compile 返回自由字符串
        // "work_item_target_missing: ..."，无稳定错误码 → 本测试 FAIL。
        let fixture = CompileInvalidationFixture::new();
        let api = LogicalRepositoryId(stable_uuid(0x0001));
        // 只写 manifest + member record，不写 selection → (Some, None) 不成对状态。
        fixture.write_manifest_with_members(&[api], &["api"]);
        let result =
            resolve_logical_work_item_plan_repository_targets(&fixture.lifecycle, &fixture.plan());
        let err = result.expect_err("(Some, None) must fail-closed");
        assert!(
            err.contains("repository_routing_target_missing"),
            "compile unpaired error must carry stable error code, got: {err}"
        );

        // 与 RepositoryRouting 判定一致（防漂移契约）
        let logical_store = LogicalCodebaseStore::new(fixture.lifecycle.app_paths());
        let manifest = logical_store
            .load_manifest("project_0001")
            .unwrap()
            .unwrap();
        let routing = RepositoryRouting::classify(Some(manifest), None);
        assert!(matches!(
            routing,
            RepositoryRouting::FailClosed {
                code: RepositoryRoutingErrorCode::TargetMissing,
                ..
            }
        ));
    }

    /// Task 8 测试用最小 WorkItem：只需 id/target_repository_id/depends_on 三个字段有意义，
    /// 其余用默认值，聚焦 change_order → 跨仓 depends_on 映射逻辑。
    fn compile_test_work_item(
        id: &str,
        target_repository_id: Option<LogicalRepositoryId>,
    ) -> LifecycleWorkItemRecord {
        let now = "2026-08-10T00:00:00Z".to_string();
        LifecycleWorkItemRecord {
            id: id.to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_legacy".to_string(),
            target_repository_id,
            story_spec_ids: Vec::new(),
            design_spec_ids: Vec::new(),
            title: format!("Work Item {id}"),
            plan_status: crate::product::models::WorkItemPlanStatus::Confirmed,
            execution_status: crate::product::models::WorkItemStatus::Pending,
            worktree_path: None,
            work_item_set_id: None,
            source_work_item_plan_id: None,
            source_outline_id: None,
            source_draft_id: None,
            planned_implementation_context: None,
            kind: crate::product::models::WorkItemKind::Backend,
            sequence_hint: None,
            depends_on: Vec::new(),
            exclusive_write_scopes: Vec::new(),
            forbidden_write_scopes: Vec::new(),
            context_budget: crate::product::models::WorkItemContextBudget::default(),
            verification_plan_ref: None,
            require_execution_plan_confirm: false,
            execution_plan_status: crate::product::models::WorkItemExecutionPlanStatus::NotStarted,
            completion_commit: None,
            completion_diff_summary_ref: None,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    #[test]
    fn compile_maps_change_order_to_cross_repo_depends_on() {
        // design change_order=[A,B]；work item 1 target=A，work item 2 target=B
        // compile → item 2 depends_on item 1（B 在 A 后）
        let repo_a = LogicalRepositoryId(stable_uuid(0x0101));
        let repo_b = LogicalRepositoryId(stable_uuid(0x0102));
        let mut items = [
            compile_test_work_item("work_item_a", Some(repo_a)),
            compile_test_work_item("work_item_b", Some(repo_b)),
        ];
        apply_change_order_cross_repo_depends_on(&mut items, &[repo_a, repo_b]).unwrap();
        assert!(
            items[1].depends_on.contains(&"work_item_a".to_string()),
            "B 在 change_order 中位于 A 之后 → item_b 应 depends_on item_a"
        );
        assert!(
            !items[0].depends_on.contains(&"work_item_b".to_string()),
            "A 在 change_order 中位于 B 之前 → item_a 不应 depends_on item_b"
        );
    }

    #[test]
    fn compile_rejects_change_order_cycle() {
        // change_order=[A,B] 且既有 WorkItem 级 depends_on A→B（A 依赖 B），叠加后形成环
        // A→B + B→A → blocker dependency_cycle。
        let repo_a = LogicalRepositoryId(stable_uuid(0x0103));
        let repo_b = LogicalRepositoryId(stable_uuid(0x0104));
        let mut items = [
            compile_test_work_item("work_item_a", Some(repo_a)),
            compile_test_work_item("work_item_b", Some(repo_b)),
        ];
        items[0].depends_on.push("work_item_b".to_string());
        let err = apply_change_order_cross_repo_depends_on(&mut items, &[repo_a, repo_b])
            .expect_err("change_order 与既有 depends_on 构成环必须失败");
        assert!(
            err.contains("dependency_cycle"),
            "环错误必须携带稳定 blocker 码 dependency_cycle，got: {err}"
        );
    }

    #[test]
    fn compile_same_repo_items_no_cross_repo_depends_from_change_order() {
        // 两 item 同 target=A → change_order 不建跨仓依赖（同仓顺序由既有 WorkItem 级
        // depends_on/sequence_hint 决定）。
        let repo_a = LogicalRepositoryId(stable_uuid(0x0105));
        let mut items = [
            compile_test_work_item("work_item_1", Some(repo_a)),
            compile_test_work_item("work_item_2", Some(repo_a)),
        ];
        apply_change_order_cross_repo_depends_on(&mut items, &[repo_a]).unwrap();
        assert!(
            items[0].depends_on.is_empty() && items[1].depends_on.is_empty(),
            "同 target 仓的 item 不因 change_order 建跨仓依赖"
        );
    }

    // ===================== SG1：单源有效图/来源边/真实 witness =====================
    // REQ-PLN-09/10、REQ-SG-02（plan Task SG1 Step 1）。全部走纯核心
    // `project_effective_planning_graph`：断言 refs/context/边/来源/witness/
    // 排序结果等真实投影事实，不匹配中文文案。

    use crate::product::planning_failure::is_complete_cycle_witness;

    /// SG1 测试上下文：refs/digest 固定；change_order 与授权成员 target
    /// 由用例显式给出（外层装载事实的纯快照）。
    fn sg_01_structure_context(
        change_order: Vec<LogicalRepositoryId>,
        physical_by_logical: BTreeMap<LogicalRepositoryId, String>,
    ) -> PlanningStructureContext {
        PlanningStructureContext {
            source_revision_ref: "plan-source/sg-01".to_string(),
            plan_candidate_ir_ref: "plan-ir/sg-01".to_string(),
            source_hash: "sg01-source-hash-0000000000000000000000000000000".to_string(),
            compiler_version: "compiler/sg-01".to_string(),
            rule_version: "rule/sg-01".to_string(),
            context_digest: "sg-01-context-digest".to_string(),
            confirmed_design_id: Some("design_0001".to_string()),
            confirmed_design_digest: Some("design-digest-0001".to_string()),
            member_table_digest: Some("member-table-digest-0001".to_string()),
            selection_digest: Some("selection-digest-0001".to_string()),
            physical_by_logical,
            change_order,
            options_digest: "options-digest-0001".to_string(),
            baseline_tree_digest: Some("baseline-digest-0001".to_string()),
            intent_digest: Some("intent-digest-0001".to_string()),
        }
    }

    /// SG1 测试 item：短编号/title/源定位固定；canonical inputs 由
    /// (provider, contract_id) 对生成，capabilities 派生自 contract_id。
    fn sg_01_structure_item(
        logical_work_item_id: &str,
        logical_target: Option<LogicalRepositoryId>,
        inputs: &[(&str, &str)],
    ) -> PlanningStructureItem {
        PlanningStructureItem {
            logical_work_item_id: logical_work_item_id.to_string(),
            short_id: logical_work_item_id.to_string(),
            title: format!("Work Item {logical_work_item_id}"),
            declared_member: None,
            logical_target,
            physical_target: None,
            canonical_inputs: inputs
                .iter()
                .enumerate()
                .map(
                    |(index, (provider, contract_id))| PlanningStructureCanonicalInput {
                        contract: RequiredInputContract {
                            contract_id: contract_id.to_string(),
                            provider_logical_work_item_id: provider.to_string(),
                            required_capabilities: vec![format!("{contract_id}.capability")],
                            compatibility_policy: ContractCompatibilityPolicy::RequireAll,
                        },
                        span: PlanningStructureSpan {
                            field: format!("input_contracts[{index}]"),
                            line: 10 + index,
                        },
                    },
                )
                .collect(),
            declared_depends_on: Vec::new(),
            source_spans: vec![PlanningStructureSpan {
                field: "identity".to_string(),
                line: 1,
            }],
        }
    }

    /// R1 事故（设计 §4.3）：WI-005 归 gateway（仓序在前）、Inputs 要求
    /// WI-004（frontend，仓序在后）→ 数据流边 WI-004→WI-005 与仓序自动边
    /// WI-005→WI-004 互相等待。
    fn sg_01_r1_fixture() -> (Vec<PlanningStructureItem>, PlanningStructureContext) {
        let gateway = LogicalRepositoryId(stable_uuid(0x0201));
        let frontend = LogicalRepositoryId(stable_uuid(0x0202));
        let physical_by_logical = BTreeMap::from([
            (gateway, "repository_gateway".to_string()),
            (frontend, "repository_frontend".to_string()),
        ]);
        let items = vec![
            sg_01_structure_item("WI-004", Some(frontend), &[]),
            sg_01_structure_item(
                "WI-005",
                Some(gateway),
                &[("WI-004", "contract.e2e-verification")],
            ),
        ];
        let context = sg_01_structure_context(vec![gateway, frontend], physical_by_logical);
        (items, context)
    }

    #[test]
    fn sg_01_dual_source_and_real_witness_r1_opposite_direction_edges_carry_sources() {
        let (items, context) = sg_01_r1_fixture();
        let report = project_effective_planning_graph(&items, &context);

        // 两方向有效边各携真实来源：数据流边 WI-004→WI-005 仅
        // canonical_input（含 contract/spans），仓序自动边 WI-005→WI-004 仅
        // change_order（含仓序索引 [0,1]）。
        assert_eq!(report.effective_edges.len(), 2, "R1 恰两条有效边");
        let data_flow = report
            .effective_edges
            .iter()
            .find(|edge| edge.from == "WI-004" && edge.to == "WI-005")
            .expect("数据流边 WI-004→WI-005 必须存在");
        assert_eq!(data_flow.sources, vec![PlanningEdgeSource::CanonicalInput]);
        assert_eq!(
            data_flow.contract_refs,
            vec!["contract.e2e-verification".to_string()]
        );
        assert_eq!(
            data_flow.spans,
            vec![PlanningStructureSpan {
                field: "input_contracts[0]".to_string(),
                line: 10,
            }]
        );
        assert!(data_flow.order_indices.is_empty());
        let auto_edge = report
            .effective_edges
            .iter()
            .find(|edge| edge.from == "WI-005" && edge.to == "WI-004")
            .expect("仓序自动边 WI-005→WI-004 必须存在");
        assert_eq!(auto_edge.sources, vec![PlanningEdgeSource::ChangeOrder]);
        assert!(auto_edge.contract_refs.is_empty());
        assert!(auto_edge.spans.is_empty());
        assert_eq!(auto_edge.order_indices, vec![0usize, 1usize]);

        // typed DependencyCycle：witness 两跳各标真实来源，且通过
        // F1.1 完整性判定（末跳回到首跳起点）。
        let cycle = report
            .failures
            .failures
            .iter()
            .find(|failure| failure.kind.stable_code() == "dependency_cycle")
            .expect("R1 必须产出 dependency_cycle typed failure");
        assert_eq!(cycle.origin, PlanningFailureOrigin::EffectiveGraph);
        assert_eq!(cycle.stage, PlanningStage::Plan);
        assert_eq!(cycle.source_ref, context.source_revision_ref);
        assert_eq!(cycle.source_hash, context.source_hash);
        let PlanningFailureKind::DependencyCycle { witness } = &cycle.kind else {
            panic!("dependency_cycle failure 必须携带 typed payload");
        };
        assert_eq!(
            witness,
            &vec![
                PlanningCycleWitnessHop {
                    from: "WI-004".to_string(),
                    to: "WI-005".to_string(),
                    sources: vec![PlanningEdgeSource::CanonicalInput],
                },
                PlanningCycleWitnessHop {
                    from: "WI-005".to_string(),
                    to: "WI-004".to_string(),
                    sources: vec![PlanningEdgeSource::ChangeOrder],
                },
            ]
        );
        assert!(is_complete_cycle_witness(witness));
        assert_eq!(
            cycle.evidence.affected_ids,
            vec!["WI-004".to_string(), "WI-005".to_string()]
        );

        // 排序阻断：不可排序、残余组恰为两节点、报告不合法。
        assert!(report.topological_order.is_none());
        assert_eq!(report.residual_nodes, vec!["WI-004", "WI-005"]);
        assert!(!report.structurally_valid);
        // 报告身份单源来自 context。
        assert_eq!(report.source_hash, context.source_hash);
        assert_eq!(report.context_digest, context.context_digest);
        assert_eq!(report.rule_version, context.rule_version);
        assert_eq!(report.compiler_version, context.compiler_version);
    }

    #[test]
    fn sg_01_dual_source_and_real_witness_same_endpoint_keeps_both_sources_and_edge_survives_single_source()
     {
        let backend = LogicalRepositoryId(stable_uuid(0x0203));
        let frontend = LogicalRepositoryId(stable_uuid(0x0204));
        let physical_by_logical = BTreeMap::from([
            (backend, "repository_backend".to_string()),
            (frontend, "repository_frontend".to_string()),
        ]);

        // 双来源：canonical input 边与仓序自动边同端点 WI-001→WI-002。
        let both = project_effective_planning_graph(
            &[
                sg_01_structure_item("WI-001", Some(backend), &[]),
                sg_01_structure_item("WI-002", Some(frontend), &[("WI-001", "contract.api")]),
            ],
            &sg_01_structure_context(vec![backend, frontend], physical_by_logical.clone()),
        );
        assert_eq!(both.effective_edges.len(), 1);
        assert_eq!(
            both.effective_edges[0].sources,
            vec![
                PlanningEdgeSource::CanonicalInput,
                PlanningEdgeSource::ChangeOrder,
            ],
            "同一端点两个来源都保留"
        );
        assert_eq!(
            both.effective_edges[0].contract_refs,
            vec!["contract.api".to_string()]
        );
        assert_eq!(both.effective_edges[0].order_indices, vec![0usize, 1usize]);
        assert_eq!(
            both.topological_order,
            Some(vec!["WI-001".to_string(), "WI-002".to_string()])
        );
        assert!(both.structurally_valid);

        // 删 canonical 来源：边仍有效（仅 change_order 来源），仍可排序。
        let change_order_only = project_effective_planning_graph(
            &[
                sg_01_structure_item("WI-001", Some(backend), &[]),
                sg_01_structure_item("WI-002", Some(frontend), &[]),
            ],
            &sg_01_structure_context(vec![backend, frontend], physical_by_logical.clone()),
        );
        assert_eq!(change_order_only.effective_edges.len(), 1);
        assert_eq!(
            change_order_only.effective_edges[0].sources,
            vec![PlanningEdgeSource::ChangeOrder]
        );
        assert!(
            change_order_only.effective_edges[0]
                .contract_refs
                .is_empty()
        );
        assert!(change_order_only.structurally_valid);

        // 删 change_order 来源（空顺序）：边仍有效（仅 canonical_input 来源）。
        let canonical_only = project_effective_planning_graph(
            &[
                sg_01_structure_item("WI-001", Some(backend), &[]),
                sg_01_structure_item("WI-002", Some(frontend), &[("WI-001", "contract.api")]),
            ],
            &sg_01_structure_context(Vec::new(), physical_by_logical),
        );
        assert_eq!(canonical_only.effective_edges.len(), 1);
        assert_eq!(
            canonical_only.effective_edges[0].sources,
            vec![PlanningEdgeSource::CanonicalInput]
        );
        assert!(canonical_only.effective_edges[0].order_indices.is_empty());
        assert!(canonical_only.structurally_valid);
    }

    #[test]
    fn sg_01_dual_source_and_real_witness_kahn_residual_downstream_nodes_not_faked_as_cycle() {
        // 纯 canonical 环 WI-A⇄WI-B + 下游 WI-C（输入来自 WI-A）：Kahn 残余
        // 含 C，但 witness 只能由真实邻接跳组成——C 不入环。
        let items = vec![
            sg_01_structure_item("WI-A", None, &[("WI-B", "contract.b")]),
            sg_01_structure_item("WI-B", None, &[("WI-A", "contract.a")]),
            sg_01_structure_item("WI-C", None, &[("WI-A", "contract.c")]),
        ];
        let report = project_effective_planning_graph(
            &items,
            &sg_01_structure_context(Vec::new(), BTreeMap::new()),
        );

        // 残余组（Kahn 分组事实）含全部三节点——它不是 witness。
        assert_eq!(report.residual_nodes, vec!["WI-A", "WI-B", "WI-C"]);
        assert!(report.topological_order.is_none());

        let cycle_failures: Vec<&PlanningFailure> = report
            .failures
            .failures
            .iter()
            .filter(|failure| failure.kind.stable_code() == "dependency_cycle")
            .collect();
        assert_eq!(cycle_failures.len(), 1, "恰一个环（不枚举全部环）");
        let PlanningFailureKind::DependencyCycle { witness } = &cycle_failures[0].kind else {
            panic!("dependency_cycle failure 必须携带 typed payload");
        };
        assert_eq!(witness.len(), 2);
        // 每一跳端点都在真实环 {WI-A, WI-B} 内：下游 C 不被误画进环。
        for hop in witness {
            assert!(
                ["WI-A", "WI-B"].contains(&hop.from.as_str()),
                "下游节点不得进入 witness：{hop:?}"
            );
            assert!(["WI-A", "WI-B"].contains(&hop.to.as_str()));
            assert_eq!(hop.sources, vec![PlanningEdgeSource::CanonicalInput]);
        }
        assert!(is_complete_cycle_witness(witness));
        assert!(!report.structurally_valid);
    }

    #[test]
    fn sg_01_dual_source_and_real_witness_hops_are_adjacent_and_report_order_stable() {
        // R1：witness 每跳都确在有效图边集内且来源逐边一致；同输入两次投影
        // 全等（固定 ID/邻接序的确定性）。
        let (items, context) = sg_01_r1_fixture();
        let first = project_effective_planning_graph(&items, &context);
        let second = project_effective_planning_graph(&items, &context);
        assert_eq!(first, second, "纯投影必须确定性");

        let cycle = first
            .failures
            .failures
            .iter()
            .find(|failure| failure.kind.stable_code() == "dependency_cycle")
            .expect("R1 必须产出 dependency_cycle");
        let PlanningFailureKind::DependencyCycle { witness } = &cycle.kind else {
            panic!("dependency_cycle failure 必须携带 typed payload");
        };
        for hop in witness {
            let edge = first
                .effective_edges
                .iter()
                .find(|edge| edge.from == hop.from && edge.to == hop.to)
                .unwrap_or_else(|| panic!("witness 跳 {hop:?} 不在有效图边集内"));
            assert_eq!(edge.sources, hop.sources, "witness 跳来源必须与边来源一致");
        }
    }

    #[test]
    fn sg_01_dual_source_and_real_witness_self_dependency_rejected_with_single_hop_witness() {
        // 自依赖：WI-001 的 canonical input provider 即自身 → 有效图自环，
        // 拒绝为 dependency_cycle，witness 是单跳真实边。
        let items = vec![sg_01_structure_item(
            "WI-001",
            None,
            &[("WI-001", "contract.self")],
        )];
        let report = project_effective_planning_graph(
            &items,
            &sg_01_structure_context(Vec::new(), BTreeMap::new()),
        );
        assert_eq!(report.effective_edges.len(), 1);
        assert_eq!(report.effective_edges[0].from, "WI-001");
        assert_eq!(report.effective_edges[0].to, "WI-001");
        assert_eq!(report.residual_nodes, vec!["WI-001"]);
        assert!(report.topological_order.is_none());

        let Some(witness) =
            report
                .failures
                .failures
                .iter()
                .find_map(|failure| match &failure.kind {
                    PlanningFailureKind::DependencyCycle { witness } => Some(witness),
                    _ => None,
                })
        else {
            panic!("自依赖必须产出 typed DependencyCycle");
        };
        assert_eq!(
            witness,
            &vec![PlanningCycleWitnessHop {
                from: "WI-001".to_string(),
                to: "WI-001".to_string(),
                sources: vec![PlanningEdgeSource::CanonicalInput],
            }]
        );
        assert!(is_complete_cycle_witness(witness));
        assert!(!report.structurally_valid);
    }

    #[test]
    fn sg_01_dual_source_and_real_witness_unknown_provider_rejected() {
        // 未知 provider：WI-002 引用本批外的 WI-404 → typed
        // UnknownProvider；不虚构边，真实 Inputs 引用不因检出被删除。
        let items = vec![
            sg_01_structure_item("WI-001", None, &[]),
            sg_01_structure_item("WI-002", None, &[("WI-404", "contract.missing")]),
        ];
        let report = project_effective_planning_graph(
            &items,
            &sg_01_structure_context(Vec::new(), BTreeMap::new()),
        );
        assert!(
            report.effective_edges.is_empty(),
            "未知 provider 不产生有效边"
        );
        assert_eq!(
            report
                .failures
                .failures
                .iter()
                .filter(|failure| failure.kind.stable_code() == "dependency_cycle")
                .count(),
            0
        );
        let unknown = report
            .failures
            .failures
            .iter()
            .find(|failure| failure.kind.stable_code() == "unknown_provider_logical_work_item")
            .expect("未知 provider 必须产出 typed failure");
        assert_eq!(unknown.origin, PlanningFailureOrigin::EffectiveGraph);
        assert_eq!(unknown.stage, PlanningStage::Plan);
        assert_eq!(
            unknown.evidence.affected_ids,
            vec!["WI-404".to_string(), "WI-002".to_string()]
        );
        let PlanningFailureKind::UnknownProvider {
            provider,
            consumer,
            contract_id,
        } = &unknown.kind
        else {
            panic!("必须携带 UnknownProvider payload");
        };
        assert_eq!(provider, "WI-404");
        assert_eq!(consumer, "WI-002");
        assert_eq!(contract_id, "contract.missing");
        assert!(!report.structurally_valid);
    }

    #[test]
    fn sg_01_dual_source_and_real_witness_same_repo_no_auto_edges_single_repo_unchanged() {
        let repo_a = LogicalRepositoryId(stable_uuid(0x0205));
        let physical_by_logical = BTreeMap::from([(repo_a, "repository_a".to_string())]);
        // 同仓两 item + 声明依赖（WI-002 声明依赖 WI-001，声明边不冒充有效边）。
        let mut declared_item = sg_01_structure_item("WI-002", Some(repo_a), &[]);
        declared_item.declared_depends_on = vec!["WI-001".to_string(), "WI-404".to_string()];
        let items = vec![
            sg_01_structure_item("WI-001", Some(repo_a), &[]),
            declared_item,
        ];

        // 同仓（change_order 单仓）：无自动边；单仓空顺序：同样无自动边。
        for change_order in [vec![repo_a], Vec::new()] {
            let report = project_effective_planning_graph(
                &items,
                &sg_01_structure_context(change_order, physical_by_logical.clone()),
            );
            assert!(
                report
                    .effective_edges
                    .iter()
                    .all(|edge| !edge.sources.contains(&PlanningEdgeSource::ChangeOrder)),
                "同仓/空顺序不得产生 change_order 自动边"
            );
            assert!(report.effective_edges.is_empty());
            assert_eq!(
                report.topological_order,
                Some(vec!["WI-001".to_string(), "WI-002".to_string()])
            );
            assert!(report.structurally_valid);
            assert!(report.residual_nodes.is_empty());
            // 声明依赖分别保留：同批可解析声明（无 canonical 边）标记为
            // 声明不一致（非有效边），批外声明不可解析。
            assert_eq!(
                report.declared_dependency_facts,
                vec![
                    PlanningStructureDeclaredDependencyFact {
                        from: "WI-001".to_string(),
                        to: "WI-002".to_string(),
                        resolvable: true,
                        effective: false,
                    },
                    PlanningStructureDeclaredDependencyFact {
                        from: "WI-404".to_string(),
                        to: "WI-002".to_string(),
                        resolvable: false,
                        effective: false,
                    },
                ]
            );
        }

        // legacy 单仓（无 target）+ 非空 change_order：无 target 的 item 不
        // 参与自动边，单仓语义不变。
        let legacy = vec![
            sg_01_structure_item("WI-001", None, &[]),
            sg_01_structure_item("WI-002", None, &[]),
        ];
        let report = project_effective_planning_graph(
            &legacy,
            &sg_01_structure_context(vec![repo_a], physical_by_logical),
        );
        assert!(report.effective_edges.is_empty());
        assert!(report.physical_targets.is_empty());
        assert!(report.structurally_valid);
    }

    #[test]
    fn sg_01_dual_source_and_real_witness_logical_physical_mapping_isomorphic_and_target_mismatch_typed()
     {
        let repo_a = LogicalRepositoryId(stable_uuid(0x0206));
        let repo_b = LogicalRepositoryId(stable_uuid(0x0207));
        let repo_c = LogicalRepositoryId(stable_uuid(0x0208));
        let physical_by_logical = BTreeMap::from([
            (repo_a, "repository_a".to_string()),
            (repo_b, "repository_b".to_string()),
        ]);
        let mut mismatched = sg_01_structure_item("WI-004", Some(repo_a), &[]);
        mismatched.physical_target = Some("repository_wrong".to_string());
        let items = vec![
            sg_01_structure_item("WI-001", Some(repo_a), &[]),
            sg_01_structure_item("WI-002", Some(repo_b), &[("WI-001", "contract.api")]),
            mismatched,
            sg_01_structure_item("WI-005", Some(repo_c), &[]),
        ];
        let report = project_effective_planning_graph(
            &items,
            &sg_01_structure_context(Vec::new(), physical_by_logical),
        );

        // logical/物理映射同构：每个可授权 item 的物理 target 与授权成员表
        // 单源一致（与 Final Compile logical_targets 同源）；未授权成员
        // 不入映射。
        assert_eq!(
            report.physical_targets,
            BTreeMap::from([
                ("WI-001".to_string(), "repository_a".to_string()),
                ("WI-002".to_string(), "repository_b".to_string()),
                ("WI-004".to_string(), "repository_a".to_string()),
            ])
        );
        // 有效边经映射投影保持结构：逻辑边 WI-001→WI-002 投影为
        // (repository_a, repository_b)，与 item 物理归属一致。
        let projected: BTreeSet<(String, String)> = report
            .effective_edges
            .iter()
            .map(|edge| {
                (
                    report.physical_targets[&edge.from].clone(),
                    report.physical_targets[&edge.to].clone(),
                )
            })
            .collect();
        assert_eq!(
            projected,
            BTreeSet::from([("repository_a".to_string(), "repository_b".to_string())])
        );

        // target×成员归属错：声明物理 target 与授权解析不一致、未授权成员
        // 各产出 typed TargetMismatch（origin=TargetAttribution）。
        let mismatches: Vec<&PlanningFailure> = report
            .failures
            .failures
            .iter()
            .filter(|failure| failure.kind.stable_code() == "target_mismatch")
            .collect();
        assert_eq!(mismatches.len(), 2);
        let mismatched_ids: Vec<&str> = mismatches
            .iter()
            .map(|failure| failure.evidence.affected_ids[0].as_str())
            .collect();
        assert_eq!(mismatched_ids, vec!["WI-004", "WI-005"]);
        for failure in &mismatches {
            assert_eq!(failure.origin, PlanningFailureOrigin::TargetAttribution);
            assert_eq!(failure.stage, PlanningStage::Plan);
            // 呈现事实（短编号/title/源定位）进入 details，供 SG2 指令渲染。
            assert_eq!(
                failure.evidence.details.get("short_id"),
                Some(&failure.evidence.affected_ids[0].to_string())
            );
            assert_eq!(
                failure.evidence.details.get("title"),
                Some(&format!("Work Item {}", failure.evidence.affected_ids[0]))
            );
            assert_eq!(
                failure.evidence.details.get("item_source_span"),
                Some(&"identity:1".to_string())
            );
        }
        let PlanningFailureKind::TargetMismatch {
            logical_work_item_id,
            declared_target,
            resolved_target,
        } = &mismatches[0].kind
        else {
            panic!("必须携带 TargetMismatch payload");
        };
        assert_eq!(logical_work_item_id, "WI-004");
        assert_eq!(declared_target.as_deref(), Some("repository_wrong"));
        assert_eq!(resolved_target, "repository_a");
        let PlanningFailureKind::TargetMismatch {
            resolved_target, ..
        } = &mismatches[1].kind
        else {
            panic!("必须携带 TargetMismatch payload");
        };
        assert!(resolved_target.starts_with("unauthorized_logical_target:"));
        assert!(!report.structurally_valid);
    }

    #[test]
    fn sg_01_dual_source_and_real_witness_change_order_rule_shared_with_final_compile() {
        // 单一规则源（不复制边算法）：纯核心的 change_order 来源边与
        // Final Compile 合并路径 `apply_change_order_cross_repo_depends_on`
        // 追加的 depends_on 完全一致（集合与逐 item 顺序）。
        let repo_a = LogicalRepositoryId(stable_uuid(0x0301));
        let repo_b = LogicalRepositoryId(stable_uuid(0x0302));
        let repo_c = LogicalRepositoryId(stable_uuid(0x0303));
        let physical_by_logical = BTreeMap::from([
            (repo_a, "repository_a".to_string()),
            (repo_b, "repository_b".to_string()),
            (repo_c, "repository_c".to_string()),
        ]);
        let items = vec![
            sg_01_structure_item("WI-001", Some(repo_a), &[]),
            sg_01_structure_item("WI-002", Some(repo_a), &[]),
            sg_01_structure_item("WI-003", Some(repo_b), &[]),
            sg_01_structure_item("WI-004", Some(repo_c), &[]),
        ];
        let report = project_effective_planning_graph(
            &items,
            &sg_01_structure_context(vec![repo_a, repo_b, repo_c], physical_by_logical),
        );
        // (from,to) 稳定排序 + 每条自动边携带仓序索引对。
        assert_eq!(
            report
                .effective_edges
                .iter()
                .map(|edge| (edge.from.as_str(), edge.to.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("WI-001", "WI-003"),
                ("WI-001", "WI-004"),
                ("WI-002", "WI-003"),
                ("WI-002", "WI-004"),
                ("WI-003", "WI-004"),
            ]
        );
        for edge in &report.effective_edges {
            assert_eq!(edge.sources, vec![PlanningEdgeSource::ChangeOrder]);
            let expected_indices = match (edge.from.as_str(), edge.to.as_str()) {
                ("WI-003", _) => vec![1usize, 2usize],
                (_, "WI-003") => vec![0usize, 1usize],
                _ => vec![0usize, 2usize],
            };
            assert_eq!(edge.order_indices, expected_indices);
        }
        assert_eq!(
            report.topological_order,
            Some(vec![
                "WI-001".to_string(),
                "WI-002".to_string(),
                "WI-003".to_string(),
                "WI-004".to_string(),
            ])
        );

        // 同规则喂 Final Compile 合并路径：depends_on 追加结果与纯核心
        // 自动边逐一对应（顺序也一致）。
        let mut work_items = [
            compile_test_work_item("WI-001", Some(repo_a)),
            compile_test_work_item("WI-002", Some(repo_a)),
            compile_test_work_item("WI-003", Some(repo_b)),
            compile_test_work_item("WI-004", Some(repo_c)),
        ];
        apply_change_order_cross_repo_depends_on(&mut work_items, &[repo_a, repo_b, repo_c])
            .unwrap();
        let expected_depends_on: BTreeMap<&str, Vec<&str>> = BTreeMap::from([
            ("WI-001", vec![]),
            ("WI-002", vec![]),
            ("WI-003", vec!["WI-001", "WI-002"]),
            ("WI-004", vec!["WI-001", "WI-002", "WI-003"]),
        ]);
        for item in &work_items {
            assert_eq!(
                item.depends_on
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
                *expected_depends_on.get(item.id.as_str()).unwrap(),
                "{} 的 depends_on 追加序与纯核心自动边一致",
                item.id
            );
        }
        let auto_edge_pairs: BTreeSet<(String, String)> = report
            .effective_edges
            .iter()
            .map(|edge| (edge.from.clone(), edge.to.clone()))
            .collect();
        let merged_pairs: BTreeSet<(String, String)> = work_items
            .iter()
            .flat_map(|item| {
                item.depends_on
                    .iter()
                    .map(|dependency| (dependency.clone(), item.id.clone()))
            })
            .collect();
        assert_eq!(auto_edge_pairs, merged_pairs);

        // 空顺序：Final Compile 合并路径不追加任何依赖（单仓红线）。
        let mut single = [
            compile_test_work_item("WI-001", Some(repo_a)),
            compile_test_work_item("WI-002", Some(repo_b)),
        ];
        apply_change_order_cross_repo_depends_on(&mut single, &[]).unwrap();
        assert!(single[0].depends_on.is_empty() && single[1].depends_on.is_empty());
    }

    #[test]
    fn sg_01_dual_source_and_real_witness_duplicate_identity_is_operational_not_structural() {
        // 重复逻辑 WI 身份是装载/投影内部错：typed Operational（不驱
        // author），绝不合并声明、不虚构结构失败。
        let items = vec![
            sg_01_structure_item("WI-001", None, &[]),
            sg_01_structure_item("WI-001", None, &[]),
        ];
        let report = project_effective_planning_graph(
            &items,
            &sg_01_structure_context(Vec::new(), BTreeMap::new()),
        );
        assert_eq!(report.failures.failures.len(), 1);
        let PlanningFailureKind::Operational { stable_code, .. } =
            &report.failures.failures[0].kind
        else {
            panic!("重复身份必须是 Operational");
        };
        assert_eq!(stable_code, "duplicate_logical_work_item_identity");
        assert_eq!(
            report.failures.failures[0].origin,
            PlanningFailureOrigin::System
        );
        assert_eq!(
            report.failures.failures[0].evidence.affected_ids,
            vec!["WI-001".to_string()]
        );
        assert!(!report.structurally_valid);
        assert_eq!(
            report.topological_order,
            Some(vec!["WI-001".to_string()]),
            "去重后图仍可排序（阻断来自 Operational，不虚构环）"
        );
    }
}
