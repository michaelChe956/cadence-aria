//! P27a 追加 RED：教学轮「**已物理启动但交付失败**」⇒ 教学 prompt 的处置必须是
//! `sent`，不是 `prepared`。
//!
//! 真实 root cause（P25 F1 复核）：`Sent` 记在 drive 的 `await` 之后 ⇒ 教学轮已
//! 物理启动（输入真实交给 provider）但随后 drive／严格收口失败时，`Sent` 没被
//! 写下，包里留下 `Prepared` ⇒ 读侧会把「已发送」误读成「只准备未发送」。
//!
//! 本用例是**真实驱动**（不是 mock publisher）：
//! - 首轮正常交付既有确定性 markdown ⇒ 制造可教的 compile 失败（真实教学授予）；
//! - 第二轮 start 回调**真实执行**并**真实入队**输入（⇒ 实际排队 2），随后
//!   **不交付任何输出**（事件通道关闭）⇒ drive 真实失败；
//! - 断言面全是 durable 事实：prompt 审计条目的处置、正文哈希/引用、真实教学判定、
//!   真实队列计数、以及生产终局发布的 A4 包（`Incomplete` + 末物理轮输出 Missing +
//!   失败源仍指上一轮）。
//!
//! 落位：`src/web/workspace_ws_handler/tests/sc_failure_prompt_sent.rs`
//! 登记：`src/web/workspace_ws_handler/tests.rs`（与既有 `mod` 登记同区）

use super::sc_failure_driver_fixture::{
    durable_sc_package_versions, evidence_entries, events_of, failure_message,
    markdown_missing_five_sections, only_chain_id, sha256_hex, start_run_with,
};
use super::single_candidate_provider_run::{ProviderRunFixture, wait_for_single_candidate_phase};
use super::*;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    ProviderCompletion, ProviderEvent, ProviderSession, StreamChunk, StreamingProviderInput,
};
use crate::product::models::SingleCandidatePhase;
use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;
use crate::web::workspace_ws_handler::StreamingProviderAdapter;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio_util::sync::CancellationToken;

/// 首轮正常交付（制造可教 compile 失败）；第二轮已物理启动但**不交付输出**
/// （事件通道关闭）⇒ drive 真实失败。
struct FailAfterStartProvider {
    output: String,
    inputs: mpsc::UnboundedSender<StreamingProviderInput>,
    starts: AtomicUsize,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for FailAfterStartProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        // 每次 start 都真实入队输入（⇒ 实际排队数 = 真实启动数）。
        let _ = self.inputs.send(input);
        let index = self.starts.fetch_add(1, Ordering::SeqCst);
        let (command_tx, _command_rx) = mpsc::channel(1);
        if index == 0 {
            let (event_tx, event_rx) = mpsc::channel(4);
            let output = self.output.clone();
            tokio::spawn(async move {
                let _ = event_tx
                    .send(ProviderEvent::Completed(ProviderCompletion::plain(output, None)))
                    .await;
            });
            return Ok(ProviderSession {
                native_session_id: None,
                events: event_rx,
                commands: command_tx,
            });
        }
        // 第二轮：物理启动已发生（输入已入队），但没有任何输出 ⇒ drive 真实失败。
        let (event_tx, event_rx) = mpsc::channel(1);
        drop(event_tx);
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn run_streaming(
        &self,
        _input: &crate::protocol::contracts::AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        unreachable!("SC provider-run 测试走 start")
    }
}

/// 教学轮已物理启动但交付失败 ⇒ prompt 处置必须是 `sent`（RED：当前是 `prepared`）。
#[tokio::test]
async fn sc_failure_package_teaching_started_without_delivery_records_sent_prompt() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let story_id = fixture.story_id.clone();
    let design_id = fixture.design_id.clone();
    let output = markdown_missing_five_sections(&story_id, &design_id, "前端渲染不在本次范围内。");
    let (input_tx, input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(FailAfterStartProvider {
        output,
        inputs: input_tx,
        starts: AtomicUsize::new(0),
    });
    let mut run = start_run_with(fixture, provider, input_rx).await;

    // 真实输入队列：原轮 + 教学轮（教学 prompt 已真实交给 provider）。
    let _original_input = run.queue.next_input().await;
    let _teaching_input = run.queue.next_input().await;
    let message = failure_message(&mut run.outbound).await;
    assert!(
        !message.is_empty(),
        "教学轮交付失败必须有真实失败面（不得静默）"
    );
    wait_for_single_candidate_phase(&run.fixture, SingleCandidatePhase::Failed).await;
    run.queue.expect_no_further_input().await;
    assert_eq!(
        run.queue.received_count(),
        2,
        "实际排队恰两条（原轮 + 教学轮）"
    );

    // durable 证据：教学判定仍是 granted（真实授予，未被失败改写）。
    let chain_id = only_chain_id(&run.fixture);
    let entries = evidence_entries(&run.fixture, &chain_id);
    let decisions = events_of(&entries, "teaching_decision");
    assert_eq!(decisions.len(), 1, "真实教学判定必须落一条");
    assert_eq!(
        decisions[0]["event"]["decision"], "granted",
        "教学已真实授予（不得被交付失败改写）"
    );
    let observed = events_of(&entries, "failure_observed");
    let compile_round_id = observed
        .iter()
        .find(|entry| entry["event"]["failure_class"] == "compile")
        .map(|entry| entry["event"]["round_id"].as_str().unwrap_or_default().to_string())
        .expect("首轮 compile 失败事实");

    // durable prompt 审计：处置必须是 sent；正文哈希/引用与已准备正文一致。
    let prompts = events_of(&entries, "prompt_prepared");
    assert_eq!(prompts.len(), 1, "恰一条 durable 教学提示审计");
    let evidence = &prompts[0]["event"]["evidence"];
    assert_eq!(
        evidence["disposition"], "sent",
        "教学轮已物理启动（prompt 已交给 provider）⇒ 处置必须是 sent，不得停在 prepared"
    );
    let prompt_ref = &evidence["prompt"];
    assert_eq!(
        prompt_ref["status"], "available",
        "已发送的 prompt 必须留可读回引用"
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
        "已发送的 prompt 正文必须与准备好的正文同字节数（同一份正文）"
    );
    assert_eq!(
        sha256_hex(body.as_bytes()),
        prompt_ref["sha256"].as_str().unwrap_or_default(),
        "已发送的 prompt 正文必须与准备好的正文同摘要"
    );

    // 生产终局发布的 A4 包：教学轮未交付 ⇒ 不完整、末物理轮输出 Missing、
    // 失败源仍指上一轮（compile 轮）。
    let packages = durable_sc_package_versions(&run.fixture);
    assert_eq!(packages.len(), 1, "durable 版本表恰有一个 A4 失败包版本");
    let package = &packages[0]["sc_failure_repair_package"];
    assert_eq!(
        package["completeness"], "incomplete",
        "末物理轮未交付 ⇒ 包必须诚实不完整"
    );
    assert_eq!(
        package["last_provider_output"]["status"], "missing",
        "末物理轮（教学轮）没有交付 ⇒ 输出必须是 Missing"
    );
    assert_eq!(
        package["last_failure_source"]["source_round_id"].as_str(),
        Some(compile_round_id.as_str()),
        "失败源必须仍指上一轮（compile 轮），不得被教学轮冒充"
    );
}
