// P08b run-bound 身份与 durable launch 纪元（`PlanSplitRunContext` 单元）。
//
// 本文件由 `gateway_start.rs` 以 `include!` 引入**同一模块**（可见性与调用面
// 完全不变），只为控制 gateway 主文件长度——**纯搬移，无行为改动**。

/// Task 1b 段②:WS streaming Plan/split 的 run 身份绑定——caller 先 begin,
/// `start_work_item_plan_author` 消费它做 prepare 前 sink 绑定,成功 complete、
/// 失败 fail;retry 每次新 handle(计划冻结「WS Plan/split run identity」)。
///
/// B2b1(Task 5.2):除 run-bound 审计身份外,携带 `(project,issue,session,
/// provider)` 定位事实,供 prepared full 指纹 map 与 FreshRunning recovery
/// 的同 run 绑定/对账(§4.2)在 `start_work_item_plan_author`/收口面使用。
pub(crate) struct PlanSplitRunContext {
    pub handle: crate::product::work_item_split_engine::parse::WorkItemSplitProviderRunHandle,
    lifecycle: crate::product::lifecycle_store::LifecycleStore,
    project_id: String,
    issue_id: String,
    provider: crate::product::models::ProviderName,
    /// P08：构造时读取的**当前 fresh operation 身份快照**
    /// `(recovery_id, operation_id)`——私有、非 wire 字段（不序列化、不进
    /// 公共契约）。prepare 阶段据此核对「本 run 属于哪个 operation」，
    /// 避免旧 context 把自己的 seq 绑给新 operation。普通（非 fresh）run
    /// 为 `None`，保持原路径。
    fresh_operation: Option<(String, String)>,
}

/// P08：从真实 session record 捕获**当前 fresh operation 身份快照**
/// `(recovery_id, operation_id)`——仅当 recovery 为 FreshRunning 且当前
/// operation 存在 Running 条目时捕获；普通（非 fresh）run 为 `None`
/// （保持原路径，不替新 FreshRunning 代绑 seq）。
fn fresh_operation_snapshot_of(
    session: &crate::product::models::WorkspaceSessionRecord,
) -> Option<(String, String)> {
    use crate::product::models::{LcFreshOperationState, LcResumeRecoveryState};
    let recovery = session.lc_resume_recovery.as_ref()?;
    if recovery.state != LcResumeRecoveryState::FreshRunning {
        return None;
    }
    let operation_id = recovery.operation_id.as_ref()?;
    let operation = recovery
        .fresh_operations
        .iter()
        .find(|operation| &operation.operation_id == operation_id)?;
    if operation.state != LcFreshOperationState::Running {
        return None;
    }
    Some((recovery.recovery_id.clone(), operation_id.clone()))
}

/// P08：begin 构造时读取真实 session 捕获当前 fresh operation 身份——读取
/// 失败即 Err（不猜 operation ID，不静默降级为「无快照」）。
fn capture_fresh_operation_snapshot(
    lifecycle: &crate::product::lifecycle_store::LifecycleStore,
    project_id: &str,
    issue_id: &str,
    workspace_session_id: &str,
) -> Result<Option<(String, String)>, String> {
    let session_path = lifecycle
        .workspace_sessions_root(project_id, issue_id)
        .join(format!("{workspace_session_id}.json"));
    let session: crate::product::models::WorkspaceSessionRecord =
        crate::product::json_store::read_json(&session_path).map_err(|error| {
            format!("fresh operation snapshot read failed for {workspace_session_id}: {error}")
        })?;
    Ok(fresh_operation_snapshot_of(&session))
}

/// begin handle:分配 run-bound 身份(sink 绑定与审计文件 key 同源)。
pub(crate) fn begin_plan_split_run(
    lifecycle: &crate::product::lifecycle_store::LifecycleStore,
    project_id: &str,
    issue_id: &str,
    provider: &crate::product::models::ProviderName,
    workspace_session_id: &str,
) -> Result<PlanSplitRunContext, String> {
    // P08：构造时捕获真实 session 的当前 fresh operation 身份（读失败 Err）——
    // 先于 begin，避免捕获失败留下孤儿 split run 记录。
    let fresh_operation =
        capture_fresh_operation_snapshot(lifecycle, project_id, issue_id, workspace_session_id)?;
    let handle = lifecycle
        .begin_work_item_split_provider_run(project_id, issue_id, provider, workspace_session_id)
        .map_err(|error| error.to_string())?;
    Ok(PlanSplitRunContext {
        handle,
        lifecycle: lifecycle.clone(),
        project_id: project_id.to_string(),
        issue_id: issue_id.to_string(),
        provider: provider.clone(),
        fresh_operation,
    })
}

/// P08b:durable restore——仅凭已 begin 且经 StrictRoundBound 校验的冻结 handle
/// 恢复 run 绑定上下文(跨进程/重启后等场景)。
///
/// 只读校验、零分配副作用:① 会话记录(`handle.workspace_session_id` 定位,
/// id/project/issue/author_provider 逐字段相等);② 该 handle 的 `run.json`
/// (`provider_run_id` 与 `handle.run_ref` 不透明字符串相等——不解析猜路径,
/// `workspace_session_id`/`role_run_seq` 相等)。running 且未落 raw 输出收口时
/// 恢复同样合法(不读 structured_output);不 begin、不推进 seq、不追加审计、
/// 不收口,拒绝时也不修复盘上事实。
pub(crate) fn plan_split_run_for_handle(
    lifecycle: &crate::product::lifecycle_store::LifecycleStore,
    project_id: &str,
    issue_id: &str,
    provider: &crate::product::models::ProviderName,
    handle: crate::product::work_item_split_engine::parse::WorkItemSplitProviderRunHandle,
) -> Result<PlanSplitRunContext, String> {
    let reject = |step: &str, detail: String| -> String {
        format!("plan_split_run_for_handle: {step}: {detail}")
    };

    let session_path = lifecycle
        .workspace_sessions_root(project_id, issue_id)
        .join(format!("{}.json", handle.workspace_session_id));
    let session: crate::product::models::WorkspaceSessionRecord =
        crate::product::json_store::read_json(&session_path).map_err(|error| {
            reject(
                "workspace_session_read",
                format!("{}: {error}", session_path.display()),
            )
        })?;
    if session.id != handle.workspace_session_id {
        return Err(reject(
            "workspace_session_identity",
            format!("{} != handle {}", session.id, handle.workspace_session_id),
        ));
    }
    if session.project_id != project_id || session.issue_id != issue_id {
        return Err(reject(
            "workspace_session_scope",
            format!(
                "{}/{} != {project_id}/{issue_id}",
                session.project_id, session.issue_id
            ),
        ));
    }
    if &session.author_provider != provider {
        return Err(reject(
            "workspace_session_provider",
            format!("{:?} != {:?}", session.author_provider, provider),
        ));
    }

    let run_path = lifecycle
        .work_item_split_provider_run_dir(&handle)
        .map_err(|error| reject("split_run_path", error.to_string()))?
        .join("run.json");
    let run: serde_json::Value =
        crate::product::json_store::read_json(&run_path).map_err(|error| {
            reject("split_run_read", format!("{}: {error}", run_path.display()))
        })?;
    let provider_run_id = run
        .get("provider_run_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            reject(
                "split_run_record",
                format!("missing provider_run_id in {}", run_path.display()),
            )
        })?
        .to_string();
    if provider_run_id != handle.run_ref {
        return Err(reject(
            "split_run_identity",
            format!(
                "provider_run_id {provider_run_id} != handle.run_ref {}",
                handle.run_ref
            ),
        ));
    }
    let run_session_id = run
        .get("workspace_session_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            reject(
                "split_run_record",
                format!("missing workspace_session_id in {}", run_path.display()),
            )
        })?;
    if run_session_id != handle.workspace_session_id {
        return Err(reject(
            "split_run_session",
            format!("{run_session_id} != handle {}", handle.workspace_session_id),
        ));
    }
    let run_role_run_seq = run
        .get("role_run_seq")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            reject(
                "split_run_record",
                format!("missing role_run_seq in {}", run_path.display()),
            )
        })?;
    if run_role_run_seq != handle.role_run_seq {
        return Err(reject(
            "split_run_seq",
            format!("{run_role_run_seq} != handle {}", handle.role_run_seq),
        ));
    }

    Ok(PlanSplitRunContext {
        handle,
        lifecycle: lifecycle.clone(),
        project_id: project_id.to_string(),
        issue_id: issue_id.to_string(),
        provider: provider.clone(),
        // P08：恢复路径已读真实 session——直接据此捕获当前 fresh operation 身份。
        fresh_operation: fresh_operation_snapshot_of(&session),
    })
}

/// B2b1(Task 5.2,设计 §4.1/§4.2):LC plan run 的 durable launch 纪元
/// 事实——全部落 `WorkspaceSessionRecord`(排它锁内 read-modify-write,
/// 与 lifecycle_store 写方同一文件锁约定)。
impl PlanSplitRunContext {
    /// 会话 record 路径(与 `LifecycleStore` 派生同源:issue 生命周期根下
    /// `workspace-sessions/{session_id}.json`)。
    fn workspace_session_path(&self) -> std::path::PathBuf {
        self.lifecycle
            .workspace_sessions_root(&self.project_id, &self.issue_id)
            .join(format!("{}.json", self.handle.workspace_session_id))
    }

    /// 锁内 read-modify-write;record 身份不符 fail-closed。
    fn with_session_record_mut(
        &self,
        mutate: impl FnOnce(
            &mut crate::product::models::WorkspaceSessionRecord,
        ) -> Result<(), crate::product::json_store::ProductStoreError>,
    ) -> Result<(), crate::product::json_store::ProductStoreError> {
        let path = self.workspace_session_path();
        let record_path = path.clone();
        crate::product::coding_attempt_store::locking::with_exclusive_lock(&path, move || {
            let mut record: crate::product::models::WorkspaceSessionRecord =
                crate::product::json_store::read_json(&record_path)?;
            if record.id != self.handle.workspace_session_id {
                return Err(
                    crate::product::json_store::ProductStoreError::IdentityMismatch {
                        kind: "workspace_session",
                        id: self.handle.workspace_session_id.clone(),
                    },
                );
            }
            mutate(&mut record)?;
            record.updated_at = chrono::Utc::now().to_rfc3339();
            crate::product::json_store::write_json(&record_path, &record)
        })
    }

    /// §4.2.1 prepared-map-before-start:同源 validated policy 冻结 digest 在
    /// session spawn 前按 `(role=Author, provider, role_run_seq)` 持久化
    /// (basis=FullLcProjectionV2;native/node 在 Prepared 阶段留空,不伪造);
    /// P08:同锁内**先核当前 operation 身份**——以构造时快照的
    /// `(recovery_id, operation_id)` 为准(recovery_id/当前 operation/FreshRunning/
    /// 条目 Running 全等),再把 `role_run_seq` 双向绑定到该 operation 条目与
    /// root `fresh_role_run_seq`(None→Some;同值幂等;异值冲突)。失配即
    /// `lc_fresh_operation_binding` 冲突上浮且零写(指纹尚未落盘),绝不把旧
    /// context 的 seq 绑给新 operation;普通(非 fresh)context 快照 None,
    /// 保持原普通路径不代绑。
    /// 既有同 (role,provider,seq) 记录原样保留(幂等,不降级不复活)。
    pub(crate) fn persist_prepared_launch_identity(
        &self,
        fingerprint_digest: &str,
    ) -> Result<(), crate::product::json_store::ProductStoreError> {
        use crate::product::models::{
            LcFreshOperationState, LcResumeRecoveryState, LogicalLaunchBasis,
            LogicalLaunchFingerprintRecord, LogicalLaunchFingerprintState, ProviderConversationRole,
        };

        let role = ProviderConversationRole::Author;
        let provider = self.provider.clone();
        let role_run_seq = self.handle.role_run_seq;
        let now = chrono::Utc::now().to_rfc3339();
        self.with_session_record_mut(move |record| {
            // P08：fresh 绑定先核「构造时快照的当前 operation」身份——失配即冲突
            // 上浮（零写：指纹尚未落盘），绝不把旧 context 的 seq 绑给新 operation。
            if let Some((expected_recovery_id, expected_operation_id)) =
                self.fresh_operation.clone()
            {
                let binding_conflict = || {
                    crate::product::json_store::ProductStoreError::Conflict {
                        kind: "lc_fresh_operation_binding".to_string(),
                        id: expected_operation_id.clone(),
                    }
                };
                let recovery = record
                    .lc_resume_recovery
                    .as_mut()
                    .ok_or_else(|| binding_conflict())?;
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
                    .ok_or_else(|| binding_conflict())?;
                if operation.state != LcFreshOperationState::Running {
                    return Err(binding_conflict());
                }
                // item 与 root 双向绑定同一 seq：None→Some；同值幂等；异值冲突。
                match operation.role_run_seq {
                    Some(bound) if bound != role_run_seq => {
                        return Err(binding_conflict());
                    }
                    Some(_) => {}
                    None => {
                        operation.role_run_seq = Some(role_run_seq);
                        operation.updated_at = now.clone();
                    }
                }
                match recovery.fresh_role_run_seq {
                    Some(bound) if bound != role_run_seq => {
                        return Err(binding_conflict());
                    }
                    Some(_) => {}
                    None => {
                        recovery.fresh_role_run_seq = Some(role_run_seq);
                    }
                }
                recovery.updated_at = now.clone();
            }
            let exists = record.logical_launch_fingerprints.iter().any(|entry| {
                entry.role == role
                    && entry.provider == provider
                    && entry.role_run_seq == role_run_seq
            });
            if !exists {
                record
                    .logical_launch_fingerprints
                    .push(LogicalLaunchFingerprintRecord {
                        schema_version: 1,
                        basis: LogicalLaunchBasis::FullLcProjectionV2,
                        role: role.clone(),
                        provider: provider.clone(),
                        role_run_seq,
                        // Prepared 阶段未绑定节点/native——不伪造虚身份;成功收口
                        // (`finalize_launch_after_success`)从真实 Completed 落的
                        // conversation ref 补 Active 绑定。
                        node_id: String::new(),
                        native_session_id: None,
                        fingerprint_digest: fingerprint_digest.to_string(),
                        // cwd/readable/target 等 planning 访问维度均在冻结指纹
                        // 内(B0a2 `from_envelope` 五参域);full candidate 消费面
                        // 接入后由其 digest 覆写。
                        planning_access_fingerprint: fingerprint_digest.to_string(),
                        state: LogicalLaunchFingerprintState::Prepared,
                        created_at: now.clone(),
                        updated_at: now.clone(),
                    });
            }
            Ok(())
        })
    }

    /// §4.2.2/§4.3 成功收口:Prepared→Active(native id/节点从真实 Completed
    /// 落的 conversation ref 绑定;Superseded/Active 不覆写不复活),并把
    /// FreshRunning recovery(同 `fresh_role_run_seq`)对账为 Resolved——
    /// `fresh_native_session_id`/`fresh_node_id`(缺失回填)与 operation
    /// Completed 同锁落盘。失败只 `warn`(纪元失败方向=等待,非 fail-open)。
    pub(crate) fn finalize_launch_after_success(&self) {
        use crate::product::models::{
            LcFreshOperationState, LcResumeRecoveryState, LogicalLaunchFingerprintState,
            ProviderConversationRole,
        };

        let role = ProviderConversationRole::Author;
        let provider = self.provider.clone();
        let role_run_seq = self.handle.role_run_seq;
        let now = chrono::Utc::now().to_rfc3339();
        let result = self.with_session_record_mut(move |record| {
            let conversation = record.provider_conversations.iter().find(|conversation| {
                conversation.role == role && conversation.provider == provider
            });
            let native_session_id = conversation.and_then(|conversation| {
                let id = conversation.provider_session_id.trim();
                (!id.is_empty()).then(|| id.to_string())
            });
            let last_node_id = conversation
                .and_then(|conversation| conversation.last_node_id.as_deref())
                .filter(|node| !node.is_empty())
                .map(str::to_string);
            for entry in record.logical_launch_fingerprints.iter_mut() {
                if entry.role == role
                    && entry.provider == provider
                    && entry.role_run_seq == role_run_seq
                    && entry.state == LogicalLaunchFingerprintState::Prepared
                {
                    entry.state = LogicalLaunchFingerprintState::Active;
                    if let Some(native) = native_session_id.as_deref() {
                        entry.native_session_id = Some(native.to_string());
                    }
                    if let Some(node) = last_node_id.as_deref() {
                        entry.node_id = node.to_string();
                    }
                    entry.updated_at = now.clone();
                }
            }
            let recovery = record.lc_resume_recovery.as_mut().filter(|recovery| {
                recovery.state == LcResumeRecoveryState::FreshRunning
                    && recovery.fresh_role_run_seq == Some(role_run_seq)
            });
            if let Some(recovery) = recovery {
                recovery.state = LcResumeRecoveryState::Resolved;
                if recovery.fresh_native_session_id.is_none() {
                    recovery.fresh_native_session_id = native_session_id.clone();
                }
                if recovery.fresh_node_id.is_none() {
                    recovery.fresh_node_id = last_node_id.clone();
                }
                let operation_id = recovery.operation_id.clone();
                if let Some(operation_id) = operation_id.as_deref() {
                    for operation in recovery.fresh_operations.iter_mut() {
                        if operation.operation_id == operation_id {
                            operation.state = LcFreshOperationState::Completed;
                            operation.role_run_seq = Some(role_run_seq);
                            if operation.fresh_node_id.is_none() {
                                operation.fresh_node_id = last_node_id.clone();
                            }
                            operation.updated_at = now.clone();
                        }
                    }
                }
                recovery.updated_at = now.clone();
            }
            Ok(())
        });
        if let Err(error) = result {
            tracing::warn!(
                %error,
                session_id = %self.handle.workspace_session_id,
                role_run_seq = self.handle.role_run_seq,
                "finalize logical launch map/recovery after success failed (fail-safe: waiting)"
            );
        }
    }

    /// §4.3「running 失败」行收口:FreshRunning recovery(同
    /// `fresh_role_run_seq`)回 Waiting,operation 落 Failed(结果保留,同 ID
    /// 只 Replay,下一次显式点击用新 ID);fresh 绑定事实保留作对账证据。
    /// map 保持 Prepared(native 未绑定,resume 自然进纪元严格门等待)。
    /// 失败只 `warn`(等待方向 fail-safe)。
    pub(crate) fn reconcile_launch_failure(&self, reason: &str) {
        use crate::product::models::{LcFreshOperationState, LcResumeRecoveryState};

        let role_run_seq = self.handle.role_run_seq;
        let now = chrono::Utc::now().to_rfc3339();
        let reason_code = format!("lc_resume_fresh_run_failed: {reason}");
        let result = self.with_session_record_mut(move |record| {
            let Some(recovery) = record.lc_resume_recovery.as_mut() else {
                return Ok(());
            };
            if recovery.state != LcResumeRecoveryState::FreshRunning
                || recovery.fresh_role_run_seq != Some(role_run_seq)
            {
                return Ok(());
            }
            recovery.state = LcResumeRecoveryState::Waiting;
            let operation_id = recovery.operation_id.take();
            if let Some(operation_id) = operation_id.as_deref() {
                for operation in recovery.fresh_operations.iter_mut() {
                    if operation.operation_id == operation_id {
                        operation.state = LcFreshOperationState::Failed;
                        operation.reason_code = Some(reason_code.clone());
                        operation.updated_at = now.clone();
                    }
                }
            }
            recovery.updated_at = now.clone();
            Ok(())
        });
        if let Err(error) = result {
            tracing::warn!(
                %error,
                session_id = %self.handle.workspace_session_id,
                role_run_seq = self.handle.role_run_seq,
                "reconcile fresh launch failure back to waiting failed"
            );
        }
    }

    /// prepare 前 run-bound audit 上下文(sink=LifecycleStore 生产 sink)。
    pub(crate) fn audit_context(
        &self,
    ) -> crate::product::logical_codebase::provider_gateway::ProviderLaunchAuditContext {
        crate::product::logical_codebase::provider_gateway::ProviderLaunchAuditContext {
            workspace_session_id: self.handle.workspace_session_id.clone(),
            role_run_seq: self.handle.role_run_seq,
            audit_sink: std::sync::Arc::new(self.lifecycle.clone()),
        }
    }

    /// 成功收口:complete 消费已有 handle,不再 `save_work_item_split_provider_run`。
    pub(crate) fn complete(
        &self,
        prompt: &str,
        structured_output: &serde_json::Value,
    ) -> Result<(), String> {
        self.lifecycle
            .complete_work_item_split_provider_run(&self.handle, prompt, structured_output)
            .map_err(|error| error.to_string())
    }

    /// 失败收口:handle 以 status=failed 落盘(reason 可观测)。
    pub(crate) fn fail(&self, reason: &str) -> Result<(), String> {
        self.lifecycle
            .fail_work_item_split_provider_run(&self.handle, reason)
            .map_err(|error| error.to_string())
    }
}

/// P08「当前 operation 归属」RED 契约（prepare 面）：prepared launch 身份
/// 落盘时，seq 必须绑定到**构造时快照的当前 operation**——旧 context（快照
/// op-A）在记录已成 op-B 后不得绑定 seq、不得写 fingerprint（冲突上浮）；
/// 快照一致的当前 operation 才双向绑定 item/root；普通（非 fresh）context
/// 保持原路径（快照 None）。
#[cfg(test)]
mod fresh_operation_binding_prepare_tests {
    use super::fresh_operation_binding_tests::{
        operation, recovery_fixture, seed_recovery, session_fixture,
    };
    use super::*;
    use crate::product::json_store::ProductStoreError;
    use crate::product::models::{LcFreshOperationState, LcResumeRecoveryState};

    /// RED⑤：旧 context 快照（op-A）与记录当前 operation（op-B）不一致 →
    /// 冲突上浮且零写（无 fingerprint、无 seq 绑定）。
    #[test]
    fn fresh_operation_binding_prepare_rejects_stale_operation_snapshot() {
        let (_tmp, store, record) = session_fixture();
        let current = recovery_fixture(
            &record,
            LcResumeRecoveryState::FreshRunning,
            Some("op_a_0001"),
            vec![operation("op_a_0001", LcFreshOperationState::Running)],
        );
        seed_recovery(&store, &record, current);
        // 构造 context：此刻记录当前 operation ＝ op-A。
        let context = begin_plan_split_run(
            &store,
            &record.project_id,
            &record.issue_id,
            &record.author_provider,
            &record.id,
        )
        .expect("begin plan split run");
        let snapshot_operation = context
            .fresh_operation
            .as_ref()
            .map(|(_, operation_id)| operation_id.clone());
        assert_eq!(
            snapshot_operation.as_deref(),
            Some("op_a_0001"),
            "构造时必须快照当前 fresh operation 身份"
        );
        // 记录被新 operation 取代（op-B）。
        let latest = store
            .get_workspace_session(&record.id)
            .expect("latest record");
        let next = recovery_fixture(
            &latest,
            LcResumeRecoveryState::FreshRunning,
            Some("op_b_0001"),
            vec![
                operation("op_a_0001", LcFreshOperationState::Running),
                operation("op_b_0001", LcFreshOperationState::Running),
            ],
        );
        seed_recovery(&store, &latest, next);

        let error = context
            .persist_prepared_launch_identity("sha256:stale")
            .expect_err("旧 operation 快照不得绑定 seq");
        match error {
            ProductStoreError::Conflict { kind, .. } => {
                assert_eq!(kind, "lc_fresh_operation_binding");
            }
            other => panic!("期望 lc_fresh_operation_binding 冲突，实际: {other:?}"),
        }
        // 零写：fingerprint 未 push、root/item 未绑定 seq。
        let after = store
            .get_workspace_session(&record.id)
            .expect("record after stale prepare");
        assert!(
            after.logical_launch_fingerprints.is_empty(),
            "冲突前不得写 fingerprint"
        );
        let current = after.lc_resume_recovery.expect("recovery kept");
        assert_eq!(current.fresh_role_run_seq, None, "冲突前不得绑定 root seq");
        assert!(
            current
                .fresh_operations
                .iter()
                .all(|operation| operation.role_run_seq.is_none()),
            "冲突前不得绑定任何 operation 的 seq"
        );
    }

    /// RED⑥：快照与记录当前 operation 一致 → item 与 root 同时绑定真实 seq。
    #[test]
    fn fresh_operation_binding_prepare_binds_current_operation_seq() {
        let (_tmp, store, record) = session_fixture();
        let current = recovery_fixture(
            &record,
            LcResumeRecoveryState::FreshRunning,
            Some("op_b_0001"),
            vec![operation("op_b_0001", LcFreshOperationState::Running)],
        );
        seed_recovery(&store, &record, current);
        let context = begin_plan_split_run(
            &store,
            &record.project_id,
            &record.issue_id,
            &record.author_provider,
            &record.id,
        )
        .expect("begin plan split run");
        let snapshot_operation = context
            .fresh_operation
            .as_ref()
            .map(|(_, operation_id)| operation_id.clone());
        assert_eq!(
            snapshot_operation.as_deref(),
            Some("op_b_0001"),
            "构造时必须快照当前 fresh operation 身份"
        );

        context
            .persist_prepared_launch_identity("sha256:current")
            .expect("prepare current operation");

        let after = store
            .get_workspace_session(&record.id)
            .expect("record after prepare");
        let current = after.lc_resume_recovery.expect("recovery kept");
        assert_eq!(
            current.fresh_role_run_seq,
            Some(context.handle.role_run_seq),
            "root 必须绑定当前 operation 的 seq"
        );
        let bound = current
            .fresh_operations
            .iter()
            .find(|operation| operation.operation_id == "op_b_0001")
            .expect("current operation entry");
        assert_eq!(
            bound.role_run_seq,
            Some(context.handle.role_run_seq),
            "当前 operation 必须与 root 绑定同一 seq"
        );
        assert_eq!(
            after.logical_launch_fingerprints.len(),
            1,
            "prepared 指纹必须落盘"
        );
    }

    /// RED⑦ 守卫：普通（非 fresh）context 保持原路径——快照为 None、prepared
    /// 指纹照常落盘、recovery 不被创建。
    #[test]
    fn fresh_operation_binding_prepare_keeps_normal_context_path() {
        let (_tmp, store, record) = session_fixture();
        let context = begin_plan_split_run(
            &store,
            &record.project_id,
            &record.issue_id,
            &record.author_provider,
            &record.id,
        )
        .expect("begin plan split run");
        assert!(
            context.fresh_operation.is_none(),
            "普通 run 不得快照 fresh operation"
        );
        context
            .persist_prepared_launch_identity("sha256:normal")
            .expect("prepare normal run");
        let after = store
            .get_workspace_session(&record.id)
            .expect("record after prepare");
        assert_eq!(
            after.logical_launch_fingerprints.len(),
            1,
            "普通路径必须照常写 prepared 指纹"
        );
        assert!(
            after.lc_resume_recovery.is_none(),
            "普通路径不得创建 fresh recovery"
        );
    }
}
