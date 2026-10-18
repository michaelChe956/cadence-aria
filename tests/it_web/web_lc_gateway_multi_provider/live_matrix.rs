//! Task 10a:证据结构单测 + 四家 `#[ignore]` 真实现场测试。
//!
//! - `lcg_t10_evidence_requires_exact_version_wire_and_native_confirmation`
//!   只验证证据结构:缺字段记录必须被拒绝(两阶段红绿的 RED 锚点);
//! - 四个 `lcg_live_*` 为真实测试,默认 `#[ignore]`;执行时必须显式开关
//!   `LC_GATEWAY_E2E=1` 且 `--ignored` 命中,开关缺失要失败而非 return 成功
//!   (Step 4 四命令归 controller 现场执行,10a 只交 harness)。

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
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
        native_resume_reattached: false,
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
        provider_pid: Some("48017".to_string()),
        pid_unavailable_reason: None,
        provider_spawn_count: 0,
        session_projection_digest: "sha256:session-projection-fixture".to_string(),
        provider_events: vec![serde_json::json!({
            "ts": "2026-10-04T00:00:00.000Z",
            "event": {"type": "tool_call", "tool": "Read", "native_session_id": "native-session-fixture"}
        })],
        execution_origin: "full_chain".to_string(),
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
    // kimi-9(r10):reattached(在途 coder run 重挂)是 resume 格第二合法
    // 会话引用后开全新会话是产品冻结设计;重挂证据见
    // coding_resume_native_reattach_tests)。
    let mut resume_wrong_native_reattached = baseline.clone();
    resume_wrong_native_reattached.fresh_or_resume = RESUME.to_string();
    resume_wrong_native_reattached.requested_resume_id = Some("native-session-fixture".to_string());
    resume_wrong_native_reattached.native_resume_confirmed_id =
        Some("other-native-session".to_string());
    resume_wrong_native_reattached.native_resume_reattached = true;
    assert!(
        resume_wrong_native_reattached
            .validate_against(&provider, &canonical_root, &member_worktree)
            .is_ok(),
        "kimi-9 形:在途 coder run 重挂(reattached)的 resume 格必须放行"
    );
    // reattached 不是缺证据的豁免:缺 confirmed id 仍拒绝。
    let mut resume_requested_only_reattached = baseline.clone();
    resume_requested_only_reattached.fresh_or_resume = RESUME.to_string();
    resume_requested_only_reattached.requested_resume_id =
        Some("native-session-fixture".to_string());
    resume_requested_only_reattached.native_resume_confirmed_id = None;
    resume_requested_only_reattached.native_resume_reattached = true;
    expect_rejected(
        resume_requested_only_reattached,
        &provider,
        &canonical_root,
        &member_worktree,
        "reattached 不豁免缺失的原生恢复确认 id",
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
    let mut bad_stage = baseline.clone();
    bad_stage.stage = "deploy".to_string();
    expect_rejected(
        bad_stage,
        &provider,
        &canonical_root,
        &member_worktree,
        "stage 不在五阶段枚举内",
    );

    // ---- F3:PID 可追溯面——缺 PID 且无不可达说明必须拒绝;
    // 有明确不可达说明(streaming 入口无 stream log 目录)则如实放行。----
    let mut missing_pid = baseline.clone();
    missing_pid.provider_pid = None;
    missing_pid.pid_unavailable_reason = None;
    expect_rejected(
        missing_pid,
        &provider,
        &canonical_root,
        &member_worktree,
        "缺 provider PID 且无不可达说明",
    );
    let mut pid_unavailable = baseline.clone();
    pid_unavailable.provider_pid = None;
    pid_unavailable.pid_unavailable_reason =
        Some("该入口未由生产路径提供 provider stream log 目录".to_string());
    assert!(
        pid_unavailable
            .validate_against(&provider, &canonical_root, &member_worktree)
            .is_ok(),
        "PID 不可达但已如实标注说明的格必须通过(时间线以事件 ts+audit seq 追溯)"
    );

    // ---- F1:事件载荷面——provider-events.jsonl 无可核对事件必须拒绝。----
    let mut missing_events = baseline.clone();
    missing_events.provider_events = Vec::new();
    expect_rejected(
        missing_events,
        &provider,
        &canonical_root,
        &member_worktree,
        "缺事件载荷(provider-events.jsonl 无可核对事件)",
    );

    // ---- F5:split_sync resume 零 spawn 面——计数>0 即语义漂移,拒绝。----
    let mut split_resume_spawned = baseline.clone();
    split_resume_spawned.entrypoint = ENTRYPOINT_SPLIT_SYNC.to_string();
    split_resume_spawned.fresh_or_resume = RESUME.to_string();
    split_resume_spawned.requested_resume_id = Some("native-session-fixture".to_string());
    split_resume_spawned.native_resume_confirmed_id = Some("native-session-fixture".to_string());
    split_resume_spawned.provider_spawn_count = 2;
    expect_rejected(
        split_resume_spawned,
        &provider,
        &canonical_root,
        &member_worktree,
        "split_sync resume 必须零 spawn",
    );
}

/// F1(r58 深掏审计):按 RunMode 分档的主格完备性裁决(纯函数,
/// `lcg_t10_cell_completeness_by_run_mode` 钉死)。既有 `>= 10` 断言在
/// 续跑轮恒假(实际 8 格)——r56 EXIT=101 的直接 panic 点即旧断言:
/// - resume:story/design/plan/split 各恰 1 个 carried 承继格(引用来源
///   轮证据链,不重新计票)+ coding/review 真实执行各 fresh/resume
///   2 格 = 8;
/// - full/capture:五阶段真实执行×fresh/resume,plan 双入口分列
///   (streaming 10 + split_sync 2)= 12(各阶段失败早退路径同样恰好
///   2 格,不删格语义保持)。
/// 格数精确相等钉总数,阶段×入口×相位覆盖钉不缺格,双断言合一。
fn assert_matrix_cell_completeness(cells: &[EvidenceCell], mode: RunMode) {
    let expected = match mode {
        RunMode::ResumeFromPlanSnapshot => 8,
        RunMode::FullChain | RunMode::CapturePlanSnapshot => 12,
    };
    assert_eq!(
        cells.len(),
        expected,
        "矩阵主格数与 RunMode({mode:?})不匹配:resume=8(4 承继+coding/review 各 2),\
         full/capture=12(五阶段×fresh/resume,plan 双入口分列),实际 {} 格",
        cells.len()
    );
    if mode == RunMode::ResumeFromPlanSnapshot {
        // 承继完整性:四承继阶段各恰一格且为 carried 态。
        for stage in ["story", "design", "plan", "split"] {
            let stage_cells = cells.iter().filter(|cell| cell.stage == stage).count();
            assert_eq!(
                stage_cells, 1,
                "resume 轮承继阶段 {stage} 必须恰 1 个 carried 格,实际 {stage_cells} 格"
            );
            assert!(
                cells
                    .iter()
                    .any(|cell| cell.stage == stage && cell.capability_state == "carried"),
                "resume 轮 {stage} 格必须为 carried 态(引用来源轮证据链)"
            );
        }
    } else {
        // 全链完整性:五阶段×两相位全覆盖,plan 双入口分列。
        let streaming = ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT;
        for (stage, entrypoint) in [
            ("story", streaming),
            ("design", streaming),
            ("plan", streaming),
            ("plan", ENTRYPOINT_SPLIT_SYNC),
            ("coding", streaming),
            ("review", streaming),
        ] {
            for phase in [FRESH, RESUME] {
                assert!(
                    cells.iter().any(|cell| cell.stage == stage
                        && cell.entrypoint == entrypoint
                        && cell.fresh_or_resume == phase),
                    "full/capture 矩阵缺 {stage}/{entrypoint}/{phase} 主格"
                );
            }
        }
    }
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

    // 全 denied 不是矩阵通过:至少一格真实 Confirmed(成功支持格具全部
    // 断言才 PASS;全部缺证据=驱动/环境失败,不得以 reason 齐全冒充)。
    assert!(
        !matrix.confirmed_cells().is_empty(),
        "矩阵无任何 Confirmed 格(全部 denied/unknown):{}",
        matrix
            .unconfirmed_cells()
            .iter()
            .map(|cell| format!(
                "{}/{}/{}: {}",
                cell.stage,
                cell.entrypoint,
                cell.fresh_or_resume,
                cell.denied_reason.as_deref().unwrap_or("?")
            ))
            .collect::<Vec<_>>()
            .join(" | ")
    );

    // 五阶段×fresh/resume 主格完备:不合并隐藏,缺格即矩阵不完整。
    // F1:按 RunMode 分档(resume 轮 story/design/plan/split 为承继格,
    // 旧 `>= 10` 在续跑轮恒假);run_provider_matrix 入口已校验过 env,
    // 此处非法值=env 中途被改,fail loudly。
    assert_matrix_cell_completeness(
        matrix.cells(),
        super::snapshot::RunMode::from_env()
            .expect("LIVE_MATRIX_RUN_MODE 已在 run_provider_matrix 入口校验"),
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

// ---------------------------------------------------------------------------
// F1(r58 深掏审计):格数断言分档单测。r56 现场 resume 轮 8 格(4 承继+
// coding/review 各 2)撞旧 `>= 10` 断言 panic(EXIT=101)——分档后
// 合法 resume 形态放行,缺格/多格/形态错配均拒绝。
// ---------------------------------------------------------------------------

use super::snapshot::RunMode;

fn completeness_cell(
    provider: &ProviderName,
    canonical_root: &Path,
    member_worktree: &Path,
    stage: &str,
    entrypoint: &str,
    phase: &str,
) -> EvidenceCell {
    let mut cell = complete_baseline_cell(provider.clone(), canonical_root, member_worktree);
    cell.stage = stage.to_string();
    cell.entrypoint = entrypoint.to_string();
    cell.fresh_or_resume = phase.to_string();
    cell
}

fn carried_completeness_cell(
    provider: &ProviderName,
    canonical_root: &Path,
    member_worktree: &Path,
    stage: &str,
    entrypoint: &str,
) -> EvidenceCell {
    let mut cell = completeness_cell(
        provider,
        canonical_root,
        member_worktree,
        stage,
        entrypoint,
        FRESH,
    );
    cell.capability_state = "carried".to_string();
    cell
}

/// resume 形态(8 格:4 承继 + coding/review 各 fresh/resume)必须放行
/// ——r56 现场 panic 点;full/capture 形态(12 格,plan 双入口分列)
/// 同样放行。
#[test]
fn lcg_t10_cell_completeness_by_run_mode() {
    let provider = ProviderName::ClaudeCode;
    let canonical_root = PathBuf::from("/tmp/lcg-t10-matrix/canonical-root");
    let member_worktree = canonical_root.join("checkouts/alpha");
    let streaming = ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT;

    let mut resume_cells = vec![
        carried_completeness_cell(
            &provider,
            &canonical_root,
            &member_worktree,
            "story",
            streaming,
        ),
        carried_completeness_cell(
            &provider,
            &canonical_root,
            &member_worktree,
            "design",
            streaming,
        ),
        carried_completeness_cell(
            &provider,
            &canonical_root,
            &member_worktree,
            "plan",
            streaming,
        ),
        carried_completeness_cell(
            &provider,
            &canonical_root,
            &member_worktree,
            "split",
            ENTRYPOINT_SPLIT_SYNC,
        ),
    ];
    for stage in ["coding", "review"] {
        for phase in [FRESH, RESUME] {
            resume_cells.push(completeness_cell(
                &provider,
                &canonical_root,
                &member_worktree,
                stage,
                streaming,
                phase,
            ));
        }
    }
    assert_matrix_cell_completeness(&resume_cells, RunMode::ResumeFromPlanSnapshot);

    let mut full_cells = Vec::new();
    for (stage, entrypoint) in [
        ("story", streaming),
        ("design", streaming),
        ("plan", streaming),
        ("plan", ENTRYPOINT_SPLIT_SYNC),
        ("coding", streaming),
        ("review", streaming),
    ] {
        for phase in [FRESH, RESUME] {
            full_cells.push(completeness_cell(
                &provider,
                &canonical_root,
                &member_worktree,
                stage,
                entrypoint,
                phase,
            ));
        }
    }
    assert_matrix_cell_completeness(&full_cells, RunMode::FullChain);
    assert_matrix_cell_completeness(&full_cells, RunMode::CapturePlanSnapshot);
}

/// 缺承继格必须拒绝(resume 轮少一格=矩阵不完整,不得静默收窄验收)。
#[test]
#[should_panic(expected = "矩阵主格数与 RunMode")]
fn lcg_t10_cell_completeness_rejects_missing_carried_stage() {
    let provider = ProviderName::ClaudeCode;
    let canonical_root = PathBuf::from("/tmp/lcg-t10-matrix/canonical-root");
    let member_worktree = canonical_root.join("checkouts/alpha");
    let streaming = ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT;
    let mut cells = vec![
        carried_completeness_cell(
            &provider,
            &canonical_root,
            &member_worktree,
            "story",
            streaming,
        ),
        carried_completeness_cell(
            &provider,
            &canonical_root,
            &member_worktree,
            "design",
            streaming,
        ),
        carried_completeness_cell(
            &provider,
            &canonical_root,
            &member_worktree,
            "plan",
            streaming,
        ),
    ];
    for stage in ["coding", "review"] {
        for phase in [FRESH, RESUME] {
            cells.push(completeness_cell(
                &provider,
                &canonical_root,
                &member_worktree,
                stage,
                streaming,
                phase,
            ));
        }
    }
    // split 承继格缺失:先撞总数(7 != 8)或承继断言,均拒绝。
    assert_matrix_cell_completeness(&cells, RunMode::ResumeFromPlanSnapshot);
}

/// resume 形态(8 格)按 full 档校验必须拒绝——钉住旧 `>= 10` 断言的
/// 拒绝形态已被分档替代:8 格只在 resume 档合法。
#[test]
#[should_panic(expected = "矩阵主格数与 RunMode")]
fn lcg_t10_cell_completeness_rejects_resume_shape_under_full_mode() {
    let provider = ProviderName::ClaudeCode;
    let canonical_root = PathBuf::from("/tmp/lcg-t10-matrix/canonical-root");
    let member_worktree = canonical_root.join("checkouts/alpha");
    let streaming = ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT;
    let mut cells = Vec::new();
    for stage in ["story", "design", "plan", "coding", "review"] {
        for phase in [FRESH, RESUME] {
            cells.push(completeness_cell(
                &provider,
                &canonical_root,
                &member_worktree,
                stage,
                streaming,
                phase,
            ));
        }
    }
    assert_matrix_cell_completeness(&cells, RunMode::FullChain);
}

// ---------------------------------------------------------------------------
// 批2 Task 22(b45):Kimi 真实宿主 CHECK 证据 validator(结构单测)+ 现场
// 测试 `b45_live_kimi_host_check`。
//
// validator 拒绝形态(计划 Step 1):缺实际版本/argv/cwd/exit/count、
// Podman 只有 version 无 build/run、长命令未越 120s 或未越外层角色
// idle 界、少职责或缺活动状态、缺终止单格/前后快照——均 FAIL。
// ---------------------------------------------------------------------------

/// 长静默 CHECK 越界下限(计划:长命令未越 120s 即 FAIL)。
pub(crate) const B45_LONG_SILENT_MIN_SECS: u64 = 120;

/// b45 必需职责(计划 Step 3:Author/规划/Coder/Reviewer)。
pub(crate) const B45_REQUIRED_DUTIES: [&str; 4] = ["author", "planner", "coder", "reviewer"];

/// CHECK 工具证据格:实际版本/argv/cwd/exit/count/活动状态/资源。
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
pub(crate) struct B45ToolRecord {
    pub tool: String,
    pub facet: String,
    pub exact_version: String,
    pub planned_command: String,
    pub actual_command: String,
    pub argv: Vec<String>,
    pub cwd: String,
    pub exit_code: Option<i32>,
    pub test_count: Option<u64>,
    pub output_excerpt: String,
    pub terminal_lifecycle: Vec<String>,
    pub resource_ids: Vec<String>,
}

/// 职责证据格:合法终态 + ACP 原始活动。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub(crate) struct B45DutyRecord {
    pub duty: String,
    pub terminal_status: Option<String>,
    pub native_activity: String,
}

/// 静默长 CHECK 记录:静默跨 120s 且越外层角色 idle 界,未被误杀。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub(crate) struct B45LongSilentRecord {
    pub tool: String,
    pub silent_gap_secs: u64,
    pub role_idle_boundary_secs: u64,
    pub completed: bool,
    pub stall_reason_codes: Vec<String>,
}

/// 总 deadline/用户取消真实终止单格。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub(crate) struct B45TerminationRecord {
    pub trigger: String,
    pub attempt_id: String,
    pub terminal_status: String,
    pub provider_pid_gone: bool,
    pub lingering_fixture_containers: Vec<String>,
}

/// b45 现场证据全集。
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
pub(crate) struct B45HostCheckEvidence {
    pub provider_exact_version: String,
    pub tools: Vec<B45ToolRecord>,
    pub duties: Vec<B45DutyRecord>,
    pub long_silent: Option<B45LongSilentRecord>,
    pub termination: Option<B45TerminationRecord>,
    pub snapshot_before: String,
    pub snapshot_after: String,
    pub coder_dirty_preserved: bool,
}

/// b45 证据 validator:返回全部缺项(逐条列明,不首错即停)。
pub(crate) fn validate_b45_host_check_evidence(
    evidence: &B45HostCheckEvidence,
) -> Result<(), Vec<String>> {
    let mut failures: Vec<String> = Vec::new();
    if evidence.provider_exact_version.trim().is_empty() {
        failures.push("缺实际 CLI 版本(kimi exact_version)".to_string());
    }
    // ---- 三工具面:Maven 容器 mvn -B test / npm ci+test+build / Podman build+run ----
    let maven_facets: Vec<_> = evidence
        .tools
        .iter()
        .filter(|record| record.tool == "maven")
        .map(|record| record.facet.as_str())
        .collect();
    if !maven_facets.iter().any(|facet| *facet == "test") {
        failures.push("缺 Maven mvn -B test 工具格".to_string());
    }
    for npm_facet in ["ci", "test", "build"] {
        if !evidence
            .tools
            .iter()
            .any(|record| record.tool == "npm" && record.facet == npm_facet)
        {
            failures.push(format!("缺 npm {npm_facet} 工具格(lock 消费链)"));
        }
    }
    let podman_facets: Vec<_> = evidence
        .tools
        .iter()
        .filter(|record| record.tool == "podman")
        .map(|record| record.facet.as_str())
        .collect();
    if podman_facets.is_empty() {
        failures.push("缺 Podman 工具格".to_string());
    } else if !podman_facets.iter().any(|facet| *facet == "build")
        || !podman_facets.iter().any(|facet| *facet == "run")
    {
        failures.push(format!(
            "Podman 只有 {podman_facets:?} 无 build+run(仅 version 不算容器 CHECK)"
        ));
    }
    // ---- 逐格要素:版本/argv/cwd/exit/count(test 面)/活动状态 ----
    for record in &evidence.tools {
        let label = format!("{}/{}", record.tool, record.facet);
        if record.exact_version.trim().is_empty() {
            failures.push(format!("{label} 缺实际工具版本"));
        }
        if record.argv.is_empty() {
            failures.push(format!("{label} 缺 argv"));
        }
        if record.cwd.trim().is_empty() {
            failures.push(format!("{label} 缺 cwd"));
        }
        if record.exit_code.is_none() {
            failures.push(format!("{label} 缺退出码"));
        }
        let is_test_facet =
            (record.tool == "maven" && record.facet == "test") || (record.tool == "npm" && record.facet == "test");
        if is_test_facet && record.test_count.is_none() {
            failures.push(format!("{label} 缺测试数量"));
        }
        if record.terminal_lifecycle.is_empty() {
            failures.push(format!("{label} 缺活动状态(terminal 生命周期)"));
        }
    }
    // ---- 职责 + 活动状态 ----
    for duty in B45_REQUIRED_DUTIES {
        match evidence.duties.iter().find(|record| record.duty == duty) {
            None => failures.push(format!("缺职责 {duty}")),
            Some(record) => {
                if record.terminal_status.as_deref().unwrap_or("").trim().is_empty() {
                    failures.push(format!("职责 {duty} 缺合法终态"));
                }
                if record.native_activity.trim().is_empty() {
                    failures.push(format!("职责 {duty} 缺 ACP 原始活动"));
                }
            }
        }
    }
    // ---- 静默长 CHECK ----
    match &evidence.long_silent {
        None => failures.push("缺静默长 CHECK 记录".to_string()),
        Some(record) => {
            if record.silent_gap_secs < B45_LONG_SILENT_MIN_SECS {
                failures.push(format!(
                    "长命令未越 {B45_LONG_SILENT_MIN_SECS}s(实测 {}s)",
                    record.silent_gap_secs
                ));
            }
            if record.role_idle_boundary_secs >= record.silent_gap_secs {
                failures.push(format!(
                    "静默 {}s 未越外层角色 idle 界 {}s",
                    record.silent_gap_secs, record.role_idle_boundary_secs
                ));
            }
            if !record.completed {
                failures.push("静默长 CHECK 未完成(被误杀或未达终态)".to_string());
            }
            if !record.stall_reason_codes.is_empty() {
                failures.push(format!(
                    "ACP stall/产品 idle 误杀残留原因码:{:?}",
                    record.stall_reason_codes
                ));
            }
        }
    }
    // ---- 总 deadline/用户取消终止单格 ----
    match &evidence.termination {
        None => failures.push("缺总deadline/用户取消真实终止单格".to_string()),
        Some(record) => {
            if record.terminal_status.trim().is_empty() {
                failures.push("终止单格缺 attempt 终态".to_string());
            }
            if !record.provider_pid_gone {
                failures.push("终止单格 provider 进程仍存活(未真实终止)".to_string());
            }
            if !record.lingering_fixture_containers.is_empty() {
                failures.push(format!(
                    "终止单格遗留 fixture 容器:{:?}",
                    record.lingering_fixture_containers
                ));
            }
        }
    }
    // ---- 前后快照 + Coder 工作保留 ----
    if evidence.snapshot_before.trim().is_empty() || evidence.snapshot_after.trim().is_empty() {
        failures.push("缺成员仓前后快照".to_string());
    }
    if !evidence.coder_dirty_preserved {
        failures.push("Coder 工作未保留(HEAD 未进且无 dirty)".to_string());
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures)
    }
}

/// 完整基线 fixture(validator 单测的过测样本;字段值模拟真实形态)。
fn b45_complete_baseline_evidence() -> B45HostCheckEvidence {
    let tool = |tool: &str, facet: &str, version: &str, test_count: Option<u64>| B45ToolRecord {
        tool: tool.to_string(),
        facet: facet.to_string(),
        exact_version: version.to_string(),
        planned_command: format!("planned {tool} {facet}"),
        actual_command: format!("actual {tool} {facet}"),
        argv: vec![tool.to_string(), facet.to_string()],
        cwd: "/worktree/fixtures".to_string(),
        exit_code: Some(0),
        test_count,
        output_excerpt: "T22 excerpt".to_string(),
        terminal_lifecycle: vec!["Started".to_string(), "Running".to_string(), "Completed".to_string()],
        resource_ids: vec!["terminal-1".to_string()],
    };
    B45HostCheckEvidence {
        provider_exact_version: "2.2.0".to_string(),
        tools: vec![
            tool("maven", "test", "Apache Maven 3.9.x (container)", Some(2)),
            tool("npm", "ci", "11.13.0", None),
            tool("npm", "test", "11.13.0", Some(2)),
            tool("npm", "build", "11.13.0", None),
            tool("podman", "build", "6.1.1", None),
            tool("podman", "run", "6.1.1", None),
        ],
        duties: B45_REQUIRED_DUTIES
            .iter()
            .map(|duty| B45DutyRecord {
                duty: duty.to_string(),
                terminal_status: Some("confirmed".to_string()),
                native_activity: format!("{duty} native session + 12 events"),
            })
            .collect(),
        long_silent: Some(B45LongSilentRecord {
            tool: "maven".to_string(),
            silent_gap_secs: 151,
            role_idle_boundary_secs: 90,
            completed: true,
            stall_reason_codes: Vec::new(),
        }),
        termination: Some(B45TerminationRecord {
            trigger: "user_cancel".to_string(),
            attempt_id: "attempt_cancel".to_string(),
            terminal_status: "aborted".to_string(),
            provider_pid_gone: true,
            lingering_fixture_containers: Vec::new(),
        }),
        snapshot_before: "head=aaa porcelain=fixtures".to_string(),
        snapshot_after: "head=bbb porcelain=src/lib.rs".to_string(),
        coder_dirty_preserved: true,
    }
}

/// Step 1 红测锚点:缺项 fixture 定向确认拒绝(每缺一项一拒,完整基线放行)。
#[test]
fn b45_host_check_validator_rejects_incomplete_evidence() {
    let complete = b45_complete_baseline_evidence();
    assert!(
        validate_b45_host_check_evidence(&complete).is_ok(),
        "完整基线必须通过:{:?}",
        validate_b45_host_check_evidence(&complete)
    );

    let mut missing_cli_version = complete.clone();
    missing_cli_version.provider_exact_version = String::new();
    let expect_reject = |evidence: B45HostCheckEvidence, needle: &str| {
        let Err(failures) = validate_b45_host_check_evidence(&evidence) else {
            panic!("缺项变体必须被拒绝(期望含 {needle})");
        };
        assert!(
            failures.iter().any(|failure| failure.contains(needle)),
            "拒绝清单须含 {needle}:{failures:?}"
        );
    };
    expect_reject(missing_cli_version, "CLI 版本");

    let mut missing_tool_version = complete.clone();
    missing_tool_version.tools[0].exact_version = String::new();
    expect_reject(missing_tool_version, "缺实际工具版本");

    let mut missing_argv = complete.clone();
    missing_argv.tools[1].argv = Vec::new();
    expect_reject(missing_argv, "缺 argv");

    let mut missing_cwd = complete.clone();
    missing_cwd.tools[2].cwd = String::new();
    expect_reject(missing_cwd, "缺 cwd");

    let mut missing_exit = complete.clone();
    missing_exit.tools[3].exit_code = None;
    expect_reject(missing_exit, "缺退出码");

    let mut missing_count = complete.clone();
    missing_count.tools[0].test_count = None;
    expect_reject(missing_count, "缺测试数量");

    let mut missing_lifecycle = complete.clone();
    missing_lifecycle.tools[4].terminal_lifecycle = Vec::new();
    expect_reject(missing_lifecycle, "缺活动状态");

    // Podman 只有 version 无 build/run。
    let mut podman_version_only = complete.clone();
    podman_version_only.tools.retain(|record| record.tool != "podman");
    podman_version_only.tools.push(tool_record(
        "podman",
        "version",
        "6.1.1",
        None,
    ));
    expect_reject(podman_version_only, "无 build+run");

    // npm 缺 build。
    let mut npm_missing_build = complete.clone();
    npm_missing_build
        .tools
        .retain(|record| !(record.tool == "npm" && record.facet == "build"));
    expect_reject(npm_missing_build, "npm build");

    // 长命令未越 120s。
    let mut short_silent = complete.clone();
    short_silent.long_silent = Some(B45LongSilentRecord {
        tool: "maven".to_string(),
        silent_gap_secs: 95,
        role_idle_boundary_secs: 90,
        completed: true,
        stall_reason_codes: Vec::new(),
    });
    expect_reject(short_silent, "未越 120");

    // 静默未越外层角色 idle 界(界>静默)。
    let mut not_crossing_idle = complete.clone();
    not_crossing_idle.long_silent = Some(B45LongSilentRecord {
        tool: "maven".to_string(),
        silent_gap_secs: 150,
        role_idle_boundary_secs: 300,
        completed: true,
        stall_reason_codes: Vec::new(),
    });
    expect_reject(not_crossing_idle, "未越外层角色 idle 界");

    // watchdog 误杀残留。
    let mut stalled = complete.clone();
    if let Some(record) = stalled.long_silent.as_mut() {
        record.stall_reason_codes = vec!["provider_idle_stall".to_string()];
    }
    expect_reject(stalled, "误杀");

    // 缺职责。
    let mut missing_duty = complete.clone();
    missing_duty.duties.retain(|record| record.duty != "reviewer");
    expect_reject(missing_duty, "缺职责 reviewer");

    // 职责缺活动状态。
    let mut duty_no_activity = complete.clone();
    if let Some(record) = duty_no_activity
        .duties
        .iter_mut()
        .find(|record| record.duty == "coder")
    {
        record.native_activity = String::new();
    }
    expect_reject(duty_no_activity, "ACP 原始活动");

    // 缺终止单格 / 终止单格进程仍活。
    let mut missing_termination = complete.clone();
    missing_termination.termination = None;
    expect_reject(missing_termination, "终止单格");
    let mut pid_alive = complete.clone();
    if let Some(record) = pid_alive.termination.as_mut() {
        record.provider_pid_gone = false;
    }
    expect_reject(pid_alive, "未真实终止");

    // 缺快照 / 工作未保留。
    let mut missing_snapshot = complete.clone();
    missing_snapshot.snapshot_after = String::new();
    expect_reject(missing_snapshot, "前后快照");
    let mut work_lost = complete.clone();
    work_lost.snapshot_after = "head=aaa porcelain=".to_string();
    work_lost.coder_dirty_preserved = false;
    expect_reject(work_lost, "未保留");
}

/// 单测内构造工具格的便捷入口(与闭包捕获无关,独立函数便于复用)。
fn tool_record(tool: &str, facet: &str, version: &str, test_count: Option<u64>) -> B45ToolRecord {
    B45ToolRecord {
        tool: tool.to_string(),
        facet: facet.to_string(),
        exact_version: version.to_string(),
        planned_command: format!("planned {tool} {facet}"),
        actual_command: format!("actual {tool} {facet}"),
        argv: vec![tool.to_string(), facet.to_string()],
        cwd: "/worktree/fixtures".to_string(),
        exit_code: Some(0),
        test_count,
        output_excerpt: "T22 excerpt".to_string(),
        terminal_lifecycle: vec!["Started".to_string(), "Running".to_string(), "Completed".to_string()],
        resource_ids: vec!["terminal-1".to_string()],
    }
}

/// 从 run_b45_kimi_host_check 的 JSON 聚合构造 typed 证据。
fn b45_evidence_from_run(run: &Value) -> B45HostCheckEvidence {
    let text = |pointer: &str| -> String {
        run.pointer(pointer)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let versions = run.pointer("/fixtures/versions").cloned().unwrap_or(Value::Null);
    let version_of = |tool: &str| -> String {
        let key = match tool {
            "maven" => "maven_container",
            other => other,
        };
        versions
            .pointer(&format!("/{key}/stdout"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .lines()
            .next()
            .unwrap_or_default()
            .to_string()
    };

    // ---- 工具格:verification-command-evidence 的实际执行条目 ----
    let mut tools: Vec<B45ToolRecord> = Vec::new();
    let evidence_rows = run
        .pointer("/coding_main/verification_evidence/body/evidence")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let lifecycle_rows = run
        .pointer("/coding_main/role_run_rows")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for row in &evidence_rows {
        let Some(actual) = row.get("actual_command").and_then(Value::as_str) else {
 continue;
        };
        let (tool, facet) = b45_classify_command(actual);
        if tool.is_empty() {
            continue;
        }
        let argv = actual
            .split_whitespace()
            .map(str::to_string)
            .collect::<Vec<_>>();
        let lifecycle = b45_terminal_lifecycle_for(&lifecycle_rows, actual);
        tools.push(B45ToolRecord {
            exact_version: version_of(&tool),
            tool,
            facet,
            planned_command: row
                .get("planned_command")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            actual_command: actual.to_string(),
            argv,
            cwd: row
                .get("actual_cwd")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            exit_code: row.get("exit_code").and_then(Value::as_i64).map(|code| code as i32),
            test_count: row.get("test_execution_count").and_then(Value::as_u64),
            output_excerpt: row
                .get("raw_output_excerpt")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            terminal_lifecycle: lifecycle,
            resource_ids: Vec::new(),
        });
    }
    // r67(T22 r5 现场):verification-command-evidence 端点面向 triage 窗口
    //(active unit 在 waiting_for_human 已 completed→422 plan_revision_
    // unresolvable,r5 实拍),端点空/失败时以 role_run_rows 的 kimi_terminal_
    // Started/Completed 审计对构建工具格——同为产品 durable 审计(命令/退出
    // 码/生命周期),测试计数从 issue 钉定的 CHECK-EVIDENCE.log 落盘面解析。
    if tools.is_empty() {
        let check_log = run
            .pointer("/coding_main/check_evidence_log")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let maven_count = check_log
            .lines()
            .filter_map(|line| {
                let rest = line.split("Tests run:").nth(1)?;
                rest.split(',')
                    .next()
                    .and_then(|count| count.trim().parse::<u64>().ok())
            })
            .max();
        let npm_count = check_log
            .lines()
            .filter_map(|line| {
                let rest = line.split("passed").next()?;
                rest.trim()
                    .rsplit(|ch: char| !ch.is_ascii_digit())
                    .next()
                    .and_then(|count| count.parse::<u64>().ok())
            })
            .filter(|count| *count > 0)
            .max();
        // 同一 terminal(事件 id)的 Started 携带命令、Completed 携带退出码:
        // 按 id 配对后分类建格(同 facet 首条为准,后续复跑只并入生命周期)。
        let mut exits: std::collections::BTreeMap<String, Option<i64>> =
            std::collections::BTreeMap::new();
        for row in &lifecycle_rows {
            let Some(payload) = row.get("payload") else { continue };
            let Some(event_id) = payload.get("event_id").and_then(Value::as_str) else {
                continue;
            };
            if !event_id.starts_with("kimi_terminal_") {
                continue;
            }
            if matches!(
                payload.get("status").and_then(Value::as_str),
                Some("Completed") | Some("Failed") | Some("Aborted")
            ) {
                exits.insert(
                    event_id.to_string(),
                    payload.get("exit_code").and_then(Value::as_i64),
                );
            }
        }
        for row in &lifecycle_rows {
            let Some(payload) = row.get("payload") else { continue };
            if payload.get("status").and_then(Value::as_str) != Some("Started") {
                continue;
            }
            let Some(event_id) = payload.get("event_id").and_then(Value::as_str) else {
                continue;
            };
            let Some(command) = payload.get("command").and_then(Value::as_str) else {
                continue;
            };
            let (tool, facet) = match b45_classify_command(command) {
                classified if !classified.0.is_empty() => classified,
                // r71(T22 r10 现场):kimi 把六条 CHECK 包进自写脚本单
                // terminal 执行(bash /tmp/run-checks-t22.sh,无工具 token)
                //——脚本文件在宿主在盘,读内容后逐行分类;行级命令共享该
                // terminal 的退出码(脚本 set -u 整体退出)。
                _ => {
                    let script_facets = command
                        .split_whitespace()
                        .find(|token| token.ends_with(".sh"))
                        .and_then(|path| std::fs::read_to_string(path).ok())
                        .map(|script| {
                            script
                                .lines()
                                .map(b45_classify_command)
                                .filter(|(tool, facet)| {
                                    !tool.is_empty() && facet != "version"
                                })
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    for (tool, facet) in script_facets {
                        if !tools
                            .iter()
                            .any(|record| record.tool == tool && record.facet == facet)
                        {
                            let is_test_facet = (tool == "maven" && facet == "test")
                                || (tool == "npm" && facet == "test");
                            let lifecycle = b45_terminal_lifecycle_for(
                                &lifecycle_rows,
                                command,
                            );
                            let test_count = if is_test_facet {
                                if tool == "maven" {
                                    maven_count
                                } else {
                                    npm_count
                                }
                            } else {
                                None
                            };
                            tools.push(B45ToolRecord {
                                exact_version: version_of(&tool),
                                tool,
                                facet,
                                planned_command: String::new(),
                                actual_command: command.to_string(),
                                argv: command
                                    .split_whitespace()
                                    .map(str::to_string)
                                    .collect::<Vec<_>>(),
                                cwd: command
                                    .split("cd ")
                                    .nth(1)
                                    .and_then(|rest| {
                                        rest.trim()
                                            .trim_start_matches('\'')
                                            .split('\'')
                                            .next()
                                    })
                                    .unwrap_or_default()
                                    .to_string(),
                                exit_code: exits
                                    .get(event_id)
                                    .and_then(|code| *code)
                                    .map(|code| code as i32),
                                test_count,
                                output_excerpt: String::new(),
                                terminal_lifecycle: lifecycle,
                                resource_ids: Vec::new(),
                            });
                        }
                    }
                    continue;
                }
            };
            if tool.is_empty() || facet == "version" {
                continue;
            }
            let is_test_facet =
                (tool == "maven" && facet == "test") || (tool == "npm" && facet == "test");
            let lifecycle = b45_terminal_lifecycle_for(&lifecycle_rows, command);
            if let Some(existing) = tools
                .iter()
                .position(|record| record.tool == tool && record.facet == facet)
            {
                tools[existing].terminal_lifecycle.extend(lifecycle);
                continue;
            }
            let cwd = command
                .split("cd ")
                .nth(1)
                .and_then(|rest| rest.trim().trim_start_matches('\'').split('\'').next())
                .unwrap_or_default()
                .to_string();
            let test_count = if is_test_facet {
                if tool == "maven" {
                    maven_count
                } else {
                    npm_count
                }
            } else {
                None
            };
            tools.push(B45ToolRecord {
                exact_version: version_of(&tool),
                tool,
                facet,
                planned_command: String::new(),
                actual_command: command.to_string(),
                argv: command
                    .split_whitespace()
                    .map(str::to_string)
                    .collect::<Vec<_>>(),
                cwd,
                exit_code: exits
                    .get(event_id)
                    .and_then(|code| *code)
                    .map(|code| code as i32),
                test_count,
                output_excerpt: String::new(),
                terminal_lifecycle: lifecycle,
                resource_ids: Vec::new(),
            });
        }
    }
    // 资源(T5 登记)附到工具格:aria.scope_id/run_id 标签的资源清单。
    let resource_ids = run
        .pointer("/provider_resources")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let record = entry.get("record")?;
                    let labels = record.get("labels").or_else(|| record.pointer("/reserved_labels"))?;
                    let has_aria_label = labels
                        .as_object()
                        .is_some_and(|map| map.keys().any(|key| key.starts_with("aria.")));
                    has_aria_label.then(|| {
                        record
                            .get("resource_id")
                            .or_else(|| record.pointer("/identity/id"))
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string()
                    })
                })
                .filter(|id| !id.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for record in &mut tools {
        record.resource_ids = resource_ids.clone();
    }

    // ---- 职责 ----
    let confirmed_cell = |cells: Option<&Value>| -> Option<String> {
        cells.and_then(Value::as_array).and_then(|rows| {
            rows.iter()
                .find(|cell| cell.get("capability_state").and_then(Value::as_str) == Some("confirmed"))
                .and_then(|cell| cell.get("native_session_id").and_then(Value::as_str))
                .map(str::to_string)
        })
    };
    let story_confirmed = confirmed_cell(run.pointer("/duties/author/story_cells"));
    let design_confirmed = confirmed_cell(run.pointer("/duties/author/design_cells"));
    let plan_confirmed = confirmed_cell(run.pointer("/duties/planner/plan_cells"));
    let review_confirmed = confirmed_cell(run.pointer("/duties/reviewer/review_cells"));
    let execution_rows = lifecycle_rows
        .iter()
        .filter(|row| {
            row.get("event_type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind.contains("ExecutionEvent"))
        })
        .count();
    let coder_terminal = run
        .pointer("/duties/coder/terminal")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let mut duties = Vec::new();
    if let Some(native) = story_confirmed.or(design_confirmed) {
        duties.push(B45DutyRecord {
            duty: "author".to_string(),
            terminal_status: Some("confirmed".to_string()),
            native_activity: format!("native_session={native}"),
        });
    } else {
        duties.push(B45DutyRecord {
            duty: "author".to_string(),
            terminal_status: None,
            native_activity: String::new(),
        });
    }
    if let Some(native) = plan_confirmed {
        duties.push(B45DutyRecord {
            duty: "planner".to_string(),
            terminal_status: Some("confirmed".to_string()),
            native_activity: format!("native_session={native}"),
        });
    } else {
        duties.push(B45DutyRecord {
            duty: "planner".to_string(),
            terminal_status: None,
            native_activity: String::new(),
        });
    }
    duties.push(B45DutyRecord {
        duty: "coder".to_string(),
        terminal_status: if coder_terminal.is_empty() { None } else { Some(coder_terminal) },
        native_activity: format!(
            "execution_events={execution_rows} tool_events={} permission_events={}",
            run.pointer("/coding_main/tool_events").and_then(Value::as_u64).unwrap_or(0),
            run.pointer("/coding_main/permission_events").and_then(Value::as_u64).unwrap_or(0),
        ),
    });
    if let Some(native) = review_confirmed {
        duties.push(B45DutyRecord {
            duty: "reviewer".to_string(),
            terminal_status: Some("confirmed".to_string()),
            native_activity: format!(
                "native_session={native} windows={}",
                run
                    .pointer("/reviewer_windows")
                    .and_then(Value::as_array)
                    .map(Vec::len)
                    .unwrap_or(0)
            ),
        });
    } else {
        duties.push(B45DutyRecord {
            duty: "reviewer".to_string(),
            terminal_status: None,
            native_activity: String::new(),
        });
    }

    // ---- 静默长 CHECK:ExecutionEvent 行间最大间隔 + Maven 命令时长 ----
    let parse_ts = |value: &Value| -> Option<chrono::DateTime<chrono::Utc>> {
        value
            .as_str()
            .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
            .map(|time| time.with_timezone(&chrono::Utc))
    };
    let mut silent_gap_secs = 0u64;
    let mut previous: Option<chrono::DateTime<chrono::Utc>> = None;
    for row in &lifecycle_rows {
        let Some(created_at) = row.get("created_at") else { continue };
        let Some(timestamp) = parse_ts(created_at) else {
            previous = None;
            continue;
        };
        if let Some(prior) = previous {
            let gap = timestamp
                .signed_duration_since(prior)
                .num_seconds()
                .max(0) as u64;
            silent_gap_secs = silent_gap_secs.max(gap);
        }
        previous = Some(timestamp);
    }
    // Maven 命令自身时长(Started→终态 同 terminal 生命周期差)。
    let maven_lifecycle = b45_terminal_lifecycle_spans(&lifecycle_rows, "mvn");
    silent_gap_secs = silent_gap_secs.max(maven_lifecycle.max_span_secs);
    let maven_completed = tools
        .iter()
        .any(|record| record.tool == "maven" && record.exit_code == Some(0));
    let stall_reason_codes = lifecycle_rows
        .iter()
        .filter_map(|row| row.pointer("/payload/reason_code").and_then(Value::as_str))
        .filter(|code| {
            code.contains("idle_stall") || code.contains("provider_stalled")
        })
        .map(str::to_string)
        .collect::<Vec<_>>();
    let long_silent = Some(B45LongSilentRecord {
        tool: "maven".to_string(),
        silent_gap_secs,
        role_idle_boundary_secs: run
            .get("coder_idle_boundary_secs")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        completed: maven_completed,
        stall_reason_codes,
    });

    // ---- 终止单格 ----
    let termination = run.pointer("/cancel_cell").map(|cell| B45TerminationRecord {
        trigger: "user_cancel".to_string(),
        attempt_id: cell
            .get("attempt_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        terminal_status: cell
            .get("terminal_status")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        provider_pid_gone: !cell
            .get("provider_pid_still_alive")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        lingering_fixture_containers: cell
            .get("lingering_fixture_containers")
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| entry.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
    });

    // ---- 快照与工作保留 ----
    let head_before = text("/snapshot_before/head");
    let head_after = text("/snapshot_after/head");
    let porcelain_after = text("/snapshot_after/porcelain");
    B45HostCheckEvidence {
        provider_exact_version: text("/provider/exact_version"),
        tools,
        duties,
        long_silent,
        termination,
        snapshot_before: format!("head={head_before} porcelain={}", text("/snapshot_before/porcelain")),
        snapshot_after: format!("head={head_after} porcelain={porcelain_after}"),
        coder_dirty_preserved: head_after != head_before || !porcelain_after.trim().is_empty(),
    }
}

/// 命令族分类(podman/npm/maven + facet)。
fn b45_classify_command(command: &str) -> (String, String) {
    let tokens: Vec<&str> = command.split_whitespace().collect();
    // 首 token 是 /bin/bash;取首个工具 token(podman/npm/mvn)按其子命令分类。
    let tool_index = tokens
        .iter()
        .position(|token| matches!(*token, "podman" | "npm" | "mvn"))
        .unwrap_or(0);
    let head = tokens.get(tool_index).copied().unwrap_or_default();
    let sub = tokens.get(tool_index + 1).copied().unwrap_or_default();
    let third = tokens.get(tool_index + 2).copied().unwrap_or_default();
    match head {
        "podman" => {
            match sub {
                "build" => ("podman".to_string(), "build".to_string()),
                "run" => {
                    if tokens.iter().any(|token| *token == "mvn") {
                        ("maven".to_string(), "test".to_string())
                    } else {
                        ("podman".to_string(), "run".to_string())
                    }
                }
                "version" => ("podman".to_string(), "version".to_string()),
                _ => ("podman".to_string(), sub.to_string()),
            }
        }
        "npm" => {
            let facet = match sub {
                "ci" => "ci",
                "test" => "test",
                "run" if third == "build" => "build",
                other => other,
            };
            ("npm".to_string(), facet.to_string())
        }
        "mvn" => ("maven".to_string(), "test".to_string()),
        _ => (String::new(), String::new()),
    }
}

/// 从 role-run 行提取匹配命令的 terminal 生命周期状态序列。
fn b45_terminal_lifecycle_for(rows: &[Value], command: &str) -> Vec<String> {
    let (_, facet) = b45_classify_command(command);
    let matcher = |row_command: &str| -> bool {
        if command.len() > 12 {
            row_command == command
        } else {
            row_command.contains(&facet) && !facet.is_empty()
        }
    };
    rows.iter()
        .filter(|row| {
            row.get("event_type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind.contains("ExecutionEvent"))
        })
        .filter(|row| {
            row.pointer("/payload/event_id")
                .and_then(Value::as_str)
                .is_some_and(|id| id.starts_with("kimi_terminal_"))
        })
        .filter(|row| {
            row.pointer("/payload/command")
                .and_then(Value::as_str)
                .map(matcher)
 .unwrap_or(false)
                || row
                    .pointer("/payload/detail")
                    .and_then(Value::as_str)
                    .map(|detail| detail.contains(&facet) && !facet.is_empty())
                    .unwrap_or(false)
        })
        .filter_map(|row| row.pointer("/payload/status").and_then(Value::as_str).map(str::to_string))
        .collect()
}

/// 同 terminal 生命周期跨度(Started→终态)对含关键字的命令的最大秒差。
fn b45_terminal_lifecycle_spans(rows: &[Value], keyword: &str) -> B45LifecycleSpans {
    let parse_ts = |value: Option<&Value>| -> Option<chrono::DateTime<chrono::Utc>> {
        value
            .and_then(Value::as_str)
            .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
            .map(|time| time.with_timezone(&chrono::Utc))
    };
    use std::collections::BTreeMap;
    let mut spans: BTreeMap<String, (Option<chrono::DateTime<chrono::Utc>>, Option<chrono::DateTime<chrono::Utc>>)> =
        BTreeMap::new();
    for row in rows {
        let Some(event_id) = row.pointer("/payload/event_id").and_then(Value::as_str) else {
 continue;
        };
        if !event_id.starts_with("kimi_terminal_") {
            continue;
        }
        let command_or_detail = row
            .pointer("/payload/command")
            .and_then(Value::as_str)
            .or_else(|| row.pointer("/payload/detail").and_then(Value::as_str))
            .unwrap_or_default();
        if !command_or_detail.contains(keyword) {
            continue;
        }
        let timestamp = parse_ts(row.get("created_at"));
        let status = row.pointer("/payload/status").and_then(Value::as_str).unwrap_or_default();
        let entry = spans.entry(event_id.to_string()).or_insert((None, None));
        if status == "Started" && timestamp.is_some() {
            entry.0 = timestamp;
        }
        if matches!(status, "Completed" | "Failed" | "Aborted") && timestamp.is_some() {
            entry.1 = timestamp;
        }
    }
    let max_span_secs = spans
        .values()
        .filter_map(|(start, end)| match (start, end) {
            (Some(start), Some(end)) => Some((start, end)),
            _ => None,
        })
        .map(|(start, end)| end.signed_duration_since(start).num_seconds().max(0) as u64)
        .max()
        .unwrap_or(0);
    B45LifecycleSpans { max_span_secs }
}

/// 生命周期跨度结果。
struct B45LifecycleSpans {
    max_span_secs: u64,
}

/// Step 3 真实执行:LC_GATEWAY_E2E=1 + --ignored 命中(缺开关=失败)。
#[tokio::test]
#[ignore = "b45 真实现场:LC_GATEWAY_E2E=1 且 Step 3 命令执行(需要 kimi 登录态/podman 镜像/node/npm)"]
async fn b45_live_kimi_host_check() {
    require_lc_gateway_e2e_switch();
    // 外层角色 idle 界压到 90s(<150s 静默):静默真实跨界(单线程 ignored 测试;
    // 引擎在 provider 调用启动时读一次 env,先例 provider_idle_stall.rs)。
    unsafe {
        std::env::set_var("ARIA_CODING_CODER_IDLE_SECS", "90");
    }
    let evidence_root = PathBuf::from("cadence/reports/lc-gateway-multi-provider/kimi-code")
        .join("host-check");
    let harness = LiveLcGatewayHarness::new();
    let run = harness
        .run_b45_kimi_host_check(&evidence_root)
        .await
        .expect("b45 现场执行失败(环境不可运行须报告 BLOCKED,不删格)");

    // 证据落盘(脱敏:不含认证材料;fixture 命令/输出均为非敏感内容)。
    std::fs::create_dir_all(&evidence_root).expect("证据目录");
    std::fs::write(
        evidence_root.join("run.json"),
        serde_json::to_vec_pretty(&run).expect("run json"),
    )
    .expect("run.json 落盘");
    let evidence = b45_evidence_from_run(&run);
    std::fs::write(
        evidence_root.join("evidence.json"),
        serde_json::to_vec_pretty(&json!({
            "validated": {
                "provider_exact_version": evidence.provider_exact_version,
                "tools": evidence.tools,
                "duties": evidence.duties,
                "long_silent": evidence.long_silent,
                "termination": evidence.termination,
                "snapshot_before": evidence.snapshot_before,
                "snapshot_after": evidence.snapshot_after,
                "coder_dirty_preserved": evidence.coder_dirty_preserved,
            }
        }))
        .expect("evidence json"),
    )
    .expect("evidence.json 落盘");

    // validator 判绿(缺项=真实失败,不删格)。
    if let Err(failures) = validate_b45_host_check_evidence(&evidence) {
        panic!("b45 证据 validator 拒绝:{failures:?}");
    }
    // Step 4 判绿附加:三工具 CHECK 面真实成功(exit=0)且数量可核对。
    for record in &evidence.tools {
        let is_check_facet = matches!(
            (record.tool.as_str(), record.facet.as_str()),
            ("maven", "test") | ("npm", "test") | ("npm", "ci") | ("npm", "build")
        ) || record.tool == "podman" && record.facet != "version";
        if is_check_facet {
            assert_eq!(
                record.exit_code,
                Some(0),
                "{}/{} CHECK 未成功(退出码非零=真实失败,记录不删格)",
                record.tool,
                record.facet
            );
        }
    }
    for (tool, facet) in [("maven", "test"), ("npm", "test")] {
        let count = evidence
            .tools
            .iter()
            .find(|record| record.tool == tool && record.facet == facet)
            .and_then(|record| record.test_count)
            .unwrap_or(0);
        assert!(count > 0, "{tool}/{facet} 测试数量必须可核对(>0)");
    }
}
