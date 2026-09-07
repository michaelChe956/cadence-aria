//! 3.6 弱模型基线加固（组 2）：SC IR 校验失败教学重驱用例。从
//! single_candidate_provider_run.rs 拆出以守住 large_file_guard 1200 行红线；
//! 共享 fixture 经 pub(super) 复用，不复制基建。

use super::single_candidate_provider_run::{
    ProviderRunFixture, SequenceOutputProvider, next_error_message, next_provider_input,
    no_more_provider_inputs, single_candidate_context, single_candidate_markdown,
    wait_for_single_candidate_phase, wait_for_stage,
};
use super::*;
use std::sync::atomic::AtomicUsize;
// 3.6 弱模型基线加固（组 2）：IR 校验失败（unknown_requirement_ref /
// acceptance_criterion_without_reviewer_check 等，即 complete_... 报
// "validate plan candidate IR failed" 的类别）与 missing_section 一样享有
// 恰一次教学重驱：错误原文逐条回灌 + 修正引用/补齐字段指令；重驱成功→正常
// 继续；重驱再败→终态含两轮；warning 级 IR findings 不触发重驱。

/// task 引用 Traceability 未登记（输入 spec 不存在）的 REQ-002——rep1c 现场
/// 错误族；markdown 语法仍合法，失败发生在 IR 校验层。
fn markdown_with_unknown_requirement_ref(story_id: &str, design_id: &str) -> String {
    single_candidate_markdown(story_id, design_id).replacen(
        "- requirement_refs: REQ-001",
        "- requirement_refs: REQ-002",
        1,
    )
}

/// reviewer_check_refs 指向不存在的 AC-002，本 item 的 AC-001 因此失去
/// reviewer check——rep1b 现场错误族（acceptance_criterion_without_reviewer_check）。
fn markdown_with_missing_reviewer_check(story_id: &str, design_id: &str) -> String {
    single_candidate_markdown(story_id, design_id).replacen(
        "- reviewer_check_refs: AC-001",
        "- reviewer_check_refs: AC-002",
        1,
    )
}

/// AC statement 改为过程证据措辞（提交历史/测试/历史），只产生 warning 级
/// process_evidence_acceptance_criterion finding——不触发重驱。
fn markdown_with_process_evidence_warning(story_id: &str, design_id: &str) -> String {
    single_candidate_markdown(story_id, design_id).replacen(
        "- statement: WHEN a request arrives THE SYSTEM SHALL expose the backend API response.",
        "- statement: WHEN 查看测试提交历史 THE SYSTEM SHALL 展示完整提交历史。",
        1,
    )
}

#[tokio::test]
async fn single_candidate_ir_validation_failure_triggers_one_teaching_reredrive_and_recovers() {
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
        "unknown_requirement_ref",
        "references unknown design requirement REQ-002",
        "修正引用/补齐字段",
        "重新输出完整 plan",
    ] {
        assert!(
            reredrive_input.prompt.contains(required),
            "IR 教学重驱 prompt 必须包含 {required}: {}",
            reredrive_input.prompt
        );
    }
    no_more_provider_inputs(&mut input_rx).await;
    wait_for_stage(&fixture.engine, WorkspaceStage::HumanConfirm).await;
    assert_eq!(
        single_candidate_generation_steps_for_session(&fixture.record.id),
        vec!["full_markdown_author", "parse_source_revision", "selector"],
        "IR 重驱恢复后必须恰好走一次完整 compile 落盘路径",
    );
    let durable = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload single-candidate session");
    assert_eq!(
        durable.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Approval),
    );
    let scope = crate::product::work_item_plan_source_store::SourceStoreScope {
        project_id: durable.project_id.clone(),
        issue_id: durable.issue_id.clone(),
        plan_id: durable.entity_id.clone(),
    };
    let source_store = crate::product::work_item_plan_source_store::WorkItemPlanSourceStore::new(
        fixture.app_paths.clone(),
    );
    let stored = source_store
        .get_source_revision(
            &scope,
            durable
                .work_item_plan_source_revision_ref
                .as_deref()
                .expect("source revision ref"),
        )
        .expect("stored source");
    assert_eq!(
        stored.source,
        single_candidate_markdown(&fixture.story_id, &fixture.design_id),
        "重驱修正后的输出必须成为唯一持久化的 source revision"
    );
}

#[tokio::test]
async fn single_candidate_ir_validation_reredrive_failure_is_terminal_with_both_rounds() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(SequenceOutputProvider {
        outputs: vec![
            markdown_with_unknown_requirement_ref(&fixture.story_id, &fixture.design_id),
            markdown_with_missing_reviewer_check(&fixture.story_id, &fixture.design_id),
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
    no_more_provider_inputs(&mut input_rx).await;
    let message = next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("validate plan candidate IR failed"),
        "IR 重驱再败必须保持既有终态错误前缀: {message}"
    );
    assert!(
        message.contains("first round") && message.contains("re-drive round"),
        "终态失败必须含两轮错误: {message}"
    );
    assert!(
        message.contains("unknown_requirement_ref")
            && message.contains("references unknown design requirement REQ-002"),
        "终态必须含首轮错误原文: {message}"
    );
    assert!(
        message.contains("acceptance_criterion_without_reviewer_check")
            && message.contains("AC-001 has no handoff reviewer check"),
        "终态必须含重驱轮错误原文: {message}"
    );
    wait_for_single_candidate_phase(
        &fixture,
        crate::product::models::SingleCandidatePhase::Failed,
    )
    .await;
}

#[tokio::test]
async fn single_candidate_ir_warning_findings_do_not_trigger_reredrive() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(SequenceOutputProvider {
        outputs: vec![markdown_with_process_evidence_warning(
            &fixture.story_id,
            &fixture.design_id,
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
        "warning 级 IR findings（非失败）不得触发重驱，必须正常继续"
    );
}
