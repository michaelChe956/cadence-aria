use super::*;
use crate::product::entity_deletion_cascade::CascadeDeleteError;
use crate::product::entity_deletion_store::{
    DeletionEntityKind, DeletionPhase, EntityDeletionManifest, EntityDeletionStore, EntityScope,
};
use crate::product::entity_operation_store::{
    EntityDeletionLease, EntityOperationError, EntityOperationStore, SharedResourceCleanupLease,
    SharedResourceKey,
};
use crate::product::models::{DeletionConfirmation, DeletionConfirmationReason};

/// DELETE /{story-specs|design-specs|work-item-plans}/{id} 的确认参数（plan §0.2）：
/// 默认 URL 无 query；仅前端确认弹窗同意后显式传 `?confirmed=true`。query 布尔
/// 不是安全授权——确认资格由服务端三分类（D1.4 `classify_...`）权威判定。
#[derive(Debug, Default, serde::Deserialize)]
pub struct DeleteEntityQuery {
    pub confirmed: Option<bool>,
}

/// 独立 work item 删除（非三类实体级联语义，D2.3：保持原行为——coding 门禁、
/// 级联清理 attempt workspace 与 shared worktree 条件清理均不变）。
pub async fn delete_work_item(
    State(state): State<WebAppState>,
    Path((project_id, issue_id, work_item_id)): Path<(String, String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let app_paths = product_app_paths(&state);
    let store = LifecycleStore::new(app_paths.clone());
    let session_ids = workspace_session_ids_for_entities(
        &store,
        &project_id,
        &issue_id,
        std::slice::from_ref(&work_item_id),
    )?;
    delete_work_item_with_cleanup(&app_paths, &store, &project_id, &issue_id, &work_item_id)
        .await?;
    evict_workspace_session_managers(&state, &session_ids).await;
    Ok(Json(json!({"status":"deleted"})))
}

pub async fn delete_work_item_plan(
    State(state): State<WebAppState>,
    Path((project_id, issue_id, plan_id)): Path<(String, String, String)>,
    Query(query): Query<DeleteEntityQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    cascade_delete_entity(
        &state,
        &project_id,
        &issue_id,
        DeletionEntityKind::Plan,
        &plan_id,
        query.confirmed,
    )
    .await?;
    Ok(Json(json!({"status":"deleted"})))
}

pub async fn delete_story_spec(
    State(state): State<WebAppState>,
    Path((project_id, issue_id, story_spec_id)): Path<(String, String, String)>,
    Query(query): Query<DeleteEntityQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    cascade_delete_entity(
        &state,
        &project_id,
        &issue_id,
        DeletionEntityKind::Story,
        &story_spec_id,
        query.confirmed,
    )
    .await?;
    Ok(Json(json!({"status":"deleted"})))
}

pub async fn delete_design_spec(
    State(state): State<WebAppState>,
    Path((project_id, issue_id, design_spec_id)): Path<(String, String, String)>,
    Query(query): Query<DeleteEntityQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    cascade_delete_entity(
        &state,
        &project_id,
        &issue_id,
        DeletionEntityKind::Design,
        &design_spec_id,
        query.confirmed,
    )
    .await?;
    Ok(Json(json!({"status":"deleted"})))
}

/// 三类实体（Story/Design/Plan）统一级联删除编排（plan Task D2.2 Step 3，
/// REQ-GRP-01、REQ-DEL-01/02/03/04/05/07）。
///
/// 顺序：scope/墓碑 → try 删除独占 → 固定 resource keys 排序 try cleanup lease
/// → 关系短锁内 manifest 采集＋coding/引用/分类门禁 → pending＋owned-index＋
/// purging 墓碑（墓碑最后立——拒绝路径零墓碑零 pending）→ 释放关系短锁 →
/// manager 失效 → 子依赖 purge（D1.1 children → source（D1.2a 原语）→
/// plan-store 产物 → revisions/publications → attempt 残留锁 → 目标仓资源 →
/// run 本体 → advance 独立 checkpoint）→ 根最后 → `list_entity_owned_residue`
/// 及 AdvanceStore list 复核目标独占内容为空 → remove pending → prune
/// owned-index（已 purge 集合清出索引；Plan 保留 worktree 墓碑 claim 集）
/// → complete → 内存面驱逐（workspace session manager）。
///
/// 内存面驱逐（⓪/⑪，v37 反馈 #3）：删除前收集该实体的 workspace session id
/// （删除后磁盘记录已读不到），删除**成功后**驱逐其内存 manager；门禁拒绝或
/// 中途失败一律不驱逐（失败零副作用）。
///
/// 重入（purging 墓碑在位）先 pending＋owner 索引（§0.5 固定来源），不重复
/// 门禁；根缺不 404；Deleted 幂等 200；任何实际 IO/JSON 错误不当空（fail-closed），
/// 拒绝不改 manager/记录。
async fn cascade_delete_entity(
    state: &WebAppState,
    project_id: &str,
    issue_id: &str,
    kind: DeletionEntityKind,
    entity_id: &str,
    confirmed: Option<bool>,
) -> ApiResult<()> {
    let app_paths = product_app_paths(state);
    let store = LifecycleStore::new(app_paths.clone());
    let deletion_store = EntityDeletionStore::new(app_paths.clone());
    let operations = EntityOperationStore::new(app_paths.clone());
    let scope = EntityScope::new(project_id, issue_id, kind, entity_id);

    // ⓪ 内存面驱逐准备（v37 反馈 #3）：级联删除会清掉该实体的 workspace
    //    session 磁盘记录，但 WorkspaceSessionRegistry 缓存的 manager 不随之
    //    失效——重新生成复用同 id session 时，WS 连接经 get_or_create_and_attach
    //    命中旧 manager，把已删会话的旧状态（messages/artifact/stage）原样发给
    //    前端。必须在磁盘删除前收集 session id（删除后
    //    `list_workspace_sessions` 已读不到这些记录）；收集失败 → 删除不执行
    //    （零副作用，与 `delete_work_item` 同纪律）。
    let entity_ids = [entity_id.to_string()];
    let mut session_ids =
        workspace_session_ids_for_entities(&store, project_id, issue_id, &entity_ids)?;

    // ① 墓碑：Deleted → 幂等成功（零动作、不动 manager/记录）；Purging → 重入续清。
    let reentry = match deletion_store
        .get(&scope)
        .map_err(product_store_api_error)?
    {
        Some(record) if record.phase == DeletionPhase::Deleted => {
            // REQ-WIGA-VOID-01：该删除早已成功收口——补一次作废（收敛
            // 「complete 与作废之间」的崩溃窗口）；尽力收口不阻断幂等成功。
            if kind == DeletionEntityKind::Plan {
                void_enrollment_after_plan_deletion(&app_paths, project_id, issue_id, entity_id);
            }
            return Ok(());
        }
        Some(_) => true,
        None => false,
    };

    // ①.5 F3：自动化占用预解除——绑定该实体的 enabled enrollment 会让
    //     autopilot 每 tick ensure/reconcile/生成写该实体（scope 共享锁），
    //     删除独占永远抢不到（真机 entity_busy 删不掉的根因）；且被删实体
    //     一旦消失该 enrollment 本身不可继续（Y 已证同向），终止是语义必然。
    //     复用与人工终止/autopilot 自愈同源的 terminate CAS；失败 → 明确
    //     automation_termination_failed，删除不执行（未取锁、零落盘）。
    let terminated_automation = terminate_automation_bound_to_entity(&app_paths, &store, &scope)?;

    // ①.6 REQ-WIGA-15（场景 1）：终止自动化登记后、取删除独占前，先取消该
    //     实体绑定 session 的在途 run 并等其真实收口——provider run 全程持
    //     scope 共享 flock（acquire_run），不先收口则删除独占永远抢不到
    //     （entity_busy 长期阻塞）。复用非 WebSocket 取消入口（等真实
    //     completion，内部 30s 兜底）；只遍历 ⓪ 已按 entity_id 精确收集的
    //     session_ids，取消不越界。registry 无该 session 的 manager（进程
    //     重启后不在内存）时零动作，删除继续按既有独占判定走——既有行为
    //     的自然延伸。
    let mut run_cancel_settled = false;
    for session_id in &session_ids {
        if let Some(manager) = state.workspace_sessions.get(session_id).await {
            // REQ-WIGA-17：按取消结果三态如实分支——NoActiveRun 属正常结果，
            // 零副作用、不打任何「取消」字样；Completed 只打事实性打点；
            // TimedOut 只打「取消未在兜底内收口」的事实（不谎称已取消/已
            // 停止），且不因此阻塞删除（按既有独占判定推进）。
            let outcome = manager.abort_active_run_and_wait_completion().await;
            if outcome.is_completed() {
                eprintln!(
                    "[aria-cancellation] entity delete cancelled in-flight run session_id={session_id}"
                );
                run_cancel_settled = true;
            } else if outcome.is_timed_out() {
                eprintln!(
                    "[aria-cancellation] entity delete run cancel did not settle within completion wait timeout session_id={session_id}"
                );
            }
        }
    }
    if run_cancel_settled {
        // REQ-WIGA-17 时序尾隙补足：completion 信号已到 ≠ scope 共享 flock
        // 已释放（run 收口点与释锁点的先后无契约保证，不得依赖未验证时序
        // 假设）。有界短退避：≤10 轮、每轮间隔 50ms（合计 ≤500ms），每轮
        // 一次「取删除独占并立即释放」的探测——探测成功即说明共享锁已释放
        // （探测不保留 lease、不触碰任何破坏性清理）；Busy/Deleting 退避
        // 重试，Deleted/Store 交给下方既有循环按既有分支处理；不得无限
        // 等待或长挂起。
        for _ in 0..10 {
            match operations.try_acquire_delete(&scope) {
                Ok(probe) => {
                    drop(probe);
                    break;
                }
                Err(EntityOperationError::Busy | EntityOperationError::Deleting) => {
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
                Err(EntityOperationError::Deleted | EntityOperationError::Store(_)) => break,
            }
        }
    }

    // ② try 删除独占（非阻塞）：争用/同 scope 运行 writer → entity_busy；
    //    已删竞态 → 幂等成功。刚终止过自动化时给在途短写一个有限排空窗口
    //    （tick 写是毫秒级；长 run 不在此等，仍按 entity_busy 报占用者）。
    let mut delete_attempts = if terminated_automation.is_some() {
        10
    } else {
        1
    };
    let lease = loop {
        match operations.try_acquire_delete(&scope) {
            Ok(lease) => break lease,
            Err(EntityOperationError::Deleted) => return Ok(()),
            Err(EntityOperationError::Busy | EntityOperationError::Deleting) => {
                delete_attempts -= 1;
                if delete_attempts > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    continue;
                }
                let occupant = automation_enrollment_bound_to_entity(&app_paths, &store, &scope)
                    .or_else(|| terminated_automation.clone());
                return Err(deletion_busy_error(&scope, occupant.as_ref()));
            }
            Err(EntityOperationError::Store(error)) => return Err(product_store_api_error(error)),
        }
    };

    // ③ Plan：闭包前捕获 plan run（N7/REQ-CS-07：冻结 target 集合，purge 后不可
    //    再读；损坏 run JSON 在任何变更前阻断删除且零落盘）。
    let plan_run = if kind == DeletionEntityKind::Plan {
        crate::product::coding_plan_run::CodingPlanRunStore::new(app_paths.clone())
            .get(project_id, issue_id, entity_id)
            .map_err(product_store_api_error)?
    } else {
        None
    };

    // ④ 固定 resource keys（manifest 目标仓 ∪ run 冻结 target ∪ legacy issue 布局）
    //    排序 try cleanup lease（§0.2 锁序 scope → resource → 关系短锁）：任何
    //    key 争用即 entity_busy，此时尚未立墓碑/pending——零破坏拒绝。
    let resource_leases = cascade_resource_cleanup_leases(
        &app_paths,
        &store,
        &scope,
        plan_run.as_ref(),
        terminated_automation.as_ref(),
    )?;

    // ⑤ 关系短锁内：权威 manifest 采集＋coding/引用/分类门禁 → pending →
    //    owned-index → purging 墓碑。门禁拒绝时墓碑尚未立起，零破坏。
    let gated = operations
        .with_issue_relations(project_id, issue_id, || {
            Ok(cascade_collect_and_gate(
                &app_paths,
                &store,
                &deletion_store,
                &lease,
                &scope,
                confirmed,
                reentry,
            ))
        })
        .map_err(product_store_api_error)?;
    let manifest = gated.map_err(cascade_gate_api_error)?;

    // ⑥.5 悬空引用失效：门禁已过、删除确定执行——source 引用本实体的
    //     automation enrollment 一并失效（F5；失败只 warn，不阻断删除）。
    if matches!(kind, DeletionEntityKind::Story | DeletionEntityKind::Design) {
        invalidate_automation_referencing_entity(&app_paths, &scope);
    }

    // ⑦ 子依赖 purge（依赖序：审计先于 refs 来源，子树先于根）。与 plan 代际
    //    取代路径**共用同一闭包实现**（`entity_deletion_cascade::purge_entity_closure`）：
    //    用户面删除走 `CodingGateMode::Refuse`（coding 门禁已在 ⑤ 拒删），
    //    代际取代走 `Cascade`（级联删 attempts 及其运行资源）。
    crate::product::entity_deletion_cascade::purge_entity_closure(
        &app_paths,
        &lease,
        &manifest,
        plan_run.as_ref(),
        &resource_leases,
        crate::product::entity_deletion_cascade::CodingGateMode::Refuse,
        Some(state.workspace_root.as_path()),
    )
    .await
    .map_err(product_store_api_error)?;

    // ⑧ 根最后（根缺=已清，不 404——REQ-DEL-07 重入）。
    store
        .delete_entity_root(&lease)
        .map_err(product_store_api_error)?;

    // ⑨ 残留复核（§0.5 固定来源反查）：目标独占内容为空才允许完成。
    let residue = store
        .list_entity_owned_residue(&scope)
        .map_err(product_store_api_error)?;
    if !entity_manifest_sets_empty(&residue) {
        return Err(ApiError::runtime(
            "entity_deletion_residue",
            "实体删除闭包存在未清残留，拒绝记完成",
            json!({
                "entity_kind": scope.kind.path_segment(),
                "entity_id": scope.entity_id,
                "residue": serde_json::to_value(&residue).unwrap_or(serde_json::Value::Null),
            }),
        ));
    }
    if kind == DeletionEntityKind::Plan {
        let artifacts = crate::product::advance_store::AdvanceStore::new(app_paths.clone())
            .list_plan_artifacts(project_id, issue_id, entity_id)
            .map_err(product_store_api_error)?;
        if !artifacts.advance_ids.is_empty() || !artifacts.retry_command_ids.is_empty() {
            return Err(ApiError::runtime(
                "entity_deletion_residue",
                "advance 产物存在未清残留，拒绝记完成",
                json!({ "entity_id": entity_id }),
            ));
        }
        if crate::product::coding_plan_run::CodingPlanRunStore::new(app_paths.clone())
            .get(project_id, issue_id, entity_id)
            .map_err(product_store_api_error)?
            .is_some()
        {
            return Err(ApiError::runtime(
                "entity_deletion_residue",
                "plan run 存在未清残留，拒绝记完成",
                json!({ "entity_id": entity_id }),
            ));
        }
        if crate::product::coding_plan_run::CodingPlanRunStore::new(app_paths.clone())
            .get_workspace_lifecycle(project_id, issue_id, entity_id)
            .map_err(product_store_api_error)?
            .is_some()
        {
            return Err(ApiError::runtime(
                "entity_deletion_residue",
                "plan workspace lifecycle metadata 存在未清残留，拒绝记完成",
                json!({ "entity_id": entity_id }),
            ));
        }
    }

    // ⑩ 收口：remove pending → prune owned-index（已 purge 集合不得残留成
    //    list-by-owner 的已删会话反查来源）→ complete（单条无内容事实；
    //    完成时间不因重试覆盖）。prune 在 complete 前：崩溃窗口由 purging
    //    重入的并集重建＋再次 prune 幂等收敛。
    deletion_store
        .remove_pending_manifest(&lease)
        .map_err(product_store_api_error)?;
    deletion_store
        .prune_owned_artifact_ids_for_completion(&lease)
        .map_err(product_store_api_error)?;
    deletion_store
        .complete(&lease)
        .map_err(product_store_api_error)?;
    // ⑩.5 REQ-WIGA-VOID-01：工作项组删除已完整收口（complete 在前——失败
    //     路径在上方已 return Err，不会到达这里）⇒ 绑定该 plan 的自动化
    //     登记一并作废。作废在删除成功之后：失败不回滚、只 warn 留诊断。
    if kind == DeletionEntityKind::Plan {
        void_enrollment_after_plan_deletion(&app_paths, project_id, issue_id, entity_id);
    }
    // ⑪ 内存面驱逐（v37 反馈 #3）：删除已完成 → 驱逐该实体的 workspace
    //    session manager。`manifest.session_ids` 覆盖 reentry（pending 原始
    //    全量 ∪ 重新采集）与 Plan 归属的子 work item 会话；与 ⓪ 的删除前
    //    快照取并集。驱逐只发生在删除成功之后——门禁拒绝/中途失败一律
    //    不驱逐（失败零副作用）。
    for session_id in &manifest.session_ids {
        if !session_ids.contains(session_id) {
            session_ids.push(session_id.clone());
        }
    }
    evict_workspace_session_managers(state, &session_ids).await;
    Ok(())
}

/// 关系短锁内的一次性采集＋门禁＋耐久化（cascade_delete_entity ⑤ 的锁内体）。
///
/// 重入（`reentry=true`）不重复门禁（删除已获授权），manifest 取 pending（原始
/// 全量）∪ 重新采集的并集（`merged_with`，不丢已索引事实）；首程在全部门禁
/// 通过后才写 pending/owned-index/墓碑，门禁顺序：coding（REQ-DEL-05）→
/// 存活引用（REQ-DEL-03）→ 分级确认（REQ-DEL-04，服务端三分类权威）。
fn cascade_collect_and_gate(
    app_paths: &ProductAppPaths,
    store: &LifecycleStore,
    deletion_store: &EntityDeletionStore,
    lease: &EntityDeletionLease,
    scope: &EntityScope,
    confirmed: Option<bool>,
    reentry: bool,
) -> Result<EntityDeletionManifest, CascadeGateError> {
    let fresh = store.collect_entity_deletion_manifest(scope)?;
    if !reentry {
        // coding 门禁：group 与所属 WI 任一 attempt 记录存在即拒（无论是否运行
        // 中；不自动 abort/delete coding；残留 lock 不构成阻挡）。
        if scope.kind == DeletionEntityKind::Plan {
            let coding_store = CodingAttemptStore::new(app_paths.clone());
            if let Some(attempt) = coding_store
                .list_attempts_for_work_item_group(
                    &scope.project_id,
                    &scope.issue_id,
                    &scope.entity_id,
                )?
                .into_iter()
                .next()
            {
                return Err(CascadeGateError::Api(coding_workspace_exists_error(
                    &scope.entity_id,
                    &attempt.id,
                )));
            }
            for work_item_id in &fresh.work_item_ids {
                if let Some(attempt) = coding_store
                    .list_attempts_for_work_item(&scope.project_id, &scope.issue_id, work_item_id)?
                    .into_iter()
                    // 他 plan 的 group attempt（group 绑定优先于 work_item 锚点
                    // 字段）不构成本 WI 的 coding 后置——门禁按 group 归属过滤。
                    .find(|attempt| {
                        attempt
                            .work_item_group_id
                            .as_deref()
                            .is_none_or(|group| group == scope.entity_id)
                    })
                {
                    return Err(CascadeGateError::Api(
                        coding_workspace_exists_for_work_item_error(work_item_id, &attempt.id),
                    ));
                }
            }
        }
        // 存活引用门禁：共享业务对象（review/run/VP/profile）被闭包外存活实体
        // 配置引用即拒——与共享基础设施（shared-worktree）保留语义分开。
        // D1.1 反查的 work_item 引用按 source 绑定判外属，比 manifest 归属
        //（record refs∪source∪set join）窄：本实体 manifest 已拥有的 WI 不算
        // 外部引用方（schema-v2 布局 WI 常无 source 字段、经 plan.work_item_ids
        // 引用归属），过滤后再判空。
        let references: Vec<_> = store
            .list_incoming_entity_references(scope)?
            .into_iter()
            .filter(|reference| {
                reference.kind != "work_item" || !fresh.work_item_ids.contains(&reference.id)
            })
            .collect();
        if !references.is_empty() {
            return Err(CascadeGateError::Api(entity_referenced_error(
                scope,
                &references,
            )));
        }
        // 分级确认门禁：Required 而 query 未显式 confirmed=true → 稳定 409。
        if let DeletionConfirmation::Required { reason } =
            store.classify_entity_deletion_confirmation(scope)?
            && confirmed != Some(true)
        {
            return Err(CascadeGateError::Api(deletion_confirmation_required_error(
                scope, reason,
            )));
        }
    }
    let manifest = match deletion_store.load_pending_manifest(lease)? {
        Some(pending) => pending.merged_with(&fresh)?,
        None => fresh,
    };
    deletion_store.save_pending_manifest(lease, &manifest)?;
    deletion_store.index_owned_artifact_ids(lease, &manifest)?;
    deletion_store.begin(lease)?;
    Ok(manifest)
}

/// 关系短锁内门禁错误的窄通道：`with_issue_relations` 以 ProductStoreError 为
/// 外层通道，业务拒绝（ApiError）经此 enum 穿出后统一映射。
enum CascadeGateError {
    Store(ProductStoreError),
    Api(ApiError),
}

impl From<ProductStoreError> for CascadeGateError {
    fn from(error: ProductStoreError) -> Self {
        Self::Store(error)
    }
}

fn cascade_gate_api_error(error: CascadeGateError) -> ApiError {
    match error {
        CascadeGateError::Store(error) => product_store_api_error(error),
        CascadeGateError::Api(error) => error,
    }
}

/// 固定 resource keys 排序 try cleanup lease（cascade_delete_entity ④）。
///
/// Plan 才有 repo 资源；Story/Design 无（空集）。key 集合 = manifest 目标仓
/// （owned WI 派生）∪ plan-run 冻结 target ∪ legacy issue 布局（记录存在才含
/// None 键）。BTreeSet<Option<Uuid>> 固定排序；任何 key 争用 → entity_busy。
fn cascade_resource_cleanup_leases(
    app_paths: &ProductAppPaths,
    store: &LifecycleStore,
    scope: &EntityScope,
    plan_run: Option<&crate::product::coding_plan_run::CodingPlanRun>,
    terminated_automation: Option<&crate::product::models::automation::IssueAutomationEnrollment>,
) -> ApiResult<Vec<(SharedResourceKey, SharedResourceCleanupLease)>> {
    if scope.kind != DeletionEntityKind::Plan {
        return Ok(Vec::new());
    }
    let (project_id, issue_id) = (&scope.project_id, &scope.issue_id);
    let mut repositories: std::collections::BTreeSet<Option<uuid::Uuid>> = Default::default();
    if let Some(run) = plan_run {
        for binding in &run.target_attempts {
            let parsed = uuid::Uuid::parse_str(&binding.target_repository_id).map_err(|_| {
                ApiError::runtime(
                    "plan_coding_run_target_invalid",
                    "plan run frozen target is not a logical repository id",
                    json!({
                        "plan_id": run.plan_id,
                        "target_repository_id": binding.target_repository_id,
                    }),
                )
            })?;
            repositories.insert(Some(parsed));
        }
    }
    let manifest = store
        .collect_entity_deletion_manifest(scope)
        .map_err(product_store_api_error)?;
    for target in &manifest.target_repository_ids {
        let parsed = uuid::Uuid::parse_str(target).map_err(|error| {
            ApiError::runtime(
                "entity_target_repository_invalid",
                "owned work item target repository is not a logical id",
                json!({
                    "target_repository_id": target,
                    "message": error.to_string(),
                }),
            )
        })?;
        repositories.insert(Some(parsed));
    }
    if store
        .get_issue_shared_worktree(project_id, issue_id)
        .map_err(product_store_api_error)?
        .is_some()
    {
        repositories.insert(None);
    }
    let operations = EntityOperationStore::new(app_paths.clone());
    let mut leases = Vec::new();
    for repository_id in repositories {
        let key = SharedResourceKey {
            project_id: project_id.clone(),
            issue_id: issue_id.clone(),
            repository_id: repository_id.map(LogicalRepositoryId),
        };
        let lease = operations
            .try_acquire_shared_resource_cleanup(&key)
            .map_err(|error| match error {
                EntityOperationError::Busy
                | EntityOperationError::Deleting
                | EntityOperationError::Deleted => {
                    // F3：key 争用仍是稳定 entity_busy，但 details 带占用者
                    // 信息——现场识别仍 enabled 且绑定该实体的自动化；否则
                    // 带上本次删除刚终止的 enrollment（其生成的在途 run/写
                    // 是残余占用者的最可能来源）。
                    let occupant = automation_enrollment_bound_to_entity(app_paths, store, scope)
                        .or_else(|| terminated_automation.cloned());
                    deletion_busy_error(scope, occupant.as_ref())
                }
                EntityOperationError::Store(inner) => product_store_api_error(inner),
            })?;
        leases.push((key, lease));
    }
    Ok(leases)
}

/// manifest 全部纯 ID 集合为空（残留复核用；scope 字段不算集合）。
fn entity_manifest_sets_empty(manifest: &EntityDeletionManifest) -> bool {
    manifest.session_ids.is_empty()
        && manifest.work_item_ids.is_empty()
        && manifest.review_ids.is_empty()
        && manifest.provider_run_ids.is_empty()
        && manifest.verification_plan_ids.is_empty()
        && manifest.repository_profile_ids.is_empty()
        && manifest.target_repository_ids.is_empty()
        && manifest.advance_ids.is_empty()
        && manifest.advance_retry_command_ids.is_empty()
}

/// 同 scope 仍有可落盘 writer / 删除准入争用 → 稳定 `entity_busy`（REQ-DEL-06）。
/// F3：details 补占用者信息——优先现场识别仍 enabled 且绑定该实体的自动化
/// enrollment；否则带上本次删除刚终止的 enrollment（其 ensure/生成的在途
/// 写/run 是残余占用者的最可能来源），便于判断该等还是该先停自动化。
fn deletion_busy_error(
    scope: &EntityScope,
    occupant: Option<&crate::product::models::automation::IssueAutomationEnrollment>,
) -> ApiError {
    let mut details = json!({
        "entity_kind": scope.kind.path_segment(),
        "entity_id": scope.entity_id,
    });
    if let Some(enrollment) = occupant {
        details["automation_enrollment_id"] = json!(enrollment.enrollment_id);
    }
    ApiError::runtime(
        "entity_busy",
        "实体存在进行中的运行或并发写入，请等待收口后重试删除",
        details,
    )
}

/// F3：识别仍占用该实体的自动化 enrollment——enabled 且未 terminated，且其
/// durable 绑定指向该实体（Plan 直绑 `plan_id`，或 session 记录的 `entity_id`
/// 即该实体）。enrollment 文件缺失/不可读/未启用/已终止/未绑定一律 None：
/// 不可证明占用不误报（不可读的 enrollment 驱动不了 autopilot，不会是占用者）。
fn automation_enrollment_bound_to_entity(
    app_paths: &ProductAppPaths,
    lifecycle: &LifecycleStore,
    scope: &EntityScope,
) -> Option<crate::product::models::automation::IssueAutomationEnrollment> {
    let enrollment =
        crate::product::issue_automation_store::IssueAutomationStore::new(app_paths.clone())
            .get(&scope.project_id, &scope.issue_id)
            .ok()??;
    if !enrollment.enabled || enrollment.terminated_at.is_some() {
        return None;
    }
    if scope.kind == DeletionEntityKind::Plan
        && enrollment.plan_id.as_deref() == Some(scope.entity_id.as_str())
    {
        return Some(enrollment);
    }
    if let Some(session_id) = enrollment.session_id.as_deref()
        && lifecycle
            .list_workspace_sessions(&scope.project_id, &scope.issue_id)
            .ok()?
            .iter()
            .any(|session| session.id == session_id && session.entity_id == scope.entity_id)
    {
        return Some(enrollment);
    }
    None
}

/// F3：取删除独占锁之前，先终止绑定该实体的自动化 enrollment（真机缺陷：
/// autopilot 每 tick 写该实体，删除独占永远抢不到；实体删除后 enrollment
/// 本身不可继续，终止是语义必然）。复用 82d8d470 同一终止 CAS
/// （`IssueAutomationStore::terminate_plan`：`terminated_at`＋`enabled=false`
/// 同锁落盘，同 revision 命令 id 稳定、重放幂等）。CAS 竞态（revision 前移/
/// 被并行终止）重读收敛后带新 revision 有限重试；终止失败返回明确
/// `automation_termination_failed`（删除未执行、零落盘），绝不退化成笼统
/// entity_busy。
fn terminate_automation_bound_to_entity(
    app_paths: &ProductAppPaths,
    lifecycle: &LifecycleStore,
    scope: &EntityScope,
) -> ApiResult<Option<crate::product::models::automation::IssueAutomationEnrollment>> {
    let mut last_conflict: Option<(
        crate::product::models::automation::IssueAutomationEnrollment,
        crate::product::models::automation::EnrollmentError,
    )> = None;
    for _ in 0..3 {
        let Some(enrollment) = automation_enrollment_bound_to_entity(app_paths, lifecycle, scope)
        else {
            return Ok(None);
        };
        let request = crate::product::models::automation::EnrollmentTerminationRequest {
            command_id: format!(
                "entity-delete-terminate-{}-r{}",
                enrollment.enrollment_id, enrollment.policy_revision
            ),
            expected_enrollment_id: enrollment.enrollment_id.clone(),
            expected_policy_revision: enrollment.policy_revision,
            reason: format!(
                "bound entity deleted: {}/{}",
                scope.kind.path_segment(),
                scope.entity_id
            ),
        };
        match crate::product::issue_automation_store::IssueAutomationStore::new(app_paths.clone())
            .terminate_plan(&scope.project_id, &scope.issue_id, request)
        {
            Ok(_) => return Ok(Some(enrollment)),
            // enrollment 文件已消失：无自动化可终止（目标已达成）。
            Err(crate::product::models::automation::EnrollmentError::NotFound) => return Ok(None),
            // CAS 竞态：重读收敛——已终止/禁用即 None，仍 enabled 带新
            // revision 重试。
            Err(error @ crate::product::models::automation::EnrollmentError::Conflict { .. }) => {
                last_conflict = Some((enrollment, error));
                continue;
            }
            Err(error) => {
                return Err(automation_termination_failed_error(
                    scope,
                    &enrollment,
                    &error.to_string(),
                ));
            }
        }
    }
    let (enrollment, error) = last_conflict.expect("conflict path always records enrollment");
    Err(automation_termination_failed_error(
        scope,
        &enrollment,
        &error.to_string(),
    ))
}

/// F3：自动化停用失败 → 稳定可诊断错误（409），删除不执行。
fn automation_termination_failed_error(
    scope: &EntityScope,
    enrollment: &crate::product::models::automation::IssueAutomationEnrollment,
    message: &str,
) -> ApiError {
    ApiError::runtime(
        "automation_termination_failed",
        "该实体的自动化未能停用，删除未执行",
        json!({
            "entity_kind": scope.kind.path_segment(),
            "entity_id": scope.entity_id,
            "automation_enrollment_id": enrollment.enrollment_id,
            "message": message,
        }),
    )
}

/// 存活后置引用拒删 → 稳定 `entity_referenced`（REQ-DEL-03），列出实际引用者。
fn entity_referenced_error(
    scope: &EntityScope,
    references: &[crate::product::entity_deletion_store::EntityReference],
) -> ApiError {
    ApiError::runtime(
        "entity_referenced",
        "实体仍被存活后置对象引用，请先删除引用它的对象",
        json!({
            "entity_kind": scope.kind.path_segment(),
            "entity_id": scope.entity_id,
            "references": references
                .iter()
                .map(|reference| json!({"kind": reference.kind, "id": reference.id}))
                .collect::<Vec<_>>(),
        }),
    )
}

/// 分级确认必答 → 稳定 `deletion_confirmation_required`（REQ-DEL-04）。
fn deletion_confirmation_required_error(
    scope: &EntityScope,
    reason: DeletionConfirmationReason,
) -> ApiError {
    let reason_code = match reason {
        DeletionConfirmationReason::PreviouslyConfirmed => "previously_confirmed",
        DeletionConfirmationReason::LegacyHistoryUnknown => "legacy_history_unknown",
    };
    ApiError::runtime(
        "deletion_confirmation_required",
        "该实体曾确认或确认历史未知，删除前需要一次确认",
        json!({
            "reason": reason_code,
            "entity_kind": scope.kind.path_segment(),
            "entity_id": scope.entity_id,
        }),
    )
}

/// F5（悬空源根修）：删除 Story/Design 时，source 引用该实体的 automation
/// enrollment 一并失效——`enabled` 且未 terminated 的走与人工终止/autopilot
/// 自愈同源的 terminate CAS（`terminated_at`＋`enabled=false` 同锁落盘、稳定
/// command_id 幂等重放）；已 terminated/disabled 的记录本身已失效（悬空
/// source 由人工重新开启的悬空重授权收口），零改写。与 ①.5 的
/// `terminate_automation_bound_to_entity`（plan/session 绑定占用者，失败阻断
/// 删除）互补：本入口是 source 引用面的**尽力失效**——失败只 warn（不得阻断
/// 级联删除本身），CAS 竞态重读收敛有限重试。
fn invalidate_automation_referencing_entity(app_paths: &ProductAppPaths, scope: &EntityScope) {
    let Ok(enrollment) =
        crate::product::issue_automation_store::IssueAutomationStore::new(app_paths.clone())
            .get(&scope.project_id, &scope.issue_id)
    else {
        return;
    };
    let Some(enrollment) = enrollment else {
        return;
    };
    if !enrollment.enabled || enrollment.terminated_at.is_some() {
        return;
    }
    let references_entity = match scope.kind {
        DeletionEntityKind::Story => enrollment
            .source
            .stories
            .iter()
            .any(|reference| reference.id == scope.entity_id),
        DeletionEntityKind::Design => enrollment
            .source
            .designs
            .iter()
            .any(|reference| reference.id == scope.entity_id),
        // Plan 绑定占用面由 ①.5 `terminate_automation_bound_to_entity` 覆盖。
        DeletionEntityKind::Plan => false,
    };
    if !references_entity {
        return;
    }
    let mut current = enrollment;
    for _ in 0..3 {
        let request = crate::product::models::automation::EnrollmentTerminationRequest {
            command_id: format!(
                "entity-delete-terminate-{}-r{}",
                current.enrollment_id, current.policy_revision
            ),
            expected_enrollment_id: current.enrollment_id.clone(),
            expected_policy_revision: current.policy_revision,
            reason: format!(
                "referenced entity deleted: {}/{}",
                scope.kind.path_segment(),
                scope.entity_id
            ),
        };
        match crate::product::issue_automation_store::IssueAutomationStore::new(app_paths.clone())
            .terminate_plan(&scope.project_id, &scope.issue_id, request)
        {
            Ok(_) => return,
            // 记录消失或已被并行终止/禁用：失效目标已达成（幂等）。
            Err(crate::product::models::automation::EnrollmentError::NotFound) => return,
            Err(crate::product::models::automation::EnrollmentError::Conflict { .. }) => {
                // CAS 竞态：重读收敛——已失效即返回，仍 enabled 换新 revision 重试。
                match crate::product::issue_automation_store::IssueAutomationStore::new(
                    app_paths.clone(),
                )
                .get(&scope.project_id, &scope.issue_id)
                {
                    Ok(Some(next)) if next.enabled && next.terminated_at.is_none() => {
                        current = next;
                        continue;
                    }
                    _ => return,
                }
            }
            Err(error) => {
                // 尽力失效：只 warn，不阻断级联删除本身。
                tracing::warn!(
                    automation_enrollment_id = %current.enrollment_id,
                    entity_kind = %scope.kind.path_segment(),
                    entity_id = %scope.entity_id,
                    error = %error,
                    "source 引用被删实体的自动化 enrollment 未能失效（删除继续）"
                );
                return;
            }
        }
    }
}

/// REQ-WIGA-VOID-01（tasks 4.1/4.2）：工作项组删除成功之后，把该 issue 绑定
/// 被删 plan 的自动化登记标成已作废（锁内命令：voided_at/void_reason、
/// revision+1、绑定前代进 previous、账本追加一条；不物理删除登记）。
///
/// 尽力收口：作废失败 MUST NOT 回滚已完成的删除（owner 裁决「删了就是
/// 全部都删了」），但 MUST NOT 静默吞掉——按结果记录可诊断日志（失败
/// warn、成功 info、无事可做 debug）。NotFound/AlreadyVoided/NotBoundToPlan
/// 都是幂等正常态（删 WIG 时无登记、重复删除重入、登记属于别的 plan）。
fn void_enrollment_after_plan_deletion(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
) {
    use crate::product::issue_automation_store::IssueAutomationStore;
    use crate::product::models::automation::VoidOutcome;
    match IssueAutomationStore::new(app_paths.clone()).void_enrollment_for_deleted_plan(
        project_id,
        issue_id,
        plan_id,
        "work_item_group_deleted",
    ) {
        Ok(VoidOutcome::Voided(enrollment)) => tracing::info!(
            automation_enrollment_id = %enrollment.enrollment_id,
            plan_id = %plan_id,
            voided_at = enrollment.voided_at.as_deref().unwrap_or_default(),
            "automation enrollment voided after work item group deletion"
        ),
        Ok(VoidOutcome::NotFound) | Ok(VoidOutcome::AlreadyVoided) => tracing::debug!(
            project_id = %project_id,
            issue_id = %issue_id,
            plan_id = %plan_id,
            "no enrollment to void after work item group deletion (idempotent)"
        ),
        Ok(VoidOutcome::NotBoundToPlan) => tracing::debug!(
            project_id = %project_id,
            issue_id = %issue_id,
            plan_id = %plan_id,
            "enrollment bound to another plan; not voided"
        ),
        Err(error) => tracing::warn!(
            project_id = %project_id,
            issue_id = %issue_id,
            plan_id = %plan_id,
            error = %error,
            "自动化登记未能随工作项组删除作废（删除已完成，不回滚）"
        ),
    }
}

/// 收集 issue 下绑定到指定实体集合的 workspace session id。必须在磁盘删除前调用：
/// 删除后 `list_workspace_sessions` 已读不到这些记录。
fn workspace_session_ids_for_entities(
    store: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    entity_ids: &[String],
) -> ApiResult<Vec<String>> {
    Ok(store
        .list_workspace_sessions(project_id, issue_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .filter(|session| entity_ids.contains(&session.entity_id))
        .map(|session| session.id)
        .collect())
}

/// 删除实体后驱逐其 workspace session 的内存 manager。registry 命中即复用
/// （socket.rs `get_or_create_and_attach` 不回读磁盘）：不驱逐则重新生成复用同 id
/// session 时，WS 会把删除前的旧 engine 状态（messages/artifact/stage）原样发给
/// 前端（v37 反馈 #3：story spec 删不掉、重新生成仍旧错误态）。
pub(crate) async fn evict_workspace_session_managers(state: &WebAppState, session_ids: &[String]) {
    for session_id in session_ids {
        state.workspace_sessions.remove(session_id).await;
    }
}

/// 产品层级联/取代删除失败 → 稳定 ApiError 码。
///
/// - `Busy` → `entity_busy`（进行中或同 scope writer 争用；取代路径据此
///   fail-closed：不建立新 plan）。
/// - `ConfirmationRequired` → `deletion_confirmation_required`。
/// - `CodingWorkspaceExists*` → `coding_workspace_exists`（用户面删除语义）。
/// - `EntityReferenced` → `entity_referenced`。
/// - `InvalidTargetRepository` → `entity_target_repository_invalid`。
pub(crate) fn cascade_delete_api_error(error: CascadeDeleteError) -> ApiError {
    use crate::product::entity_deletion_cascade::CascadeDeleteError;
    match error {
        CascadeDeleteError::Store(inner) => product_store_api_error(inner),
        CascadeDeleteError::Busy => ApiError::runtime(
            "entity_busy",
            "实体存在进行中的运行或并发写入，请等待收口后重试删除",
            json!({}),
        ),
        CascadeDeleteError::InvalidTargetRepository {
            target_repository_id,
        } => ApiError::runtime(
            "entity_target_repository_invalid",
            "owned work item target repository is not a logical id",
            json!({ "target_repository_id": target_repository_id }),
        ),
    }
}

pub(crate) async fn delete_work_item_with_cleanup(
    app_paths: &ProductAppPaths,
    store: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    work_item_id: &str,
) -> ApiResult<()> {
    if schema_v2_plan_for_work_item(app_paths, store, project_id, issue_id, work_item_id)
        .map_err(product_store_api_error)?
        .is_some()
    {
        return Err(ApiError::validation(
            "schema_v2_group_delete_required",
            "schema v2 work items must be deleted through their work item plan",
        ));
    }
    // work item 级门禁：存在 coding attempt 则拒绝，要求先删 coding workspace。
    // 覆盖 legacy plan 遍历删除下属 work item、以及独立 DELETE /work-items/{id} 两个入口。
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    if let Some(attempt) = coding_store
        .list_attempts_for_work_item(project_id, issue_id, work_item_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .next()
    {
        return Err(coding_workspace_exists_for_work_item_error(
            work_item_id,
            &attempt.id,
        ));
    }
    let work_item = store
        .list_work_items(project_id, issue_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .find(|work_item| work_item.id == work_item_id)
        .ok_or_else(|| {
            product_store_api_error(ProductStoreError::NotFound {
                kind: "work_item",
                id: work_item_id.to_string(),
            })
        })?;
    let repository = find_repository(app_paths, project_id, &work_item.repository_id)?;
    let attempts = coding_store
        .list_attempts_for_work_item(project_id, issue_id, work_item_id)
        .map_err(product_store_api_error)?;
    for attempt in attempts {
        let attempt = abort_attempt_if_active(&coding_store, attempt)?;
        cleanup_coding_attempt_workspace(&repository, &attempt).await?;
    }
    coding_store
        .delete_attempts_for_work_item(project_id, issue_id, work_item_id)
        .map_err(product_store_api_error)?;
    store
        .delete_work_item(project_id, issue_id, work_item_id)
        .map_err(product_store_api_error)?;
    Ok(())
}

fn schema_v2_plan_lineage(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
) -> Result<Option<crate::product::models::WorkItemPlanLineage>, ProductStoreError> {
    let revision_store = WorkItemRevisionStore::new(app_paths.clone());
    match revision_store.get_plan_lineage(project_id, issue_id, plan_id) {
        Ok(lineage) => Ok(Some(lineage)),
        Err(ProductStoreError::NotFound {
            kind: "work_item_plan_lineage",
            ..
        }) => Ok(None),
        Err(error) => Err(error),
    }
}

fn schema_v2_plan_for_work_item(
    app_paths: &ProductAppPaths,
    store: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    work_item_id: &str,
) -> Result<Option<crate::product::models::WorkItemPlanLineage>, ProductStoreError> {
    let revision_store = WorkItemRevisionStore::new(app_paths.clone());
    for plan in store.list_issue_work_item_plans(project_id, issue_id)? {
        let Some(lineage) = schema_v2_plan_lineage(app_paths, project_id, issue_id, &plan.id)?
        else {
            continue;
        };
        let active_revision_id = lineage.active_revision_id.as_deref().ok_or_else(|| {
            ProductStoreError::IdentityMismatch {
                kind: "runtime_binding_missing",
                id: plan.id.clone(),
            }
        })?;
        let revision =
            revision_store.get_plan_revision(project_id, issue_id, &plan.id, active_revision_id)?;
        if revision.work_item_bindings.contains_key(work_item_id) {
            return Ok(Some(lineage));
        }
    }
    Ok(None)
}
