//! Task 11a:失败矩阵结构面——零 spawn 证据会计必须分列数清 process 与协议
//! child(Step 2 红锚点,实施计划 Task 11 段 488-511 行)。
//!
//! 计划 Interfaces(492 行):spawn 统计必须分列 session/协议 child、
//! extension/MCP 后代、native 方法次数;availability `--version` 另记,
//! 不冒充 session;完整 fixture 失败证据都逐格保留;不能观察 OS 保护则
//! Unknown。现有 harness 的唯一 spawn 计数源
//! `count_session_provider_starts` 只数成功启动的 `ProviderStart` audit
//! 记录——被拒启动不落 audit,「无记录」不能证明「零 child」;
//! `EvidenceCell::provider_spawn_count` 单一维度也无法分列五类观察对象。
//! 本单测钉死 cell.json 的零 spawn 会计五维与「零 spawn 被拒格不得携带
//! provider-start 成功证据」的结构校验(当前红;Step 3 实现会计与真实
//! 探针后转绿)。真实现场断言组见 boundary_matrix.rs 四家 `#[ignore]`
//! 测试。

use std::path::{Path, PathBuf};

use cadence_aria::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
use cadence_aria::product::models::ProviderName;
use serde_json::Value;

use super::harness::{ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT, EvidenceCell, FRESH};

/// 零 spawn 会计五维(计划 Interfaces 冻结;cell.json 落盘键)。
pub(crate) const ZERO_SPAWN_ACCOUNTING_KEYS: [&str; 5] = [
    "session_child_count",
    "native_handshake_count",
    "extension_mcp_descendant_count",
    "native_method_count",
    "availability_version_count",
];

/// Task 11 Step 1 负向固定场景集(计划 494 行枚举:缺 readiness/正文/
/// receipt/trust、launch/boundary Unknown/Denied、version/wire 漂移、
/// Codex danger、role 非法、false 标志、resume Unknown/fingerprint、
/// target/git pointer 与 D4 缺失/漂移)。失败矩阵必须逐格保留每个场景的
/// 证据,不得删格/合并隐藏;boundary_matrix 现场覆盖断言消费本清单。
pub(crate) const FIXED_FAILURE_SCENARIOS: [&str; 17] = [
    "missing_readiness",
    "missing_body",
    "missing_receipt",
    "missing_trust",
    "launch_unknown",
    "launch_denied",
    "boundary_unknown",
    "boundary_denied",
    "version_drift",
    "wire_drift",
    "codex_danger_full_access",
    "illegal_role",
    "false_flag",
    "resume_unknown",
    "fingerprint_drift",
    "target_git_pointer_missing_or_drift",
    "d4_missing_or_drift",
];

/// 读取 cell.json 会计维度:缺维度=不可证(u64::MAX),不把「无记录」
/// 冒充 0(fail-closed;boundary_matrix 现场 helper 同语义)。
pub(crate) fn cell_accounting_dimension(cell_json: &Value, key: &str) -> u64 {
    cell_json
        .get(key)
        .and_then(Value::as_u64)
        .unwrap_or(u64::MAX)
}

/// capability_state 落盘字符串 → 可审计三态证据(harness cell 落盘值
/// "confirmed"/"denied"/其余(含 "unknown"/"carried")的映射)。
pub(crate) fn capability_state_evidence(state: &str) -> ProviderCapabilityEvidence {
    match state {
        "confirmed" => ProviderCapabilityEvidence::Confirmed,
        "denied" => ProviderCapabilityEvidence::denied("cell denied"),
        _ => ProviderCapabilityEvidence::unknown(),
    }
}

/// 成功格基线(与 live_matrix 结构测试同形态):全部断言要素齐备,
/// 必须通过结构校验。
fn success_baseline_cell(
    provider: ProviderName,
    canonical_root: &Path,
    member_worktree: &Path,
) -> EvidenceCell {
    EvidenceCell {
        provider: provider.clone(),
        exact_version: "2.1.283-boundary-fixture".to_string(),
        stage: "coding".to_string(),
        entrypoint: ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT.to_string(),
        fresh_or_resume: FRESH.to_string(),
        process_cwd: canonical_root.to_path_buf(),
        target: member_worktree.to_path_buf(),
        audit_projection_digest: "sha256:audit-projection-digest".to_string(),
        frozen_projection_digest: "sha256:audit-projection-digest".to_string(),
        native_resume_confirmed_id: None,
        native_resume_reattached: false,
        requested_resume_id: None,
        argv_or_wire_capture_exists: true,
        approval_and_tool_events_exist: true,
        completed_product_artifact_exists: true,
        run_ref: "boundary_run_0001".to_string(),
        run_ref_is_unique_within_entrypoint: true,
        action: "coding_target_write".to_string(),
        role: "executor".to_string(),
        gateway_dialect: "claude_code_cli_v1".to_string(),
        wire_dialect: "claude-stream-json".to_string(),
        native_session_id: "native-session-fixture".to_string(),
        workspace_session_id: "workspace_session_0001".to_string(),
        argv: vec![
            "claude".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
        ],
        capability_state: "confirmed".to_string(),
        denied_reason: None,
        provider_pid: Some("48017".to_string()),
        pid_unavailable_reason: None,
        provider_spawn_count: 1,
        session_projection_digest: "sha256:session-projection-fixture".to_string(),
        provider_events: vec![serde_json::json!({
            "ts": "2026-10-09T00:00:00.000Z",
            "event": {"type": "tool_call", "tool": "Write", "native_session_id": "native-session-fixture"}
        })],
        execution_origin: "full_chain".to_string(),
    }
}

/// 被拒(零 spawn)格:launch 在 spawn 前被门拒——键保留、成功证据空、
/// waiting reason 落 denied_reason、spawn 计数 0(与 harness denied_cell
/// 的零 spawn 形态一致:spawn 0 → unknown)。
fn rejected_launch_cell(
    provider: ProviderName,
    canonical_root: &Path,
    member_worktree: &Path,
) -> EvidenceCell {
    EvidenceCell {
        provider,
        exact_version: String::new(),
        stage: "coding".to_string(),
        entrypoint: ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT.to_string(),
        fresh_or_resume: FRESH.to_string(),
        process_cwd: canonical_root.to_path_buf(),
        target: member_worktree.to_path_buf(),
        audit_projection_digest: String::new(),
        frozen_projection_digest: String::new(),
        native_resume_confirmed_id: None,
        native_resume_reattached: false,
        requested_resume_id: None,
        argv_or_wire_capture_exists: false,
        approval_and_tool_events_exist: false,
        completed_product_artifact_exists: false,
        run_ref: "boundary_run_0002".to_string(),
        run_ref_is_unique_within_entrypoint: true,
        action: "coding_target_write".to_string(),
        role: "executor".to_string(),
        gateway_dialect: String::new(),
        wire_dialect: String::new(),
        native_session_id: String::new(),
        workspace_session_id: "workspace_session_0002".to_string(),
        argv: Vec::new(),
        capability_state: "unknown".to_string(),
        denied_reason: Some(
            "launch_denied:write_boundary Unknown(探针未确认,零 spawn 等待显式修复杂件)"
                .to_string(),
        ),
        provider_pid: None,
        pid_unavailable_reason: None,
        provider_spawn_count: 0,
        session_projection_digest: String::new(),
        provider_events: vec![serde_json::json!({
            "ts": "2026-10-09T00:00:01.000Z",
            "event": {"type": "launch_rejected", "reason": "write_boundary Unknown"}
        })],
        execution_origin: "full_chain".to_string(),
    }
}

/// 单个矛盾/缺字段变体必须被证据结构校验拒绝。
fn expect_rejected(
    variant: EvidenceCell,
    provider: &ProviderName,
    canonical_root: &Path,
    member_worktree: &Path,
    context: &str,
) {
    let rejection = variant.validate_against(provider, canonical_root, member_worktree);
    assert!(
        rejection.is_err(),
        "{context}:矛盾记录必须被结构校验拒绝(得到了 {rejection:?})"
    );
}

#[test]
fn lcg_t11_zero_spawn_evidence_counts_process_and_protocol_children() {
    let canonical_root = PathBuf::from("/tmp/lcg-t11-boundary/canonical-root");
    let member_worktree = canonical_root.join("checkouts/alpha");
    let provider = ProviderName::ClaudeCode;

    // 合法成功格基线:结构校验必须继续放行(新增拒绝规则不得误伤)。
    let success = success_baseline_cell(provider.clone(), &canonical_root, &member_worktree);
    assert!(
        success
            .validate_against(&provider, &canonical_root, &member_worktree)
            .is_ok(),
        "完整成功格必须通过结构校验(零 spawn 会计新维度不得破坏合法证据)"
    );

    // 被拒(零 spawn)格:launch 在 spawn 前被门拒,成功证据全空。
    let rejected = rejected_launch_cell(provider.clone(), &canonical_root, &member_worktree);
    let rejected_json = rejected.to_cell_json();

    // ---- A. 零 spawn 会计五维分列(红锚点:provider_spawn_count 单维只
    // 源自成功 ProviderStart audit 计数——被拒启动不落 audit,「无记录」
    // 不能证明「零 child」) ----
    let rejected_session_child_count =
        cell_accounting_dimension(&rejected_json, "session_child_count");
    assert_eq!(
        rejected_session_child_count, 0,
        "被拒格必须显式证明零 session child:现有 provider_spawn_count 源自成功 audit \
         计数,被拒启动无 audit 记录,缺分列维度即不可证零"
    );
    let rejected_native_handshake_count =
        cell_accounting_dimension(&rejected_json, "native_handshake_count");
    assert_eq!(
        rejected_native_handshake_count, 0,
        "被拒格必须显式证明零协议握手 child(协议栈独立于进程栈,成功 audit 计数覆盖不了)"
    );
    assert_eq!(
        cell_accounting_dimension(&rejected_json, "extension_mcp_descendant_count"),
        0,
        "extension/MCP 后代必须独立计数(经 provider 子进程派生,并入 session 维度即漏计)"
    );
    assert_eq!(
        cell_accounting_dimension(&rejected_json, "native_method_count"),
        0,
        "native 方法调用次数必须独立计数(RPC/ACP 方法面独立于进程面)"
    );
    // availability `--version` 另记:键必须存在(值=真实探测次数,可≥0;
    // 探测不是 session child,不得计入 session_child_count)。
    assert!(
        rejected_json.get("availability_version_count").is_some(),
        "availability --version 探测必须另记维度,不冒充 session child(缺键=未分列)"
    );
    // 成功格同样必须携带五维会计(确认格的 child 观测分列可审计)。
    let success_json = success.to_cell_json();
    for key in ZERO_SPAWN_ACCOUNTING_KEYS {
        assert!(
            success_json.get(key).is_some(),
            "成功格 cell.json 必须携带零 spawn 会计维度 {key}(分列可审计)"
        );
    }

    // ---- B. 零 spawn 被拒格不得携带 provider-start 成功证据(伪装格必须
    // 被结构校验拒绝;握手失败已创建=kill/reap 的格 spawn≥1,不属此列)。
    // 这是 `!rejected_provider_start_success_exists` 的结构面。----
    for state in ["denied", "unknown"] {
        let mut disguised =
            success_baseline_cell(provider.clone(), &canonical_root, &member_worktree);
        disguised.capability_state = state.to_string();
        disguised.denied_reason =
            Some("launch_denied:write_boundary 探针未确认,零 spawn 等待显式修复杂件".to_string());
        expect_rejected(
            disguised,
            &provider,
            &canonical_root,
            &member_worktree,
            &format!("零 spawn 被拒格(state={state})伪装携带 provider-start 成功证据"),
        );
    }

    // ---- C. 缺证据格不得为 Confirmed(不能观察 OS 保护则 Unknown)。----
    let incomplete_cell_state = capability_state_evidence(&rejected.capability_state);
    assert_ne!(
        incomplete_cell_state,
        ProviderCapabilityEvidence::Confirmed,
        "缺证据格(不能观察 OS 保护=Unknown)不得为 Confirmed"
    );
}
