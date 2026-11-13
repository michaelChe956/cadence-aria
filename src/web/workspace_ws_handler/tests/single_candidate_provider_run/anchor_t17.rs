// B2c（Task 5.5，tasks.md 5.5 / 设计 §5.3 SC 纪律①② + D1）：SC author 真链回归。
//
// - anchor_t17_sc_business_material_and_compile_target_do_not_follow_native_anchor：
//   多成员 LC 会话（focus=首成员）full SC 生成——业务材料（成员 0 checkout 的
//   language.md）与 compile target 跟随**业务成员解析**（workspace_repository_
//   for_session），不跟随 native 聚合根锚；native envelope 恒聚合根 root
//   （REQ-PLN-08）；source→IR physical（成员物理仓 id，非聚合视图）→确认
//   compile（Approval→Final Compile→Confirmed，work item 落业务成员仓）。
// - anchor_t17_sc_fresh_reuses_logical_reservation_without_rearm：修订轮 native
//   mismatch（旧 native 吊销）→ precheck 先于新业务 reserve → typed
//   NativeSupersededWaiting（不 generic Failed/不 rearm，原 key/phase 保留）→
//   显式 fresh claim → dispatch（sc_author tag 放行）→ LcFreshContext 重入
//   （不 reserve 第二次；resume=None 全新 native；返修 prompt 自足携带原
//   source/review）→ 原 compiler/确认门收口；recovery Resolved。

/// 记录 input 并立即完成一轮输出的 author provider（可携带 native id——
/// fresh 链对账锚：完成事件的 provider_session_id 经 record_provider_session
/// 落 conversation，再由成功收口绑定 recovery.fresh_native_session_id）。
struct T17RecordingAuthorProvider {
    output: String,
    native_session_id: Option<String>,
    inputs: mpsc::UnboundedSender<StreamingProviderInput>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for T17RecordingAuthorProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let _ = self.inputs.send(input);
        let output = self.output.clone();
        let native_session_id = self.native_session_id.clone();
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

    /// gateway 启动经 `start_validated` 分发（同 LcRecordingAuthorProvider：
    /// 拆出 input 后与裸 start 同路径）。
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
        unreachable!("t17 tests use start")
    }
}

/// 读下一帧 error 消息文本（等待面上浮断言用）。
async fn t17_next_error_message(outbound_rx: &mut mpsc::Receiver<OutboundControl>) -> String {
    loop {
        let frame = tokio::time::timeout(std::time::Duration::from_secs(2), outbound_rx.recv())
            .await
            .expect("must receive an outbound frame")
            .expect("outbound channel must stay open");
        if let OutboundControl::Text(text) = frame {
            let value: serde_json::Value = serde_json::from_str(&text).expect("json frame");
            if value["type"] == "error" {
                return value["message"].as_str().unwrap_or_default().to_string();
            }
        }
    }
}

fn t17_latest_record(
    fixture: &ProviderRunFixture,
) -> crate::product::models::WorkspaceSessionRecord {
    fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("latest durable record")
}

/// 修订轮 verdict：fresh 返修 prompt 必须回灌的 reviewer findings。
fn t17_revise_verdict() -> crate::web::workspace_ws_types::ReviewVerdict {
    crate::web::workspace_ws_types::ReviewVerdict {
        verdict: crate::web::workspace_ws_types::ReviewVerdictType::Revise,
        comments: "t17 reviewer 意见：WI-001 契约缺口需返修".to_string(),
        summary: "t17 SC 返修轮".to_string(),
        findings: vec![crate::web::workspace_ws_types::ReviewFinding {
            severity: crate::web::workspace_ws_types::ReviewFindingSeverity::MustFix,
            message: "t17 finding: capability 缺口".to_string(),
            evidence: "WI-001 Outputs".to_string(),
            required_action: "补 capability 声明".to_string(),
            category: None,
            class_hint: None,
            contract_field: None,
            machine_repair: None,
            owner_question: None,
            owner_options: None,
        disposition: None, target_work_item_id: None, finding_key: None, }],
        review_gate: crate::web::workspace_ws_types::ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    }
}

#[tokio::test]
async fn anchor_t17_sc_business_material_and_compile_target_do_not_follow_native_anchor() {
    let member_material =
        "## 语言规则\n\n- **必须使用中文** - t17 成员规则原文（成员 0 checkout）。\n";
    let root_material = "## 语言规则\n\n- t17 聚合根规则（不得进入 SC author prompt）。\n";
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(T17RecordingAuthorProvider {
        output: single_candidate_markdown("story_0001", "design_0001"),
        native_session_id: None,
        inputs: input_tx,
    });
    let fixture = ProviderRunFixture::new_logical_multi(
        vec![
            MemberRuleMaterial::Present(member_material.to_string()),
            MemberRuleMaterial::Present(
                "## 语言规则\n\n- 成员 1 规则原文（非聚焦成员，不参与本会话消费）。\n".to_string(),
            ),
        ],
        Some(root_material.to_string()),
        LogicalAliasFocus::FirstMember,
        provider.clone(),
    );
    let member_root = fixture.member_checkout_root();
    let (context, _outbound_rx) = single_candidate_context(&fixture, provider);
    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
            resume_recovery: None,
        },
    )
    .await;

    let full_input = tokio::time::timeout(std::time::Duration::from_secs(2), input_rx.recv())
        .await
        .expect("SC author provider must receive the full markdown input")
        .expect("SC author provider input");
    // ① 业务材料：成员 0 checkout 的 language.md 原文进入 prompt；聚合根规则
    //    不得进入（材料路径 = member_root/.claude/rules/language.md，非聚合根）。
    assert!(
        full_input
            .prompt
            .contains("t17 成员规则原文（成员 0 checkout）"),
        "SC author prompt 必须消费聚焦成员 checkout 的 language.md 原文"
    );
    assert!(
        !full_input.prompt.contains("t17 聚合根规则"),
        "聚合根规则不得进入 SC author prompt（业务材料不跟随 native 聚合根锚）"
    );
    // SC 消费锚（repository.path）= 聚焦成员 checkout（prompt 材料/仓结构/
    // compile target 同一来源）；native envelope = gateway 冻结的聚合根 root。
    let canonical_member_root = std::fs::canonicalize(&member_root).expect("canonical member root");
    assert_eq!(
        full_input.working_dir, canonical_member_root,
        "SC 业务材料锚必须是聚焦成员 checkout（working_dir=repository.path）"
    );
    let aggregate_root = member_root
        .parent()
        .expect("alias fixture members live under the aggregate root")
        .to_path_buf();
    let canonical_aggregate_root =
        std::fs::canonicalize(&aggregate_root).expect("canonical aggregate root");
    assert_eq!(
        full_input.working_directory,
        Some(canonical_aggregate_root),
        "native envelope 必须恒聚合根 root（REQ-PLN-08：cwd/target 不绑成员身份）"
    );

    wait_for_stage(&fixture.engine, WorkspaceStage::HumanConfirm).await;
    let durable = t17_latest_record(&fixture);
    assert_eq!(
        durable.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Approval)
    );

    // ② source→IR physical：IR item target = 聚焦成员物理仓 id，非聚合视图。
    let physical_member_id =
        crate::product::issue_store::IssueStore::new(fixture.app_paths.clone())
            .get(&durable.project_id, &durable.issue_id)
            .expect("issue record")
            .repo_id
            .expect("issue anchored on member 0 physical repository");
    let aggregate_view_id = format!(
        "lc_aggregate_root_view_{}",
        crate::product::logical_codebase::store::legacy_logical_codebase_id("project_0001")
    );
    let scope = crate::product::work_item_plan_source_store::SourceStoreScope {
        project_id: durable.project_id.clone(),
        issue_id: durable.issue_id.clone(),
        plan_id: durable.entity_id.clone(),
    };
    let ir_record = crate::product::work_item_plan_source_store::WorkItemPlanSourceStore::new(
        fixture.app_paths.clone(),
    )
    .get_plan_candidate_ir(
        &scope,
        durable.plan_candidate_ir_ref.as_deref().expect("IR ref"),
    )
    .expect("stored SC plan candidate IR");
    assert!(
        !ir_record.ir.items.is_empty(),
        "SC IR must compile to at least one item"
    );
    assert_eq!(
        ir_record.ir.items[0].target_repository_id, physical_member_id,
        "IR compile target 必须是业务成员物理仓 id（不跟随 native 聚合根锚）"
    );
    assert_ne!(
        ir_record.ir.items[0].target_repository_id, aggregate_view_id,
        "聚合视图 id 不得写入 IR target（design §3.2）"
    );

    // ③ 确认 compile：Approval → Final Compile → Confirmed/Completed——
    //    confirmed compile 结果 ok，work item 落业务成员仓。
    {
        let mut engine = fixture.engine.lock().await;
        engine.enter_policy_valid_work_item_plan_compile().await;
        assert_eq!(
            engine.session().stage,
            WorkspaceStage::Completed,
            "确认 compile 必须真实成功收口到 Completed"
        );
    }
    let plan = fixture
        .lifecycle
        .get_issue_work_item_plan(&durable.project_id, &durable.issue_id, &durable.entity_id)
        .expect("plan after confirmed compile");
    assert_eq!(
        plan.status,
        crate::product::models::IssueWorkItemPlanStatus::Confirmed
    );
    // SC Final Compile 经 plan_store 收口：work item 以 plan.work_item_ids 落
    // 计划（子 WorkItem 会话由 finalizer 创建）；IR target→effective logical
    // 成员映射在 compile 内强制（聚合视图 id 会以 work_item_target_missing
    // 直接失败），compile 成功即证明 IR physical target 生效。
    assert_eq!(
        plan.work_item_ids.len(),
        1,
        "Final Compile must create the work item"
    );
    let terminal = t17_latest_record(&fixture);
    assert_eq!(
        terminal.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Completed)
    );
}

/// 直接改写 durable record（seed 漂移形态用；与 engine 测试 write_json 同惯例）。
fn t17_rewrite_record(
    fixture: &ProviderRunFixture,
    mutate: impl FnOnce(&mut crate::product::models::WorkspaceSessionRecord),
) -> crate::product::models::WorkspaceSessionRecord {
    let mut record = t17_latest_record(fixture);
    mutate(&mut record);
    let path = fixture
        .app_paths
        .issue_root(&record.project_id, &record.issue_id)
        .join("workspace-sessions")
        .join(format!("{}.json", record.id));
    crate::product::json_store::write_json(&path, &record).expect("rewrite durable record");
    record
}

/// 落一份上一版 candidate source（fresh 返修 prompt 的 [previous_source] 材料），
/// 返回 (source_ref, content_hash)。
fn t17_put_previous_source(fixture: &ProviderRunFixture, text: &str) -> (String, String) {
    use sha2::Digest;
    let record = t17_latest_record(fixture);
    let scope = crate::product::work_item_plan_source_store::SourceStoreScope {
        project_id: record.project_id.clone(),
        issue_id: record.issue_id.clone(),
        plan_id: record.entity_id.clone(),
    };
    let source_hash = hex::encode(sha2::Sha256::digest(text.as_bytes()));
    let mut source = crate::product::work_item_plan_source_store::SourceRevisionRecord {
        id: format!("source-{}", &source_hash[..16]),
        source: text.to_string(),
        source_revision_hash: source_hash,
        content_hash: String::new(),
    };
    source.content_hash = source.content_hash().expect("source content hash");
    let content_hash = source.content_hash.clone();
    let source_ref = crate::product::work_item_plan_source_store::WorkItemPlanSourceStore::new(
        fixture.app_paths.clone(),
    )
    .put_source_revision(&scope.project_id, &scope.issue_id, &scope.plan_id, &source)
    .expect("put previous source revision");
    (source_ref, content_hash)
}

#[tokio::test]
async fn anchor_t17_sc_fresh_reuses_logical_reservation_without_rearm() {
    // fresh 链经 gateway 启动（LC 会话唯一物理启动口）——gateway author 即
    // 对账观测锚（t15 同型）：记录 input、以 t17-fresh-native 完成；mismatch
    // 阶段 ① 零启动（precheck 先于 spawn，哑 provider 占位）。
    let (fresh_input_tx, mut fresh_input_rx) = mpsc::unbounded_channel();
    let fresh_provider = Arc::new(T17RecordingAuthorProvider {
        output: single_candidate_markdown("story_0001", "design_0001"),
        native_session_id: Some("t17-fresh-native".to_string()),
        inputs: fresh_input_tx,
    });
    let fixture = ProviderRunFixture::new_logical(true, fresh_provider.clone());
    let session_id = fixture.record.id.clone();
    let reservation_key = format!("single_candidate_author:{session_id}:0");

    // 漂移形态 seed（修订轮中断后旧 native 被吊销）：
    // phase=Generate、ledger 保留原 reservation key、上一轮 Evaluate 已完成
    // （source/IR/report 三 refs 齐备——生产中 refs 由上一轮 complete/evaluation
    // 原子写入，repair 形态三者在场；fresh 返修轮 complete 走
    // is_repair_generation CAS 臂换新 refs）、Author conversation 指向旧
    // native 且已被 tombstone。
    let (source_ref, content_hash) =
        t17_put_previous_source(&fixture, "t17-previous-source-body（上一版 source 全文）");
    let seeded = t17_rewrite_record(&fixture, |record| {
        record.status = crate::product::models::WorkspaceSessionStatus::Running;
        record.single_candidate_phase =
            Some(crate::product::models::SingleCandidatePhase::Generate);
        record.provider_start_ledger = vec![
            crate::product::work_item_plan_policy::ProviderStartLedgerEntry {
                provider_start_idempotency_key: reservation_key.clone(),
                started: true,
                provider: None,
                started_at: None,
                provider_attempt_key: None,
            },
        ];
        record.work_item_plan_source_revision_ref = Some(source_ref.clone());
        record.plan_candidate_ir_ref = Some(format!(
            "project/{}/issue/{}/plan/{}/plan_candidate_ir/ir-t17-previous",
            record.project_id, record.issue_id, record.entity_id
        ));
        record.mechanical_report_ref = Some(format!(
            "project/{}/issue/{}/plan/{}/mechanical_report/report-t17-previous",
            record.project_id, record.issue_id, record.entity_id
        ));
        record.provider_conversations = vec![crate::product::models::ProviderConversationRef {
            role: crate::product::models::ProviderConversationRole::Author,
            provider: ProviderName::ClaudeCode,
            provider_session_id: "t17-old-native".to_string(),
            updated_at: "2026-10-12T00:00:00Z".to_string(),
            last_node_id: Some("timeline_node_001".to_string()),
        }];
        record.superseded_native_sessions =
            vec![crate::product::models::SupersededNativeSessionRecord {
                role: crate::product::models::ProviderConversationRole::Author,
                provider: ProviderName::ClaudeCode,
                native_session_id: "t17-old-native".to_string(),
                reason_code: "resume_fingerprint_unavailable".to_string(),
                recovery_id: "seed".to_string(),
                superseded_at: "2026-10-12T00:00:00Z".to_string(),
            }];
    });
    // 引擎内存镜像同步（漂移前的修订轮形态：phase=Generate + pending verdict）。
    {
        let mut engine = fixture.engine.lock().await;
        let repository_path = engine.session.repository_path.clone();
        engine.session = crate::product::workspace_engine::WorkspaceSession::from_record(seeded);
        engine.session.repository_path = repository_path;
        engine.latest_review_verdict = Some(t17_revise_verdict());
    }
    let ledger_before = serde_json::to_vec(&t17_latest_record(&fixture).provider_start_ledger)
        .expect("serialize ledger before");

    // ① Ordinary 重入：native mismatch precheck 先于新业务 reserve——typed
    //    NativeSupersededWaiting（非 generic Failed），不 rearm。
    let (context, mut outbound_rx) = single_candidate_context(&fixture, provider_placeholder());
    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
            resume_recovery: None,
        },
    )
    .await;
    let waiting_message = t17_next_error_message(&mut outbound_rx).await;
    assert!(
        waiting_message.contains("resume_fingerprint_unavailable"),
        "mismatch 等待消息必须携带稳定判别码，got: {waiting_message}"
    );
    assert!(
        waiting_message.contains("显式"),
        "mismatch 等待消息必须给出显式恢复出口，got: {waiting_message}"
    );
    let after_mismatch = t17_latest_record(&fixture);
    assert_eq!(
        after_mismatch.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Generate),
        "等待保留原 phase（不落 Failed、不回 Prepare 重臂）"
    );
    assert_ne!(
        after_mismatch.status,
        crate::product::models::WorkspaceSessionStatus::Failed,
        "mismatch 等待不是终态失败"
    );
    assert_eq!(
        serde_json::to_vec(&after_mismatch.provider_start_ledger).expect("serialize ledger"),
        ledger_before,
        "等待不得追加 provider_start_ledger（不 reserve 第二次）"
    );
    let recovery = after_mismatch
        .lc_resume_recovery
        .clone()
        .expect("mismatch must persist a waiting recovery record");
    assert_eq!(
        recovery.state,
        crate::product::models::LcResumeRecoveryState::Waiting
    );
    assert_eq!(recovery.reason_code, "resume_fingerprint_unavailable");
    let crate::product::models::LcRecoveryRunContext::ScAuthor {
        single_candidate_phase,
        provider_start_reservation_key,
        source_revision_ref: payload_source_ref,
        candidate_source_hash,
        ..
    } = &recovery.run_context
    else {
        panic!(
            "mismatch 捕获必须是 sc_author tag，got {:?}",
            recovery.run_context
        )
    };
    assert_eq!(
        *single_candidate_phase,
        crate::product::models::SingleCandidatePhase::Generate
    );
    assert_eq!(
        provider_start_reservation_key.as_deref(),
        Some(reservation_key.as_str())
    );
    assert_eq!(payload_source_ref.as_deref(), Some(source_ref.as_str()));
    assert_eq!(
        candidate_source_hash.as_deref(),
        Some(content_hash.as_str())
    );

    // ② 显式 fresh：claim → dispatch（sc_author tag 放行）→ LcFreshContext 重入
    //    （provider 经 fixture gateway 观测；dispatch registry 同源注册供
    //    spawn 的 provider 解析面）。
    let lifecycle = fixture.lifecycle.clone();
    assert!(matches!(
        lifecycle.claim_lc_fresh(&session_id, &recovery.recovery_id, "op-t17-sc-fresh"),
        Ok(crate::product::lifecycle_store::LcFreshClaimOutcome::Claimed { .. })
    ));
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, fresh_provider);
    let mut run_context = ProviderRunContext::test_fixture(
        Arc::new(registry),
        fixture.engine.clone(),
        fixture.workspace_runs.clone(),
        session_id.clone(),
        fixture.app_paths.clone(),
        t17_latest_record(&fixture),
    );
    run_context.manager = fixture.manager.clone();
    let (dispatch_tx, mut dispatch_rx) = mpsc::channel::<OutboundControl>(8);
    dispatch_lc_fresh_recovery_run(run_context, dispatch_tx).await;

    // 超时必须带出 run 的对外帧：否则只看到 Elapsed，无从区分「typed 拒」
    // 与「认领后臂内早退」。
    let fresh_input = match tokio::time::timeout(
        std::time::Duration::from_secs(2),
        fresh_input_rx.recv(),
    )
    .await
    {
        Ok(input) => input.expect("fresh provider input"),
        Err(elapsed) => {
            let frame = tokio::time::timeout(
                std::time::Duration::from_millis(500),
                dispatch_rx.recv(),
            )
            .await;
            panic!("显式 fresh 授权后 sc_author 必须真实 spawn（非 typed 拒）: {elapsed:?} / outbound={frame:?}");
        }
    };
    // fresh 重入是全新 native run：resume=None（不回捞旧 native）。
    assert!(
        fresh_input.resume_provider_session_id.is_none(),
        "fresh 物理 run 必须全新启动（resume=None）"
    );
    // 返修 prompt 自足：携带原 source 全文 + reviewer findings（含原 source/review
    // 完整上下文），不依赖任何历史会话。
    assert!(
        fresh_input.prompt.contains("[previous_source]"),
        "fresh 返修 prompt 必须显式携带上一版 source 小节"
    );
    assert!(
        fresh_input
            .prompt
            .contains("t17-previous-source-body（上一版 source 全文）"),
        "fresh 返修 prompt 必须内嵌上一版 source 全文"
    );
    assert!(
        fresh_input.prompt.contains("t17 finding: capability 缺口"),
        "fresh 返修 prompt 必须回灌 reviewer findings"
    );
    assert!(
        fresh_input.prompt.contains("不依赖任何历史会话"),
        "fresh 返修 prompt 必须声明全新会话自足语义"
    );

    // ③ 原门收口：compiler/Evaluate/确认门链保留，同 reservation/预算。
    wait_for_single_candidate_phase(
        &fixture,
        crate::product::models::SingleCandidatePhase::Approval,
    )
    .await;
    let after_fresh = t17_latest_record(&fixture);
    assert_eq!(
        serde_json::to_vec(&after_fresh.provider_start_ledger).expect("serialize ledger after"),
        ledger_before,
        "fresh 重入不 reserve 第二次（reservation_after == reservation_before）"
    );
    assert_eq!(
        after_fresh.run_history.repairs_used, 0,
        "fresh 不是新的 SC logical 预算（repairs_used 不变）"
    );
    let resolved = after_fresh
        .lc_resume_recovery
        .clone()
        .expect("recovery kept after fresh run");
    assert_eq!(
        resolved.state,
        crate::product::models::LcResumeRecoveryState::Resolved
    );
    assert_eq!(
        resolved.fresh_native_session_id.as_deref(),
        Some("t17-fresh-native"),
        "Resolved 必须绑定新物理 native run 证据"
    );
    assert!(
        resolved.fresh_role_run_seq.is_some(),
        "Resolved 必须绑定新物理 run 的 role_run_seq（新 native 证据，非新 logical reservation）"
    );
    assert_ne!(
        after_fresh.work_item_plan_source_revision_ref,
        Some(source_ref.clone()),
        "fresh 返修轮必须产出新 candidate source（原 compiler 门收口）"
    );
    // dispatch 面无 typed 拒帧（放行成功；等待帧仅 mismatch 阶段出现）。
    assert!(
        !matches!(
            tokio::time::timeout(
                std::time::Duration::from_millis(200),
                dispatch_rx.recv()
            )
            .await,
            Ok(Some(OutboundControl::Text(json)))
                if serde_json::from_str::<serde_json::Value>(&json)
                    .map(|value| value["type"] == "error" || value["type"] == "protocol_error")
                    .unwrap_or(false)
        ),
        "fresh dispatch 不得产生 typed 拒帧"
    );
}

/// mismatch 阶段的 StartGeneration 只为触发 precheck，不期望 provider 输入；
/// 用哑 provider 占位（零启动）。
fn provider_placeholder() -> Arc<dyn StreamingProviderAdapter> {
    struct NeverStartProvider;
    #[async_trait::async_trait]
    impl StreamingProviderAdapter for NeverStartProvider {
        async fn start(
            &self,
            _input: StreamingProviderInput,
            _cancel: CancellationToken,
        ) -> Result<ProviderSession, ProviderAdapterError> {
            panic!("mismatch 等待必须先于 provider 启动（零 spawn）")
        }
        async fn run_streaming(
            &self,
            _input: &crate::protocol::contracts::AdapterInput,
            _cancel: CancellationToken,
        ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
            unreachable!("t17 mismatch stage must not run streaming")
        }
    }
    Arc::new(NeverStartProvider)
}
