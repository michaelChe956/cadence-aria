//! bound-sc-generation-teaching（序 3）Task 1：SC generate 段教学重驱的
//! **替换前现状行为基线**（特征化用例，防退化对照；tasks 1.7）。
//!
//! Task 6 收口前的实现以两个进程内布尔槽表达「compile/IR 共享槽 + 结构门
//! 独立槽、每槽至多一次额外 provider 驱动」，槽位不落 durable；Task 6 已
//! 删除双槽、四类失败统一消费同一耐久教学窗口（establish → 漂移核验 →
//! claim）。本文件钉住四个可观察面：
//!
//! 1. 非可教学失败（mixed operational 集）⇒ 零额外 provider 驱动，首轮
//!    启动后 durable `provider_start_ledger` 的 `single_candidate_author:`
//!    前缀条目数**零增加**；
//! 2. 单次 parse 可教学失败 ⇒ 物理上**恰好一次**额外驱动（原调用 + 1 =
//!    合计 2 次输入），A/B 池计数（`run_history.repairs_used` /
//!    `manual_repairs_used`）增量为 0；教学 claim 追加一条 durable 占位
//!    并原位回填（前缀条目数 +1，收口后复测值）；
//! 3. 槽位耗尽后再次失败 ⇒ 终态 WS 错误事件与既有失败收口一致（两轮
//!    诊断 + 显式重开指引 + phase Failed），无第三次驱动，错误面不渲染成
//!    模型可修语法错误（不含重驱 prompt 的指令措辞）；
//! 4. parse 可教学失败后又触发结构门拒收 ⇒ 单窗口收口后原调用之外**一次**
//!    额外驱动（合计 2 次输入；Task 6 改写自双槽基线值 +2/合计 3）。
//!
//! 基线即绿：断言数值一律是「基线对照值，Task 6 收口后已复测」。

use super::single_candidate_provider_run::{
    ProviderRunFixture, SequenceOutputProvider, next_error_message, next_provider_input,
    no_more_provider_inputs, single_candidate_context, single_candidate_markdown,
    wait_for_single_candidate_phase, wait_for_stage,
};
use super::*;
use std::sync::atomic::AtomicUsize;

/// durable `provider_start_ledger` 中 `single_candidate_author:` 前缀条目数
/// （基线取值口径：只数 SC author 启动键，不含其它角色/路径条目）。
fn author_ledger_prefix_count(fixture: &ProviderRunFixture) -> usize {
    fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload session for ledger baseline")
        .provider_start_ledger
        .iter()
        .filter(|entry| {
            entry
                .provider_start_idempotency_key
                .starts_with("single_candidate_author:")
        })
        .count()
}

/// 会话观测面预算计数快照（repairs_used / manual_repairs_used）。
fn run_history_budget(fixture: &ProviderRunFixture) -> (u32, u32) {
    let record = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload session for budget baseline");
    (
        record.run_history.repairs_used,
        record.run_history.manual_repairs_used,
    )
}

/// 删除一个 section 块制造 missing_section 类 parse 语法失败（手法同
/// single_candidate_provider_run/reredrive_and_reopen.rs 的同名私有
/// helper；该 helper 非 pub(super)，按基线文件自含原则本地复刻）。
fn markdown_missing_section(story_id: &str, design_id: &str, section_heading: &str) -> String {
    single_candidate_markdown(story_id, design_id)
        .split("\n\n")
        .filter(|chunk| !chunk.starts_with(section_heading))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// WI-001 的 Inputs 消费自己输出的 contract.backend-api，形成 WI-001→WI-001
/// 自环，只在候选结构门暴露（dependency_cycle；手法同
/// tests/single_candidate_structure_reredrive.rs 的同名私有 helper，
/// 本地复刻）。
fn markdown_with_dependency_self_cycle(story_id: &str, design_id: &str) -> String {
    single_candidate_markdown(story_id, design_id).replacen(
        "### Inputs\n\n### Outputs",
        "### Inputs\n- contract_id: contract.backend-internal\n- provider_logical_work_item_id: WI-001\n- required_capabilities: api.backend.read\n- compatibility_policy: require_all\n\n### Outputs",
        1,
    )
}

/// ① 非可教学失败（mixed operational 集：基线解析 fail-closed 混入 IR
/// typed 失败）⇒ 零额外 provider 驱动；首轮启动后 durable 账本前缀条目数
/// 零增加。operational 注入手法同 typed_teaching.rs（删除夹具仓 .git）。
#[tokio::test]
async fn teaching_baseline_no_teachable_failure_no_extra_start() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let issue = crate::product::issue_store::IssueStore::new(fixture.app_paths.clone())
        .get(&fixture.record.project_id, &fixture.record.issue_id)
        .expect("load fixture issue");
    let repository =
        crate::product::repository_store::RepositoryStore::new(fixture.app_paths.clone())
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
    let budget_before = run_history_budget(&fixture);
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
    // 种子前口径：首轮启动已认领原调用预留键后再取基线值。
    let prefix_count_after_first_start = author_ledger_prefix_count(&fixture);
    // 零额外驱动：非可教学失败不得触发任何教学重驱。
    no_more_provider_inputs(&mut input_rx).await;
    let message = next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("validate plan candidate IR failed") && message.contains("基准分支"),
        "mixed operational 集合按既有 IR 终态面收口: {message}"
    );
    assert!(
        !message.contains("after one teaching re-drive"),
        "非可教学失败零重驱: {message}"
    );
    wait_for_single_candidate_phase(
        &fixture,
        crate::product::models::SingleCandidatePhase::Failed,
    )
    .await;
    // 基线对照值（Task 6 后须复测）：失败收口不追加任何 SC author 启动键。
    assert_eq!(
        author_ledger_prefix_count(&fixture),
        prefix_count_after_first_start,
        "非可教学失败 ⇒ durable 账本 single_candidate_author: 前缀条目数零增加"
    );
    assert_eq!(
        run_history_budget(&fixture),
        budget_before,
        "非可教学失败不得扣减 A/B 池计数"
    );
}

/// ② 单次 parse 可教学失败（missing_section）⇒ 物理上恰好一次额外驱动
/// （原调用 + 1 = 合计 2 次输入），A/B 池计数增量 0；durable 账本前缀条目
/// 数不随重驱增长（重驱不单独入账——现状基线对照值，Task 4/6 后须复测）。
#[tokio::test]
async fn teaching_baseline_single_teachable_failure_exactly_one_extra_start() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(SequenceOutputProvider {
        outputs: vec![
            markdown_missing_section(&fixture.story_id, &fixture.design_id, "### Goal"),
            single_candidate_markdown(&fixture.story_id, &fixture.design_id),
        ],
        inputs: input_tx,
        next: AtomicUsize::new(0),
    });
    let budget_before = run_history_budget(&fixture);
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

    let first_input = next_provider_input(&mut input_rx).await;
    assert!(
        first_input.prompt.contains("[markdown_grammar]"),
        "首轮必须保持完整 markdown author prompt"
    );
    // 恰好一次额外驱动（合计 2 次）：重驱轮携带 typed 失败原文回灌。
    let reredrive_input = next_provider_input(&mut input_rx).await;
    assert!(
        reredrive_input.prompt.contains("missing_section")
            && reredrive_input
                .prompt
                .contains("立即输出完整 work-item-plan markdown source"),
        "parse 教学重驱 prompt 必须携带失败原文与完整重输出指令: {}",
        reredrive_input.prompt
    );
    no_more_provider_inputs(&mut input_rx).await;
    wait_for_stage(&fixture.engine, WorkspaceStage::HumanConfirm).await;
    let durable = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload single-candidate session");
    assert_eq!(
        durable.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Approval),
        "重驱恢复后必须回到既有确认链"
    );
    // 复测值（Task 6 收口后）：物理额外启动 1 次；教学 claim 追加一条
    // durable 占位并原位回填（前缀条目数 +1）。
    assert_eq!(
        author_ledger_prefix_count(&fixture),
        2,
        "收口后：教学重驱恰好追加一条 durable 账本占位（合计 2 次物理启动）"
    );
    // 基线对照值：教学重驱不扣 A/B 池（两计数增量均为 0）。
    assert_eq!(
        run_history_budget(&fixture),
        budget_before,
        "教学重驱不得扣减 repairs_used / manual_repairs_used"
    );
}

/// ③ 槽位耗尽后再次失败 ⇒ 终态 WS 错误事件与既有失败收口一致（两轮诊断 +
/// 显式重开指引 + phase Failed），无第三次驱动；错误面不渲染成模型可修
/// 语法错误（不含重驱 prompt 的指令措辞）。
#[tokio::test]
async fn teaching_baseline_terminal_face_after_exhaustion() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(SequenceOutputProvider {
        outputs: vec![
            markdown_missing_section(&fixture.story_id, &fixture.design_id, "### Goal"),
            markdown_missing_section(&fixture.story_id, &fixture.design_id, "### Tasks"),
        ],
        inputs: input_tx,
        next: AtomicUsize::new(0),
    });
    let budget_before = run_history_budget(&fixture);
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
    // 槽位被首轮可教学失败消耗：重驱轮再败 ⇒ 槽位耗尽，直接终态。
    let reredrive_input = next_provider_input(&mut input_rx).await;
    no_more_provider_inputs(&mut input_rx).await;
    let message = next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("compile markdown source failed"),
        "终态必须保持既有失败收口前缀: {message}"
    );
    assert!(
        message.contains("first round") && message.contains("re-drive round"),
        "终态失败必须含两轮错误: {message}"
    );
    assert!(
        message.matches("missing_section").count() >= 2,
        "终态必须含两轮错误原文: {message}"
    );
    assert!(
        message.contains("请显式重新开始生成"),
        "终态失败必须带显式重开指引: {message}"
    );
    // 错误面不被渲染成模型可修语法错误：终态是 WS 错误事件，不得携带
    // 重驱 prompt 的指令措辞（教学指令只进 provider prompt，不进错误面）。
    //
    // ⚠️ 否定标记必须**随真实措辞自校准**：从**本次实际观察到的重驱 prompt**
    // 里取指令块（首段，失败事实在其后），而不是硬编码两句旧措辞——否则教学
    // 块一改写（effort #6 将把教学块迁出 prompts.rs），字面量从错误面消失
    // ⇒ 断言恒真空转、失去防退化作用；且新措辞漏进错误面时也抓不到。
    // 提取不到指令块 ⇒ **响亮失败**（不得静默空转）。
    let instruction_block = reredrive_input
        .prompt
        .split("\n\n")
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();
    let instruction_markers: Vec<&str> = instruction_block
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    assert!(
        !instruction_markers.is_empty(),
        "重驱 prompt 的指令块必须可提取（否则负向断言空转）: {}",
        reredrive_input.prompt
    );
    for marker in &instruction_markers {
        assert!(
            !message.contains(marker),
            "槽位耗尽的终态错误面不得渲染成模型可修语法错误（含重驱指令行）: {marker}"
        );
    }
    wait_for_single_candidate_phase(
        &fixture,
        crate::product::models::SingleCandidatePhase::Failed,
    )
    .await;
    // 复测值（Task 6 收口后）：耗尽后不得追加任何启动键（教学 claim 的
    // 那一条占位已在第二次失败前发生——对照首轮预留恰 +1）。
    assert_eq!(
        author_ledger_prefix_count(&fixture),
        2,
        "槽位耗尽后不得追加 durable 账本启动键（第二次失败前恰一条 claim 占位）"
    );
    assert_eq!(
        run_history_budget(&fixture),
        budget_before,
        "终态收口不得扣减 A/B 池计数"
    );
}

/// ④ parse 可教学失败后再触发结构门拒收（原双槽序列）⇒ 单窗口收口：
/// 物理上原调用之外**一次**额外驱动（合计 2 次输入），结构类第二次
/// claim `Denied(Exhausted)` ⇒ 链停止（终态 Failed，不开人工门）、
/// refs 保持最后合法值、前缀条目合计 2（第三次为 0）。
/// Task 6 改写自双槽基线值（+2/合计 3 → +1/合计 2）并更名。
#[tokio::test]
async fn teaching_consolidated_dual_slot_sequence_single_extra_start() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(SequenceOutputProvider {
        outputs: vec![
            // 第 1 轮：missing_section（parse 语法类）→ 消费窗口 claim。
            markdown_missing_section(&fixture.story_id, &fixture.design_id, "### Goal"),
            // 第 2 轮：parse/IR 通过，候选结构门拒收（依赖环）→ 窗口已
            // 耗尽，第二次 claim Denied ⇒ 链停止（第 3 轮输出不被消费）。
            markdown_with_dependency_self_cycle(&fixture.story_id, &fixture.design_id),
            single_candidate_markdown(&fixture.story_id, &fixture.design_id),
        ],
        inputs: input_tx,
        next: AtomicUsize::new(0),
    });
    let budget_before = run_history_budget(&fixture);
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
    // 额外驱动 1/1：parse 教学重驱（消费同一窗口）。
    let parse_reredrive_input = next_provider_input(&mut input_rx).await;
    assert!(
        parse_reredrive_input.prompt.contains("missing_section"),
        "第二轮必须是 parse 教学重驱: {}",
        parse_reredrive_input.prompt
    );
    // 单窗口收口：结构拒收不再有独立槽 ⇒ 无第三次输入，链停止于终态。
    no_more_provider_inputs(&mut input_rx).await;
    let message = next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("candidate structure gate rejected")
            && message.contains("first round")
            && message.contains("missing_section")
            && message.contains("请显式重新开始生成"),
        "结构类 Denied 必须按既有失败面回显两轮真实原因: {message}"
    );
    wait_for_single_candidate_phase(
        &fixture,
        crate::product::models::SingleCandidatePhase::Failed,
    )
    .await;
    let durable = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload single-candidate session");
    assert!(
        durable.work_item_plan_source_revision_ref.is_none()
            && durable.plan_candidate_ir_ref.is_none(),
        "链停止时不得提升坏 candidate（refs 保持最后合法值）"
    );
    // 复测值（Task 6 收口后）：物理额外启动 1 次；教学 claim 恰一条占位
    // （前缀条目合计 2，第三次为 0）。
    assert_eq!(
        author_ledger_prefix_count(&fixture),
        2,
        "收口后：原调用 + 一次教学 = 前缀条目合计 2（第三次为 0）"
    );
    assert_eq!(
        run_history_budget(&fixture),
        budget_before,
        "单窗口重驱不得扣减 repairs_used / manual_repairs_used"
    );
}
