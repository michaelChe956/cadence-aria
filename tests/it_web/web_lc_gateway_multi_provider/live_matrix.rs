//! Task 10a:证据结构单测 + 四家 `#[ignore]` 真实现场测试。
//!
//! - `lcg_t10_evidence_requires_exact_version_wire_and_native_confirmation`
//!   只验证证据结构:缺字段记录必须被拒绝(两阶段红绿的 RED 锚点);
//! - 四个 `lcg_live_*` 为真实测试,默认 `#[ignore]`;执行时必须显式开关
//!   `LC_GATEWAY_E2E=1` 且 `--ignored` 命中,开关缺失要失败而非 return 成功
//!   (Step 4 四命令归 controller 现场执行,10a 只交 harness)。

use std::path::{Path, PathBuf};

use cadence_aria::product::models::ProviderName;

use super::harness::{
    ENTRYPOINT_SPLIT_SYNC, ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT, EvidenceCell, FRESH,
    LC_GATEWAY_E2E_SWITCH, LiveLcGatewayHarness, RESUME,
};

/// 真实现场开关门:缺失/非 1 时测试失败(fail),不得静默 return 成功。
pub(crate) fn require_lc_gateway_e2e_switch() {
    match std::env::var(LC_GATEWAY_E2E_SWITCH).as_deref() {
        Ok("1") => {}
        other => panic!(
            "lcg_live_* 需要 {LC_GATEWAY_E2E_SWITCH}=1 才能执行真实现场矩阵 \
             (当前值:{other:?});开关缺失即失败,不以静默通过冒充 E2E"
        ),
    }
}

/// 结构测试的完整基线格:全部断言要素齐备,必须通过结构校验。
fn complete_baseline_cell(
    provider: ProviderName,
    canonical_root: &Path,
    member_worktree: &Path,
) -> EvidenceCell {
    EvidenceCell {
        provider: provider.clone(),
        exact_version: "2.1.283-matrix-fixture".to_string(),
        stage: "story".to_string(),
        entrypoint: ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT.to_string(),
        fresh_or_resume: FRESH.to_string(),
        process_cwd: canonical_root.to_path_buf(),
        target: member_worktree.to_path_buf(),
        audit_projection_digest: "sha256:audit-projection-digest".to_string(),
        frozen_projection_digest: "sha256:audit-projection-digest".to_string(),
        native_resume_confirmed_id: None,
        requested_resume_id: None,
        argv_or_wire_capture_exists: true,
        approval_and_tool_events_exist: true,
        completed_product_artifact_exists: true,
        run_ref: "split_run_0001".to_string(),
        run_ref_is_unique_within_entrypoint: true,
        action: "planning_read_only".to_string(),
        role: "work_item_splitter".to_string(),
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
    }
}

/// 单个缺字段/不一致变体必须被证据结构校验拒绝。
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
        "{context}:缺字段/不一致记录必须被拒绝(得到了 {rejection:?})"
    );
}

#[test]
fn lcg_t10_evidence_requires_exact_version_wire_and_native_confirmation() {
    let canonical_root = PathBuf::from("/tmp/lcg-t10-matrix/canonical-root");
    let member_worktree = canonical_root.join("checkouts/alpha");
    let provider = ProviderName::ClaudeCode;

    // 完整基线格必须通过结构校验(拒绝逻辑不得把合法证据一起拒掉)。
    let baseline = complete_baseline_cell(provider.clone(), &canonical_root, &member_worktree);
    assert!(
        baseline
            .validate_against(&provider, &canonical_root, &member_worktree)
            .is_ok(),
        "完整证据格(含 exact version/wire 捕获/原生确认)必须通过结构校验"
    );

    // ---- exact version 面 ----
    let mut missing_version = baseline.clone();
    missing_version.exact_version = String::new();
    expect_rejected(
        missing_version,
        &provider,
        &canonical_root,
        &member_worktree,
        "exact_version 为空",
    );

    // ---- wire/argv 面:真实 provider-start wire 不能空 argv 占位 ----
    let mut missing_argv_capture = baseline.clone();
    missing_argv_capture.argv_or_wire_capture_exists = false;
    expect_rejected(
        missing_argv_capture,
        &provider,
        &canonical_root,
        &member_worktree,
        "argv/wire 捕获缺失",
    );
    let mut empty_argv_payload = baseline.clone();
    empty_argv_payload.argv = Vec::new();
    expect_rejected(
        empty_argv_payload,
        &provider,
        &canonical_root,
        &member_worktree,
        "argv 载荷为空(空 argv 占位不算真实 wire)",
    );

    // ---- approval/tool 事件面 ----
    let mut missing_events = baseline.clone();
    missing_events.approval_and_tool_events_exist = false;
    expect_rejected(
        missing_events,
        &provider,
        &canonical_root,
        &member_worktree,
        "approval/tool 事件缺失",
    );

    // ---- 原生恢复确认面:confirmed != requested 必须拒绝 ----
    let mut resume_requested_only = baseline.clone();
    resume_requested_only.fresh_or_resume = RESUME.to_string();
    resume_requested_only.requested_resume_id = Some("native-session-fixture".to_string());
    resume_requested_only.native_resume_confirmed_id = None;
    expect_rejected(
        resume_requested_only,
        &provider,
        &canonical_root,
        &member_worktree,
        "请求 resume 但缺原生恢复确认",
    );
    let mut resume_wrong_native = baseline.clone();
    resume_wrong_native.fresh_or_resume = RESUME.to_string();
    resume_wrong_native.requested_resume_id = Some("native-session-fixture".to_string());
    resume_wrong_native.native_resume_confirmed_id = Some("other-native-session".to_string());
    expect_rejected(
        resume_wrong_native,
        &provider,
        &canonical_root,
        &member_worktree,
        "原生恢复确认 id 与请求 id 不一致",
    );

    // ---- 投影摘要面:audit 摘要与冻结摘要漂移必须拒绝 ----
    let mut digest_drift = baseline.clone();
    digest_drift.audit_projection_digest = "sha256:drifted".to_string();
    expect_rejected(
        digest_drift,
        &provider,
        &canonical_root,
        &member_worktree,
        "audit 投影摘要 != 冻结投影摘要",
    );

    // ---- cwd/target 面 ----
    let mut cwd_drift = baseline.clone();
    cwd_drift.process_cwd = member_worktree.clone();
    expect_rejected(
        cwd_drift,
        &provider,
        &canonical_root,
        &member_worktree,
        "进程 cwd 不是 canonical root",
    );
    let mut target_drift = baseline.clone();
    target_drift.target = canonical_root.clone();
    expect_rejected(
        target_drift,
        &provider,
        &canonical_root,
        &member_worktree,
        "target 不是成员 worktree",
    );

    // ---- 完成产物面 ----
    let mut missing_artifact = baseline.clone();
    missing_artifact.completed_product_artifact_exists = false;
    expect_rejected(
        missing_artifact,
        &provider,
        &canonical_root,
        &member_worktree,
        "完成产物缺失",
    );

    // ---- entrypoint 枚举面 ----
    let mut bad_entrypoint = baseline.clone();
    bad_entrypoint.entrypoint = "legacy_direct".to_string();
    expect_rejected(
        bad_entrypoint,
        &provider,
        &canonical_root,
        &member_worktree,
        "entrypoint 不在两入口枚举内",
    );

    // ---- run_ref 唯一性面 ----
    let mut duplicate_run_ref = baseline.clone();
    duplicate_run_ref.run_ref_is_unique_within_entrypoint = false;
    expect_rejected(
        duplicate_run_ref,
        &provider,
        &canonical_root,
        &member_worktree,
        "run_ref 在 entrypoint 内重复",
    );

    // ---- provider 面:格所属 provider 与所选 provider 不一致必须拒绝 ----
    let mut wrong_provider = baseline.clone();
    wrong_provider.provider = ProviderName::Codex;
    expect_rejected(
        wrong_provider,
        &provider,
        &canonical_root,
        &member_worktree,
        "格 provider 与所选 provider 不一致",
    );

    // ---- 阶段枚举面:五阶段之外不是合法格 ----
    let mut bad_stage = baseline;
    bad_stage.stage = "deploy".to_string();
    expect_rejected(
        bad_stage,
        &provider,
        &canonical_root,
        &member_worktree,
        "stage 不在五阶段枚举内",
    );
}

/// 单家 provider 的真实现场矩阵:断言组与矩阵完备性(459-482 行)。
async fn run_live_five_stage_matrix(selected_provider: ProviderName) {
    require_lc_gateway_e2e_switch();
    let evidence_root = live_evidence_root(&selected_provider);
    let harness = LiveLcGatewayHarness::new();
    let matrix = match harness
        .run_provider_matrix(selected_provider.clone(), &evidence_root)
        .await
    {
        Ok(matrix) => matrix,
        Err(failure) => panic!(
            "run_provider_matrix 现场执行失败 [{}]{}: {} (环境不可运行须报告 BLOCKED,不删格)",
            failure.reason_code,
            failure
                .stage
                .map(|stage| format!("/{stage}"))
                .unwrap_or_default(),
            failure.message
        ),
    };

    // 矩阵级校验:harness 返回的矩阵确属所选 provider;被结构校验拒绝的
    // 格都有非空 reason(与 unconfirmed 格的分列记录一致)。
    assert_eq!(matrix.provider, selected_provider);
    assert!(
        matrix
            .rejections
            .iter()
            .all(|rejection| !rejection.reason.is_empty())
    );

    let canonical_root = matrix.canonical_root().to_path_buf();
    let selected_member_worktree = matrix.member_worktree().to_path_buf();

    // 五阶段×fresh/resume 主格完备:不合并隐藏,缺格即矩阵不完整。
    assert!(
        matrix.cells().len() >= 10,
        "五阶段×fresh/resume 至少 10 主格(Plan 两入口另列),实际 {} 格",
        matrix.cells().len()
    );

    // 成功支持格具全部断言才 PASS;缺证据格保持 Unknown/Denied + reason。
    for cell in matrix.confirmed_cells() {
        assert_eq!(cell.provider, selected_provider);
        assert_eq!(cell.process_cwd, canonical_root);
        assert_eq!(cell.target, selected_member_worktree);
        assert!(!cell.exact_version.is_empty());
        assert_eq!(cell.audit_projection_digest, cell.frozen_projection_digest);
        assert_eq!(cell.native_resume_confirmed_id, cell.requested_resume_id);
        assert!(cell.argv_or_wire_capture_exists && cell.approval_and_tool_events_exist);
        assert!(cell.completed_product_artifact_exists);
        assert!(
            cell.entrypoint == "workspace_streaming_plan/split" || cell.entrypoint == "split_sync"
        );
        assert!(cell.run_ref_is_unique_within_entrypoint);
    }
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
    // Plan 两入口分列:不得只有单一入口冒充双栈证据。
    assert!(
        matrix
            .cells()
            .iter()
            .any(|cell| cell.entrypoint == ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT),
        "缺少 workspace_streaming_plan/split 入口证据"
    );
    assert!(
        matrix
            .cells()
            .iter()
            .any(|cell| cell.entrypoint == ENTRYPOINT_SPLIT_SYNC),
        "缺少 split_sync 入口证据"
    );
}

/// 现场证据落点(计划 Task 10 Files):`cadence/reports/lc-gateway-multi-provider/
/// <provider>/matrix/`,provider 目录固定为 claude-code/codex/pi/kimi-code。
fn live_evidence_root(provider: &ProviderName) -> PathBuf {
    let directory = match provider {
        ProviderName::ClaudeCode => "claude-code",
        ProviderName::Codex => "codex",
        ProviderName::Pi => "pi",
        ProviderName::KimiCode => "kimi-code",
        ProviderName::Fake => "fake",
    };
    PathBuf::from("cadence/reports/lc-gateway-multi-provider")
        .join(directory)
        .join("matrix")
}

#[tokio::test]
#[ignore = "真实现场矩阵:LC_GATEWAY_E2E=1 且 Step 4 命令执行(需要真实 CLI/trust/#8 政策就绪)"]
async fn lcg_live_claude_five_stages_fresh_resume() {
    run_live_five_stage_matrix(ProviderName::ClaudeCode).await;
}

#[tokio::test]
#[ignore = "真实现场矩阵:LC_GATEWAY_E2E=1 且 Step 4 命令执行(需要真实 CLI/trust/#8 政策就绪)"]
async fn lcg_live_codex_five_stages_fresh_resume() {
    run_live_five_stage_matrix(ProviderName::Codex).await;
}

#[tokio::test]
#[ignore = "真实现场矩阵:LC_GATEWAY_E2E=1 且 Step 4 命令执行(需要真实 CLI/trust/#8 政策就绪)"]
async fn lcg_live_pi_five_stages_fresh_resume() {
    run_live_five_stage_matrix(ProviderName::Pi).await;
}

#[tokio::test]
#[ignore = "真实现场矩阵:LC_GATEWAY_E2E=1 且 Step 4 命令执行(需要真实 CLI/trust/#8 政策就绪)"]
async fn lcg_live_kimi_five_stages_fresh_resume() {
    run_live_five_stage_matrix(ProviderName::KimiCode).await;
}
