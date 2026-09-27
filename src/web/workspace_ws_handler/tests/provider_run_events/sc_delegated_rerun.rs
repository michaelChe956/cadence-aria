use super::*;

// ---------------------------------------------------------------------------
// F2（P0 真实链 workspace_session_0018 现场）：无附件 REST 反馈链的 SC 委托
// 返修接力在 engine/provider run 事件路由层被静默吞没。
//
// 现场：REST human-actions feedback → spawn HumanGateScManualRevision（注册
// 节点=仍开启的 human_confirm 门节点）→ 修订完成 → 复评 revise → policy 委托
// ProviderRunRequested{WorkItemPlanSingleCandidateAuthor}（emit 时活动节点仍=
// 同一门节点）→ from_event「同节点去重」命中 → drain（tracing::debug 零可见）
// → followups 同时按 phase=Generate 让位 → 双方都退出、无人驱动重跑 →
// durable 停留 (running, generate)：无门、无 run、零事件，会话永久卡 running。
// ---------------------------------------------------------------------------

/// F2 红锚1（drain 修复）：修订 run 注册节点与委托接力请求节点同为门节点时，
/// 同节点去重不得吞掉不同 kind 的接力——它是 0018 现场唯一被委托的接续路径。
#[tokio::test]
async fn sc_delegated_rerun_relay_is_not_drained_by_gate_node_kind_collision() {
    let (fixture, engine_rx) =
        ProviderRunFixture::new_with_engine_rx(WorkItemPlanFlowKind::SingleCandidate);
    persist_review_rounds(&fixture, 1);
    // 只 drain 不接力：委托接力由本测试手动发起（确定性复现 0018 碰撞）。
    let mut engine_rx = engine_rx;
    let drain_engine = tokio::spawn(async move { while engine_rx.recv().await.is_some() {} });

    let author_starts = Arc::new(AtomicUsize::new(0));
    let author = Arc::new(ScSequenceAuthorProvider {
        outputs: vec![single_candidate_markdown(
            &fixture.story_id,
            &fixture.design_id,
        )],
        starts: author_starts.clone(),
        held: Mutex::new(Vec::new()),
    });
    let review_starts = Arc::new(AtomicUsize::new(0));
    let reviewer = Arc::new(ScSequenceReviewProvider {
        outputs: vec![sc_pass_review_output()],
        starts: review_starts.clone(),
        held: Mutex::new(Vec::new()),
    });
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, author);
    registry.register(ProviderName::Codex, reviewer);
    let (context, _outbound_rx, run_context, outbound_tx) =
        sc_context_with_registry(&fixture, registry);

    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: review_enabled_provider_config(),
            reviewer_enabled: true,
        },
    )
    .await;

    // 等 SC Approval 门开（pass → EnterHumanGate/审批门，含 human gate snapshot）。
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let stage = {
                let engine = fixture.engine.lock().await;
                engine.session().stage.clone()
            };
            if stage == WorkspaceStage::HumanConfirm {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("SC approval gate must open after the passing review");

    // REST feedback 等价链：apply → TurnOpened → handler spawn（注册节点=门节点）。
    let effect = apply_human_gate_feedback(
        fixture.engine.clone(),
        HumanGateFeedbackInput {
            command_id: "f2-red-collision".to_string(),
            feedback: "请修订当前候选的验证计划".to_string(),
        },
    )
    .await;
    let HumanGateFeedbackEffect::TurnOpened { turn, prompt, .. } = effect else {
        panic!("human gate feedback must open a revision turn");
    };
    let gate_node = {
        let engine = fixture.engine.lock().await;
        engine
            .active_timeline_node_id()
            .expect("gate node must be active")
    };
    spawn_provider_run_from_handler(
        run_context.clone(),
        ProviderRunKind::HumanGateScManualRevision {
            turn_id: turn.turn_id,
            prompt,
        },
        outbound_tx.clone(),
    )
    .await
    .expect("revision run spawn must succeed");

    // 修订 run 在途（held provider 永不完成），注册节点必须是门节点（0018 现场）。
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if author_starts.load(Ordering::SeqCst) >= 2 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("revision run must reach its held author provider");
    let active = fixture
        .manager
        .active_run()
        .await
        .expect("held revision run must be registered");
    assert_eq!(
        active.node_id.as_deref(),
        Some(gate_node.as_str()),
        "修订 run 注册节点必须仍是 human_confirm 门节点（0018 现场前置）"
    );
    let superseded_token = active.token;
    let superseded_cancel = active.cancel.clone();

    // 委托接力：同节点、不同 kind（WorkItemPlanSingleCandidateAuthor）。
    let (relay_outbound_tx, _relay_outbound_rx) = mpsc::channel(8);
    spawn_provider_run_from_event(
        run_context,
        ProviderRunKind::WorkItemPlanSingleCandidateAuthor,
        Some(gate_node),
        relay_outbound_tx,
    )
    .await
    .expect("delegated rerun relay must report spawn success");

    // 修复前：同节点去重把接力静默 drain（Ok(()) 零回执）——在途修订 run 既不被
    // supersede、也不注册接力 run，重跑永不启动（0018 卡死形态）。修复后：接力
    // 放行到 from_handler，取消在途 token 并登记新 run。
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let replaced = fixture
                .manager
                .active_run()
                .await
                .is_some_and(|run| run.token != superseded_token);
            if replaced && superseded_cancel.is_cancelled() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect(
        "delegated rerun relay must supersede the in-flight revision run and register          the rerun (silent drain leaves both untouched)",
    );
    assert!(
        superseded_cancel.is_cancelled(),
        "被接替的在途修订 run 必须收到取消（drain 路径不会取消任何 token）"
    );

    let _ = fixture.manager.abort_active_run().await;
    drain_engine.abort();
}

/// F2 红锚2（spawn 失败可见性）：委托态（SC phase=Generate、无在途 run、durable
/// running）会话的接力 spawn 失败必须有可见结果——回落人工门（durable
/// WaitingForHuman + 门节点摘要携带原因），不得静默滞留 running（0018 卡
/// 20+ 分钟实证）。
#[tokio::test]
async fn sc_delegated_rerun_relay_failure_falls_back_to_human_gate_durable() {
    let (fixture, engine_rx) =
        ProviderRunFixture::new_with_engine_rx(WorkItemPlanFlowKind::SingleCandidate);
    persist_review_rounds(&fixture, 1);
    let mut engine_rx = engine_rx;
    let drain_engine = tokio::spawn(async move { while engine_rx.recv().await.is_some() {} });

    let author_starts = Arc::new(AtomicUsize::new(0));
    let author = Arc::new(ScSequenceAuthorProvider {
        outputs: vec![
            single_candidate_markdown(&fixture.story_id, &fixture.design_id),
            // 修订候选必须与首轮不同（candidate 哈希变更），否则复评落回同一
            // review cycle 触发 scope 违规（真实链的 provider 输出天然不同）。
            single_candidate_markdown(&fixture.story_id, &fixture.design_id).replacen(
                "### Notes",
                "### Notes\n- 修订轮：收紧验证计划。",
                1,
            ),
        ],
        starts: author_starts.clone(),
        held: Mutex::new(Vec::new()),
    });
    let review_starts = Arc::new(AtomicUsize::new(0));
    let reviewer = Arc::new(ScSequenceReviewProvider {
        outputs: vec![sc_pass_review_output(), sc_repairable_revise_output()],
        starts: review_starts.clone(),
        held: Mutex::new(Vec::new()),
    });
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, author);
    registry.register(ProviderName::Codex, reviewer);
    let (context, _outbound_rx, run_context, outbound_tx) =
        sc_context_with_registry(&fixture, registry);

    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: review_enabled_provider_config(),
            reviewer_enabled: true,
        },
    )
    .await;

    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let stage = {
                let engine = fixture.engine.lock().await;
                engine.session().stage.clone()
            };
            if stage == WorkspaceStage::HumanConfirm {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("SC approval gate must open after the passing review");

    let effect = apply_human_gate_feedback(
        fixture.engine.clone(),
        HumanGateFeedbackInput {
            command_id: "f2-red-fallback".to_string(),
            feedback: "请修订当前候选的验证计划".to_string(),
        },
    )
    .await;
    let HumanGateFeedbackEffect::TurnOpened { turn, prompt, .. } = effect else {
        panic!("human gate feedback must open a revision turn");
    };
    spawn_provider_run_from_handler(
        run_context,
        ProviderRunKind::HumanGateScManualRevision {
            turn_id: turn.turn_id,
            prompt,
        },
        outbound_tx,
    )
    .await
    .expect("revision run spawn must succeed");

    // 修订完成 → 复评 revise → policy 委托 → phase=Generate；修订 run 退场。
    // 这就是 0018 的卡死前置态：durable (running, generate)、无在途 run、门未开。
    let delegation_landed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let (phase, status, has_run) = {
                let engine = fixture.engine.lock().await;
                let durable = fixture
                    .lifecycle
                    .get_workspace_session(&fixture.record.id)
                    .expect("reload session");
                (
                    engine.session().single_candidate_phase.clone(),
                    durable.status.clone(),
                    fixture.manager.active_run().await.is_some(),
                )
            };
            if phase == Some(crate::product::models::SingleCandidatePhase::Generate)
                && status == crate::product::models::WorkspaceSessionStatus::Running
                && !has_run
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await;
    if delegation_landed.is_err() {
        let engine = fixture.engine.lock().await;
        let durable = fixture
            .lifecycle
            .get_workspace_session(&fixture.record.id)
            .expect("reload session");
        let has_run = fixture.manager.active_run().await.is_some();
        panic!(
            "delegation must durably land; phase={:?} stage={:?} status={:?} has_run={has_run} author_starts={} review_starts={} diagnostics={:?}",
            engine.session().single_candidate_phase,
            engine.session().stage,
            durable.status,
            author_starts.load(Ordering::SeqCst),
            review_starts.load(Ordering::SeqCst),
            durable.policy_diagnostics,
        );
    }

    // 无附件接力 spawn 失败（author provider 不在 registry）：0018 现场的
    // 「run 未启动且错误不可见」输入形态。
    let mut relay_registry = ProviderRegistry::new();
    relay_registry.register(
        ProviderName::Codex,
        Arc::new(PendingStartProvider {
            starts: Arc::new(AtomicUsize::new(0)),
            held_event_senders: Arc::new(Mutex::new(Vec::new())),
        }),
    );
    let mut relay_context = ProviderRunContext::test_fixture(
        Arc::new(relay_registry),
        fixture.engine.clone(),
        fixture.workspace_runs.clone(),
        fixture.record.id.clone(),
        fixture.app_paths.clone(),
        fixture.record.clone(),
    );
    relay_context.manager = fixture.manager.clone();
    let (relay_outbound_tx, _relay_outbound_rx) = mpsc::channel(8);
    let relay_node = {
        let engine = fixture.engine.lock().await;
        engine.active_timeline_node_id()
    };
    let relay_result = spawn_provider_run_from_event(
        relay_context,
        ProviderRunKind::WorkItemPlanSingleCandidateAuthor,
        relay_node,
        relay_outbound_tx,
    )
    .await;
    assert!(
        relay_result.is_err(),
        "relay spawn must surface the provider-unavailable failure"
    );

    // 修复前：错误只进丢弃通道，会话永久滞留 running（红锚）；修复后：回落
    // 人工门，durable WaitingForHuman + 新门节点摘要携带失败原因。
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let durable = fixture
                .lifecycle
                .get_workspace_session(&fixture.record.id)
                .expect("reload session");
            if durable.status == crate::product::models::WorkspaceSessionStatus::WaitingForHuman {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("delegated relay failure must durably fall back to the human gate");
    let stage = {
        let engine = fixture.engine.lock().await;
        engine.session().stage.clone()
    };
    assert_eq!(
        stage,
        WorkspaceStage::HumanConfirm,
        "接力失败回落后引擎必须停在可操作的人工门"
    );
    let durable_nodes = fixture
        .lifecycle
        .load_timeline_nodes(&fixture.record.id)
        .expect("load durable timeline nodes");
    let gate_summary = durable_nodes
        .iter()
        .rev()
        .find(|node| {
            node.node_type == crate::web::workspace_ws_types::TimelineNodeType::HumanConfirm
                && node.status == crate::web::workspace_ws_types::TimelineNodeStatus::Active
        })
        .and_then(|node| node.summary.clone())
        .unwrap_or_default();
    assert!(
        gate_summary.contains("返修接力"),
        "回落门节点摘要必须携带失败原因，got: {gate_summary}"
    );

    drain_engine.abort();
}

/// SC author 桩：按序产出 outputs，随后挂起（事件端存活但永不完成）。
struct ScSequenceAuthorProvider {
    outputs: Vec<String>,
    starts: Arc<AtomicUsize>,
    held: Mutex<Vec<mpsc::Sender<ProviderEvent>>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ScSequenceAuthorProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let start = self.starts.fetch_add(1, Ordering::SeqCst) + 1;
        let (event_tx, event_rx) = mpsc::channel(1);
        let (command_tx, _command_rx) = mpsc::channel(1);
        if let Some(output) = self.outputs.get(start - 1).cloned() {
            let _ = event_tx
                .send(ProviderEvent::Completed(ProviderCompletion::plain(
                    output, None,
                )))
                .await;
        } else {
            self.held.lock().await.push(event_tx);
        }
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn run_streaming(
        &self,
        _input: &crate::protocol::contracts::AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        unreachable!("workspace runs use start")
    }
}

/// SC reviewer 桩：按序产出 verdict JSON（结构化契约哨兵对齐），随后挂起。
struct ScSequenceReviewProvider {
    outputs: Vec<String>,
    starts: Arc<AtomicUsize>,
    held: Mutex<Vec<mpsc::Sender<ProviderEvent>>>,
}

fn sc_pass_review_output() -> String {
    serde_json::json!({
        "verdict": "pass",
        "review_scope": "outline",
        "generation_round_id": "round-1",
        "summary": "候选自洽，等待人工审批",
        "findings": [],
    })
    .to_string()
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ScSequenceReviewProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let start = self.starts.fetch_add(1, Ordering::SeqCst) + 1;
        let (event_tx, event_rx) = mpsc::channel(1);
        let (command_tx, _command_rx) = mpsc::channel(1);
        if let Some(payload) = self.outputs.get(start - 1).cloned() {
            let contract = input.structured_output_contract.clone();
            let output = match contract.as_ref() {
                Some(contract) => structured_output_sentinel(
                    &contract.nonce,
                    &serde_json::from_str(&payload).expect("review fixture payload"),
                ),
                None => payload,
            };
            let completion = ProviderCompletion::from_output(output, contract.as_ref(), None);
            let _ = event_tx.send(ProviderEvent::Completed(completion)).await;
        } else {
            self.held.lock().await.push(event_tx);
        }
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn run_streaming(
        &self,
        _input: &crate::protocol::contracts::AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        unreachable!("workspace runs use start")
    }
}

/// F2 复活锚：0018 形态的卡死 durable 会话（status=running、门节点仍 Active、
/// phase=Generate、无在途 run）在 manager 重建时走同源引擎守卫回落人工门——
/// durable WaitingForHuman + 新门节点摘要携带原因（续链，人工可再反馈或放弃），
/// 不再滞留 running。注：重建时引擎 stage 取自最后 Active 门节点（HumanConfirm），
/// 不在 F-23 僵尸恢复的 stage 集合内，故必须由本臂覆盖。
#[tokio::test]
async fn sc_delegated_rerun_orphan_reopens_human_gate_on_manager_recreate() {
    let (fixture, engine_rx) =
        ProviderRunFixture::new_with_engine_rx(WorkItemPlanFlowKind::SingleCandidate);
    persist_review_rounds(&fixture, 1);
    let mut engine_rx = engine_rx;
    let drain = tokio::spawn(async move { while engine_rx.recv().await.is_some() {} });
    let author = Arc::new(ScSequenceAuthorProvider {
        outputs: vec![single_candidate_markdown(
            &fixture.story_id,
            &fixture.design_id,
        )],
        starts: Arc::new(AtomicUsize::new(0)),
        held: Mutex::new(Vec::new()),
    });
    let reviewer = Arc::new(ScSequenceReviewProvider {
        outputs: vec![sc_pass_review_output()],
        starts: Arc::new(AtomicUsize::new(0)),
        held: Mutex::new(Vec::new()),
    });
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, author);
    registry.register(ProviderName::Codex, reviewer);
    let (context, _outbound_rx, _run_context, _outbound_tx) =
        sc_context_with_registry(&fixture, registry);
    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: review_enabled_provider_config(),
            reviewer_enabled: true,
        },
    )
    .await;
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let stage = {
                let engine = fixture.engine.lock().await;
                engine.session().stage.clone()
            };
            if stage == WorkspaceStage::HumanConfirm {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("gate must open");

    // 0018 卡死形态：durable (running, generate)、门节点仍 Active、无在途 run。
    {
        let mut record = fixture
            .lifecycle
            .get_workspace_session(&fixture.record.id)
            .expect("reload session");
        record.status = crate::product::models::WorkspaceSessionStatus::Running;
        record.single_candidate_phase =
            Some(crate::product::models::SingleCandidatePhase::Generate);
        let path = fixture
            .app_paths
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id));
        crate::product::json_store::write_json(&path, &record).expect("persist zombie state");
    }

    let root = fixture.root_path();
    let state = WebAppState::new(
        root.clone(),
        crate::web::runtime::WebRuntime::new_fake(root),
    );
    let manager =
        crate::web::workspace_session::WorkspaceSessionManager::create(&state, &fixture.record.id)
            .await
            .expect("manager recreate must succeed");
    let durable = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload after create");
    let nodes = fixture
        .lifecycle
        .load_timeline_nodes(&fixture.record.id)
        .expect("load timeline");
    drop(manager);
    drain.abort();
    assert_eq!(
        durable.status,
        crate::product::models::WorkspaceSessionStatus::WaitingForHuman,
        "委托态孤儿必须在 manager 重建时回落人工门（durable WaitingForHuman）"
    );
    let gate_summary = nodes
        .iter()
        .rev()
        .find(|node| {
            node.node_type == crate::web::workspace_ws_types::TimelineNodeType::HumanConfirm
                && node.status == crate::web::workspace_ws_types::TimelineNodeStatus::Active
        })
        .and_then(|node| node.summary.clone())
        .unwrap_or_default();
    assert!(
        gate_summary.contains("返修接力"),
        "回落门节点摘要必须携带失败原因，got: {gate_summary}"
    );
}
