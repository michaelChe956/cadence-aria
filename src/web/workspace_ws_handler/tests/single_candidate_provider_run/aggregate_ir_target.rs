// B1 铁律（多仓 plan 聚合视图缺口）E2E：多成员 LC plan 会话
// （`LogicalAliasFocus::NonUnique` → 会话业务 target=聚合根视图）的
// SC 真链：IR item target 必须按每 WI Plan Intent 成员归属落成员物理仓
// ID（聚合视图 ID 不得流入 IR target），确认 Final Compile 收口
// Confirmed/Completed，work item 逐 WI 落归属成员仓——session_0003 现场
// （Final Compile work_item_target_missing 卡死 owner 验收）的回归。

/// output 可后置的 author provider：fixture 建立（随机成员 UUID 生成）
/// 之后才能构造引用成员 UUID 的 plan markdown。
struct AggregateAuthorProvider {
    output: Arc<std::sync::Mutex<String>>,
    inputs: mpsc::UnboundedSender<StreamingProviderInput>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for AggregateAuthorProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let _ = self.inputs.send(input);
        let output = self.output.lock().expect("output lock").clone();
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
        unreachable!("aggregate tests use start")
    }
}

fn aggregate_plan_markdown(design_id: &str, lc_id: &str, member0: &str, member1: &str) -> String {
    let wi = |id: &str, suffix: &str, title: &str, kind: &str, member: &str, contract: &str| {
        format!(
            "## Work Item {id}: {title}\n\n\
             ### Identity\n- schema_version: 1\n- logical_work_item_id: {id}\n- title: {title}\n- kind: {kind}\n\n\
             ### Goal\n- summary: WHEN a {kind} request arrives THE SYSTEM SHALL return the planned response.\n\n\
             ### Non Goals\n- non_goals: The other member repository is out of scope.\n\n\
             ### Dependencies\n- depends_on: []\n\n\
             ### Plan Intent\n- intent: create\n- provider_work_item_id: {id}\n- intent_target_kind: logical_codebase\n- intent_target_codebase: {lc_id}\n- intent_target_logical_repo: {member}\n\n\
             ### Inputs\n\n\
             ### Outputs\n- contract_id: {contract}\n- capabilities: api.{kind}.read\n\n\
             ### Tasks\n- task_id: TASK-{suffix}\n- statement: WHEN a {kind} request arrives THE SYSTEM SHALL return the planned response.\n- requirement_refs: REQ-{suffix}\n- done_when_refs: AC-{suffix}\n\n\
             ### Write Policy\n- exclusive_scopes: src/{kind}/**\n- forbidden_scopes: web/**\n\n\
             ### Acceptance Criteria\n- criterion_id: AC-{suffix}\n- statement: WHEN a {kind} request arrives THE SYSTEM SHALL expose the planned response.\n- required_evidence: source_diff\n- required_evidence: manual_check\n\n\
             ### Verification\n- check_id: CHECK-{suffix}\n- manual_instruction: Inspect the {kind} response manually.\n- required: true\n- non_zero_test_execution_required: false\n\n\
             ### Handoff Schema\n- required_fields: commit_sha\n- provided_contract_refs: []\n- reviewer_check_refs: AC-{suffix}\n\n\
             ### Blockers\n- reason_code: no_trusted_command_catalog\n- route: operational_gate\n- target_contract_refs: {contract}\n\n\
             ### Traceability\n- source_type: design_spec\n- source_id: {design_id}\n- requirement_id: REQ-{suffix}\n\n"
        )
    };
    format!(
        "# Work Item Plan\n\n{}{}",
        wi(
            "WI-001",
            "001",
            "Backend busi API",
            "backend",
            member0,
            "contract.busi-api"
        ),
        wi(
            "WI-002",
            "002",
            "Web member selector",
            "frontend",
            member1,
            "contract.web-selector"
        ),
    )
}

#[tokio::test]
async fn aggregate_plan_ir_targets_land_member_repositories() {
    let member_material = "## 语言规则\n\n- 聚合根成员 0 规则原文。\n";
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let output = Arc::new(std::sync::Mutex::new(String::new()));
    let provider = Arc::new(AggregateAuthorProvider {
        output: output.clone(),
        inputs: input_tx,
    });
    let fixture = ProviderRunFixture::new_logical_multi(
        vec![
            MemberRuleMaterial::Present(member_material.to_string()),
            MemberRuleMaterial::Present("## 语言规则\n\n- 成员 1 规则原文。\n".to_string()),
        ],
        Some("## 语言规则\n\n- 聚合根规则（launch 材料锚）。\n".to_string()),
        LogicalAliasFocus::NonUnique,
        provider.clone(),
    );
    // 成员 UUID 在 fixture 建立时生成；plan markdown 的 Plan Intent 逐字引用。
    let manifest =
        crate::product::logical_codebase::LogicalCodebaseStore::new(fixture.app_paths.clone())
            .load_manifest("project_0001")
            .expect("load alias lc manifest")
            .expect("alias lc manifest exists");
    assert_eq!(
        manifest.member_ids.len(),
        2,
        "fixture must register two members"
    );
    let [member0, member1] = manifest.member_ids.as_slice() else {
        panic!("fixture members");
    };
    let lc_id = crate::product::logical_codebase::store::legacy_logical_codebase_id("project_0001");
    *output.lock().expect("set provider output") = aggregate_plan_markdown(
        &fixture.design_id,
        &lc_id,
        &member0.0.to_string(),
        &member1.0.to_string(),
    );

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
    let _ = input_rx.recv().await;

    wait_for_stage(&fixture.engine, WorkspaceStage::HumanConfirm).await;
    let durable = t17_latest_record(&fixture);
    assert_eq!(
        durable.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Approval)
    );

    // source→IR：每 WI 的 IR target = 其 Plan Intent 成员归属的物理成员仓 id
    //（绝非聚合视图 id——session_0003 现场 Final Compile work_item_target_missing）。
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
    .expect("stored aggregate plan candidate IR");
    assert_eq!(
        ir_record.ir.items.len(),
        2,
        "aggregate plan must compile 2 WIs"
    );
    let member_store =
        crate::product::logical_codebase::LogicalCodebaseStore::new(fixture.app_paths.clone());
    let member0_phys = member_store
        .load_member("project_0001", *member0)
        .expect("member0 record")
        .expect("member0 exists")
        .physical_repository_id;
    let member1_phys = member_store
        .load_member("project_0001", *member1)
        .expect("member1 record")
        .expect("member1 exists")
        .physical_repository_id;
    assert_ne!(member0_phys, member1_phys, "members must be distinct repos");
    assert_eq!(
        ir_record.ir.items[0].target_repository_id, member0_phys,
        "WI-001 IR target 必须是其成员归属的物理成员仓 id"
    );
    assert_eq!(
        ir_record.ir.items[1].target_repository_id, member1_phys,
        "WI-002 IR target 必须是其成员归属的物理成员仓 id"
    );
    assert!(
        !ir_record.ir.items.iter().any(|item| item
            .target_repository_id
            .starts_with(crate::product::workspace_repository::AGGREGATE_ROOT_VIEW_ID_PREFIX)),
        "聚合视图 id 不得流入任何 IR target（B1 铁律）"
    );

    // 确认 Final Compile：Approval → Completed（聚合会话此前恒
    // work_item_target_missing 卡死 owner 验收）。
    {
        let mut engine = fixture.engine.lock().await;
        engine.enter_policy_valid_work_item_plan_compile().await;
        assert_eq!(
            engine.session().stage,
            WorkspaceStage::Completed,
            "聚合多仓 plan 确认 compile 必须成功收口到 Completed"
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
    assert_eq!(
        plan.work_item_ids.len(),
        2,
        "Final Compile must create both work items"
    );
    // SC Final Compile 经 plan_store 收口（同 anchor_t17 口径）：compile 内
    // logical_targets 强制 per-WI 逻辑成员有效并解析成员物理仓，compile
    // 成功 + Confirmed + 双 work item 即证明聚合会话 per-WI 成员目标生效。
    let terminal = t17_latest_record(&fixture);
    assert_eq!(
        terminal.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Completed)
    );
}
