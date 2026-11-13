//! P27b：SC 教学来源与预算的**真实 capture** 矩阵（Plan §7：S16 / S19 / S22 / S23）。
//!
//! 本文件只做两件**最窄**的事，全部经真实生产路径：
//!
//! 1. `sc_teaching_integration_compile_source_block_bytes_equal_failed_source`
//!    ——真实 provider 输入队列里抓到的**教学重驱 prompt**，其失败源码围栏块的
//!    **原始字节**必须逐字等于本轮 durable 失败源引用（`rounds.jsonl` 的
//!    `failure_observed.source` 的 bytes/sha256）。
//!    提取只依据**结构**（durable 值锚点 + 动态围栏 + 字节数/摘要复算），
//!    **不 pin 任何人类可读措辞**、不按固定句子拼、不 trim 末尾 LF。
//! 2. `sc_teaching_integration_over_budget_prompt_never_starts_extra_provider`
//!    ——失败源大到把最终 prompt 顶出硬预算（`WORK_ITEM_PLAN_MARKDOWN_PROMPT_MAX_BYTES`）
//!    时：最终全文**逐字留存**供审计（disposition=`rejected_budget`，正文文件字节
//!    数 == 最终全文字节数 > 上限），且**零额外 provider 启动、零教学 claim**。
//!
//! 零教学 claim 的判据（窗口模型语义，before/after 口径）：窗口建立时 `attempts`
//! 只固化**原调用占位**（`attempt_no == 1`）；**教学 claim 会追加一条
//! `attempt_no == 2`** ⇒ 零 claim = durable 窗口里**没有任何 `attempt_no >= 2`**
//! 且快照相对运行前基线**零增量**（不用「看起来像 1」的断言，也不用无限轮询：
//! 预算被拒时窗口可能根本不存在）。
//!
//! 计数口径：物理启动数只认 adapter `start` 回调真实入队的条数 + durable
//! `provider_start_ledger` 前缀条目数（**不按路由长度、文本长度或阶段名推断**）。
//!
//! 落位：`src/web/workspace_ws_handler/tests/sc_teaching_source.rs`
//! 登记：`src/web/workspace_ws_handler/tests.rs`（与既有 `mod` 登记同区）

use super::sc_failure_driver_fixture::{
    author_start_count, evidence_entries, extract_source_body, failure_message,
    markdown_missing_five_sections, only_chain_id, sha256_hex, start_run, teaching_attempt_numbers,
    window_snapshot,
};
use super::single_candidate_provider_run::{ProviderRunFixture, wait_for_single_candidate_phase};
use super::*;
use crate::product::models::SingleCandidatePhase;
use crate::product::work_item_split_engine::prompts::WORK_ITEM_PLAN_MARKDOWN_PROMPT_MAX_BYTES;

/// 预算上限只有这一个来源（矩阵不得另立第二数值）。
const MAX_BYTES: usize = WORK_ITEM_PLAN_MARKDOWN_PROMPT_MAX_BYTES;

/// ① 教学重驱 prompt 的失败源正文必须逐字等于 durable 失败源引用。
///
/// 真实 capture：教学重驱 prompt 是 adapter `start` 回调**真实入队**的第二条输入。
#[tokio::test]
async fn sc_teaching_integration_compile_source_block_bytes_equal_failed_source() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let story_id = fixture.story_id.clone();
    let design_id = fixture.design_id.clone();
    // 中文前言：归一化面会裁掉文档标题之前的前言 ⇒ raw ≠ source（摘要不同）。
    let output = format!(
        "好的，这是本次的实现计划：\n\n{}",
        markdown_missing_five_sections(&story_id, &design_id, "前端渲染不在本次范围内。")
    );
    let mut run = start_run(fixture, vec![output]).await;

    // 真实输入队列：原轮 + 恰一次教学重驱。
    let _original_input = run.queue.next_input().await;
    let teaching_input = run.queue.next_input().await;
    let message = failure_message(&mut run.outbound).await;
    assert!(
        message.contains("missing_section"),
        "真实失败面必须是可教学的 missing_section: {message}"
    );
    wait_for_single_candidate_phase(&run.fixture, SingleCandidatePhase::Failed).await;

    // durable 事实：失败轮的 raw / source 引用（生产写入）。
    //
    // 真实重驱语义：原轮失败 ⇒ 恰一次教学重驱**又把同一失败面喂回去**，教学轮
    // 也真实失败 ⇒ durable 里恰有**两轮**机械失败事实（原轮 + 这一轮教学；规格
    // 允许上限恰为一次教学重驱），此后进入终局。这不是缺陷，因此断言计数 2。
    let chain_id = only_chain_id(&run.fixture);
    let entries = evidence_entries(&run.fixture, &chain_id);
    let observed: Vec<&serde_json::Value> = entries
        .iter()
        .filter(|entry| entry["event"]["kind"] == "failure_observed")
        .collect();
    assert_eq!(
        observed.len(),
        2,
        "机械失败事实应为两轮：原轮 + 恰一次教学重驱轮（教学重驱复发了同一个真实失败面）"
    );
    let mut observed_round_ids: Vec<&str> = observed
        .iter()
        .map(|entry| entry["event"]["round_id"].as_str().unwrap_or_default())
        .collect();
    observed_round_ids.sort_unstable();
    observed_round_ids.dedup();
    assert_eq!(
        observed_round_ids.len(),
        2,
        "两轮失败事实必须来自两个不同的轮（原轮 / 教学轮），实测 {observed_round_ids:?}"
    );
    // 教学 prompt 绑定的是**产生该 prompt 的那一轮** ⇒ 第一条失败事实（原轮）。
    let failure = &observed[0]["event"];
    let raw = &failure["raw"];
    let source = &failure["source"];
    assert_eq!(
        raw["status"], "available",
        "provider 原文必须已归档（可读回）"
    );
    assert_eq!(source["status"], "available", "失败源必须已归档（可读回）");
    let source_sha256 = source["sha256"].as_str().expect("durable source sha256");
    let source_bytes = source["bytes"].as_u64().expect("durable source bytes");
    assert_ne!(
        raw["sha256"], source["sha256"],
        "中文前言被裁掉 ⇒ raw 与归一化失败源的摘要必须不同"
    );

    // 真实 capture 的教学 prompt：结构化提取失败源正文并逐字核对。
    // 锚点只用**值**（durable 摘要/locator），不 pin 任何人类可读措辞。
    assert!(
        teaching_input.prompt.contains(source_sha256),
        "教学 prompt 必须携带 durable 失败源摘要（值锚点）"
    );
    assert!(
        teaching_input
            .prompt
            .contains(source["locator"].as_str().unwrap_or_default()),
        "教学 prompt 必须携带 durable 失败源 locator（值锚点）"
    );
    let body = extract_source_body(&teaching_input.prompt, source_sha256, source_bytes);
    assert_eq!(
        body.len() as u64,
        source_bytes,
        "捕获到的失败源字节数必须逐字等于 durable 失败源字节数"
    );
    assert_eq!(
        sha256_hex(body.as_bytes()),
        source_sha256,
        "捕获到的失败源摘要必须逐字等于 durable 失败源摘要"
    );
    assert!(
        !body.contains("好的，这是本次的实现计划"),
        "失败源必须是归一化后的正文（不含 provider 前言）"
    );

    // 计数口径：实际启动 = 原轮 + 恰一次教学重驱（队列 + durable 账本两处事实）。
    run.queue.expect_no_further_input().await;
    assert_eq!(
        run.queue.received_count(),
        2,
        "实际 provider 启动回调恰两条：原轮 + 恰一次教学重驱（没有第三次启动）"
    );
    assert_eq!(
        author_start_count(&run.fixture),
        2,
        "durable 物理启动账本：原轮 + 恰一次教学重驱"
    );
}

/// ② 最终 prompt 顶出硬预算 ⇒ 全文逐字留存供审计，且零额外启动、零教学 claim。
///
/// 真实 capture：provider 只真实启动一次（原轮）；教学轮**不得**启动。
#[tokio::test]
async fn sc_teaching_integration_over_budget_prompt_never_starts_extra_provider() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let story_id = fixture.story_id.clone();
    let design_id = fixture.design_id.clone();
    // 超预算失败源：中文填充让归一化失败源远超硬上限。
    let filler = "填充中文内容。".repeat(20_000);
    let output = format!(
        "好的，这是本次的实现计划：\n\n{}",
        markdown_missing_five_sections(&story_id, &design_id, &filler)
    );
    // 运行前 durable 基线：还没有教学窗口、没有任何教学 attempt。
    assert!(
        window_snapshot(&fixture).is_none(),
        "运行前 durable 基线：教学窗口尚未建立"
    );
    assert!(
        teaching_attempt_numbers(&fixture).is_empty(),
        "运行前 durable 基线：没有任何教学 attempt"
    );
    let mut run = start_run(fixture, vec![output]).await;

    // 真实输入队列：只有原轮。
    let _original_input = run.queue.next_input().await;
    let message = failure_message(&mut run.outbound).await;
    assert!(
        message.contains("missing_section"),
        "真实失败面必须是可教学的 missing_section: {message}"
    );
    wait_for_single_candidate_phase(&run.fixture, SingleCandidatePhase::Failed).await;
    run.queue.expect_no_further_input().await;

    // 零额外物理启动（队列事实 + durable 账本事实）。
    assert_eq!(
        author_start_count(&run.fixture),
        1,
        "超预算不得启动教学重驱（durable 启动账本只有原轮）"
    );

    // 零教学 claim：教学 claim 会追加一条 `attempt_no == 2` ⇒ durable 窗口里
    // 不得出现任何 `attempt_no >= 2`（窗口也可能因预算被拒而根本不存在）。
    let attempt_numbers = teaching_attempt_numbers(&run.fixture);
    assert!(
        attempt_numbers.iter().all(|attempt_no| *attempt_no == 1),
        "零教学 claim：durable 窗口里不得出现 attempt_no>=2 的教学 attempt（实测 {attempt_numbers:?}）"
    );
    if let Some(after) = window_snapshot(&run.fixture) {
        assert!(
            after.claimable_attempt_no.is_none() || after.attempts.len() == 1,
            "窗口若存在 ⇒ 未消费教学 attempt（claimable={:?}，attempts={:?}）",
            after.claimable_attempt_no,
            after.attempts
        );
    }

    // 全文逐字留存供审计：durable 提示条目 disposition=rejected_budget，
    // 正文文件字节数 == 最终全文字节数（> 硬上限）。
    let chain_id = only_chain_id(&run.fixture);
    let entries = evidence_entries(&run.fixture, &chain_id);
    let prompts: Vec<&serde_json::Value> = entries
        .iter()
        .filter(|entry| {
            entry["event"]["kind"] == "prompt_prepared"
                || entry["event"]["kind"] == "prompt_unavailable"
        })
        .collect();
    assert_eq!(
        prompts.len(),
        1,
        "必须留一条 durable 教学提示审计（超预算也要可审计）"
    );
    let evidence = &prompts[0]["event"]["evidence"];
    assert_eq!(
        evidence["disposition"], "rejected_budget",
        "超预算的处置必须是 rejected_budget（不是 unavailable，也不是 sent）"
    );
    let prompt_ref = &evidence["prompt"];
    assert_eq!(
        prompt_ref["status"], "available",
        "超预算也要留下可读回的全文引用（不得丢正文）"
    );
    let locator = prompt_ref["locator"].as_str().expect("prompt locator");
    let digest = locator.rsplit('/').next().expect("prompt digest");
    let ordinal = evidence["target_round_ordinal"]
        .as_u64()
        .expect("target ordinal");
    let body_path = super::sc_failure_driver_fixture::evidence_root(&run.fixture)
        .join(&chain_id)
        .join("prompts")
        .join(format!("{ordinal}-{digest}.txt"));
    let body = std::fs::read_to_string(&body_path).expect("read audited prompt body");
    assert_eq!(
        body.len() as u64,
        prompt_ref["bytes"].as_u64().expect("prompt bytes"),
        "审计正文必须逐字等于最终全文（字节数一致）"
    );
    assert_eq!(
        sha256_hex(body.as_bytes()),
        prompt_ref["sha256"].as_str().unwrap_or_default(),
        "审计正文摘要必须逐字等于最终全文摘要"
    );
    assert!(
        body.len() > MAX_BYTES,
        "本用例的最终全文必须真的超出硬上限（实测 {} B，上限 {MAX_BYTES} B）",
        body.len()
    );
}
