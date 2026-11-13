//! P27 定向行为矩阵的**共享真实夹具**（P27a 失败包矩阵 / P27b 来源预算矩阵共用；
//! 容量分片产物：共用装配与读面抽到一处，避免每个用例文件重复 setup）。既有
//! fixture 与确定性 adapter 原样复用（不复制），本模块只补四件矩阵专用的事：
//!
//! 1. **真实 provider 输入队列 spy**：物理启动计数只认 adapter `start` 回调真实
//!    入队的条数 + durable `provider_start_ledger` 前缀条目数（不按路由/文本推断）；
//! 2. **真实 durable 读取**：`rounds.jsonl` / `artifact_versions.json` /
//!    `candidate-attempts.jsonl` / 会话 JSON（只解析，不重建事实）；
//! 3. **教学窗口状态快照**：before/after 口径（窗口建立时只固化原调用占位
//!    `attempt_no == 1`；教学 claim 会追加 `attempt_no == 2` ⇒ 零 claim 的判据是
//!    「durable 窗口里没有任何 `attempt_no >= 2`」）；
//! 4. **失败样本**与**结构化 source 块提取**（动态围栏 + durable 值锚点，
//!    不 pin 任何人类可读措辞、不 trim 末尾 LF）。
//!
//! 落位：`src/web/workspace_ws_handler/tests/sc_failure_driver_fixture.rs`

use super::single_candidate_provider_run::{
    ProviderRunFixture, SequenceOutputProvider, next_error_message, next_provider_input,
    no_more_provider_inputs, single_candidate_context, single_candidate_markdown,
};
use super::*;
use crate::cross_cutting::streaming_provider::StreamingProviderInput;
use crate::product::lifecycle_store::candidate_attempts::CandidateAttemptRecord;
use sha2::Digest as _;
use std::sync::atomic::AtomicUsize;

/// 正文 UTF-8 字节的 sha256 小写 hex（与证据库同一口径）。
pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(sha2::Sha256::digest(bytes))
}

// ── 真实 durable 读取 ─────────────────────────────────────────────────────

/// 生产写入的严格证据分区根目录（真实磁盘路径）。
pub(super) fn evidence_root(fixture: &ProviderRunFixture) -> std::path::PathBuf {
    fixture
        .lifecycle
        .workspace_timeline_root_for_session(&fixture.record.id)
        .expect("timeline root")
        .join("sc-failure-evidence")
}

/// 失败链目录名列表（真实磁盘目录；未建立 ⇒ 空列表）。
pub(super) fn chain_ids(fixture: &ProviderRunFixture) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(evidence_root(fixture)) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| !name.starts_with('.'))
        .collect();
    ids.sort();
    ids
}

/// 唯一的失败链目录名（本矩阵用例均为单链会话）。
pub(super) fn only_chain_id(fixture: &ProviderRunFixture) -> String {
    let ids = chain_ids(fixture);
    assert_eq!(ids.len(), 1, "本矩阵用例只应有一条失败链，实际 {ids:?}");
    ids.into_iter().next().expect("chain id")
}

/// 严格证据日志的真实条目（只解析 JSON，不重建事实）。
pub(super) fn evidence_entries(
    fixture: &ProviderRunFixture,
    chain_id: &str,
) -> Vec<serde_json::Value> {
    let path = evidence_root(fixture).join(chain_id).join("rounds.jsonl");
    let raw = std::fs::read_to_string(&path).expect("read strict evidence log");
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("parse evidence entry"))
        .collect()
}

/// 按事件 `kind` 取真实条目。
pub(super) fn events_of<'a>(
    entries: &'a [serde_json::Value],
    kind: &str,
) -> Vec<&'a serde_json::Value> {
    entries
        .iter()
        .filter(|entry| entry["event"]["kind"] == kind)
        .collect()
}

/// durable `artifact_versions.json` 的全部版本（生产 store 读面）。
pub(super) fn durable_artifact_versions(fixture: &ProviderRunFixture) -> Vec<serde_json::Value> {
    fixture
        .lifecycle
        .list_artifact_versions(&fixture.record.id)
        .expect("read durable artifact versions")
        .iter()
        .map(|version| serde_json::to_value(version).expect("serialize artifact version"))
        .collect()
}

/// durable 版本表里的 A4 失败包版本。`ArtifactVersion` 的载荷是**扁平**序列化
/// （`ArtifactPayload` 为 untagged，A4 变体直接是 `sc_failure_repair_package` 顶层键；
/// 与磁盘 `artifact_versions.json` 真实形状一致，无 `payload` 包装层）。
pub(super) fn durable_sc_package_versions(fixture: &ProviderRunFixture) -> Vec<serde_json::Value> {
    durable_artifact_versions(fixture)
        .into_iter()
        .filter(|version| version["sc_failure_repair_package"].is_object())
        .collect()
}

/// 会话的真实候选尝试流水（durable issue 级文件）。
pub(super) fn candidate_attempts(fixture: &ProviderRunFixture) -> Vec<CandidateAttemptRecord> {
    fixture
        .lifecycle
        .list_candidate_attempts(&fixture.record.id)
        .expect("read candidate attempts")
}

/// durable 物理启动账本里 SC author 前缀条目数（权威启动口径）。
pub(super) fn author_start_count(fixture: &ProviderRunFixture) -> usize {
    let prefix = format!("single_candidate_author:{}:", fixture.record.id);
    fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload session")
        .provider_start_ledger
        .iter()
        .filter(|entry| entry.provider_start_idempotency_key.starts_with(&prefix))
        .count()
}

// ── 运行驱动 + 真实输入队列 spy ──────────────────────────────────────────

/// 真实 provider 输入队列 spy：矩阵的「实际物理启动」以此为准。
pub(super) struct ProviderQueueSpy {
    receiver: mpsc::UnboundedReceiver<StreamingProviderInput>,
    received: usize,
}

impl ProviderQueueSpy {
    /// 取下一条真实 provider 输入（真实 adapter 的 start 回调入队的那一条）。
    pub(super) async fn next_input(&mut self) -> StreamingProviderInput {
        self.received += 1;
        next_provider_input(&mut self.receiver).await
    }

    /// 断言此后不再有 provider 输入（零额外物理启动）。
    pub(super) async fn expect_no_further_input(&mut self) {
        no_more_provider_inputs(&mut self.receiver).await;
    }

    /// 已收到的真实输入条数。
    pub(super) fn received_count(&self) -> usize {
        self.received
    }
}

/// 一次真实 SC 运行：夹具 + adapter + 真实输入队列 + 出站通道。
pub(super) struct ScMatrixRun {
    pub(super) fixture: ProviderRunFixture,
    pub(super) queue: ProviderQueueSpy,
    pub(super) outbound: mpsc::Receiver<OutboundControl>,
}

/// 既有确定性 adapter（按序给不同输出；最后一次输出会被重复使用）。
pub(super) fn sequence_provider(
    outputs: Vec<String>,
) -> (
    Arc<SequenceOutputProvider>,
    mpsc::UnboundedReceiver<StreamingProviderInput>,
) {
    let (input_tx, input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(SequenceOutputProvider {
        outputs,
        inputs: input_tx,
        next: AtomicUsize::new(0),
    });
    (provider, input_rx)
}

/// 起一次真实 SC 生成：真实 handler 入口 + 真实 adapter + 真实 store。
pub(super) async fn start_run_with(
    fixture: ProviderRunFixture,
    provider: Arc<dyn StreamingProviderAdapter>,
    input_rx: mpsc::UnboundedReceiver<StreamingProviderInput>,
) -> ScMatrixRun {
    let (context, outbound) = single_candidate_context(&fixture, provider);
    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
            resume_recovery: None,
        },
    )
    .await;
    ScMatrixRun {
        fixture,
        queue: ProviderQueueSpy {
            receiver: input_rx,
            received: 0,
        },
        outbound,
    }
}

/// 起一次真实 SC 生成（既有确定性 adapter）。
pub(super) async fn start_run(fixture: ProviderRunFixture, outputs: Vec<String>) -> ScMatrixRun {
    let (provider, input_rx) = sequence_provider(outputs);
    start_run_with(fixture, provider, input_rx).await
}

/// 等真实失败原因（出站 error 帧）。
pub(super) async fn failure_message(outbound: &mut mpsc::Receiver<OutboundControl>) -> String {
    next_error_message(outbound).await
}

// ── 教学窗口状态快照（before/after 口径） ────────────────────────────────

/// 教学窗口状态快照（durable 事实）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WindowSnapshot {
    pub(super) windows: usize,
    /// 各 attempt 的 `attempt_no`（有序）。
    pub(super) attempts: Vec<u8>,
    pub(super) max_attempts: u8,
    pub(super) repair_once_approved: bool,
    pub(super) claimable_attempt_no: Option<u8>,
}

/// 读一次 durable 教学窗口快照（窗口未建立 ⇒ `None`）。
pub(super) fn window_snapshot(fixture: &ProviderRunFixture) -> Option<WindowSnapshot> {
    let record = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload session");
    let window = record.sc_generation_teaching_windows.last()?;
    Some(WindowSnapshot {
        windows: record.sc_generation_teaching_windows.len(),
        attempts: window
            .attempts
            .iter()
            .map(|attempt| attempt.attempt_no)
            .collect(),
        max_attempts: window.max_attempts,
        repair_once_approved: window.repair_once_approved,
        claimable_attempt_no: window.claimable_attempt_no(),
    })
}

/// durable 窗口里全部教学 attempt 的 `attempt_no`（教学 claim 追加 `>= 2`）。
pub(super) fn teaching_attempt_numbers(fixture: &ProviderRunFixture) -> Vec<u8> {
    let record = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload session");
    record
        .sc_generation_teaching_windows
        .iter()
        .flat_map(|window| window.attempts.iter().map(|attempt| attempt.attempt_no))
        .collect()
}

// ── 失败样本（复用既有夹具样本，不另造） ──────────────────────────────────

/// 删除五段 section，制造**多段** missing_section 编译失败（真实 cause 数组 >4 条）。
///
/// 删除 `### Goal` / `### Tasks` / `### Acceptance Criteria` / `### Verification` /
/// `### Handoff Schema` 五段 ⇒ 真实诊断条数 ≥5；`### Non Goals` 保留，超长填充放在
/// 它的值里（不新增 section、不新增 key ⇒ 不引入别的诊断码）。
pub(super) fn markdown_missing_five_sections(
    story_id: &str,
    design_id: &str,
    filler: &str,
) -> String {
    let markdown = single_candidate_markdown(story_id, design_id)
        .split("\n\n")
        .filter(|chunk| {
            ![
                "### Goal",
                "### Tasks",
                "### Acceptance Criteria",
                "### Verification",
                "### Handoff Schema",
            ]
            .iter()
            .copied()
            .any(|section| chunk.starts_with(section))
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    markdown.replace(
        "- non_goals: Frontend rendering is out of scope.",
        &format!("- non_goals: {filler}"),
    )
}

/// IR 预检失败样本：非法 requirement 引用（复用既有 IR 重驱夹具函数）。
pub(super) fn markdown_unknown_requirement_ref(story_id: &str, design_id: &str) -> String {
    super::single_candidate_ir_reredrive::markdown_with_unknown_requirement_ref(story_id, design_id)
}

/// **自然**不可教的 compile 失败样本：重复 trusted command（lowering 类不在教学
/// 重驱白名单 ⇒ 零重驱，天然终局）。
///
/// 与既有夹具的私有样本 `single_candidate_markdown_with_duplicate_command` 同款
/// （既有 helper 是私有的，这里只复用其**样本内容**，不改既有文件可见性）。
pub(super) fn markdown_duplicate_trusted_command(story_id: &str, design_id: &str) -> String {
    single_candidate_markdown(story_id, design_id).replacen(
        "- check_id: CHECK-001\n- manual_instruction: Inspect the backend API response manually.",
        "- check_id: CHECK-001\n- command: cargo test --locked --lib backend_api\n- manual_instruction: Inspect the backend API response manually.\n- required: true\n- non_zero_test_execution_required: false\n- check_id: CHECK-002\n- command: cargo test --locked --lib backend_api\n- manual_instruction: Confirm the backend API response manually.\n- required: true\n- non_zero_test_execution_required: false",
        1,
    )
}

// ── 结构化 source 块提取（不 pin 措辞、不 trim 末尾 LF） ──────────────────

/// 从真实 prompt 文本里按**结构**切出失败源正文：枚举所有「整行反引号」的围栏行
/// ⇒ 对每一对同文本围栏，取两围栏之间的**原始字节**（不含围栏行、不含闭围栏前
/// 那个 LF、不 trim 末尾 LF）⇒ 以 durable 的 `bytes`/`sha256` 复算判定哪一段才是
/// 失败源正文。只依据**值锚点**与**动态围栏**，不 pin 任何人类可读措辞。
pub(super) fn extract_source_body(prompt: &str, source_sha256: &str, source_bytes: u64) -> String {
    let mut fences: Vec<(usize, usize, String)> = Vec::new();
    let mut offset = 0usize;
    for line in prompt.split_inclusive('\n') {
        let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
        if trimmed.len() >= 3 && trimmed.chars().all(|ch| ch == '`') {
            fences.push((offset, offset + line.len(), trimmed.to_string()));
        }
        offset += line.len();
    }
    for (open_index, open) in fences.iter().enumerate() {
        for close in fences.iter().skip(open_index + 1) {
            if close.2 != open.2 {
                continue;
            }
            let body_start = open.1;
            let body_end = close.0.saturating_sub(1);
            if body_end < body_start {
                continue;
            }
            let body = &prompt[body_start..body_end];
            if body.len() as u64 == source_bytes && sha256_hex(body.as_bytes()) == source_sha256 {
                return body.to_string();
            }
        }
    }
    panic!(
        "prompt 内没有与 durable 失败源（bytes={source_bytes}, sha256={source_sha256}）逐字相等的围栏块"
    );
}

// ── 取消用例的 adapter 放在 `sc_failure_package.rs`（只有取消用例用它） ──
