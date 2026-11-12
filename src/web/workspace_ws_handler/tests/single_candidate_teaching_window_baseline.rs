//! bound-sc-generation-teaching（序 3）Task 1：SC generate 段教学重驱的
//! **替换前现状行为基线**（特征化用例，防退化对照；tasks 1.7）。
//!
//! 现实现用两个进程内布尔槽（`compile_ir_rerun_used` / `structure_rerun_used`，
//! 声明/消费/置位点见 run/single_candidate.rs）表达「compile/IR 共享槽 +
//! 结构门独立槽、每槽至多一次额外 provider 驱动」，槽位不落 durable。本
//! 文件在**未改实现**上钉住四个可观察面，供 Task 6 单槽收口后逐条比对：
//!
//! 1. 非可教学失败（mixed operational 集）⇒ 零额外 provider 驱动，首轮
//!    启动后 durable `provider_start_ledger` 的 `single_candidate_author:`
//!    前缀条目数**零增加**；
//! 2. 单次 parse 可教学失败 ⇒ 物理上**恰好一次**额外驱动（原调用 + 1 =
//!    合计 2 次输入），A/B 池计数（`run_history.repairs_used` /
//!    `manual_repairs_used`）增量为 0（现状本就不扣，基线对照值）；durable
//!    账本前缀条目数不随重驱增长（重驱不单独入账——现状基线对照值，
//!    Task 4/6 后须复测）；
//! 3. 槽位耗尽后再次失败 ⇒ 终态 WS 错误事件与既有失败收口一致（两轮
//!    诊断 + 显式重开指引 + phase Failed），无第三次驱动，错误面不渲染成
//!    模型可修语法错误（不含重驱 prompt 的指令措辞）；
//! 4. parse 可教学失败后又触发结构门拒收 ⇒ 双槽互不挤占，物理上原调用
//!    之外**两次**额外驱动（合计 3 次输入；双槽现状基线对照值；Task 6
//!    收口后本用例期望值改写为合计 2 次输入并更名
//!    `teaching_consolidated_dual_slot_sequence_single_extra_start`）。
//!
//! 基线即绿：本文件是特征化基线，不是 TDD 红态——四条用例都在未改实现
//! 上直接通过；断言数值一律是「基线对照值，Task 6 后须复测」。

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
    let prefix_count_after_first_start = author_ledger_prefix_count(&fixture);
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
    // 基线对照值（Task 4/6 后须复测）：物理额外启动 1 次，durable 账本不随
    // 重驱单独入账（前缀条目数保持首轮预留，不增）。
    assert_eq!(
        author_ledger_prefix_count(&fixture),
        prefix_count_after_first_start,
        "现状：教学重驱不追加 durable 账本条目（Task 4 落实际启动观察后须复测）"
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
    let prefix_count_after_first_start = author_ledger_prefix_count(&fixture);
    // 槽位被首轮可教学失败消耗：重驱轮再败 ⇒ 槽位耗尽，直接终态。
    let _reredrive_input = next_provider_input(&mut input_rx).await;
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
    assert!(
        !message.contains("立即输出完整 work-item-plan markdown source")
            && !message.contains("重新输出完整 plan"),
        "槽位耗尽的终态错误面不得渲染成模型可修语法错误: {message}"
    );
    wait_for_single_candidate_phase(
        &fixture,
        crate::product::models::SingleCandidatePhase::Failed,
    )
    .await;
    // 基线对照值（Task 6 后须复测）：耗尽后不得追加任何启动键。
    assert_eq!(
        author_ledger_prefix_count(&fixture),
        prefix_count_after_first_start,
        "槽位耗尽后不得追加 durable 账本启动键"
    );
    assert_eq!(
        run_history_budget(&fixture),
        budget_before,
        "终态收口不得扣减 A/B 池计数"
    );
}

/// ④ parse 可教学失败（missing_section 占 compile/IR 槽）后再触发结构门
/// 拒收（dependency_cycle 占结构槽）⇒ 双槽互不挤占：物理上原调用之外
/// **两次**额外驱动（合计 3 次输入）——双槽现状基线对照值；Task 6 收口后
/// 本用例改写为合计 2 次输入并更名
/// `teaching_consolidated_dual_slot_sequence_single_extra_start`。durable
/// 账本前缀条目数保持首轮预留（重驱不单独入账，基线对照值）。
#[tokio::test]
async fn teaching_baseline_dual_slot_sequence_current_behavior() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(SequenceOutputProvider {
        outputs: vec![
            // 第 1 轮：missing_section（parse 语法类）→ 占 compile/IR 槽。
            markdown_missing_section(&fixture.story_id, &fixture.design_id, "### Goal"),
            // 第 2 轮：parse/IR 通过，候选结构门拒收（依赖环）→ 结构门槽。
            markdown_with_dependency_self_cycle(&fixture.story_id, &fixture.design_id),
            // 第 3 轮：结构门重驱后的合法候选。
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

    let _first_input = next_provider_input(&mut input_rx).await;
    let prefix_count_after_first_start = author_ledger_prefix_count(&fixture);
    // 额外驱动 1/2：parse 教学重驱（compile/IR 槽）。
    let parse_reredrive_input = next_provider_input(&mut input_rx).await;
    assert!(
        parse_reredrive_input.prompt.contains("missing_section"),
        "第二轮必须是 parse 教学重驱: {}",
        parse_reredrive_input.prompt
    );
    // 额外驱动 2/2：结构门教学重驱（双槽现状：结构槽不被 parse 槽挤占）。
    let structure_reredrive_input = next_provider_input(&mut input_rx).await;
    assert!(
        structure_reredrive_input
            .prompt
            .contains("final_compile_structure_rejected")
            && structure_reredrive_input.prompt.contains("依赖环"),
        "第三轮必须是结构门教学重驱（结构槽独立）: {}",
        structure_reredrive_input.prompt
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
        "双槽重驱恢复后必须回到既有审批链"
    );
    // 基线对照值（Task 6 后须复测）：物理额外启动 2 次（双槽现状），durable
    // 账本不随任一重驱单独入账（前缀条目数保持首轮预留，不增）。
    assert_eq!(
        author_ledger_prefix_count(&fixture),
        prefix_count_after_first_start,
        "现状：双槽重驱均不追加 durable 账本条目（Task 4/6 后须复测）"
    );
    assert_eq!(
        run_history_budget(&fixture),
        budget_before,
        "双槽重驱不得扣减 repairs_used / manual_repairs_used"
    );
}
