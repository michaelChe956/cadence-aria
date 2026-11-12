//! bound-sc-generation-teaching（序 3）Task 6：handler 单槽收口的端到端
//! 验收（REQ-WSC-28/28E，tasks 3.1–3.4、8.1）。
//!
//! 双布尔槽（compile/IR 共享槽 + 结构门独立槽）删除后，四类可教学失败
//! （parse 语法 / 事实完整 typed lowering / 事实完整 typed IR 校验 /
//! 确定性结构拒收）在重驱判定处统一消费**同一**耐久教学窗口：
//! establish（首次）→ verify_frozen_inputs（漂移 ⇒ 零额外调用 +
//! `sc_teaching_frozen_input_drift` 诊断）→ claim；Granted/Replay ⇒ 恰
//! 一次重驱（原调用在内合计两次物理启动）；Denied ⇒ 停止该生成自动链
//! （不提升坏 candidate、不开人工门、不留 repair_reservation 预领）；
//! 第二次交付仍失败 ⇒ close(FailedTerminal)，无第三次启动。
//!
//! 另承接 Task 4 评审遗留：Minor-2 的 web 层接线点（重驱真实启动 ⇒
//! attempt ActualStarted + 占位条目原位补全）折入
//! `parse_failure_consumes_window_and_reruns_once` 断言；冻结输入漂移的
//! 三侧钉死（零额外调用 / 诊断 / 不渲染成可修语法错误）由
//! `frozen_input_drift_zero_extra_and_diagnostic_not_rendered_as_fixable`
//! 承载（store 面的 mark_unknown→Replay 补钉见
//! lifecycle_store/workspace_sc_teaching_window_tests.rs）。

use super::single_candidate_ir_reredrive::markdown_with_unknown_requirement_ref;
use super::single_candidate_provider_run::{
    ProviderRunFixture, SequenceOutputProvider, next_error_message, next_provider_input,
    no_more_provider_inputs, single_candidate_context, single_candidate_markdown,
    wait_for_single_candidate_phase,
};
use super::*;
use crate::product::models::{SingleCandidatePhase, WorkspaceSessionRecord};
use crate::product::planning_failure::PlanningFailureKind;
use crate::product::work_item_plan_policy::{
    ProviderStartLedgerEntry, ScGenerationTeachingWindow, ScTeachingAttemptLifecycle,
    ScTeachingWindowCloseReason, ScTeachingWindowRequest, ScTeachingWindowStatus,
};
use std::sync::atomic::AtomicUsize;

/// durable `provider_start_ledger` 中 `single_candidate_author:` 前缀条目数
/// （取值口径同基线文件：只数 SC author 启动键）。
fn author_ledger_prefix_count(fixture: &ProviderRunFixture) -> usize {
    fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload session for ledger count")
        .provider_start_ledger
        .iter()
        .filter(|entry| {
            entry
                .provider_start_idempotency_key
                .starts_with("single_candidate_author:")
        })
        .count()
}

/// 末尾教学窗口（无窗口 ⇒ panic——仅在教学已发生的用例内使用）。
fn last_teaching_window(fixture: &ProviderRunFixture) -> ScGenerationTeachingWindow {
    fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload session for teaching window")
        .sc_generation_teaching_windows
        .last()
        .expect("teaching window must exist")
        .clone()
}

/// durable 会话记录中的教学窗口总数。
fn teaching_window_count(fixture: &ProviderRunFixture) -> usize {
    fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload session for window count")
        .sc_generation_teaching_windows
        .len()
}

/// durable policy 诊断中是否含指定 code。
fn has_policy_diagnostic(fixture: &ProviderRunFixture, code: &str) -> bool {
    fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload session for diagnostics")
        .policy_diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == code)
}

/// 会话观测面预算计数快照（repairs_used / manual_repairs_used）。
fn run_history_budget(fixture: &ProviderRunFixture) -> (u32, u32) {
    let record = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload session for budget");
    (
        record.run_history.repairs_used,
        record.run_history.manual_repairs_used,
    )
}

/// durable 是否残留 A 通道 repair_reservation 预领（Denied 不得旁路 A 通道）。
fn repair_reservation_absent(fixture: &ProviderRunFixture) -> bool {
    fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload session for repair reservation")
        .repair_reservation
        .is_none()
}

/// Task 7：「用户 turn/hold/receipt」的 durable 观测面（REQ-WSC-28A/28E
/// 「不伪造用户 turn/hold/receipt」）。SC 链的确认门快照本身是既有门面
/// （教学与否都会出现）；真正的 turn 记录是快照内的反馈轮次计数器与
/// 结构预算聚合（StructuralGovernanceState 的 hold/receipt）。
fn user_turn_surfaces(fixture: &ProviderRunFixture) -> UserTurnSurfaces {
    let record = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload session for user turn surfaces");
    let (accepted_feedback_turns, attempts_used) = record
        .human_gate_snapshot
        .map(|snapshot| (snapshot.accepted_feedback_turns, snapshot.attempts_used))
        .unwrap_or((None, 0));
    let (active_holds, user_receipts) = record
        .structural_governance_state
        .map(|state| (state.active_hold_count(), state.user_receipts.len() as u32))
        .unwrap_or((0, 0));
    UserTurnSurfaces {
        accepted_feedback_turns,
        attempts_used,
        active_holds,
        user_receipts,
        human_gate_reservation_present: record.human_gate_reservation.is_some(),
    }
}

/// [`user_turn_surfaces`] 的投影（Task 7）。
struct UserTurnSurfaces {
    accepted_feedback_turns: Option<u32>,
    attempts_used: u32,
    active_holds: u32,
    user_receipts: u32,
    human_gate_reservation_present: bool,
}

/// 删除一个 section 块制造 missing_section 类 parse 语法失败（手法同基线
/// 文件的自含复刻原则）。
fn markdown_missing_section(story_id: &str, design_id: &str, section_heading: &str) -> String {
    single_candidate_markdown(story_id, design_id)
        .split("\n\n")
        .filter(|chunk| !chunk.starts_with(section_heading))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// WI-001 的 Inputs 消费自己输出的 contract.backend-api 形成自环，只在
/// 候选结构门暴露（dependency_cycle；同基线文件自含复刻）。
fn markdown_with_dependency_self_cycle(story_id: &str, design_id: &str) -> String {
    single_candidate_markdown(story_id, design_id).replacen(
        "### Inputs\n\n### Outputs",
        "### Inputs\n- contract_id: contract.backend-internal\n- provider_logical_work_item_id: WI-001\n- required_capabilities: api.backend.read\n- compatibility_policy: require_all\n\n### Outputs",
        1,
    )
}

/// 经 policy route CAS 播种 durable（其余字段保序复制，只改 phase/ledger；
/// 教学窗口字段不在 persist 清单内 ⇒ 既有窗口原样保留）。
fn seed_via_policy_route(
    fixture: &ProviderRunFixture,
    phase: Option<SingleCandidatePhase>,
    ledger: Vec<ProviderStartLedgerEntry>,
) -> WorkspaceSessionRecord {
    let record = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload session before seeding");
    let persist = crate::product::lifecycle_store::PolicyRoutePersist {
        status: record.status.clone(),
        single_candidate_phase: phase,
        run_history: record.run_history.clone(),
        scope: record.review_invocation_scope.clone(),
        gate: record.human_gate_snapshot.clone(),
        diagnostics: record.policy_diagnostics.clone(),
        repair_reservation: None,
        provider_start_ledger: ledger,
        author_rewrite_budget: record.author_rewrite_budget.clone(),
    };
    fixture
        .lifecycle
        .compare_and_save_policy_route(&record, persist)
        .expect("seed durable state via policy route CAS")
}

/// 播种后同步 engine 的 runtime 镜像（避免 run 启动路径用旧镜像判相位）。
async fn reload_engine_session(fixture: &ProviderRunFixture, record: &WorkspaceSessionRecord) {
    fixture
        .engine
        .lock()
        .await
        .reload_session_from_record(record.clone());
}

/// 与真实重驱必然计算出的 digest 全部不同的播种用窗口请求（冻结输入
/// 漂移用例的「旧事实」侧）。
fn drifted_seed_request() -> ScTeachingWindowRequest {
    let digest64 = |seed: &str| format!("{seed:0<64}");
    ScTeachingWindowRequest {
        provider_path_admissible: true,
        failure_source_ref: None,
        failure_source_sha256: digest64("aa"),
        failure_typed_kinds: vec![PlanningFailureKind::Unknown {
            code: "seed".to_string(),
        }],
        business_request_digest: digest64("bb"),
        material_digests: vec![digest64("cc")],
        context_digest: digest64("dd"),
        allowed_edit_closure: "seed closure".to_string(),
        instruction_digest: digest64("ee"),
        prompt_ref: "seed_prompt_builder".to_string(),
        prompt_digest: digest64("ff"),
    }
}

/// 播种前捕获的 fixture 原相位（播种不得破坏 run 启动的 Prepare/Generate 门）。
fn original_phase(fixture: &ProviderRunFixture) -> Option<SingleCandidatePhase> {
    fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload fixture session before seeding")
        .single_candidate_phase
}

async fn start_generation(context: WorkspaceInboundContext) {
    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
            resume_recovery: None,
        },
    )
    .await;
}

/// 3.1：parse 可教学失败 ⇒ establish+claim Granted、恰一次重驱、durable 出现
/// `sc_generation:{session}:0:attempt:2` 且 ActualStarted（Task 4 评审
/// Minor-2 的 web 层接线补钉：真实 run 驱动 ⇒ 占位条目原位补全）。
#[tokio::test]
async fn parse_failure_consumes_window_and_reruns_once() {
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
    start_generation(context).await;

    let _first_input = next_provider_input(&mut input_rx).await;
    // 恰一次重驱：重驱轮携带 typed 失败原文回灌。
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
    super::single_candidate_provider_run::wait_for_stage(
        &fixture.engine,
        WorkspaceStage::HumanConfirm,
    )
    .await;
    let durable = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload single-candidate session");
    assert_eq!(
        durable.single_candidate_phase,
        Some(SingleCandidatePhase::Approval),
        "重驱恢复后必须回到既有确认链"
    );
    // 窗口：一条、两个 attempt、attempt2 已回填 ActualStarted。
    assert_eq!(teaching_window_count(&fixture), 1, "单窗口承载 parse 教学");
    let window = last_teaching_window(&fixture);
    let attempt_key = ScGenerationTeachingWindow::attempt_key(&fixture.record.id, 0, 2);
    assert_eq!(window.attempts.len(), 2, "原调用 + 一次教学重驱");
    let attempt2 = window.attempts.last().expect("attempt 2");
    assert_eq!(attempt2.attempt_key, attempt_key);
    assert!(
        matches!(
            attempt2.lifecycle,
            ScTeachingAttemptLifecycle::Terminal {
                outcome: crate::product::work_item_plan_policy::ScTeachingAttemptOutcome::Delivered
            }
        ),
        "交付成功后教学 attempt 以 Delivered 关窗（ActualStarted 的持久证据\
         见下方占位条目原位补全）: {:?}",
        attempt2.lifecycle
    );
    assert_eq!(
        attempt2.launch_ledger_key.as_deref(),
        Some(format!("single_candidate_author:{}:1", fixture.record.id).as_str()),
        "claim 占位键 = 前缀最大后缀 + 1"
    );
    // 原调用在内合计两次物理启动：前缀条目合计 2，claim 条目已原位补全
    //（绝对计数——claim 与首轮启动是异步落账，相对差值有竞态）。
    assert_eq!(
        author_ledger_prefix_count(&fixture),
        2,
        "教学 claim 恰好追加一条占位并原位补全（原调用 + 教学 = 合计 2 次物理启动）"
    );
    let entry = durable
        .provider_start_ledger
        .iter()
        .find(|entry| entry.provider_attempt_key.as_deref() == Some(attempt_key.as_str()))
        .expect("claim placeholder ledger entry");
    assert!(
        entry.started
            && entry.provider.is_some()
            && entry.started_at.is_some()
            && entry.provider_start_idempotency_key
                == format!("single_candidate_author:{}:1", fixture.record.id),
        "回填必须原位补全占位条目: {entry:?}"
    );
    assert_eq!(
        run_history_budget(&fixture),
        budget_before,
        "教学重驱不得扣减 repairs_used / manual_repairs_used"
    );
}

/// 3.1：结构拒收（此前无 compile 失败）⇒ 同一窗口 claim、一次重驱。
#[tokio::test]
async fn structure_failure_consumes_same_window_when_compile_slot_unused() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(SequenceOutputProvider {
        outputs: vec![
            markdown_with_dependency_self_cycle(&fixture.story_id, &fixture.design_id),
            single_candidate_markdown(&fixture.story_id, &fixture.design_id),
        ],
        inputs: input_tx,
        next: AtomicUsize::new(0),
    });
    let (context, _outbound_rx) = single_candidate_context(&fixture, provider);
    start_generation(context).await;

    let _first_input = next_provider_input(&mut input_rx).await;
    let reredrive_input = next_provider_input(&mut input_rx).await;
    assert!(
        reredrive_input
            .prompt
            .contains("final_compile_structure_rejected")
            && reredrive_input.prompt.contains("依赖环"),
        "结构拒收教学重驱 prompt 必须携带修复指令: {}",
        reredrive_input.prompt
    );
    no_more_provider_inputs(&mut input_rx).await;
    super::single_candidate_provider_run::wait_for_stage(
        &fixture.engine,
        WorkspaceStage::HumanConfirm,
    )
    .await;
    // 同一窗口（不再有独立结构槽）：一条窗口、两个 attempt、成功后关窗。
    assert_eq!(teaching_window_count(&fixture), 1);
    let window = last_teaching_window(&fixture);
    assert_eq!(window.attempts.len(), 2);
    assert!(
        matches!(
            attempt_status(&window, 2),
            Some(&ScTeachingAttemptLifecycle::Terminal {
                outcome: crate::product::work_item_plan_policy::ScTeachingAttemptOutcome::Delivered
            })
        ),
        "结构重驱交付成功后以 Delivered 关窗（真实启动的账本证据随占位原位补全）"
    );
    assert!(
        matches!(
            window.status,
            ScTeachingWindowStatus::Closed {
                reason: ScTeachingWindowCloseReason::Delivered
            }
        ),
        "重驱交付成功必须以 Delivered 关窗: {:?}",
        window.status
    );
    assert_eq!(
        author_ledger_prefix_count(&fixture),
        2,
        "结构类同样只消费窗口的一次 claim（原调用 + 教学 = 合计 2 次物理启动）"
    );
}

fn attempt_status(
    window: &ScGenerationTeachingWindow,
    attempt_no: u8,
) -> Option<&ScTeachingAttemptLifecycle> {
    window
        .attempts
        .iter()
        .find(|attempt| attempt.attempt_no == attempt_no)
        .map(|attempt| &attempt.lifecycle)
}

/// 3.1/8.1：parse 重驱后再结构拒收 ⇒ 第二次 claim `Denied(Exhausted)`、
/// 前缀条目合计 2（第三次为 0）、链停止（终态 Failed，不开人工门）、
/// refs 保持最后合法值（未提升坏 candidate）。
#[tokio::test]
async fn consolidated_sequence_parse_then_structure_single_extra_start() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(SequenceOutputProvider {
        outputs: vec![
            markdown_missing_section(&fixture.story_id, &fixture.design_id, "### Goal"),
            markdown_with_dependency_self_cycle(&fixture.story_id, &fixture.design_id),
            single_candidate_markdown(&fixture.story_id, &fixture.design_id),
        ],
        inputs: input_tx,
        next: AtomicUsize::new(0),
    });
    let budget_before = run_history_budget(&fixture);
    let (context, mut outbound_rx) = single_candidate_context(&fixture, provider);
    start_generation(context).await;

    let _first_input = next_provider_input(&mut input_rx).await;
    let parse_reredrive_input = next_provider_input(&mut input_rx).await;
    assert!(
        parse_reredrive_input.prompt.contains("missing_section"),
        "第二轮必须是 parse 教学重驱: {}",
        parse_reredrive_input.prompt
    );
    // 单窗口收口：结构拒收不再有自己的槽 ⇒ 无第三次启动。
    no_more_provider_inputs(&mut input_rx).await;
    let message = next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("candidate structure gate rejected")
            && message.contains("first round")
            && message.contains("missing_section")
            && message.contains("请显式重新开始生成"),
        "结构类 Denied 必须按既有失败面回显两轮真实原因: {message}"
    );
    wait_for_single_candidate_phase(&fixture, SingleCandidatePhase::Failed).await;
    let durable = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload single-candidate session");
    assert!(
        durable.work_item_plan_source_revision_ref.is_none()
            && durable.plan_candidate_ir_ref.is_none(),
        "链停止时不得提升坏 candidate（refs 保持最后合法值）"
    );
    assert!(
        repair_reservation_absent(&fixture),
        "Denied 不得旁路 A 通道"
    );
    // 窗口：两个 attempt 后关窗（Exhausted），第三次为 0。
    let window = last_teaching_window(&fixture);
    assert_eq!(window.attempts.len(), 2);
    assert!(
        matches!(
            window.status,
            ScTeachingWindowStatus::Closed {
                reason: ScTeachingWindowCloseReason::Exhausted
            }
        ),
        "第二次交付失败必须以 FailedTerminal 关窗: {:?}",
        window.status
    );
    assert!(matches!(
        attempt_status(&window, 2),
        Some(ScTeachingAttemptLifecycle::Terminal {
            outcome:
                crate::product::work_item_plan_policy::ScTeachingAttemptOutcome::FailedTerminal
        })
    ));
    assert_eq!(
        author_ledger_prefix_count(&fixture),
        2,
        "前缀条目合计 2（第三次为 0）"
    );
    assert_eq!(
        run_history_budget(&fixture),
        budget_before,
        "链停止不得扣减 A/B 池计数"
    );
}

/// 8.1：结构重驱后再 parse 失败 ⇒ 同一窗口耗尽、单次额外启动、终态。
#[tokio::test]
async fn consolidated_sequence_structure_then_parse_single_extra_start() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(SequenceOutputProvider {
        outputs: vec![
            markdown_with_dependency_self_cycle(&fixture.story_id, &fixture.design_id),
            markdown_missing_section(&fixture.story_id, &fixture.design_id, "### Tasks"),
            single_candidate_markdown(&fixture.story_id, &fixture.design_id),
        ],
        inputs: input_tx,
        next: AtomicUsize::new(0),
    });
    let (context, mut outbound_rx) = single_candidate_context(&fixture, provider);
    start_generation(context).await;

    let _first_input = next_provider_input(&mut input_rx).await;
    let structure_reredrive_input = next_provider_input(&mut input_rx).await;
    assert!(
        structure_reredrive_input.prompt.contains("依赖环"),
        "第二轮必须是结构门教学重驱: {}",
        structure_reredrive_input.prompt
    );
    no_more_provider_inputs(&mut input_rx).await;
    let message = next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("compile markdown source failed")
            && message.contains("after one teaching re-drive")
            && message.contains("final_compile_structure_rejected")
            && message.contains("missing_section"),
        "parse 类 Denied 必须按既有失败面回显两轮真实原因: {message}"
    );
    wait_for_single_candidate_phase(&fixture, SingleCandidatePhase::Failed).await;
    let window = last_teaching_window(&fixture);
    assert_eq!(window.attempts.len(), 2);
    assert!(
        matches!(
            window.status,
            ScTeachingWindowStatus::Closed {
                reason: ScTeachingWindowCloseReason::Exhausted
            }
        ),
        "窗口耗尽关窗: {:?}",
        window.status
    );
    assert_eq!(
        author_ledger_prefix_count(&fixture),
        2,
        "原调用 + 一次教学 = 前缀条目合计 2（第三次为 0）"
    );
}

/// 8.1：IR 重驱后再结构拒收 ⇒ 同一窗口耗尽、单次额外启动、终态。
#[tokio::test]
async fn consolidated_sequence_ir_then_structure_single_extra_start() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(SequenceOutputProvider {
        outputs: vec![
            markdown_with_unknown_requirement_ref(&fixture.story_id, &fixture.design_id),
            markdown_with_dependency_self_cycle(&fixture.story_id, &fixture.design_id),
            single_candidate_markdown(&fixture.story_id, &fixture.design_id),
        ],
        inputs: input_tx,
        next: AtomicUsize::new(0),
    });
    let (context, mut outbound_rx) = single_candidate_context(&fixture, provider);
    start_generation(context).await;

    let _first_input = next_provider_input(&mut input_rx).await;
    let ir_reredrive_input = next_provider_input(&mut input_rx).await;
    assert!(
        ir_reredrive_input
            .prompt
            .contains("unknown_requirement_ref"),
        "第二轮必须是 IR 教学重驱: {}",
        ir_reredrive_input.prompt
    );
    no_more_provider_inputs(&mut input_rx).await;
    let message = next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("candidate structure gate rejected")
            && message.contains("unknown_requirement_ref")
            && message.contains("请显式重新开始生成"),
        "结构类 Denied 必须回显两轮真实原因: {message}"
    );
    wait_for_single_candidate_phase(&fixture, SingleCandidatePhase::Failed).await;
    let window = last_teaching_window(&fixture);
    assert_eq!(window.attempts.len(), 2);
    assert!(
        matches!(
            window.status,
            ScTeachingWindowStatus::Closed {
                reason: ScTeachingWindowCloseReason::Exhausted
            }
        ),
        "IR→结构序列共享同一窗口耗尽: {:?}",
        window.status
    );
    assert_eq!(author_ledger_prefix_count(&fixture), 2);
}

/// 3.3：混合 Unknown/Operational 失败集合 ⇒ 零 claim、零额外启动、错误面
/// 回显真实原因（不渲染成模型可修语法错误）。
#[tokio::test]
async fn mixed_operational_set_zero_extra_and_real_reason_surfaced() {
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
        .expect("strip .git to inject operational failure");
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(SequenceOutputProvider {
        outputs: vec![markdown_with_unknown_requirement_ref(
            &fixture.story_id,
            &fixture.design_id,
        )],
        inputs: input_tx,
        next: AtomicUsize::new(0),
    });
    let budget_before = run_history_budget(&fixture);
    let (context, mut outbound_rx) = single_candidate_context(&fixture, provider);
    start_generation(context).await;

    let _first_input = next_provider_input(&mut input_rx).await;
    let prefix_count_after_first_start = author_ledger_prefix_count(&fixture);
    no_more_provider_inputs(&mut input_rx).await;
    let message = next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("validate plan candidate IR failed") && message.contains("基准分支"),
        "混合 operational 集合按既有 IR 终态面收口: {message}"
    );
    assert!(
        !message.contains("after one teaching re-drive")
            && !message.contains("立即输出完整 work-item-plan markdown source"),
        "operational 集合零重驱且不渲染成可修语法错误: {message}"
    );
    wait_for_single_candidate_phase(&fixture, SingleCandidatePhase::Failed).await;
    assert_eq!(
        teaching_window_count(&fixture),
        0,
        "非可教学失败不得建立/消费任何窗口"
    );
    assert_eq!(
        author_ledger_prefix_count(&fixture),
        prefix_count_after_first_start,
        "零 claim ⇒ 前缀条目零增加"
    );
    assert_eq!(
        run_history_budget(&fixture),
        budget_before,
        "零重驱不得扣减 A/B 池计数"
    );
}

/// 3.4：窗口 approved 但链已消费第二次物理机会 ⇒ 教学请求零额外、无第三次
/// 启动（判据③「前缀非教学条目 2 条 ⇒ max=1」的建立面变体由
/// denied_establishment 用例与 Task 3 store 用例分别钉住）。
#[tokio::test]
async fn recovery_consumed_second_chance_no_third_start() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(SequenceOutputProvider {
        outputs: vec![
            markdown_missing_section(&fixture.story_id, &fixture.design_id, "### Goal"),
            markdown_missing_section(&fixture.story_id, &fixture.design_id, "### Tasks"),
            single_candidate_markdown(&fixture.story_id, &fixture.design_id),
        ],
        inputs: input_tx,
        next: AtomicUsize::new(0),
    });
    let (context, mut outbound_rx) = single_candidate_context(&fixture, provider);
    start_generation(context).await;

    let _first_input = next_provider_input(&mut input_rx).await;
    let reredrive_input = next_provider_input(&mut input_rx).await;
    no_more_provider_inputs(&mut input_rx).await;
    let message = next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("compile markdown source failed")
            && message.contains("after one teaching re-drive"),
        "第二次交付失败必须按既有失败面收口: {message}"
    );
    wait_for_single_candidate_phase(&fixture, SingleCandidatePhase::Failed).await;
    let window = last_teaching_window(&fixture);
    assert_eq!(
        window.attempts.len(),
        2,
        "第二次物理机会已消费（原调用 + 教学重驱）"
    );
    assert!(
        matches!(
            window.status,
            ScTeachingWindowStatus::Closed {
                reason: ScTeachingWindowCloseReason::Exhausted
            }
        ),
        "第二次机会消费后窗口关闭，无第三次启动: {:?}",
        window.status
    );
    assert_eq!(author_ledger_prefix_count(&fixture), 2, "恰两次物理启动");
    // 第三次为 0 的负向钉子：重驱轮之后无新输入（no_more 已断言）。
    assert!(reredrive_input.prompt.contains("missing_section"));
}

/// 判据不成立（判据③：前缀非教学条目 2 条）⇒ durable 含
/// `sc_teaching_window_not_approved`、零额外启动（「关闭某项即等价于只回退
/// 该项」的对照）。
#[tokio::test]
async fn denied_establishment_records_diagnostic_and_zero_extra() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    // 播种一条非教学前缀条目（policy route 保**原相位**不变——相位被改会
    // 破坏 run 启动的 Prepare/Generate 门；不触发业务键幂等匹配）：
    // 连同本轮原调用 ⇒ 非教学条目 2 条 > 1 ⇒ 判据③ 拒绝。
    let original_phase = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload fixture session before seeding")
        .single_candidate_phase;
    let seeded = seed_via_policy_route(
        &fixture,
        original_phase,
        vec![ProviderStartLedgerEntry {
            provider_start_idempotency_key: format!(
                "single_candidate_author:{}:0",
                fixture.record.id
            ),
            started: true,
            provider: None,
            started_at: None,
            provider_attempt_key: None,
        }],
    );
    reload_engine_session(&fixture, &seeded).await;
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(SequenceOutputProvider {
        outputs: vec![
            markdown_missing_section(&fixture.story_id, &fixture.design_id, "### Goal"),
            single_candidate_markdown(&fixture.story_id, &fixture.design_id),
        ],
        inputs: input_tx,
        next: AtomicUsize::new(0),
    });
    let (context, mut outbound_rx) = single_candidate_context(&fixture, provider);
    start_generation(context).await;

    let _first_input = next_provider_input(&mut input_rx).await;
    let prefix_count_after_first_start = author_ledger_prefix_count(&fixture);
    no_more_provider_inputs(&mut input_rx).await;
    let message = next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("compile markdown source failed") && message.contains("missing_section"),
        "判据不成立仍按既有失败面回显真实原因: {message}"
    );
    assert!(
        !message.contains("after one teaching re-drive"),
        "零额外启动 ⇒ 单轮失败面: {message}"
    );
    wait_for_single_candidate_phase(&fixture, SingleCandidatePhase::Failed).await;
    assert!(
        has_policy_diagnostic(&fixture, "sc_teaching_window_not_approved"),
        "判据不成立必须同事务留痕 durable 诊断"
    );
    let window = last_teaching_window(&fixture);
    assert_eq!(
        (window.max_attempts, window.repair_once_approved),
        (1, false),
        "未批准 ⇒ max=1"
    );
    assert_eq!(
        window.attempts.len(),
        1,
        "未批准窗口不含任何教学 claim（零额外）"
    );
    assert_eq!(
        author_ledger_prefix_count(&fixture),
        prefix_count_after_first_start,
        "零 claim ⇒ 前缀条目零增加（除播种与原调用外）"
    );
    assert!(repair_reservation_absent(&fixture), "不得旁路 A 通道");
}

/// 冻结输入漂移（窗口冻结事实 ≠ 当前失败事实）⇒ 零额外调用 +
/// `sc_teaching_frozen_input_drift` 诊断 + 既有失败面（不渲染成可修语法
/// 错误）三侧钉死。
#[tokio::test]
async fn frozen_input_drift_zero_extra_and_diagnostic_not_rendered_as_fixable() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    // 播种一个已批准的 Active 窗口（冻结事实与真实重驱必然不同）：
    // ① 相位暂置 Generate + 一条非教学前缀条目（建立判据齐备）；
    // ② establish（批准）；
    // ③ 相位拨回**原相位**（run 启动回落「新业务启动」分支，键序号 :1，
    //    不触发业务键幂等匹配、不破坏 Prepare/Generate 门）。
    let original_phase = original_phase(&fixture);
    let entry = ProviderStartLedgerEntry {
        provider_start_idempotency_key: format!("single_candidate_author:{}:0", fixture.record.id),
        started: true,
        provider: None,
        started_at: None,
        provider_attempt_key: None,
    };
    let generate =
        seed_via_policy_route(&fixture, Some(SingleCandidatePhase::Generate), vec![entry]);
    let established = fixture
        .lifecycle
        .establish_sc_teaching_window(&generate, &drifted_seed_request())
        .expect("seed establish teaching window");
    let record = match established {
        crate::product::lifecycle_store::ScTeachingEstablishOutcome::Established {
            record, ..
        } => record,
        _ => panic!("seeding establish must create the window"),
    };
    let seeded = seed_via_policy_route(
        &fixture,
        original_phase,
        record.provider_start_ledger.clone(),
    );
    reload_engine_session(&fixture, &seeded).await;
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(SequenceOutputProvider {
        outputs: vec![
            markdown_missing_section(&fixture.story_id, &fixture.design_id, "### Goal"),
            single_candidate_markdown(&fixture.story_id, &fixture.design_id),
        ],
        inputs: input_tx,
        next: AtomicUsize::new(0),
    });
    let (context, mut outbound_rx) = single_candidate_context(&fixture, provider);
    start_generation(context).await;

    let _first_input = next_provider_input(&mut input_rx).await;
    let prefix_count_after_first_start = author_ledger_prefix_count(&fixture);
    no_more_provider_inputs(&mut input_rx).await;
    let message = next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("compile markdown source failed")
            && message.contains("missing_section")
            && message.contains("请显式重新开始生成"),
        "漂移必须按既有失败面回显真实原因: {message}"
    );
    assert!(
        !message.contains("立即输出完整 work-item-plan markdown source")
            && !message.contains("重新输出完整"),
        "漂移不得渲染成模型可修语法错误: {message}"
    );
    wait_for_single_candidate_phase(&fixture, SingleCandidatePhase::Failed).await;
    assert!(
        has_policy_diagnostic(&fixture, "sc_teaching_frozen_input_drift"),
        "漂移必须留痕 durable 诊断 sc_teaching_frozen_input_drift"
    );
    let window = last_teaching_window(&fixture);
    assert_eq!(
        window.attempts.len(),
        1,
        "漂移 ⇒ 不 claim（窗口 attempts 零变化）"
    );
    assert!(
        matches!(window.status, ScTeachingWindowStatus::Active),
        "漂移不消费窗口: {:?}",
        window.status
    );
    assert_eq!(
        author_ledger_prefix_count(&fixture),
        prefix_count_after_first_start,
        "漂移 ⇒ 零额外调用（前缀条目零增加）"
    );
}
// Task 7（tasks 4.1/8.2，REQ-WSC-28A §1.7 验收行）：一次 C 教学后四数值
// **同时**断言——A 池 `repairs_used` 增量 0、B 池 `manual_repairs_used`
// 增量 0、窗口 attempts==2（C 已消费 1）、前缀条目合计 2（实际启动合计
// 2）；且不伪造用户 turn/hold/receipt（human gate / 结构预算聚合记录
// 不变）。以 IR 类失败承载（parse 类的 wiring 面已由
// parse_failure_consumes_window_and_reruns_once 钉住）。
#[tokio::test]
async fn one_teaching_keeps_ab_pools_zero_c_consumed_one_starts_two() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(SequenceOutputProvider {
        outputs: vec![
            markdown_with_unknown_requirement_ref(&fixture.story_id, &fixture.design_id),
            single_candidate_markdown(&fixture.story_id, &fixture.design_id),
        ],
        inputs: input_tx,
        next: AtomicUsize::new(0),
    });
    let budget_before = run_history_budget(&fixture);
    assert_eq!(budget_before, (0, 0), "前提：fresh 会话两池已用计数为 0");
    let (context, _outbound_rx) = single_candidate_context(&fixture, provider);
    start_generation(context).await;

    let _first_input = next_provider_input(&mut input_rx).await;
    let reredrive_input = next_provider_input(&mut input_rx).await;
    assert!(
        reredrive_input.prompt.contains("unknown_requirement_ref"),
        "第二轮必须是 IR 教学重驱: {}",
        reredrive_input.prompt
    );
    no_more_provider_inputs(&mut input_rx).await;
    super::single_candidate_provider_run::wait_for_stage(
        &fixture.engine,
        WorkspaceStage::HumanConfirm,
    )
    .await;
    let durable = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload single-candidate session");
    assert_eq!(
        durable.single_candidate_phase,
        Some(SingleCandidatePhase::Approval),
        "重驱交付成功后回到既有确认链"
    );
    // —— 四数值（各自独立断言，任何一个漂移都单独失败）——
    let budget_after = run_history_budget(&fixture);
    assert_eq!(
        budget_after.0, budget_before.0,
        "① A 池 repairs_used 增量必须为 0（C 不扣评审机械池）"
    );
    assert_eq!(
        budget_after.1, budget_before.1,
        "② B 池 manual_repairs_used 增量必须为 0（C 不扣 author 重写/反馈池）"
    );
    let window = last_teaching_window(&fixture);
    assert_eq!(
        window.attempts.len(),
        2,
        "③ 窗口 attempts==2（原调用 + C 已消费 1 次教学机会）"
    );
    assert_eq!(
        author_ledger_prefix_count(&fixture),
        2,
        "④ 前缀条目合计 2（实际物理启动合计 2，C 只消费窗口内的一次）"
    );
    // —— 不伪造用户 turn/hold/receipt ——
    // SC 链的确认门快照由既有 ensure_human_gate_snapshot_after_compile_failure
    // 等门面合法物化（教学与否都会出现）；「用户 turn」的观测点是快照内的
    // turn 计数器（accepted_feedback_turns / attempts_used）与结构预算聚合。
    let turns = user_turn_surfaces(&fixture);
    assert!(
        matches!(turns.accepted_feedback_turns, None | Some(0)),
        "C 教学不得接受任何反馈 turn: {:?}",
        turns.accepted_feedback_turns
    );
    assert_eq!(turns.attempts_used, 0, "C 教学不得消耗任何修复轮次");
    assert_eq!(
        (turns.active_holds, turns.user_receipts),
        (0, 0),
        "C 教学不得伪造结构预算 hold/receipt"
    );
    assert!(
        !turns.human_gate_reservation_present,
        "C 教学不得伪造 human gate 预留"
    );
}

/// Task 7（tasks 4.2，REQ-WSC-28A）：A/B 池余额充足也不得为 C 的第三次
/// 调用兜底——C 的上限只由窗口 attempt 上限表达。两轮 parse 失败后链停止：
/// 第三次启动仍为 0，且无 `repair_reservation` 预领（不旁路 A 通道）。
#[tokio::test]
async fn ab_balance_does_not_back_third_start() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    // 前提钉死：两池余额充足（fresh 记录 used=0/0 ⇒ A 余额 = max_repairs、
    // B 余额 = max_manual_repairs，均 > 0）——若实现把第三次启动落到 A/B
    // 池兜底，余额并不构成阻碍。
    let budgets = crate::product::work_item_plan_policy::RunBudgets::default();
    let (repairs_used_before, manual_used_before) = run_history_budget(&fixture);
    assert_eq!(
        (repairs_used_before, manual_used_before),
        (0, 0),
        "前提：fresh 记录两池已用计数为 0"
    );
    assert!(
        budgets.max_repairs > repairs_used_before,
        "前提：A 池余额充足（max_repairs={}）",
        budgets.max_repairs
    );
    assert!(
        budgets.max_manual_repairs > manual_used_before,
        "前提：B 池余额充足（max_manual_repairs={}）",
        budgets.max_manual_repairs
    );
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(SequenceOutputProvider {
        outputs: vec![
            markdown_missing_section(&fixture.story_id, &fixture.design_id, "### Goal"),
            markdown_missing_section(&fixture.story_id, &fixture.design_id, "### Tasks"),
            single_candidate_markdown(&fixture.story_id, &fixture.design_id),
        ],
        inputs: input_tx,
        next: AtomicUsize::new(0),
    });
    let (context, mut outbound_rx) = single_candidate_context(&fixture, provider);
    start_generation(context).await;

    let _first_input = next_provider_input(&mut input_rx).await;
    let reredrive_input = next_provider_input(&mut input_rx).await;
    assert!(
        reredrive_input.prompt.contains("missing_section"),
        "第二轮必须是 parse 教学重驱: {}",
        reredrive_input.prompt
    );
    // A/B 余额充足 ⇒ 第三次启动仍为 0（无新输入）。
    no_more_provider_inputs(&mut input_rx).await;
    let message = next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("compile markdown source failed")
            && message.contains("after one teaching re-drive"),
        "第二次交付失败必须按既有失败面收口: {message}"
    );
    wait_for_single_candidate_phase(&fixture, SingleCandidatePhase::Failed).await;
    let window = last_teaching_window(&fixture);
    assert_eq!(
        window.attempts.len(),
        2,
        "C 的上限只由窗口表达：两 attempt 后耗尽"
    );
    assert!(
        matches!(
            window.status,
            ScTeachingWindowStatus::Closed {
                reason: ScTeachingWindowCloseReason::Exhausted
            }
        ),
        "窗口耗尽关窗（A/B 余额不续期）: {:?}",
        window.status
    );
    assert_eq!(
        author_ledger_prefix_count(&fixture),
        2,
        "A/B 余额充足 ⇒ 第三次启动仍为 0（前缀条目合计 2）"
    );
    assert!(
        repair_reservation_absent(&fixture),
        "C 耗尽不得落到 A 通道 repair_reservation 预领"
    );
    assert_eq!(
        run_history_budget(&fixture),
        (repairs_used_before, manual_used_before),
        "两池计数不被 C 的停止面扣减"
    );
}

/// Task 7（tasks 4.3/4.4，REQ-WSC-28E）：第二次失败后的停止面——无第三次
/// 启动；`work_item_plan_source_revision_ref`/`plan_candidate_ir_ref` 保持
/// 最后合法值（fresh 链无合法 candidate ⇒ 保持 None，不提升坏 candidate）；
/// durable 无新增 candidate attempt 行、无新 ledger 条目、无平台自造
/// turn/candidate/generation 记录。
#[tokio::test]
async fn stop_after_second_failure_keeps_refs_and_no_auto_new_turn() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let attempts_before = fixture
        .lifecycle
        .list_candidate_attempts(&fixture.record.id)
        .expect("fresh candidate attempt log")
        .len();
    assert_eq!(
        attempts_before, 0,
        "基线（绝对）：fresh 会话的尝试流水为 0 行"
    );
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(SequenceOutputProvider {
        outputs: vec![
            markdown_missing_section(&fixture.story_id, &fixture.design_id, "### Goal"),
            markdown_with_dependency_self_cycle(&fixture.story_id, &fixture.design_id),
            single_candidate_markdown(&fixture.story_id, &fixture.design_id),
        ],
        inputs: input_tx,
        next: AtomicUsize::new(0),
    });
    let (context, mut outbound_rx) = single_candidate_context(&fixture, provider);
    start_generation(context).await;

    let _first_input = next_provider_input(&mut input_rx).await;
    let parse_reredrive_input = next_provider_input(&mut input_rx).await;
    assert!(
        parse_reredrive_input.prompt.contains("missing_section"),
        "第二轮必须是 parse 教学重驱: {}",
        parse_reredrive_input.prompt
    );
    // 无第三次启动：重驱轮之后无新输入。
    no_more_provider_inputs(&mut input_rx).await;
    let message = next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("candidate structure gate rejected")
            && message.contains("first round")
            && message.contains("missing_section")
            && message.contains("请显式重新开始生成"),
        "第二次失败必须按既有失败面回显两轮真实原因: {message}"
    );
    wait_for_single_candidate_phase(&fixture, SingleCandidatePhase::Failed).await;
    let durable = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload single-candidate session");
    // refs 保持最后合法值：fresh 链从未出现合法 candidate ⇒ 两 refs 保持
    // None（停止面不提升坏 candidate、也不清空/改写最后合法值）。
    assert!(
        durable.work_item_plan_source_revision_ref.is_none()
            && durable.plan_candidate_ir_ref.is_none(),
        "refs 保持最后合法值（未提升坏 candidate）"
    );
    // 无新 ledger 条目：恰两条（原调用 + 教学 claim 原位补全），无第三条。
    assert_eq!(
        author_ledger_prefix_count(&fixture),
        2,
        "无新 ledger 条目：前缀恰两条物理启动的账，无第三条"
    );
    // 无新增 candidate attempt 行（REQ-WSC-25 现状口径：流水行只由 engine
    // 候选编译边界写入——结构拒收轮 1 行；parse/IR 类失败在 handler 侧
    // typed 判定、不产生流水行，Task 8 阶段细化落地后须复测本值）：
    // 两次物理交付恰写 1 行，无平台为「补机会」自造的额外行。
    let attempts_after = fixture
        .lifecycle
        .list_candidate_attempts(&fixture.record.id)
        .expect("candidate attempt log after stop")
        .len();
    assert_eq!(
        attempts_after, 1,
        "无新增 candidate attempt 行：恰结构拒收轮的 1 行（无第三行）"
    );
    // 无平台自造 turn/hold/receipt 记录（观测点同用例 1：快照 turn 计数器
    // + 结构预算聚合；停止面若物化确认门快照，其计数必须为零）。
    let turns = user_turn_surfaces(&fixture);
    assert!(
        matches!(turns.accepted_feedback_turns, None | Some(0)),
        "停止后不得自造反馈 turn: {:?}",
        turns.accepted_feedback_turns
    );
    assert_eq!(turns.attempts_used, 0, "停止后不得自造修复轮次");
    assert_eq!(
        (turns.active_holds, turns.user_receipts),
        (0, 0),
        "停止后不得自造结构预算 hold/receipt"
    );
    assert!(
        !turns.human_gate_reservation_present,
        "停止后不得自造 human gate 预留"
    );
    assert!(
        durable.review_invocation_scope.is_none(),
        "停止面不发起 review/confirm"
    );
    assert!(
        repair_reservation_absent(&fixture),
        "停止面不旁路 A 通道（无 repair_reservation 预领）"
    );
    assert_eq!(
        teaching_window_count(&fixture),
        1,
        "无平台自造 generation 记录：不为「补机会」开第二窗口"
    );
    let window = last_teaching_window(&fixture);
    assert_eq!(
        window.attempts.len(),
        2,
        "第二次交付失败后窗口以两 attempt 耗尽收口"
    );
    assert!(
        matches!(
            window.status,
            ScTeachingWindowStatus::Closed {
                reason: ScTeachingWindowCloseReason::Exhausted
            }
        ),
        "close(FailedTerminal) 关窗: {:?}",
        window.status
    );
}
