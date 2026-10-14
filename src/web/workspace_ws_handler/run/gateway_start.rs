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
            let mut recovery = record.lc_resume_recovery.as_mut().filter(|recovery| {
                recovery.state == LcResumeRecoveryState::FreshRunning
                    && recovery.fresh_role_run_seq == Some(role_run_seq)
            });
            if let Some(recovery) = recovery.as_deref_mut() {
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

/// B2a（Task 5.1）/B2b3（Task 5.4）/B2c（Task 5.5）/B2d（Task 5.6）/B1
/// （终审 P1，author 族）fresh dispatch 的 tag 覆盖面——A/B 发布纪律
/// （设计 §4.5/§10.1）：fresh 启动能力按子包逐 tag 放开，未接管 tag
/// typed 拒零 spawn。已覆盖：
/// `legacy_outline_rebuild`（5.1，dispatch 时新解析 resolved planning
/// context 组装）；`legacy_draft`/`legacy_batch`/`legacy_plan_revision`
/// （5.4，kind 可直接组装，Draft/Batch 的业务代事实 spawn 前按 durable
/// active index typed 校验，PlanRevision 的 retained/redo 对照由臂内重算
/// 消费 engine artifact）；`sc_author`（5.5，payload phase/reservation
/// key/source ref spawn 前对 durable typed 对照）；`sc_manual_revision`
/// （5.6，payload turn 事实 spawn 前对 durable turn/ledger typed 对照）；
/// author 族（`author`/`author_choice_followup`/`author_feedback`/
/// `reviewer_revision`，终审 P1：业务四元组身份/artifact/feedback spawn 前
/// 对 durable typed 对照，臂内 author_root_launch/review drive 消费点以
/// FreshRunning 重入——resume=None 全新 native + full-fresh prompt 自足）。
/// 其余 tag typed 拒（fresh 链恒 `resume_provider_session_id=None`，旧
/// audit 不回捞）。
async fn lc_fresh_dispatch_kind(
    recovery: &crate::product::models::LcResumeRecoveryRecord,
    run_context: &ProviderRunContext,
) -> Result<ProviderRunKind, LcFreshDispatchRejection> {
    match lc_fresh_run_kind_for_recovery_context(&recovery.run_context)? {
        LcFreshRunKind::NeedsFreshPlanningContext => {
            let decision =
                crate::web::workspace_ws_handler::planning_resume_decision_with_fresh_index(
                    &run_context.app_paths,
                    &run_context.session_record.project_id,
                    &run_context.session_record.issue_id,
                )
                .await
                .map_err(|error| {
                    lc_context_unsupported(format!(
                        "legacy_outline_rebuild fresh dispatch failed to resolve planning context: {error}"
                    ))
                })?;
            let resolved = match decision {
                // 漂移仍在：携带 rebuilt（重新 build 的权威上下文）。
                Some(crate::product::logical_codebase::ResumeDecision::StaleContext {
                    rebuilt,
                    ..
                }) => rebuilt,
                // 快照已对齐（如等待期外力修复）：现解析结果即新 snapshot。
                Some(crate::product::logical_codebase::ResumeDecision::SameContext(resolved)) => {
                    resolved
                }
                None => {
                    return Err(lc_context_unsupported(
                        "legacy_outline_rebuild fresh dispatch requires logical planning routing"
                            .to_string(),
                    ));
                }
            };
            Ok(ProviderRunKind::WorkItemPlanOutlineRebuild {
                rebuilt: Box::new(resolved),
            })
        }
        // B2b3（Task 5.4，设计 §5.3 Draft/Batch/PlanRevision 行）：三 tag 的
        // full fresh 臂接管。Draft/Batch 业务代（generation/outline/batch）
        // 在 spawn 前按 durable active index typed 校验——漂移即
        // `lc_resume_recovery_stale` 拒收回等待，不用旧权限强行完成；
        // PlanRevision 的 retained/redo 重算对照在臂内（需 engine artifact）。
        kind @ LcFreshRunKind::Kind(
            ProviderRunKind::WorkItemPlanDraft { .. }
            | ProviderRunKind::WorkItemPlanBatch
            | ProviderRunKind::WorkItemPlanRevision { .. },
        ) => {
            verify_lc_fresh_business_generation(&recovery.run_context, run_context)?;
            match kind {
                LcFreshRunKind::Kind(kind) => Ok(kind),
                // 上面 match 收窄到 Kind；NeedsFreshPlanningContext 分支已消费。
                LcFreshRunKind::NeedsFreshPlanningContext => {
                    unreachable!("NeedsFreshPlanningContext arm is matched before the Kind arm")
                }
            }
        }
        // B2c（Task 5.5，设计 §5.3 sc_author 行）：SC author tag 的 full
        // fresh 臂接管——payload 的 phase/reservation key/source ref 与
        // durable typed 对照（漂移即 `lc_resume_recovery_stale` 拒收回
        // 等待，不用旧权限强行完成），对照通过即放行 kind；臂内
        // （provider_run SC 臂）以 LcFreshContext 重入：同 logical
        // reservation 不 reserve 第二次、resume=None 全新 native、verdict
        // 在场走 fresh 返修 prompt（上一版 source 全文自足）。
        kind @ LcFreshRunKind::Kind(ProviderRunKind::WorkItemPlanSingleCandidateAuthor) => {
            verify_lc_fresh_sc_author_payload(&recovery.run_context, run_context)?;
            match kind {
                LcFreshRunKind::Kind(kind) => Ok(kind),
                // 上面 match 收窄到 Kind；NeedsFreshPlanningContext 分支已消费。
                LcFreshRunKind::NeedsFreshPlanningContext => {
                    unreachable!("NeedsFreshPlanningContext arm is matched before the Kind arm")
                }
            }
        }
        // B2d（Task 5.6，设计 §5.3 sc_manual_revision 行）：SC manual tag 的
        // full fresh 臂接管——payload 冻结的 turn 事实（command/source_hash/
        // attempt/budget/状态非终态/reservation key/source ref）与 durable
        // typed 对照（漂移即 `lc_resume_recovery_stale` 拒收回等待，不用旧
        // turn 权限强行完成），对照通过即放行 kind；臂内（provider_run
        // manual 臂）同 turn 重入：同 turn prompt 重装（candidate hash 复
        // 验）、resume=None 全新 native、run_sc_manual_revision_turn 同
        // turn 完成回原 human gate（不新 turn/attempt/预算、不通用
        // StartGeneration rearm）。
        kind @ LcFreshRunKind::Kind(ProviderRunKind::HumanGateScManualRevision { .. }) => {
            verify_lc_fresh_sc_manual_revision_payload(&recovery.run_context, run_context)?;
            match kind {
                LcFreshRunKind::Kind(kind) => Ok(kind),
                // 上面 match 收窄到 Kind；NeedsFreshPlanningContext 分支已消费。
                LcFreshRunKind::NeedsFreshPlanningContext => {
                    unreachable!("NeedsFreshPlanningContext arm is matched before the Kind arm")
                }
            }
        }
        // B1（终审 P1，B1→B2 接缝，设计 §5.3 author 族行）：author 族四 tag
        //（author/author_choice_followup/author_feedback/reviewer_revision，
        // 生产唯一可达等待出生=review drive 的 ReviewerRevision mismatch
        // persist）的 full fresh dispatch 臂接管——payload 冻结的业务四元组
        //（entity/workspace 身份、author/reviewer provider、artifact 引用、
        // feedback）spawn 前与 durable typed 对照（等待期任一维度被推进即
        // 漂移，`lc_resume_recovery_stale` 拒收回等待，不用旧权限强行完
        // 成），对照通过即放行 kind；臂内（provider_run Author/ChoiceFollowup
        // 臂 + review drive revision 消费点）按 FreshRunning 重入：resume=None
        // 全新 native、full-fresh prompt 自足（B1a
        // `build_revision_input_with_resume(false)` 合同：artifact/feedback/
        // review 全量内嵌）。4.2 的 OperationResult/claim 链零改动。
        kind @ LcFreshRunKind::Kind(
            ProviderRunKind::Author { .. }
            | ProviderRunKind::AuthorChoiceFollowup { .. }
            | ProviderRunKind::Revision,
        ) => {
            verify_lc_fresh_author_family_payload(&recovery.run_context, run_context)?;
            match kind {
                LcFreshRunKind::Kind(kind) => Ok(kind),
                // 上面 match 收窄到 Kind；NeedsFreshPlanningContext 分支已消费。
                LcFreshRunKind::NeedsFreshPlanningContext => {
                    unreachable!("NeedsFreshPlanningContext arm is matched before the Kind arm")
                }
            }
        }
        // full fresh 臂接管——dispatch 直接放行既有 kind；臂内
        // （work_item_plan_legacy_author.inc.rs）以
        // build_outline_fresh_revision_invocation 构建 prompt 自足的 full
        // fresh 返修（旧 outline 全文/feedback/review/完整材料）并全新启动
        // （resume=None、全新节点、原 outline 确认门）；正常 resume 的增量
        // 返修/首发出生不受影响。legacy_outline_rebuild 仍走上方
        // NeedsFreshPlanningContext 臂（dispatch 时新解析 planning context）。
        kind @ LcFreshRunKind::Kind(
            ProviderRunKind::WorkItemPlanLegacyAuthor
            | ProviderRunKind::WorkItemPlanOutlineRevision { .. },
        ) => match kind {
            LcFreshRunKind::Kind(kind) => Ok(kind),
            // 上面 match 收窄到 Kind；NeedsFreshPlanningContext 分支已消费。
            LcFreshRunKind::NeedsFreshPlanningContext => {
                unreachable!("NeedsFreshPlanningContext arm is matched before the Kind arm")
            }
        },
        LcFreshRunKind::Kind(kind) => Err(lc_context_unsupported(format!(
            "run_context tag {} (resolved kind {kind:?}) fresh handler lands with the remaining B2 packages; explicit fresh stays waiting",
            lc_recovery_context_tag_label(&recovery.run_context)
        ))),
    }
}

/// B2b3（Task 5.4，设计 §5.3）：Draft/Batch tag 的业务代 typed 校验——
/// payload 携带的 generation/outline/batch 事实与 durable active index
/// 不一致时以 `lc_resume_recovery_stale` 拒收（等待新漂移裁决）；index
/// 读取失败按 store unavailable 家族 typed 上浮。LegacyPlanRevision 不在
/// 此校验（retained/redo 由臂内重算对照）。
fn verify_lc_fresh_business_generation(
    context: &crate::product::models::LcRecoveryRunContext,
    run_context: &ProviderRunContext,
) -> Result<(), LcFreshDispatchRejection> {
    use crate::product::models::LcRecoveryRunContext;
    use crate::product::work_item_plan_store::WorkItemPlanStore;

    let (generation_round_id, active_outline_id, batch_id) = match context {
        LcRecoveryRunContext::LegacyDraft {
            generation_round_id,
            active_outline_id,
            ..
        } => (
            generation_round_id.as_str(),
            Some(active_outline_id.as_str()),
            None,
        ),
        LcRecoveryRunContext::LegacyBatch {
            generation_round_id,
            batch_id,
            ..
        } => (generation_round_id.as_str(), None, Some(batch_id.as_str())),
        _ => return Ok(()),
    };
    let stale = |message: String| LcFreshDispatchRejection {
        code: crate::product::lifecycle_store::LcFreshClaimOutcome::STALE_SUPERSEDED,
        message,
    };
    let index = WorkItemPlanStore::new(run_context.app_paths.clone())
        .load_active_index(
            &run_context.session_record.project_id,
            &run_context.session_record.issue_id,
            &run_context.session_record.entity_id,
        )
        .map_err(|error| LcFreshDispatchRejection {
            code: crate::web::workspace_ws_handler::socket::LC_RESUME_STORE_UNAVAILABLE_CODE,
            message: format!("load work item plan active index failed: {error}"),
        })?
        .ok_or_else(|| {
            stale(
                "fresh business generation verify failed: work item plan active index missing"
                    .to_string(),
            )
        })?;
    if index.current_generation_round_id != generation_round_id {
        return Err(stale(format!(
            "fresh business generation verify failed: payload round {generation_round_id} != durable round {}",
            index.current_generation_round_id
        )));
    }
    if let Some(expected_outline) = active_outline_id
        && index.active_outline_id.as_deref() != Some(expected_outline)
    {
        return Err(stale(format!(
            "fresh business generation verify failed: payload outline {expected_outline} != durable outline {:?}",
            index.active_outline_id
        )));
    }
    if let Some(expected_batch) = batch_id
        && !index.batches.iter().any(|batch| {
            batch.batch_id == expected_batch
                && batch.generation_round_id == index.current_generation_round_id
        })
    {
        return Err(stale(format!(
            "fresh business generation verify failed: payload batch {expected_batch} not found in durable generation {}",
            index.current_generation_round_id
        )));
    }
    Ok(())
}

/// B2c（Task 5.5，设计 §5.3 sc_author 行）：sc_author payload 与 durable 的
/// typed 对照——payload phase/reservation key/source ref 必须与当前 durable
/// 事实一致（等待期间 phase/ledger/source 被推进即漂移，`lc_resume_recovery_stale`
/// 拒收回等待，不用旧权限强行完成）；record 读取失败按 store unavailable
/// 家族 typed 上浮。同 logical reservation 重入的资格由此锚定：payload key
/// 即 durable ledger 既有键（fresh 不 reserve 第二次）。
fn verify_lc_fresh_sc_author_payload(
    context: &crate::product::models::LcRecoveryRunContext,
    run_context: &ProviderRunContext,
) -> Result<(), LcFreshDispatchRejection> {
    let crate::product::models::LcRecoveryRunContext::ScAuthor {
        single_candidate_phase,
        provider_start_reservation_key,
        source_revision_ref,
        ..
    } = context
    else {
        return Ok(());
    };
    let stale = |message: String| LcFreshDispatchRejection {
        code: crate::product::lifecycle_store::LcFreshClaimOutcome::STALE_SUPERSEDED,
        message,
    };
    let record =
        crate::product::lifecycle_store::LifecycleStore::new(run_context.app_paths.clone())
            .get_workspace_session(&run_context.session_id)
            .map_err(|error| LcFreshDispatchRejection {
                code: crate::web::workspace_ws_handler::socket::LC_RESUME_STORE_UNAVAILABLE_CODE,
                message: format!(
                    "load workspace session for sc author fresh verify failed: {error}"
                ),
            })?;
    if record.single_candidate_phase.as_ref() != Some(single_candidate_phase) {
        return Err(stale(format!(
            "sc author fresh verify failed: payload phase {single_candidate_phase:?} != durable phase {:?}",
            record.single_candidate_phase
        )));
    }
    if let Some(expected_key) = provider_start_reservation_key.as_deref()
        && !record
            .provider_start_ledger
            .iter()
            .any(|entry| entry.provider_start_idempotency_key == expected_key)
    {
        return Err(stale(format!(
            "sc author fresh verify failed: payload reservation key {expected_key} absent from durable provider start ledger"
        )));
    }
    if source_revision_ref != &record.work_item_plan_source_revision_ref {
        return Err(stale(format!(
            "sc author fresh verify failed: payload source ref {source_revision_ref:?} != durable source ref {:?}",
            record.work_item_plan_source_revision_ref
        )));
    }
    Ok(())
}

/// B2d（Task 5.6，设计 §5.3 sc_manual_revision 行）：sc_manual payload 与
/// durable 的 typed 对照——payload 冻结的 turn 事实（command_id/source_
/// hash/attempt_no/budget_reserved、turn 仍非终态、reservation key 在
/// durable ledger、source ref 与 record 一致）必须与当前 durable 事实一致
///（等待期间 turn 被推进/换代/终态化即漂移，`lc_resume_recovery_stale`
/// 拒收回等待，不用旧 turn 权限强行完成）；store 读取失败按 store
/// unavailable 家族 typed 上浮。同 turn 重入的资格由此锚定：payload 的
/// reservation key 即 durable ledger 既有键（fresh 不 reserve 第二次）。
fn verify_lc_fresh_sc_manual_revision_payload(
    context: &crate::product::models::LcRecoveryRunContext,
    run_context: &ProviderRunContext,
) -> Result<(), LcFreshDispatchRejection> {
    let crate::product::models::LcRecoveryRunContext::ScManualRevision {
        turn_id,
        command_id,
        source_hash,
        attempt_no,
        budget_reserved,
        human_gate_reservation_key,
        source_revision_ref,
        ..
    } = context
    else {
        return Ok(());
    };
    let stale = |message: String| LcFreshDispatchRejection {
        code: crate::product::lifecycle_store::LcFreshClaimOutcome::STALE_SUPERSEDED,
        message,
    };
    let store = crate::product::lifecycle_store::LifecycleStore::new(run_context.app_paths.clone());
    let record = store
        .get_workspace_session(&run_context.session_id)
        .map_err(|error| LcFreshDispatchRejection {
            code: crate::web::workspace_ws_handler::socket::LC_RESUME_STORE_UNAVAILABLE_CODE,
            message: format!("load workspace session for sc manual fresh verify failed: {error}"),
        })?;
    let turn = store
        .get_human_gate_turn(&run_context.session_id, turn_id)
        .map_err(|error| LcFreshDispatchRejection {
            code: crate::web::workspace_ws_handler::socket::LC_RESUME_STORE_UNAVAILABLE_CODE,
            message: format!("load human gate turn for sc manual fresh verify failed: {error}"),
        })?;
    if !matches!(
        turn.status,
        crate::product::models::HumanGateTurnStatus::Reserved
            | crate::product::models::HumanGateTurnStatus::Running
    ) {
        return Err(stale(format!(
            "sc manual fresh verify failed: turn {} is terminal ({:?})",
            turn.turn_id, turn.status
        )));
    }
    if turn.command_id != *command_id
        || turn.source_hash != *source_hash
        || turn.attempt_no != *attempt_no
        || turn.budget_reserved != *budget_reserved
    {
        return Err(stale(format!(
            "sc manual fresh verify failed: payload turn facts (command {command_id}, \
             source_hash {source_hash}, attempt {attempt_no}, budget {budget_reserved}) \
             != durable turn (command {}, source_hash {}, attempt {}, budget {})",
            turn.command_id, turn.source_hash, turn.attempt_no, turn.budget_reserved
        )));
    }
    if let Some(expected_key) = human_gate_reservation_key.as_deref()
        && !record
            .provider_start_ledger
            .iter()
            .any(|entry| entry.provider_start_idempotency_key == expected_key)
    {
        return Err(stale(format!(
            "sc manual fresh verify failed: payload reservation key {expected_key} absent from durable provider start ledger"
        )));
    }
    if source_revision_ref != &record.work_item_plan_source_revision_ref {
        return Err(stale(format!(
            "sc manual fresh verify failed: payload source ref {source_revision_ref:?} != durable source ref {:?}",
            record.work_item_plan_source_revision_ref
        )));
    }
    // SG6b：结构授权窗口 join（recovery×turn×grant×attempt×prompt）。
    verify_sc_manual_fresh_structural_grant_join(&record, context)
}

/// SG6b（plan v1.1 Task SG6b / REQ-SG-05「显式 fresh 同 attempt
/// None-resume/子 prompt」）：sc_manual_revision 显式 fresh dispatch 的
/// 结构授权窗口 join（recovery×turn×grant×attempt×prompt 的 durable 侧
/// 判据，纯函数零 I/O）——活动 grant 绑定本 turn 且 phase=Attempt2Claimed
/// 时，fresh 重入驱动的就是 claim 绑定的冻结机器指令子 prompt：grant 必
/// 须携带完整 prompt 身份且恰消费下一 attempt（grant.attempt_no ==
/// payload attempt_no + 1）；漂移（错 recovery/身份缺失/attempt 越界/
/// 窗口已 closed）一律 `lc_resume_recovery_stale` 拒收回等待（零
/// start、预算不改写）。无结构窗口（无 grant）保持原 fresh 路径不变。
fn verify_sc_manual_fresh_structural_grant_join(
    record: &crate::product::models::WorkspaceSessionRecord,
    context: &crate::product::models::LcRecoveryRunContext,
) -> Result<(), LcFreshDispatchRejection> {
    let crate::product::models::LcRecoveryRunContext::ScManualRevision {
        turn_id,
        command_id,
        attempt_no,
        ..
    } = context
    else {
        return Ok(());
    };
    verify_sc_manual_fresh_grant_join_core(
        record
            .structural_governance_state
            .as_ref()
            .and_then(|state| state.current_grant.as_ref()),
        turn_id,
        command_id,
        *attempt_no,
    )
}

fn verify_sc_manual_fresh_grant_join_core(
    grant: Option<&crate::product::models::workspace::StructuralRevisionGrant>,
    turn_id: &str,
    command_id: &str,
    attempt_no: u32,
) -> Result<(), LcFreshDispatchRejection> {
    use crate::product::models::workspace::StructuralGrantPhase;
    let stale = |message: String| LcFreshDispatchRejection {
        code: crate::product::lifecycle_store::LcFreshClaimOutcome::STALE_SUPERSEDED,
        message,
    };
    let Some(grant) = grant else {
        // 无结构授权窗口：原 fresh 路径不变。
        return Ok(());
    };
    match grant.turn_id.as_deref() {
        Some(grant_turn_id) if grant_turn_id == turn_id => {}
        Some(grant_turn_id) => {
            return Err(stale(format!(
                "sc manual fresh structural join failed: grant is bound to turn {grant_turn_id}, not {turn_id}"
            )));
        }
        None => {
            return Err(stale(
                "sc manual fresh structural join failed: grant has no bound turn".to_string(),
            ));
        }
    }
    if let Some(grant_command_id) = grant.command_id.as_deref()
        && grant_command_id != command_id
    {
        return Err(stale(format!(
            "sc manual fresh structural join failed: grant is bound to command {grant_command_id}, not {command_id}"
        )));
    }
    match grant.phase {
        Some(StructuralGrantPhase::Attempt2Claimed) => {
            let (prompt_ref, prompt_digest) = (
                grant.prompt_ref.as_deref().unwrap_or_default(),
                grant.prompt_digest.as_deref().unwrap_or_default(),
            );
            if prompt_ref.trim().is_empty() || prompt_digest.trim().is_empty() {
                return Err(stale(format!(
                    "sc manual fresh structural join failed: claimed grant carries an incomplete frozen prompt identity (ref {prompt_ref:?}, digest present: {})",
                    !prompt_digest.trim().is_empty()
                )));
            }
            // 恰消费下一 attempt：子 prompt 绑定 payload attempt + 1。
            if grant.attempt_no != attempt_no.saturating_add(1) {
                return Err(stale(format!(
                    "sc manual fresh structural join failed: claimed grant attempt {} does not continue payload attempt {attempt_no}",
                    grant.attempt_no
                )));
            }
            Ok(())
        }
        Some(
            StructuralGrantPhase::Reviewing
            | StructuralGrantPhase::Resolved
            | StructuralGrantPhase::SystemFault
            | StructuralGrantPhase::BusinessGate,
        ) => Err(stale(format!(
            "sc manual fresh structural join failed: structural window is closed for delivery (phase {:?}); explicit fresh stays waiting",
            grant.phase
        ))),
        // 未 claim 窗口（AuthorRunning/Assessing/InstructionFrozen/
        // BlockedByLcRecovery 且无 claim 身份）：fresh 重入＝原反馈 prompt
        // 的同 turn 重装，放行；携带 claim 身份的非 Attempt2Claimed 相位
        // 是不一致 durable，拒收。
        _ => {
            if grant
                .prompt_ref
                .as_deref()
                .is_some_and(|v| !v.trim().is_empty())
            {
                return Err(stale(format!(
                    "sc manual fresh structural join failed: grant phase {:?} carries a claimed prompt identity; refusing inconsistent structural state",
                    grant.phase
                )));
            }
            Ok(())
        }
    }
}

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
/// 漂移 → `lc_resume_recovery_stale` 拒收回等待；store 读取失败按 store
/// unavailable 家族 typed 上浮。
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
}
