//! 作者阶段候选结构门拒收的「恰一次教学重驱」端到端用例（SG3a.1 +
//! REQ-WSC-18/25）。与 compile/IR 重驱共享 runner 的唯一槽位
//! （first_round_failure 单槽）——从 single_candidate_provider_run.rs 拆出
//! 以守住 large_file_guard 1200 行红线；共享 fixture 经 pub(super) 复用。
//!
//! 钉住的行为：
//! 1. 槽位空闲的结构拒收：作者确实被驱动第二次（恰好两次 provider 输入），
//!    重驱 prompt 是可执行修复指令；重驱成功 → 正常提升 current refs、进入
//!    既有确认链（相位 Approval，不是 failed）；
//! 2. 重驱后仍被拒：落人工门（WaitingForHuman + 门快照），会话不是 failed，
//!    provider 恰好驱动两次（不再第三次）；
//! 3. 每次尝试都进 candidate-attempts.jsonl；durable auto_rewrite 不留
//!    running 记录、auto_rewrites_used 不变。

use super::single_candidate_provider_run::{
    ProviderRunFixture, SequenceOutputProvider, next_provider_input, no_more_provider_inputs,
    single_candidate_context, single_candidate_markdown, wait_for_stage,
};
use super::*;
use std::sync::atomic::AtomicUsize;

/// WI-001 的 canonical Inputs 消费**自己**输出的 contract.backend-api——
/// provider 在批内且能力声明自洽（IR 校验通过），依赖图成 WI-001→WI-001
/// 自环，只在候选结构门暴露（dependency_cycle，witness 不完整族）。
fn markdown_with_self_cycle(story_id: &str, design_id: &str) -> String {
    single_candidate_markdown(story_id, design_id).replacen(
        "### Inputs\n\n### Outputs",
        "### Inputs\n- contract_id: contract.backend-internal\n- provider_logical_work_item_id: WI-001\n- required_capabilities: api.backend.read\n- compatibility_policy: require_all\n\n### Outputs",
        1,
    )
}

fn attempt_rows(
    fixture: &ProviderRunFixture,
) -> Vec<crate::product::lifecycle_store::candidate_attempts::CandidateAttemptRecord> {
    fixture
        .lifecycle
        .list_candidate_attempts(&fixture.record.id)
        .expect("read candidate attempt log")
}

/// 等待 durable 会话进入「WaitingForHuman 且门快照在场」的落门形态。
async fn wait_for_human_gate(fixture: &ProviderRunFixture) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let record = fixture
                .lifecycle
                .get_workspace_session(&fixture.record.id)
                .expect("reload session");
            if record.status == crate::product::models::WorkspaceSessionStatus::WaitingForHuman
                && record.human_gate_snapshot.is_some()
            {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("structure rejection rerun failure must land on the human gate");
}

/// ① 槽位空闲的结构拒收 → 恰一次教学重驱 → 重驱成功恢复正常链。
#[tokio::test]
async fn single_candidate_structure_rejection_triggers_one_teaching_reredrive_and_recovers() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(SequenceOutputProvider {
        outputs: vec![
            markdown_with_self_cycle(&fixture.story_id, &fixture.design_id),
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
            resume_recovery: None,
        },
    )
    .await;

    // 作者确实被驱动第二次：恰好两次 provider 输入（首轮 + 重驱）。
    let first_input = next_provider_input(&mut input_rx).await;
    assert!(
        first_input.prompt.contains("[markdown_grammar]"),
        "first round must stay the full markdown author prompt"
    );
    let reredrive_input = next_provider_input(&mut input_rx).await;
    for required in [
        "候选没有通过候选结构门",
        "final_compile_structure_rejected",
        "依赖环",
        "消环要点",
        "不得删除真实业务数据流",
        "不得改动仓序本身",
        "必须输出完整修订版 markdown",
        "不得输出 diff、patch",
    ] {
        assert!(
            reredrive_input.prompt.contains(required),
            "结构拒收教学重驱 prompt 必须包含 {required}: {}",
            reredrive_input.prompt
        );
    }
    no_more_provider_inputs(&mut input_rx).await;

    // 重驱成功：进入既有确认链（HumanConfirm 相位 = Approval），不是 failed。
    wait_for_stage(&fixture.engine, WorkspaceStage::HumanConfirm).await;
    let durable = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload single-candidate session");
    assert_eq!(
        durable.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Approval),
        "重驱恢复后必须回到既有审批链"
    );
    assert!(
        durable.work_item_plan_source_revision_ref.is_some()
            && durable.plan_candidate_ir_ref.is_some(),
        "重驱后的合法候选必须正常提升 current refs（零提升只对被拒候选）"
    );

    // 每次尝试都进流水：首轮拒收恰好一行失败（重驱成功行由终编译追加）。
    let attempts = attempt_rows(&fixture);
    assert_eq!(attempts.len(), 1, "首轮结构拒收必须落一行尝试流水");
    assert!(matches!(
        attempts[0].outcome,
        crate::product::lifecycle_store::candidate_attempts::CandidateAttemptOutcome::Failed
    ));
    assert_eq!(
        attempts[0].failure.as_ref().expect("failure facts").code,
        "final_compile_structure_rejected"
    );

    // 不虚记自动重写预算：durable 不留 running，auto_rewrites_used 不变。
    let status = fixture
        .lifecycle
        .load_candidate_compile_status(&fixture.record.id)
        .expect("load candidate compile status")
        .expect("durable candidate compile status must exist");
    assert!(
        status.auto_rewrite.is_none(),
        "作者阶段重驱不得留下 auto_rewrite running 记录"
    );
    assert_eq!(
        status.auto_rewrites_used, 0,
        "作者阶段重驱不消耗编译阶段自动重写预算"
    );
}

/// ② 重驱后仍被拒 → 落人工门（不是 failed），provider 恰好驱动两次，
///    两次拒收在流水中留下两行失败。
#[tokio::test]
async fn single_candidate_structure_rejection_reredrive_failure_lands_human_gate() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(SequenceOutputProvider {
        outputs: vec![
            markdown_with_self_cycle(&fixture.story_id, &fixture.design_id),
            // 内容寻址幂等键：同源候选重放＝同一次尝试（零膨胀），真实重驱
            // 轮的 source 必然不同——以改标题体现「第二次真实尝试」。
            markdown_with_self_cycle(&fixture.story_id, &fixture.design_id)
                .replace("Backend API", "Backend API round-2"),
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
            resume_recovery: None,
        },
    )
    .await;

    let _first_input = next_provider_input(&mut input_rx).await;
    let _reredrive_input = next_provider_input(&mut input_rx).await;
    no_more_provider_inputs(&mut input_rx).await;

    // 落人工门：WaitingForHuman + 门快照在场，会话不是 failed。
    wait_for_human_gate(&fixture).await;
    let durable = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload single-candidate session");
    assert_ne!(
        durable.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Failed),
        "结构拒收重驱再败必须落门，不得终态失败"
    );
    assert_eq!(
        durable.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Generate),
        "相位保持 Generate（owner 决定下一步）"
    );
    assert!(
        durable.work_item_plan_source_revision_ref.is_none()
            && durable.plan_candidate_ir_ref.is_none(),
        "两次被拒候选都零提升（current refs 语义不变）"
    );

    // 两次拒收 = 两行失败（每次尝试都进流水）。
    let attempts = attempt_rows(&fixture);
    assert_eq!(
        attempts.len(),
        2,
        "两次结构拒收必须各落一行尝试流水：{attempts:?}"
    );
    assert!(attempts.iter().all(|record| {
        record
            .failure
            .as_ref()
            .is_some_and(|failure| failure.code == "final_compile_structure_rejected")
    }));

    // 门摘要：owner 读到「候选没有通过候选结构门」的人话原因与下一步。
    let engine = fixture.engine.lock().await;
    let gate_summary = engine
        .timeline_nodes
        .iter()
        .rev()
        .find(|node| {
            matches!(
                node.node_type,
                crate::web::workspace_ws_types::TimelineNodeType::HumanConfirm
            )
        })
        .and_then(|node| node.summary.clone())
        .expect("人工门节点必须带摘要");
    drop(engine);
    assert!(
        gate_summary.contains("候选没有通过候选结构门"),
        "门摘要必须说清结构门拒收：{gate_summary}"
    );

    // 预算同样不虚记。
    let status = fixture
        .lifecycle
        .load_candidate_compile_status(&fixture.record.id)
        .expect("load candidate compile status")
        .expect("durable candidate compile status must exist");
    assert!(status.auto_rewrite.is_none());
    assert_eq!(status.auto_rewrites_used, 0);
}
