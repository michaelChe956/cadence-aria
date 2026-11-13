//! P27a：SC 失败修复包的**真实终局驱动**矩阵（Plan §7：S01/S02/S03/S07/S08/S13/S15）。
//!
//! 本文件不是组包器回声，也**不在测试里补生产缺失**：每个用例都经真实 handler
//! 入口 + 既有确定性 adapter 跑到真实终局，由生产代码写入真实 durable 证据日志
//!（`rounds.jsonl` + `chain_terminal`）；**失败包版本必须由生产终局发布**到
//! durable `artifact_versions.json`，本文件只断言回读出来的 durable 事实。
//!
//! 真实缺口：**P19d（终局实际发布接线）尚未落地** ⇒ 本文件的 A4 版本断言现在是
//! **真实 RED**（终局没有发布任何 A4 版本）。这是生产缺口，不是夹具问题；测试
//! 不代产品实现（不在用例里调 `build_sc_failure_repair_package` +
//! `publish_sc_failure_repair_package` 造包）。
//!
//! 断言面（全部为回读出来的 durable 事实）：
//! - durable 证据：轮绑定、`failure_observed` 的全量 cause 数组与引用、
//!   `teaching_decision`、`chain_terminal`；
//! - durable A4 版本：版本号（**由 durable 版本表实际内容派生，不 pin 字面量**）、
//!   节点 artifact_ref（由实际版本号派生）、包内轮次与 cause 数组、终端类、完整性；
//! - 计数：物理启动数只认 adapter `start` 回调真实入队条数 + durable
//!   `provider_start_ledger` 前缀条目数。
//!
//! 落位：`src/web/workspace_ws_handler/tests/sc_failure_package.rs`
//! 登记：`src/web/workspace_ws_handler/tests.rs`（与既有 `mod` 登记同区）

use super::sc_failure_driver_fixture::{
    author_start_count, candidate_attempts, chain_ids, durable_artifact_versions,
    durable_sc_package_versions, evidence_entries, events_of, failure_message,
    markdown_missing_five_sections, markdown_unknown_requirement_ref, only_chain_id,
    seed_exhausted_teaching_window, session_json_bytes, start_run, start_run_with,
    window_snapshot,
};
use super::single_candidate_provider_run::{ProviderRunFixture, wait_for_single_candidate_phase};
use super::*;
use crate::cross_cutting::provider_adapter::{ProviderAdapterError, StreamingProviderAdapter};
use crate::cross_cutting::streaming_provider::{
    ProviderSession, StreamChunk, StreamingProviderInput,
};
use crate::product::lifecycle_store::candidate_attempts::CandidateAttemptOutcome;
use crate::product::models::SingleCandidatePhase;
use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;
use tokio_util::sync::CancellationToken;

/// 真实 durable 失败包版本：终局必须**由生产发布**一个 A4 版本。
///
/// 版本号与节点引用都**由 durable 内容派生**（不 pin 字面量版本号）：A4 版本是
/// durable 版本表里承载 `sc_failure_repair_package` 载荷的那一条；节点 artifact_ref
/// 必须指向它的实际版本号。
fn published_failure_package(fixture: &ProviderRunFixture) -> serde_json::Value {
    let packages = durable_sc_package_versions(fixture);
    assert_eq!(
        packages.len(),
        1,
        "真实终局必须发布且仅发布一个 A4 失败包版本（当前 {} 个）",
        packages.len()
    );
    let version = packages.into_iter().next().expect("durable A4 版本");
    let version_number = version["version"].as_u64().expect("A4 版本号");
    let durable_max = durable_artifact_versions(fixture)
        .iter()
        .filter_map(|entry| entry["version"].as_u64())
        .max()
        .unwrap_or_default();
    assert!(
        version_number >= 1 && version_number <= durable_max,
        "A4 版本号必须是 durable 版本表里的实际序号（实测 {version_number}，表内最大 {durable_max}）"
    );
    let expected_ref = format!("artifact_version_{version_number:03}");
    assert!(
        session_json_bytes(fixture).contains(&expected_ref),
        "节点 artifact_ref 必须指向实际发布的版本（{expected_ref}）"
    );
    version
}

/// ① 终局失败（无教学重驱）⇒ 生产必须发布 durable 失败包版本。
///
/// 「无教学重驱」用**生产真实判定**实现：durable 账本已有两条历史启动 ⇒ 窗口未
/// 批准 ⇒ 教学判定 `Denied`（断言面是 durable 的 `teaching_decision` 事件）。
#[tokio::test]
async fn sc_failure_package_compile_terminal_publishes_durable_package() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let story_id = fixture.story_id.clone();
    let design_id = fixture.design_id.clone();
    seed_exhausted_teaching_window(&fixture).await;
    let output = markdown_missing_five_sections(&story_id, &design_id, "前端渲染不在本次范围内。");
    let mut run = start_run(fixture, vec![output]).await;

    let _original_input = run.queue.next_input().await;
    let message = failure_message(&mut run.outbound).await;
    assert!(
        message.contains("missing_section"),
        "真实失败面必须是 missing_section: {message}"
    );
    wait_for_single_candidate_phase(&run.fixture, SingleCandidatePhase::Failed).await;
    run.queue.expect_no_further_input().await;

    // durable 证据（生产写入）。
    let chain_id = only_chain_id(&run.fixture);
    let entries = evidence_entries(&run.fixture, &chain_id);
    let observed = events_of(&entries, "failure_observed");
    assert_eq!(observed.len(), 1, "一个机械失败轮");
    let failure = &observed[0]["event"];
    assert_eq!(failure["failure_class"], "compile");
    let reasons = failure["blocking_reasons"]
        .as_array()
        .expect("blocking reasons array");
    assert!(
        reasons.len() > 4,
        "完整阻断原因不被摘要替代（实测 {} 条）",
        reasons.len()
    );
    for section in [
        "Goal",
        "Tasks",
        "Acceptance Criteria",
        "Verification",
        "Handoff Schema",
    ] {
        assert!(
            reasons
                .iter()
                .any(|reason| reason.as_str().unwrap_or_default().contains(section)),
            "缺 section 的真实原因必须在场: {section}"
        );
    }
    assert!(
        !failure["repair_instructions"]
            .as_array()
            .expect("repair instructions array")
            .is_empty(),
        "修复指令必须在场"
    );
    assert_eq!(failure["raw"]["status"], "available", "provider 原文已归档");
    assert_eq!(failure["source"]["status"], "available", "失败源已归档");
    let decisions = events_of(&entries, "teaching_decision");
    assert_eq!(decisions.len(), 1, "真实教学判定必须落一条");
    assert_ne!(
        decisions[0]["event"]["decision"], "granted",
        "窗口未批准 ⇒ 不得授予教学重驱"
    );
    let terminal = events_of(&entries, "chain_terminal");
    assert_eq!(terminal.len(), 1, "真实终局条目必须落一条");
    assert_eq!(terminal[0]["event"]["terminal_class"], "compile");

    // 生产终局发布的 durable A4 版本（当前 P19d 未接线 ⇒ 真实 RED）。
    let version = published_failure_package(&run.fixture);
    let package = &version["payload"]["sc_failure_repair_package"];
    assert_eq!(package["chain_id"], chain_id, "A4 包必须绑定本轮真实失败链");
    assert_eq!(package["terminal_class"], "compile");
    assert_eq!(package["completeness"], "complete");
    let rounds = package["rounds"].as_array().expect("package rounds");
    assert_eq!(rounds.len(), 1, "无教学重驱 ⇒ 包内恰一轮");
    let round = &rounds[0];
    assert_eq!(round["failure_class"], "compile");
    assert_eq!(round["launch_state"], "started");
    assert_eq!(
        round["blocking_reasons"].as_array().expect("round reasons").len(),
        reasons.len(),
        "包内 cause 数组与 durable 证据逐条一致（不被摘要替代）"
    );
    assert_eq!(round["raw"], failure["raw"], "包内 raw 引用与 durable 证据逐字一致");
    assert_eq!(
        round["source"], failure["source"],
        "包内 source 引用与 durable 证据逐字一致"
    );

    // 计数口径：真实启动 = 原轮一次（零教学重驱）。
    assert_eq!(
        author_start_count(&run.fixture),
        3,
        "durable 启动账本：两条历史 + 原轮一次（零教学重驱）"
    );
}

/// ② 二次失败（教学重驱后再败）⇒ 包内两轮、终局类取最后一轮。
#[tokio::test]
async fn sc_failure_package_second_failure_keeps_both_rounds_in_durable_package() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let story_id = fixture.story_id.clone();
    let design_id = fixture.design_id.clone();
    let mut run = start_run(
        fixture,
        vec![
            markdown_missing_five_sections(&story_id, &design_id, "前端渲染不在本次范围内。"),
            markdown_unknown_requirement_ref(&story_id, &design_id),
        ],
    )
    .await;

    let _original_input = run.queue.next_input().await;
    let _teaching_input = run.queue.next_input().await;
    let message = failure_message(&mut run.outbound).await;
    assert!(
        message.contains("unknown_requirement_ref"),
        "次轮真实失败面必须是 IR typed 失败: {message}"
    );
    wait_for_single_candidate_phase(&run.fixture, SingleCandidatePhase::Failed).await;
    run.queue.expect_no_further_input().await;

    let chain_id = only_chain_id(&run.fixture);
    let entries = evidence_entries(&run.fixture, &chain_id);
    let observed = events_of(&entries, "failure_observed");
    assert_eq!(observed.len(), 2, "两个机械失败轮（首轮 + 教学重驱轮）");
    assert_eq!(observed[0]["event"]["failure_class"], "compile");
    assert_eq!(observed[1]["event"]["failure_class"], "ir_preflight");
    let first_reasons = observed[0]["event"]["blocking_reasons"]
        .as_array()
        .expect("first round reasons");
    assert!(
        first_reasons.len() > 4,
        "首轮完整 cause 数组不被次轮摘要替代（实测 {} 条）",
        first_reasons.len()
    );
    let second_reasons = observed[1]["event"]["blocking_reasons"]
        .as_array()
        .expect("second round reasons");
    assert!(
        second_reasons
            .iter()
            .any(|reason| reason.as_str().unwrap_or_default().contains("unknown_requirement_ref")),
        "次轮 typed 原因在场: {second_reasons:?}"
    );

    let version = published_failure_package(&run.fixture);
    let package = &version["payload"]["sc_failure_repair_package"];
    assert_eq!(package["chain_id"], chain_id, "A4 包必须绑定本轮真实失败链");
    assert_eq!(package["terminal_class"], "ir_preflight", "终局类取最后一轮");
    let rounds = package["rounds"].as_array().expect("package rounds");
    assert_eq!(rounds.len(), 2, "包内两轮齐全");
    assert_eq!(rounds[0]["failure_class"], "compile");
    assert_eq!(rounds[1]["failure_class"], "ir_preflight");
    assert_eq!(
        rounds[0]["blocking_reasons"]
            .as_array()
            .expect("first round reasons")
            .len(),
        first_reasons.len(),
        "包内首轮 cause 数组与 durable 证据逐条一致"
    );
    assert_eq!(
        rounds[1]["blocking_reasons"]
            .as_array()
            .expect("second round reasons")
            .len(),
        second_reasons.len(),
        "包内次轮 typed 原因条数与 durable 证据一致"
    );
    assert_eq!(
        author_start_count(&run.fixture),
        2,
        "durable 启动账本：原轮 + 恰一次教学重驱"
    );
}

/// 真实「在途」provider：入队真实输入后悬挂到取消（收到取消即收口）。
///
/// 取消用例专用（不进共享夹具）：它是「真实在途 run」的最小 adapter。
struct HangingProvider {
    inputs: mpsc::UnboundedSender<StreamingProviderInput>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for HangingProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let _ = self.inputs.send(input);
        let (event_tx, event_rx) = mpsc::channel(1);
        let (command_tx, _command_rx) = mpsc::channel(1);
        tokio::spawn(async move {
            tokio::select! {
                _ = cancel.cancelled() => {}
                _ = std::future::pending::<()>() => {}
            }
            drop(event_tx);
        });
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

/// ③ 失败前取消 ⇒ 零失败包（真实取消：既有 `abort_active_run` 取消在途 run）。
#[tokio::test]
async fn sc_failure_package_cancelled_before_failure_publishes_nothing() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(HangingProvider { inputs: input_tx });
    let mut run = start_run_with(fixture, provider, input_rx).await;

    // run 已在途（真实 provider 输入已入队）⇒ 取消在途 run。
    let _in_flight_input = run.queue.next_input().await;
    run.fixture.manager.abort_active_run().await;
    run.queue.expect_no_further_input().await;

    // 零失败包：durable 版本表里没有 A4 失败包版本。
    assert!(
        durable_sc_package_versions(&run.fixture).is_empty(),
        "失败前取消 ⇒ 不产失败包"
    );
    // 教学窗口零 attempt（取消发生在任何教学判定之前）。
    assert!(
        window_snapshot(&run.fixture).is_none(),
        "失败前取消 ⇒ 不建立教学窗口（无任何教学 attempt）"
    );
    // 证据日志：没有终局条目、也没有机械失败事实（是「失败前取消」）。
    for chain_id in chain_ids(&run.fixture) {
        let entries = evidence_entries(&run.fixture, &chain_id);
        assert!(
            events_of(&entries, "chain_terminal").is_empty(),
            "失败前取消 ⇒ 不写终局条目"
        );
        assert!(
            events_of(&entries, "failure_observed").is_empty(),
            "失败前取消 ⇒ 不写机械失败事实"
        );
    }
    // 候选尝试流水：一失败一行（本用例零失败行 ⇒ 无失败尝试）。
    let attempts = candidate_attempts(&run.fixture);
    assert!(
        attempts
            .iter()
            .all(|record| record.outcome != CandidateAttemptOutcome::Failed),
        "失败前取消 ⇒ 候选尝试流水里没有失败行"
    );
}
