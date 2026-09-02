use crate::cross_cutting::streaming_provider::{
    ProviderCompletion, ProviderEvent, ProviderSession, StreamChunk, StreamingProviderInput,
};
use crate::product::app_paths::ProductAppPaths;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    HumanGateTurnStatus, SingleCandidatePhase, WorkspaceSessionRecord, WorkspaceSessionStatus,
};
use crate::product::work_item_plan_policy::PolicyDiagnostic;
use crate::product::work_item_plan_policy::{
    HumanGateSnapshot, HumanReason, RunPolicy, WorkItemPlanFlowKind,
};
use crate::product::workspace_engine::{HumanGateCommandOutcome, HumanGateFeedbackInput};
use crate::protocol::contracts::AdapterInput;
use crate::web::handlers::workspace_session_takeover;
use crate::web::workspace_ws_types::ArtifactPayload;
use std::collections::VecDeque;
use std::sync::Mutex as StdMutex;
use tempfile::TempDir;
use tokio::time::{Duration, timeout};

const REP4_CANDIDATE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/product/work_item_plan_compiler/fixtures/work-item-plan-rep4.md"
));

fn campaign_candidate_v2() -> String {
    campaign_candidate_base().replace("Backend levels API", "Backend levels API round-2")
}

fn campaign_candidate_v3() -> String {
    campaign_candidate_base().replace("Backend levels API", "Backend levels API round-3")
}

/// rep4 fixture 面向 turn 路径校验：其 WI-003 handoff 提供了
/// `contract.levels-integration` 但没有任何消费者 input contract，批准链的
/// canonical 校验（`unconsumed_required_handoff`，Error 级）会拒绝它。
/// campaign 批准链的脚本化候选必须 handoff-clean：逐行剔除该 provided 行
/// （其余逐字保留），使修订后 re-approve 能走通完整 compile→confirm 链。
fn campaign_candidate_base() -> String {
    REP4_CANDIDATE.replace(
        "- provided_contract_refs: contract.levels-integration",
        "- provided_contract_refs: []",
    )
}

/// 脚本化 fake revision provider：按脚本依次返回完整 SC markdown 候选、
/// validation reject（坏 markdown）、transport 死亡或挂起（单飞）。
/// 只经测试构造器注入，生产路径不读取。
enum RevisionScriptStep {
    Complete(String),
    ValidationReject,
    TransportDeath,
    Hang,
}

struct ScriptedRevisionProvider {
    script: StdMutex<VecDeque<RevisionScriptStep>>,
    starts: StdMutex<Vec<usize>>,
    hang_release: Arc<tokio::sync::Notify>,
    hang_entered: Arc<tokio::sync::Notify>,
}

impl ScriptedRevisionProvider {
    fn new(script: Vec<RevisionScriptStep>) -> Arc<Self> {
        Arc::new(Self {
            script: StdMutex::new(script.into()),
            starts: StdMutex::new(Vec::new()),
            hang_release: Arc::new(tokio::sync::Notify::new()),
            hang_entered: Arc::new(tokio::sync::Notify::new()),
        })
    }

    fn start_count(&self) -> usize {
        self.starts.lock().expect("starts lock").len()
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ScriptedRevisionProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let attempt = self.starts.lock().expect("starts lock").len() + 1;
        self.starts.lock().expect("starts lock").push(attempt);
        let step = self
            .script
            .lock()
            .expect("script lock")
            .pop_front()
            .unwrap_or(RevisionScriptStep::Complete(REP4_CANDIDATE.to_string()));
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        match step {
            RevisionScriptStep::Complete(output) => {
                tokio::spawn(async move {
                    let _ = event_tx
                        .send(ProviderEvent::TextDelta {
                            content: output.clone(),
                        })
                        .await;
                    let _ = event_tx
                        .send(ProviderEvent::Completed(ProviderCompletion::plain(
                            output, None,
                        )))
                        .await;
                });
            }
            RevisionScriptStep::ValidationReject => {
                tokio::spawn(async move {
                    let _ = event_tx
                        .send(ProviderEvent::Completed(ProviderCompletion::plain(
                            "# 不是合法的 Work Item Plan\n".to_string(),
                            None,
                        )))
                        .await;
                });
            }
            RevisionScriptStep::TransportDeath => {
                tokio::spawn(async move {
                    let _ = event_tx
                        .send(ProviderEvent::Failed {
                            message: "campaign scripted transport death".to_string(),
                        })
                        .await;
                });
            }
            RevisionScriptStep::Hang => {
                let entered = self.hang_entered.clone();
                let release = self.hang_release.clone();
                tokio::spawn(async move {
                    entered.notify_one();
                    tokio::select! {
                        _ = cancel.cancelled() => {}
                        _ = release.notified() => {
                            let _ = event_tx
                                .send(ProviderEvent::Completed(ProviderCompletion::plain(
                                    REP4_CANDIDATE.to_string(),
                                    None,
                                )))
                                .await;
                        }
                    }
                });
            }
        }
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "campaign tests use start only",
            0,
        ))
    }
}

/// 每步审计行（harness contract）；candidate 只保存 digest/ref。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct CampaignStepAudit {
    scenario_id: &'static str,
    command_id: String,
    turn_id: Option<String>,
    attempt_no: Option<u32>,
    budget_before: u32,
    budget_after: Option<u32>,
    provider_start_keys: Vec<String>,
    event_prefix_digest: String,
    artifact_ref: Option<String>,
    observed_status: String,
}

struct CampaignStage3Harness {
    root: TempDir,
    app_paths: ProductAppPaths,
    lifecycle: LifecycleStore,
    engine: Arc<Mutex<WorkspaceEngine>>,
    provider: Arc<ScriptedRevisionProvider>,
    outbound_tx: mpsc::Sender<OutboundControl>,
    outbound_rx: tokio::sync::Mutex<mpsc::Receiver<OutboundControl>>,
    session_id: String,
    project_id: String,
    issue_id: String,
}

/// 真实批准链基座（accepted contract drafts → SC Approval 门），镜像
/// `conversational_gate_amendment_real_chain::real_approval_fixture`：
/// 生产 SC 流在门开启前经 `update_artifact(Markdown)` 持久化候选文本。
async fn campaign_stage3_fixture(
    budget: u32,
    script: Vec<RevisionScriptStep>,
) -> CampaignStage3Harness {
    let (root, lifecycle, _plan_id, mut engine) =
        crate::product::workspace_engine::tests::make_work_item_plan_engine_with_accepted_contract_drafts();
    let app_paths = lifecycle.app_paths();
    crate::product::workspace_engine::tests::single_candidate_recovery::single_candidate_recovery_record(
        &lifecycle,
        &mut engine,
        SingleCandidatePhase::Approval,
        RunPolicy::Interactive,
    );
    // 生产 SC 流在门开启前已把门候选文本持久化为 session 的当前
    // source/IR/mechanical-report 三 refs（8.2c 恢复的 fail-closed 源哈希核对、
    // 批准链 compile 都以它为准）。recovery 式 fixture 只落了占位 source；
    // 这里用真实门候选文本铺出同一 durable 形态。必须在 update_artifact
    // （markdown 候选）之前执行：refs 构造要读 outline 形态的 artifact。
    let gate_refs =
        crate::product::workspace_engine::tests::single_candidate_recovery::single_candidate_recovery_persist_candidate_artifacts(
            &lifecycle,
            &engine,
            "campaign-gate",
            REP4_CANDIDATE,
        );
    crate::product::workspace_engine::tests::single_candidate_recovery::single_candidate_recovery_update_refs(
        &lifecycle,
        &mut engine,
        SingleCandidatePhase::Approval,
        gate_refs,
    );
    engine
        .update_artifact(ArtifactPayload::Markdown {
            markdown: REP4_CANDIDATE.to_string(),
            diff: None,
        })
        .await;
    let artifact = engine.session.artifact.clone();
    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("session record");
    record.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
    record.run_policy = RunPolicy::Interactive;
    record.single_candidate_phase = Some(SingleCandidatePhase::Approval);
    record.status = WorkspaceSessionStatus::WaitingForHuman;
    record.human_gate_snapshot = Some(HumanGateSnapshot {
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: budget,
        trigger: HumanReason::NativeHumanRequired,
        resumable: true,
    });
    let session_path = app_paths
        .issue_root(&record.project_id, &record.issue_id)
        .join("workspace-sessions")
        .join(format!("{}.json", record.id));
    crate::product::json_store::write_json(&session_path, &record)
        .expect("persist campaign gate session");
    let (event_tx, _event_rx) = mpsc::channel(64);
    let mut session = WorkspaceSession::from_record(record.clone());
    session.stage = WorkspaceStage::HumanConfirm;
    session.session_status = WorkspaceSessionStatus::WaitingForHuman;
    session.artifact = artifact;
    let engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        lifecycle.clone(),
        event_tx,
        session,
    );
    let engine = Arc::new(Mutex::new(engine));
    let provider = ScriptedRevisionProvider::new(script);
    let (outbound_tx, outbound_rx) = mpsc::channel(256);
    CampaignStage3Harness {
        root,
        app_paths,
        lifecycle,
        engine,
        provider,
        outbound_tx,
        outbound_rx: tokio::sync::Mutex::new(outbound_rx),
        session_id: record.id.clone(),
        project_id: record.project_id.clone(),
        issue_id: record.issue_id.clone(),
    }
}

impl CampaignStage3Harness {
    /// 每条入站消息一个全新 context（context 按值消费），共享 harness 的
    /// outbound channel 与引擎。
    async fn send(&self, message: WsInMessage) {
        let record = self.session_record().await;
        let current_run = Arc::new(Mutex::new(None));
        let workspace_runs = WorkspaceRunRegistry::default();
        let mut registry = ProviderRegistry::new();
        registry.register(
            ProviderName::ClaudeCode,
            self.provider.clone() as Arc<dyn StreamingProviderAdapter>,
        );
        let context = WorkspaceInboundContext {
            app_state: WebAppState::new(
                self.root.path().to_path_buf(),
                crate::web::runtime::WebRuntime::new_fake(self.root.path().to_path_buf()),
            ),
            engine: self.engine.clone(),
            run_context: ProviderRunContext {
                provider_registry: Arc::new(registry),
                engine: self.engine.clone(),
                current_run: current_run.clone(),
                workspace_runs: workspace_runs.clone(),
                session_id: self.session_id.clone(),
                next_run_id: Arc::new(Mutex::new(0)),
                app_paths: self.app_paths.clone(),
                session_record: record,
            },
            outbound_tx: self.outbound_tx.clone(),
            current_run,
            workspace_runs,
            session_id: self.session_id.clone(),
        };
        handle_workspace_inbound_message(context, message).await;
    }

    /// 模拟第二个 ws worker（独立 engine 实例，同一 durable session）：
    /// 单飞期 in-flight turn 的 busy 判定来自 durable turn 状态，不依赖共享内存锁。
    async fn send_isolated_worker(&self, message: WsInMessage) {
        let record = self.session_record().await;
        let (event_tx, _event_rx) = mpsc::channel(64);
        let mut session = WorkspaceSession::from_record(record.clone());
        session.stage = WorkspaceStage::HumanConfirm;
        session.session_status = WorkspaceSessionStatus::WaitingForHuman;
        session.artifact = Some(ArtifactPayload::Markdown {
            markdown: REP4_CANDIDATE.to_string(),
            diff: None,
        });
        let engine = Arc::new(Mutex::new(WorkspaceEngine::new_persistent(
            Arc::new(CheckpointStore::new(
                self.root.path().join("isolated-worker-checkpoints"),
            )),
            self.lifecycle.clone(),
            event_tx,
            session,
        )));
        let current_run = Arc::new(Mutex::new(None));
        let workspace_runs = WorkspaceRunRegistry::default();
        let mut registry = ProviderRegistry::new();
        registry.register(
            ProviderName::ClaudeCode,
            self.provider.clone() as Arc<dyn StreamingProviderAdapter>,
        );
        let context = WorkspaceInboundContext {
            app_state: WebAppState::new(
                self.root.path().to_path_buf(),
                crate::web::runtime::WebRuntime::new_fake(self.root.path().to_path_buf()),
            ),
            engine: engine.clone(),
            run_context: ProviderRunContext {
                provider_registry: Arc::new(registry),
                engine,
                current_run: current_run.clone(),
                workspace_runs: workspace_runs.clone(),
                session_id: self.session_id.clone(),
                next_run_id: Arc::new(Mutex::new(0)),
                app_paths: self.app_paths.clone(),
                session_record: record,
            },
            outbound_tx: self.outbound_tx.clone(),
            current_run,
            workspace_runs,
            session_id: self.session_id.clone(),
        };
        handle_workspace_inbound_message(context, message).await;
    }

    async fn session_record(&self) -> WorkspaceSessionRecord {
        self.lifecycle
            .get_workspace_session(&self.session_id)
            .expect("durable session")
    }

    fn session_record_blocking(&self) -> WorkspaceSessionRecord {
        self.lifecycle
            .get_workspace_session(&self.session_id)
            .expect("durable session")
    }

    async fn next_outbound(&self) -> WsOutMessage {
        let mut rx = self.outbound_rx.lock().await;
        let outbound = timeout(Duration::from_secs(10), rx.recv())
            .await
            .expect("outbound within timeout")
            .expect("outbound channel open");
        let OutboundControl::Text(json) = outbound else {
            panic!("expected text outbound");
        };
        serde_json::from_str(&json).expect("outbound ws json")
    }

    /// 跳过非门事件（session_state/provider_status/stream），返回下一个匹配事件。
    async fn await_gate_event(&self, kind: &str) -> WsOutMessage {
        loop {
            let message = self.next_outbound().await;
            let r#type = serde_json::to_value(&message)
                .expect("serialize outbound")
                .get("type")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string();
            if r#type == kind {
                return message;
            }
        }
    }

    fn durable_turns(&self) -> Vec<crate::product::models::HumanGateTurn> {
        self.lifecycle
            .list_human_gate_turns(&self.session_id)
            .expect("durable turns")
    }

    fn session_bytes(&self) -> Vec<u8> {
        let path = self
            .app_paths
            .issue_root(&self.project_id, &self.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", self.session_id));
        std::fs::read(&path).expect("session bytes")
    }

    fn turn_bytes(&self, turn_id: &str) -> Vec<u8> {
        let path = self
            .app_paths
            .issue_root(&self.project_id, &self.issue_id)
            .join("workspace-sessions")
            .join(&self.session_id)
            .join("human-gate-turns")
            .join(format!("{turn_id}.json"));
        std::fs::read(&path).expect("turn bytes")
    }

    fn provider_start_keys(&self) -> Vec<String> {
        self.session_record_blocking()
            .provider_start_ledger
            .iter()
            .map(|entry| entry.provider_start_idempotency_key.clone())
            .collect()
    }

    fn budget_remaining(&self) -> u32 {
        self.session_record_blocking()
            .human_gate_snapshot
            .as_ref()
            .expect("snapshot")
            .manual_repairs_remaining
    }
}

#[allow(clippy::too_many_arguments)]
fn campaign_step_audit(
    scenario_id: &'static str,
    command_id: &str,
    turn: Option<&crate::product::models::HumanGateTurn>,
    budget_before: u32,
    budget_after: Option<u32>,
    provider_start_keys: Vec<String>,
    event_prefix: &[u8],
    observed_status: &str,
) -> CampaignStepAudit {
    use sha2::{Digest, Sha256};
    CampaignStepAudit {
        scenario_id,
        command_id: command_id.to_string(),
        turn_id: turn.map(|turn| turn.turn_id.clone()),
        attempt_no: turn.map(|turn| turn.attempt_no),
        budget_before,
        budget_after,
        provider_start_keys,
        event_prefix_digest: hex::encode(Sha256::digest(event_prefix)),
        artifact_ref: turn.and_then(|turn| turn.result_artifact_ref.clone()),
        observed_status: observed_status.to_string(),
    }
}
