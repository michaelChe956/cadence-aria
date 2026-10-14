//! B1（终审 P1，B1→B2 接缝）：author 族（Story/Design 修订链）显式 fresh
//! dispatch 臂回归——anchor_t 系（t19）。
//!
//! 生产唯一可达的 author 族等待出生 = `drive_revision_session` 的
//! ReviewerRevision mismatch persist（review/drive.rs）。修复前死循环：
//! gateway_start catch-all 对 author 族 typed 拒 → 回等待；普通
//! StartGeneration 被 `lc_resume_recovery_active` 拒；RequestRevision 被
//! socket 门拒——owner 主线无出口。
//!
//! 本文件验证（B2 同型）：
//! - Story/Design 正例：等待 → claim（FreshReserved）→ dispatch（author 族
//!   臂放行）→ **真实 fresh spawn**（resume=None、prompt 自足携带
//!   artifact/feedback/review 全量）→ 完成 → 回 AuthorConfirm 门 →
//!   recovery Resolved（fresh native/node 回填）；
//! - 漂移负例：冻结 reviewer 与 durable 不一致 → dispatch typed
//!   `lc_resume_recovery_stale` 拒收回等待、零 spawn。

use super::*;
use crate::cross_cutting::streaming_provider::{
    ProviderCompletion, ProviderEvent, ProviderSession, StreamingProviderInput,
};
use crate::product::lifecycle_store::{CreateWorkspaceSessionInput, LcFreshClaimOutcome};
use crate::product::models::{
    LcFreshOperationState, LcRecoveryBusinessContext, LcRecoveryRunContext, LcResumeRecoveryRecord,
    LcResumeRecoveryState, ProviderConversationRef, ProviderConversationRole,
    SupersededNativeSessionRecord, WorkspaceType,
};
use crate::web::workspace_ws_types::{
    ArtifactPayload, ReviewFinding, ReviewFindingSeverity, ReviewGate, ReviewVerdict,
    ReviewVerdictType,
};
use std::sync::Arc;

use super::single_candidate_lc_admission::{AliasLcGatewayExtras, LcDialectCapabilitySource};
use super::single_candidate_provider_run::ProviderRunFixture;

const T19_OLD_NATIVE: &str = "t19-old-native";
const T19_FEEDBACK: &str = "T19冻结返修反馈全文标记：补充修订边界与验收口径";

/// 记录 input 并立即完成一轮合规产出的 author provider（native id 为 fresh
/// 链对账锚：完成后经 record_provider_session 落 conversation，再由 fresh
/// 收口绑定 recovery.fresh_native_session_id）。
struct T19RecordingAuthorProvider {
    output: String,
    native_session_id: &'static str,
    inputs: mpsc::UnboundedSender<StreamingProviderInput>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for T19RecordingAuthorProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let _ = self.inputs.send(input);
        let output = self.output.clone();
        let native_session_id = Some(self.native_session_id.to_string());
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(1);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::Completed(ProviderCompletion::plain(
                    output,
                    native_session_id,
                )))
                .await;
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }

    /// gateway 启动经 `start_validated` 分发（同 t17 夹具：拆出 input 后与
    /// 裸 start 同路径）。
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
        unreachable!("t19 tests use start")
    }
}

/// Story 修订产物（过 artifact gate：必填章节 + source id + [REQ-*]/[AC-*]）。
fn t19_story_output() -> String {
    "```artifact\n# Story Spec\n\n\
     ## 范围\n来源 source id: Issue issue_0001；T19 fresh 修订产物。\n\n\
     ## 用户故事\n作为用户，我希望修订产物可直接确认。\n\n\
     ## 功能需求\n- [REQ-T19-1] 修订产物包含全部必填章节。\n\n\
     ## 成功标准\n- [AC-T19-1] 修订产物确认门可达。\n\n\
     ## 待确认项\n无待确认项\n\n\
     ## 非功能需求\n无。\n\
     ```\n"
        .to_string()
}

/// Design 修订产物（过 artifact gate：必填章节 + source id + [DEC-*]/[CMP-*]/[API-*]）。
fn t19_design_output() -> String {
    "```artifact\n# Design Spec\n\n\
     ## 设计范围\n来源 source id: Issue issue_0001；T19 fresh 修订产物。\n\n\
     ## 设计决策\n- [DEC-T19-1] 修订走显式 fresh 全新会话。\n\n\
     ## 公共组件\n- [CMP-T19-1] dispatch 臂业务对照。\n\n\
     ## API 契约\n- [API-T19-1] claim 后 dispatch 放行。\n\n\
     ## 数据模型\n无新增实体。\n\n\
     ## 风险\n无。\n\n\
     ## 追踪关系\n- [DEC-T19-1] 关联 [REQ-T19-1]。\n\
     ```\n"
        .to_string()
}

/// 上一版产物全文（fresh prompt 必须内嵌的自足材料）。
fn t19_prior_artifact_marker(workspace_type: WorkspaceType) -> String {
    match workspace_type {
        WorkspaceType::Design => "T19上一版Design产物全文标记".to_string(),
        _ => "T19上一版Story产物全文标记".to_string(),
    }
}

fn t19_prior_artifact(workspace_type: WorkspaceType) -> ArtifactPayload {
    ArtifactPayload::Markdown {
        markdown: format!(
            "# {} Spec\n\n{}：待修订的旧稿正文。",
            match workspace_type {
                WorkspaceType::Design => "Design",
                _ => "Story",
            },
            t19_prior_artifact_marker(workspace_type),
        ),
        diff: None,
    }
}

/// 修订轮 verdict：fresh 返修 prompt 必须回灌的 reviewer 意见。
fn t19_review_verdict() -> ReviewVerdict {
    ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: "T19 reviewer 意见全文标记：边界缺口需返修".to_string(),
        summary: "T19 评审摘要标记".to_string(),
        findings: vec![ReviewFinding {
            severity: ReviewFindingSeverity::MustFix,
            message: "T19 finding：修订边界缺口".to_string(),
            evidence: "范围章节".to_string(),
            required_action: "补充修订边界".to_string(),
            category: None,
            class_hint: None,
            contract_field: None,
            machine_repair: None,
        }],
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    }
}

fn t19_business(
    record: &crate::product::models::WorkspaceSessionRecord,
    effective_reviewer: Option<ProviderName>,
) -> LcRecoveryBusinessContext {
    LcRecoveryBusinessContext {
        entity_id: record.entity_id.clone(),
        plan_id: None,
        workspace_type: record.workspace_type.clone(),
        business_repository_physical_id: None,
        business_repository_logical_id: None,
        business_checkout_id: None,
        business_repository_path: None,
        source_story_ids: Vec::new(),
        source_design_ids: Vec::new(),
        plan_options_ref: None,
        artifact_refs: Vec::new(),
        review_refs: Vec::new(),
        author_provider: record.author_provider.clone(),
        effective_reviewer_provider: effective_reviewer,
        original_node_id: Some("timeline_node_revision_0001".to_string()),
        original_role_run_seq: None,
    }
}

/// 直接改写 durable record（seed 漂移形态用；与 engine 测试 write_json 同惯例）：
/// 等待出生前的修订轮形态——status Running、Author conversation 指向旧 native
/// 且已被 tombstone（生产 mismatch 收口语义）。
fn t19_rewrite_record(
    app_paths: &ProductAppPaths,
    record: &crate::product::models::WorkspaceSessionRecord,
) -> crate::product::models::WorkspaceSessionRecord {
    let mut seeded = record.clone();
    seeded.status = crate::product::models::WorkspaceSessionStatus::Running;
    seeded.provider_conversations = vec![ProviderConversationRef {
        role: ProviderConversationRole::Author,
        provider: seeded.author_provider.clone(),
        provider_session_id: T19_OLD_NATIVE.to_string(),
        updated_at: "2026-10-12T00:00:00Z".to_string(),
        last_node_id: Some("timeline_node_revision_0001".to_string()),
    }];
    seeded.superseded_native_sessions = vec![SupersededNativeSessionRecord {
        role: ProviderConversationRole::Author,
        provider: seeded.author_provider.clone(),
        native_session_id: T19_OLD_NATIVE.to_string(),
        reason_code: "resume_fingerprint_mismatch".to_string(),
        recovery_id: "seed".to_string(),
        superseded_at: "2026-10-12T00:00:00Z".to_string(),
    }];
    let path = app_paths
        .issue_root(&seeded.project_id, &seeded.issue_id)
        .join("workspace-sessions")
        .join(format!("{}.json", seeded.id));
    crate::product::json_store::write_json(&path, &seeded).expect("rewrite durable record");
    seeded
}

/// 落 Waiting 等待记录（等待出生面归 mismatch 收口链；本文件验证 fresh 消费）。
fn t19_seed_waiting(
    lifecycle: &LifecycleStore,
    seeded_record: &crate::product::models::WorkspaceSessionRecord,
    effective_reviewer: Option<ProviderName>,
) -> LcResumeRecoveryRecord {
    t19_seed_waiting_with_feedback(
        lifecycle,
        seeded_record,
        effective_reviewer,
        Some(T19_FEEDBACK.to_string()),
    )
}

/// [`t19_seed_waiting`] 的 feedback 覆盖项变体（`None`＝冻结反馈缺失形态）。
fn t19_seed_waiting_with_feedback(
    lifecycle: &LifecycleStore,
    seeded_record: &crate::product::models::WorkspaceSessionRecord,
    effective_reviewer: Option<ProviderName>,
    original_feedback: Option<String>,
) -> LcResumeRecoveryRecord {
    let recovery = LcResumeRecoveryRecord {
        schema_version: 1,
        recovery_id: "recovery_anchor_t19".to_string(),
        failed_provider_node_id: "timeline_node_revision_0001".to_string(),
        reason_code: "resume_fingerprint_mismatch".to_string(),
        role: ProviderConversationRole::Author,
        provider: seeded_record.author_provider.clone(),
        superseded_native_session_id: Some(T19_OLD_NATIVE.to_string()),
        previous_fingerprint_digest: Some("sha256:t19-old".to_string()),
        previous_session_status: seeded_record.status.clone(),
        previous_stage: "revision".to_string(),
        run_context: LcRecoveryRunContext::ReviewerRevision {
            business: t19_business(seeded_record, effective_reviewer),
            original_feedback,
        },
        state: LcResumeRecoveryState::Waiting,
        operation_id: None,
        fresh_node_id: None,
        fresh_native_session_id: None,
        fresh_role_run_seq: None,
        fresh_operations: Vec::new(),
        detected_at: "2026-10-12T00:00:00Z".to_string(),
        updated_at: "2026-10-12T00:00:00Z".to_string(),
    };
    lifecycle
        .compare_and_save_lc_resume_state(seeded_record, recovery.clone())
        .expect("seed t19 waiting recovery");
    recovery
}

/// 等待收口后的引擎形态（mirror 生产 reconcile）：stage=PrepareContext、
/// pending/verdict 按 durable 派生（verdict 在场、pending 由 fresh 臂从冻结
/// payload 透传恢复）、artifact=上一版全文。
async fn t19_rearm_engine_after_waiting(
    engine: &Arc<Mutex<WorkspaceEngine>>,
    seeded_record: &crate::product::models::WorkspaceSessionRecord,
    workspace_type: WorkspaceType,
) {
    let mut engine = engine.lock().await;
    let repository_path = engine.session.repository_path.clone();
    engine.session = WorkspaceSession::from_record(seeded_record.clone());
    engine.session.repository_path = repository_path;
    engine.session.stage = WorkspaceStage::PrepareContext;
    engine.session.artifact = Some(t19_prior_artifact(workspace_type));
    engine.latest_review_verdict = Some(t19_review_verdict());
    engine.pending_revision_context = None;
}

/// claim → dispatch（author 族臂），返回 (input 通道, outbound 通道)。
async fn t19_claim_and_dispatch(
    fixture: &ProviderRunFixture,
    lifecycle: &LifecycleStore,
    session_id: &str,
    recovery: &LcResumeRecoveryRecord,
    operation_id: &str,
    engine_arc: Arc<Mutex<WorkspaceEngine>>,
    provider: Arc<dyn StreamingProviderAdapter>,
) -> mpsc::Receiver<OutboundControl> {
    assert!(matches!(
        lifecycle.claim_lc_fresh(session_id, &recovery.recovery_id, operation_id),
        Ok(LcFreshClaimOutcome::Claimed { .. })
    ));
    let record = lifecycle
        .get_workspace_session(session_id)
        .expect("record after claim");
    let mut registry = ProviderRegistry::new();
    registry.register(record.author_provider.clone(), provider);
    let mut run_context = ProviderRunContext::test_fixture(
        Arc::new(registry),
        engine_arc,
        WorkspaceRunRegistry::default(),
        session_id.to_string(),
        fixture.app_paths.clone(),
        record,
    );
    run_context.manager = fixture.manager.clone();
    let (outbound_tx, outbound_rx) = mpsc::channel::<OutboundControl>(8);
    dispatch_lc_fresh_recovery_run(run_context, outbound_tx).await;
    outbound_rx
}

/// 读下一帧 (type, code)（typed 拒帧断言用）。
async fn t19_next_frame_code(
    outbound_rx: &mut mpsc::Receiver<OutboundControl>,
) -> (serde_json::Value, String) {
    let frame = tokio::time::timeout(std::time::Duration::from_secs(1), outbound_rx.recv())
        .await
        .expect("必须收到 typed 帧")
        .expect("outbound frame");
    match frame {
        OutboundControl::Text(text) => {
            let value: serde_json::Value = serde_json::from_str(&text).expect("json frame");
            let code = value["code"].as_str().unwrap_or_default().to_string();
            (value.clone(), code)
        }
        other => panic!("期望 Text 帧，got {other:?}"),
    }
}

fn t19_latest_recovery(lifecycle: &LifecycleStore, session_id: &str) -> LcResumeRecoveryRecord {
    lifecycle
        .get_workspace_session(session_id)
        .expect("latest record")
        .lc_resume_recovery
        .expect("recovery kept")
}

/// Story 正例：等待 → claim → dispatch → 真实 fresh spawn（resume=None、
/// prompt 含 artifact/feedback/review 全量）→ 完成 → Resolved。
#[tokio::test]
async fn anchor_t19_story_revision_fresh_dispatch_resolves_after_completion() {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(T19RecordingAuthorProvider {
        output: t19_story_output(),
        native_session_id: "t19-fresh-native-story",
        inputs: input_tx,
    });
    let (fixture, mut engine_rx) =
        ProviderRunFixture::new_logical_story_with_gateway_extras_and_engine_rx(
            provider.clone(),
            AliasLcGatewayExtras {
                capability: Some(Arc::new(LcDialectCapabilitySource)),
                ..AliasLcGatewayExtras::default()
            },
            ProviderName::ClaudeCode,
        );
    tokio::spawn(async move { while engine_rx.recv().await.is_some() {} });
    let session_id = fixture.record.id.clone();

    // 等待形态 seed：旧 native 已 tombstone、ReviewerRevision 冻结 feedback。
    let seeded = t19_rewrite_record(&fixture.app_paths, &fixture.record);
    t19_seed_waiting(&fixture.lifecycle, &seeded, Some(ProviderName::Codex));
    t19_rearm_engine_after_waiting(&fixture.engine, &seeded, WorkspaceType::Story).await;

    let mut outbound_rx = t19_claim_and_dispatch(
        &fixture,
        &fixture.lifecycle,
        &session_id,
        &t19_latest_recovery(&fixture.lifecycle, &session_id),
        "op-t19-story-fresh",
        fixture.engine.clone(),
        provider,
    )
    .await;

    // ① 唯一启动口真实 spawn：全新会话（resume=None），prompt 自足。
    let input = tokio::time::timeout(std::time::Duration::from_secs(5), input_rx.recv())
        .await
        .expect("显式 fresh 授权后 author 族（story 修订）必须真实 spawn（非 typed 拒）")
        .expect("provider input");
    assert!(
        input.resume_provider_session_id.is_none(),
        "fresh 物理 run 必须全新启动（旧 native {} 已 tombstone 不回捞）",
        T19_OLD_NATIVE
    );
    assert!(
        input
            .prompt
            .contains(t19_prior_artifact_marker(WorkspaceType::Story).as_str()),
        "fresh prompt 必须内嵌上一版产物全文"
    );
    assert!(
        input.prompt.contains(T19_FEEDBACK),
        "fresh prompt 必须携带冻结返修 feedback 全文"
    );
    assert!(
        input
            .prompt
            .contains("T19 reviewer 意见全文标记：边界缺口需返修"),
        "fresh prompt 必须回灌 reviewer 审核意见全文"
    );
    assert!(
        input.prompt.contains("T19 评审摘要标记"),
        "fresh prompt 必须携带 reviewer 摘要"
    );

    // ② 完成回原确认门：AuthorConfirm + recovery Resolved（native/node 回填）。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let stage = {
            let engine = fixture.engine.lock().await;
            engine.session().stage.clone()
        };
        if stage == WorkspaceStage::AuthorConfirm {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "fresh 修订 run 必须完成回 AuthorConfirm 门"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let resolved = t19_latest_recovery(&fixture.lifecycle, &session_id);
    assert_eq!(
        resolved.state,
        LcResumeRecoveryState::Resolved,
        "fresh run 完成后 recovery 必须 Resolved（owner 主线恢复出口）"
    );
    assert_eq!(
        resolved.fresh_native_session_id.as_deref(),
        Some("t19-fresh-native-story"),
        "Resolved 必须绑定新物理 native run 证据"
    );
    assert!(
        resolved.fresh_node_id.is_some(),
        "Resolved 必须绑定 fresh 修订节点"
    );
    assert!(resolved.fresh_operations.iter().any(|operation| {
        operation.operation_id == "op-t19-story-fresh"
            && operation.state == LcFreshOperationState::Completed
    }));
    assert!(
        input_rx.try_recv().is_err(),
        "dispatch 后无第二 input（无重复 spawn）"
    );
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(200), outbound_rx.recv()).await,
            Ok(Some(OutboundControl::Text(json)))
                if serde_json::from_str::<serde_json::Value>(&json)
                    .map(|value| value["type"] == "error" || value["type"] == "protocol_error")
                    .unwrap_or(false)
        ),
        "fresh dispatch 不得产生 typed 拒帧"
    );

    // ③（max-task 定案补验②）一次 fresh 即解锁（非永久锁）：Resolved 后
    // 普通 RequestRevision 语义的修订重驱不再被 LC_RESUME_RECOVERY_ACTIVE
    // 拒收——engine 不落等待臂，正常经 gateway 驱动（下一条 input 即证据），
    // 且 resume 面续接 fresh native（锚修复后指纹恒聚合根，scope 无关不漂）。
    let direct_sentinel = Arc::new(T19RecordingAuthorProvider {
        output: t19_story_output(),
        native_session_id: "t19-direct-sentinel",
        inputs: {
            let (sentinel_tx, _sentinel_rx) = mpsc::unbounded_channel();
            sentinel_tx
        },
    });
    {
        let mut engine = fixture.engine.lock().await;
        engine.session.stage = WorkspaceStage::Revision;
        engine.pending_revision_context = Some("Resolved 后普通修订重驱".to_string());
        let (_plain_command_tx, plain_command_rx) = mpsc::channel(8);
        engine
            .drive_revision_session(direct_sentinel, plain_command_rx)
            .await;
    }
    let plain_input = tokio::time::timeout(std::time::Duration::from_secs(5), input_rx.recv())
        .await
        .expect("Resolved 后普通修订重驱必须真实 spawn（不再被恢复等待拒收）")
        .expect("plain revision input");
    assert!(
        !plain_input.prompt.is_empty(),
        "普通修订重驱 prompt 非空（非等待臂 typed 错误路径）"
    );
    assert_eq!(
        plain_input.resume_provider_session_id.as_deref(),
        Some("t19-fresh-native-story"),
        "锚恒聚合根 → 指纹不漂 → 普通重驱原样续接 fresh native（不 supersede）"
    );
    let post_resolve_recovery = t19_latest_recovery(&fixture.lifecycle, &session_id);
    assert_eq!(
        post_resolve_recovery.state,
        LcResumeRecoveryState::Resolved,
        "普通修订重驱不得复活恢复等待（Resolved 只保留证据）"
    );
}

/// Design 同型正例：同一 author 族臂对 Design 修订链等价放行与收口。
#[tokio::test]
async fn anchor_t19_design_revision_fresh_dispatch_resolves_after_completion() {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(T19RecordingAuthorProvider {
        output: t19_design_output(),
        native_session_id: "t19-fresh-native-design",
        inputs: input_tx,
    });
    let (fixture, mut engine_rx) =
        ProviderRunFixture::new_logical_story_with_gateway_extras_and_engine_rx(
            provider.clone(),
            AliasLcGatewayExtras {
                capability: Some(Arc::new(LcDialectCapabilitySource)),
                ..AliasLcGatewayExtras::default()
            },
            ProviderName::ClaudeCode,
        );
    tokio::spawn(async move { while engine_rx.recv().await.is_some() {} });

    // Design 工作区会话（同 issue 聚合）：gateway 从 Story 会话引擎克隆注入
    // （生产同 issue 唯一 gateway）。
    let design_record = fixture
        .lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: fixture.design_id.clone(),
            workspace_type: WorkspaceType::Design,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: Some(ProviderName::Codex),
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .expect("create design workspace session");
    let gateway = fixture
        .engine
        .lock()
        .await
        .logical_provider_gateway()
        .expect("story fixture gateway");
    let mut design_session = WorkspaceSession::from_record(design_record.clone());
    design_session.repository_path = Some(fixture.member_checkout_root());
    let (engine_tx, mut engine_rx) = mpsc::channel::<EngineEvent>(64);
    tokio::spawn(async move { while engine_rx.recv().await.is_some() {} });
    let design_engine = Arc::new(Mutex::new(
        WorkspaceEngine::new_persistent(
            Arc::new(CheckpointStore::new(
                fixture.root_path().join("checkpoints-t19-design"),
            )),
            fixture.lifecycle.clone(),
            engine_tx,
            design_session,
        )
        .with_logical_provider_gateway(gateway),
    ));
    let session_id = design_record.id.clone();

    let seeded = t19_rewrite_record(&fixture.app_paths, &design_record);
    t19_seed_waiting(&fixture.lifecycle, &seeded, Some(ProviderName::Codex));
    t19_rearm_engine_after_waiting(&design_engine, &seeded, WorkspaceType::Design).await;

    let _outbound_rx = t19_claim_and_dispatch(
        &fixture,
        &fixture.lifecycle,
        &session_id,
        &t19_latest_recovery(&fixture.lifecycle, &session_id),
        "op-t19-design-fresh",
        design_engine.clone(),
        provider,
    )
    .await;

    let input = tokio::time::timeout(std::time::Duration::from_secs(5), input_rx.recv())
        .await
        .expect("显式 fresh 授权后 author 族（design 修订）必须真实 spawn（非 typed 拒）")
        .expect("provider input");
    assert!(input.resume_provider_session_id.is_none());
    assert!(
        input
            .prompt
            .contains(t19_prior_artifact_marker(WorkspaceType::Design).as_str()),
        "fresh prompt 必须内嵌上一版 Design 产物全文"
    );
    assert!(input.prompt.contains(T19_FEEDBACK));
    assert!(
        input
            .prompt
            .contains("T19 reviewer 意见全文标记：边界缺口需返修")
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let stage = {
            let engine = design_engine.lock().await;
            engine.session().stage.clone()
        };
        if stage == WorkspaceStage::AuthorConfirm {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "fresh 修订 run 必须完成回 AuthorConfirm 门"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let resolved = t19_latest_recovery(&fixture.lifecycle, &session_id);
    assert_eq!(resolved.state, LcResumeRecoveryState::Resolved);
    assert_eq!(
        resolved.fresh_native_session_id.as_deref(),
        Some("t19-fresh-native-design")
    );
}

/// 漂移负例：冻结 reviewer 与 durable 不一致 → typed `lc_resume_recovery_stale`
/// 拒收回等待、operation Failed、零 spawn。
#[tokio::test]
async fn anchor_t19_author_family_business_drift_waits_typed_stale() {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(T19RecordingAuthorProvider {
        output: t19_story_output(),
        native_session_id: "t19-fresh-native-unused",
        inputs: input_tx,
    });
    let (fixture, mut engine_rx) =
        ProviderRunFixture::new_logical_story_with_gateway_extras_and_engine_rx(
            provider.clone(),
            AliasLcGatewayExtras {
                capability: Some(Arc::new(LcDialectCapabilitySource)),
                ..AliasLcGatewayExtras::default()
            },
            ProviderName::ClaudeCode,
        );
    tokio::spawn(async move { while engine_rx.recv().await.is_some() {} });
    let session_id = fixture.record.id.clone();

    let seeded = t19_rewrite_record(&fixture.app_paths, &fixture.record);
    // 冻结 reviewer=KimiCode，durable record reviewer=Codex——等待期 provider
    // 维度漂移。
    t19_seed_waiting(&fixture.lifecycle, &seeded, Some(ProviderName::KimiCode));
    t19_rearm_engine_after_waiting(&fixture.engine, &seeded, WorkspaceType::Story).await;

    let mut outbound_rx = t19_claim_and_dispatch(
        &fixture,
        &fixture.lifecycle,
        &session_id,
        &t19_latest_recovery(&fixture.lifecycle, &session_id),
        "op-t19-drift",
        fixture.engine.clone(),
        provider,
    )
    .await;

    let (_frame, code) = t19_next_frame_code(&mut outbound_rx).await;
    assert_eq!(
        code, "lc_resume_recovery_stale",
        "业务四元组漂移必须 typed Stale 拒收（不用旧权限强行完成）"
    );
    let after = t19_latest_recovery(&fixture.lifecycle, &session_id);
    assert_eq!(after.state, LcResumeRecoveryState::Waiting);
    assert_eq!(
        after.fresh_operations[0].state,
        LcFreshOperationState::Failed
    );
    assert_eq!(
        after.fresh_operations[0].reason_code.as_deref(),
        Some("lc_resume_recovery_stale")
    );
    match tokio::time::timeout(std::time::Duration::from_millis(300), input_rx.recv()).await {
        Err(_) | Ok(None) => {}
        Ok(Some(_)) => panic!("业务四元组漂移必须零 spawn"),
    }
}

/// 复审 P2：fresh 修订链**驱动前**失败（input 构建 Err 形态——冻结反馈缺失
/// 且无 verdict，`build_revision_input_with_resume` fail-closed）也必须对账
/// recovery：回 Waiting + operation Failed——不得滞留 FreshRunning 令后续
/// claim 恒 Busy（owner 主线无 typed 出口）。新 operation 可再 claim 是回归
/// 判据（非 Busy/Stale）。
#[tokio::test]
async fn anchor_t19_author_family_fresh_prestart_failure_returns_to_waiting() {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(T19RecordingAuthorProvider {
        output: t19_story_output(),
        native_session_id: "t19-fresh-native-unused",
        inputs: input_tx,
    });
    let (fixture, mut engine_rx) =
        ProviderRunFixture::new_logical_story_with_gateway_extras_and_engine_rx(
            provider.clone(),
            AliasLcGatewayExtras {
                capability: Some(Arc::new(LcDialectCapabilitySource)),
                ..AliasLcGatewayExtras::default()
            },
            ProviderName::ClaudeCode,
        );
    tokio::spawn(async move { while engine_rx.recv().await.is_some() {} });
    let session_id = fixture.record.id.clone();

    let seeded = t19_rewrite_record(&fixture.app_paths, &fixture.record);
    // 冻结反馈缺失（original_feedback=None）——fresh 臂不恢复 pending；配合
    // verdict 缺失，input 构建 fail-closed（review verdict is unavailable）。
    t19_seed_waiting_with_feedback(&fixture.lifecycle, &seeded, Some(ProviderName::Codex), None);
    t19_rearm_engine_after_waiting(&fixture.engine, &seeded, WorkspaceType::Story).await;
    {
        let mut engine = fixture.engine.lock().await;
        engine.latest_review_verdict = None;
    }

    let _outbound_rx = t19_claim_and_dispatch(
        &fixture,
        &fixture.lifecycle,
        &session_id,
        &t19_latest_recovery(&fixture.lifecycle, &session_id),
        "op-t19-prestart-fail",
        fixture.engine.clone(),
        provider,
    )
    .await;

    // 驱动前失败必须对账回 Waiting（轮询等待异步 run task 收口）。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let after = t19_latest_recovery(&fixture.lifecycle, &session_id);
        if after.state == LcResumeRecoveryState::Waiting {
            assert_eq!(
                after.fresh_operations[0].state,
                LcFreshOperationState::Failed,
                "驱动前失败的 operation 必须 Failed（结果保留）"
            );
            assert_eq!(after.operation_id, None);
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "fresh 驱动前失败后 recovery 必须回 Waiting（不得滞留 FreshRunning 令 claim 恒 Busy），当前 state={:?}",
            after.state
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    // 新 operation 可再 claim（非 Busy/Stale）——owner 主线出口恢复。
    assert!(matches!(
        fixture.lifecycle.claim_lc_fresh(
            &session_id,
            "recovery_anchor_t19",
            "op-t19-prestart-retry"
        ),
        Ok(LcFreshClaimOutcome::Claimed { .. })
    ));
    match tokio::time::timeout(std::time::Duration::from_millis(300), input_rx.recv()).await {
        Err(_) | Ok(None) => {}
        Ok(Some(_)) => panic!("驱动前失败必须零 provider spawn"),
    }
}
