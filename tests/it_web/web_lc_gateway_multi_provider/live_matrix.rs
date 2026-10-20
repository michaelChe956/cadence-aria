//! Task 10a:证据结构单测 + 四家 `#[ignore]` 真实现场测试。
//!
//! - `lcg_t10_evidence_requires_exact_version_wire_and_native_confirmation`
//!   只验证证据结构:缺字段记录必须被拒绝(两阶段红绿的 RED 锚点);
//! - 四个 `lcg_live_*` 为真实测试,默认 `#[ignore]`;执行时必须显式开关
//!   `LC_GATEWAY_E2E=1` 且 `--ignored` 命中,开关缺失要失败而非 return 成功
//!   (Step 4 四命令归 controller 现场执行,10a 只交 harness)。

use std::path::{Path, PathBuf};

use cadence_aria::product::logical_codebase::provider_boundary_probe::provider_family_text;
use cadence_aria::product::models::ProviderName;
use serde_json::{Value, json};

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
        carried_source_evidence_root: None,
        carried_workspace_digest: String::new(),
        carried_aggregate_digest: String::new(),
        carried_capability_digest: String::new(),
        carried_policy_digest: String::new(),
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
///
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

// ---------------------------------------------------------------------------
// 证据根计算(唯一计算点):全部生产者(cell.json/汇总/快照/边界/失败落盘)
// 与消费者(承继格 `source_evidence_root` 解析、快照续跑定位)共用本段,
// 防多处各自拼路径导致 run-id 只落在生产者一侧、carried 引用失效。
// ---------------------------------------------------------------------------

/// 冻结报告根(仓库相对;计划 Files)。
pub(crate) const LC_GATEWAY_REPORTS_ROOT: &str = "cadence/reports/lc-gateway-multi-provider";

/// T22 证据根 run-id(缺省空=旧固定路径;设值作 provider 证据根末段)。
pub(crate) const B45_T22_RUN_ID_ENV: &str = "B45_T22_RUN_ID";
/// T23 证据根 run-id(同上;含五阶段/roles 矩阵/boundary/failures/汇总)。
pub(crate) const B45_T23_RUN_ID_ENV: &str = "B45_T23_RUN_ID";
/// T24 证据根 run-id(同上;pi/ 下全部 facet)。
pub(crate) const B45_T24_RUN_ID_ENV: &str = "B45_T24_RUN_ID";

/// run-id 路径段校验:必须是单一路径段(防注入逃出证据根),非空白,
/// 字符集 `[A-Za-z0-9._-]`,且不得为 `.`/`..`。
fn validate_run_id_segment(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("run-id 为空".to_string());
    }
    if matches!(trimmed, "." | "..") {
        return Err(format!("run-id 不得为 {trimmed:?}(非单一路径段)"));
    }
    if !trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
    {
        return Err(format!(
            "run-id {trimmed:?} 含非法字符(只允许 ASCII 字母数字与 . _ -)"
        ));
    }
    Ok(trimmed.to_string())
}

/// 纯计算:provider 证据根 = `<reports>/<provider 族名>[/<run-id>]`。
///
/// - `run_id=None`(env 未设/空白)→ 旧固定路径,向后兼容;
/// - `run_id=Some`(env 设值)→ 追加为**末段**:该任务全部 facet 与兄弟
///   `snapshots/`(capture→resume 同一 run-id 共用)随之按轮隔离。
pub(crate) fn provider_evidence_root_with_run_id(
    provider: &ProviderName,
    run_id: Option<&str>,
) -> Result<PathBuf, String> {
    let mut root = PathBuf::from(LC_GATEWAY_REPORTS_ROOT).join(provider_family_text(provider));
    if let Some(run_id) = run_id {
        root.push(validate_run_id_segment(run_id)?);
    }
    Ok(root)
}

/// run-id env 读取(去空白;缺省/空白=None)。合法性由
/// [`provider_evidence_root_with_run_id`] 统一校验。
pub(crate) fn evidence_run_id_from_env(run_id_env: &str) -> Option<String> {
    std::env::var(run_id_env)
        .ok()
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty())
}

/// env 版唯一入口:非法 run-id fail-closed(panic,不静默落回固定路径)。
pub(crate) fn provider_evidence_root(provider: &ProviderName, run_id_env: &str) -> PathBuf {
    let run_id = evidence_run_id_from_env(run_id_env);
    provider_evidence_root_with_run_id(provider, run_id.as_deref())
        .unwrap_or_else(|error| panic!("{run_id_env} 非法:{error}"))
}

/// 现场证据落点(计划 Task 10 Files):`cadence/reports/lc-gateway-multi-provider/
/// <provider>/matrix/`(设 `B45_T23_RUN_ID` 时 `<provider>/<run-id>/matrix/`),
/// provider 目录固定为 claude-code/codex/pi/kimi-code。
fn live_evidence_root(provider: &ProviderName) -> PathBuf {
    provider_evidence_root(provider, B45_T23_RUN_ID_ENV).join("matrix")
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
    if !maven_facets.contains(&"test") {
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
    } else if !podman_facets.contains(&"build") || !podman_facets.contains(&"run") {
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
        // r74(F4):maven/npm 全 facet 与 podman/build 的 cwd 必须含 fixture
        // 路径段(钉定命令均 cd 进 fixtures/*;podman/run 按命令(6)从仓库
        // 根执行,豁免)——外层聚合根 cd 遮蔽内层 fixture cd 的失真取法
        // 在此暴露。
        let requires_fixture_cwd = record.tool == "maven"
            || record.tool == "npm"
            || (record.tool == "podman" && record.facet == "build");
        if requires_fixture_cwd && !record.cwd.contains("fixture") {
            failures.push(format!(
                "{label} cwd 缺 fixture 路径段(实测 {:?})",
                record.cwd
            ));
        }
        if record.exit_code.is_none() {
            failures.push(format!("{label} 缺退出码"));
        }
        let is_test_facet = (record.tool == "maven" && record.facet == "test")
            || (record.tool == "npm" && record.facet == "test");
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
                if record
                    .terminal_status
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .is_empty()
                {
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
        cwd: "/worktree/fixtures/maven-fixture".to_string(),
        exit_code: Some(0),
        test_count,
        output_excerpt: "T22 excerpt".to_string(),
        terminal_lifecycle: vec![
            "Started".to_string(),
            "Running".to_string(),
            "Completed".to_string(),
        ],
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

/// r74-F1(T22 审查):参数位工具名不得产出工具格——r17 绿灯实证
/// `git status…which podman npm mvn`/`grep 'podman run'`/echo 文案被当执行。
#[test]
fn b45_classify_rejects_argument_position_tool_tokens() {
    // 参数位(which/grep/echo 文案):一律不分类。
    assert_eq!(
        b45_classify_command("/bin/bash -c which podman npm mvn java"),
        (String::new(), String::new()),
        "which 的参数位工具名不得当执行"
    );
    assert_eq!(
        b45_classify_command("grep -n 'podman run --rm aria-t22-check' log.txt"),
        (String::new(), String::new()),
        "grep 的模式串不得当执行"
    );
    assert_eq!(
        b45_command_facets("echo \"next: npm test and podman build\" | tee log"),
        Vec::<(String, String)>::new(),
        "echo 文案不得产出任何面目"
    );
    // 命令位(段首/操作符后):正常分类。
    assert_eq!(
        b45_command_facets("cd fixtures/npm-fixture && npm ci; npm test; npm run build"),
        vec![
            ("npm".to_string(), "ci".to_string()),
            ("npm".to_string(), "test".to_string()),
            ("npm".to_string(), "build".to_string()),
        ],
        "复合命令操作符后的命令位全提取"
    );
    assert_eq!(
        b45_command_facets("podman build -t aria-t22-check . && podman run --rm aria-t22-check"),
        vec![
            ("podman".to_string(), "build".to_string()),
            ("podman".to_string(), "run".to_string()),
        ]
    );
    // 粘连分号形态(x; podman run)。
    assert_eq!(
        b45_classify_command("echo x; podman run --rm aria-t22-check"),
        ("podman".to_string(), "run".to_string())
    );
}

/// r74-F2(T22 审查):退出码按 (role_run_id, event_id) 配对——裸 id 跨
/// 会话撞号可把真实非零覆盖成同号 0(假绿);CHECK 工具格只认 coder 行。
/// 以 r17 实证形态构造:coder term-7 exit=1,code_reviewer 同号 term-7
/// exit=0 在后——工具格必须取 coder 的 1(真实失败不被掩盖)。
#[test]
fn b45_tool_exit_pairs_within_coder_role_run() {
    let run = json!({
        "coding_main": {
            "role_run_rows": [
                {"role": "coder", "role_run_id": "coding_role_run_0002", "created_at": "2026-10-18T09:00:00Z", "event_type": "ExecutionEvent", "payload": {"event_id": "kimi_terminal_term-7", "status": "Started", "command": "/bin/bash -c cd '/w/fixtures/maven-fixture' && podman run --rm maven:3.9 mvn -B test"}},
                {"role": "coder", "role_run_id": "coding_role_run_0002", "created_at": "2026-10-18T09:02:30Z", "event_type": "ExecutionEvent", "payload": {"event_id": "kimi_terminal_term-7", "status": "Completed", "exit_code": 1}},
                {"role": "code_reviewer", "role_run_id": "coding_role_run_0004", "created_at": "2026-10-18T09:05:00Z", "event_type": "ExecutionEvent", "payload": {"event_id": "kimi_terminal_term-7", "status": "Started", "command": "/bin/bash -c cd '/w' && echo probe; npm --version"}},
                {"role": "code_reviewer", "role_run_id": "coding_role_run_0004", "created_at": "2026-10-18T09:05:01Z", "event_type": "ExecutionEvent", "payload": {"event_id": "kimi_terminal_term-7", "status": "Completed", "exit_code": 0}},
            ],
            "verification_evidence": {"status": 422, "body": {"code": "x"}},
            "check_evidence_log": "Tests run: 2, Failures: 0\n2 passed, 0 failed\n",
        },
        "fixtures": {"versions": {"maven_container": {"stdout": "Apache Maven 3.9.16\n"}, "npm": {"stdout": "11.13.0\n"}, "podman": {"stdout": "6.1.1\n"}}},
    });
    let evidence = b45_evidence_from_run(&run);
    let maven = evidence
        .tools
        .iter()
        .find(|record| record.tool == "maven" && record.facet == "test")
        .expect("coder 的 mvn 执行必须建格");
    assert_eq!(
        maven.exit_code,
        Some(1),
        "退出码必须取 coder 同 role_run 的 1,不被 reviewer 同号 0 覆盖"
    );
    // reviewer 的探测命令(echo probe; npm --version)不产出 npm 工具格。
    assert!(
        !evidence.tools.iter().any(|record| record.tool == "npm"),
        "code_reviewer 行不得入 coder CHECK 工具格"
    );
}

/// r74-F3(T22 审查):npm 计数不得吞任意行尾数字——r17 实测 144=
/// TOTAL_LINES(split().next() 对无 passed 行返回整行)。
#[test]
fn b45_npm_count_requires_passed_marker() {
    let run = json!({
        "coding_main": {
            "role_run_rows": [
                {"role": "coder", "role_run_id": "r1", "created_at": "2026-10-18T09:00:00Z", "event_type": "ExecutionEvent", "payload": {"event_id": "kimi_terminal_term-1", "status": "Started", "command": "/bin/bash -c cd '/w/fixtures/npm-fixture' && npm test"}},
                {"role": "coder", "role_run_id": "r1", "created_at": "2026-10-18T09:00:05Z", "event_type": "ExecutionEvent", "payload": {"event_id": "kimi_terminal_term-1", "status": "Completed", "exit_code": 0}},
            ],
            "verification_evidence": {"status": 422, "body": {}},
            "check_evidence_log": "TOTAL_LINES=144\nfiles checked=7\n2 passed, 0 failed\n",
        },
        "fixtures": {"versions": {"maven_container": {"stdout": "Apache Maven 3.9.16\n"}, "npm": {"stdout": "11.13.0\n"}, "podman": {"stdout": "6.1.1\n"}}},
    });
    let evidence = b45_evidence_from_run(&run);
    let npm_test = evidence
        .tools
        .iter()
        .find(|record| record.tool == "npm" && record.facet == "test")
        .expect("npm test 建格");
    assert_eq!(
        npm_test.test_count,
        Some(2),
        "计数必须来自含 passed 的行(2),不得吞 TOTAL_LINES=144"
    );
}

/// r19(T22 r18/r19 现场回归):coder 以 `cmd='…'` / `run_check ID '…'` 转发
/// 载荷执行——三 npm 格必须建格,maven/test、podman/build、podman/run 的
/// cwd 必须落到各自夹具目录(复合命令按 facet 取前缀,不得共用末尾 cd)。
#[test]
fn b45_evidence_recovers_subshell_and_mount_fixture_cwd() {
    let maven_command = "/bin/bash -c cd '/tmp/outer' && cd /tmp/outer/alpha/.worktrees/aria-issues/issue_0001 && cmd='cd fixtures/maven-fixture && podman run --rm -v /tmp/aria-bug45/t22/m2-repo:/root/.m2 -v \"$PWD\":/workspace -w /workspace maven:3.9-eclipse-temurin-17 mvn -B test'; out=$(mktemp); bash -c \"$cmd\" >\"$out\" 2>&1; rc=$?; printf '===== CHECK-001 =====\\n' >> CHECK-EVIDENCE.log; cat \"$out\" >> CHECK-EVIDENCE.log; printf 'EXIT: %s\\n' \"$rc\" >> CHECK-EVIDENCE.log; echo \"CHECK-001 rc=$rc\"";
    let combined_command = "/bin/bash -c cd '/tmp/outer' && cd /tmp/outer/alpha/.worktrees/aria-issues/issue_0001 && run_check() { id=\"$1\"; cmd=\"$2\"; out=$(mktemp); bash -c \"$cmd\" >\"$out\" 2>&1; rc=$?; cat \"$out\" >> CHECK-EVIDENCE.log; echo \"$id rc=$rc\"; }; run_check CHECK-002 'cd fixtures/npm-fixture && npm ci'; run_check CHECK-003 'cd fixtures/npm-fixture && npm test'; run_check CHECK-004 'cd fixtures/npm-fixture && npm run build'; run_check CHECK-005 'cd fixtures/podman-fixture && podman build -t aria-t22-check \"$PWD\"'; run_check CHECK-006 'cd fixtures/podman-fixture && podman run --rm aria-t22-check'";
    let run = json!({
        "coding_main": {
            "role_run_rows": [
                {"role": "coder", "role_run_id": "r1", "created_at": "2026-10-18T09:00:00Z", "event_type": "ExecutionEvent", "payload": {"event_id": "kimi_terminal_term-1", "status": "Started", "cwd": "/tmp/outer/alpha/.worktrees/aria-issues/issue_0001", "command": maven_command}},
                {"role": "coder", "role_run_id": "r1", "created_at": "2026-10-18T09:02:00Z", "event_type": "ExecutionEvent", "payload": {"event_id": "kimi_terminal_term-1", "status": "Completed", "exit_code": 0}},
                {"role": "coder", "role_run_id": "r1", "created_at": "2026-10-18T09:03:00Z", "event_type": "ExecutionEvent", "payload": {"event_id": "kimi_terminal_term-2", "status": "Started", "cwd": "/tmp/outer/alpha/.worktrees/aria-issues/issue_0001", "command": combined_command}},
                {"role": "coder", "role_run_id": "r1", "created_at": "2026-10-18T09:05:00Z", "event_type": "ExecutionEvent", "payload": {"event_id": "kimi_terminal_term-2", "status": "Completed", "exit_code": 0}},
                {"role": "coder", "role_run_id": "r1", "created_at": "2026-10-18T09:05:30Z", "event_type": "ExecutionEvent", "payload": {"event_id": "kimi_terminal_term-3", "status": "Started", "cwd": "/tmp/outer", "command": "npm -v; podman --version; mvn -v"}},
                {"role": "coder", "role_run_id": "r1", "created_at": "2026-10-18T09:05:31Z", "event_type": "ExecutionEvent", "payload": {"event_id": "kimi_terminal_term-3", "status": "Completed", "exit_code": 0}},
            ],
            "verification_evidence": {"status": 422, "body": {}},
            "check_evidence_log": "Tests run: 2, Failures: 0, Errors: 0, Skipped: 0\n2 passed, 0 failed\n",
        },
        "fixtures": {"versions": {"maven_container": {"stdout": "Apache Maven 3.9.16\n"}, "npm": {"stdout": "11.13.0\n"}, "podman": {"stdout": "6.1.1\n"}}},
    });
    let evidence = b45_evidence_from_run(&run);
    for facet in ["ci", "test", "build"] {
        let npm = evidence
            .tools
            .iter()
            .find(|record| record.tool == "npm" && record.facet == facet)
            .unwrap_or_else(|| panic!("npm {facet} 必须建格"));
        assert_eq!(
            npm.cwd, "/tmp/outer/alpha/.worktrees/aria-issues/issue_0001/fixtures/npm-fixture",
            "npm {facet} cwd 必须绝对化到夹具目录"
        );
    }
    for (tool, facet, expected) in [
        (
            "maven",
            "test",
            "/tmp/outer/alpha/.worktrees/aria-issues/issue_0001/fixtures/maven-fixture",
        ),
        (
            "podman",
            "build",
            "/tmp/outer/alpha/.worktrees/aria-issues/issue_0001/fixtures/podman-fixture",
        ),
        (
            "podman",
            "run",
            "/tmp/outer/alpha/.worktrees/aria-issues/issue_0001/fixtures/podman-fixture",
        ),
    ] {
        let record = evidence
            .tools
            .iter()
            .find(|record| record.tool == tool && record.facet == facet)
            .unwrap_or_else(|| panic!("{tool}/{facet} 必须建格"));
        assert_eq!(record.cwd, expected, "{tool}/{facet} cwd");
    }
    assert!(
        !evidence
            .tools
            .iter()
            .any(|record| record.facet.starts_with('-')),
        "版本探针不得建出 -v/--version 且: {:?}",
        evidence
            .tools
            .iter()
            .map(|record| format!("{}/{}", record.tool, record.facet))
            .collect::<Vec<_>>()
    );
}

/// r74-F4(T22 审查):cwd 取最后一个 cd 的目标段——外层聚合根 cd 遮蔽
/// 内层 fixture cd 的失真取法(r17 实证)必须修正;validator 侧
/// fixture 段规则由 b45_host_check_validator 的 cwd 校验承载。
#[test]
fn b45_command_cwd_takes_last_cd_segment() {
    assert_eq!(
        b45_command_cwd(
            "cd '/tmp/.tmpXXX' && cd '/tmp/.tmpXXX/alpha/fixtures/maven-fixture' && podman run --rm maven mvn -B test"
        ),
        "/tmp/.tmpXXX/alpha/fixtures/maven-fixture",
        "取最后一个 cd 目标(内层 fixture),非外层聚合根"
    );
    assert_eq!(
        b45_command_cwd("cd /workspace && npm ci"),
        "/workspace",
        "无引号 cd 形态"
    );
    assert_eq!(
        b45_command_cwd("podman run --rm aria-t22-check"),
        "",
        "无 cd 为空"
    );
}

/// r18 实证形态:npm 在子壳 `(cd fixtures/npm-fixture && npm ci)` 内执行,
/// sub token 带 `);` 尾缀(旧口径误判 facet `ci)` → 三格全缺)。
#[test]
fn b45_command_facets_normalizes_subshell_suffix() {
    assert_eq!(
        b45_command_facets(
            "/bin/bash -c cd '/tmp/outer' && cd /tmp/outer/alpha/.worktrees/aria-issues/issue_0001 && { (cd fixtures/npm-fixture && npm ci); echo \"EXIT_CODE=$?\"; } 2>&1 | tee /tmp/aria-b45-evidence/CHECK-002.out"
        ),
        vec![("npm".to_string(), "ci".to_string())],
        "子壳尾括号归一后必须识别 npm ci"
    );
    assert_eq!(
        b45_command_facets(
            "/bin/bash -c cd '/tmp/outer' && cd /tmp/outer/alpha/.worktrees/aria-issues/issue_0001 && { (cd fixtures/npm-fixture && npm run build); echo \"EXIT_CODE=$?\"; } 2>&1 | tee /tmp/x.out"
        ),
        vec![("npm".to_string(), "build".to_string())],
        "子壳内 npm run build"
    );
}

/// r19(T22 r18 现场):kimi 偏离钉定形态——maven/podman 从 issue 根以
/// `-v 夹具:/workspace -w /workspace` / `podman build 夹具` 执行,cd 目标
/// 止于 issue 根;cwd 必须取命令中最后的夹具路径 token(挂载宿主源/
/// 构建上下文),不得留外层聚合根。
#[test]
fn b45_command_cwd_recovers_fixture_from_mount_and_build_context() {
    assert_eq!(
        b45_command_cwd(
            "/bin/bash -c cd '/tmp/outer' && cd /tmp/outer/alpha/.worktrees/aria-issues/issue_0001 && podman run --rm -v /tmp/aria-bug45/t22/m2-repo:/root/.m2 -v fixtures/maven-fixture:/workspace -w /workspace maven:3.9-eclipse-temurin-17 mvn -B test"
        ),
        "fixtures/maven-fixture",
        "maven -v 夹具宿主源即生效工作目录"
    );
    assert_eq!(
        b45_command_cwd(
            "cd /tmp/x/issue_0001 && podman run --rm -v \"$(pwd)/fixtures/maven-fixture:/workspace\" -w /workspace maven:3.9 mvn -B test"
        ),
        "fixtures/maven-fixture",
        "$(pwd)/ 前缀与引号归一"
    );
    assert_eq!(
        b45_command_cwd(
            "cd /tmp/x/issue_0001 && podman build -t aria-t22-check fixtures/podman-fixture"
        ),
        "fixtures/podman-fixture",
        "podman build 构建上下文即夹具目录"
    );
    assert_eq!(
        b45_command_cwd(
            "cd /tmp/x/issue_0001 && podman build -t aria-t22-check . && cat fixtures/podman-fixture/Containerfile"
        ),
        "/tmp/x/issue_0001",
        "fixtures/<名>/<文件> 不得冒充夹具目录"
    );
    assert_eq!(
        b45_resolve_cwd("cd fixtures/npm-fixture && npm ci", "/tmp/outer/issue_0001"),
        "/tmp/outer/issue_0001/fixtures/npm-fixture",
        "相对 cwd 按事件基线绝对化"
    );
    assert_eq!(
        b45_resolve_cwd(
            "cd /abs/fixtures/npm-fixture && npm ci",
            "/tmp/outer/issue_0001"
        ),
        "/abs/fixtures/npm-fixture",
        "绝对 cwd 不重复拼接"
    );
}

/// r19 实证形态:命令经 `run_check ID 'cd 夹具 && npm …'` 单引号载荷转发,
/// 尾 token 为 `ci';`;引号不归一 → facet `ci'`,三 npm 格全缺。
#[test]
fn b45_command_facets_handles_quoted_bash_c_payload() {
    let command = "/bin/bash -c cd '/tmp/outer' && cd /tmp/outer/alpha/.worktrees/aria-issues/issue_0001 && run_check() { id=\"$1\"; cmd=\"$2\"; bash -c \"$cmd\"; }; run_check CHECK-002 'cd fixtures/npm-fixture && npm ci'; run_check CHECK-003 'cd fixtures/npm-fixture && npm test'; run_check CHECK-004 'cd fixtures/npm-fixture && npm run build'; run_check CHECK-005 'cd fixtures/podman-fixture && podman build -t aria-t22-check \"$PWD\"'; run_check CHECK-006 'cd fixtures/podman-fixture && podman run --rm aria-t22-check'";
    assert_eq!(
        b45_command_facets(command),
        vec![
            ("npm".to_string(), "ci".to_string()),
            ("npm".to_string(), "test".to_string()),
            ("npm".to_string(), "build".to_string()),
            ("podman".to_string(), "build".to_string()),
            ("podman".to_string(), "run".to_string()),
        ],
        "引号载荷内的复合命令须全提取"
    );
}

/// r19 实证:版本探针 `npm -v`/`podman --version` 曾建出 npm/-v 格并要求
/// fixture cwd;版本形态一律归 version 且被 facet 提取过滤。
#[test]
fn b45_version_probes_do_not_build_tool_cells() {
    let probes = "/bin/bash -c cd '/tmp/outer' && command -v podman; command -v npm; npm -v 2>/dev/null; node -v; podman --version; mvn -v";
    assert_eq!(
        b45_command_facets(probes),
        Vec::<(String, String)>::new(),
        "版本探针不得产出工具格"
    );
}

/// r19 实证:终态 ExecutionEvent 无 command/detail——跨度的关键字过滤只
/// 能落在 Started 行;否则 151s Maven 跨度整段丢弃,退化为行间 90s gap。
#[test]
fn b45_terminal_lifecycle_span_pairs_without_completion_command() {
    let rows = vec![
        json!({"created_at": "2026-10-18T17:22:50.976724144+00:00", "event_type": "ExecutionEvent", "payload": {"event_id": "kimi_terminal_term-10", "status": "Started", "command": "cmd='cd fixtures/maven-fixture && podman run --rm maven mvn -B test'; bash -c \"$cmd\""}}),
        json!({"created_at": "2026-10-18T17:23:20.000000000+00:00", "event_type": "TextDelta", "payload": {"content": "still running"}}),
        json!({"created_at": "2026-10-18T17:25:22.515046954+00:00", "event_type": "ExecutionEvent", "payload": {"event_id": "kimi_terminal_term-10", "status": "Completed", "exit_code": 0}}),
    ];
    assert_eq!(
        b45_terminal_lifecycle_spans(&rows, "mvn").max_span_secs,
        151,
        "Maven 跨度取 Started→Completed(151s),不被行间 gap 稀释"
    );
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
    podman_version_only
        .tools
        .retain(|record| record.tool != "podman");
    podman_version_only
        .tools
        .push(tool_record("podman", "version", "6.1.1", None));
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
    missing_duty
        .duties
        .retain(|record| record.duty != "reviewer");
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
        cwd: "/worktree/fixtures/maven-fixture".to_string(),
        exit_code: Some(0),
        test_count,
        output_excerpt: "T22 excerpt".to_string(),
        terminal_lifecycle: vec![
            "Started".to_string(),
            "Running".to_string(),
            "Completed".to_string(),
        ],
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
    let versions = run
        .pointer("/fixtures/versions")
        .cloned()
        .unwrap_or(Value::Null);
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
            exit_code: row
                .get("exit_code")
                .and_then(Value::as_i64)
                .map(|code| code as i32),
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
        // r74(F3):split_once("passed")——无 passed 的行(split().next()
        // 返回整行)曾把任意行尾数字当计数(r17 实测 144=TOTAL_LINES)。
        let npm_count = check_log
            .lines()
            .filter_map(|line| {
                let (before, _) = line.split_once("passed")?;
                before
                    .trim()
                    .rsplit(|ch: char| !ch.is_ascii_digit())
                    .next()
                    .and_then(|count| count.parse::<u64>().ok())
            })
            .filter(|count| *count > 0)
            .max();
        // r74(F2):同一 terminal(事件 id)的 Started 携带命令、Completed
        // 携带退出码——退出码按 (role_run_id, event_id) 配对:event_id 跨
        // provider 会话撞号(code_reviewer/coder 各有 term-N),按裸 id 配对
        // 可把真实非零覆盖成同号 0(假绿);且 CHECK 工具格只认 role=coder
        // 终端(评审会话的探测命令不入 coder CHECK 证据)。
        let coder_rows: Vec<&Value> = lifecycle_rows
            .iter()
            .filter(|row| row.get("role").and_then(Value::as_str) == Some("coder"))
            .collect();
        let mut exits: std::collections::BTreeMap<(String, String), Option<i64>> =
            std::collections::BTreeMap::new();
        for row in &coder_rows {
            let Some(payload) = row.get("payload") else {
                continue;
            };
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
                    (
                        row.get("role_run_id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        event_id.to_string(),
                    ),
                    payload.get("exit_code").and_then(Value::as_i64),
                );
            }
        }
        for row in &coder_rows {
            let Some(payload) = row.get("payload") else {
                continue;
            };
            if payload.get("status").and_then(Value::as_str) != Some("Started") {
                continue;
            }
            let Some(event_id) = payload.get("event_id").and_then(Value::as_str) else {
                continue;
            };
            let Some(command) = payload.get("command").and_then(Value::as_str) else {
                continue;
            };
            // r19:事件自带 cwd 是命令执行基线,相对 cd/夹具路径据此绝对化。
            let base_cwd = payload
                .get("cwd")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let command_facets = b45_command_facets_with_index(command);
            let (tool, facet, facet_index) = match command_facets.first().cloned() {
                Some(classified) => classified,
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
                                .flat_map(b45_command_facets)
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
                            let lifecycle = b45_terminal_lifecycle_for(&lifecycle_rows, command);
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
                                cwd: b45_resolve_cwd(command, base_cwd),
                                exit_code: exits
                                    .get(&(
                                        row.get("role_run_id")
                                            .and_then(Value::as_str)
                                            .unwrap_or_default()
                                            .to_string(),
                                        event_id.to_string(),
                                    ))
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
            // r72:同命令余下面目补格(复合命令 npm ci;test;build 等)。
            for (extra_tool, extra_facet, extra_index) in command_facets.iter().skip(1) {
                if tools
                    .iter()
                    .any(|record| record.tool == *extra_tool && record.facet == *extra_facet)
                {
                    continue;
                }
                let extra_test = (extra_tool == "maven" && extra_facet == "test")
                    || (extra_tool == "npm" && extra_facet == "test");
                tools.push(B45ToolRecord {
                    exact_version: version_of(extra_tool),
                    tool: extra_tool.clone(),
                    facet: extra_facet.clone(),
                    planned_command: String::new(),
                    actual_command: command.to_string(),
                    argv: command
                        .split_whitespace()
                        .map(str::to_string)
                        .collect::<Vec<_>>(),
                    cwd: b45_resolve_facet_cwd(command, *extra_index, base_cwd),
                    exit_code: exits
                        .get(&(
                            row.get("role_run_id")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                            event_id.to_string(),
                        ))
                        .and_then(|code| *code)
                        .map(|code| code as i32),
                    test_count: if extra_test {
                        if extra_tool == "maven" {
                            maven_count
                        } else {
                            npm_count
                        }
                    } else {
                        None
                    },
                    output_excerpt: String::new(),
                    terminal_lifecycle: lifecycle.clone(),
                    resource_ids: Vec::new(),
                });
            }
            let cwd = b45_resolve_facet_cwd(command, facet_index, base_cwd);
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
                    .get(&(
                        row.get("role_run_id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        event_id.to_string(),
                    ))
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
                    let labels = record
                        .get("labels")
                        .or_else(|| record.pointer("/reserved_labels"))?;
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
                .find(|cell| {
                    cell.get("capability_state").and_then(Value::as_str) == Some("confirmed")
                })
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
        terminal_status: if coder_terminal.is_empty() {
            None
        } else {
            Some(coder_terminal)
        },
        native_activity: format!(
            "execution_events={execution_rows} tool_events={} permission_events={}",
            run.pointer("/coding_main/tool_events")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            run.pointer("/coding_main/permission_events")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        ),
    });
    if let Some(native) = review_confirmed {
        duties.push(B45DutyRecord {
            duty: "reviewer".to_string(),
            terminal_status: Some("confirmed".to_string()),
            native_activity: format!(
                "native_session={native} windows={}",
                run.pointer("/reviewer_windows")
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
        let Some(created_at) = row.get("created_at") else {
            continue;
        };
        let Some(timestamp) = parse_ts(created_at) else {
            previous = None;
            continue;
        };
        if let Some(prior) = previous {
            let gap = timestamp.signed_duration_since(prior).num_seconds().max(0) as u64;
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
        .filter(|code| code.contains("idle_stall") || code.contains("provider_stalled"))
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
    let termination = run
        .pointer("/cancel_cell")
        .map(|cell| B45TerminationRecord {
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
        snapshot_before: format!(
            "head={head_before} porcelain={}",
            text("/snapshot_before/porcelain")
        ),
        snapshot_after: format!("head={head_after} porcelain={porcelain_after}"),
        coder_dirty_preserved: head_after != head_before || !porcelain_after.trim().is_empty(),
    }
}

/// r19:路径 token 归一——引号/尾随操作符剥离、`$(pwd)/`/`$PWD/`/`./`
/// 前缀剥离、`:容器端` 尾段剥离(podman `-v 源:目标` 挂载、镜像标签)。
fn b45_normalize_path_token(token: &str) -> String {
    let trimmed = token.trim_matches(['\'', '"', ',', ';', '&', '|', ')']);
    let stripped = trimmed
        .strip_prefix("$(pwd)/")
        .or_else(|| trimmed.strip_prefix("$PWD/"))
        .or_else(|| trimmed.strip_prefix("./"))
        .unwrap_or(trimmed);
    stripped
        .split(':')
        .next()
        .unwrap_or_default()
        .trim_end_matches('/')
        .to_string()
}

/// r19:夹具目录 token——只认 `fixtures/<名>` 精确两段(排除
/// `fixtures/x/pom.xml` 之类文件路径与 `-w /workspace` 容器侧路径)。
fn b45_fixture_path_token(token: &str) -> Option<String> {
    let normalized = b45_normalize_path_token(token);
    let mut segments = normalized.rsplit('/');
    match (segments.next(), segments.next()) {
        (Some(name), Some("fixtures")) if !name.is_empty() => Some(normalized),
        _ => None,
    }
}

/// r74(F4):从命令串提取工作目录——取**最后一个** `cd '目标'`/`cd 目标` 的
/// 目标段(外层聚合根 cd 会遮蔽内层 fixture cd,首段取法失真);脚本内联
/// 路径同用(r17 实证外层 cd /tmp/.tmpXXX 遮蔽 fixtures/maven-fixture)。
/// r19(T22 r18 现场):kimi 偏离钉定形态时 maven/podman 从 issue 根以
/// `podman run -v 夹具:/workspace -w /workspace` 或 `podman build 夹具`
/// 执行,cd 目标止于 issue 根;此时以命令中最后的夹具路径 token(挂载
/// 宿主源/构建上下文)作为生效工作目录,不得留外层聚合根。
fn b45_command_cwd(command: &str) -> String {
    let mut cwd = String::new();
    let mut fixture_cwd = String::new();
    for token in command.split_whitespace() {
        if let Some(path) = b45_fixture_path_token(token) {
            fixture_cwd = path;
        }
    }
    let mut rest = command;
    while let Some(position) = rest.find("cd ") {
        let after = &rest[position + 3..];
        let target = after.trim_start();
        let segment = if let Some(stripped) = target.strip_prefix('\'') {
            stripped.split('\'').next().unwrap_or_default()
        } else if let Some(stripped) = target.strip_prefix('"') {
            stripped.split('"').next().unwrap_or_default()
        } else {
            target
                .split_whitespace()
                .next()
                .map(|token| token.trim_end_matches(';'))
                .unwrap_or_default()
        };
        if !segment.is_empty() {
            cwd = b45_normalize_path_token(segment);
        }
        rest = after;
    }
    if b45_fixture_path_token(&cwd).is_some() || fixture_cwd.is_empty() {
        cwd
    } else {
        fixture_cwd
    }
}

/// r19:相对 cwd 拼接事件声明的绝对基线(ExecutionEvent payload.cwd),
/// 使证据落盘面的 cwd 为宿主绝对路径(validator 只校验 fixture 段,
/// 绝对化保证人工复核可定位)。
fn b45_join_cwd(cwd: String, base: &str) -> String {
    if cwd.is_empty() || cwd.starts_with('/') || base.trim().is_empty() {
        return cwd;
    }
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        cwd.trim_start_matches("./")
    )
}

fn b45_resolve_cwd(command: &str, base: &str) -> String {
    b45_join_cwd(b45_command_cwd(command), base)
}

/// r19:按工具 token 位次取 cwd——复合 CHECK 命令各段 cd 到不同夹具时,
/// 取该 token 之前前缀的最后一个 cd/夹具 token;前缀无夹具(如
/// `podman build 夹具目录` 从仓库根执行)则取该段参数里的夹具(构建
/// 上下文),仍无则回退前缀 cwd。
fn b45_tool_cwd(command: &str, index: usize) -> String {
    let tokens: Vec<&str> = command.split_whitespace().collect();
    let prefix_cwd = b45_command_cwd(&tokens[..index.min(tokens.len())].join(" "));
    if b45_fixture_path_token(&prefix_cwd).is_some() {
        return prefix_cwd;
    }
    let mut segment_fixture = String::new();
    for token in tokens.get(index..).unwrap_or_default() {
        if let Some(path) = b45_fixture_path_token(token) {
            segment_fixture = path;
        }
        if matches!(*token, "&&" | "||" | ";") || token.ends_with(';') || token.ends_with("&&") {
            break;
        }
    }
    if segment_fixture.is_empty() {
        prefix_cwd
    } else {
        segment_fixture
    }
}

fn b45_resolve_facet_cwd(command: &str, index: usize, base: &str) -> String {
    b45_join_cwd(b45_tool_cwd(command, index), base)
}

/// r74(F1):token 是否处于命令位——段首,或前一 token 是/以
/// &&/||/;/|/(/!/{ 操作符收尾(echo "x"; podman run 的粘连分号形态)。
fn token_is_command_position(tokens: &[&str], index: usize) -> bool {
    if index == 0 {
        return true;
    }
    let previous = tokens[index - 1];
    matches!(previous, "&&" | "||" | ";" | "|" | "(" | "{" | "!")
        || previous.ends_with(';')
        || previous.ends_with('|')
        || previous.ends_with('&')
}

/// 命令族分类(podman/npm/maven + facet)。
fn b45_classify_command(command: &str) -> (String, String) {
    let tokens: Vec<&str> = command.split_whitespace().collect();
    // r74(F1):只认命令位的工具 token——段首,或 &&/||/;/|/(/!/{ 之后
    //(含前 token 以这些操作符收尾的粘连形态)。参数位(which podman/
    // grep 'podman run'/echo 文案)不产出工具格(r17 绿灯实证该缺陷:
    // maven/test 的 actual_command=git status…which,无真实 mvn 执行)。
    let tool_index = tokens
        .iter()
        .enumerate()
        .find(|(index, token)| {
            matches!(**token, "podman" | "npm" | "mvn")
                && token_is_command_position(&tokens, *index)
        })
        .map(|(index, _)| index)
        .unwrap_or(0);
    let head = tokens.get(tool_index).copied().unwrap_or_default();
    // r74:粘连操作符形态(npm ci; npm test)的子命令 token 带尾分号,
    // 归一化后再分类。
    // r19:单引号包裹的 bash -c 载荷(`run_check CHECK-002 'cd x && npm ci'`)
    // 尾 token 为 `ci';`——引号同属包裹符,不归一即误判 facet `ci'`。
    let sub = tokens
        .get(tool_index + 1)
        .copied()
        .unwrap_or_default()
        .trim_end_matches([';', '|', '&', ')', '\'', '"']);
    let third = tokens
        .get(tool_index + 2)
        .copied()
        .unwrap_or_default()
        .trim_end_matches([';', '|', '&', ')', '\'', '"']);
    match head {
        "podman" => {
            match sub {
                "build" => ("podman".to_string(), "build".to_string()),
                "run" => {
                    if tokens.contains(&"mvn") {
                        ("maven".to_string(), "test".to_string())
                    } else {
                        ("podman".to_string(), "run".to_string())
                    }
                }
                // r19:`podman --version`/`-v` 为版本探针,不是容器 CHECK。
                "version" | "--version" | "-v" => ("podman".to_string(), "version".to_string()),
                _ => ("podman".to_string(), sub.to_string()),
            }
        }
        "npm" => {
            let facet = match sub {
                "ci" => "ci",
                "test" => "test",
                "run" if third == "build" => "build",
                // r19:`npm -v`/`--version` 版本探针不得建 CHECK 格(曾产出
                // npm/-v 并要求 fixture cwd)。
                "--version" | "-v" => "version",
                other => other,
            };
            ("npm".to_string(), facet.to_string())
        }
        "mvn" => {
            if matches!(sub, "--version" | "-v") {
                ("maven".to_string(), "version".to_string())
            } else {
                ("maven".to_string(), "test".to_string())
            }
        }
        _ => (String::new(), String::new()),
    }
}

/// r72(T22 r14 现场):复合命令多面提取——coder 实执行常把多条 CHECK
/// 串在一个 terminal(`npm ci; npm test; npm run build` / `podman build &&
/// podman run`),首 token 分类只出第一个 facet,后续全漏。扫描命令中
/// 全部工具 token 位次,逐一分类去重(podman run 含 mvn 仍归 maven/test)。
fn b45_command_facets(command: &str) -> Vec<(String, String)> {
    b45_command_facets_with_index(command)
        .into_iter()
        .map(|(tool, facet, _)| (tool, facet))
        .collect()
}

/// r19:带工具 token 位次的面目提取——复合 CHECK 命令内各段可 cd 到不同
/// 夹具(`npm ci` 在 npm-fixture、`podman build` 在 podman-fixture),
/// 工具格 cwd 必须取该 token 之前的命令前缀,不能共用命令末尾 cd。
fn b45_command_facets_with_index(command: &str) -> Vec<(String, String, usize)> {
    let tokens: Vec<&str> = command.split_whitespace().collect();
    let mut facets: Vec<(String, String, usize)> = Vec::new();
    for index in 0..tokens.len() {
        if !matches!(tokens[index], "podman" | "npm" | "mvn")
            || !token_is_command_position(&tokens, index)
        {
            continue;
        }
        let (tool, facet) = b45_classify_command(&tokens[index..].join(" "));
        if tool.is_empty() || facet == "version" {
            continue;
        }
        if !facets.iter().any(|(t, f, _)| *t == tool && *f == facet) {
            facets.push((tool, facet, index));
        }
    }
    facets
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
        .filter_map(|row| {
            row.pointer("/payload/status")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
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
    // r72(T22 r14 现场):event_id 跨 provider 会话撞号(两个会话各有
    // term-14)——按 id 合并会把后段 Started 覆盖前段,150s 静默 Maven
    // (term-14 Started 09:08:38→Completed 09:11:09,exit 0)跨度归零、
    // 实测退化成行间 gap。改时序开合配对:每 id 维护当前打开的 Started,
    // 终态行闭合并记跨度后清空(后段同名 terminal 重新打开独立计时)。
    let mut open_started: BTreeMap<String, chrono::DateTime<chrono::Utc>> = BTreeMap::new();
    let mut max_span_secs: u64 = 0;
    for row in rows {
        let Some(event_id) = row.pointer("/payload/event_id").and_then(Value::as_str) else {
            continue;
        };
        if !event_id.starts_with("kimi_terminal_") {
            continue;
        }
        let Some(timestamp) = parse_ts(row.get("created_at")) else {
            continue;
        };
        let status = row
            .pointer("/payload/status")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if status == "Started" {
            // 关键字只在 Started 行判定(命令载荷在 Started);终态行无
            // command/detail——r19 实证:在终态行再做关键字过滤会把
            // 151s 的 Maven 跨度整段丢弃,退化为 90s 行间 gap。
            let command_or_detail = row
                .pointer("/payload/command")
                .and_then(Value::as_str)
                .or_else(|| row.pointer("/payload/detail").and_then(Value::as_str))
                .unwrap_or_default();
            if command_or_detail.contains(keyword) {
                open_started.insert(event_id.to_string(), timestamp);
            }
        } else if matches!(status, "Completed" | "Failed" | "Aborted")
            && let Some(start) = open_started.remove(event_id)
        {
            max_span_secs = max_span_secs
                .max(timestamp.signed_duration_since(start).num_seconds().max(0) as u64);
        }
    }
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
    let evidence_root =
        provider_evidence_root(&ProviderName::KimiCode, B45_T22_RUN_ID_ENV).join("host-check");
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

// ---------------------------------------------------------------------------
// 证据根 run-id 纯逻辑负测(类1′ ②;不需要真实 CLI/LC_GATEWAY_E2E):
// 未设 env 旧行为不变、设值作末段且按轮隔离、注入形态 fail-closed、
// 快照兄弟目录随根同轮隔离(capture→resume 同 run-id 同根)。
// ---------------------------------------------------------------------------

#[cfg(test)]
mod evidence_root_tests {
    use super::*;
    use crate::web_lc_gateway_multi_provider::snapshot::snapshots_root;

    fn legacy(provider: &ProviderName) -> PathBuf {
        PathBuf::from(LC_GATEWAY_REPORTS_ROOT).join(provider_family_text(provider))
    }

    #[test]
    fn evidence_root_without_run_id_keeps_legacy_fixed_path() {
        for provider in [
            ProviderName::ClaudeCode,
            ProviderName::Codex,
            ProviderName::Pi,
            ProviderName::KimiCode,
        ] {
            assert_eq!(
                provider_evidence_root_with_run_id(&provider, None).expect("legacy root"),
                legacy(&provider),
                "未设 run-id 必须保持旧固定路径"
            );
        }
        assert_eq!(
            provider_evidence_root_with_run_id(&ProviderName::ClaudeCode, None)
                .expect("legacy root"),
            PathBuf::from("cadence/reports/lc-gateway-multi-provider/claude-code"),
            "旧固定路径逐字不变(claude-code)"
        );
        assert_eq!(
            provider_evidence_root_with_run_id(&ProviderName::KimiCode, None).expect("legacy root"),
            PathBuf::from("cadence/reports/lc-gateway-multi-provider/kimi-code"),
            "旧固定路径逐字不变(kimi-code)"
        );
        // 空白 env 视同未设(读取侧去空白后过滤空串)。
        assert_eq!(
            evidence_run_id_from_env("B45_TEST_RUN_ID_DEFINITELY_UNSET"),
            None,
            "未设 run-id env 必须为 None(向后兼容前提)"
        );
    }

    #[test]
    fn evidence_root_appends_run_id_as_last_segment() {
        let run_root =
            provider_evidence_root_with_run_id(&ProviderName::Pi, Some("20261020T010203Z-capture"))
                .expect("run root");
        assert_eq!(
            run_root,
            legacy(&ProviderName::Pi).join("20261020T010203Z-capture"),
            "run-id 必须是证据根末段"
        );
        // 同家三轮各自留证:不同 run-id 根互不相同,也不覆盖旧固定根。
        let round_two =
            provider_evidence_root_with_run_id(&ProviderName::Pi, Some("20261020T020304Z-capture"))
                .expect("run root 2");
        assert_ne!(run_root, round_two, "不同 run-id 必须各自留证");
        assert_ne!(
            run_root,
            legacy(&ProviderName::Pi),
            "带 run-id 不落旧固定根"
        );
        // 异家并行:同 run-id 下各家族根互不相同。
        let other_family = provider_evidence_root_with_run_id(
            &ProviderName::KimiCode,
            Some("20261020T010203Z-capture"),
        )
        .expect("run root");
        assert_ne!(
            run_root, other_family,
            "同 run-id 异家证据根必须互不相同(不互相覆盖)"
        );
    }

    #[test]
    fn evidence_root_run_id_scopes_snapshot_sibling_for_capture_and_resume() {
        let run_root =
            provider_evidence_root_with_run_id(&ProviderName::ClaudeCode, Some("round-1"))
                .expect("run root");
        let matrix_root = run_root.join("b45-t23-roles").join("matrix");
        // capture 轮与 resume 轮用同一 env → 同一 evidence_root(纯函数幂等)。
        assert_eq!(
            provider_evidence_root_with_run_id(&ProviderName::ClaudeCode, Some("round-1"))
                .expect("run root")
                .join("b45-t23-roles")
                .join("matrix"),
            matrix_root,
            "capture→resume 必须解析到同一证据根"
        );
        // 快照兄弟目录(capture 落盘 / resume 定位)随 run-id 同轮隔离。
        assert_eq!(
            snapshots_root(&matrix_root),
            run_root.join("b45-t23-roles").join("snapshots"),
            "快照根必须是同 run-id 内的兄弟目录"
        );
        assert_eq!(
            snapshots_root(&run_root.join("matrix")),
            run_root.join("snapshots"),
            "五阶段矩阵快照根必须是同 run-id 内的兄弟目录"
        );
    }

    #[test]
    fn evidence_root_rejects_run_id_escaping_provider_directory() {
        for bad in [
            "",
            "   ",
            ".",
            "..",
            "a/b",
            "..\\escape",
            "a b",
            "a:b",
            "r1/",
        ] {
            assert!(
                provider_evidence_root_with_run_id(&ProviderName::Pi, Some(bad)).is_err(),
                "非法 run-id {bad:?} 必须 fail-closed(不得静默拼进证据根)"
            );
        }
        for good in ["r1", "round-1", "20261020T010203Z", "a_b.c-1"] {
            assert!(
                provider_evidence_root_with_run_id(&ProviderName::Pi, Some(good)).is_ok(),
                "合法 run-id {good:?} 必须接受"
            );
        }
    }
}
