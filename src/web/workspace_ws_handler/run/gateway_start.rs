//! T10a:聚合规划 author 启动点经 gateway 的统一 helper。
//!
//! 逻辑会话(已注入 `LogicalCodebaseProviderGateway`)的 planning author run 经
//! gateway `validate` + `start_streaming` 启动并留 audit;传统单仓/未注入 gateway
//! 时保持原 `provider.start` 路径(Legacy 零变化)。
//!
//! Task 2.8(REQ-PLN-01/07,planning snapshot 贯穿)——provider spawn cwd 恒为
//! gateway 冻结的 canonical 聚合根;B2b1(Task 5.2,设计 §3/REQ-PLN-08)把
//! target 同步收敛到 E 分离 resolver 的 native anchor:
//! - envelope target 恒 `PolicyTarget::aggregate_root(canonical root)`——
//!   native anchor 不再从 business working_dir/成员 checkout 推导
//!   (`resolve_plan_author_launch` 消费 `workspace_native_launch_anchor_for_session`
//!   做 fail-closed 共同守卫;成员业务身份/材料路径不入 envelope 授权面);
//! - cwd 与 target 同根(root),business 材料仍由 `workspace_repository_for_session`
//!   原语义解析(prompt/compile 消费面零变化)。
//!
//! T3(裁决 A):planning author 的 `ValidatedSessionLaunchPolicy` 前移到 prompt 构建
//! 之前 resolve(`resolve_plan_author_launch`),使 work_item_split_engine 的 outline/
//! draft prompt 能按 `RoutingReferenceContext` 注入 Logical 路由引用;已 resolve 的
//! policy 原样透传给 `start_work_item_plan_author` 复用,避免二次 validate。
//!
//! C-2:launch request 的 `ProviderRef` 由 `session.author_provider` 经集中映射
//! `ProviderRef::from_provider_name` 派生(fail-closed:Pi/KimiCode 等无 gateway
//! 真实 dialect 的 provider 在此显式失败,不再硬编码 ClaudeCode 静默替换)。
//! `cap_managed_snapshot` capability ref 约定不变;Codex 的 REQ-ENV-05 路由阻断
//! 仍由 gateway 路由级硬门施加。
//!
//! B2b1(Task 5.2,设计 §4.2 prepared-map-before-start):Logical 分支携带 split
//! run handle 时,prepared full 指纹在 session spawn 前按 (role,provider,
//! role_run_seq) 持久化(I/O 失败零 start);成功收口把 map 转 Active 并绑定
//! 真实 native id,同 run 对账 FreshRunning recovery 的 Resolved 终态。

use std::path::PathBuf;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_adapter::{
    PROVIDER_ERROR_STDERR_TAIL_BYTES, ProviderAdapterError,
};
use crate::cross_cutting::session_launch::ValidatedStreamingProviderInput;
use crate::cross_cutting::streaming_provider::{
    ProviderSession, StreamingProviderAdapter, StreamingProviderInput,
};
use crate::product::cadence_skills::routing_reference::{
    RoutingReferenceContext, routing_reference_context_from_policy,
};
use crate::product::logical_codebase::{
    LogicalCodebaseProviderGateway, PolicyTarget, ProviderGatewayError, ProviderRef,
    SessionLaunchRequest, SessionPolicyAction, ValidatedSessionLaunchPolicy,
};
use crate::product::models::ProviderName;
use crate::product::workspace_engine::{ProviderRunKind, WorkspaceEngine};
use crate::web::workspace_ws_handler::ProviderRunContext;

/// 逻辑会话经 gateway 启动所需的已解析 launch 信息。
/// `Clone`:F2-B SC compile 教学重驱需在同一 run 内复用已 resolve 的 launch
/// (避免二次 gateway validate);字段均为不可变快照,克隆无副作用。
#[derive(Clone)]
pub(crate) struct LogicalPlanLaunch {
    pub gateway: Arc<LogicalCodebaseProviderGateway>,
    pub project_id: String,
    /// provider spawn cwd:gateway 冻结的 canonical 聚合根(Task 2.8,
    /// REQ-PLN-07 唯一来源;禁止以成员 checkout/target worktree 覆盖)。
    /// B2b1(Task 5.2,REQ-PLN-08):该 root 同时是 envelope target 的唯一
    /// 来源——native anchor 不从 business working_dir/成员身份推导。
    pub working_dir: PathBuf,
    /// session 配置的 author provider(C-2:launch request provider ref 的唯一
    /// 来源,勿从 input.provider_type 或 UI 默认推导)。
    pub author_provider: ProviderName,
}

impl LogicalPlanLaunch {
    /// 组装 planning 只读 `SessionLaunchRequest`(cwd/target 同为 canonical
    /// 聚合根,见文件头 B2b1 说明);provider ref 由 `author_provider` 经集中
    /// 映射派生,不支持的 provider 显式返回 `UnsupportedCapability` 而非
    /// 静默回退 Claude(C-2)。
    pub(crate) fn planning_request(&self) -> Result<SessionLaunchRequest, ProviderGatewayError> {
        // B2b1(REQ-PLN-08):LC 规划 native anchor 恒聚合根视图——target 不绑
        // 成员 logical/checkout 身份(checkout_id/logical_repository_id 为空),
        // 成员业务身份/材料路径不作 envelope 授权面(design §3.2)。
        let target = PolicyTarget::aggregate_root(self.working_dir.clone());

        Ok(SessionLaunchRequest {
            project_id: self.project_id.clone(),
            provider: ProviderRef::from_provider_name(
                &self.author_provider,
                "cap_managed_snapshot",
            )?,
            action: SessionPolicyAction::PlanningReadOnly,
            target,
            // Task 2.8(REQ-PLN-01/07):cwd 恒为 canonical root(≠成员
            // target),由结构体字面量显式提供——`SessionLaunchRequest::planning`
            // 的 cwd==target 默认仅适用单仓形态。
            working_directory: self.working_dir.clone(),
            readable_roots: vec![self.working_dir.clone()],
            writable_roots: Vec::new(),
            config_artifact_ref: "sha256:managed-config-artifact".to_string(),
        })
    }

    /// 经 gateway 校验并冻结为 `ValidatedSessionLaunchPolicy`。
    pub(crate) fn validate(&self) -> Result<ValidatedSessionLaunchPolicy, ProviderGatewayError> {
        self.gateway.validate(self.planning_request()?)
    }
}

/// 已冻结政策的逻辑会话启动:launch + validated policy 成对保存,供 prompt 构建
/// (取 `RoutingReferenceContext::Logical`)与 provider 启动(复用 validated policy)共用。
#[derive(Clone)]
pub(crate) struct ValidatedPlanLaunch {
    pub launch: LogicalPlanLaunch,
    pub validated: ValidatedSessionLaunchPolicy,
}

/// planning author 启动选择:Legacy(直接 `provider.start`)或 Logical(经 gateway)。
#[derive(Clone)]
pub(crate) enum PlanAuthorLaunch {
    Legacy,
    Logical(Box<ValidatedPlanLaunch>),
}

impl PlanAuthorLaunch {
    /// 由此启动派生 prompt 注入用的路由引用上下文:Legacy 用 `Legacy`,
    /// Logical 用 validated policy 的 envelope 字段构造 `LogicalPolicyReference`。
    pub(crate) fn routing_context(&self) -> RoutingReferenceContext {
        match self {
            PlanAuthorLaunch::Legacy => RoutingReferenceContext::Legacy,
            PlanAuthorLaunch::Logical(plan) => {
                routing_reference_context_from_policy(&plan.validated)
            }
        }
    }
}

/// 组装逻辑会话 launch(gateway 已注入时);非逻辑会话/未注入 gateway 时 `None`。
/// Task 2.8/REQ-PLN-07:cwd 唯一来源是 gateway 冻结的 canonical root
/// (`authority_root()`,双工厂 root assertion 与登记/聚合投影一致)——不读
/// `session.repository_path`(成员 checkout 是 business 材料锚,不迁移入
/// native launch)。
pub(crate) fn logical_plan_launch_for(engine: &WorkspaceEngine) -> Option<LogicalPlanLaunch> {
    let gateway = engine.logical_provider_gateway()?;
    let (project_id, _business_target_worktree) = engine.logical_planning_launch()?;
    Some(LogicalPlanLaunch {
        // Task 2.8:cwd 唯一来源是 gateway 冻结的 canonical root——不读
        // session.repository_path(成员 checkout 只作 business 材料),杜绝
        // 成员 cwd fallback(REQ-PLN-07)。
        working_dir: gateway.authority_root().to_path_buf(),
        gateway,
        project_id,
        // C-2:provider 身份唯一来源于 session 配置。
        author_provider: engine.session().author_provider.clone(),
    })
}

/// B2b1(Task 5.2,设计 §3/REQ-PLN-08):plan native launch 消费 E 分离
/// resolver 的 native_launch_anchor——共同守卫(selection 失效/focus 越界/
/// manifest/selection 不成对)fail-closed,native launch 不得绕过。Logical
/// 路由下 anchor 即聚合根视图,其根与 gateway 冻结的 canonical authority
/// root 同源(双工厂 root assertion);envelope target 由 gateway 权威派生,
/// 不在此二次推导。
fn ensure_plan_native_anchor_resolved(engine: &WorkspaceEngine) -> Result<(), String> {
    use crate::product::workspace_repository::workspace_native_launch_anchor_for_session;

    let Some(store) = engine.lifecycle_store.as_ref() else {
        return Err(format!(
            "logical plan launch requires a persistent lifecycle store (session {})",
            engine.session.session_id
        ));
    };
    let record = store
        .get_workspace_session(&engine.session.session_id)
        .map_err(|error| error.to_string())?;
    let app_paths = store.app_paths();
    workspace_native_launch_anchor_for_session(&app_paths, store, &record)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// 在 prompt 构建之前 resolve 出 planning author 启动:非逻辑会话 → `Legacy`;
/// 逻辑会话 → 先消费 E native anchor(fail-closed),再经 gateway `validate`
/// 冻结 policy 并返回 `Logical`。gateway 校验失败映射为
/// `ProviderAdapterError`,与启动路径的错误形态一致。
pub(crate) fn resolve_plan_author_launch(
    engine: &WorkspaceEngine,
) -> Result<PlanAuthorLaunch, ProviderAdapterError> {
    let Some(launch) = logical_plan_launch_for(engine) else {
        return Ok(PlanAuthorLaunch::Legacy);
    };
    if let Err(message) = ensure_plan_native_anchor_resolved(engine) {
        return Err(ProviderAdapterError::provider_unavailable(format!(
            "logical_plan_native_anchor_unavailable: {message}"
        )));
    }
    let validated = launch
        .validate()
        .map_err(|error| map_gateway_error_to_adapter(&error))?;
    Ok(PlanAuthorLaunch::Logical(Box::new(ValidatedPlanLaunch {
        launch,
        validated,
    })))
}

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
}

/// begin handle:分配 run-bound 身份(sink 绑定与审计文件 key 同源)。
pub(crate) fn begin_plan_split_run(
    lifecycle: &crate::product::lifecycle_store::LifecycleStore,
    project_id: &str,
    issue_id: &str,
    provider: &crate::product::models::ProviderName,
    workspace_session_id: &str,
) -> Result<PlanSplitRunContext, String> {
    let handle = lifecycle
        .begin_work_item_split_provider_run(project_id, issue_id, provider, workspace_session_id)
        .map_err(|error| error.to_string())?;
    Ok(PlanSplitRunContext {
        handle,
        lifecycle: lifecycle.clone(),
        project_id: project_id.to_string(),
        issue_id: issue_id.to_string(),
        provider: provider.clone(),
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
    /// 同锁内绑定 FreshRunning recovery 的 `fresh_role_run_seq`(§4.1 fresh
    /// run/node/native 绑定;仅 FreshRunning 且未绑定时推进,不覆写既成事实)。
    /// 既有同 (role,provider,seq) 记录原样保留(幂等,不降级不复活)。
    pub(crate) fn persist_prepared_launch_identity(
        &self,
        fingerprint_digest: &str,
    ) -> Result<(), crate::product::json_store::ProductStoreError> {
        use crate::product::models::{
            LcResumeRecoveryState, LogicalLaunchBasis, LogicalLaunchFingerprintRecord,
            LogicalLaunchFingerprintState, ProviderConversationRole,
        };

        let role = ProviderConversationRole::Author;
        let provider = self.provider.clone();
        let role_run_seq = self.handle.role_run_seq;
        let now = chrono::Utc::now().to_rfc3339();
        self.with_session_record_mut(move |record| {
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
            if let Some(recovery) = record.lc_resume_recovery.as_mut()
                && recovery.state == LcResumeRecoveryState::FreshRunning
                && recovery.fresh_role_run_seq.is_none()
            {
                recovery.fresh_role_run_seq = Some(role_run_seq);
                recovery.updated_at = now.clone();
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

/// Task 1b 段②生产臂:Logical 启动才分配 split run 身份(begin→Some);
/// Legacy(直连 `provider.start`)保持 `None`,run 记录与 sink 绑定零变化。
/// begin 失败(durable 存储故障)fail-closed 上浮。
pub(crate) fn begin_plan_split_run_if_logical(
    launch: &PlanAuthorLaunch,
    lifecycle: &crate::product::lifecycle_store::LifecycleStore,
    project_id: &str,
    issue_id: &str,
    provider: &ProviderName,
    workspace_session_id: &str,
) -> Result<Option<PlanSplitRunContext>, String> {
    match launch {
        PlanAuthorLaunch::Legacy => Ok(None),
        PlanAuthorLaunch::Logical(_) => begin_plan_split_run(
            lifecycle,
            project_id,
            issue_id,
            provider,
            workspace_session_id,
        )
        .map(Some),
    }
}

/// Task 1b 段②生产臂:驱动成功后的 run 收口——structured 输出可解析则
/// `complete`(存档该 run 的结构化产物),不可解析则 `fail`。收口落盘失败
/// 不阻断业务流(`tracing::warn`,与 `commit_rebuilt_snapshot_after_provider_start`
/// 同语义:split run 记录是审计身份,不是主产物)。
/// B2b1(Task 5.2,§4.2.2/§4.3):成功/失败各自完成 launch 纪元收口——
/// map Prepared→Active + FreshRunning recovery Resolved 对账 / 失败回 Waiting。
pub(crate) fn close_plan_split_run(
    run: Option<&PlanSplitRunContext>,
    prompt: &str,
    full_output: &str,
) {
    let Some(run) = run else { return };
    match crate::web::workspace_ws_handler::run::parse_work_item_split_structured_output(
        full_output,
    ) {
        Ok(structured_output) => {
            if let Err(error) = run.complete(prompt, &structured_output) {
                tracing::warn!(%error, "complete plan split provider run failed");
            }
            run.finalize_launch_after_success();
        }
        Err(message) => {
            let reason = format!("structured output parse failed: {message}");
            if let Err(error) = run.fail(&reason) {
                tracing::warn!(%error, "fail plan split provider run failed");
            }
            run.reconcile_launch_failure(&reason);
        }
    }
}

/// Task 1b 段②生产臂:Markdown 产物(无 structured sentinel,如 SC plan
/// markdown/human-gate 修订)的 run 收口——原始全文按 JSON 字符串存档。
/// B2b1(Task 5.2):同 `close_plan_split_run` 的 launch 纪元成功收口。
pub(crate) fn close_plan_split_run_with_markdown(
    run: Option<&PlanSplitRunContext>,
    prompt: &str,
    full_output: &str,
) {
    let Some(run) = run else { return };
    let structured_output = serde_json::Value::String(full_output.to_string());
    if let Err(error) = run.complete(prompt, &structured_output) {
        tracing::warn!(%error, "complete plan split provider run failed");
    }
    run.finalize_launch_after_success();
}

/// Task 1b 段②生产臂:驱动失败(run 未产出可用输出)的 run 收口。
/// B2b1(Task 5.2,§4.3):FreshRunning recovery 回 Waiting(operation Failed,
/// 结果保留),fresh 绑定事实保留作对账证据。
pub(crate) fn fail_plan_split_run(run: Option<&PlanSplitRunContext>, reason: &str) {
    let Some(run) = run else { return };
    if let Err(error) = run.fail(reason) {
        tracing::warn!(%error, "fail plan split provider run failed");
    }
    run.reconcile_launch_failure(reason);
}

/// 逻辑会话经 gateway 启动,否则原 `provider.start`。返回 `ProviderSession`。
///
/// `launch` 为 `Logical` 时复用已 resolve 的 validated policy 经
/// `gateway.start_streaming` 启动并留 audit;gateway 错误映射为
/// `ProviderAdapterError`(与 `drive_author_provider_session_via_gateway` 相同形态),
/// 使调用点后续对 `Err` 的处理方式与直接 `provider.start` 完全一致。
/// `launch` 为 `Legacy` 时原样透传 `provider.start(input, cancel)`(Legacy 零变化)。
///
pub(crate) async fn start_work_item_plan_author(
    launch: PlanAuthorLaunch,
    provider: Arc<dyn StreamingProviderAdapter>,
    input: StreamingProviderInput,
    cancel: CancellationToken,
    plan_split_run: Option<&PlanSplitRunContext>,
) -> Result<ProviderSession, ProviderAdapterError> {
    // 启动握手停滞界：adapter 的 `start` 在返回 `ProviderSession` 之前可能卡死
    // 在握手（ACP/JSON-RPC 无应答），而 F-19 系列看门狗**只在拿到 session 之后**
    // 的驱动循环内生效——握手期此前只受整 run 总预算 3h 兜底，现场形态即
    // 「37–40 分钟零 provider 事件、零 WS 帧」的静默等待。此处是该 path 的
    // 唯一物理启动口（七个 caller 共用），在该口收口一次即可覆盖 SC 单候选
    // 确认 run、SC 返修接力 run（HumanGateScManualRevision）、教学重驱与
    // legacy/draft/batch author 全部启动路径。取值与关系见
    // `PROVIDER_LAUNCH_STALL_TIMEOUT_PRODUCTION`。
    let stall_timeout = crate::product::workspace_engine::provider_launch_stall_timeout();
    match tokio::time::timeout(
        stall_timeout,
        start_work_item_plan_author_inner(launch, provider, input, cancel, plan_split_run),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => {
            // 稳定原因码 `provider_launch_stall` + provider 超时码（HardTimeout
            // killed）：随既有 Err 通道上浮——门修订轮由调用方
            // `fail_human_gate_turn(ProviderErr)` 收口（turn 终态、门保持开态
            // 可再操作），SC 生成轮由驱动入口 `finish_failed_run` 回落可重跑
            // 起点。launch future 已被 drop（adapter 的进程归属守卫随之收口），
            // 不留孤儿子进程。
            Err(ProviderAdapterError::timeout_with_details(
                format!(
                    "provider_launch_stall: provider 会话在 {} 秒内未完成启动握手（未产生任何 provider 动作），运行已由启动停滞界中止，可重新开始生成",
                    stall_timeout.as_secs()
                ),
                String::new(),
                String::new(),
                stall_timeout.as_millis() as u64,
            ))
        }
    }
}

async fn start_work_item_plan_author_inner(
    launch: PlanAuthorLaunch,
    provider: Arc<dyn StreamingProviderAdapter>,
    mut input: StreamingProviderInput,
    cancel: CancellationToken,
    plan_split_run: Option<&PlanSplitRunContext>,
) -> Result<ProviderSession, ProviderAdapterError> {
    let PlanAuthorLaunch::Logical(plan) = launch else {
        return provider.start(input, cancel).await;
    };
    let plan = *plan;
    // T6 fix-loop r2:分类下沉到 Logical 分支唯一物理启动口——七个已包
    // caller(WIP draft/batch/human-gate 修订、legacy outline 首发/
    // AutoRevision、SC 教学/主 spawn)与 followups 四处续驱(U4-U7)
    // 共用此口,不再依赖 caller 逐臂枚举;provider 取 launch 携带的
    // session author(与 envelope 冻结同源);Legacy 分支不经此面。
    let trust_provider = plan.launch.author_provider.clone();
    // Task 2.8(REQ-PLN-03,planning snapshot 贯穿):input 的独立 cwd 重绑
    // envelope 冻结的 canonical root——cwd/target 分离贯穿正常 run(worktree=
    // 成员 checkout)与 B3 StaleContext 重建 run(worktree=rebuilt.cwd=root),
    // spawn 前复验恒以 envelope root 为准,`input.working_dir` 保持 target
    // 语义原样透传(与 2.1 author 链 `provider_drive.rs` 的重绑同型)。
    input.working_directory = Some(plan.validated.envelope().working_directory.clone());
    // A1-b1(Bug45 批2 环境回退):plan/split 受信环境注入停止——子进程经
    // 底座(process_manager)继承宿主环境,caller input.env_vars 原样透传
    // (仅作显式覆盖层);prepared-map/gateway 冻结分流与 trust 分类不变。
    // Task 1b 段②:WS plan/split caller 携 run handle 时,经 gateway
    // `prepare_streaming_launch` 在 prepare 前绑定 run-bound sink(含无通用
    // tool_policy 角色)并以冻结接口组装 validated input;无 handle 的存量
    // 调用面保持原 validate 复用路径(后续 caller 迁移逐步收敛)。
    let validated_input = match plan_split_run {
        Some(run_ctx) => {
            // B2b1(Task 5.2,设计 §4.2.1 prepared-map-before-start):同源
            // validated policy 冻结指纹在 session spawn 前按 native/run 绑定
            // 持久化(prepared map),并绑定 FreshRunning recovery 的
            // fresh_role_run_seq;I/O 失败零 start(fail-closed)。
            run_ctx
                .persist_prepared_launch_identity(&plan.validated.fingerprint().digest)
                .map_err(|error| {
                    ProviderAdapterError::provider_unavailable(format!(
                        "logical_launch_prepared_map_unavailable: {error}"
                    ))
                })?;
            plan.launch
                .gateway
                .prepare_streaming_launch(
                    input,
                    plan.launch
                        .planning_request()
                        .map_err(|error| map_gateway_error_to_adapter(&error))?,
                    run_ctx.audit_context(),
                )
                .map_err(|error| map_gateway_error_to_adapter(&error))?
        }
        None => ValidatedStreamingProviderInput::new(input, plan.validated),
    };

    plan.launch
        .gateway
        .start_streaming(validated_input, cancel)
        .await
        .map_err(|error| {
            let mut mapped = map_gateway_error_to_adapter(&error);
            mapped.details = plan.launch.gateway.classify_launch_trust_failure_details(
                &error,
                &trust_provider,
                &mapped.details,
                crate::product::logical_codebase::provider_trust::ProviderTrustScenario::Generation,
            );
            mapped
        })
}

fn map_gateway_error_to_adapter(error: &ProviderGatewayError) -> ProviderAdapterError {
    // 诊断直通（claude×轻 握手谜团第 2 轮）:Adapter 变体不再丢弃 stderr 字段——
    // stderr 尾部（有界）并入 details（随驱动 Err → EngineEvent::Error → WS error
    // 消息上浮），stderr 字段同源保留；其余 gateway 校验错误维持 Display 文案不变。
    match error {
        ProviderGatewayError::Adapter(inner) => {
            let mut mapped = ProviderAdapterError::provider_unavailable(error.to_string());
            ProviderAdapterError::append_bounded_stderr_tail(
                &mut mapped.details,
                &inner.stderr,
                PROVIDER_ERROR_STDERR_TAIL_BYTES,
            );
            mapped.stderr = inner.stderr.clone();
            mapped
        }
        _ => ProviderAdapterError::provider_unavailable(error.to_string()),
    }
}

// ============================================================================
// B2a（Task 5.1，tasks.md 5.1 / 设计 §5.3）：显式 fresh claim 后的 tag 精确
// dispatch——claim（FreshReserved）→ run_context tag → ProviderRunKind 精确
// 对照；未知/非法/未接管 tag typed 拒（lc_resume_recovery_context_unsupported），
// 不回落通用 plan_author。native 启动唯一经 `start_work_item_plan_author`
// Logical 分支（B0 full prepare 消费；fresh 链 resume_provider_session_id=None，
// 旧 audit 只由 B4.1 资格过滤消费，不回捞续驱）。
// ============================================================================

/// §5.3 tag→ProviderRunKind 精确对照的结果：可直接组装的 kind，或需要
/// dispatch 时新解析 planning context 的 rebuild（仅 fresh 授权后用新
/// resolved snapshot）。
#[derive(Debug, Clone)]
pub(crate) enum LcFreshRunKind {
    Kind(ProviderRunKind),
    NeedsFreshPlanningContext,
}

/// typed 拒（`lc_resume_recovery_context_unsupported` 族）。
#[derive(Debug, Clone)]
pub(crate) struct LcFreshDispatchRejection {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

fn lc_context_unsupported(message: String) -> LcFreshDispatchRejection {
    LcFreshDispatchRejection {
        code: crate::web::workspace_ws_handler::socket::LC_RESUME_RECOVERY_CONTEXT_UNSUPPORTED_CODE,
        message,
    }
}

/// run_context tag 的稳定短名（typed 拒帧可观测文案用，不泄业务 payload）。
fn lc_recovery_context_tag_label(
    context: &crate::product::models::LcRecoveryRunContext,
) -> &'static str {
    use crate::product::models::LcRecoveryRunContext;
    match context {
        LcRecoveryRunContext::Author { .. } => "author",
        LcRecoveryRunContext::AuthorChoiceFollowup { .. } => "author_choice_followup",
        LcRecoveryRunContext::AuthorFeedback { .. } => "author_feedback",
        LcRecoveryRunContext::ReviewerRevision { .. } => "reviewer_revision",
        LcRecoveryRunContext::LegacyOutlineAuthor { .. } => "legacy_outline_author",
        LcRecoveryRunContext::LegacyOutlineRevision { .. } => "legacy_outline_revision",
        LcRecoveryRunContext::LegacyOutlineRebuild { .. } => "legacy_outline_rebuild",
        LcRecoveryRunContext::LegacyDraft { .. } => "legacy_draft",
        LcRecoveryRunContext::LegacyBatch { .. } => "legacy_batch",
        LcRecoveryRunContext::LegacyPlanRevision { .. } => "legacy_plan_revision",
        LcRecoveryRunContext::ScAuthor { .. } => "sc_author",
        LcRecoveryRunContext::ScManualRevision { .. } => "sc_manual_revision",
    }
}

/// B2a（Task 5.1，设计 §5.3）：RecoveryRecord tagged `run_context` → fresh
/// 显式启动的 `ProviderRunKind` 精确对照（纯映射，零副作用）。feedback 按
/// durable 冻结值原样透传（不重算/不丢）；非法 flow typed 拒，不回落
/// `work_item_plan_author_for_durable_flow` 通用兜底。
pub(crate) fn lc_fresh_run_kind_for_recovery_context(
    context: &crate::product::models::LcRecoveryRunContext,
) -> Result<LcFreshRunKind, LcFreshDispatchRejection> {
    use crate::product::models::LcRecoveryRunContext;
    use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;
    match context {
        LcRecoveryRunContext::Author {
            original_content, ..
        } => Ok(LcFreshRunKind::Kind(ProviderRunKind::Author {
            content: original_content.clone().unwrap_or_default(),
        })),
        LcRecoveryRunContext::AuthorChoiceFollowup {
            original_content, ..
        } => Ok(LcFreshRunKind::Kind(
            ProviderRunKind::AuthorChoiceFollowup {
                content: original_content.clone().unwrap_or_default(),
            },
        )),
        LcRecoveryRunContext::AuthorFeedback { .. }
        | LcRecoveryRunContext::ReviewerRevision { .. } => {
            Ok(LcFreshRunKind::Kind(ProviderRunKind::Revision))
        }
        LcRecoveryRunContext::LegacyOutlineAuthor { flow, .. } => {
            if !matches!(flow, WorkItemPlanFlowKind::Legacy) {
                return Err(lc_context_unsupported(format!(
                    "legacy_outline_author run_context carries non-legacy flow {flow:?}"
                )));
            }
            Ok(LcFreshRunKind::Kind(
                ProviderRunKind::WorkItemPlanLegacyAuthor,
            ))
        }
        LcRecoveryRunContext::LegacyOutlineRevision {
            normalized_feedback,
            ..
        } => Ok(LcFreshRunKind::Kind(
            ProviderRunKind::WorkItemPlanOutlineRevision {
                feedback: if normalized_feedback.trim().is_empty() {
                    None
                } else {
                    Some(normalized_feedback.clone())
                },
            },
        )),
        LcRecoveryRunContext::LegacyOutlineRebuild { .. } => {
            Ok(LcFreshRunKind::NeedsFreshPlanningContext)
        }
        LcRecoveryRunContext::LegacyDraft {
            pending_feedback, ..
        } => Ok(LcFreshRunKind::Kind(ProviderRunKind::WorkItemPlanDraft {
            feedback: pending_feedback.clone(),
        })),
        LcRecoveryRunContext::LegacyBatch { .. } => {
            Ok(LcFreshRunKind::Kind(ProviderRunKind::WorkItemPlanBatch))
        }
        LcRecoveryRunContext::LegacyPlanRevision {
            overall_feedback, ..
        } => Ok(LcFreshRunKind::Kind(
            ProviderRunKind::WorkItemPlanRevision {
                feedback: overall_feedback.clone(),
            },
        )),
        LcRecoveryRunContext::ScAuthor { flow, .. } => {
            if !matches!(flow, WorkItemPlanFlowKind::SingleCandidate) {
                return Err(lc_context_unsupported(format!(
                    "sc_author run_context carries non-single-candidate flow {flow:?}"
                )));
            }
            Ok(LcFreshRunKind::Kind(
                ProviderRunKind::WorkItemPlanSingleCandidateAuthor,
            ))
        }
        LcRecoveryRunContext::ScManualRevision { turn_id, .. } => {
            Ok(LcFreshRunKind::Kind(
                ProviderRunKind::HumanGateScManualRevision {
                    turn_id: turn_id.clone(),
                    // prompt 留空：臂内按 durable 同 turn 重装配（§5.3 同 turn
                    // 语义；不在此重算 prompt）。
                    prompt: String::new(),
                },
            ))
        }
    }
}

include!("gateway_start_parts/lc_fresh_payload_verify.inc.rs");

include!("gateway_start_parts/author_family_verify_and_dispatch.inc.rs");

#[cfg(test)]
mod tests {
    use super::*;

    /// 诊断直通（claude×轻 握手谜团第 2 轮）：workitem author 启动路径的 gateway
    /// 错误映射同样不得丢弃 adapter stderr——stderr 尾部（有界）并入 details，
    /// stderr 字段同源保留（随驱动 Err → EngineEvent::Error → WS error 消息上浮）。
    #[test]
    fn gateway_error_mapping_carries_bounded_stderr_tail() {
        let stderr_head = "NOISE_HEAD_MARKER".to_string() + &"c".repeat(700);
        let stderr = format!("{stderr_head}\nSENTINEL_WS_GW_STDERR_TAIL");
        let inner = ProviderAdapterError::parse_error(
            "claude policy session: handshake failed: claude policy handshake cancelled",
            String::new(),
            stderr,
        );
        let mapped = map_gateway_error_to_adapter(&ProviderGatewayError::Adapter(inner));
        assert!(
            mapped.details.contains("SENTINEL_WS_GW_STDERR_TAIL"),
            "gateway 映射应携带 adapter stderr 尾部，got: {}",
            mapped.details
        );
        assert!(
            !mapped.details.contains("NOISE_HEAD_MARKER"),
            "stderr 尾部应有界（丢头保尾），got: {}",
            mapped.details
        );
        assert!(
            mapped.stderr.contains("SENTINEL_WS_GW_STDERR_TAIL"),
            "stderr 字段应同源保留，got: {}",
            mapped.stderr
        );
    }

    /// SG6b：sc_manual_revision 显式 fresh dispatch 的结构授权窗口 join
    ///（recovery×turn×grant×attempt×prompt 纯核心判据）。
    fn sg06b_grant(
        turn_id: &str,
        phase: crate::product::models::workspace::StructuralGrantPhase,
        attempt_no: u32,
        prompt_identity: bool,
    ) -> crate::product::models::workspace::StructuralRevisionGrant {
        crate::product::models::workspace::StructuralRevisionGrant {
            grant_id: format!("structural_grant_{turn_id}"),
            authorization_hash: "f".repeat(64),
            turn_id: Some(turn_id.to_string()),
            command_id: Some(format!("cmd_{turn_id}")),
            candidate_source_revision_ref: None,
            plan_candidate_ir_ref: None,
            mechanical_report_ref: None,
            phase: Some(phase),
            attempt_no,
            prompt_ref: prompt_identity.then(|| format!("structural-prompt://{turn_id}")),
            prompt_digest: prompt_identity.then(|| "d".repeat(64)),
            instruction_digest: prompt_identity.then(|| "b".repeat(64)),
            failing_source_hash: Some("a".repeat(64)),
            review_identity: None,
            granted_at: "2026-10-14T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn sg_06b_sc_manual_fresh_grant_join_matrix() {
        use crate::product::lifecycle_store::LcFreshClaimOutcome;
        use crate::product::models::workspace::StructuralGrantPhase;

        let turn_id = "turn_sg6b_fresh";
        let command_id = "cmd_turn_sg6b_fresh";
        let stale_code = LcFreshClaimOutcome::STALE_SUPERSEDED;

        // 无结构窗口（无 grant）：原 fresh 路径不变。
        assert!(verify_sc_manual_fresh_grant_join_core(None, turn_id, command_id, 1).is_ok());

        // claim 绑定子 prompt 的合法 join：grant=Attempt2Claimed、完整
        // prompt 身份、恰消费下一 attempt（payload attempt 1 → grant 2）。
        let claimed = sg06b_grant(turn_id, StructuralGrantPhase::Attempt2Claimed, 2, true);
        assert!(
            verify_sc_manual_fresh_grant_join_core(Some(&claimed), turn_id, command_id, 1).is_ok(),
            "合法 claim join：显式 fresh 同 attempt 交付冻结子 prompt"
        );

        // 错 recovery（grant 绑定其它 turn）→ stale（0 start/预算不改写）。
        let mismatch = verify_sc_manual_fresh_grant_join_core(
            Some(&claimed),
            "turn_sg6b_other",
            command_id,
            1,
        )
        .expect_err("cross-turn join failure must be stale");
        assert_eq!(mismatch.code, stale_code);

        // 错 digest/ref（claim 无 prompt 身份）→ stale。
        let identity_missing =
            sg06b_grant(turn_id, StructuralGrantPhase::Attempt2Claimed, 2, false);
        let missing =
            verify_sc_manual_fresh_grant_join_core(Some(&identity_missing), turn_id, command_id, 1)
                .expect_err("claim without prompt identity must be stale");
        assert_eq!(missing.code, stale_code);

        // attempt 越界（grant 未消费下一 attempt）→ stale。
        let attempt_mismatch = sg06b_grant(turn_id, StructuralGrantPhase::Attempt2Claimed, 1, true);
        let overreach =
            verify_sc_manual_fresh_grant_join_core(Some(&attempt_mismatch), turn_id, command_id, 1)
                .expect_err("grant attempt must be exactly payload attempt + 1");
        assert_eq!(overreach.code, stale_code);

        // 窗口已 closed（终态/评审相位）→ stale。
        for phase in [
            StructuralGrantPhase::Reviewing,
            StructuralGrantPhase::Resolved,
            StructuralGrantPhase::SystemFault,
            StructuralGrantPhase::BusinessGate,
        ] {
            let closed = sg06b_grant(turn_id, phase, 2, true);
            let rejected =
                verify_sc_manual_fresh_grant_join_core(Some(&closed), turn_id, command_id, 1)
                    .expect_err("closed window must be stale");
            assert_eq!(rejected.code, stale_code);
        }

        // 未 claim 窗口（AuthorRunning 等无 prompt 身份）→ 原反馈 fresh 放行。
        let unclaimed = sg06b_grant(turn_id, StructuralGrantPhase::AuthorRunning, 0, false);
        assert!(
            verify_sc_manual_fresh_grant_join_core(Some(&unclaimed), turn_id, command_id, 1)
                .is_ok()
        );
    }

    // -----------------------------------------------------------------------
    // P08b(durable restore):已 begin 的 split run 句柄在 store 重建后必须能
    // 恢复同身份 `PlanSplitRunContext`——不重新分配 seq、不 begin、不写盘;
    // 跨 session/project/issue/provider 绑定与盘上 run/session 记录篡改一律
    // fail-closed;running 轮且无 raw 输出收口时恢复合法。
    // -----------------------------------------------------------------------
    fn p08b_fixture() -> (
        tempfile::TempDir,
        crate::product::lifecycle_store::LifecycleStore,
        String,
        String,
        ProviderName,
        crate::product::work_item_split_engine::parse::WorkItemSplitProviderRunHandle,
    ) {
        use crate::product::app_paths::ProductAppPaths;
        use crate::product::lifecycle_store::{CreateWorkspaceSessionInput, LifecycleStore};
        use crate::product::models::WorkspaceType;

        let tmp = tempfile::TempDir::new().expect("p08b temp root");
        let store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
        let session = store
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_p08b".to_string(),
                issue_id: "issue_p08b".to_string(),
                entity_id: "plan_p08b".to_string(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Codex,
                reviewer_provider: None,
                review_rounds: 0,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            })
            .expect("p08b session");
        let provider = session.author_provider.clone();
        let ctx = begin_plan_split_run(&store, "project_p08b", "issue_p08b", &provider, &session.id)
            .expect("p08b begin");
        (
            tmp,
            store,
            "project_p08b".to_string(),
            "issue_p08b".to_string(),
            provider,
            ctx.handle,
        )
    }

    #[test]
    fn p08b_restore_reuses_handle_identity_across_store_rebuild() {
        use crate::product::lifecycle_store::LifecycleStore;

        let (_tmp, store, project, issue, provider, handle) = p08b_fixture();
        let run_dir = store
            .work_item_split_provider_run_dir(&handle)
            .expect("p08b run dir");
        let run_path = run_dir.join("run.json");
        let session_path = store
            .workspace_sessions_root(&project, &issue)
            .join(format!("{}.json", handle.workspace_session_id));
        let run_before = std::fs::read(&run_path).expect("p08b run.json bytes");
        let session_before = std::fs::read(&session_path).expect("p08b session bytes");
        let seq_dirs_before = std::fs::read_dir(run_dir.parent().expect("p08b session dir"))
            .expect("p08b split runs dir")
            .count();

        // 仅凭 durable handle 在新 store 实例恢复:running 轮、无 raw 输出收口合法。
        let reopened = LifecycleStore::new(store.app_paths());
        let restored =
            plan_split_run_for_handle(&reopened, &project, &issue, &provider, handle.clone())
                .expect("p08b durable handle must restore without new allocation");

        assert_eq!(restored.handle, handle, "恢复句柄身份必须逐字段相同");
        assert_eq!(restored.project_id, project, "绑定 project 原样恢复");
        assert_eq!(restored.issue_id, issue, "绑定 issue 原样恢复");
        assert_eq!(restored.provider, provider, "绑定 provider 原样恢复");
        assert_eq!(
            restored.lifecycle.app_paths().root(),
            store.app_paths().root(),
            "恢复上下文必须克隆同一 lifecycle 根"
        );
        // 不 begin/不推进 seq/不写盘:run 记录、会话记录、run 目录集合逐字节不变。
        assert_eq!(std::fs::read(&run_path).expect("p08b run.json bytes"), run_before);
        assert_eq!(std::fs::read(&session_path).expect("p08b session bytes"), session_before);
        assert_eq!(
            std::fs::read_dir(run_dir.parent().expect("p08b session dir"))
                .expect("p08b split runs dir")
                .count(),
            seq_dirs_before,
            "restore 不得分配新 role_run_seq 目录"
        );
    }

    #[test]
    fn p08b_restore_rejects_tampered_bindings() {
        let (_tmp, store, project, issue, provider, handle) = p08b_fixture();
        let run_path = store
            .work_item_split_provider_run_dir(&handle)
            .expect("p08b run dir")
            .join("run.json");

        // ① 调用方绑定参数与 durable 事实不符:project/issue/provider 参数表。
        for (label, wrong_project, wrong_issue, wrong_provider) in [
            ("project", "project_p08b_other", issue.as_str(), provider.clone()),
            ("issue", project.as_str(), "issue_p08b_other", provider.clone()),
            ("provider", project.as_str(), issue.as_str(), ProviderName::ClaudeCode),
        ] {
            let rejected = plan_split_run_for_handle(
                &store,
                wrong_project,
                wrong_issue,
                &wrong_provider,
                handle.clone(),
            );
            assert!(rejected.is_err(), "绑定不符({label})必须拒绝");
        }

        // ② 盘上 run.json 的 role_run_seq 被改写:必须可见 Err 且不静默修复盘上事实。
        let run_original = std::fs::read(&run_path).expect("p08b run.json bytes");
        let mut run_json: serde_json::Value =
            serde_json::from_slice(&run_original).expect("p08b json");
        run_json["role_run_seq"] = serde_json::json!(handle.role_run_seq + 1);
        let run_tampered = serde_json::to_vec(&run_json).expect("p08b encode");
        std::fs::write(&run_path, &run_tampered).expect("p08b tamper run.json");
        assert!(
            plan_split_run_for_handle(&store, &project, &issue, &provider, handle.clone()).is_err(),
            "run.json role_run_seq 篡改必须拒绝"
        );
        assert_eq!(
            std::fs::read(&run_path).expect("p08b run.json bytes"),
            run_tampered,
            "拒绝对盘上事实零修复"
        );
        std::fs::write(&run_path, &run_original).expect("p08b restore run.json");

        // ③ 会话记录 author_provider 被改写:provider 绑定不再成立,必须拒绝。
        let session_path = store
            .workspace_sessions_root(&project, &issue)
            .join(format!("{}.json", handle.workspace_session_id));
        let session_original = std::fs::read(&session_path).expect("p08b session bytes");
        let mut session_json: serde_json::Value =
            serde_json::from_slice(&session_original).expect("p08b session json");
        session_json["author_provider"] = serde_json::json!("claude_code");
        std::fs::write(&session_path, serde_json::to_vec(&session_json).expect("p08b encode"))
            .expect("p08b tamper session");
        assert!(
            plan_split_run_for_handle(&store, &project, &issue, &provider, handle.clone()).is_err(),
            "session author_provider 篡改必须拒绝"
        );
        std::fs::write(&session_path, &session_original).expect("p08b restore session");
    }
}
