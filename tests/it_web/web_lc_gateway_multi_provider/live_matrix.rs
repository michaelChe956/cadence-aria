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
