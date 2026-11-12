use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64};
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
use crate::product::workspace_engine::conversational_gate::ConfirmationTrustContext;
use crate::product::workspace_engine::{EngineEvent, WorkspaceEngine, WorkspaceSession};
use crate::product::workspace_repository::workspace_repository_for_session;
use crate::web::state::WebAppState;
use crate::web::workspace_context::ensure_workspace_context_message;
use crate::web::workspace_session::{ConnectionRole, LeaseState, WorkspaceSessionRegistry};
use crate::web::workspace_ws_handler::{
    OutboundControl, ProviderCommand, ProviderRunContext, ProviderRunKind,
};
use crate::web::workspace_ws_types::WsOutMessage;

mod arbitration;
mod attachment;
mod choices;
use choices::ChoiceClaimRecord;

pub use choices::ChoiceReplyError;
mod degraded_diagnostics;
mod durable_projection;
/// A1b1（Task 2.3）：durable 纯投影加载核心的 crate 出口——2.3 tuple 内部
/// 消费、part_07 typed 断言与 2.4 router/attach/socket Result cutover 共用。
pub(crate) use durable_projection::DurableProjectionError;
// part_07 typed 断言用;库构建只经 durable_projection 内部路径,不经此处再导出。
#[cfg(test)]
pub(crate) use durable_projection::{
    DURABLE_PROJECTION_UNAVAILABLE_CODE, DurableWorkspaceProjection, WorkspaceSessionIdentity,
    load_durable_workspace_projection,
};
pub(crate) use lease_diagnostics::lease_diagnostics_path;
mod lease_diagnostics;
mod runs;

pub use runs::RunAbortOutcome;

/// REQ-WIGA-17：取消等待 completion 的兜底时长生产默认（30s，毫秒）。
const DEFAULT_RUN_COMPLETION_WAIT_TIMEOUT_MILLIS: u64 = 30_000;

pub(super) struct Attachment {
    pub(super) outbound_tx: mpsc::Sender<OutboundControl>,
    /// 一旦直播通道溢出，该连接只能通过重连回到一致状态；此后 router 不再向其投递。
    pub(super) degraded: bool,
    // Hello 前的一个 RTT 内，连接维持 legacy driver 等价，待 Hello 归一后覆盖。
    // REQ-DLS-04：attach 对租约零效应，role 与 epoch 之外不携带任何租约状态。
    role: ConnectionRole,
    after_event_seq: Option<u64>,
    lease_epoch: u64,
}

pub(super) struct ManagerState {
    /// P0 1.3：choice 应答 claim（短临界区登记；提交/回执在锁外）。
    /// 键 = (run_incarnation, choice_id)。
    pub(super) choice_claims: HashMap<(String, String), ChoiceClaimRecord>,
    /// P0 1.3：run 终态后保留的有界 claim 状态（同 command 查询面）。
    pub(super) finished_choice_status:
        std::collections::VecDeque<crate::web::choice_reply::ChoiceReplyStatus>,
    /// 尚未由首条入站或宽限期裁决的连接不可接收直播帧，保证初帧/回放顺序。
    pending_attachments: HashMap<String, Attachment>,
    /// 已完成首帧或 cursor 回放裁决的连接接收直播帧。
    pub(super) attachments: HashMap<String, Attachment>,
    next_run_id: u64,
    active_run: Option<ActiveRun>,
    /// 批 2 T2（REQ-WIGA-09）：run token → completion watch——任务真实收口
    /// 点 `finish_run` 按 token 发信号；`abort_active_run_and_wait_completion`
    /// 在锁外等待真实 completion（不以 is_active_run()==false 冒充退出）。
    pub(super) run_completions: HashMap<u64, tokio::sync::watch::Sender<bool>>,
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
    /// F2（workspace_session_0018 现场）：启动该 run 的 kind——「同节点去重」
    /// 只对「同 kind 同节点」的重复接力生效。无附件 REST 反馈链里，门修订 run
    /// 注册的节点仍是 human_confirm 门节点，委托返修接力请求（不同 kind）在
    /// emit 时也解析到同一门节点；仅按 node_id 去重会把它误 drain，而 followups
    /// 同时按 phase=Generate 让位——双方都退出，会话永久卡 running。
    pub kind: ProviderRunKind,
    pub cancel: CancellationToken,
    pub command_tx: mpsc::Sender<ProviderCommand>,
    /// F-24：挂起 provider choice 的完整 wire 帧（保持到达顺序）。到达时注册，
    /// 应答/终态时移除；attach/重订阅/degraded 恢复按此补发，用户不可见窗口
    /// 即 run 楔死窗口（0484 现场锚）。
    pub pending_choices: Arc<StdMutex<Vec<WsOutMessage>>>,
    /// 启动该 run 时的授权 epoch；角色仲裁在 Task 8 落地。
    pub lease_epoch: u64,
    /// P0 1.3（REQ-WIGA-05）：run 化身——每次启动生成 UUID，绝不复用可
    /// 重启归零的 id/token；choice 应答按它判旧（旧 run 一律 Expired）。
    pub run_incarnation: String,
}

/// test-only：活跃 run 的可比对快照（token/run_incarnation）——P1 Task 5
/// 自动认领不 supersede 断言专用；不进入生产接口。
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveRunRefForTest {
    pub token: u64,
    pub run_incarnation: String,
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
    /// REQ-DLS-03：租约转移 append-only 诊断流（打点在状态锁外，失败零影响）。
    lease_diagnostics: lease_diagnostics::LeaseDiagnostics,
    /// C3/REQ-HTR-03：degraded 转移 append-only 诊断流（同构通道，失败零影响）。
    degraded_diagnostics: degraded_diagnostics::DegradedDeliveryDiagnostics,
    /// 序号在 manager 生命周期内严格单调；manager 被回收后 durable 重建会改走
    /// snapshot 基线，故不需要将它持久化。
    pub(super) next_event_seq: AtomicU64,
    /// C3/REQ-HTR-01：人工确认门开标志（router 消费 HumanGateOpened/Closed
    /// 维护，锁无关）。Abort 矩阵校验在 engine 锁被 in-flight 修订 run 持有
    /// 时以它判定门态（非阻塞拒收）；初值 false（manager 重建后由下一次
    /// 门事件/全量基线收敛，误判面= 放行进取消臂=既有语义，fail-open 安全）。
    human_confirm_gate_open: AtomicBool,
    /// D2.1a（fix-cascade-deletion）：实体删除后的旧 Arc 失效标志——dispatch
    /// （start_run 族）与 attach 检查面消费；registry 驱逐后的重建面由
    /// `create` 的墓碑检查拒绝。
    invalidated_for_deletion: AtomicBool,
    /// REQ-WIGA-17：`abort_active_run_and_wait_completion` 等 completion 的
    /// 兜底时长（毫秒）——生产默认 30s；`set_run_completion_wait_timeout`
    /// 仅供测试注入可测时长（锁无关读取，Relaxed 足够）。
    run_completion_wait_timeout_millis: AtomicU64,
    pub session_id: String,
    pub session_record: WorkspaceSessionRecord,
    pub app_paths: ProductAppPaths,
    provider_registry: Arc<ProviderRegistry>,
    workspace_runs: crate::web::state::WorkspaceRunRegistry,
    registry: WorkspaceSessionRegistry,
}

/// D2.1a：旧 Arc 失效/重建拒绝的稳定消息（dispatch/attach/create 共用）。
pub(crate) const ENTITY_DELETED_STABLE_MESSAGE: &str =
    "entity_deleted: workspace session owner entity has been deleted";
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
                run_completions: HashMap::new(),
                choice_claims: HashMap::new(),
                finished_choice_status: std::collections::VecDeque::new(),
                lease: LeaseState::default(),
                journal: EventJournal::default(),
                recovery_error: None,
            }),
            lease_diagnostics: lease_diagnostics::LeaseDiagnostics::new(
                session_id,
                Some(
                    std::env::temp_dir()
                        .join(session_id)
                        .join(lease_diagnostics::LEASE_DIAGNOSTICS_FILE),
                ),
            ),
            degraded_diagnostics: degraded_diagnostics::DegradedDeliveryDiagnostics::new(
                session_id,
                Some(
                    std::env::temp_dir()
                        .join(session_id)
                        .join(degraded_diagnostics::DEGRADED_DIAGNOSTICS_FILE),
                ),
            ),
            session_id: session_id.to_string(),
            session_record: test_session_record(session_id),
            app_paths: ProductAppPaths::new(std::env::temp_dir().join(session_id)),
            provider_registry: Arc::new(ProviderRegistry::new()),
            workspace_runs: crate::web::state::WorkspaceRunRegistry::default(),
            next_event_seq: AtomicU64::new(1),
            human_confirm_gate_open: AtomicBool::new(false),
            invalidated_for_deletion: AtomicBool::new(false),
            run_completion_wait_timeout_millis: AtomicU64::new(
                DEFAULT_RUN_COMPLETION_WAIT_TIMEOUT_MILLIS,
            ),
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
                run_completions: HashMap::new(),
                active_run: None,
                choice_claims: HashMap::new(),
                finished_choice_status: std::collections::VecDeque::new(),
                journal: EventJournal::default(),
                lease: LeaseState::default(),
                recovery_error: None,
            }),
            // D2.1a：诊断分区随 fixture 的 app_paths 唯一（record id 跨 fixture
            // 重号不共享 temp 路径），并接 scope 准入（真实 store 在场）。
            lease_diagnostics: lease_diagnostics::LeaseDiagnostics::new(
                session_id,
                Some(
                    app_paths
                        .state_root()
                        .join(session_id)
                        .join(lease_diagnostics::LEASE_DIAGNOSTICS_FILE),
                ),
            )
            .with_admission(LifecycleStore::new(app_paths.clone())),
            degraded_diagnostics: degraded_diagnostics::DegradedDeliveryDiagnostics::new(
                session_id,
                Some(
                    app_paths
                        .state_root()
                        .join(session_id)
                        .join(degraded_diagnostics::DEGRADED_DIAGNOSTICS_FILE),
                ),
            )
            .with_admission(LifecycleStore::new(app_paths.clone())),
            next_event_seq: AtomicU64::new(1),
            human_confirm_gate_open: AtomicBool::new(false),
            invalidated_for_deletion: AtomicBool::new(false),
            run_completion_wait_timeout_millis: AtomicU64::new(
                DEFAULT_RUN_COMPLETION_WAIT_TIMEOUT_MILLIS,
            ),
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
pub(crate) fn test_session_record(session_id: &str) -> WorkspaceSessionRecord {
    WorkspaceSessionRecord {
        id: session_id.to_string(),
        project_id: "project_test".to_string(),
        issue_id: "issue_test".to_string(),
        entity_id: "entity_test".to_string(),
        workspace_type: crate::product::models::WorkspaceType::Story,
        status: crate::product::models::WorkspaceSessionStatus::Open,
        author_provider: crate::product::models::ProviderName::Fake,
        reviewer_provider: Some(crate::product::models::ProviderName::Fake),

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
        author_rewrite_budget: None,
        policy_diagnostics: Vec::new(),
        provider_start_ledger: Vec::new(),
        sc_generation_teaching_windows: Vec::new(),
        single_candidate_phase: None,
        work_item_plan_source_revision_ref: None,
        plan_candidate_ir_ref: None,
        mechanical_report_ref: None,
        publication_provenance_ref: None,
        approval_attempt_id: None,
        approved_at: None,
        compile_reservation: None,
        work_item_runtime_binding: None,
        work_item_child_binding: None,
        provider_conversations: Vec::new(),
        messages: Vec::new(),
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        sc_recovery_claim: None,
        logical_launch_fingerprints: Vec::new(),
        superseded_native_sessions: Vec::new(),
        lc_resume_recovery: None,
        structural_governance_state: None,
        review_identity_id: String::new(),
    }
}

impl WorkspaceSessionManager {
    /// 仅供 HTTP 预算测试：长持 engine 锁模拟 provider run 占用。
    #[cfg(test)]
    pub(crate) async fn lock_engine_for_test(
        &self,
    ) -> tokio::sync::MutexGuard<'_, WorkspaceEngine> {
        self.engine.lock().await
    }

    /// P0 1.2（REQ-WIGA-08）：对外 SessionState 帧统一以 durable enrollment
    /// 覆盖 automation 归属；读取失败不伪 client——置 None（前端保持未知）
    /// 并打点（HTTP 面由 handler 显式报错）。
    pub(crate) fn project_automation_ownership(&self, frame: &mut WsOutMessage) {
        let store = crate::product::issue_automation_store::IssueAutomationStore::new(
            self.app_paths.clone(),
        );
        if let Err(error) = crate::product::issue_automation_store::project_session_automation(
            frame,
            &self.session_record,
            &store,
        ) {
            eprintln!(
                "[aria-automation-ownership] projection failed session={}: {error}",
                self.session_id
            );
            if let WsOutMessage::SessionState { automation, .. } = frame {
                *automation = None;
            }
        }
        self.stamp_pending_choice_run_ids(frame);
    }

    /// REQ-HGC-04：本页底部「继续自动化」入口的服务端权威事实（WS
    /// `SessionState.plan_continue_available`）——与 HTTP control 投影的
    /// `allowed_actions` 含 `continue_plan` **同源**（同一
    /// `crate::web::plan_stop` 判定），两面不可能互相矛盾。
    ///
    /// 读取失败（会话记录 / enrollment 读不到）→ `None`（**不猜**）：前端按
    /// 「无真实续跑入口」处理，不得给假指引。
    pub(crate) fn project_plan_continue_available(&self, frame: &mut WsOutMessage) {
        let available =
            crate::web::plan_stop::plan_continue_available(&self.app_paths, &self.session_id);
        if let WsOutMessage::SessionState {
            plan_continue_available,
            ..
        } = frame
        {
            *plan_continue_available = available;
        }
    }

    /// P0 1.3（REQ-WIGA-05）：pending choice 投影注入当前 run 化身（引擎/
    /// durable 投影都不知道 manager run；无活跃 run 保持 None——旧载荷兼容，
    /// 前端不猜 run）。真实链 2026-09-26 发现：provider run 长期持有 engine
    /// 锁时 attach/广播全部走 durable 降级帧，此前只在 engine 空闲帧注入会
    /// 让无 driver 的驾驶舱拿不到 expected_run_id（REST 作答恒 410）——降级
    /// 帧同样过本函数。
    pub(crate) fn stamp_pending_choice_run_ids(&self, frame: &mut WsOutMessage) {
        if let WsOutMessage::SessionState {
            pending_choice_requests,
            ..
        } = frame
            && let Some(incarnation) = self.active_run_incarnation()
        {
            for request in pending_choice_requests.iter_mut() {
                if request.expected_run_id.is_none() {
                    request.expected_run_id = Some(incarnation.clone());
                }
            }
        }
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
        // D2.1a（fix-cascade-deletion，plan Interfaces）：registry 重建面
        //（旧 Arc 驱逐后的复活口）——purging/deleted 会话不重建 manager。
        // scope 解析是只读文件访问（非阻塞，registry creation lock 内侧执行，
        // 与 scope flock 无等待交互，锁序固定无环）。
        let run_scope = crate::product::provider_workspace_runner::resolve_session_run_scope(
            &app_paths,
            &session_record,
        )
        .map_err(|error| format!("workspace session run admission failed: {error}"))?;
        if let Some(scope) = &run_scope
            && let Some(record) =
                crate::product::entity_deletion_store::EntityDeletionStore::new(app_paths.clone())
                    .get(scope)
                    .map_err(|error| format!("entity deletion record read failed: {error}"))?
        {
            return Err(match record.phase {
                crate::product::entity_deletion_store::DeletionPhase::Purging => {
                    "entity_deleting: workspace session owner entity is being deleted"
                }
                crate::product::entity_deletion_store::DeletionPhase::Deleted => {
                    ENTITY_DELETED_STABLE_MESSAGE
                }
            }
            .to_string());
        }
        let session_record =
            ensure_workspace_context_message(&app_paths, &lifecycle, session_record)
                .await
                .map_err(|error| format!("workspace context unavailable: {error}"))?;
        let repository = workspace_repository_for_session(&app_paths, &lifecycle, &session_record)
            .map_err(|error| format!("workspace repository unavailable: {error}"))?;
        // REQ-DLS-03：打点路径解析失败（如会话目录不可定位）静默降级为无 durable 腿。
        // D2.1a：两诊断通道接 scope 准入（purging/deleted 迟到打点不重建
        // 已删除 timeline 分区）。
        let lease_diagnostics = lease_diagnostics::LeaseDiagnostics::new(
            session_id,
            lease_diagnostics::lease_diagnostics_path(&lifecycle, session_id),
        )
        .with_admission(lifecycle.clone());
        let degraded_diagnostics = degraded_diagnostics::DegradedDeliveryDiagnostics::new(
            session_id,
            degraded_diagnostics::degraded_diagnostics_path(&lifecycle, session_id),
        )
        .with_admission(lifecycle.clone());
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
        // C-2（与 C-1 的关系）：本 factory 注入点只负责组装 gateway（policy/
        // capability bootstrap + 注册表接线），不承担 admission 判定。LC 会话
        // 规划 author 运行链路的 admission 预检由 C-1 在
        // `run_single_candidate_author` 读取 language rules 之前统一前置
        // （`LogicalCodebaseProviderAdmissionPreflight`：成员规则缺失/漂移转
        // waiting/Prepare 面，单仓 legacy 零变化）；此处不重复设卡，避免二次
        // 判定产生漂移面。
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
                .build_readonly_for_lc(&session_record.project_id, Some(lc_id.as_str()))
                .map_err(|error| format!("logical gateway build failed: {error}"))?;
            // T4.3（REQ-REG-15/D2）：在已解析 project/issue/LC 并构造只读
            // gateway 的现有段注入确认 trust 冻结上下文——复用同一
            // WebAppState 缓存的 aggregate initialization trust precondition
            //（生产/fake HOME 策略同源，不新造第二套 HOME 推导），冻结
            // issue 原始 LC 归属与 gateway 冻结 root（`authority_root()`）；
            // gateway 只持只读 source，不持可写 registry。依赖图缺 trust
            // precondition 时不注入（fail-closed 延后到确认面
            // ContextWaiting，不阻断 create）。
            let frozen_root = gateway.authority_root().to_path_buf();
            if let Some(precondition) = state
                .aggregate_initialization_dependencies()
                .trust()
                .cloned()
            {
                engine_workspace =
                    engine_workspace.with_confirmation_trust_context(ConfirmationTrustContext {
                        precondition,
                        issue_lc_id: Some(lc_id.clone()),
                        frozen_root,
                    });
            }
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
                run_completions: HashMap::new(),
                next_run_id: 0,
                active_run: None,
                choice_claims: HashMap::new(),
                finished_choice_status: std::collections::VecDeque::new(),
                lease: LeaseState::default(),
                recovery_error: None,
                journal: EventJournal::default(),
            }),
            lease_diagnostics,
            degraded_diagnostics,
            session_id: session_id.to_string(),
            session_record,
            app_paths,
            provider_registry: state.provider_registry.clone(),
            workspace_runs: state.workspace_runs.clone(),
            registry: state.workspace_sessions.clone(),
            next_event_seq: AtomicU64::new(1),
            human_confirm_gate_open: AtomicBool::new(false),
            invalidated_for_deletion: AtomicBool::new(false),
            run_completion_wait_timeout_millis: AtomicU64::new(
                DEFAULT_RUN_COMPLETION_WAIT_TIMEOUT_MILLIS,
            ),
        });
        manager.spawn_event_router(engine_rx, state.workspace_runs.clone());
        manager.recover_on_creation().await;
        Ok(manager)
    }

    /// 仅供测试注入（REQ-WIGA-17）：把 `abort_active_run_and_wait_completion`
    /// 等 completion 的兜底时长缩到可测级（毫秒，原子写）。生产默认 30s，
    /// 生产代码不调用（集成测试在库外无法使用 `#[cfg(test)]` 项，故为 pub）。
    #[doc(hidden)]
    pub fn set_run_completion_wait_timeout(&self, timeout: std::time::Duration) {
        self.run_completion_wait_timeout_millis.store(
            timeout.as_millis() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
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
            app_paths: self.app_paths.clone(),
            session_record: self.session_record.clone(),
        }
    }

    /// C3/REQ-HTR-01：人工确认门是否开着（router 维护，锁无关）。
    pub fn human_confirm_gate_open(&self) -> bool {
        self.human_confirm_gate_open
            .load(std::sync::atomic::Ordering::Acquire)
    }

    /// D2.1a（fix-cascade-deletion，plan Interfaces）：实体删除后的旧 Arc
    /// 失效标记。只在删除独占已取得/无 run 时由删除编排调用（D2.2 升级
    /// 消费），无 cancel 副作用：不 abort 活 run、不写 durable、不把
    /// registry remove 当 stop；dispatch（start_run/
    /// try_start_run_if_idle）与 attach（register_attachment）检查后稳定
    /// 拒绝，旧 Arc 不可再驱动 provider 或重挂新连接。
    pub fn invalidate_for_entity_deletion(&self) {
        self.invalidated_for_deletion
            .store(true, std::sync::atomic::Ordering::Release);
        eprintln!(
            "[aria-entity-deletion] workspace manager invalidated session={}",
            self.session_id
        );
    }

    /// D2.1a：旧 Arc 是否已因实体删除失效（dispatch/attach 检查面）。
    pub fn invalidated_for_entity_deletion(&self) -> bool {
        self.invalidated_for_deletion
            .load(std::sync::atomic::Ordering::Acquire)
    }

    /// C3/REQ-HTR-03：degraded 进入打点（锁外调用，失败零影响）。
    pub(super) fn record_degraded_enter(&self, connection_id: &str, event_seq: u64) {
        self.degraded_diagnostics
            .record_enter(connection_id, event_seq);
    }

    /// C3/REQ-HTR-03：degraded 退出打点（锁外调用，失败零影响）。
    pub(super) fn record_degraded_exit(&self, connection_id: &str, event_seq: u64) {
        self.degraded_diagnostics
            .record_exit(connection_id, event_seq);
    }

    /// C3/REQ-HTR-01：router 事件面维护门开标志。
    pub(super) fn set_human_confirm_gate_open(&self, open: bool) {
        self.human_confirm_gate_open
            .store(open, std::sync::atomic::Ordering::Release);
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
        self: &Arc<Self>,
        status: crate::web::workspace_ws_types::WsProviderStatus,
    ) {
        self.broadcast(WsOutMessage::ProviderStatus { status });
    }

    /// 仅供单测广播关键帧（REQ-HTR-03 白名单面）。
    #[cfg(test)]
    pub(crate) fn broadcast_test_keyframe(self: &Arc<Self>, message: WsOutMessage) {
        self.broadcast(message);
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
                run_completions: HashMap::new(),
                attachments: HashMap::new(),
                next_run_id: 0,
                active_run: None,
                choice_claims: HashMap::new(),
                finished_choice_status: std::collections::VecDeque::new(),
                lease: LeaseState::default(),
                journal: EventJournal::default(),
                recovery_error: None,
            }),
            lease_diagnostics: lease_diagnostics::LeaseDiagnostics::new(
                session_id,
                Some(
                    std::env::temp_dir()
                        .join(session_id)
                        .join(lease_diagnostics::LEASE_DIAGNOSTICS_FILE),
                ),
            ),
            degraded_diagnostics: degraded_diagnostics::DegradedDeliveryDiagnostics::new(
                session_id,
                Some(
                    std::env::temp_dir()
                        .join(session_id)
                        .join(degraded_diagnostics::DEGRADED_DIAGNOSTICS_FILE),
                ),
            ),
            session_id: session_id.to_string(),
            session_record,
            app_paths,
            provider_registry: Arc::new(ProviderRegistry::new()),
            workspace_runs: crate::web::state::WorkspaceRunRegistry::default(),
            next_event_seq: AtomicU64::new(1),
            human_confirm_gate_open: AtomicBool::new(false),
            invalidated_for_deletion: AtomicBool::new(false),
            run_completion_wait_timeout_millis: AtomicU64::new(
                DEFAULT_RUN_COMPLETION_WAIT_TIMEOUT_MILLIS,
            ),
            registry: WorkspaceSessionRegistry::default(),
        });
        manager.spawn_event_router(
            engine_rx,
            crate::web::state::WorkspaceRunRegistry::default(),
        );
        manager
    }
}

/// 批 2 T2（REQ-WIGA-09）：plan 终止引擎侧收口的单 attempt 结果（T3 REST
/// 投影的内部形态；status: aborted/already_terminal/cleanup_failed）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanTerminationAttemptOutcome {
    pub attempt_id: String,
    pub status: &'static str,
    pub reason_code: Option<String>,
    pub message: Option<String>,
}

/// 批 2 T2：终止引擎侧整体结果（session 终止事实＋逐 attempt 收口）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanTerminationEngineOutcome {
    pub session_terminated: bool,
    pub session_confirmed_preserved: bool,
    pub session_error: Option<String>,
    pub attempts: Vec<PlanTerminationAttemptOutcome>,
}

/// 批 2 T2（REQ-WIGA-09，§4 终止顺序①之后的引擎侧收口）：plan 终止的
/// 专用取消契约——T3 统一终止服务在 enrollment CAS（`terminate_plan`）落
/// 盘后调用。步骤：②失效 CodingPlanRun（禁新 run，无 run 幂等）；③取消
/// plan session 活跃 run、等**真实 completion**（不持 engine 锁）、CAS
/// Terminated（已 Confirmed 不改）；④逐 attempt 经 coding registry abort
///（runner 内 AbortAttempt 干净退出，见 runner_support/gates）；⑤逐
/// attempt 专用收口（关门/Aborted/保留文件），单仓异常继续后仓。
pub(crate) async fn terminate_plan_engine_side(
    app_paths: ProductAppPaths,
    session_registry: WorkspaceSessionRegistry,
    coding_runs: crate::web::state::CodingRunRegistry,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
    bound_session_id: Option<&str>,
) -> PlanTerminationEngineOutcome {
    let mut outcome = PlanTerminationEngineOutcome {
        session_terminated: false,
        session_confirmed_preserved: false,
        session_error: None,
        attempts: Vec::new(),
    };
    // ②禁新 run：失效 CodingPlanRun。
    if let Err(error) = crate::product::coding_plan_run::CodingPlanRunStore::new(app_paths.clone())
        .invalidate_run_for_plan(project_id, issue_id, plan_id)
    {
        outcome.session_error = Some(format!("invalidate coding plan run failed: {error}"));
    }
    // ③取消 plan session 活跃 run（真实 completion 等待）＋CAS Terminated。
    if let Some(session_id) = bound_session_id {
        if let Some(manager) = session_registry.get(session_id).await {
            // REQ-WIGA-17：返回类型升格为 RunAbortOutcome；人工终止路径的
            // 既有语义是丢弃取消结果（行为、顺序、文案不变）。
            let _ = manager.abort_active_run_and_wait_completion().await;
        }
        let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(app_paths.clone());
        for _ in 0..3 {
            let record = match lifecycle.get_workspace_session(session_id) {
                Ok(record) => record,
                Err(error) => {
                    outcome
                        .session_error
                        .get_or_insert_with(|| format!("session read failed: {error}"));
                    break;
                }
            };
            match record.status {
                crate::product::models::workspace::WorkspaceSessionStatus::Terminated => {
                    outcome.session_terminated = true;
                    break;
                }
                crate::product::models::workspace::WorkspaceSessionStatus::Confirmed => {
                    outcome.session_confirmed_preserved = true;
                    break;
                }
                _ => {
                    match lifecycle.compare_and_update_workspace_session_status(
                        &record,
                        crate::product::models::workspace::WorkspaceSessionStatus::Terminated,
                    ) {
                        Ok(_) => {
                            outcome.session_terminated = true;
                            break;
                        }
                        Err(crate::product::json_store::ProductStoreError::IdentityMismatch {
                            ..
                        }) => continue,
                        Err(error) => {
                            outcome.session_error.get_or_insert_with(|| {
                                format!("session terminate CAS failed: {error}")
                            });
                            break;
                        }
                    }
                }
            }
        }
    }
    // ④⑤逐 attempt：registry abort（取消 token＋AbortAttempt＋等 completion）
    // 后专用收口；单仓异常继续后仓。
    let coding_store =
        crate::product::coding_attempt_store::CodingAttemptStore::new(app_paths.clone());
    let attempts = match coding_store.list_attempts_for_plan(project_id, issue_id, plan_id) {
        Ok(attempts) => attempts,
        Err(error) => {
            outcome.attempts.push(PlanTerminationAttemptOutcome {
                attempt_id: plan_id.to_string(),
                status: "cleanup_failed",
                reason_code: Some("list_attempts_failed".to_string()),
                message: Some(error.to_string()),
            });
            return outcome;
        }
    };
    let (event_tx, _event_rx) =
        mpsc::channel::<crate::web::coding_ws_handler::CodingWsOutMessage>(64);
    let engine = crate::product::coding_workspace_engine::CodingWorkspaceEngine::new(
        coding_store.clone(),
        crate::product::git_workspace_service::GitWorkspaceService::new(),
        event_tx,
    );
    for attempt in attempts {
        let attempt_key = crate::web::state::CodingAttemptRunKey::new(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        );
        coding_runs.abort_attempt(&attempt_key).await;
        let pre_terminal = matches!(
            attempt.status,
            crate::product::coding_models::CodingAttemptStatus::Completed
                | crate::product::coding_models::CodingAttemptStatus::Failed
                | crate::product::coding_models::CodingAttemptStatus::Aborted
        );
        match engine
            .finalize_attempt_for_plan_termination(project_id, issue_id, &attempt.id)
            .await
        {
            Ok(_) => outcome.attempts.push(PlanTerminationAttemptOutcome {
                attempt_id: attempt.id.clone(),
                status: if pre_terminal {
                    "already_terminal"
                } else {
                    "aborted"
                },
                reason_code: None,
                message: None,
            }),
            Err(error) => outcome.attempts.push(PlanTerminationAttemptOutcome {
                attempt_id: attempt.id.clone(),
                status: "cleanup_failed",
                reason_code: Some("finalize_failed".to_string()),
                message: Some(error.to_string()),
            }),
        }
    }
    outcome
}

/// 保持 `WsOutMessage` schema 不变，在 JSON 顶层增加可选 `event_seq`。
pub(super) fn inject_event_seq(message: String, seq: u64) -> Option<String> {
    let mut value = serde_json::from_str::<serde_json::Value>(&message).ok()?;
    let object = value.as_object_mut()?;
    object.insert("event_seq".to_string(), serde_json::Value::from(seq));
    serde_json::to_string(&value).ok()
}
