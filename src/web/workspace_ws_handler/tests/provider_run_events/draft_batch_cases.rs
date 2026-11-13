//! P4（任务5，brief task-5）：draft/batch 两族臂的行为门禁（characterization
//! 与守恒锚）。本文件**不引用**新臂符号——全部经真实 spawn 链驱动
//! （`spawn_provider_run_from_event`→小外壳→task body→boxed drive→臂），
//! 迁移前即可编译运行（brief Step 2「行为对照迁移前先绿作为保护」），
//! 迁移后必须保持零差异。
//!
//! - `fresh_draft_batch_payload_and_retry_once_are_preserved`：durable 冻结
//!   feedback 与刻意不同的内存 pending 构造输入——断言使用冻结值
//!   （`Option<Option<String>>` 的外层=「fresh 冻结载荷在场」、内层=冻结值
//!   本身，内存 pending 不读不 take）、provider 全新启动（resume=None）、
//!   RetryOnce 按原预算循环（恰一轮重试后收敛，不第三轮）且每轮新
//!   split handle（Logical 启动路下两轮两个不同 role_run_seq 的 durable
//!   run 记录）、不新建业务 generation/batch；batch 的 AwaitConfirmation
//!   仍 feedback=None 继续原 loop（对拍 draft 的 break——两者不统一）。
//! - `draft_batch_early_stop_does_not_finish_twice`：load/build/start/output/
//!   parse 失败与 AwaitConfirmation 逐分支锚定原 tail（mark/drop/Error 帧
//!   的原顺序与有无），Stop 后零后续 start、不补二次 finish/错误帧。
//!
//! 三守卫不可混用（r16 现场死锁根修，缺陷#10家族——设计 §3.4）：engine
//! MutexGuard 按值属 drive/当前臂；idle `provider_drive_guard` 是外层 Option
//! 的 `&mut`；`entity_run_lease` 是独立外层 binding（不传给臂）。

use super::super::single_candidate_lc_admission::{
    AliasLcGatewayExtras, LcDialectCapabilitySource,
};
use super::*;
use crate::product::logical_codebase::LogicalRepositoryId;
use crate::product::models::{
    LcRecoveryBusinessContext, LcRecoveryDraftRef, LcRecoveryRunContext, LcResumeRecoveryRecord,
    LcResumeRecoveryState, ProviderConversationRef, ProviderConversationRole,
    TrustedDraftVerificationCommand, WorkItemBatchRecord, WorkItemBatchStatus,
    WorkItemDraftCandidate, WorkItemDraftRecord, WorkItemDraftStatus,
    WorkItemDraftVerificationPlan, WorkItemGenerationMode, WorkItemKind, WorkItemOutline,
    WorkItemOutlineSessionFit, WorkItemPlanDraftActiveIndex, WorkItemPlanOutline, WorkspaceType,
};
use crate::web::workspace_ws_types::{
    ArtifactPayload, TimelineNodeType, WorkItemPlanOutlineCandidateDto,
};
use std::collections::{BTreeMap, VecDeque};

// ---------------------------------------------------------------------------
// 行为夹具：provider / 轮次脚本 / 业务种子 / 恢复记录
// ---------------------------------------------------------------------------

/// 每轮 start 的输出脚本（按轮次弹出；fresh/失败支共用一个 provider 形态）。
enum CaseDraftRound {
    /// sentinel 包裹的单条 draft 载荷（empty_write_scopes=true 触发本地校验
    /// 错误 → RetryOnce；false 为可完整收敛的合法候选）。
    DraftPayload {
        outline_id: String,
        /// outline 条目的 title（合法候选的 canonical identity 必须与 outline
        /// identity 全等——title/kind 匹配是 draft 本地校验门）。
        title: String,
        logical_work_item_id: String,
        target_repository_id: Option<LogicalRepositoryId>,
        empty_write_scopes: bool,
    },
    /// 无 sentinel 纯文本（`parse_work_item_split_structured_output` 失败支）。
    PlainText,
    /// sentinel 包裹但 draft 载荷非法（`parse_work_item_draft_output` 失败支）。
    InvalidDraftPayload,
}

/// 记录 input 的 draft/batch provider：逐轮按脚本产出 sentinel 结构化输出；
/// `fail_start=true` 时 start 直接 Err（provider start/驱动失败支）。
struct CaseDraftBatchProvider {
    rounds: std::sync::Mutex<VecDeque<CaseDraftRound>>,
    starts: Arc<AtomicUsize>,
    inputs: mpsc::UnboundedSender<StreamingProviderInput>,
    fail_start: bool,
}

impl CaseDraftBatchProvider {
    fn next_output(&self, prompt: &str) -> String {
        const NONCE_TAG: &str = "<ARIA_STRUCTURED_OUTPUT nonce=\"";
        let nonce = prompt
            .rfind(NONCE_TAG)
            .map(|start| {
                prompt[start + NONCE_TAG.len()..]
                    .split('"')
                    .next()
                    .unwrap_or_default()
                    .to_string()
            })
            .unwrap_or_default();
        assert!(
            !nonce.is_empty(),
            "draft/batch prompt 必须携带 sentinel nonce"
        );
        let round = self
            .rounds
            .lock()
            .expect("case rounds")
            .pop_front()
            .expect("case scripted round exhausted");
        match round {
            CaseDraftRound::DraftPayload {
                outline_id,
                title,
                logical_work_item_id,
                target_repository_id,
                empty_write_scopes,
            } => {
                let mut contract = crate::product::work_item_contract::canonical_contract_fixture(
                    &logical_work_item_id,
                );
                contract.identity.title = title;
                contract.identity.kind = "backend".to_string();
                contract.input_contracts.clear();
                contract.handoff_contract.provided_contract_refs.clear();
                contract.write_policy.exclusive_scopes = if empty_write_scopes {
                    Vec::new()
                } else {
                    vec![format!("src/{outline_id}.rs")]
                };
                let checks = contract.verification_checks.clone();
                structured_output_sentinel(
                    &nonce,
                    &serde_json::json!({
                        "draft": {
                            "outline_id": outline_id,
                            "logical_work_item_id": logical_work_item_id,
                            "target_repository_id": target_repository_id,
                            "canonical_contract": contract,
                            "verification_plan": { "checks": checks }
                        }
                    }),
                )
            }
            CaseDraftRound::PlainText => {
                "case plain output without structured sentinel".to_string()
            }
            CaseDraftRound::InvalidDraftPayload => structured_output_sentinel(
                &nonce,
                &serde_json::json!({ "drafts": Vec::<u8>::new() }),
            ),
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for CaseDraftBatchProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        let _ = self.inputs.send(input.clone());
        if self.fail_start {
            return Err(ProviderAdapterError::provider_unavailable(
                "case draft/batch provider start failure".to_string(),
            ));
        }
        let output = self.next_output(&input.prompt);
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(1);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::Completed(ProviderCompletion::plain(
                    output, None,
                )))
                .await;
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }

    /// gateway 启动经 `start_validated` 分发：拆出 input 后与裸 start 同路径。
    async fn start_validated(
        &self,
        validated: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (input, _launch) = validated.into_parts();
        self.start(input, cancel).await
    }

    async fn run_streaming(
        &self,
        _input: &crate::protocol::contracts::AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        unreachable!("workspace runs use start")
    }
}

/// 单条 outline 的最小权威材料（trusted 目录覆盖 canonical 夹具的 required
/// 命令——缺目录会被本地校验拒为 operational 错误，无法构成合法轮）。
fn case_outline_item(
    outline_id: &str,
    logical_work_item_id: &str,
    title: &str,
    scope: &str,
    target: Option<LogicalRepositoryId>,
    story_id: &str,
    design_id: &str,
) -> WorkItemOutline {
    let trusted_command =
        crate::product::work_item_contract::canonical_contract_fixture(logical_work_item_id)
            .verification_checks[0]
            .command
            .clone()
            .unwrap_or_default();
    WorkItemOutline {
        target_repository_id: target,
        outline_id: outline_id.to_string(),
        logical_work_item_id: logical_work_item_id.to_string(),
        title: title.to_string(),
        kind: WorkItemKind::Backend,
        goal: format!("{title} goal"),
        scope: vec![scope.to_string()],
        non_goals: Vec::new(),
        estimated_context_tokens: Some(12_000),
        session_fit: Some(WorkItemOutlineSessionFit::FitsSingleAgentSession),
        source_story_spec_ids: vec![story_id.to_string()],
        source_design_spec_ids: vec![design_id.to_string()],
        exclusive_write_scopes: vec![scope.to_string()],
        forbidden_write_scopes: Vec::new(),
        depends_on: Vec::new(),
        verification_intent: vec![format!("cargo test --locked --lib {outline_id}")],
        trusted_verification_commands: vec![TrustedDraftVerificationCommand {
            command: trusted_command,
            cwd: String::new(),
            purpose: format!("{outline_id} trusted check"),
            source_ref: "case-trust".to_string(),
        }],
        handoff_notes: format!("handoff {outline_id}"),
    }
}

/// 注入 engine session 的 outline 候选 artifact（draft/batch 输入构建与完成
/// 校验的比对基准）。
fn case_outline_artifact(
    items: Vec<WorkItemOutline>,
    story_id: &str,
    design_id: &str,
) -> ArtifactPayload {
    ArtifactPayload::WorkItemPlanOutlineCandidate {
        outline_candidate: Box::new(WorkItemPlanOutlineCandidateDto {
            outline: WorkItemPlanOutline {
                id: "outline_p5".to_string(),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                source_story_spec_ids: vec![story_id.to_string()],
                source_design_spec_ids: vec![design_id.to_string()],
                strategy_summary: "P5 draft/batch case strategy".to_string(),
                work_item_outlines: items,
                dependency_graph: Vec::new(),
                risks: Vec::new(),
                handoff_strategy: "serial handoff".to_string(),
                status: "confirmed".to_string(),
            },
            design_context_gaps: Vec::new(),
            validator_findings: Vec::new(),
            context_blockers: Vec::new(),
            current_generation_round_id: Some("round_0001".to_string()),
            selected_generation_mode: None,
        }),
    }
}

/// 已接受 draft 记录（anchor_t16 同构：canonical 夹具 + 清依赖声明）。
fn case_accepted_draft_record(
    plan_id: &str,
    outline: &WorkItemOutline,
    batch_id: Option<&str>,
) -> WorkItemDraftRecord {
    let now = "2026-11-06T00:00:00Z";
    let mut contract = crate::product::work_item_contract::canonical_contract_fixture(
        &outline.logical_work_item_id,
    );
    contract.identity.title = outline.title.clone();
    contract.identity.kind = "backend".to_string();
    contract.goal.summary = format!("实现 {}", outline.outline_id);
    contract.input_contracts.clear();
    contract.handoff_contract.provided_contract_refs.clear();
    contract.write_policy.exclusive_scopes = outline.exclusive_write_scopes.clone();
    WorkItemDraftRecord {
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        plan_id: plan_id.to_string(),
        draft_id: format!("draft_{}", outline.outline_id),
        outline_id: outline.outline_id.clone(),
        generation_round_id: "round_0001".to_string(),
        batch_id: batch_id.map(str::to_string),
        attempt_index: 1,
        outline_version_ref: "outline_p5".to_string(),
        generation_mode: if batch_id.is_some() {
            WorkItemGenerationMode::Batch
        } else {
            WorkItemGenerationMode::Serial
        },
        generation_diagnostics: None,
        candidate: WorkItemDraftCandidate {
            target_repository_id: outline.target_repository_id,
            outline_id: outline.outline_id.clone(),
            logical_work_item_id: outline.logical_work_item_id.clone(),
            canonical_contract_candidate: contract.clone(),
            verification_plan: WorkItemDraftVerificationPlan {
                checks: contract.verification_checks,
            },
        },
        status: WorkItemDraftStatus::Accepted,
        active: true,
        superseded_by_draft_id: None,
        supersede_reason: None,
        copied_from_draft_id: None,
        review_node_id: None,
        review_verdict_ref: None,
        generated_from_node_id: "timeline_node_p5_seed".to_string(),
        accepted_at: Some(now.to_string()),
        superseded_at: None,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    }
}

/// serial draft 场景的业务种子：active index（round_0001 + 活动 outline +
/// 已接受 draft 记录）。
fn case_seed_serial_business(fixture: &ProviderRunFixture, outline: &WorkItemOutline) {
    let store =
        crate::product::work_item_plan_store::WorkItemPlanStore::new(fixture.app_paths.clone());
    store
        .put_draft_record(&case_accepted_draft_record(
            &fixture.record.entity_id,
            outline,
            None,
        ))
        .expect("put accepted draft record");
    store
        .save_active_index(&WorkItemPlanDraftActiveIndex {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: fixture.record.entity_id.clone(),
            current_generation_round_id: "round_0001".to_string(),
            outline_state: "confirmed".to_string(),
            active_outline_id: Some(outline.outline_id.clone()),
            outline_to_current_draft_id: BTreeMap::from([(
                outline.outline_id.clone(),
                format!("draft_{}", outline.outline_id),
            )]),
            draft_statuses: BTreeMap::from([(
                format!("draft_{}", outline.outline_id),
                WorkItemDraftStatus::Accepted,
            )]),
            batches: Vec::new(),
            updated_at: "2026-11-06T00:00:00Z".to_string(),
        })
        .expect("save serial active index");
}

/// batch 场景的业务种子：batch_0001（Generating，含已接受的首条 item draft）。
fn case_seed_batch_business(fixture: &ProviderRunFixture, first_outline: &WorkItemOutline) {
    let store =
        crate::product::work_item_plan_store::WorkItemPlanStore::new(fixture.app_paths.clone());
    store
        .put_draft_record(&case_accepted_draft_record(
            &fixture.record.entity_id,
            first_outline,
            Some("batch_0001"),
        ))
        .expect("put batch draft record");
    store
        .save_active_index(&WorkItemPlanDraftActiveIndex {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: fixture.record.entity_id.clone(),
            current_generation_round_id: "round_0001".to_string(),
            outline_state: "confirmed".to_string(),
            active_outline_id: Some(first_outline.outline_id.clone()),
            outline_to_current_draft_id: BTreeMap::from([(
                first_outline.outline_id.clone(),
                format!("draft_{}", first_outline.outline_id),
            )]),
            draft_statuses: BTreeMap::from([(
                format!("draft_{}", first_outline.outline_id),
                WorkItemDraftStatus::Accepted,
            )]),
            batches: vec![WorkItemBatchRecord {
                batch_id: "batch_0001".to_string(),
                generation_round_id: "round_0001".to_string(),
                mode: WorkItemGenerationMode::Batch,
                item_draft_ids: vec![format!("draft_{}", first_outline.outline_id)],
                status: WorkItemBatchStatus::Generating,
                validation_failed_ids: Vec::new(),
                created_at: "2026-11-06T00:00:00Z".to_string(),
            }],
            updated_at: "2026-11-06T00:00:00Z".to_string(),
        })
        .expect("save batch active index");
}

/// FreshRunning 恢复记录落盘（typed fresh 支的臂内消费源；权威 CAS 写）。
fn case_seed_fresh_recovery(
    fixture: &ProviderRunFixture,
    recovery_id: &str,
    run_context: LcRecoveryRunContext,
) {
    let expected = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("baseline session record");
    let recovery = LcResumeRecoveryRecord {
        schema_version: 1,
        recovery_id: recovery_id.to_string(),
        failed_provider_node_id: "timeline_node_p5_failed".to_string(),
        reason_code: "planning_context_stale".to_string(),
        role: ProviderConversationRole::Author,
        provider: ProviderName::ClaudeCode,
        superseded_native_session_id: Some("native-p5-old".to_string()),
        previous_fingerprint_digest: Some("sha256:p5-old".to_string()),
        previous_session_status: expected.status.clone(),
        previous_stage: "running".to_string(),
        run_context,
        state: LcResumeRecoveryState::FreshRunning,
        operation_id: None,
        fresh_node_id: None,
        fresh_native_session_id: None,
        fresh_role_run_seq: None,
        fresh_operations: Vec::new(),
        detected_at: "2026-11-06T00:00:00Z".to_string(),
        updated_at: "2026-11-06T00:00:00Z".to_string(),
    };
    fixture
        .lifecycle
        .compare_and_save_lc_resume_state(&expected, recovery)
        .expect("seed FreshRunning recovery");
}

/// 恢复记录的业务上下文（P5 最小权威值）。
fn case_recovery_business(fixture: &ProviderRunFixture) -> LcRecoveryBusinessContext {
    LcRecoveryBusinessContext {
        entity_id: fixture.record.entity_id.clone(),
        plan_id: Some(fixture.record.entity_id.clone()),
        workspace_type: WorkspaceType::WorkItemPlan,
        business_repository_physical_id: None,
        business_repository_logical_id: None,
        business_checkout_id: None,
        business_repository_path: None,
        source_story_ids: vec![fixture.story_id.clone()],
        source_design_ids: vec![fixture.design_id.clone()],
        plan_options_ref: None,
        artifact_refs: Vec::new(),
        review_refs: Vec::new(),
        author_provider: ProviderName::ClaudeCode,
        effective_reviewer_provider: Some(ProviderName::Codex),
        original_node_id: Some("timeline_node_p5_failed".to_string()),
        original_role_run_seq: None,
    }
}

/// 旧 Author native 会话引用（fresh resume=None 的对账锚点）。
fn case_old_author_conversation_ref() -> ProviderConversationRef {
    ProviderConversationRef {
        role: ProviderConversationRole::Author,
        provider: ProviderName::ClaudeCode,
        provider_session_id: "native-p5-old".to_string(),
        updated_at: "2026-11-06T00:00:00Z".to_string(),
        last_node_id: Some("timeline_node_p5_failed".to_string()),
    }
}

/// 直驱 `spawn_provider_run_from_event` 的 run_context（真实 spawn 链）。
fn case_event_run_context(
    fixture: &ProviderRunFixture,
    provider: Arc<dyn StreamingProviderAdapter>,
) -> ProviderRunContext {
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, provider);
    let mut run_context = ProviderRunContext::test_fixture(
        Arc::new(registry),
        fixture.engine.clone(),
        fixture.workspace_runs.clone(),
        fixture.record.id.clone(),
        fixture.app_paths.clone(),
        fixture.record.clone(),
    );
    run_context.manager = fixture.manager.clone();
    run_context
}

/// LC 别名夹具的唯一成员 logical id（draft target 同源锚点，REQ-PLN-02）。
fn case_member_logical_id(fixture: &ProviderRunFixture) -> LogicalRepositoryId {
    crate::product::repository_store::RepositoryStore::new(fixture.app_paths.clone())
        .list("project_0001")
        .expect("alias member projection")
        .into_iter()
        .find_map(|record| record.logical_repository_id)
        .expect("alias fixture registers exactly one logical member")
}

// ---------------------------------------------------------------------------
// 行为观察助手
// ---------------------------------------------------------------------------

/// 等待 manager 注册被 finish_run 清空（Stop/正常尾部收口的可观测面）。
async fn case_wait_run_registration_cleared(
    manager: &Arc<crate::web::workspace_session::WorkspaceSessionManager>,
) {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while manager.active_run().await.is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("run 注册必须被 finish_run 清空");
}

/// 读取下一条 type=="error" 的出站帧消息文本。
async fn case_next_error_message(outbound_rx: &mut mpsc::Receiver<OutboundControl>) -> String {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let outbound = outbound_rx.recv().await.expect("outbound control expected");
            let OutboundControl::Text(json) = outbound else {
                continue;
            };
            let value: serde_json::Value = serde_json::from_str(&json).expect("outbound json");
            if value["type"] == "error" {
                return value["message"].as_str().unwrap_or_default().to_string();
            }
        }
    })
    .await
    .expect("error outbound expected")
}

/// 观察窗口内不得出现 error 帧（不补错误帧分支的负锚）。
async fn case_assert_no_error_frame(outbound_rx: &mut mpsc::Receiver<OutboundControl>, note: &str) {
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(250), outbound_rx.recv()).await,
            Ok(Some(OutboundControl::Text(json)))
                if serde_json::from_str::<serde_json::Value>(&json)
                    .map(|value| value["type"] == "error")
                    .unwrap_or(false)
        ),
        "{note}"
    );
}

/// 观察窗口内不得再出现 provider input（Stop 后零后续 start / 无 followups）。
async fn case_assert_no_more_provider_inputs(
    input_rx: &mut mpsc::UnboundedReceiver<StreamingProviderInput>,
    note: &str,
) {
    if let Ok(Some(input)) =
        tokio::time::timeout(std::time::Duration::from_millis(250), input_rx.recv()).await
    {
        let head: String = input.prompt.chars().take(160).collect();
        panic!("{note}——意外 input prompt[..]={head}");
    }
}

/// Logical 启动路的 durable split run 记录（(role_run_seq, status) 升序）。
fn case_split_runs(app_paths: &ProductAppPaths, session_id: &str) -> Vec<(String, String)> {
    let dir = app_paths
        .root()
        .join("work-item-split-runs")
        .join(session_id);
    let mut runs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let seq = entry.file_name().to_string_lossy().to_string();
            let Ok(json) = std::fs::read_to_string(entry.path().join("run.json")) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&json) else {
                continue;
            };
            runs.push((
                seq,
                value["status"].as_str().unwrap_or_default().to_string(),
            ));
        }
    }
    runs.sort();
    runs
}

/// 重读业务 active index。
fn case_active_index(fixture: &ProviderRunFixture) -> WorkItemPlanDraftActiveIndex {
    crate::product::work_item_plan_store::WorkItemPlanStore::new(fixture.app_paths.clone())
        .load_active_index("project_0001", "issue_0001", &fixture.record.entity_id)
        .expect("load active index")
        .expect("active index present")
}

/// 收口后 engine 锁必须立即可重取（三守卫合同的可观测面）。
async fn case_assert_engine_lock_reacquirable(fixture: &ProviderRunFixture, note: &str) {
    if tokio::time::timeout(std::time::Duration::from_secs(1), async {
        let _guard = fixture.engine.lock().await;
    })
    .await
    .is_err()
    {
        panic!("{note}——engine 锁必须可重取");
    }
}

// ---------------------------------------------------------------------------
// 用例 1（brief Step 1）：fresh 冻结载荷 / RetryOnce / 每轮新 split handle
// ---------------------------------------------------------------------------

/// P4 行为合同总用例：两个子场景各自独立夹具。
#[tokio::test]
async fn fresh_draft_batch_payload_and_retry_once_are_preserved() {
    draft_fresh_uses_frozen_feedback_retry_once_budget_and_new_split_handles().await;
    batch_fresh_uses_frozen_payload_keeps_batch_and_continues_loop().await;
}

/// draft fresh（Logical 启动路）：durable 冻结 feedback 优先于内存 pending，
/// resume=None；两轮本地校验失败恰消耗一次 RetryOnce 预算后收敛（无第三轮），
/// 每轮 begin 新 split handle；业务 generation/批次零重建；AwaitConfirmation
/// 走 draft 的 break（落 WorkItemDraftConfirm 后 Continue 正常尾部）。
async fn draft_fresh_uses_frozen_feedback_retry_once_budget_and_new_split_handles() {
    let starts = Arc::new(AtomicUsize::new(0));
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(CaseDraftBatchProvider {
        rounds: std::sync::Mutex::new(VecDeque::new()),
        starts: starts.clone(),
        inputs: input_tx,
        fail_start: false,
    });
    let (fixture, mut engine_rx) =
        ProviderRunFixture::new_logical_legacy_with_gateway_extras_and_engine_rx(
            true,
            provider.clone(),
            AliasLcGatewayExtras {
                capability: Some(Arc::new(LcDialectCapabilitySource)),
                ..AliasLcGatewayExtras::default()
            },
        );
    // 持续 drain engine 事件（夹具要求）。
    tokio::spawn(async move { while engine_rx.recv().await.is_some() {} });
    let member = case_member_logical_id(&fixture);
    let outline = case_outline_item(
        "outline_a",
        "wi_a",
        "P5-A",
        "src/a.rs",
        Some(member),
        &fixture.story_id,
        &fixture.design_id,
    );
    {
        let mut engine = fixture.engine.lock().await;
        engine.update_artifact(case_outline_artifact(
            vec![outline.clone()],
            &fixture.story_id,
            &fixture.design_id,
        )).await.expect("fixture artifact write");
        engine.session.provider_conversations = vec![case_old_author_conversation_ref()];
        engine.pending_revision_context =
            Some("P5内存残留反馈（不得进入draft fresh输入）".to_string());
    }
    case_seed_serial_business(&fixture, &outline);
    let frozen_feedback = "P5draft冻结返修反馈（take前durable）".to_string();
    case_seed_fresh_recovery(
        &fixture,
        "recovery_p5_draft",
        LcRecoveryRunContext::LegacyDraft {
            business: case_recovery_business(&fixture),
            generation_round_id: "round_0001".to_string(),
            active_outline_id: "outline_a".to_string(),
            draft_refs: vec![LcRecoveryDraftRef {
                draft_id: "draft_outline_a".to_string(),
                status: WorkItemDraftStatus::Accepted,
                version: None,
                hash: None,
            }],
            pending_feedback: Some(frozen_feedback.clone()),
            outline_ref: Some("outline_p5".to_string()),
            candidate_ref: None,
        },
    );
    // 两轮都发空 write_scopes 候选：第一轮触发 RetryOnce，第二轮预算已耗
    //（initial_repair_attempted）→ ValidationFailed 终态收敛。
    *provider.rounds.lock().expect("rounds") = VecDeque::from(vec![
        CaseDraftRound::DraftPayload {
            outline_id: "outline_a".to_string(),
            title: "P5-A".to_string(),
            logical_work_item_id: "wi_a".to_string(),
            target_repository_id: Some(member),
            empty_write_scopes: true,
        },
        CaseDraftRound::DraftPayload {
            outline_id: "outline_a".to_string(),
            title: "P5-A".to_string(),
            logical_work_item_id: "wi_a".to_string(),
            target_repository_id: Some(member),
            empty_write_scopes: true,
        },
    ]);

    let run_context = case_event_run_context(&fixture, provider);
    let manager = run_context.manager.clone();
    let (outbound_tx, _outbound_rx) = mpsc::channel(64);
    spawn_provider_run_from_event(
        run_context,
        ProviderRunKind::WorkItemPlanDraft {
            feedback: Some(frozen_feedback.clone()),
        },
        None,
        outbound_tx,
    )
    .await
    .expect("spawn fresh draft run");
    // —— 冻结值优先 + 全新启动（两轮同纪律）——
    let first = tokio::time::timeout(std::time::Duration::from_secs(5), input_rx.recv())
        .await
        .expect("fresh draft 必须真实启动 provider（第一轮）")
        .expect("provider input");
    assert_eq!(
        first.resume_provider_session_id, None,
        "fresh 全新启动（旧 native-p5-old 在 conversation 引用中在场）"
    );
    assert!(
        first.prompt.contains(&frozen_feedback),
        "draft fresh 的 feedback 源是 durable 冻结值"
    );
    assert!(
        !first
            .prompt
            .contains("P5内存残留反馈（不得进入draft fresh输入）"),
        "内存 pending 不得混入 fresh 输入"
    );
    let second = tokio::time::timeout(std::time::Duration::from_secs(5), input_rx.recv())
        .await
        .expect("RetryOnce 必须按原预算发起第二轮 start")
        .expect("provider input");
    assert_eq!(second.resume_provider_session_id, None);
    assert!(
        second.prompt.contains(&frozen_feedback),
        "RetryOnce 轮的返修 feedback 保留冻结原文（combine 原样前置）"
    );
    assert!(
        second.prompt.contains("write_scope_required"),
        "第二轮 prompt 必须携带第一轮本地校验 findings（RetryOnce 反馈链）"
    );
    assert!(
        !second
            .prompt
            .contains("P5内存残留反馈（不得进入draft fresh输入）")
    );
    assert_eq!(
        starts.load(Ordering::SeqCst),
        2,
        "RetryOnce 预算恰一轮重试：两轮后收敛，不得有第三轮 start"
    );

    // —— 每轮新 split handle（Logical 启动路：两轮两个不同 role_run_seq）——
    let split_runs = case_split_runs(&fixture.app_paths, &fixture.record.id);
    assert_eq!(
        split_runs.len(),
        2,
        "RetryOnce 循环内每轮 begin 新 split handle（实测 {split_runs:?}）"
    );
    assert_ne!(
        split_runs[0].0, split_runs[1].0,
        "两轮 handle 的 role_run_seq 必须不同"
    );
    assert!(
        split_runs.iter().all(|(_, status)| status == "completed"),
        "两轮结构化输出均可解析，split run 均按 complete 收口（实测 {split_runs:?}）"
    );

    // —— 业务代同原：generation/outline/批次零重建；RetryOnce 预算耗尽后
    //    ValidationFailed 终态（不再第三轮）。——
    let index = case_active_index(&fixture);
    assert_eq!(index.current_generation_round_id, "round_0001");
    assert_eq!(index.active_outline_id.as_deref(), Some("outline_a"));
    assert!(index.batches.is_empty(), "draft fresh 不得新建业务 batch");
    assert!(
        index
            .draft_statuses
            .values()
            .any(|status| *status == WorkItemDraftStatus::ValidationFailed),
        "预算耗尽后的失败候选必须以 validation_failed 终态落盘（实测 {:?}）",
        index.draft_statuses
    );

    // —— 内存 pending 未被 take；draft 的 AwaitConfirmation 走 break：
    //    落 WorkItemDraftConfirm 后 Continue 正常尾部（注册清空）。——
    {
        let engine = fixture.engine.lock().await;
        assert_eq!(
            engine.pending_revision_context.as_deref(),
            Some("P5内存残留反馈（不得进入draft fresh输入）"),
            "fresh 构建不得 take 内存 pending"
        );
        assert_eq!(
            engine.active_node_type(),
            Some(TimelineNodeType::WorkItemDraftConfirm),
            "draft AwaitConfirmation 必须经 break 落草稿确认门"
        );
    }
    case_assert_engine_lock_reacquirable(&fixture, "draft fresh 收口后").await;
    case_wait_run_registration_cleared(&manager).await;
    case_assert_no_more_provider_inputs(&mut input_rx, "draft fresh 正常尾部后不得再 start").await;
}

/// batch fresh（Legacy 直连路）：冻结 payload 优先且 resume=None（旧 native
/// 在场）；不另建 batch/不重置 generation；AwaitConfirmation 仍 feedback=None
/// 继续原 loop（outline_a 确认后接着跑 outline_b 的完整提示词——与 draft 的
/// break 差异不统一），整批完成落 WorkItemBatchConfirm。
async fn batch_fresh_uses_frozen_payload_keeps_batch_and_continues_loop() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::Legacy);
    let starts = Arc::new(AtomicUsize::new(0));
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(CaseDraftBatchProvider {
        rounds: std::sync::Mutex::new(VecDeque::from(vec![
            CaseDraftRound::DraftPayload {
                outline_id: "outline_a".to_string(),
                title: "P5-BA".to_string(),
                logical_work_item_id: "wi_a".to_string(),
                target_repository_id: None,
                empty_write_scopes: false,
            },
            CaseDraftRound::DraftPayload {
                outline_id: "outline_b".to_string(),
                title: "P5-BB".to_string(),
                logical_work_item_id: "wi_b".to_string(),
                target_repository_id: None,
                empty_write_scopes: false,
            },
        ])),
        starts: starts.clone(),
        inputs: input_tx,
        fail_start: false,
    });
    let outline_a = case_outline_item(
        "outline_a",
        "wi_a",
        "P5-BA",
        "src/a.rs",
        None,
        &fixture.story_id,
        &fixture.design_id,
    );
    let outline_b = case_outline_item(
        "outline_b",
        "wi_b",
        "P5-BB",
        "web/b.rs",
        None,
        &fixture.story_id,
        &fixture.design_id,
    );
    {
        let mut engine = fixture.engine.lock().await;
        engine.update_artifact(case_outline_artifact(
            vec![outline_a.clone(), outline_b.clone()],
            &fixture.story_id,
            &fixture.design_id,
        )).await.expect("fixture artifact write");
        engine.session.provider_conversations = vec![case_old_author_conversation_ref()];
        engine.pending_revision_context =
            Some("P5batch内存残留反馈（不得进入fresh输入）".to_string());
    }
    case_seed_batch_business(&fixture, &outline_a);
    let frozen_feedback = "P5batch冻结返修反馈（take前durable）".to_string();
    case_seed_fresh_recovery(
        &fixture,
        "recovery_p5_batch",
        LcRecoveryRunContext::LegacyBatch {
            business: case_recovery_business(&fixture),
            generation_round_id: "round_0001".to_string(),
            active_outline_id: "outline_a".to_string(),
            draft_refs: vec![LcRecoveryDraftRef {
                draft_id: "draft_outline_a".to_string(),
                status: WorkItemDraftStatus::Accepted,
                version: None,
                hash: None,
            }],
            pending_feedback: Some(frozen_feedback.clone()),
            outline_ref: Some("outline_p5".to_string()),
            candidate_ref: None,
            batch_id: "batch_0001".to_string(),
            batch_item_draft_refs: vec![LcRecoveryDraftRef {
                draft_id: "draft_outline_a".to_string(),
                status: WorkItemDraftStatus::Accepted,
                version: None,
                hash: None,
            }],
        },
    );

    let run_context = case_event_run_context(&fixture, provider);
    let manager = run_context.manager.clone();
    let (outbound_tx, _outbound_rx) = mpsc::channel(64);
    spawn_provider_run_from_event(
        run_context,
        ProviderRunKind::WorkItemPlanBatch,
        None,
        outbound_tx,
    )
    .await
    .expect("spawn fresh batch run");
    // 完成信号先等两轮真实 input（Continue 正常尾部的 finish_run 在最后）。

    // —— 冻结值优先 + 全新启动 ——
    let first = tokio::time::timeout(std::time::Duration::from_secs(5), input_rx.recv())
        .await
        .expect("fresh batch 必须真实启动 provider（outline_a）")
        .expect("provider input");
    assert_eq!(first.resume_provider_session_id, None);
    assert!(first.prompt.contains(&frozen_feedback));
    assert!(
        !first
            .prompt
            .contains("P5batch内存残留反馈（不得进入fresh输入）")
    );
    // —— AwaitConfirmation 仍 feedback=None 继续原 loop（非 draft 的 break）：
    //    outline_a 确认后 loop 条件仍成立，接着跑 outline_b 的完整提示词。——
    let second = tokio::time::timeout(std::time::Duration::from_secs(5), input_rx.recv())
        .await
        .expect("batch AwaitConfirmation 后必须按原 loop 续跑下一 outline")
        .expect("provider input");
    assert_eq!(second.resume_provider_session_id, None);
    assert!(
        !second.prompt.contains(&frozen_feedback),
        "AwaitConfirmation 轮 feedback=None：不得携带冻结返修反馈（完整提示词）"
    );
    assert!(
        !second
            .prompt
            .contains("P5batch内存残留反馈（不得进入fresh输入）")
    );
    assert_eq!(starts.load(Ordering::SeqCst), 2);

    // —— 不另建 batch / generation：同 batch 续跑并推进 item drafts。——
    let index = case_active_index(&fixture);
    assert_eq!(index.current_generation_round_id, "round_0001");
    assert_eq!(index.batches.len(), 1, "batch fresh 不得另建业务 batch");
    let batch = &index.batches[0];
    assert_eq!(batch.batch_id, "batch_0001");
    assert_eq!(batch.status, WorkItemBatchStatus::Completed);
    assert_eq!(
        batch.item_draft_ids.len(),
        3,
        "seed 1 条 + 两轮续写 = 3 条 item draft（实测 {:?}）",
        batch.item_draft_ids
    );
    assert_eq!(index.active_outline_id, None, "整批完成后活动 outline 清空");

    // —— 内存 pending 未被 take；整批完成落 WorkItemBatchConfirm（Continue）。——
    {
        let engine = fixture.engine.lock().await;
        assert_eq!(
            engine.pending_revision_context.as_deref(),
            Some("P5batch内存残留反馈（不得进入fresh输入）")
        );
        assert_eq!(
            engine.active_node_type(),
            Some(TimelineNodeType::WorkItemBatchConfirm),
            "整批完成必须落 batch 确认门"
        );
    }
    case_assert_engine_lock_reacquirable(&fixture, "batch fresh 收口后").await;
    case_wait_run_registration_cleared(&manager).await;
    case_assert_no_more_provider_inputs(&mut input_rx, "batch fresh 正常尾部后不得再 start").await;
}

// ---------------------------------------------------------------------------
// 用例 2（brief Step 1）：early stop 分支的原 tail 锚定——行数门禁拆分至
// `draft_batch_cases/early_stop_cases.rs`（#[path] 链下显式路径）。
// ---------------------------------------------------------------------------
#[path = "draft_batch_cases/early_stop_cases.rs"]
mod early_stop_cases;
