use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
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
use crate::web::workspace_session::{ConnectionRole, LeaseState, WorkspaceSessionRegistry};
use crate::web::workspace_ws_handler::{OutboundControl, ProviderCommand, ProviderRunContext};
use crate::web::workspace_ws_types::WsOutMessage;

mod arbitration;
mod attachment;
mod choices;
mod durable_projection;
mod runs;

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
    /// F-24：挂起 provider choice 的完整 wire 帧（保持到达顺序）。到达时注册，
    /// 应答/终态时移除；attach/重订阅/degraded 恢复按此补发，用户不可见窗口
    /// 即 run 楔死窗口（0484 现场锚）。
    pub pending_choices: Arc<StdMutex<Vec<WsOutMessage>>>,
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

    pub fn engine(&self) -> Arc<Mutex<WorkspaceEngine>> {
        self.engine.clone()
    }

    pub fn engine_tx(&self) -> mpsc::Sender<EngineEvent> {
        self.engine_tx.clone()
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

/// 保持 `WsOutMessage` schema 不变，在 JSON 顶层增加可选 `event_seq`。
pub(super) fn inject_event_seq(message: String, seq: u64) -> Option<String> {
    let mut value = serde_json::from_str::<serde_json::Value>(&message).ok()?;
    let object = value.as_object_mut()?;
    object.insert("event_seq".to_string(), serde_json::Value::from(seq));
    serde_json::to_string(&value).ok()
}
