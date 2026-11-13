// Task 4（MODIFIED REQ-WSC-09）：教学重驱资格 typed 完备判据的端到端用例。
// 资格 = 失败集合里存在可定位、可回喂的 typed 事实（parse 码表 /
// lowering fault / IR fault）；混有 operational 失败（数据库/基线/成员解析）
// 时零重驱。五条核心用例与计划 Review Focus 一一对应：
// ① mixed_operational_failure_set_grants_zero_reredrive（RF①）；
// ② wording_or_prefix_alone_grants_no_eligibility（RF②）；
// ③ second_teachable_category_consumes_no_second_slot（RF⑤）；
// ④ typed_lowering_fault_gets_exactly_one_reredrive_with_typed_payload；
// ⑤ equivalence_canonicalization_audited_to_candidate_attempts（3.3/1.1）。
// verdict 级单测（含 RF①/RF② 的判定函数面）在
// `run/single_candidate_compile_preflight.rs` 的内联 tests 模块。
/// typed lowering fault 夹具：Plan Intent 声明 `logical_codebase` 分支但
/// `intent_target_codebase` 在场且值为空白——parse 层通过（`": "` 分隔符
/// 后的空白值合法），lowering 层 `required()` 判空 ⇒ `RequiredFieldAbsent`
/// 在构造点显式写入，诊断行可定位（字段所在行 > 0）。
/// 注意不能用 `- key:` 裸空值（无 `": "` 分隔符会被 parse 层以
/// unknown_structured_key 拒绝，到不了 lowering）。
fn markdown_with_typed_lowering_fault(story_id: &str, design_id: &str) -> String {
    single_candidate_markdown(story_id, design_id).replacen(
        "### Inputs\n\n",
        "### Plan Intent\n- intent: create\n- provider_work_item_id: WI-001\n- intent_target_kind: logical_codebase\n- intent_target_codebase: \n- intent_target_logical_repo: 00000000-0000-4000-8000-000000000001\n\n### Inputs\n\n",
        1,
    )
}

/// 等价解析采用夹具：`single_repository` 分支的值填在
/// `intent_target_logical_repo`，取夹具登记成员的逻辑仓 UUID ⇒ 成员表
/// `physical_by_logical` 唯一命中被采用，并产出一条
/// `EquivalenceCanonicalization` 审计（调用点在测试内以 manifest 成员
/// UUID 构造，输出可后置）。
fn markdown_with_equivalence_adoption(
    story_id: &str,
    design_id: &str,
    member_logical_id: &str,
) -> String {
    single_candidate_markdown(story_id, design_id).replacen(
        "### Inputs\n\n",
        &format!(
            "### Plan Intent\n- intent: create\n- provider_work_item_id: WI-001\n- intent_target_kind: single_repository\n- intent_target_logical_repo: {member_logical_id}\n\n### Inputs\n\n"
        ),
        1,
    )
}

/// RF①：失败集合同时含 operational 失败（基线解析 fail-closed）与可教类
/// （IR typed fault）时零重驱——禁止 `any(可教)` 放行，按既有 IR 终态面
/// 报 operational 原文。基线 operational 注入：删除夹具仓的 `.git`（默认
/// 基准分支解析 fail-closed，`IssueBaselineError` 诊断进 operational 集）。
#[tokio::test]
async fn mixed_operational_failure_set_grants_zero_reredrive() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let issue = crate::product::issue_store::IssueStore::new(fixture.app_paths.clone())
        .get(&fixture.record.project_id, &fixture.record.issue_id)
        .expect("load fixture issue");
    let repository = crate::product::repository_store::RepositoryStore::new(
        fixture.app_paths.clone(),
    )
    .list(&fixture.record.project_id)
    .expect("list fixture repositories")
    .into_iter()
    .find(|record| Some(record.id.as_str()) == issue.repo_id.as_deref())
    .expect("fixture repository");
    std::fs::remove_dir_all(repository.path.join(".git"))
        .expect("strip .git to inject baseline operational failure");
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(SequenceOutputProvider {
        outputs: vec![
            super::single_candidate_ir_reredrive::markdown_with_unknown_requirement_ref(
                &fixture.story_id,
                &fixture.design_id,
            ),
            super::single_candidate_ir_reredrive::markdown_with_unknown_requirement_ref(
                &fixture.story_id,
                &fixture.design_id,
            ),
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
            resume_recovery: None,
        },
    )
    .await;

    let _full_input = next_provider_input(&mut input_rx).await;
    no_more_provider_inputs(&mut input_rx).await;
    let message = next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("validate plan candidate IR failed"),
        "operational 混入时按既有 IR 终态面收口: {message}"
    );
    assert!(
        message.contains("基准分支"),
        "operational 失败原文（基线解析 fail-closed）必须随终态呈现: {message}"
    );
    assert!(
        !message.contains("after one teaching re-drive"),
        "混合 operational 集合零重驱（禁止 any(可教) 放行）: {message}"
    );
    wait_for_single_candidate_phase(
        &fixture,
        crate::product::models::SingleCandidatePhase::Failed,
    )
    .await;
}

/// RF②（端到端面）：message 含「缺少」措辞、但无 typed fault 的 lowering
/// 失败不获重驱——措辞/前缀不是资格源（`is_lowering_absence_message`
/// 旁路已删，不得以任何形式回来）。verdict 单测面见 preflight 内联测试。
#[tokio::test]
async fn wording_or_prefix_alone_grants_no_eligibility() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let output =
        single_candidate_markdown_with_batched_traceability(&fixture.story_id, &fixture.design_id);
    let provider = Arc::new(RecordingOutputProvider {
        output,
        inputs: input_tx,
    });
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

    let _full_input = next_provider_input(&mut input_rx).await;
    no_more_provider_inputs(&mut input_rx).await;
    let message = next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("compile markdown source failed")
            && message.contains("缺少 requirement_id"),
        "无 typed fault 的 lowering 失败必须按既有路径报原文: {message}"
    );
    assert!(
        !message.contains("after one teaching re-drive"),
        "message 措辞（缺少/缺失）不构成教学重驱资格源: {message}"
    );
    wait_for_single_candidate_phase(
        &fixture,
        crate::product::models::SingleCandidatePhase::Failed,
    )
    .await;
}

/// RF⑤：同一 candidate 先后命中两个可教类别（先 typed lowering fault 后
/// typed IR fault）时，compile/IR 共享槽只放第一次——第二次失败直接终态，
/// 终态含既有 `(after one teaching re-drive)` 文案。
#[tokio::test]
async fn second_teachable_category_consumes_no_second_slot() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(SequenceOutputProvider {
        outputs: vec![
            markdown_with_typed_lowering_fault(&fixture.story_id, &fixture.design_id),
            super::single_candidate_ir_reredrive::markdown_with_unknown_requirement_ref(
                &fixture.story_id,
                &fixture.design_id,
            ),
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
            resume_recovery: None,
        },
    )
    .await;

    let _full_input = next_provider_input(&mut input_rx).await;
    let reredrive_input = next_provider_input(&mut input_rx).await;
    assert!(
        reredrive_input.prompt.contains("intent_target_codebase"),
        "首轮 typed lowering fault 的重驱 prompt 必须携带 typed 载荷: {}",
        reredrive_input.prompt
    );
    no_more_provider_inputs(&mut input_rx).await;
    let message = next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("validate plan candidate IR failed (after one teaching re-drive)"),
        "第二可教类别不获第二次重驱，直接终态: {message}"
    );
    assert!(
        message.contains("first round") && message.contains("re-drive round"),
        "终态必须携带两轮诊断: {message}"
    );
    assert!(
        message.contains("unknown_requirement_ref"),
        "第二轮 IR typed 失败原文必须在场: {message}"
    );
    wait_for_single_candidate_phase(
        &fixture,
        crate::product::models::SingleCandidatePhase::Failed,
    )
    .await;
}

/// RF④ 之外的教学载荷核（typed payload 渲染）：`BranchKeyMismatch`
/// （聚合视图会话，值不在成员表 fail-closed）的重驱 prompt 必须含 field
/// 名、行号、期望形态与原值，且不含裸 message 堆砌；重驱再败 → 终态。
#[tokio::test]
async fn typed_lowering_fault_gets_exactly_one_reredrive_with_typed_payload() {
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
    let manifest =
        crate::product::logical_codebase::LogicalCodebaseStore::new(fixture.app_paths.clone())
            .load_manifest("project_0001")
            .expect("load alias lc manifest")
            .expect("alias lc manifest exists");
    let [member0, member1] = manifest.member_ids.as_slice() else {
        panic!("fixture members");
    };
    let lc_id = crate::product::logical_codebase::store::legacy_logical_codebase_id("project_0001");
    // single_repository 的值填进 intent_target_logical_repo 且不在成员表
    // ⇒ BranchKeyMismatch typed fault（行号取该字段实际行）。
    let bad = aggregate_plan_markdown(
        &fixture.design_id,
        &lc_id,
        &member0.0.to_string(),
        &member1.0.to_string(),
    )
    .replacen(
        &format!(
            "- intent_target_kind: logical_codebase\n- intent_target_codebase: {lc_id}\n- intent_target_logical_repo: {}",
            member0.0
        ),
        "- intent_target_kind: single_repository\n- intent_target_logical_repo: legacy-alias-value-not-in-member-table",
        1,
    );
    let fault_line = bad
        .lines()
        .position(|line| line.trim() == "- intent_target_logical_repo: legacy-alias-value-not-in-member-table")
        .expect("fault line")
        + 1;
    *output.lock().expect("set provider output") = bad;

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

    let _full_input = next_provider_input(&mut input_rx).await;
    let reredrive_input = next_provider_input(&mut input_rx).await;
    for required in [
        "intent_target_repo",
        "- intent_target_repo: value",
        "legacy-alias-value-not-in-member-table",
        &format!("第 {fault_line} 行"),
    ] {
        assert!(
            reredrive_input.prompt.contains(required),
            "typed lowering fault 的重驱 prompt 必须渲染 typed 载荷（{required}）: {}",
            reredrive_input.prompt
        );
    }
    assert!(
        !reredrive_input.prompt.contains("值填在了") && !reredrive_input.prompt.contains("不在成员表"),
        "重驱 prompt 不得回灌裸诊断 message 堆砌: {}",
        reredrive_input.prompt
    );
    no_more_provider_inputs(&mut input_rx).await;
    let message = next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("compile markdown source failed (after one teaching re-drive)"),
        "typed lowering fault 恰一次重驱后仍败必须终态: {message}"
    );
    wait_for_single_candidate_phase(
        &fixture,
        crate::product::models::SingleCandidatePhase::Failed,
    )
    .await;
}

/// 3.3/1.1（handler 侧）：等价解析采用后 candidate-attempts 序列出现对应
/// `EquivalenceCanonicalization` 行——generate 段 handler 是唯一写入点，
/// completion 复验侧不重复写（两次编译仅 1 条审计）。
#[tokio::test]
async fn equivalence_canonicalization_audited_to_candidate_attempts() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    // 成员 UUID 在 fixture 建立时生成：Plan Intent 逐字引用 ⇒ 成员表
    // physical_by_logical 唯一命中被采用（输出可后置的序列 provider）。
    let manifest =
        crate::product::logical_codebase::LogicalCodebaseStore::new(fixture.app_paths.clone())
            .load_manifest("project_0001")
            .expect("load alias lc manifest")
            .expect("alias lc manifest exists");
    let [member0] = manifest.member_ids.as_slice() else {
        panic!("fixture members");
    };
    let provider = Arc::new(SequenceOutputProvider {
        outputs: vec![markdown_with_equivalence_adoption(
            &fixture.story_id,
            &fixture.design_id,
            &member0.0.to_string(),
        )],
        inputs: input_tx,
        next: AtomicUsize::new(0),
    });
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

    let _full_input = next_provider_input(&mut input_rx).await;
    no_more_provider_inputs(&mut input_rx).await;
    wait_for_stage(&fixture.engine, WorkspaceStage::HumanConfirm).await;
    let durable = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload single-candidate session");
    assert_eq!(
        durable.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Approval),
        "等价采用夹具必须正常编译收口（审计不改变链路）"
    );
    let attempts = fixture
        .lifecycle
        .list_issue_candidate_attempts(&durable.project_id, &durable.issue_id)
        .expect("read candidate attempt ledger");
    let canonicalizations: Vec<_> = attempts
        .iter()
        .flat_map(|record| record.equivalence_canonicalizations.iter())
        .cloned()
        .collect();
    assert_eq!(
        canonicalizations.len(),
        1,
        "generate 段与 completion 复验两次编译仅落 1 条等价规范化审计: {canonicalizations:?}"
    );
    assert_eq!(canonicalizations[0].kind, "intent_target_repository");
    assert_eq!(canonicalizations[0].original_key, "intent_target_logical_repo");
    assert_eq!(canonicalizations[0].value, member0.0.to_string());
    assert_eq!(canonicalizations[0].resolution_basis, "physical_by_logical");
}

/// 伪编号夹具：Traceability 与 task `requirement_refs` 同时改为 REQ-002
///（WI 内自洽 ⇒ 编译器/IR 校验通过），REQ-002 不在已确认 spec 登记清单。
fn markdown_with_forged_registered_ref(story_id: &str, design_id: &str) -> String {
    single_candidate_markdown(story_id, design_id)
        .replacen("- requirement_refs: REQ-001", "- requirement_refs: REQ-002", 1)
        .replacen("- requirement_id: REQ-001", "- requirement_id: REQ-002", 1)
}

/// 伪编号（r41 B2/r45）分支的覆盖钉子：WI 内自洽但清单外的引用，handler
/// 侧登记清单比对只产呈现原文、**无 typed 事实**（不伪造 locator）⇒ 不授
/// 教学重驱资格，零重驱、按既有 IR 终态面收口（相对 r41「可救一次」的
/// 行为收窄，属 MODIFIED REQ-WSC-09「无 fault 不重驱」的规则内结果）。
#[tokio::test]
async fn forged_registered_ref_gets_no_teaching_reredrive() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    // 夹具 spec 无版本正文 ⇒ 登记清单为空、该分支不可达；种子一版含 REQ-001
    // 的 story 正文使登记清单 = [REQ-001]。
    fixture
        .lifecycle
        .append_version(crate::product::lifecycle_store::AppendSpecVersionInput {
            project_id: fixture.record.project_id.clone(),
            issue_id: fixture.record.issue_id.clone(),
            entity_id: fixture.story_id.clone(),
            markdown: "# Story\n\n- REQ-001：既有能力。\n".to_string(),
            provider_run_refs: Vec::new(),
            review_refs: Vec::new(),
            confirmed_by: None,
        })
        .expect("seed registered requirement ids");
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(SequenceOutputProvider {
        outputs: vec![
            markdown_with_forged_registered_ref(&fixture.story_id, &fixture.design_id),
            markdown_with_forged_registered_ref(&fixture.story_id, &fixture.design_id),
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
            resume_recovery: None,
        },
    )
    .await;

    let _full_input = next_provider_input(&mut input_rx).await;
    no_more_provider_inputs(&mut input_rx).await;
    let message = next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("validate plan candidate IR failed")
            && message.contains("已确认 spec 之外的编号")
            && message.contains("REQ-002"),
        "伪编号分支必须按既有 IR 终态面报原文: {message}"
    );
    assert!(
        !message.contains("after one teaching re-drive"),
        "伪编号分支无 typed 事实，不授教学重驱资格: {message}"
    );
    wait_for_single_candidate_phase(
        &fixture,
        crate::product::models::SingleCandidatePhase::Failed,
    )
    .await;
}

/// Task 4 评审 Important-1 的 E2E 护栏：指令数 > 上限的 compile 失败经
/// handler 组合路径（compile_failure_surface 返回**未截断**指令行 →
/// prompt 渲染器单点截断）时，重驱 prompt 的计数摘要必须报告**真实总数**
///（不是截断后长度 5，也不是被 take 后的条数）。
fn markdown_with_eight_unknown_keys(story_id: &str, design_id: &str) -> String {
    let bogus_keys = (0..8)
        .map(|index| format!("- bogus_key_{index:02}: x\n"))
        .collect::<String>();
    single_candidate_markdown(story_id, design_id).replacen(
        "- reviewer_check_refs: AC-001\n",
        &format!("- reviewer_check_refs: AC-001\n{bogus_keys}"),
        1,
    )
}

#[tokio::test]
async fn reredrive_prompt_instruction_summary_reports_true_total_through_handler_path() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let output = markdown_with_eight_unknown_keys(&fixture.story_id, &fixture.design_id);
    let provider = Arc::new(RecordingOutputProvider {
        output,
        inputs: input_tx,
    });
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

    let _full_input = next_provider_input(&mut input_rx).await;
    let reredrive_input = next_provider_input(&mut input_rx).await;
    assert!(
        reredrive_input
            .prompt
            .contains("共 8 条契约修复指令"),
        "重驱 prompt 的指令摘要必须报告真实总数（8 条，经未截断入参）: {}",
        reredrive_input.prompt
    );
    assert!(
        !reredrive_input.prompt.contains("共 5 条"),
        "摘要不得报告截断后长度（Important-1 缺陷形态）: {}",
        reredrive_input.prompt
    );
    // A3 起 prompt 自含上一轮失败源码全文，源码里本就带着 bogus_key_00..07，
    // 因此「超上限不得逐条透传」只能钉**指令段**，不能对全文做子串否定。
    let (instructions_section, source_section) = reredrive_input
        .prompt
        .split_once("上一轮失败源码（")
        .expect("教学 prompt 必须自带上一轮失败源码块");
    assert!(
        instructions_section.contains("bogus_key_03")
            && !instructions_section.contains("bogus_key_04"),
        "指令面：上限内（前 4 条）逐条在场，超上限以摘要收口: {instructions_section}"
    );
    // 同一处的正向对立面：源码块必须逐字含完整上一轮源码（含被指令面省略的
    // 第 5..8 条），否则「完整失败源自含」就是假的。
    assert!(
        source_section.contains("bogus_key_07"),
        "源码块必须逐字内嵌完整上一轮源码（含指令面已省略的条目）：{source_section}"
    );
    no_more_provider_inputs(&mut input_rx).await;
    let message = next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("compile markdown source failed (after one teaching re-drive)")
            && message.contains("共 8 条契约修复指令"),
        "终态 reasons 面同样以真总数摘要收口: {message}"
    );
    wait_for_single_candidate_phase(
        &fixture,
        crate::product::models::SingleCandidatePhase::Failed,
    )
    .await;
}
