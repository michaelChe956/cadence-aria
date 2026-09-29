// C2 Task 11（REQ-CG-03，#15）：SC 人工返修完整预算与完整有序传输。
//
// 固定 32,000B 拒绝改为完整预算口径：扣回合前以 UTF-8 字节计算完整组装输入
// （候选全文＋固定合同＋feedback＋上下文）与所选 provider 静态预算比较——
// 不超 inline 整体内联；超 inline 但硬限内先持久化组装 artifact（readback
// digest 校验）经引用传输完整原文并记录组装 digest；artifact 不可读或完整
// 输入超硬限在 turn CAS 之前拒绝停等（门状态、manual_repairs_remaining 与
// provider 启动计数不变），落大候选停等等待事实（Task 12 投影
// large_candidate_blocked），用户点击"分段返修／重试"后才开新回合。

use crate::product::models::ProviderName;
use crate::product::workspace_engine::conversational_gate::{
    render_sc_revision_delivery, sc_provider_input_budget,
};
use crate::product::workspace_engine::prompts::assemble_sc_revision_input;
use crate::web::workspace_ws_types::ArtifactPayload;

/// 大候选 fixture：durable_revision_fixture 基础上把 author provider 切到
/// `provider`，候选 markdown 撑到 `candidate_bytes` 量级（含 28,695B 及更大
/// 形态；现测试仅 "候选".repeat(32_000)）。
fn large_candidate_revision_fixture(
    session_id: &str,
    provider: ProviderName,
    candidate_bytes: usize,
) -> (
    tempfile::TempDir,
    crate::product::lifecycle_store::LifecycleStore,
    crate::product::workspace_engine::WorkspaceEngine,
    String,
) {
    let (tmp, lifecycle, mut engine) = durable_revision_fixture(session_id, 3);
    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("durable session");
    record.author_provider = provider.clone();
    crate::product::json_store::write_json(
        &lifecycle
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist author provider");
    engine.session.author_provider = provider;

    let mut markdown =
        String::from("# Work Item Plan\n\n## Work Item WI-001: 大候选\n\n### Outputs\n");
    let line = "- 能力细则行，候选全文每行都必须完整保留，不得截断或摘要替代。\n";
    while markdown.len() < candidate_bytes {
        markdown.push_str(line);
    }
    engine.session.artifact = Some(ArtifactPayload::Markdown {
        markdown: markdown.clone(),
        diff: None,
    });
    (tmp, lifecycle, engine, markdown)
}

fn sc_revision_partition_path(
    lifecycle: &crate::product::lifecycle_store::LifecycleStore,
    engine: &crate::product::workspace_engine::WorkspaceEngine,
    file: &str,
) -> std::path::PathBuf {
    lifecycle
        .app_paths()
        .issue_root(&engine.session().project_id, &engine.session().issue_id)
        .join("workspace-sessions")
        .join(&engine.session().session_id)
        .join(file)
}

#[tokio::test]
async fn large_candidate_transports_full_text_within_hard_limit() {
    // 28,695B 样本之上再加大：~40KB 候选（超 32,000B inline、远小于
    // ClaudeCode 300,000B 硬限）。
    let (_tmp, lifecycle, mut engine, markdown) =
        large_candidate_revision_fixture("session_sc_large", ProviderName::ClaudeCode, 40_000);

    let outcome = engine
        .handle_human_gate_feedback(HumanGateFeedbackInput {
            command_id: "cmd_large_candidate_revision".to_string(),
            feedback: "只修订 WI-001 标题，其余逐字保留".to_string(),
        })
        .await
        .expect("large candidate revision opens");
    let (turn, remaining_budget, prompt) = match outcome {
        HumanGateCommandOutcome::TurnOpened {
            turn,
            remaining_budget,
            prompt,
        } => (turn, remaining_budget, prompt),
        other => panic!("expected opened turn, got {other:?}"),
    };
    // 候选全文完整进入交付 prompt（不截断、不以摘要替代）。
    assert!(
        prompt.contains(&markdown),
        "full candidate text must reach the provider without truncation"
    );
    assert!(prompt.len() > SC_MANUAL_REVISION_PROMPT_QUALITY_BUDGET_BYTES);
    // 正常开回合：预算扣 1、门状态保持等待面。
    assert_eq!(remaining_budget, 2);
    let saved = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("session after open");
    assert_eq!(
        saved
            .human_gate_snapshot
            .expect("gate snapshot")
            .manual_repairs_remaining,
        2
    );
    let _ = turn;

    // 组装 digest 记录 + 组装 artifact：readback 与全文一致。
    let assembly_path =
        sc_revision_partition_path(&lifecycle, &engine, "sc-revision-assembly.json");
    let assembly: serde_json::Value =
        crate::product::json_store::read_json(&assembly_path).expect("durable assembly record");
    use sha2::Digest as _;
    assert_eq!(assembly["total_bytes"].as_u64(), Some(prompt.len() as u64));
    let digest = assembly["transport"]["assembly_digest"]
        .as_str()
        .expect("assembly digest recorded")
        .to_string();
    let expected_digest = format!("sha256:{:x}", sha2::Sha256::digest(prompt.as_bytes()));
    assert_eq!(digest, expected_digest);
    let artifact_ref = assembly["transport"]["artifact_ref"]
        .as_str()
        .expect("artifact transport")
        .to_string();
    let artifact_path = sc_revision_partition_path(&lifecycle, &engine, &artifact_ref);
    assert_eq!(
        std::fs::read_to_string(&artifact_path).expect("artifact readback"),
        prompt,
        "persisted assembly artifact must equal the full prompt"
    );
    // 无停等等待事实。
    assert!(
        !sc_revision_partition_path(&lifecycle, &engine, "sc-revision-blocked.json").exists()
    );
}

#[tokio::test]
async fn small_candidate_stays_inlined_without_extra_writes() {
    let (_tmp, lifecycle, mut engine) = durable_revision_fixture("session_sc_inline", 3);

    let outcome = engine
        .handle_human_gate_feedback(HumanGateFeedbackInput {
            command_id: "cmd_small_candidate_revision".to_string(),
            feedback: "修订当前候选标题".to_string(),
        })
        .await
        .expect("small candidate revision opens");
    let HumanGateCommandOutcome::TurnOpened { prompt, .. } = outcome else {
        panic!("expected opened turn, got {outcome:?}")
    };
    assert!(prompt.len() <= SC_MANUAL_REVISION_PROMPT_QUALITY_BUDGET_BYTES);
    // inline 交付零新增持久化（assembly/blocked 记录均不落盘）。
    assert!(
        !sc_revision_partition_path(&lifecycle, &engine, "sc-revision-assembly.json").exists(),
        "inlined delivery must not write an assembly record"
    );
    assert!(
        !sc_revision_partition_path(&lifecycle, &engine, "sc-revision-blocked.json").exists()
    );
}

#[tokio::test]
async fn oversized_input_over_hard_limit_rejects_before_turn_cas() {
    // Fake provider 硬限=inline（32,000B，未显式冻结依据的 provider 保持现行为）。
    let (_tmp, lifecycle, mut engine, _markdown) = large_candidate_revision_fixture(
        "session_sc_over_hard",
        ProviderName::Fake,
        40_000,
    );
    let before = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("durable session before");
    let before_bytes = serde_json::to_vec(&before).expect("serialize before");

    let outcome = engine
        .handle_human_gate_feedback(HumanGateFeedbackInput {
            command_id: "cmd_over_hard_limit".to_string(),
            feedback: "修订当前候选标题".to_string(),
        })
        .await
        .expect("over-hard rejection");
    assert!(matches!(
        &outcome,
        HumanGateCommandOutcome::Rejected { code, .. }
            if code == "HUMAN_GATE_REVISION_INPUT_OVER_HARD_LIMIT"
    ));

    // CAS 前拒绝：无 turn、session 字节不变（预算/门状态/provider 启动计数）。
    assert!(
        lifecycle
            .list_human_gate_turns(engine.session().session_id.as_str())
            .expect("list turns")
            .is_empty(),
        "over-hard rejection must not create a turn"
    );
    let after = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("durable session after");
    assert_eq!(
        serde_json::to_vec(&after).expect("serialize after"),
        before_bytes,
        "over-hard rejection must not mutate durable session"
    );
    // 大候选停等等待事实落地（Task 12 投影 large_candidate_blocked）。
    let blocked_path =
        sc_revision_partition_path(&lifecycle, &engine, "sc-revision-blocked.json");
    let blocked: serde_json::Value =
        crate::product::json_store::read_json(&blocked_path).expect("blocked record");
    assert_eq!(
        blocked["reason_code"].as_str(),
        Some("HUMAN_GATE_REVISION_INPUT_OVER_HARD_LIMIT")
    );
    assert!(
        blocked["total_bytes"].as_u64().expect("total bytes") > 32_000,
        "blocked record must carry the full assembled size"
    );
}

#[tokio::test]
async fn unreadable_assembly_artifact_rejects_before_turn_cas() {
    let (_tmp, lifecycle, mut engine, _markdown) =
        large_candidate_revision_fixture("session_sc_unreadable", ProviderName::ClaudeCode, 40_000);
    let partition = sc_revision_partition_path(&lifecycle, &engine, "sc-revision-blocked.json")
        .parent()
        .expect("session partition parent")
        .to_path_buf();
    std::fs::create_dir_all(&partition).expect("create session partition");
    let inputs_dir = partition.join("sc-revision-inputs");
    // 占位同名文件使 artifact 目录不可创建（持久化不可读路径）。
    std::fs::write(&inputs_dir, b"not-a-dir").expect("block artifact dir");

    let outcome = engine
        .handle_human_gate_feedback(HumanGateFeedbackInput {
            command_id: "cmd_unreadable_artifact".to_string(),
            feedback: "修订当前候选标题".to_string(),
        })
        .await
        .expect("unreadable artifact rejection");
    assert!(matches!(
        &outcome,
        HumanGateCommandOutcome::Rejected { code, .. }
            if code == "HUMAN_GATE_REVISION_INPUT_ARTIFACT_UNREADABLE"
    ));
    assert!(
        lifecycle
            .list_human_gate_turns(engine.session().session_id.as_str())
            .expect("list turns")
            .is_empty(),
        "unreadable artifact must not create a turn"
    );
    let blocked_path =
        sc_revision_partition_path(&lifecycle, &engine, "sc-revision-blocked.json");
    let blocked: serde_json::Value =
        crate::product::json_store::read_json(&blocked_path).expect("blocked record");
    assert_eq!(
        blocked["reason_code"].as_str(),
        Some("HUMAN_GATE_REVISION_INPUT_ARTIFACT_UNREADABLE")
    );
    // 预算未被消耗。
    let saved = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("session after reject");
    assert_eq!(
        saved
            .human_gate_snapshot
            .expect("gate snapshot")
            .manual_repairs_remaining,
        3
    );
}

#[test]
fn ordered_chunks_reassemble_to_full_text_within_inline_sized_pieces() {
    // 纯函数：>inline 组装经完整有序分块——每块 ≤inline，帧头带 i/n+digest，
    // 交付形态携带全文（不截断、无缺块）。
    let mut prompt = String::from("# Work Item Plan\n\n");
    while prompt.len() < 70_000 {
        prompt.push_str("候选正文行，分块传输后必须逐字节恢复。\n");
    }
    let budget = sc_provider_input_budget(&ProviderName::ClaudeCode);
    let assembled =
        assemble_sc_revision_input(&prompt, &budget).expect("within hard limit assembles");
    assert_eq!(assembled.total_bytes, prompt.len());
    let delivery = render_sc_revision_delivery(&prompt, &budget);
    assert!(
        delivery.contains("[sc_revision_chunk "),
        "chunk frames must be present for oversized delivery"
    );
    // 完整性反演：去帧按序拼接逐字节恢复全文（不截断、无缺块）。
    assert_eq!(
        crate::product::workspace_engine::prompts::reassemble_sc_revision_delivery(&delivery),
        prompt
    );
}
