// Task 2（REQ-WSC-09）：SC compile 教学重驱白名单从 missing_section 扩到 parse 语法类
// 集合（`single_candidate.rs` → `TEACHABLE_PARSE_FAILURE_CODES`：missing_section /
// invalid_ears / invalid_work_item_id）。本文件补齐诊断 open item ⑥ 指出的覆盖缺口：
// 此前没有「非 missing_section 的 parse 类失败触发重驱」的回归网。
//
// 断言面：①invalid_ears 单独触发恰一次、回灌 code:line:message、重驱成功继续链路；
// ②invalid_work_item_id 单独触发恰一次并恢复；③两类 parse 失败先后命中时总共仍只
// 重驱一次（终态带 `(after one teaching re-drive)` 与两轮原文）；④非 parse 类
// （lowering_error）失败零重驱，直接走既有终态路径。

/// `THE SYSTEM SHALL` 后缺半角空格：归一化的三类空白操作不含「关键字后补空格」，
/// 故该 invalid_ears 形态确定性归一化不救（REQ-WSC-02 边界），只剩教学重驱一条
/// 自愈路径——正是本任务要补的网。
fn markdown_with_unrescued_invalid_ears(story_id: &str, design_id: &str) -> String {
    single_candidate_markdown(story_id, design_id).replacen(
        "- statement: WHEN a request arrives THE SYSTEM SHALL return the planned API response.",
        "- statement: WHEN a request arrives THE SYSTEM SHALLreturn the planned API response.",
        1,
    )
}

/// `- task_id` 后缀非数字：parse 语法类 invalid_work_item_id（「标识符必须使用固定
/// 前缀并以数字结尾」）。
fn markdown_with_invalid_work_item_id(story_id: &str, design_id: &str) -> String {
    single_candidate_markdown(story_id, design_id).replacen(
        "- task_id: TASK-001",
        "- task_id: TASK-1X",
        1,
    )
}

/// 非 parse 类 compile 失败：`required_evidence` 值不在 canonical enum 内，parse 全通过、
/// 由 lowering 层报 `lowering_error`。
fn markdown_with_lowering_error(story_id: &str, design_id: &str) -> String {
    single_candidate_markdown(story_id, design_id).replacen(
        "- required_evidence: source_diff",
        "- required_evidence: bogus_kind",
        1,
    )
}

async fn single_candidate_phase_of(
    fixture: &ProviderRunFixture,
) -> Option<crate::product::models::SingleCandidatePhase> {
    fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload single-candidate session")
        .single_candidate_phase
}

/// F-48 形态（条件以 `」` 收尾后直接接关键字）：交付侧确定性归一化直接救回，
/// 不消耗任何 provider 驱动，并落一条可判定「本次交付被救回」的审计事件
/// （REQ-WSC-02）。
fn markdown_with_rescuable_ears_gap(story_id: &str, design_id: &str) -> String {
    single_candidate_markdown(story_id, design_id).replacen(
        "- statement: WHEN a request arrives THE SYSTEM SHALL return the planned API response.",
        "- statement: WHEN 请求到达「后台」THE SYSTEM SHALL 返回既定 API 响应。",
        1,
    )
}

#[tokio::test]
async fn single_candidate_ears_spacing_rescue_skips_reredrive_and_leaves_audit_event() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(RecordingOutputProvider {
        output: markdown_with_rescuable_ears_gap(&fixture.story_id, &fixture.design_id),
        inputs: input_tx,
    });
    let (context, _outbound_rx) = single_candidate_context(&fixture, provider);

    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
        },
    )
    .await;

    // 确定性救回（`」THE SYSTEM SHALL` 前补半角空格）不消耗重驱槽：恰一次 provider 驱动。
    let _first_input = next_provider_input(&mut input_rx).await;
    no_more_provider_inputs(&mut input_rx).await;
    wait_for_stage(&fixture.engine, WorkspaceStage::HumanConfirm).await;
    assert_eq!(
        single_candidate_phase_of(&fixture).await,
        Some(crate::product::models::SingleCandidatePhase::Approval),
    );

    let audit_events: Vec<String> = fixture
        .lifecycle
        .load_timeline_nodes(&fixture.record.id)
        .expect("load timeline nodes")
        .iter()
        .flat_map(|node| {
            fixture
                .lifecycle
                .load_node_detail(&fixture.record.id, &node.node_id)
                .map(|detail| detail.execution_events)
                .unwrap_or_default()
        })
        .filter_map(|event| event["event_id"].as_str().map(str::to_string))
        .filter(|event_id| event_id.starts_with("single_candidate_ears_spacing_normalized_"))
        .collect();
    assert_eq!(
        audit_events.len(),
        1,
        "一次救回必须恰落一条可判定审计事件: {audit_events:?}"
    );
}

#[tokio::test]
async fn single_candidate_invalid_ears_triggers_exactly_one_teaching_reredrive_and_recovers() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(SequenceOutputProvider {
        outputs: vec![
            markdown_with_unrescued_invalid_ears(&fixture.story_id, &fixture.design_id),
            single_candidate_markdown(&fixture.story_id, &fixture.design_id),
        ],
        inputs: input_tx,
        next: AtomicUsize::new(0),
    });
    let (context, _outbound_rx) = single_candidate_context(&fixture, provider);

    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
        },
    )
    .await;

    let first_input = next_provider_input(&mut input_rx).await;
    assert!(
        first_input.prompt.contains("[markdown_grammar]"),
        "first round must stay the full markdown author prompt"
    );
    let reredrive_input = next_provider_input(&mut input_rx).await;
    for required in [
        "立即输出完整 work-item-plan markdown source",
        "invalid_ears",
        "statement 必须符合 EARS 模板",
    ] {
        assert!(
            reredrive_input.prompt.contains(required),
            "invalid_ears 教学重驱提示词必须回灌 {required}: {}",
            reredrive_input.prompt
        );
    }
    no_more_provider_inputs(&mut input_rx).await;

    wait_for_stage(&fixture.engine, WorkspaceStage::HumanConfirm).await;
    assert_eq!(
        single_candidate_phase_of(&fixture).await,
        Some(crate::product::models::SingleCandidatePhase::Approval),
        "重驱成功后必须继续既有链路（而非终态）"
    );
}

#[tokio::test]
async fn single_candidate_invalid_work_item_id_triggers_exactly_one_teaching_reredrive_and_recovers(
) {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(SequenceOutputProvider {
        outputs: vec![
            markdown_with_invalid_work_item_id(&fixture.story_id, &fixture.design_id),
            single_candidate_markdown(&fixture.story_id, &fixture.design_id),
        ],
        inputs: input_tx,
        next: AtomicUsize::new(0),
    });
    let (context, _outbound_rx) = single_candidate_context(&fixture, provider);

    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
        },
    )
    .await;

    let _first_input = next_provider_input(&mut input_rx).await;
    let reredrive_input = next_provider_input(&mut input_rx).await;
    for required in [
        "立即输出完整 work-item-plan markdown source",
        "invalid_work_item_id",
        "标识符必须使用固定前缀并以数字结尾",
    ] {
        assert!(
            reredrive_input.prompt.contains(required),
            "invalid_work_item_id 教学重驱提示词必须回灌 {required}: {}",
            reredrive_input.prompt
        );
    }
    no_more_provider_inputs(&mut input_rx).await;

    wait_for_stage(&fixture.engine, WorkspaceStage::HumanConfirm).await;
    assert_eq!(
        single_candidate_phase_of(&fixture).await,
        Some(crate::product::models::SingleCandidatePhase::Approval),
    );
}

#[tokio::test]
async fn single_candidate_two_parse_class_failures_share_the_single_reredrive_slot() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(SequenceOutputProvider {
        outputs: vec![
            markdown_with_unrescued_invalid_ears(&fixture.story_id, &fixture.design_id),
            markdown_with_invalid_work_item_id(&fixture.story_id, &fixture.design_id),
        ],
        inputs: input_tx,
        next: AtomicUsize::new(0),
    });
    let (context, mut outbound_rx) = single_candidate_context(&fixture, provider);

    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
        },
    )
    .await;

    let _first_input = next_provider_input(&mut input_rx).await;
    let _reredrive_input = next_provider_input(&mut input_rx).await;
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(100), input_rx.recv()).await,
            Ok(Some(_))
        ),
        "两类 parse 失败先后命中时，教学重驱总共仍至多一次"
    );

    let message = next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("compile markdown source failed (after one teaching re-drive)"),
        "重驱后终态必须带既有 (after one teaching re-drive) 语义: {message}"
    );
    assert!(
        message.contains("first round") && message.contains("re-drive round"),
        "终态必须携带两轮诊断: {message}"
    );
    assert!(
        message.contains("invalid_ears") && message.contains("invalid_work_item_id"),
        "两轮原文必须各自在场: {message}"
    );
    assert!(
        message.contains("请显式重新开始生成"),
        "终态必须给出显式重开指引: {message}"
    );
    wait_for_single_candidate_phase(
        &fixture,
        crate::product::models::SingleCandidatePhase::Failed,
    )
    .await;
}

#[tokio::test]
async fn single_candidate_lowering_failure_never_consumes_the_reredrive_slot() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(RecordingOutputProvider {
        output: markdown_with_lowering_error(&fixture.story_id, &fixture.design_id),
        inputs: input_tx,
    });
    let (context, mut outbound_rx) = single_candidate_context(&fixture, provider);

    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
        },
    )
    .await;

    let _first_input = next_provider_input(&mut input_rx).await;
    no_more_provider_inputs(&mut input_rx).await;
    let message = next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("compile markdown source failed") && message.contains("lowering_error"),
        "非 parse 类失败必须按既有路径报原文: {message}"
    );
    assert!(
        !message.contains("after one teaching re-drive"),
        "非 parse 类失败不得消耗重驱槽: {message}"
    );
    wait_for_single_candidate_phase(
        &fixture,
        crate::product::models::SingleCandidatePhase::Failed,
    )
    .await;
}
