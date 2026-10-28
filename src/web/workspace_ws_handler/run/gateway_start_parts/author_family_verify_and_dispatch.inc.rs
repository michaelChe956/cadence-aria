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
/// 的节点绑定为 recovery `fresh_node_id`(仅 FreshRunning 且未绑定;kind
/// 判别式守卫)。best-effort:并发让位/节点未建时不阻塞 dispatch——
/// 成功收口(`PlanSplitRunContext::finalize_launch_after_success`)按
/// conversation last_node 补全;失败只 `warn`。
async fn lc_bind_fresh_node_after_spawn(
    store: &crate::product::lifecycle_store::LifecycleStore,
    run_context: &ProviderRunContext,
    run_kind: &ProviderRunKind,
    recovery: &crate::product::models::LcResumeRecoveryRecord,
) {
    use crate::product::models::LcResumeRecoveryState;

    if recovery.state != LcResumeRecoveryState::FreshRunning {
        return;
    }
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
            if let Some(recovery) = record.lc_resume_recovery.as_mut()
                && recovery.state == LcResumeRecoveryState::FreshRunning
                && recovery.fresh_node_id.is_none()
            {
                recovery.fresh_node_id = Some(node_id.clone());
                recovery.updated_at = now.clone();
                record.updated_at = now;
                crate::product::json_store::write_json(&record_path, &record)?;
            }
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
