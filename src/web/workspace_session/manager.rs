use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

use tokio::sync::{Mutex, mpsc};

use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::cross_cutting::streaming_provider::ChoiceRequestSource;
use crate::product::app_paths::ProductAppPaths;
use crate::product::checkpoint_store::CheckpointStore;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::logical_codebase::resolve_issue_logical_codebase_id;
use crate::product::models::WorkspaceSessionRecord;
use crate::product::workspace_engine::{EngineEvent, WorkspaceEngine, WorkspaceSession};
use crate::product::workspace_repository::workspace_repository_for_session;
use crate::web::state::WebAppState;
use crate::web::workspace_context::ensure_workspace_context_message;
use crate::web::workspace_ws_handler::{
    OutboundControl, ProviderRunContext, map_engine_event, spawn_provider_run_from_event,
};
use crate::web::workspace_ws_types::WsOutMessage;

struct ManagerState {
    attachments: HashMap<String, mpsc::Sender<OutboundControl>>,
}

/// 一个 durable workspace session 的唯一运行期所有者。
///
/// provider task 会在其整个生命周期中持有 engine 锁。因此 attach 不能等待该锁：它先
/// 尝试短暂获取锁，连续两次失败后使用一次性 durable 投影器渲染快照。投影器只读、
/// 不驱动 provider、不登记 run、不订阅 engine event；它保留第二连接立即可读的能力，
/// 但不再创建第二个拥有 run 所有权的 engine。
pub struct WorkspaceSessionManager {
    engine: Arc<Mutex<WorkspaceEngine>>,
    engine_tx: mpsc::Sender<EngineEvent>,
    state: StdMutex<ManagerState>,
    pub session_id: String,
    pub session_record: WorkspaceSessionRecord,
    pub app_paths: ProductAppPaths,
    provider_registry: Arc<ProviderRegistry>,
    current_run: Arc<Mutex<Option<crate::web::state::WorkspaceActiveRun>>>,
    next_run_id: Arc<Mutex<u64>>,
}
#[cfg(test)]
impl WorkspaceSessionManager {
    /// 仅供 registry 并发单测制造可做指针比较的具体 manager；不启动 router 或 provider。
    pub(super) fn test_fixture(session_id: &str) -> Arc<Self> {
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
                attachments: HashMap::new(),
            }),
            session_id: session_id.to_string(),
            session_record: test_session_record(session_id),
            app_paths: ProductAppPaths::new(std::env::temp_dir().join(session_id)),
            provider_registry: Arc::new(ProviderRegistry::new()),
            current_run: Arc::new(Mutex::new(None)),
            next_run_id: Arc::new(Mutex::new(0)),
        })
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
                attachments: HashMap::new(),
            }),
            session_id: session_id.to_string(),
            session_record,
            app_paths,
            provider_registry: state.provider_registry.clone(),
            current_run: Arc::new(Mutex::new(None)),
            next_run_id: Arc::new(Mutex::new(0)),
        });
        manager.spawn_event_router(engine_rx, state.workspace_runs.clone());
        Ok(manager)
    }

    /// Task 2 前保留既有 ProviderRunContext 形状，但所有 attachment 共用同一组
    /// current_run/next_run_id，避免 engine 所有权迁移后再次复制 run 所有权。
    pub(crate) fn provider_run_context(
        &self,
        workspace_runs: crate::web::state::WorkspaceRunRegistry,
    ) -> ProviderRunContext {
        ProviderRunContext {
            provider_registry: self.provider_registry.clone(),
            engine: self.engine(),
            current_run: self.current_run.clone(),
            workspace_runs,
            session_id: self.session_id.clone(),
            next_run_id: self.next_run_id.clone(),
            app_paths: self.app_paths.clone(),
            session_record: self.session_record.clone(),
        }
    }

    pub(crate) fn current_run(&self) -> Arc<Mutex<Option<crate::web::state::WorkspaceActiveRun>>> {
        self.current_run.clone()
    }

    pub fn engine(&self) -> Arc<Mutex<WorkspaceEngine>> {
        self.engine.clone()
    }

    pub fn engine_tx(&self) -> mpsc::Sender<EngineEvent> {
        self.engine_tx.clone()
    }

    /// 注册连接的出站通道并返回 initial snapshot 与已恢复 choice。
    pub(crate) async fn attach(
        &self,
        connection_id: &str,
        outbound_tx: mpsc::Sender<OutboundControl>,
    ) -> (WsOutMessage, Option<WsOutMessage>) {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .attachments
            .insert(connection_id.to_string(), outbound_tx);

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
        self.durable_projection()
    }

    pub async fn detach(&self, connection_id: &str) {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .attachments
            .remove(connection_id);
    }

    /// 一次性只读 durable 投影器。禁止将它扩展为第二 engine 状态源：没有 spawn、没有
    /// provider/run 注册、没有 event receiver，并且使用有接收端但永不消费的 channel。
    fn durable_projection(&self) -> (WsOutMessage, Option<WsOutMessage>) {
        let lifecycle = LifecycleStore::new(self.app_paths.clone());
        let checkpoint_store = Arc::new(CheckpointStore::new(self.app_paths.issue_lifecycle_root(
            &self.session_record.project_id,
            &self.session_record.issue_id,
        )));
        let (projection_tx, _projection_rx) = mpsc::channel::<EngineEvent>(1);
        let mut session = WorkspaceSession::from_record(self.session_record.clone());
        if let Ok(repository) =
            workspace_repository_for_session(&self.app_paths, &lifecycle, &self.session_record)
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

    /// session-owned event router：engine event 映射后 fan-out 到所有 attachment。
    /// channel 满或关闭仅跳过该 attachment；T9/T11 再加入 journal 与降级状态。
    fn spawn_event_router(
        self: &Arc<Self>,
        mut engine_rx: mpsc::Receiver<EngineEvent>,
        workspace_runs: crate::web::state::WorkspaceRunRegistry,
    ) {
        let manager = self.clone();
        tokio::spawn(async move {
            while let Some(event) = engine_rx.recv().await {
                match event {
                    EngineEvent::ProviderRunRequested { kind, node_id } => {
                        let outbound = manager
                            .state
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .attachments
                            .values()
                            .next()
                            .cloned();
                        let Some(outbound) = outbound else {
                            eprintln!(
                                "[aria-broadcast] provider run requested without attachment session={}",
                                manager.session_id
                            );
                            continue;
                        };
                        let run_context = manager.provider_run_context(workspace_runs.clone());
                        tokio::spawn(async move {
                            if let Err(message) = spawn_provider_run_from_event(
                                run_context,
                                kind,
                                node_id,
                                outbound.clone(),
                            )
                            .await
                            {
                                let _ = outbound.try_send(OutboundControl::Text(
                                    serde_json::to_string(&WsOutMessage::Error { message })
                                        .unwrap_or_else(|_| {
                                            "{\"type\":\"error\",\"message\":\"serialization failed\"}"
                                                .to_string()
                                        }),
                                ));
                            }
                        });
                    }
                    EngineEvent::ArtifactBatchUpdate { mut updates } => {
                        updates.sort_by_key(|update| update.version);
                        for update in updates {
                            manager.broadcast(
                                crate::web::workspace_ws_handler::ws_artifact_update(
                                    update.version,
                                    update.payload,
                                ),
                            );
                        }
                    }
                    EngineEvent::ChoiceRequest {
                        id,
                        prompt,
                        options,
                        allow_multiple,
                        allow_free_text,
                        questions,
                        source,
                    } => {
                        if source != ChoiceRequestSource::TextFallback {
                            let _ = workspace_runs
                                .register_choice(&manager.session_id, id.clone())
                                .await;
                        }
                        manager.broadcast(WsOutMessage::ChoiceRequest {
                            id,
                            prompt,
                            options: options
                                .into_iter()
                                .map(crate::web::workspace_ws_handler::ws_choice_option)
                                .collect(),
                            allow_multiple,
                            allow_free_text,
                            questions: questions
                                .into_iter()
                                .map(crate::web::workspace_ws_handler::ws_choice_question)
                                .collect(),
                            source: source.as_str().to_string(),
                        });
                    }
                    event => {
                        if let Some(message) = map_engine_event(event) {
                            manager.broadcast(message);
                        }
                    }
                }
            }
        });
    }

    fn broadcast(&self, message: WsOutMessage) {
        let Ok(json) = serde_json::to_string(&message) else {
            eprintln!(
                "[aria-broadcast] serialize failed session={}",
                self.session_id
            );
            return;
        };
        let attachments = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .attachments
            .iter()
            .map(|(connection_id, sender)| (connection_id.clone(), sender.clone()))
            .collect::<Vec<_>>();
        for (connection_id, sender) in attachments {
            if sender
                .try_send(OutboundControl::Text(json.clone()))
                .is_err()
            {
                eprintln!(
                    "[aria-broadcast] attachment unreachable session={} connection={connection_id}",
                    self.session_id
                );
            }
        }
    }
}
