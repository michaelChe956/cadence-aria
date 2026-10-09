//! Task 11a:四家越界写、D4 与失败零 spawn 真实现场测试(`#[ignore]`
//! 骨架,实施计划 Task 11 段 Step 1/4)。
//!
//! - Step 4 现场命令(逐家,计划 509 行):
//!   `LC_GATEWAY_E2E=1 cargo test --locked --test it_web
//!    lcg_live_boundary_and_failures_<provider> -- --ignored --nocapture
//!    --test-threads=1`;
//! - 断言组逐条来自计划 Step 1(496-505 行);真实探针与
//!   `LiveLcGatewayHarness::run_boundary_and_failure_matrix`(计划
//!   Interfaces 冻结:`provider: ProviderName` 入参、
//!   `Result<LiveMatrixEvidence, LiveMatrixFailure>` 同 Task 10 schema)
//!   属 Task 11 Step 3。11a 骨架阶段现场执行必须以 BLOCKED 失败告终,
//!   绝不以缺探针/删格冒充矩阵通过;
//! - 现场证据落点(计划 Files):`cadence/reports/lc-gateway-multi-provider/
//!   <provider>/boundary/`、`failures/`,provider 目录名固定为
//!   claude-code/codex/pi/kimi-code。

use cadence_aria::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
use cadence_aria::product::models::ProviderName;
use serde_json::Value;

use super::failure_matrix::{
    FIXED_FAILURE_SCENARIOS, ZERO_SPAWN_ACCOUNTING_KEYS, capability_state_evidence,
    cell_accounting_dimension,
};
use super::harness::{EvidenceCell, LiveMatrixEvidence, LiveMatrixFailure};
use super::live_matrix::require_lc_gateway_e2e_switch;

/// 零 spawn 被拒格(spawn 前被门拒:未确认且 spawn 计数为 0)。握手失败
/// 已创建 child 的 kill/reap 格 spawn≥1,不属零 spawn 格——计划 Step 3
/// 「不能把后者计入零 spawn 成功格」。
fn zero_spawn_rejected_cells(matrix: &LiveMatrixEvidence) -> Vec<&EvidenceCell> {
    matrix
        .cells()
        .iter()
        .filter(|cell| cell.capability_state != "confirmed" && cell.provider_spawn_count == 0)
        .collect()
}

/// 零 spawn 被拒格在指定会计维度的总和:缺维度=不可证(u64::MAX,
/// fail-closed),不把「无记录」当 0。
fn zero_spawn_rejected_accounting(matrix: &LiveMatrixEvidence, key: &str) -> u64 {
    zero_spawn_rejected_cells(matrix)
        .into_iter()
        .map(|cell| cell_accounting_dimension(&cell.to_cell_json(), key))
        .fold(0u64, u64::saturating_add)
}

/// 矩阵中是否存在携带该布尔证据键为 true 的格(缺键=false,fail-closed)。
fn any_cell_flag(matrix: &LiveMatrixEvidence, key: &str) -> bool {
    matrix.cells().iter().any(|cell| {
        cell.to_cell_json()
            .get(key)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    })
}

/// 保护面摘要(前/后):取首个携带完整证据对的格;缺证据=显式哨兵
/// (前后必不相等,断言红,fail-closed)。
fn protected_digests(matrix: &LiveMatrixEvidence) -> (String, String) {
    const MISSING_BEFORE: &str = "evidence-missing#protected_digest_before";
    const MISSING_AFTER: &str = "evidence-missing#protected_digest_after";
    for cell in matrix.cells() {
        let cell_json = cell.to_cell_json();
        if let (Some(before), Some(after)) = (
            cell_json
                .get("protected_digest_before")
                .and_then(Value::as_str),
            cell_json
                .get("protected_digest_after")
                .and_then(Value::as_str),
        ) {
            return (before.to_string(), after.to_string());
        }
    }
    (MISSING_BEFORE.to_string(), MISSING_AFTER.to_string())
}

/// 矩阵级契约:矩阵完备性 + 计划 Step 1 断言块(496-505 行)逐条。
fn assert_boundary_failure_matrix_contract(
    matrix: &LiveMatrixEvidence,
    selected_provider: &ProviderName,
) {
    assert_eq!(matrix.provider, *selected_provider, "矩阵确属所选 provider");
    assert!(
        matrix
            .rejections
            .iter()
            .all(|rejection| !rejection.reason.is_empty()),
        "被结构校验拒绝的格必须带非空 reason"
    );
    for cell in matrix.unconfirmed_cells() {
        assert!(
            cell.denied_reason
                .as_deref()
                .is_some_and(|reason| !reason.is_empty()),
            "缺证据格(stage={}/entrypoint={}/{})必须记录 Unknown/Denied reason",
            cell.stage,
            cell.entrypoint,
            cell.fresh_or_resume
        );
    }

    // 完整 fixture 失败证据逐格保留:固定负向场景(计划 494 行枚举)必须
    // 各自有格(failure_scenario 落 cell.json),不得删格/合并隐藏。
    let covered: Vec<String> = matrix
        .cells()
        .iter()
        .filter_map(|cell| {
            cell.to_cell_json()
                .get("failure_scenario")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    let missing_scenarios: Vec<&str> = FIXED_FAILURE_SCENARIOS
        .iter()
        .copied()
        .filter(|scenario| !covered.iter().any(|seen| seen == scenario))
        .collect();
    assert!(
        missing_scenarios.is_empty(),
        "失败矩阵必须逐格覆盖固定负向场景集(计划 494 行枚举),缺:{missing_scenarios:?}"
    );
    assert!(
        !matrix.unconfirmed_cells().is_empty(),
        "失败证据必须逐格保留:矩阵无任何未确认格=失败格被删/被 Confirmed 冒充"
    );

    // 零 spawn 会计五维必须全格落盘(availability `--version` 另记,
    // 不冒充 session child)。
    for cell in matrix.cells() {
        let cell_json = cell.to_cell_json();
        for key in ZERO_SPAWN_ACCOUNTING_KEYS {
            assert!(
                cell_json.get(key).is_some(),
                "格(stage={}/entrypoint={}/{})缺零 spawn 会计维度 {key}",
                cell.stage,
                cell.entrypoint,
                cell.fresh_or_resume
            );
        }
    }

    // ---- 计划 Step 1 断言块(496-505 行)逐条 ----
    let rejected_session_child_count =
        zero_spawn_rejected_accounting(matrix, "session_child_count");
    let rejected_native_handshake_count =
        zero_spawn_rejected_accounting(matrix, "native_handshake_count");
    let rejected_provider_start_success_exists = zero_spawn_rejected_cells(matrix)
        .iter()
        .any(|cell| cell.argv_or_wire_capture_exists && !cell.argv.is_empty());
    let coding_target_controlled_file_written =
        any_cell_flag(matrix, "coding_target_controlled_file_written");
    let protected_writes_all_refused_by_os_or_native_policy = any_cell_flag(
        matrix,
        "protected_writes_all_refused_by_os_or_native_policy",
    );
    let (protected_before, protected_after) = protected_digests(matrix);
    let d4_detected_or_delivered_without_non_target_drift =
        any_cell_flag(matrix, "d4_detected_or_delivered_without_non_target_drift");
    let incomplete_cell_state = matrix
        .unconfirmed_cells()
        .first()
        .map(|cell| capability_state_evidence(&cell.capability_state))
        .unwrap_or(ProviderCapabilityEvidence::Confirmed);

    assert_eq!(
        rejected_session_child_count, 0,
        "被拒(零 spawn)格必须显式证明零 session child(缺维度=不可证零)"
    );
    assert_eq!(
        rejected_native_handshake_count, 0,
        "被拒(零 spawn)格必须显式证明零协议握手 child"
    );
    assert!(
        !rejected_provider_start_success_exists,
        "被拒(零 spawn)格不得携带 provider-start 成功证据(握手失败已创建=kill/reap \
         的格不属零 spawn 格,不得混入)"
    );
    assert!(
        coding_target_controlled_file_written,
        "Coding 正向:target 内受控文件必须真实写入(真实 git add/commit)"
    );
    assert!(
        protected_writes_all_refused_by_os_or_native_policy,
        "保护面(root/非 target main/worktree/.git/.aria)写必须全部被 OS 或原生策略拒绝,\
         每条有拒绝原文与文件 digest"
    );
    assert_eq!(
        protected_before, protected_after,
        "保护面摘要前后必须一致(模型「不写」不是 attempt 证据)"
    );
    assert!(
        d4_detected_or_delivered_without_non_target_drift,
        "D4:所有 active main 快照(HEAD+porcelain)+root/metadata 快照可观测且无\
         非 target 漂移"
    );
    assert_ne!(
        incomplete_cell_state,
        ProviderCapabilityEvidence::Confirmed,
        "缺证据格不得为 Confirmed(不能观察 OS 保护则 Unknown)"
    );
}

/// 单家 provider 的真实现场越界写/D4/失败零 spawn 矩阵。
async fn run_live_boundary_and_failures(selected_provider: ProviderName) {
    require_lc_gateway_e2e_switch();
    let matrix = match boundary_failure_matrix(&selected_provider).await {
        Ok(matrix) => matrix,
        Err(failure) => panic!(
            "run_boundary_and_failure_matrix 现场执行失败 [{}]{}: {} \
             (环境不可运行须报告 BLOCKED,不删格)",
            failure.reason_code,
            failure
                .stage
                .map(|stage| format!("/{stage}"))
                .unwrap_or_default(),
            failure.message
        ),
    };
    assert_boundary_failure_matrix_contract(&matrix, &selected_provider);
}

/// Task 11 Step 3 接线点(计划 Interfaces 冻结接口形状):
/// `LiveLcGatewayHarness::run_boundary_and_failure_matrix(
///     provider: ProviderName) -> Result<LiveMatrixEvidence, LiveMatrixFailure>`
/// ——与 Task 10 同 schema;证据落计划 Files 冻结目录
/// `cadence/reports/lc-gateway-multi-provider/<provider>/boundary/` 与
/// `failures/`。
///
/// 11a 骨架阶段该方法尚未实现:现场执行必须以 BLOCKED 失败告终,不以
/// 缺探针/删格冒充矩阵通过;Step 3 落地后本函数体改为直接转调 harness
/// 方法,断言组保持不变。
async fn boundary_failure_matrix(
    provider: &ProviderName,
) -> Result<LiveMatrixEvidence, LiveMatrixFailure> {
    Err(LiveMatrixFailure {
        reason_code: "boundary_failure_matrix_not_wired".to_string(),
        message: format!(
            "LiveLcGatewayHarness::run_boundary_and_failure_matrix 尚未实现\
             (Task 11 Step 3 真实探针):provider {provider:?} 的越界写/D4/\
             失败零 spawn 矩阵不得以缺证据通过"
        ),
        stage: None,
    })
}

#[tokio::test]
#[ignore = "真实现场矩阵:LC_GATEWAY_E2E=1 且 Step 4 命令执行(需要真实 CLI/trust/#8 政策就绪;Task 11 Step 3 探针接线)"]
async fn lcg_live_boundary_and_failures_claude() {
    run_live_boundary_and_failures(ProviderName::ClaudeCode).await;
}

#[tokio::test]
#[ignore = "真实现场矩阵:LC_GATEWAY_E2E=1 且 Step 4 命令执行(需要真实 CLI/trust/#8 政策就绪;Task 11 Step 3 探针接线)"]
async fn lcg_live_boundary_and_failures_codex() {
    run_live_boundary_and_failures(ProviderName::Codex).await;
}

#[tokio::test]
#[ignore = "真实现场矩阵:LC_GATEWAY_E2E=1 且 Step 4 命令执行(需要真实 CLI/trust/#8 政策就绪;Task 11 Step 3 探针接线)"]
async fn lcg_live_boundary_and_failures_pi() {
    run_live_boundary_and_failures(ProviderName::Pi).await;
}

#[tokio::test]
#[ignore = "真实现场矩阵:LC_GATEWAY_E2E=1 且 Step 4 命令执行(需要真实 CLI/trust/#8 政策就绪;Task 11 Step 3 探针接线)"]
async fn lcg_live_boundary_and_failures_kimi() {
    run_live_boundary_and_failures(ProviderName::KimiCode).await;
}
