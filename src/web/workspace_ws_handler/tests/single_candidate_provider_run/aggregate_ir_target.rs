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

/// C2（tasks 7.1）：从真实 provider 输入提取首轮 prompt 的 `structure:` 段
/// （`[confirmed_context]` 内 `structure:` 起至 `baseline:`/`story_spec_ids:`
/// 上一行止，段边界与 build_work_item_plan_markdown_prompt 模板一致）。
fn sc_structure_section(prompt: &str) -> &str {
    let start = prompt
        .find("\nstructure:")
        .expect("first-round prompt must carry a structure: section")
        + "\nstructure:".len();
    let rest = &prompt[start..];
    let end = ["\nbaseline:", "\nstory_spec_ids:"]
        .iter()
        .filter_map(|marker| rest.find(marker))
        .min()
        .unwrap_or(rest.len());
    &rest[..end]
}

/// C2：真链启动并捕获首轮 provider 输入，等待收口（HumanConfirm）后确认
/// 零额外 provider 启动——三个接线用例共用的驱动骨架。
async fn sc_start_and_capture_first_input(
    fixture: &ProviderRunFixture,
    provider: Arc<dyn StreamingProviderAdapter>,
    input_rx: &mut mpsc::UnboundedReceiver<StreamingProviderInput>,
) -> StreamingProviderInput {
    let (context, _outbound_rx) = single_candidate_context(fixture, provider);
    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
            resume_recovery: None,
        },
    )
    .await;
    let first_input = next_provider_input(input_rx).await;
    wait_for_stage(&fixture.engine, WorkspaceStage::HumanConfirm).await;
    no_more_provider_inputs(input_rx).await;
    first_input
}

/// C2（tasks 7.1，REQ-WSC-06「成员仓身份头在预算内全部在场」）：双成员
/// 聚合根 + focus=FirstMember（session 仓解析到成员 checkout ⇒
/// repository.path 是成员目录而非聚合根——聚合根必须取 launch 的
/// working_dir）真链首轮 prompt：`structure:` 段必须含全部成员身份头、
/// 子项按聚合根相对前缀呈现、整段 ≤3,200 B、无显式失败标记、除首轮外
/// 零额外 provider 启动。改前单仓摘要只列成员 checkout 内部条目
/// （`dir: .claude`），身份头完全缺席 ⇒ 本用例改前因断言缺失而红。
#[tokio::test]
async fn sc_first_round_structure_carries_all_member_headers_within_budget() {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let output = Arc::new(std::sync::Mutex::new(String::new()));
    let provider = Arc::new(AggregateAuthorProvider {
        output: output.clone(),
        inputs: input_tx,
    });
    let fixture = ProviderRunFixture::new_logical_multi(
        vec![
            MemberRuleMaterial::Present("## 语言规则\n\n- 聚合根成员 0 规则原文。\n".to_string()),
            MemberRuleMaterial::Present("## 语言规则\n\n- 成员 1 规则原文。\n".to_string()),
        ],
        Some("## 语言规则\n\n- 聚合根规则（launch 材料锚）。\n".to_string()),
        LogicalAliasFocus::FirstMember,
        provider.clone(),
    );
    let manifest =
        crate::product::logical_codebase::LogicalCodebaseStore::new(fixture.app_paths.clone())
            .load_manifest("project_0001")
            .expect("load alias lc manifest")
            .expect("alias lc manifest exists");
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
    let first_input = sc_start_and_capture_first_input(&fixture, provider, &mut input_rx).await;

    let member_store =
        crate::product::logical_codebase::LogicalCodebaseStore::new(fixture.app_paths.clone());
    let checkouts = member_store
        .list_checkouts("project_0001")
        .expect("list fixture checkouts");
    let structure = sc_structure_section(&first_input.prompt);
    let mut headers = Vec::new();
    for member_id in [member0, member1] {
        let physical = member_store
            .load_member("project_0001", *member_id)
            .expect("member record")
            .expect("member exists")
            .physical_repository_id;
        let prefix = checkouts
            .iter()
            .find(|checkout| checkout.logical_repository_id == *member_id)
            .expect("member checkout")
            .canonical_path
            .file_name()
            .expect("checkout dir name")
            .to_string_lossy()
            .into_owned();
        headers.push((format!("member {physical}: root {prefix}"), prefix));
    }
    assert!(
        structure.len() <= 3_200,
        "structure 段必须 ≤3,200 B（实测 {} B）: {structure}",
        structure.len()
    );
    for (header, prefix) in &headers {
        assert!(
            structure.contains(header.as_str()),
            "全部成员身份头必须在场（缺 {header}）: {structure}"
        );
        assert!(
            structure.contains(&format!("  {prefix}/.claude/")),
            "子项必须按聚合根相对前缀呈现（缺 {prefix}/.claude/）: {structure}"
        );
    }
    assert_eq!(
        structure
            .lines()
            .filter(|line| line.starts_with("member "))
            .count(),
        headers.len(),
        "身份头行数必须恰等于成员数: {structure}"
    );
    assert!(
        !structure.contains("member_inventory_budget_exceeded")
            && !structure.contains("repository_structure_unavailable")
            && !structure.contains("dir: .claude"),
        "预算内不得出现显式失败标记，也不得退化为成员 checkout 内部清单: {structure}"
    );
}

/// C2（tasks 7.1，REQ-WSC-06「单仓摘要逐字不变」）：成员集合缺席（无
/// gateway launch 的 legacy 单仓）真链——`structure:` 段必须与改前单仓
/// fixture 逐字节一致（与 `summarize_repository_structure(path, false)`
/// 原实现直算值比对；该 fixture 的 LC 权威登记在但 launch 缺席 ⇒ 成员
/// 缺席回退分支生效）。
#[tokio::test]
async fn sc_structure_section_byte_identical_when_members_absent() {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let output = Arc::new(std::sync::Mutex::new(String::new()));
    let provider = Arc::new(AggregateAuthorProvider {
        output: output.clone(),
        inputs: input_tx,
    });
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    *output.lock().expect("set provider output") =
        single_candidate_markdown(&fixture.story_id, &fixture.design_id);
    let pre_change_summary =
        crate::product::work_item_split_engine::context::summarize_repository_structure(
            &fixture.member_checkout_root(),
            false,
        );
    let first_input = sc_start_and_capture_first_input(&fixture, provider, &mut input_rx).await;
    assert_eq!(
        sc_structure_section(&first_input.prompt),
        pre_change_summary,
        "成员缺席时 structure 段必须与改前单仓摘要逐字节一致"
    );
}

/// C2（tasks 7.3，REQ-WSC-32「被省略条目的完整诊断可从审计面取回」——
/// A1 单独交付时无法在真链上断言的那条）：40 个裸 Work Item 标题 ⇒ ~560
/// 条 missing_section parse 诊断 ⇒ 教学事实块 ≫32,000 B 必有整条省略。
/// 断言：重驱 prompt 显式记账（总数/呈现/省略）且审计分区不进 prompt；
/// durable 流水（candidate-attempts）的 [sc_parse_fact_audit] 分区含全部
/// 条目的完整五字段，呈现/省略计数与教学面逐条对得上；failure_id 仍等于
/// sha256(series/stage/code/detail)（审计在 detail 之外 ⇒ 同诊断集失败
/// 身份不变）；恰两次 provider 启动（首轮 + 恰一次教学重驱）。
#[tokio::test]
async fn omitted_parse_diagnostics_fully_retrievable_from_durable_audit() {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let output = Arc::new(std::sync::Mutex::new(String::new()));
    let provider = Arc::new(AggregateAuthorProvider {
        output: output.clone(),
        inputs: input_tx,
    });
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let mut oversized_parse_failure = String::from("# Work Item Plan\n");
    for index in 1..=40 {
        oversized_parse_failure.push_str(&format!(
            "\n## Work Item WI-{index:03}: Fact fixture {index}\n"
        ));
    }
    *output.lock().expect("set provider output") = oversized_parse_failure;
    let (context, mut outbound_rx) = single_candidate_context(&fixture, provider);
    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
            resume_recovery: None,
        },
    )
    .await;
    let _first_input = next_provider_input(&mut input_rx).await;
    let reredrive_input = next_provider_input(&mut input_rx).await;
    no_more_provider_inputs(&mut input_rx).await;
    let _message = next_error_message(&mut outbound_rx).await;
    wait_for_single_candidate_phase(
        &fixture,
        crate::product::models::SingleCandidatePhase::Failed,
    )
    .await;

    let accounting = reredrive_input
        .prompt
        .lines()
        .find(|line| line.contains("parse 事实块有界投影收口"))
        .expect("reredrive prompt must carry the parse fact budget accounting")
        .to_string();
    let count_after = |label: &str| -> usize {
        accounting
            .split(&format!("{label} "))
            .nth(1)
            .and_then(|rest| rest.split(' ').next())
            .and_then(|count| count.parse::<usize>().ok())
            .unwrap_or_else(|| panic!("accounting count after {label}: {accounting}"))
    };
    let (total, shown, omitted) = (count_after("共"), count_after("呈现"), count_after("省略"));
    assert!(omitted > 0, "事实块必须超预算产生省略: {accounting}");
    assert_eq!(total, shown + omitted, "记账必须自洽: {accounting}");
    assert!(
        !reredrive_input.prompt.contains("[sc_parse_fact_audit"),
        "审计分区绝不进 prompt"
    );

    let row = fixture
        .lifecycle
        .list_candidate_attempts(&fixture.record.id)
        .expect("list candidate attempts")
        .into_iter()
        .find(|record| {
            matches!(
                record.stage,
                crate::product::lifecycle_store::candidate_attempts::CandidateAttemptStage::Parse
            )
        })
        .expect("parse stage attempt row");
    let reason = row
        .source_missing_reason
        .as_deref()
        .expect("source_missing_reason");
    let (detail, audit) = reason
        .split_once("\n[sc_parse_fact_audit")
        .expect("audit partition must follow the unchanged detail");
    let audit_entries: Vec<&str> = audit
        .lines()
        .skip(1)
        .filter(|line| !line.is_empty())
        .collect();
    assert_eq!(
        audit_entries.len(),
        total,
        "审计分区必须含全部 {total} 条（含被省略条目），实测 {} 条",
        audit_entries.len()
    );
    let mut shown_in_prompt = 0;
    for entry in &audit_entries {
        let fields: Vec<&str> = entry.split('|').collect();
        assert_eq!(
            fields.len(),
            5,
            "审计条目必须含完整五字段 code|line|field|message|repair_example: {entry}"
        );
        assert!(
            fields.iter().all(|field| !field.is_empty()) && fields[0] == "missing_section",
            "五字段非空且码稳定: {entry}"
        );
        assert!(fields[1].parse::<usize>().is_ok(), "行号可解析: {entry}");
        let prompt_block = format!(
            "- {}:{}:{}\n  缺失字段: {}\n",
            fields[0], fields[1], fields[3], fields[2]
        );
        if reredrive_input.prompt.contains(&prompt_block) {
            shown_in_prompt += 1;
        }
    }
    assert_eq!(
        shown_in_prompt, shown,
        "审计逐条对账：教学面呈现条数与审计在场条数必须一致"
    );

    let durable = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("durable session");
    let series = crate::product::workspace_engine::current_sc_author_business_key(&durable)
        .unwrap_or_else(|| "no-launch".to_string());
    let failure = row.failure.as_ref().expect("failure");
    let expected_failure_id = {
        use sha2::Digest as _;
        hex::encode(sha2::Sha256::digest(
            format!("{}/Parse/{}/{}", series, failure.code, detail).as_bytes(),
        ))
    };
    assert_eq!(
        failure.failure_id, expected_failure_id,
        "审计接入不得改变 failure_id（被哈希 detail 逐字未动）"
    );
}

// append to src/web/workspace_ws_handler/tests/single_candidate_provider_run/aggregate_ir_target.rs
// (included into module `single_candidate_provider_run`: mpsc/Arc/session types/
//  provider_session_with_output/fixture+helpers are already in scope; no new mount).
// RED until P08 wires begin_sc_failure_round before the physical start.

/// Provider whose `start` body *is* the physical start edge: it reads the strict
/// evidence log on disk at that instant and records what it sees, so "RoundBound
/// durable before start" is observed as a fact, not inferred from a zero counter.
struct StartOrderProvider {
    output: String,
    inputs: mpsc::UnboundedSender<StreamingProviderInput>,
    timeline_root: std::path::PathBuf,
    starts: Arc<std::sync::atomic::AtomicUsize>,
    round_bound_at_start: Arc<std::sync::Mutex<Option<bool>>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for StartOrderProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.starts
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let observed = evidence_log_has_round_bound(&self.timeline_root);
        *self.round_bound_at_start.lock().expect("order slot") = Some(observed);
        let _ = self.inputs.send(input);
        provider_session_with_output(self.output.clone()).await
    }

    async fn run_streaming(
        &self,
        _input: &crate::protocol::contracts::AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        unreachable!("workspace provider-run tests use start")
    }
}

/// 本会话严格证据日志（`<timeline>/sc-failure-evidence/*/rounds.jsonl`）是否已含
/// 一条可完整反序列化的 `RoundBound` 条目——按真实 store 条目类型解析 JSON 行。
fn evidence_log_has_round_bound(timeline_root: &std::path::Path) -> bool {
    use crate::product::lifecycle_store::sc_failure_evidence::{
        ScFailureEvidenceEntry, ScFailureEvidenceEvent,
    };
    let Ok(chains) = std::fs::read_dir(timeline_root.join("sc-failure-evidence")) else {
        return false;
    };
    chains.flatten().any(|chain| {
        let Ok(body) = std::fs::read_to_string(chain.path().join("rounds.jsonl")) else {
            return false;
        };
        body.lines().any(|line| {
            serde_json::from_str::<ScFailureEvidenceEntry>(line)
                .map(|entry| matches!(entry.event, ScFailureEvidenceEvent::RoundBound { .. }))
                .unwrap_or(false)
        })
    })
}

fn start_order_provider(
    fixture: &ProviderRunFixture,
) -> (
    Arc<StartOrderProvider>,
    mpsc::UnboundedReceiver<StreamingProviderInput>,
) {
    let (input_tx, input_rx) = mpsc::unbounded_channel();
    let timeline_root = fixture
        .lifecycle
        .workspace_timeline_root_for_session(&fixture.record.id)
        .expect("timeline root");
    let provider = Arc::new(StartOrderProvider {
        output: single_candidate_markdown(&fixture.story_id, &fixture.design_id),
        inputs: input_tx,
        timeline_root,
        starts: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        round_bound_at_start: Arc::new(std::sync::Mutex::new(None)),
    });
    (provider, input_rx)
}

/// P08：SC 原轮物理 start 之前 RoundBound 必须已 durable（start 回调内可读）。
#[tokio::test]
async fn sc_round_bound_is_durable_at_the_physical_start_edge() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (provider, mut input_rx) = start_order_provider(&fixture);
    // start 回调发首轮输入：等到它即证明 start 已观测；helper 继续等 HumanConfirm
    // 并确认零额外启动。
    let _first = sc_start_and_capture_first_input(&fixture, provider.clone(), &mut input_rx).await;

    assert_eq!(
        provider.starts.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "SC 原轮恰一次物理启动"
    );
    assert_eq!(
        *provider.round_bound_at_start.lock().expect("order slot"),
        Some(true),
        "物理 start 回调内必须已能读到完整的 RoundBound 条目（先落盘，后启动）"
    );
    let durable = t17_latest_record(&fixture);
    // SC 生成面的耐久物理启动口径 = durable `provider_start_ledger` 中
    // `single_candidate_author:{id}:` 前缀条目数（Task 7 确立口径，权威实现见
    // tests/single_candidate_teaching_window_fault_matrix.rs 的 prefix_count 与
    // tests/single_candidate_teaching_window_baseline.rs 的
    // author_ledger_prefix_count：教学重驱恰追加一条 ⇒ 原轮后为 1）。
    //
    // 口径说明（为何不是 `is_actual_launch_observation()`）：SC 原轮只落**预领**
    // 条目——`reserve_single_candidate_provider_start`（lifecycle_store/
    // workspace_single_candidate.rs）写入 `started=false / provider=None /
    // started_at=None`（无 provider 回填面），故该条目按
    // `ProviderStartLedgerEntry::is_actual_launch_observation` 口径不是
    // 「实际启动观察」（既有同族断言见 tests/provider_run_events.rs:951-958
    // 「预领占位非实际启动观察」）。
    // 本臂的「实际启动一次」事实由 adapter start 回调计数（上方 `starts == 1`）
    // 与 start 回调内的 RoundBound 严格读回共同承担。
    assert_eq!(
        durable
            .provider_start_ledger
            .iter()
            .filter(|entry| {
                entry
                    .provider_start_idempotency_key
                    .starts_with(&format!("single_candidate_author:{}:", durable.id))
            })
            .count(),
        1,
        "耐久账本恰一条 SC author 启动键（原轮物理启动一次）"
    );
}

/// P08：绑定落盘失败 ⇒ 物理 start 绝不发生，也不 claim 教学预算。
#[tokio::test]
async fn sc_bound_write_failure_blocks_physical_start_and_teaching_claim() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    // 真实故障、非 mock：证据分区路径被普通文件占位，证据目录无法建立。
    let timeline_root = fixture
        .lifecycle
        .workspace_timeline_root_for_session(&fixture.record.id)
        .expect("timeline root");
    std::fs::create_dir_all(&timeline_root).expect("timeline dir");
    std::fs::write(
        timeline_root.join("sc-failure-evidence"),
        b"not-a-directory",
    )
    .expect("inject file-instead-of-dir");

    let (provider, _input_rx) = start_order_provider(&fixture);
    let (context, mut outbound_rx) = single_candidate_context(&fixture, provider.clone());
    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
            resume_recovery: None,
        },
    )
    .await;

    // spawn 只登记 run（不等待执行）：先等可见失败原因，再等 durable 终态。
    let message = next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("sc_failure"),
        "失败原因必须可见（严格证据写失败）: {message}"
    );
    wait_for_single_candidate_phase(
        &fixture,
        crate::product::models::SingleCandidatePhase::Failed,
    )
    .await;

    assert_eq!(
        provider.starts.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "绑定落盘失败不得新增任何 provider start"
    );
    assert_eq!(
        *provider.round_bound_at_start.lock().expect("order slot"),
        None,
        "provider 从未启动"
    );
    let durable = t17_latest_record(&fixture);
    // 口径：此处数 `is_actual_launch_observation()`（provider+started_at 齐备的实际
    // 启动观察）——SC 原轮的**预领**条目不计入（口径见上臂说明）；教学重驱回填
    // 会写入这样的观察条目，故此断言守的是「故障后零实际启动观察（含教学面）」。
    assert_eq!(
        durable
            .provider_start_ledger
            .iter()
            .filter(|entry| entry.is_actual_launch_observation())
            .count(),
        0,
        "耐久账本零 actual-launch 观察（教学回填面同样为零）"
    );
    assert!(
        durable.sc_generation_teaching_windows.is_empty(),
        "绑定失败不 claim 教学窗口"
    );
}
