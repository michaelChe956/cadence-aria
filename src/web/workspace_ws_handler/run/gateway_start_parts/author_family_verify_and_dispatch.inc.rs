/// B1（终审 P1，设计 §5.3 author 族行）：author 族 payload 与 durable 的
/// typed 对照——冻结四元组逐维度比对：
/// - 身份：entity/workspace_type 与 durable record 一致（会话换代即漂移）；
/// - author/reviewer：冻结 provider 与 durable record 一致（等待期换
///   provider 即漂移）；
/// - artifact：冻结 artifact_refs（version/hash）与 durable artifact 版本
///   一致（未冻结 version/hash 的引用跳过，不倒算——与 B1b 入场门 ⑦ 同
///   纪律，birth 位 `lc_recovery_business_context` 冻结空集）；
/// - feedback：ReviewerRevision/AuthorFeedback 冻结 original_feedback 与原
///   修订节点 durable revision_feedback 一致（双侧均在场才比对；等待期反馈
///   被替换即漂移）。
///   漂移 → `lc_resume_recovery_stale` 拒收回等待；store 读取失败按 store
///   unavailable 家族 typed 上浮。
fn verify_lc_fresh_author_family_payload(
    context: &crate::product::models::LcRecoveryRunContext,
    run_context: &ProviderRunContext,
) -> Result<(), LcFreshDispatchRejection> {
    use crate::product::lifecycle_store::LifecycleStore;
    use crate::product::models::LcRecoveryRunContext;

    let (business, frozen_feedback) = match context {
        LcRecoveryRunContext::Author { business, .. }
        | LcRecoveryRunContext::AuthorChoiceFollowup { business, .. } => (business, None),
        LcRecoveryRunContext::AuthorFeedback {
            business,
            original_feedback,
        }
        | LcRecoveryRunContext::ReviewerRevision {
            business,
            original_feedback,
        } => (business, original_feedback.clone()),
        _ => return Ok(()),
    };
    let stale = |message: String| LcFreshDispatchRejection {
        code: crate::product::lifecycle_store::LcFreshClaimOutcome::STALE_SUPERSEDED,
        message,
    };
    let store_unavailable = |label: &str, error: crate::product::json_store::ProductStoreError| {
        LcFreshDispatchRejection {
            code: crate::web::workspace_ws_handler::socket::LC_RESUME_STORE_UNAVAILABLE_CODE,
            message: format!("load {label} for author family fresh verify failed: {error}"),
        }
    };
    let store = LifecycleStore::new(run_context.app_paths.clone());
    let record = store
        .get_workspace_session(&run_context.session_id)
        .map_err(|error| store_unavailable("workspace session", error))?;
    if business.entity_id != record.entity_id {
        return Err(stale(format!(
            "author family fresh verify failed: payload entity {} != durable entity {}",
            business.entity_id, record.entity_id
        )));
    }
    if business.workspace_type != record.workspace_type {
        return Err(stale(format!(
            "author family fresh verify failed: payload workspace {:?} != durable workspace {:?}",
            business.workspace_type, record.workspace_type
        )));
    }
    if business.author_provider != record.author_provider {
        return Err(stale(format!(
            "author family fresh verify failed: payload author {:?} != durable author {:?}",
            business.author_provider, record.author_provider
        )));
    }
    if business.effective_reviewer_provider != record.reviewer_provider {
        return Err(stale(format!(
            "author family fresh verify failed: payload reviewer {:?} != durable reviewer {:?}",
            business.effective_reviewer_provider, record.reviewer_provider
        )));
    }
    if !business.artifact_refs.is_empty() {
        let versions = store
            .list_artifact_versions(&run_context.session_id)
            .map_err(|error| store_unavailable("artifact versions", error))?;
        for artifact_ref in &business.artifact_refs {
            let Some(frozen_version) = artifact_ref.version else {
                continue;
            };
            let Some(current) = versions
                .iter()
                .find(|candidate| candidate.version == frozen_version)
            else {
                return Err(stale(format!(
                    "author family fresh verify failed: frozen artifact version {frozen_version} disappeared from durable versions"
                )));
            };
            if let Some(frozen_hash) = artifact_ref.hash.as_deref()
                && author_family_artifact_digest(&current.payload).as_deref() != Some(frozen_hash)
            {
                return Err(stale(format!(
                    "author family fresh verify failed: frozen artifact version {frozen_version} content drifted"
                )));
            }
        }
    }
    if let (Some(frozen), Some(node_id)) = (
        frozen_feedback
            .as_deref()
            .filter(|feedback| !feedback.trim().is_empty()),
        business
            .original_node_id
            .as_deref()
            .filter(|node_id| !node_id.is_empty()),
    ) && let Ok(detail) = store.load_node_detail(&run_context.session_id, node_id)
        && let Some(durable) = detail
            .revision_feedback
            .as_deref()
            .filter(|feedback| !feedback.trim().is_empty())
        && durable.trim() != frozen.trim()
    {
        return Err(stale(format!(
            "author family fresh verify failed: frozen feedback drifted from durable revision node {node_id}"
        )));
    }
    Ok(())
}

/// B1（终审 P1）：recovery 冻结 artifact 引用的当前内容指纹——serde 规范
/// 序列化后 sha256（`sha256:<hex>`），与 socket.rs 入场门 ⑦ 的
/// `lc_artifact_payload_digest` 同式比对（内容变化即 stale，不复活旧 source）。
fn author_family_artifact_digest(
    payload: &crate::web::workspace_ws_types::ArtifactPayload,
) -> Option<String> {
    use sha2::Digest as _;
    let canonical = serde_json::to_string(payload).ok()?;
    Some(format!(
        "sha256:{:x}",
        sha2::Sha256::digest(canonical.as_bytes())
    ))
}

fn lc_touch_operation(
    record: &mut crate::product::models::LcResumeRecoveryRecord,
    operation_id: &str,
    mut mutate: impl FnMut(&mut crate::product::models::LcFreshOperationResult),
) {
    for operation in record.fresh_operations.iter_mut() {
        if operation.operation_id == operation_id {
            mutate(operation);
        }
    }
}

fn lc_resume_store_unavailable_frame(
    session_id: &str,
    error: &crate::product::json_store::ProductStoreError,
) -> crate::web::workspace_ws_types::WsOutMessage {
    crate::web::workspace_ws_types::WsOutMessage::ProtocolError {
        code: crate::web::workspace_ws_handler::socket::LC_RESUME_STORE_UNAVAILABLE_CODE
            .to_string(),
        message: format!("durable projection unavailable for session {session_id}: {error}"),
        context: None,
    }
}

/// B2b1(Task 5.2,5.1 移交项):fresh spawn 认领成功后,把 manager 活动 run
/// 的节点绑定为 recovery `fresh_node_id`(kind 判别式守卫)。P08:绑定只认
/// dispatch 快照里的**当前 operation**——锁内重读后核 recovery_id/当前
/// operation/FreshRunning/条目 Running,再把同一真实节点双向绑定到该
/// operation 条目与 root(同值幂等;异值冲突);身份不符即冲突上浮且零写
/// (不猜 ID、不改旧 operation)。
/// best-effort:并发让位/节点未建时不阻塞 dispatch——
/// 成功收口(`PlanSplitRunContext::finalize_launch_after_success`)按
/// conversation last_node 补全;失败只 `warn`。
async fn lc_bind_fresh_node_after_spawn(
    store: &crate::product::lifecycle_store::LifecycleStore,
    run_context: &ProviderRunContext,
    run_kind: &ProviderRunKind,
    recovery: &crate::product::models::LcResumeRecoveryRecord,
) {
    use crate::product::models::{LcFreshOperationState, LcResumeRecoveryState};

    if recovery.state != LcResumeRecoveryState::FreshRunning {
        return;
    }
    // P08：绑定只认「dispatch 快照里的当前 operation」——无 operation_id 即不猜
    // ID、不写盘（不把旧 operation 的 node 绑到当前 operation）。
    let Some(expected_operation_id) = recovery.operation_id.clone() else {
        return;
    };
    let expected_recovery_id = recovery.recovery_id.clone();
    let Some(node_id) = run_context
        .manager
        .active_run()
        .await
        .filter(|active| std::mem::discriminant(&active.kind) == std::mem::discriminant(run_kind))
        .and_then(|active| active.node_id)
    else {
        return;
    };
    let session_id = run_context.session_record.id.clone();
    let path = store
        .workspace_sessions_root(
            &run_context.session_record.project_id,
            &run_context.session_record.issue_id,
        )
        .join(format!("{session_id}.json"));
    let now = chrono::Utc::now().to_rfc3339();
    let record_path = path.clone();
    let result =
        crate::product::coding_attempt_store::locking::with_exclusive_lock(&path, move || {
            let mut record: crate::product::models::WorkspaceSessionRecord =
                crate::product::json_store::read_json(&record_path)?;
            if record.id != session_id {
                return Err(
                    crate::product::json_store::ProductStoreError::IdentityMismatch {
                        kind: "workspace_session",
                        id: session_id.clone(),
                    },
                );
            }
            // P08：锁内重读后按 dispatch 快照核当前 operation——身份不符即冲突
            // 上浮且零写（不猜 ID、不改旧 operation）。
            let binding_conflict = || crate::product::json_store::ProductStoreError::Conflict {
                kind: "lc_fresh_operation_binding",
                id: expected_operation_id.clone(),
            };
            let recovery = record
                .lc_resume_recovery
                .as_mut()
                .ok_or_else(binding_conflict)?;
            if recovery.recovery_id != expected_recovery_id
                || recovery.state != LcResumeRecoveryState::FreshRunning
                || recovery.operation_id.as_deref() != Some(expected_operation_id.as_str())
            {
                return Err(binding_conflict());
            }
            let operation = recovery
                .fresh_operations
                .iter_mut()
                .find(|operation| operation.operation_id == expected_operation_id)
                .ok_or_else(binding_conflict)?;
            if operation.state != LcFreshOperationState::Running {
                return Err(binding_conflict());
            }
            // item 与 root 双向绑定同一真实节点：None→Some；同值幂等；异值冲突。
            let mut changed = false;
            if matches!(&operation.fresh_node_id, Some(bound) if bound != &node_id) {
                return Err(binding_conflict());
            }
            if operation.fresh_node_id.is_none() {
                operation.fresh_node_id = Some(node_id.clone());
                operation.updated_at = now.clone();
                changed = true;
            }
            if matches!(&recovery.fresh_node_id, Some(bound) if bound != &node_id) {
                return Err(binding_conflict());
            }
            if recovery.fresh_node_id.is_none() {
                recovery.fresh_node_id = Some(node_id.clone());
                changed = true;
            }
            if !changed {
                return Ok(());
            }
            recovery.updated_at = now.clone();
            record.updated_at = now;
            crate::product::json_store::write_json(&record_path, &record)?;
            Ok(())
        });
    if let Err(error) = result {
        tracing::warn!(
            %error,
            session_id = %run_context.session_id,
            "bind fresh node id after spawn failed (finalization backfills it)"
        );
    }
}

/// §4.3 prepare 失败行收口：operation failed（稳定 reason code）、
/// recovery 回 Waiting（同 ID 仍 Replay、下一次显式点击用新 ID）、typed
/// 拒帧上浮请求连接；零 spawn。
/// B2b3（Task 5.4）：臂内 fresh 消费面（PlanRevision retained/redo 对照
/// 失败等 spawn 后拒收）复用同收口——pub(crate) 供 run 家族臂体调用。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn lc_fail_fresh_operation_to_waiting(
    store: &crate::product::lifecycle_store::LifecycleStore,
    session_id: &str,
    outbound_tx: &tokio::sync::mpsc::Sender<crate::web::workspace_ws_handler::OutboundControl>,
    expected: &crate::product::models::WorkspaceSessionRecord,
    recovery: &crate::product::models::LcResumeRecoveryRecord,
    operation_id: &str,
    rejection: &LcFreshDispatchRejection,
) {
    use crate::product::models::{LcFreshOperationState, LcResumeRecoveryState};
    let now = chrono::Utc::now().to_rfc3339();
    let mut next = recovery.clone();
    next.state = LcResumeRecoveryState::Waiting;
    next.operation_id = None;
    next.updated_at = now.clone();
    lc_touch_operation(&mut next, operation_id, |operation| {
        operation.state = LcFreshOperationState::Failed;
        operation.reason_code = Some(rejection.code.to_string());
        operation.updated_at = now.clone();
    });
    match store.compare_and_save_lc_resume_state(expected, next) {
        // B2b1(5.1 审查收窄,Main 裁决附带项):CAS 败给并发写方(如
        // FreshRunning 推进/新漂移替换)不是 store 不可用——让位且不误报
        // store_unavailable typed 码。
        Err(crate::product::json_store::ProductStoreError::Conflict { .. }) => {
            tracing::debug!(
                session_id,
                operation_id,
                "lc fresh fail-path CAS lost to a concurrent writer; conceding"
            );
            return;
        }
        Err(error) => {
            let _ = crate::web::workspace_ws_handler::send_json_outbound(
                outbound_tx,
                &lc_resume_store_unavailable_frame(session_id, &error),
            )
            .await;
            return;
        }
        Ok(_) => {}
    }
    let _ = crate::web::workspace_ws_handler::send_json_outbound(
        outbound_tx,
        &crate::web::workspace_ws_types::WsOutMessage::ProtocolError {
            code: rejection.code.to_string(),
            message: format!(
                "lc resume fresh dispatch for operation {operation_id} rejected: {}",
                rejection.message
            ),
            context: Some(serde_json::json!({
                "recovery_id": recovery.recovery_id,
                "operation_id": operation_id,
            })),
        },
    )
    .await;
}

/// B2a（Task 5.1）：decisions `claim_lc_fresh` 受理后的 dispatch 入口。
/// 幂等：仅推进 FreshReserved（Waiting=无受理、FreshRunning/Resolved=已
/// 推进、同 ID Replay/Busy/Stale 归 socket 入站门与 store claim）。
/// 推进与启动之间以整记录 CAS single-flight；spawn 采用 ClaimIfIdle（fresh
/// 语义不 supersede 无关 run，§4.3 原业务 run/turn 保持）。
pub(crate) async fn dispatch_lc_fresh_recovery_run(
    run_context: ProviderRunContext,
    outbound_tx: tokio::sync::mpsc::Sender<crate::web::workspace_ws_handler::OutboundControl>,
) {
    use crate::product::lifecycle_store::LifecycleStore;
    use crate::product::models::{LcFreshOperationState, LcResumeRecoveryState};
    use crate::web::workspace_ws_handler::socket::LC_RESUME_RECOVERY_BUSY_CODE;

    let store = LifecycleStore::new(run_context.app_paths.clone());
    let latest = match store.get_workspace_session(&run_context.session_id) {
        Ok(record) => record,
        Err(error) => {
            let _ = crate::web::workspace_ws_handler::send_json_outbound(
                &outbound_tx,
                &lc_resume_store_unavailable_frame(&run_context.session_id, &error),
            )
            .await;
            return;
        }
    };
    let Some(recovery) = latest.lc_resume_recovery.clone() else {
        return;
    };
    if recovery.state != LcResumeRecoveryState::FreshReserved {
        return;
    }
    let Some(operation_id) = recovery.operation_id.clone() else {
        return;
    };
    match lc_fresh_dispatch_kind(&recovery, &run_context).await {
        Ok(run_kind) => {
            let now = chrono::Utc::now().to_rfc3339();
            let mut next = recovery.clone();
            next.state = LcResumeRecoveryState::FreshRunning;
            next.updated_at = now.clone();
            lc_touch_operation(&mut next, &operation_id, |operation| {
                operation.state = LcFreshOperationState::Running;
                operation.updated_at = now.clone();
            });
            // 整记录 CAS:并发 dispatch 只有一个赢(§5.1 single-flight)。
            // B2b1(5.1 审查收窄):Conflict=输给并发写方(静默让位);其余
            // 错误才是 store 不可用(typed 上浮,不吞写失败)。
            let saved = match store.compare_and_save_lc_resume_state(&latest, next) {
                Ok(saved) => saved,
                Err(crate::product::json_store::ProductStoreError::Conflict { .. }) => return,
                Err(error) => {
                    let _ = crate::web::workspace_ws_handler::send_json_outbound(
                        &outbound_tx,
                        &lc_resume_store_unavailable_frame(&run_context.session_id, &error),
                    )
                    .await;
                    return;
                }
            };
            let saved_recovery = saved
                .lc_resume_recovery
                .clone()
                .unwrap_or_else(|| recovery.clone());
            // spawn 必须观察 CAS **之后**的状态：`run_context.session_record` 是
            // dispatch 入口快照（FreshReserved），而臂内 SC 轮身份锚读的是由该
            // 快照构建的引擎镜像。不在此更新，就会把已提交的 FreshRunning 读回成
            // FreshReserved，进而以 `sc_failure_fresh_operation_not_running` 拒启。
            let mut run_context = run_context;
            run_context.session_record = saved.clone();
            let spawned = super::spawn_provider_run_claiming_idle(
                run_context.clone(),
                run_kind.clone(),
                outbound_tx.clone(),
            )
            .await;
            if spawned == Ok(true) {
                // B2b1(5.1 移交项):dispatch 绑 fresh_node_id——spawn 认领后
                // manager 活动 run 的节点(best-effort:rebuild 类 run 节点在
                // 臂内新建,此处可为 None,由成功收口按 conversation last_node
                // 补全;kind 判别式守卫,不误绑无关 run)。
                lc_bind_fresh_node_after_spawn(&store, &run_context, &run_kind, &saved_recovery)
                    .await;
            }
            match spawned {
                Ok(true) => {}
                // 认领窗口被占/run 面拒启:按 prepare 失败收口(operation
                // failed、recovery 回 Waiting),typed 上浮——不残留无 run
                // 的 FreshRunning。
                Ok(false) => {
                    lc_fail_fresh_operation_to_waiting(
                        &store,
                        &run_context.session_id,
                        &outbound_tx,
                        &saved,
                        &saved_recovery,
                        &operation_id,
                        &LcFreshDispatchRejection {
                            code: LC_RESUME_RECOVERY_BUSY_CODE,
                            message: "session holds an active run; explicit fresh stays waiting"
                                .to_string(),
                        },
                    )
                    .await;
                }
                Err(message) => {
                    lc_fail_fresh_operation_to_waiting(
                        &store,
                        &run_context.session_id,
                        &outbound_tx,
                        &saved,
                        &saved_recovery,
                        &operation_id,
                        &LcFreshDispatchRejection {
                            code: LC_RESUME_RECOVERY_BUSY_CODE,
                            message: format!("fresh run spawn failed: {message}"),
                        },
                    )
                    .await;
                }
            }
        }
        Err(rejection) => {
            lc_fail_fresh_operation_to_waiting(
                &store,
                &run_context.session_id,
                &outbound_tx,
                &latest,
                &recovery,
                &operation_id,
                &rejection,
            )
            .await;
        }
    }
}

/// P08「当前 operation 归属」RED 契约（node 绑定面）：fresh spawn 后的
/// node 绑定必须落在「dispatch 快照里的当前 operation」上——快照 operation
/// 与 durable 记录当前 operation 不一致时零写（不猜 ID、不改旧
/// operation）；一致时 root 与当前 operation 双向绑定同一真实 node。
///
/// 同模块另提供 prepare 面（`PlanSplitRunContext`）复用的真实 store 夹具。
#[cfg(test)]
mod fresh_operation_binding_tests {
    use super::*;
    use crate::product::lifecycle_store::{CreateWorkspaceSessionInput, LifecycleStore};
    use crate::product::models::{
        LcFreshOperationResult, LcFreshOperationState, LcRecoveryBusinessContext,
        LcRecoveryRunContext, LcResumeRecoveryRecord, LcResumeRecoveryState,
        ProviderConversationRole, ProviderName, WorkspaceSessionRecord, WorkspaceType,
    };

    pub(super) const RECOVERY_ID: &str = "recovery_fresh_binding_0001";
    pub(super) const OLD_NODE_ID: &str = "timeline_node_old_0001";
    pub(super) const FRESH_NODE_ID: &str = "timeline_node_fresh_0001";

    /// 真实 store 会话夹具（durable record 落盘，断言一律回读真实 record）。
    pub(super) fn session_fixture() -> (tempfile::TempDir, LifecycleStore, WorkspaceSessionRecord) {
        let tmp = tempfile::tempdir().unwrap();
        let store = LifecycleStore::new(crate::product::app_paths::ProductAppPaths::new(
            tmp.path().join(".aria"),
        ));
        let record = store
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "plan_0001".to_string(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Fake,
                reviewer_provider: Some(ProviderName::Fake),
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            })
            .expect("create workspace session");
        (tmp, store, record)
    }

    /// fresh operation 历史条目（未绑定物理 run）。
    pub(super) fn operation(
        operation_id: &str,
        state: LcFreshOperationState,
    ) -> LcFreshOperationResult {
        LcFreshOperationResult {
            operation_id: operation_id.to_string(),
            state,
            fresh_node_id: None,
            reason_code: None,
            role_run_seq: None,
            accepted_at: "2026-11-13T00:00:00Z".to_string(),
            updated_at: "2026-11-13T00:00:00Z".to_string(),
        }
    }

    /// fresh recovery 夹具（`operation_id`＝记录当前 operation；`operations`
    /// ＝历史 operation 列表）。
    pub(super) fn recovery_fixture(
        record: &WorkspaceSessionRecord,
        state: LcResumeRecoveryState,
        operation_id: Option<&str>,
        operations: Vec<LcFreshOperationResult>,
    ) -> LcResumeRecoveryRecord {
        LcResumeRecoveryRecord {
            schema_version: 1,
            recovery_id: RECOVERY_ID.to_string(),
            failed_provider_node_id: OLD_NODE_ID.to_string(),
            reason_code: "resume_fingerprint_mismatch".to_string(),
            role: ProviderConversationRole::Author,
            provider: record.author_provider.clone(),
            superseded_native_session_id: Some("native_old_0001".to_string()),
            previous_fingerprint_digest: Some("sha256:old".to_string()),
            previous_session_status: record.status.clone(),
            previous_stage: "revision".to_string(),
            run_context: LcRecoveryRunContext::ReviewerRevision {
                business: LcRecoveryBusinessContext {
                    entity_id: record.entity_id.clone(),
                    plan_id: None,
                    workspace_type: record.workspace_type.clone(),
                    business_repository_physical_id: None,
                    business_repository_logical_id: None,
                    business_checkout_id: None,
                    business_repository_path: None,
                    source_story_ids: Vec::new(),
                    source_design_ids: Vec::new(),
                    plan_options_ref: None,
                    artifact_refs: Vec::new(),
                    review_refs: Vec::new(),
                    author_provider: record.author_provider.clone(),
                    effective_reviewer_provider: record.reviewer_provider.clone(),
                    original_node_id: Some(OLD_NODE_ID.to_string()),
                    original_role_run_seq: None,
                },
                original_feedback: None,
            },
            state,
            operation_id: operation_id.map(|id| id.to_string()),
            fresh_node_id: None,
            fresh_native_session_id: None,
            fresh_role_run_seq: None,
            fresh_operations: operations,
            detected_at: "2026-11-13T00:00:00Z".to_string(),
            updated_at: "2026-11-13T00:00:00Z".to_string(),
        }
    }

    /// 把 recovery 落盘（整记录 CAS，真实 store 路径）。
    pub(super) fn seed_recovery(
        store: &LifecycleStore,
        record: &WorkspaceSessionRecord,
        recovery: LcResumeRecoveryRecord,
    ) -> WorkspaceSessionRecord {
        store
            .compare_and_save_lc_resume_state(record, recovery)
            .expect("seed lc resume recovery")
    }

    /// 真实 run 上下文夹具：manager 登记一个带 node 的活动 run。
    fn run_context_fixture(
        tmp: &tempfile::TempDir,
        store: &LifecycleStore,
        record: &WorkspaceSessionRecord,
    ) -> ProviderRunContext {
        let (event_tx, _event_rx) =
            tokio::sync::mpsc::channel::<crate::product::workspace_engine::EngineEvent>(8);
        let checkpoint_store = Arc::new(crate::product::checkpoint_store::CheckpointStore::new(
            tmp.path().join("checkpoints"),
        ));
        let engine = Arc::new(tokio::sync::Mutex::new(
            crate::product::workspace_engine::WorkspaceEngine::new(
                checkpoint_store,
                event_tx,
                crate::product::workspace_engine::WorkspaceSession::from_record(record.clone()),
            ),
        ));
        ProviderRunContext::test_fixture(
            Arc::new(crate::cross_cutting::provider_registry::ProviderRegistry::new()),
            engine,
            crate::web::state::WorkspaceRunRegistry::default(),
            record.id.clone(),
            store.app_paths(),
            record.clone(),
        )
    }

    /// RED③：快照 operation 与记录当前 operation 不一致 → 零写（不把旧
    /// 快照的 node 绑到当前 operation，也不改任何 operation 条目）。
    #[tokio::test]
    async fn fresh_operation_binding_node_bind_rejects_stale_operation_snapshot() {
        let (tmp, store, record) = session_fixture();
        let current = recovery_fixture(
            &record,
            LcResumeRecoveryState::FreshRunning,
            Some("op_current_0001"),
            vec![
                operation("op_old_0001", LcFreshOperationState::Running),
                operation("op_current_0001", LcFreshOperationState::Running),
            ],
        );
        seed_recovery(&store, &record, current);
        // dispatch 持有的快照：旧 operation。
        let snapshot = recovery_fixture(
            &record,
            LcResumeRecoveryState::FreshRunning,
            Some("op_old_0001"),
            vec![operation("op_old_0001", LcFreshOperationState::Running)],
        );
        let run_context = run_context_fixture(&tmp, &store, &record);
        let run_kind = ProviderRunKind::WorkItemPlanBatch;
        run_context
            .manager
            .try_start_run_if_idle(run_kind.clone(), Some(FRESH_NODE_ID.to_string()))
            .await
            .expect("claim idle run")
            .expect("run claimed");

        lc_bind_fresh_node_after_spawn(&store, &run_context, &run_kind, &snapshot).await;

        let after = store
            .get_workspace_session(&record.id)
            .expect("record after stale bind");
        let current = after.lc_resume_recovery.expect("recovery kept");
        assert_eq!(
            current.fresh_node_id, None,
            "旧 operation 快照不得把 node 绑到当前 operation"
        );
        assert!(
            current
                .fresh_operations
                .iter()
                .all(|operation| operation.fresh_node_id.is_none()),
            "任何 operation 条目都不得被旧快照改写 node"
        );
    }

    /// RED④：快照 operation 与记录当前 operation 一致 → root 与当前
    /// operation 双向绑定同一真实 node；旧 operation 不受影响；同值重放
    /// 幂等。
    #[tokio::test]
    async fn fresh_operation_binding_node_bind_binds_current_operation() {
        let (tmp, store, record) = session_fixture();
        let current = recovery_fixture(
            &record,
            LcResumeRecoveryState::FreshRunning,
            Some("op_current_0001"),
            vec![
                operation("op_old_0001", LcFreshOperationState::Running),
                operation("op_current_0001", LcFreshOperationState::Running),
            ],
        );
        seed_recovery(&store, &record, current);
        let snapshot = recovery_fixture(
            &record,
            LcResumeRecoveryState::FreshRunning,
            Some("op_current_0001"),
            vec![operation("op_current_0001", LcFreshOperationState::Running)],
        );
        let run_context = run_context_fixture(&tmp, &store, &record);
        let run_kind = ProviderRunKind::WorkItemPlanBatch;
        run_context
            .manager
            .try_start_run_if_idle(run_kind.clone(), Some(FRESH_NODE_ID.to_string()))
            .await
            .expect("claim idle run")
            .expect("run claimed");

        lc_bind_fresh_node_after_spawn(&store, &run_context, &run_kind, &snapshot).await;

        let after = store
            .get_workspace_session(&record.id)
            .expect("record after bind");
        let current = after.lc_resume_recovery.expect("recovery kept");
        assert_eq!(
            current.fresh_node_id.as_deref(),
            Some(FRESH_NODE_ID),
            "root 镜像必须绑定真实节点"
        );
        let bound = current
            .fresh_operations
            .iter()
            .find(|operation| operation.operation_id == "op_current_0001")
            .expect("current operation entry");
        assert_eq!(
            bound.fresh_node_id.as_deref(),
            Some(FRESH_NODE_ID),
            "当前 operation 必须与 root 双向绑定同一节点"
        );
        let old = current
            .fresh_operations
            .iter()
            .find(|operation| operation.operation_id == "op_old_0001")
            .expect("old operation entry");
        assert_eq!(
            old.fresh_node_id, None,
            "旧 operation 的 node 归属不得被改写"
        );

        // 同值重放幂等（零新增绑定、零改写）。
        let before = std::fs::read(
            store
                .workspace_sessions_root(&record.project_id, &record.issue_id)
                .join(format!("{}.json", record.id)),
        )
        .expect("read record bytes");
        lc_bind_fresh_node_after_spawn(&store, &run_context, &run_kind, &snapshot).await;
        let after = std::fs::read(
            store
                .workspace_sessions_root(&record.project_id, &record.issue_id)
                .join(format!("{}.json", record.id)),
        )
        .expect("read record bytes");
        assert_eq!(before, after, "同值重放必须幂等（不改记录字节）");
    }
}
