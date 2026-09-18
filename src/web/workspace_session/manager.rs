use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

use super::journal::EventJournal;
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::product::app_paths::ProductAppPaths;
use crate::product::checkpoint_store::CheckpointStore;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::logical_codebase::resolve_issue_logical_codebase_id;
use crate::product::models::WorkspaceSessionRecord;
use crate::product::workspace_engine::{EngineEvent, WorkspaceEngine, WorkspaceSession};
use crate::product::workspace_repository::workspace_repository_for_session;
use crate::web::state::WebAppState;
use crate::web::workspace_context::ensure_workspace_context_message;
use crate::web::workspace_session::{
    ConnectionRole, LeaseState, WorkspaceSessionRegistry, is_write_message,
};
use crate::web::workspace_ws_handler::{
    OutboundControl, ProviderCommand, ProviderRunContext, ProviderRunKind,
    planning_resume_decision_with_fresh_index, planning_resume_run_kind,
    spawn_provider_run_from_handler,
};
use crate::web::workspace_ws_types::{WsInMessage, WsOutMessage};

pub(super) struct Attachment {
    pub(super) outbound_tx: mpsc::Sender<OutboundControl>,
    /// 一旦直播通道溢出，该连接只能通过重连回到一致状态；此后 router 不再向其投递。
    pub(super) degraded: bool,
    // Hello 前的一个 RTT 内，连接维持 legacy driver 等价，待 Hello 归一后覆盖。
    role: ConnectionRole,
    after_event_seq: Option<u64>,
    lease_epoch: u64,
    provisional_lease: Option<LeaseState>,
}

pub(super) struct ManagerState {
    /// 尚未由首条入站或宽限期裁决的连接不可接收直播帧，保证初帧/回放顺序。
    pending_attachments: HashMap<String, Attachment>,
    /// 已完成首帧或 cursor 回放裁决的连接接收直播帧。
    pub(super) attachments: HashMap<String, Attachment>,
    next_run_id: u64,
    active_run: Option<ActiveRun>,
    pub(super) journal: EventJournal,
    lease: LeaseState,
    recovery_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ActiveRun {
    pub id: u64,
    pub token: u64,
    /// 已登记的活动 run 所属 timeline 节点；用于拒绝同一节点的重复启动请求。
    pub node_id: Option<String>,
    pub cancel: CancellationToken,
    pub command_tx: mpsc::Sender<ProviderCommand>,
    pub pending_choice_ids: Arc<Mutex<HashSet<String>>>,
    /// 启动该 run 时的授权 epoch；角色仲裁在 Task 8 落地。
    pub lease_epoch: u64,
}

/// 一个 durable workspace session 的唯一运行期所有者。
///
/// provider task 会在其整个生命周期中持有 engine 锁。因此 attach 不能等待该锁：它先
/// 尝试短暂获取锁，连续两次失败后使用一次性 durable 投影器渲染快照。投影器只读、
/// 不驱动 provider、不登记 run、不订阅 engine event；它保留第二连接立即可读的能力，
/// 但不再创建第二个拥有 run 所有权的 engine。
pub struct WorkspaceSessionManager {
    pub(super) engine: Arc<Mutex<WorkspaceEngine>>,
    engine_tx: mpsc::Sender<EngineEvent>,
    pub(super) state: StdMutex<ManagerState>,
    /// 序号在 manager 生命周期内严格单调；manager 被回收后 durable 重建会改走
    /// snapshot 基线，故不需要将它持久化。
    pub(super) next_event_seq: AtomicU64,
    pub session_id: String,
    pub session_record: WorkspaceSessionRecord,
    pub app_paths: ProductAppPaths,
    provider_registry: Arc<ProviderRegistry>,
    workspace_runs: crate::web::state::WorkspaceRunRegistry,
    registry: WorkspaceSessionRegistry,
}
#[cfg(test)]
impl WorkspaceSessionManager {
    /// 仅供 registry 并发单测制造可做指针比较的具体 manager；不启动 router 或 provider。
    pub(crate) fn test_fixture(session_id: &str) -> Arc<Self> {
        let (engine_tx, _engine_rx) = mpsc::channel(1);
        let engine = WorkspaceEngine::new(
            Arc::new(CheckpointStore::new(std::env::temp_dir().join(session_id))),
            engine_tx.clone(),
            WorkspaceSession::from_record(test_session_record(session_id)),
        );
        Arc::new(Self {
            engine: Arc::new(Mutex::new(engine)),
            engine_tx,
            state: StdMutex::new(ManagerState {
                pending_attachments: HashMap::new(),
                attachments: HashMap::new(),
                next_run_id: 0,
                active_run: None,
                lease: LeaseState::default(),
                journal: EventJournal::default(),
                recovery_error: None,
            }),
            session_id: session_id.to_string(),
            session_record: test_session_record(session_id),
            app_paths: ProductAppPaths::new(std::env::temp_dir().join(session_id)),
            provider_registry: Arc::new(ProviderRegistry::new()),
            workspace_runs: crate::web::state::WorkspaceRunRegistry::default(),
            next_event_seq: AtomicU64::new(1),
            registry: WorkspaceSessionRegistry::default(),
        })
    }

    pub(crate) fn test_fixture_with_parts(
        session_id: &str,
        engine: Arc<Mutex<WorkspaceEngine>>,
        provider_registry: Arc<ProviderRegistry>,
        app_paths: ProductAppPaths,
        session_record: WorkspaceSessionRecord,
    ) -> Arc<Self> {
        let (engine_tx, _engine_rx) = mpsc::channel(1);
        Arc::new(Self {
            engine,
            engine_tx,
            state: StdMutex::new(ManagerState {
                pending_attachments: HashMap::new(),
                attachments: HashMap::new(),
                next_run_id: 0,
                active_run: None,
                journal: EventJournal::default(),
                lease: LeaseState::default(),
                recovery_error: None,
            }),
            next_event_seq: AtomicU64::new(1),
            session_id: session_id.to_string(),
            session_record,
            app_paths,
            provider_registry,
            workspace_runs: crate::web::state::WorkspaceRunRegistry::default(),
            registry: WorkspaceSessionRegistry::default(),
        })
    }

    pub(crate) async fn test_set_active_run(&self, run: ActiveRun) {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .active_run = Some(run);
    }
}

#[cfg(test)]
fn test_session_record(session_id: &str) -> WorkspaceSessionRecord {
    WorkspaceSessionRecord {
        id: session_id.to_string(),
        project_id: "project_test".to_string(),
        issue_id: "issue_test".to_string(),
        entity_id: "entity_test".to_string(),
        workspace_type: crate::product::models::WorkspaceType::Story,
        status: crate::product::models::WorkspaceSessionStatus::Open,
        author_provider: crate::product::models::ProviderName::Fake,
        reviewer_provider: crate::product::models::ProviderName::Fake,
        review_rounds: 0,
        permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
        provisional_reviewer_provider: None,
        reviewer_enabled_at_start: None,
        superpowers_enabled: false,
        openspec_enabled: false,
        flow_kind: crate::product::work_item_plan_policy::WorkItemPlanFlowKind::Legacy,
        run_policy: crate::product::work_item_plan_policy::RunPolicy::Interactive,
        run_history: crate::product::work_item_plan_policy::RunHistory::default(),
        review_invocation_scope: None,
        human_gate_snapshot: None,
        repair_reservation: None,
        human_gate_reservation: None,
        policy_diagnostics: Vec::new(),
        provider_start_ledger: Vec::new(),
        single_candidate_phase: None,
        work_item_plan_source_revision_ref: None,
        plan_candidate_ir_ref: None,
        mechanical_report_ref: None,
        publication_provenance_ref: None,
        approval_attempt_id: None,
        approved_at: None,
        compile_reservation: None,
        work_item_runtime_binding: None,
        provider_conversations: Vec::new(),
        messages: Vec::new(),
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

impl WorkspaceSessionManager {
    /// 将原 socket-local engine 构建链迁入每 session 一次的工厂。
    pub async fn create(state: &WebAppState, session_id: &str) -> Result<Arc<Self>, String> {
        let app_paths = ProductAppPaths::new(state.workspace_root.join(".aria"));
        let lifecycle = LifecycleStore::new(app_paths.clone());
        let session_record = lifecycle
            .get_workspace_session(session_id)
            .map_err(|error| format!("workspace session not found: {error}"))?;
        let session_record =
            ensure_workspace_context_message(&app_paths, &lifecycle, session_record)
                .await
                .map_err(|error| format!("workspace context unavailable: {error}"))?;
        let repository = workspace_repository_for_session(&app_paths, &lifecycle, &session_record)
            .map_err(|error| format!("workspace repository unavailable: {error}"))?;
        let checkpoint_store = Arc::new(CheckpointStore::new(
            app_paths.issue_lifecycle_root(&session_record.project_id, &session_record.issue_id),
        ));
        let (engine_tx, engine_rx) = mpsc::channel::<EngineEvent>(64);
        let mut session = WorkspaceSession::from_record(session_record.clone());
        session.repository_path = Some(repository.path);
        if let Ok(checkpoints) = checkpoint_store.list_checkpoints(&session.session_id) {
            session.restore_checkpoint_ids(&checkpoints);
        }
        let mut engine_workspace = WorkspaceEngine::new_persistent(
            checkpoint_store,
            lifecycle,
            engine_tx.clone(),
            session,
        );
        if repository.logical_repository_id.is_some() {
            let lc_id = resolve_issue_logical_codebase_id(
                &app_paths,
                &session_record.project_id,
                &session_record.issue_id,
            )
            .map_err(|error| format!("logical codebase resolution failed: {error}"))?;
            let factory = state
                .gateway_factory()
                .ok_or_else(|| "logical gateway factory unavailable".to_string())?;
            let gateway = factory
                .build_for_lc(&session_record.project_id, lc_id.as_deref())
                .map_err(|error| format!("logical gateway build failed: {error}"))?;
            engine_workspace = engine_workspace.with_logical_provider_gateway(Arc::new(gateway));
        }
        let engine = Arc::new(Mutex::new(engine_workspace));
        {
            let mut locked = engine.lock().await;
            locked
                .ensure_plan_repair_artifacts()
                .await
                .map_err(|error| format!("plan repair artifact bootstrap failed: {error:?}"))?;
        }

        let manager = Arc::new(Self {
            engine,
            engine_tx,
            state: StdMutex::new(ManagerState {
                pending_attachments: HashMap::new(),
                attachments: HashMap::new(),
                next_run_id: 0,
                active_run: None,
                lease: LeaseState::default(),
                recovery_error: None,
                journal: EventJournal::default(),
            }),
            session_id: session_id.to_string(),
            session_record,
            app_paths,
            provider_registry: state.provider_registry.clone(),
            workspace_runs: state.workspace_runs.clone(),
            registry: state.workspace_sessions.clone(),
            next_event_seq: AtomicU64::new(1),
        });
        manager.spawn_event_router(engine_rx, state.workspace_runs.clone());
        manager.recover_on_creation().await;
        Ok(manager)
    }

    /// 构造所有 run 调用方共享的上下文；run 所有权仅存在于本 manager。
    pub(crate) fn provider_run_context(
        self: &Arc<Self>,
        workspace_runs: crate::web::state::WorkspaceRunRegistry,
    ) -> ProviderRunContext {
        ProviderRunContext {
            provider_registry: self.provider_registry.clone(),
            manager: self.clone(),
            engine: self.engine(),
            workspace_runs,
            session_id: self.session_id.clone(),
            connection_id: None,
            lease_epoch: None,
            app_paths: self.app_paths.clone(),
            session_record: self.session_record.clone(),
        }
    }
    /// registry 在 sessions 互斥中调用。新连接先进入 pending 表：在首条入站裁决
    /// 前绝不接收直播，避免直播帧越过初帧或 cursor 回放。
    pub(crate) fn register_attachment(
        &self,
        connection_id: &str,
        outbound_tx: mpsc::Sender<OutboundControl>,
    ) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(message) = state.recovery_error.clone() {
            let _ = outbound_tx.try_send(OutboundControl::Text(
                serde_json::to_string(&WsOutMessage::Error { message }).unwrap_or_else(|_| {
                    "{\"type\":\"error\",\"message\":\"serialization failed\"}".to_string()
                }),
            ));
        }
        // role 字段直到 Hello 才可见。先以 legacy Driver 绑定保住旧客户端和第二
        // 连接 abort 语义；若随后显式声明 Observer，bind_role 原样回滚此次临时接管。
        let provisional_lease = state.lease.clone();
        state.lease.acquire(connection_id);
        let lease_epoch = state.lease.epoch;
        state.pending_attachments.insert(
            connection_id.to_string(),
            Attachment {
                outbound_tx,
                degraded: false,
                role: ConnectionRole::Driver,
                after_event_seq: None,
                lease_epoch,
                provisional_lease: Some(provisional_lease),
            },
        );
    }

    /// 仅在初帧已排队、或 cursor 分支即将冻结 journal 时转为直播 attachment。
    pub(crate) fn activate_attachment(&self, connection_id: &str) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(attachment) = state.pending_attachments.remove(connection_id) {
            state
                .attachments
                .insert(connection_id.to_string(), attachment);
        }
    }

    /// 每次启动都在 `start_run_from_attachment` 的同一 manager 临界区内完成
    /// lease epoch 校验、旧 run 取出与新 run 登记。socket 路径额外在取得 engine 锁前
    /// 通过 `abort_active_run_from_attachment` 低延迟中止当前 run；启动前仍复检 epoch。
    pub async fn start_run(
        &self,
        _kind: ProviderRunKind,
        requested_node_id: Option<String>,
    ) -> Result<
        (
            u64,
            u64,
            CancellationToken,
            mpsc::Receiver<ProviderCommand>,
            Option<String>,
        ),
        String,
    > {
        self.start_run_from_attachment(None, None, requested_node_id)
            .await
    }

    /// 在取得 engine 锁前，以 attachment epoch 核验 lease 并立即 supersede 当前
    /// run。故已被接管的迟到写永远不会取消 run，同时保留同一 holder 覆盖流式 run 的
    /// 既有低延迟时序。
    pub async fn abort_active_run_from_attachment(
        &self,
        connection_id: Option<&str>,
        attachment_epoch: Option<u64>,
    ) -> Result<(), String> {
        let run = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(connection_id) = connection_id
                && (state.lease.holder.as_deref() != Some(connection_id)
                    || attachment_epoch != Some(state.lease.epoch))
            {
                return Err("STALE_DRIVER_LEASE".to_string());
            }
            let run = state.active_run.take();
            if run.is_some() {
                state.journal.mark_run_terminal();
            }
            run
        };
        if let Some(run) = run {
            Self::cancel_run(&run);
        }
        Ok(())
    }

    /// 在 provider 启动前再次检查 socket attachment 的 lease epoch，并在同一 manager
    /// 临界区内取出被覆盖的 run、登记新 run，避免并发 relay/socket 启动覆盖 run 时
    /// 遗留未受 manager 管理且未取消的 provider task。
    pub async fn start_run_from_attachment(
        &self,
        connection_id: Option<&str>,
        attachment_epoch: Option<u64>,
        requested_node_id: Option<String>,
    ) -> Result<
        (
            u64,
            u64,
            CancellationToken,
            mpsc::Receiver<ProviderCommand>,
            Option<String>,
        ),
        String,
    > {
        let (replaced_run, run) = {
            let (command_tx, command_rx) = mpsc::channel(8);
            let cancel = CancellationToken::new();
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(connection_id) = connection_id
                && (state.lease.holder.as_deref() != Some(connection_id)
                    || attachment_epoch != Some(state.lease.epoch))
            {
                return Err("STALE_DRIVER_LEASE".to_string());
            }
            let replaced_run = state.active_run.take();
            state.next_run_id += 1;
            let run_id = state.next_run_id;
            let token = crate::web::workspace_ws_handler::NEXT_ACTIVE_RUN_TOKEN
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let node_id = requested_node_id;
            let lease_epoch = state.lease.epoch;
            state.active_run = Some(ActiveRun {
                id: run_id,
                token,
                node_id: node_id.clone(),
                cancel: cancel.clone(),
                command_tx,
                pending_choice_ids: Arc::new(Mutex::new(HashSet::new())),
                lease_epoch,
            });
            state
                .journal
                .mark_run_started(self.next_event_seq.load(Ordering::Relaxed));
            (replaced_run, (run_id, token, cancel, command_rx, node_id))
        };
        if let Some(run) = replaced_run {
            Self::cancel_run(&run);
        }
        Ok(run)
    }

    pub(crate) fn lease_epoch_for_connection(&self, connection_id: &str) -> Option<u64> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state
            .attachments
            .get(connection_id)
            .or_else(|| state.pending_attachments.get(connection_id))
            .map(|attachment| attachment.lease_epoch)
    }
    pub async fn finish_run(self: &Arc<Self>, token: u64) {
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if state
                .active_run
                .as_ref()
                .is_some_and(|run| run.token == token)
            {
                state.active_run = None;
                state.journal.mark_run_terminal();
            }
        }
        self.maybe_recycle().await;
    }

    pub async fn abort_active_run(&self) -> bool {
        let run = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let run = state.active_run.take();
            if run.is_some() {
                state.journal.mark_run_terminal();
            }
            run
        };
        if let Some(run) = run {
            Self::cancel_run(&run);
            true
        } else {
            false
        }
    }

    pub async fn active_run(&self) -> Option<ActiveRun> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .active_run
            .clone()
    }

    pub async fn active_run_command_tx(&self) -> Option<mpsc::Sender<ProviderCommand>> {
        self.active_run().await.map(|run| run.command_tx)
    }

    pub async fn replace_command_tx_if_token(
        &self,
        token: u64,
        command_tx: mpsc::Sender<ProviderCommand>,
    ) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(run) = state.active_run.as_mut()
            && run.token == token
        {
            run.command_tx = command_tx;
        }
    }

    pub fn is_active_run(&self) -> bool {
        self.state
            .lock()
            .map(|state| state.active_run.is_some())
            .unwrap_or(true)
    }

    fn cancel_run(run: &ActiveRun) {
        eprintln!(
            "[aria-cancellation] workspace abort_workspace_run cancelling runner token trigger=abort_workspace_run run_id=run-{} run_token={} node_id={:?}",
            run.id, run.token, run.node_id
        );
        let _ = run.command_tx.try_send(ProviderCommand::Abort);
        run.cancel.cancel();
    }

    /// REQ-WCR-03：连接关闭只摘除 attachment；若它持有 lease，仅撤销该授权。
    /// 连接关闭绝不取消 run、不写入 durable 终态也不改写 engine 状态。
    pub(crate) async fn handle_connection_closed(self: &Arc<Self>, connection_id: &str) {
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.lease.revoke_if_holder(connection_id);
            state.pending_attachments.remove(connection_id);
            state.attachments.remove(connection_id);
        }
        self.maybe_recycle().await;
    }

    pub fn engine(&self) -> Arc<Mutex<WorkspaceEngine>> {
        self.engine.clone()
    }

    pub fn engine_tx(&self) -> mpsc::Sender<EngineEvent> {
        self.engine_tx.clone()
    }

    /// Hello 入口完成 wire role 的一次归一。pending 期间的连接也必须先归一，
    /// 随后 cursor 分支会在同一状态锁内将其转为直播 attachment。
    pub(crate) fn bind_role(
        &self,
        outbound_tx: &mpsc::Sender<OutboundControl>,
        role: ConnectionRole,
        after_event_seq: Option<u64>,
    ) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let connection_id = state
            .pending_attachments
            .iter()
            .find_map(|(connection_id, attachment)| {
                attachment
                    .outbound_tx
                    .same_channel(outbound_tx)
                    .then(|| connection_id.clone())
            })
            .or_else(|| {
                state
                    .attachments
                    .iter()
                    .find_map(|(connection_id, attachment)| {
                        attachment
                            .outbound_tx
                            .same_channel(outbound_tx)
                            .then(|| connection_id.clone())
                    })
            });
        let Some(connection_id) = connection_id else {
            return;
        };
        let provisional_lease =
            if let Some(attachment) = state.pending_attachments.get_mut(&connection_id) {
                attachment.role = role;
                attachment.after_event_seq = after_event_seq;
                attachment.provisional_lease.take()
            } else if let Some(attachment) = state.attachments.get_mut(&connection_id) {
                attachment.role = role;
                attachment.after_event_seq = after_event_seq;
                attachment.provisional_lease.take()
            } else {
                return;
            };
        if role == ConnectionRole::Observer {
            if let Some(rollback) = provisional_lease
                && state.lease.holder.as_deref() == Some(connection_id.as_str())
            {
                state.lease = rollback;
            }
            return;
        }
        state.lease.acquire(&connection_id);
        let lease_epoch = state.lease.epoch;
        if let Some(attachment) = state.pending_attachments.get_mut(&connection_id) {
            attachment.lease_epoch = lease_epoch;
        } else if let Some(attachment) = state.attachments.get_mut(&connection_id) {
            attachment.lease_epoch = lease_epoch;
        }
    }

    pub(crate) fn connection_role(&self, connection_id: &str) -> ConnectionRole {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state
            .attachments
            .get(connection_id)
            .or_else(|| state.pending_attachments.get(connection_id))
            .map(|attachment| attachment.role)
            .unwrap_or(ConnectionRole::Driver)
    }

    /// 读循环在取得 engine 锁之前调用。无 role 已在 Hello 入口归一为 Driver；任何
    /// 非 holder 的 Driver 写面都必须由 lease 以 STALE_DRIVER_LEASE 拒绝。
    pub(crate) fn arbitrate(
        &self,
        connection_id: &str,
        message: &WsInMessage,
    ) -> Result<(), ConnectionRole> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (role, lease_epoch) = state
            .attachments
            .get(connection_id)
            .or_else(|| state.pending_attachments.get(connection_id))
            .map(|attachment| (attachment.role, attachment.lease_epoch))
            // attachment 只会在 socket 收尾时摘除；若发生内部竞态，写面保守拒绝。
            .unwrap_or((ConnectionRole::Observer, 0));
        if !is_write_message(message) {
            return Ok(());
        }
        if role == ConnectionRole::Observer {
            return Err(ConnectionRole::Observer);
        }
        if state.lease.holder.as_deref() != Some(connection_id)
            || !state.lease.matches_epoch(lease_epoch)
        {
            return Err(ConnectionRole::Driver);
        }
        Ok(())
    }
    /// 返回已登记 attachment 的 initial snapshot 与已恢复 choice。
    pub(crate) async fn attached_session_state(&self) -> (WsOutMessage, Option<WsOutMessage>) {
        for attempt in 0..2 {
            if let Ok(engine) = self.engine.try_lock() {
                return (
                    engine.build_session_state(),
                    engine.pending_author_choice_request_message(),
                );
            }
            if attempt == 0 {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }
        }
        self.durable_projection_offload().await
    }

    /// F-06/F-09：durable 投影的同步读盘挪入阻塞线程池——观察 attach 频发且 run 任务
    /// 长期持有 engine 锁时，fallback 不得阻塞 async worker。JoinHandle 异常（panic/
    /// 取消）时退回原同步路径：投影只读，最坏也只是回到旧行为。
    async fn durable_projection_offload(&self) -> (WsOutMessage, Option<WsOutMessage>) {
        let app_paths = self.app_paths.clone();
        let session_record = self.session_record.clone();
        match tokio::task::spawn_blocking(move || {
            Self::durable_projection_with(app_paths, session_record)
        })
        .await
        {
            Ok(projection) => projection,
            Err(join_error) => {
                eprintln!(
                    "[aria-durable-projection] blocking offload join failed session={}: {join_error}",
                    self.session_id
                );
                self.durable_projection()
            }
        }
    }

    /// 在序列化边界为 attach snapshot 注入当前 event_seq 基线。T10 将在首个 Hello 的
    /// cursor 分支抑制该初帧，确保回放事件不会被前端基线去重吞掉；本阶段仍维持既有
    /// 立即 attach。返回 JSON 而非重新反序列化枚举，以保留增量字段。
    pub(crate) fn serialize_attach_session_state(&self, message: WsOutMessage) -> Option<String> {
        let seq = self
            .next_event_seq
            .load(Ordering::Relaxed)
            .saturating_sub(1);
        inject_event_seq(serde_json::to_string(&message).ok()?, seq)
    }

    /// 完成 cursor 重订阅：先在同一状态锁内确认 attachment 在直播表中，再冻结
    /// journal 窗口；锁外投递使 broadcaster 可以继续推进。重叠帧由客户端按
    /// `event_seq` 去重，因此该顺序没有遗漏窗口。
    pub(crate) async fn resubscribe(
        &self,
        outbound_tx: &mpsc::Sender<OutboundControl>,
        connection_id: &str,
        after_event_seq: u64,
    ) {
        enum Resubscription {
            Replay(Vec<String>),
            ActiveRunWindow { baseline: u64, events: Vec<String> },
            Snapshot { baseline: u64 },
        }

        let subscription = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let connection_id =
                state
                    .pending_attachments
                    .iter()
                    .find_map(|(connection_id, attachment)| {
                        attachment
                            .outbound_tx
                            .same_channel(outbound_tx)
                            .then(|| connection_id.clone())
                    });
            if let Some(connection_id) = connection_id
                && let Some(attachment) = state.pending_attachments.remove(&connection_id)
            {
                state.attachments.insert(connection_id, attachment);
            }
            match state.journal.replay_after(after_event_seq) {
                Some(events) => Resubscription::Replay(events),
                None => match state.journal.active_run_window() {
                    Some((first_seq, events)) => Resubscription::ActiveRunWindow {
                        baseline: first_seq.saturating_sub(1),
                        events,
                    },
                    None => Resubscription::Snapshot {
                        baseline: self
                            .next_event_seq
                            .load(Ordering::Relaxed)
                            .saturating_sub(1),
                    },
                },
            }
        };

        match subscription {
            Resubscription::Replay(events) => {
                for event in events {
                    if outbound_tx
                        .send(OutboundControl::Text(event))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                let (_, choice) = self.attached_session_state().await;
                let _ = send_optional_message(outbound_tx, choice).await;
            }
            Resubscription::ActiveRunWindow { baseline, events } => {
                let (snapshot, choice) = self.attached_session_state().await;
                if !self
                    .send_snapshot_with_baseline(
                        outbound_tx,
                        snapshot.with_connection_id(connection_id),
                        baseline,
                    )
                    .await
                {
                    return;
                }
                if !send_optional_message(outbound_tx, choice).await {
                    return;
                }
                for event in events {
                    if outbound_tx
                        .send(OutboundControl::Text(event))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
            Resubscription::Snapshot { baseline } => {
                let (snapshot, choice) = self.attached_session_state().await;
                if !self
                    .send_snapshot_with_baseline(
                        outbound_tx,
                        snapshot.with_connection_id(connection_id),
                        baseline,
                    )
                    .await
                {
                    return;
                }
                let _ = send_optional_message(outbound_tx, choice).await;
            }
        }
    }

    async fn send_snapshot_with_baseline(
        &self,
        outbound_tx: &mpsc::Sender<OutboundControl>,
        snapshot: WsOutMessage,
        baseline: u64,
    ) -> bool {
        let Some(json) = inject_event_seq(
            match serde_json::to_string(&snapshot) {
                Ok(json) => json,
                Err(_) => return false,
            },
            baseline,
        ) else {
            return false;
        };
        outbound_tx.send(OutboundControl::Text(json)).await.is_ok()
    }

    pub async fn detach(self: &Arc<Self>, connection_id: &str) {
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.attachments.remove(connection_id);
            state.pending_attachments.remove(connection_id);
        }
        self.maybe_recycle().await;
    }

    pub(crate) fn is_recyclable(&self) -> bool {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.active_run.is_none()
            && state.attachments.is_empty()
            && state.pending_attachments.is_empty()
            && !self
                .workspace_runs
                .provider_drive_in_progress(&self.session_id)
    }

    async fn maybe_recycle(self: &Arc<Self>) {
        let registry = self.registry.clone();
        let session_id = self.session_id.clone();
        registry.remove_if_idle(&session_id, self).await;
    }

    /// 一次性只读 durable 投影器。禁止将它扩展为第二 engine 状态源：没有 spawn、没有
    /// provider/run 注册、没有 event receiver，并且使用有接收端但永不消费的 channel。
    /// 读盘重（Lifecycle/Checkpoint 仓库 + JSON 反序列化），async 上下文必须经
    /// `durable_projection_offload` 调用；`current_session_state` 的同步降级路径是
    /// 唯一的直接调用方。
    pub(super) fn durable_projection(&self) -> (WsOutMessage, Option<WsOutMessage>) {
        Self::durable_projection_with(self.app_paths.clone(), self.session_record.clone())
    }

    fn durable_projection_with(
        app_paths: ProductAppPaths,
        session_record: WorkspaceSessionRecord,
    ) -> (WsOutMessage, Option<WsOutMessage>) {
        let lifecycle = LifecycleStore::new(app_paths.clone());
        let checkpoint_store = Arc::new(CheckpointStore::new(
            app_paths.issue_lifecycle_root(&session_record.project_id, &session_record.issue_id),
        ));
        let (projection_tx, _projection_rx) = mpsc::channel::<EngineEvent>(1);
        let mut session = WorkspaceSession::from_record(session_record.clone());
        if let Ok(repository) =
            workspace_repository_for_session(&app_paths, &lifecycle, &session_record)
        {
            session.repository_path = Some(repository.path);
        }
        if let Ok(checkpoints) = checkpoint_store.list_checkpoints(&session.session_id) {
            session.restore_checkpoint_ids(&checkpoints);
        }
        let engine =
            WorkspaceEngine::new_persistent(checkpoint_store, lifecycle, projection_tx, session);
        (
            engine.build_session_state(),
            engine.pending_author_choice_request_message(),
        )
    }

    async fn recover_on_creation(self: &Arc<Self>) {
        let run_context = self.provider_run_context(self.workspace_runs.clone());
        let recovery_error = match self.recover_human_gate_turns(&run_context).await {
            Ok(()) => self.recover_outline_run(&run_context).await,
            Err(error) => Err(error),
        };
        if let Err(message) = recovery_error {
            eprintln!(
                "[aria-recovery] workspace session recovery skipped session={}: {message}",
                self.session_id
            );
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .recovery_error = Some(message);
        }
    }

    async fn recover_human_gate_turns(
        self: &Arc<Self>,
        run_context: &ProviderRunContext,
    ) -> Result<(), String> {
        let human_gate_recovery = {
            let mut engine = self.engine.lock().await;
            engine.recover_human_gate_turns(false)
        }
        .map_err(|error| format!("human gate recovery failed: {error}"))?;
        for (turn_id, action) in human_gate_recovery {
            if !matches!(
                action,
                crate::product::workspace_engine::HumanGateRecoveryAction::ResumeSameTurn { .. }
            ) {
                continue;
            }
            let run_kind = crate::product::workspace_engine::provider_run_kind_for_human_gate(
                self.session_record.flow_kind,
                &turn_id,
            )?;
            let (outbound_tx, _outbound_rx) = mpsc::channel(1);
            spawn_provider_run_from_handler(run_context.clone(), run_kind, outbound_tx).await?;
        }
        Ok(())
    }

    async fn recover_outline_run(
        self: &Arc<Self>,
        run_context: &ProviderRunContext,
    ) -> Result<(), String> {
        let outline_resume_kind = {
            let engine = self.engine.lock().await;
            let durable_flow_kind = self.session_record.flow_kind;
            if let Some(error) = engine.outline_revision_recovery_error() {
                Err(format!("outline revision recovery failed: {error}"))
            } else {
                let should_resume = engine.session().workspace_type
                    == crate::product::models::WorkspaceType::WorkItemPlan
                    && engine.session().stage == crate::product::workspace_engine::WorkspaceStage::Running
                    && engine.active_node_type()
                        == Some(crate::web::workspace_ws_types::TimelineNodeType::WorkItemPlanOutlineRun)
                    && engine.active_run_id().is_none();
                if !should_resume {
                    Ok(None)
                } else if let Some(node_id) = engine.active_timeline_node_id() {
                    match LifecycleStore::new(self.app_paths.clone())
                        .load_node_detail(&self.session_id, &node_id)
                    {
                        Ok(detail) => Ok(Some(if durable_flow_kind
                            == crate::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate
                        {
                            ProviderRunKind::work_item_plan_author_for_durable_flow(durable_flow_kind)
                        } else if detail.is_revision {
                            ProviderRunKind::WorkItemPlanOutlineRevision {
                                feedback: detail.revision_feedback,
                            }
                        } else {
                            ProviderRunKind::work_item_plan_author_for_durable_flow(durable_flow_kind)
                        })),
                        Err(crate::product::json_store::ProductStoreError::NotFound { .. }) => {
                            Ok(Some(ProviderRunKind::work_item_plan_author_for_durable_flow(
                                durable_flow_kind,
                            )))
                        }
                        Err(error) => Err(format!(
                            "resume outline run detail failed for {node_id}: {error}"
                        )),
                    }
                } else {
                    Err("resume outline run detail failed: active node id unavailable".to_string())
                }
            }
        };
        if let Some(run_kind) = outline_resume_kind?
            && self.active_run().await.is_none()
        {
            let decision = planning_resume_decision_with_fresh_index(
                &self.app_paths,
                &self.session_record.project_id,
                &self.session_record.issue_id,
            )
            .await?;
            let (outbound_tx, _outbound_rx) = mpsc::channel(1);
            spawn_provider_run_from_handler(
                run_context.clone(),
                planning_resume_run_kind(&decision, run_kind),
                outbound_tx,
            )
            .await?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn attachment_is_degraded(&self, connection_id: &str) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .attachments
            .get(connection_id)
            .is_some_and(|attachment| attachment.degraded)
    }

    #[cfg(test)]
    pub(crate) async fn attach(
        &self,
        connection_id: &str,
        outbound_tx: mpsc::Sender<OutboundControl>,
    ) {
        self.register_attachment(connection_id, outbound_tx);
        self.activate_attachment(connection_id);
    }

    #[cfg(test)]
    pub(crate) async fn broadcast_test_event(
        &self,
        status: crate::web::workspace_ws_types::WsProviderStatus,
    ) {
        self.broadcast(WsOutMessage::ProviderStatus { status });
    }

    #[cfg(test)]
    pub(crate) fn test_fixture_with_event_router(session_id: &str) -> Arc<Self> {
        let root = tempfile::tempdir().expect("router fixture root");
        let app_paths = ProductAppPaths::new(root.keep().join(".aria"));
        let session_record = test_session_record(session_id);
        let (engine_tx, engine_rx) = mpsc::channel(8);
        let engine = WorkspaceEngine::new_persistent(
            Arc::new(CheckpointStore::new(std::env::temp_dir().join(session_id))),
            LifecycleStore::new(app_paths.clone()),
            engine_tx.clone(),
            WorkspaceSession::from_record(session_record.clone()),
        );
        let manager = Arc::new(Self {
            engine: Arc::new(Mutex::new(engine)),
            engine_tx,
            state: StdMutex::new(ManagerState {
                pending_attachments: HashMap::new(),
                attachments: HashMap::new(),
                next_run_id: 0,
                active_run: None,
                lease: LeaseState::default(),
                journal: EventJournal::default(),
                recovery_error: None,
            }),
            session_id: session_id.to_string(),
            session_record,
            app_paths,
            provider_registry: Arc::new(ProviderRegistry::new()),
            workspace_runs: crate::web::state::WorkspaceRunRegistry::default(),
            next_event_seq: AtomicU64::new(1),
            registry: WorkspaceSessionRegistry::default(),
        });
        manager.spawn_event_router(
            engine_rx,
            crate::web::state::WorkspaceRunRegistry::default(),
        );
        manager
    }
}

async fn send_optional_message(
    outbound_tx: &mpsc::Sender<OutboundControl>,
    message: Option<WsOutMessage>,
) -> bool {
    let Some(message) = message else {
        return true;
    };
    let Ok(json) = serde_json::to_string(&message) else {
        return false;
    };
    outbound_tx.send(OutboundControl::Text(json)).await.is_ok()
}

/// 保持 `WsOutMessage` schema 不变，在 JSON 顶层增加可选 `event_seq`。
pub(super) fn inject_event_seq(message: String, seq: u64) -> Option<String> {
    let mut value = serde_json::from_str::<serde_json::Value>(&message).ok()?;
    let object = value.as_object_mut()?;
    object.insert("event_seq".to_string(), serde_json::Value::from(seq));
    serde_json::to_string(&value).ok()
}
