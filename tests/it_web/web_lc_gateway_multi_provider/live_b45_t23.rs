//! 批2 Task 23(b45):四家一致性矩阵与 Codex 双审批真机探针。
//!
//! - `b45_live_four_provider_roles`:四家×六职责(M1–M5/M7)×Direct/LC 双入口
//!   ×fresh/resume 矩阵编排——聚合既有 `run_provider_matrix`(五阶段:
//!   M1 Author=story/design/plan×2、M2 Coder=coding、M3 Reviewer=review、
//!   M7 生命周期=fresh/resume 与 spawn 计数)与 `run_boundary_and_failure_
//!   matrix`(M4 group/repair=coding 正向/受控写/D4、M5 失败关闭=17 固定
//!   负向场景零 spawn),逐家断言六职责域覆盖并落盘单家汇总
//!   b45-t23-roles-matrix-<provider>.json(每格 CLI exact version/native
//!   ID/argv 或 wire/policy digest/cwd/target/spawn 计数全要素,cell.json
//!   由 harness 落盘)。M6 Codex 岗位
//!   由 `b45_live_codex_danger_apply_patch` 承担。
//! - `b45_live_codex_danger_apply_patch`:固定实际 CLI 版本,danger-full-
//!   access 下 on-request(岗位纪律档)与 never(Coder 原生档)两个隔离
//!   fixture 真实执行受控 apply_patch;保存审批帧出现/未出现、fileChange
//!   事实、实际 diff、事件后中断 turn、durable 审计与最终 Failed/不可采信。
//!   事件不可观测或不能可靠中断=该版本机制未通过,不写 Decline 预防、
//!   不退 read-only、不清岗位规则(计划 Task 23 Step 4 红线)。
//!
//! 分批收证:`B45_T23_PROVIDERS`(逗号分隔 claude/codex/pi/kimi 子集,
//! 缺省四家全跑);单家现场命令沿用 `LIVE_MATRIX_RUN_MODE` 快照身份合同。
//!
//! 类1′ 证据完整性(防异家并行/多轮互相覆盖):
//! - 汇总按家分文件 `b45-t23-roles-matrix-<provider>.json`,每家跑完立即
//!   落盘(不生成跨家合并视图,controller 需要时自行合并);
//! - 证据根可接 `B45_T23_RUN_ID`(缺省/空=旧固定路径,向后兼容;设值作
//!   provider 证据根末段)。capture→resume 是同一逻辑轮,两步必须用同一
//!   run-id:承继格 `carried_source_evidence_root`(矩阵证据根)与快照
//!   定位按同一根计算,只有根一致时来源链才可复核。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cadence_aria::cross_cutting::codex_provider::CodexProvider;
use cadence_aria::cross_cutting::streaming_provider::{
    ProviderEvent, ProviderPermissionMode, ProviderToolPolicy, StreamingProviderAdapter,
    StreamingProviderInput,
};
use cadence_aria::cross_cutting::tool_policy_audit::{
    AUTHOR_FILE_CHANGE_DETECTED, RoleRunBoundAuditSink, TOOL_POLICY_RUN_AUDIT_PARTITION,
};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::lifecycle_store::LifecycleStore;
use cadence_aria::product::logical_codebase::provider_boundary_probe::provider_family_text;
use cadence_aria::product::models::ProviderName;
use cadence_aria::protocol::contracts::{AdapterRole, ProviderType};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::failure_matrix::{
    FIXED_FAILURE_SCENARIOS, ZERO_SPAWN_ACCOUNTING_KEYS, cell_accounting_dimension,
};
use super::harness::{
    ENTRYPOINT_SPLIT_SYNC, ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT, EvidenceCell, FRESH,
    LiveLcGatewayHarness, LiveMatrixEvidence, RESUME, carried_cell,
    task11_failure_scenario_binds_session,
};
use super::live_matrix::{
    B45_T23_RUN_ID_ENV, evidence_run_id_from_env, provider_evidence_root,
    provider_evidence_root_with_run_id, require_lc_gateway_e2e_switch,
};

/// 矩阵证据根(计划 Files:cadence/reports/lc-gateway-multi-provider/
/// <provider>/b45-t23-roles/<facet>/;设 `B45_T23_RUN_ID` 时
/// `<provider>/<run-id>/b45-t23-roles/<facet>/`,按轮隔离)。
fn b45_t23_evidence_root(provider: &ProviderName, facet: &str) -> PathBuf {
    provider_evidence_root(provider, B45_T23_RUN_ID_ENV)
        .join("b45-t23-roles")
        .join(facet)
}

/// 单家汇总文件名(类1′ ①:per-provider 分文件,异家并行/多轮互不覆盖)。
pub(crate) fn b45_t23_summary_filename(provider: &ProviderName) -> String {
    format!(
        "b45-t23-roles-matrix-{}.json",
        provider_family_text(provider)
    )
}

/// 落盘单家汇总到给定证据根(纯 IO;单元测试直接调用 tempdir 根)。
///
/// 只写本家文件,不生成跨家合并视图:异家并行/同家多轮不会互相覆盖,
/// 判定只认各 provider 的 cell.json 与本家汇总(controller 需要合并视图
/// 时自行合并)。
fn b45_t23_write_provider_summary_at(
    evidence_root: &Path,
    provider: &ProviderName,
    run_id: Option<&str>,
    summary: &Value,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(evidence_root)
        .map_err(|error| format!("创建汇总目录 {}: {error}", evidence_root.display()))?;
    let path = evidence_root.join(b45_t23_summary_filename(provider));
    let payload = json!({
        "provider": provider_family_text(provider),
        "run_id": run_id,
        "providers": { format!("{provider:?}"): summary },
        "m6_codex_danger": "b45_live_codex_danger_apply_patch",
    });
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&payload).map_err(|error| format!("汇总序列化失败:{error}"))?,
    )
    .map_err(|error| format!("{} 落盘失败:{error}", path.display()))?;
    Ok(path)
}

/// 每家跑完立即落本家汇总(不跨家合并、不等进程末尾):后一家失败不会
/// 抹掉前一家已落证据;证据根含可选 `B45_T23_RUN_ID` 段。
fn b45_t23_write_provider_summary(provider: &ProviderName, summary: &Value) -> PathBuf {
    let run_id = evidence_run_id_from_env(B45_T23_RUN_ID_ENV);
    let root = provider_evidence_root_with_run_id(provider, run_id.as_deref())
        .unwrap_or_else(|error| panic!("{B45_T23_RUN_ID_ENV} 非法:{error}"));
    b45_t23_write_provider_summary_at(&root, provider, run_id.as_deref(), summary)
        .unwrap_or_else(|error| panic!("roles-matrix.json 落盘失败:{error}"))
}

/// 分批 provider 清集(env 缺省=四家全跑)。
fn b45_t23_providers_from_env() -> Vec<ProviderName> {
    let all = [
        ProviderName::ClaudeCode,
        ProviderName::Codex,
        ProviderName::Pi,
        ProviderName::KimiCode,
    ];
    match std::env::var("B45_T23_PROVIDERS") {
        Ok(raw) if !raw.trim().is_empty() => raw
            .split(',')
            .map(|token| match token.trim() {
                "claude" => ProviderName::ClaudeCode,
                "codex" => ProviderName::Codex,
                "pi" => ProviderName::Pi,
                "kimi" => ProviderName::KimiCode,
                other => {
                    panic!("B45_T23_PROVIDERS 未知 provider:{other:?}(合法:claude/codex/pi/kimi)")
                }
            })
            .collect(),
        _ => all.to_vec(),
    }
}

/// 单格摘要(M1–M7 每格要素:CLI exact version/native ID/argv 或 wire/
/// policy digest/cwd/target/spawn 计数;全量形态见 cell.json)。
fn b45_t23_cell_summary(cell: &EvidenceCell) -> Value {
    json!({
        "stage": cell.stage,
        "entrypoint": cell.entrypoint,
        "fresh_or_resume": cell.fresh_or_resume,
        "capability_state": cell.capability_state,
        "exact_version": cell.exact_version,
        "native_session_id": cell.native_session_id,
        "argv_or_wire_capture_exists": cell.argv_or_wire_capture_exists,
        "approval_and_tool_events_exist": cell.approval_and_tool_events_exist,
        "audit_projection_digest": cell.audit_projection_digest,
        "frozen_projection_digest": cell.frozen_projection_digest,
        "process_cwd": cell.process_cwd,
        "target": cell.target,
        "provider_spawn_count": cell.provider_spawn_count,
        "native_resume_confirmed_id": cell.native_resume_confirmed_id,
        "requested_resume_id": cell.requested_resume_id,
        "execution_origin": cell.execution_origin,
        "denied_reason": cell.denied_reason,
    })
}

/// resume 轮必需承继阶段(阶段, 入口):story/design/plan(streaming)+
/// split(split_sync)。
const RESUME_CARRIED_STAGES: [(&str, &str); 4] = [
    ("story", ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT),
    ("design", ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT),
    ("plan", ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT),
    ("split", ENTRYPOINT_SPLIT_SYNC),
];

/// full_chain 轮逐 stage(plan 两入口)至少 1 个 fresh 执行格。
const FULL_CHAIN_FRESH_STAGES: [(&str, &str); 6] = [
    ("story", ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT),
    ("design", ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT),
    ("plan", ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT),
    ("plan", ENTRYPOINT_SPLIT_SYNC),
    ("coding", ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT),
    ("review", ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT),
];

/// 轮次覆盖断言(纯函数;负测在 `mod tests` 钉死拒绝形态)。
///
/// - resume 轮:story/design/plan(streaming)+split(split_sync)各恰 1 个
///   carried 格且来源链可复核;coding/review 各恰 1 个 executed-resume
///   (真实执行、native ID 相同;carried 冒充 resume 拒绝)。
/// - full_chain 轮:逐 stage(plan 两入口)至少 1 个 fresh 执行格。
fn verify_round_coverage(resume_round: bool, cells: &[EvidenceCell]) -> Result<(), String> {
    if resume_round {
        verify_resume_round(cells)
    } else {
        verify_full_chain_round(cells)
    }
}

fn verify_resume_round(cells: &[EvidenceCell]) -> Result<(), String> {
    for (stage, entrypoint) in RESUME_CARRIED_STAGES {
        let carried: Vec<&EvidenceCell> = cells
            .iter()
            .filter(|cell| {
                cell.stage == stage
                    && cell.entrypoint == entrypoint
                    && cell.capability_state == "carried"
            })
            .collect();
        if carried.len() != 1 {
            return Err(format!(
                "resume 轮 {stage}/{entrypoint} 必须恰 1 个 carried 格,实际 {}",
                carried.len()
            ));
        }
        let cell = carried[0];
        if !cell.execution_origin.starts_with("carried:") || cell.run_ref.trim().is_empty() {
            return Err(format!(
                "resume 轮 {stage}/{entrypoint} 承继引用形态不完整:origin={:?} run_ref={:?}",
                cell.execution_origin, cell.run_ref
            ));
        }
        if cell
            .carried_source_evidence_root
            .as_deref()
            .unwrap_or_default()
            .trim()
            .is_empty()
        {
            return Err(format!(
                "resume 轮 {stage}/{entrypoint} 承继格缺来源证据根(引用不可复核)"
            ));
        }
        for (label, digest) in [
            ("workspace", cell.carried_workspace_digest.as_str()),
            ("aggregate", cell.carried_aggregate_digest.as_str()),
            ("capability", cell.carried_capability_digest.as_str()),
            ("policy", cell.carried_policy_digest.as_str()),
        ] {
            if digest.trim().is_empty() {
                return Err(format!(
                    "resume 轮 {stage}/{entrypoint} 承继格缺 {label} 摘要(引用不可复核)"
                ));
            }
        }
    }
    for stage in ["coding", "review"] {
        let executed: Vec<&EvidenceCell> = cells
            .iter()
            .filter(|cell| {
                cell.stage == stage
                    && cell.capability_state == "confirmed"
                    && cell.fresh_or_resume == RESUME
            })
            .collect();
        if executed.len() != 1 {
            return Err(format!(
                "resume 轮 {stage} 必须恰 1 个 executed-resume 格(native ID 相同),实际 {}",
                executed.len()
            ));
        }
        let cell = executed[0];
        if cell.execution_origin.starts_with("carried:") {
            return Err(format!(
                "resume 轮 {stage}:carried 格冒充 executed-resume(承继格不重新计票,不得计作原生恢复)"
            ));
        }
        match (
            cell.requested_resume_id.as_deref(),
            cell.native_resume_confirmed_id.as_deref(),
        ) {
            (Some(requested), Some(confirmed))
                if !requested.trim().is_empty() && requested == confirmed => {}
            _ => {
                return Err(format!(
                    "resume 轮 {stage} executed-resume 的 native ID 不一致:请求 {:?} 确认 {:?}",
                    cell.requested_resume_id, cell.native_resume_confirmed_id
                ));
            }
        }
    }
    Ok(())
}

fn verify_full_chain_round(cells: &[EvidenceCell]) -> Result<(), String> {
    for (stage, entrypoint) in FULL_CHAIN_FRESH_STAGES {
        let fresh_executed = cells.iter().any(|cell| {
            cell.stage == stage
                && cell.entrypoint == entrypoint
                && cell.fresh_or_resume == FRESH
                && cell.capability_state == "confirmed"
                && !cell.execution_origin.starts_with("carried:")
        });
        if !fresh_executed {
            return Err(format!(
                "full_chain 轮 {stage}/{entrypoint} 缺 fresh 执行格(不得以 carried/resume 冒充)"
            ));
        }
    }
    Ok(())
}

/// 逐阶段 spawn 仪器断言(计划 §1.1 ③):fresh/执行 resume 必须有真实新增
/// spawn(≥1);split_sync resume 契约零 spawn;零 spawn 格必须带五维会计
/// 与可证伪观测(不用单维 0)。
fn verify_spawn_instrumentation(cells: &[EvidenceCell]) -> Result<(), String> {
    for cell in cells {
        if cell.capability_state == "carried" {
            continue;
        }
        let split_sync_resume =
            cell.entrypoint == ENTRYPOINT_SPLIT_SYNC && cell.fresh_or_resume == RESUME;
        if split_sync_resume {
            if cell.provider_spawn_count != 0 {
                return Err(format!(
                    "split_sync resume 必须零 spawn,实测 {} ({})",
                    cell.provider_spawn_count, cell.run_ref
                ));
            }
        } else if cell.capability_state == "confirmed" && cell.provider_spawn_count == 0 {
            return Err(format!(
                "fresh/执行 resume 格必须 ≥1 真实 spawn,实测 0({}/{}/{};仪器未接入或真实未启动)",
                cell.stage, cell.entrypoint, cell.fresh_or_resume
            ));
        }
        if cell.provider_spawn_count == 0 {
            verify_zero_spawn_evidence(cell)?;
        }
    }
    Ok(())
}

/// 零 spawn 证明:五维会计齐备 + session child 维为 0 + 可证伪观测
/// (zero_spawn_observation 前后审计相等/未启动标记 或 真实拒绝探针)。
/// 单维 0 不构成证明;native_handshake 允许 ≥0(会话此前可能已握手,
/// 该维不是本格新增 spawn 的判据)。
fn verify_zero_spawn_evidence(cell: &EvidenceCell) -> Result<(), String> {
    let cell_json = cell.to_cell_json();
    for key in ZERO_SPAWN_ACCOUNTING_KEYS {
        if cell_accounting_dimension(&cell_json, key) == u64::MAX {
            return Err(format!(
                "零 spawn 格 {} 缺五维会计维度 {key}(缺维度=不可证零)",
                cell.run_ref
            ));
        }
    }
    for key in [
        "session_child_count",
        "extension_mcp_descendant_count",
        "native_method_count",
    ] {
        if cell_accounting_dimension(&cell_json, key) != 0 {
            return Err(format!(
                "零 spawn 格 {} 的五维会计不自洽:{key}={}",
                cell.run_ref,
                cell_accounting_dimension(&cell_json, key)
            ));
        }
    }
    let observed_zero = cell.provider_events.iter().any(|event| {
        let payload = &event["event"];
        match payload["type"].as_str() {
            Some("zero_spawn_observation") => {
                // F2(修复轮):before 缺记(null,无 not_started)不构成
                // 佐证——仪器缺口必须红,不得以 after 冒充 before。
                // M5/F4(修复轮):not_applicable 窗口(场景 launch 不
                // 绑定审计分区)的计数不作证据——0==0 无信息;该窗口的
                // 零 spawn 证据由五维会计+拒绝探针承担(上方已判),
                // 标注本身与冻结绑定清单的交叉核对在 verify_failure_cell。
                let not_applicable = payload["audit_window"].as_str() == Some("not_applicable");
                let audits_equal = payload["not_started"].as_bool() == Some(true)
                    || (payload["provider_start_audits_before"].as_u64().is_some()
                        && payload["provider_start_audits_before"].as_u64()
                            == payload["provider_start_audits_after"].as_u64());
                not_applicable || audits_equal
            }
            Some("launch_rejected_probe") => payload["refused"].as_bool() == Some(true),
            _ => false,
        }
    });
    if !observed_zero {
        return Err(format!(
            "零 spawn 格 {} 仅有单维 provider_spawn_count=0,缺五维会计佐证\
             (zero_spawn_observation 前后审计相等/未启动 或 launch_rejected_probe.refused)",
            cell.run_ref
        ));
    }
    Ok(())
}

/// M5 失败关闭:17 固定负向场景逐格三面(真实拒绝 或 合法 unknown 且带
/// unobservable gate+非空原因;拒绝探针 refused/gate + audits 前后——
/// 仅对绑定 sid 的场景判等,未绑定场景 not_applicable 不判等;五维)。
fn verify_failure_scenarios(cells: &[EvidenceCell]) -> Result<(), String> {
    let failure_cells: Vec<&EvidenceCell> = cells
        .iter()
        .filter(|cell| {
            cell.boundary_matrix_evidence_flags()
                .is_some_and(|flags| flags.contains_key("failure_scenario"))
        })
        .collect();
    if failure_cells.len() != FIXED_FAILURE_SCENARIOS.len() {
        return Err(format!(
            "M5 失败关闭:负向场景格数({})必须等于固定清单({})",
            failure_cells.len(),
            FIXED_FAILURE_SCENARIOS.len()
        ));
    }
    for cell in failure_cells {
        verify_failure_cell(cell)?;
    }
    Ok(())
}

fn verify_failure_cell(cell: &EvidenceCell) -> Result<(), String> {
    let scenario = cell
        .boundary_matrix_evidence_flags()
        .and_then(|flags| {
            flags
                .get("failure_scenario")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .ok_or_else(|| "失败场景格缺 failure_scenario flag".to_string())?;
    let probe = cell
        .provider_events
        .iter()
        .find(|event| event["event"]["type"].as_str() == Some("launch_rejected_probe"))
        .map(|event| &event["event"])
        .ok_or_else(|| format!("失败场景 {scenario} 缺拒绝探针(launch_rejected_probe)"))?;
    let refusal_gate = probe["refusal_gate"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .to_string();
    let refused = probe["refused"].as_bool().unwrap_or(false);
    let reason = cell.denied_reason.as_deref().unwrap_or_default().trim();
    match cell.capability_state.as_str() {
        "denied" => {
            if !refused {
                return Err(format!(
                    "失败场景 {scenario}:denied 但拒绝探针 refused=false({reason:?})"
                ));
            }
            if refusal_gate.is_empty() {
                return Err(format!("失败场景 {scenario}:拒绝探针 refusal_gate 为空"));
            }
        }
        "unknown" => {
            if refusal_gate != "unobservable" {
                return Err(format!(
                    "失败场景 {scenario}:unknown 必须带 unobservable gate,实际 {refusal_gate:?}"
                ));
            }
            if reason.is_empty() {
                return Err(format!("失败场景 {scenario}:unknown 缺非空原因"));
            }
        }
        other => {
            return Err(format!(
                "失败场景 {scenario}:状态 {other} 非法(只允许真实拒绝 denied 或合法 unknown)"
            ));
        }
    }
    let observation = cell
        .provider_events
        .iter()
        .find(|event| event["event"]["type"].as_str() == Some("zero_spawn_observation"))
        .map(|event| &event["event"])
        .ok_or_else(|| format!("失败场景 {scenario} 缺 zero_spawn_observation(审计前后)"))?;
    // M5/F4(修复轮):audits 相等仅对绑定 sid 的场景是零 spawn 证据;
    // 未绑定场景(validate/能力分格面拒绝,launch 从不落在合成 sid 分区)
    // 的相等是 0==0 无信息——标注 not_applicable,判等不适用,零 spawn
    // 由五维会计+拒绝探针承担。标注与冻结绑定清单交叉核对,防止证据
    // 侧随意声明窗口逃避判等或伪造证据口径。
    let binds_session = task11_failure_scenario_binds_session(&scenario);
    match observation["audit_window"].as_str().unwrap_or_default() {
        "session" => {
            if !binds_session {
                return Err(format!(
                    "失败场景 {scenario}:launch 不绑定 sid,审计分区恒空——\
                     不得声明 session 审计窗口(0==0 无信息;须标注 not_applicable)"
                ));
            }
            let before = observation["provider_start_audits_before"].as_u64();
            let after = observation["provider_start_audits_after"].as_u64();
            if before != after {
                return Err(format!(
                    "失败场景 {scenario}:拒绝后 audits 变化(before={before:?} after={after:?},零 spawn 不成立)"
                ));
            }
        }
        "not_applicable" => {
            if binds_session {
                return Err(format!(
                    "失败场景 {scenario}:launch 绑定 sid,审计前后对照是零 spawn\
                     证据——不得标注 not_applicable 逃避判等"
                ));
            }
            // 审计分区与场景 launch 无绑定:前后计数仅记录,不作证据。
        }
        other => {
            return Err(format!(
                "失败场景 {scenario}:zero_spawn_observation 缺 audit_window 标注\
                 ({other:?};session/not_applicable 必须如实声明)"
            ));
        }
    }
    verify_zero_spawn_evidence(cell)?;
    // 被拒格:除 session child 外,协议握手与 MCP/native 方法维也不得在场
    //(失败矩阵五维口径;握手失败已创建 child 的 kill/reap 格不属被拒格)。
    let cell_json = cell.to_cell_json();
    for key in ["native_handshake_count"] {
        if cell_accounting_dimension(&cell_json, key) != 0 {
            return Err(format!(
                "失败场景 {scenario}:被拒格 {key}={} 非零(握手/method 不应发生在拒绝格)",
                cell_accounting_dimension(&cell_json, key)
            ));
        }
    }
    Ok(())
}

/// 六职责域覆盖断言(M6 由 danger 探针承担,此处只记录归属)。
///
/// 返回该 provider 的职责域摘要(roles-matrix.json 的单家段)。
fn b45_t23_assert_role_coverage(
    provider: &ProviderName,
    matrix: &LiveMatrixEvidence,
    boundary: &LiveMatrixEvidence,
) -> Value {
    let run_mode = super::snapshot::RunMode::from_env()
        .expect("LIVE_MATRIX_RUN_MODE 已在 run_provider_matrix 入口校验");
    let resume_round = matches!(run_mode, super::snapshot::RunMode::ResumeFromPlanSnapshot);
    let confirmed = matrix.confirmed_cells();
    let stage_confirmed = |stage: &str| confirmed.iter().filter(|cell| cell.stage == stage).count();

    // 轮次覆盖(§1.1 ②术语修正):resume 轮=4 承继 + coding/review 各 1
    // executed-resume(native 同 ID);full_chain 轮=逐 stage(plan 两入口)
    // 至少 1 个 fresh 执行格。负测:删必需格/拿 carried 冒充 resume→红。
    if let Err(error) = verify_round_coverage(resume_round, matrix.cells()) {
        panic!("M1/M7 轮次覆盖断言失败:{error}");
    }
    // spawn 仪器(§1.1 ③):逐阶段 before/after 计数;fresh/执行 resume≥1、
    // split_sync resume=0;零 spawn 格必须五维会计 + 可证伪观测。
    if let Err(error) = verify_spawn_instrumentation(matrix.cells()) {
        panic!("spawn 仪器断言失败:{error}");
    }
    // M2 Coder / M3 Reviewer:真实执行轮必须有 confirmed 格。
    assert!(
        stage_confirmed("coding") > 0,
        "M2 Coder:coding 必须有真实 confirmed 格"
    );
    assert!(
        stage_confirmed("review") > 0,
        "M3 Reviewer:review 必须有真实 confirmed 格"
    );
    // confirmed 格全要素(Step 1 断言组已在 validate_against 承担,此处
    // 抽查 M 矩阵要素:CLI exact version 与 argv/wire 捕获不得空壳)。
    for cell in confirmed {
        assert!(
            !cell.exact_version.trim().is_empty(),
            "confirmed 格缺 CLI exact version({}/{})",
            cell.stage,
            cell.entrypoint
        );
        assert!(
            cell.argv_or_wire_capture_exists,
            "confirmed 格缺 argv/wire 捕获({}/{})",
            cell.stage, cell.entrypoint
        );
    }

    let boundary_refs: Vec<&str> = boundary
        .cells()
        .iter()
        .map(|cell| cell.run_ref.as_str())
        .collect();
    // M4 group/repair:边界段 coding 正向(受控 commit+正向宿主写)、
    // 受控写保护面与 D4 前后快照——逐格按状态断言 confirmed(受控写
    // 双拒/D4 无漂移),不以格存在冒充。
    assert!(
        boundary
            .cells()
            .iter()
            .any(|cell| cell.stage == "coding" && cell.capability_state == "confirmed"),
        "M4 group/repair:边界矩阵缺 coding 正向 confirmed 格"
    );
    for (label, prefix) in [
        ("受控写保护面", "t11-boundary-protected-writes-"),
        ("D4 前后快照", "t11-boundary-d4-"),
    ] {
        let cell = boundary
            .cells()
            .iter()
            .find(|cell| cell.run_ref.starts_with(prefix))
            .unwrap_or_else(|| panic!("M4 group/repair:缺 {label}格({prefix}*)"));
        assert_eq!(
            cell.capability_state, "confirmed",
            "M4 group/repair:{label}格必须 confirmed(受控写双拒/D4 无漂移);\
             denied_reason={:?}",
            cell.denied_reason
        );
    }
    // M5 失败关闭(§1.1 ④):17 固定负向场景逐格——真实拒绝(denied +
    // refused=true + gate 非空)或合法 unknown(unobservable gate + 非空
    // 原因);拒绝探针 audits 前后相等;五维会计复用 failure_matrix 口径。
    if let Err(error) = verify_failure_scenarios(boundary.cells()) {
        panic!("M5 失败关闭断言失败:{error}");
    }
    let expected_failures = FIXED_FAILURE_SCENARIOS.len();
    let failure_cells: Vec<&EvidenceCell> = boundary
        .cells()
        .iter()
        .filter(|cell| {
            cell.boundary_matrix_evidence_flags()
                .is_some_and(|flags| flags.contains_key("failure_scenario"))
        })
        .collect();
    assert_eq!(
        failure_cells.len(),
        expected_failures,
        "M5 失败关闭:负向场景格数({})必须等于固定清单({expected_failures}),不删格",
        failure_cells.len()
    );

    json!({
        "provider": format!("{provider:?}"),
        "run_mode": format!("{run_mode:?}"),
        "matrix_cells": matrix.cells().iter().map(b45_t23_cell_summary).collect::<Vec<_>>(),
        "boundary_cells": boundary.cells().iter().map(b45_t23_cell_summary).collect::<Vec<_>>(),
        "matrix_rejections": matrix
                .rejections
                .iter()
                .map(|rejection| json!({
                    "stage": rejection.stage,
                    "entrypoint": rejection.entrypoint,
                    "fresh_or_resume": rejection.fresh_or_resume,
                    "reason": rejection.reason,
                }))
                .collect::<Vec<_>>(),
        "duties": {
            "m1_author": ["story", "design", "plan(streaming+split_sync)"],
            "m2_coder": ["coding"],
            "m3_reviewer": ["review"],
            "m4_group_repair": boundary_refs,
            "m5_fail_closed": format!("{} scenarios", failure_cells.len()),
            "m6_codex_danger": "b45_live_codex_danger_apply_patch",
            "m7_lifecycle": if resume_round { "resume(same native id)" } else { "fresh(spawn counting)" },
        },
    })
}

/// Step 3 真实执行:四家×六职责×Direct/LC×fresh/resume/retry 矩阵。
#[tokio::test]
#[ignore = "b45 真实现场:LC_GATEWAY_E2E=1 且 Step 3 命令执行(四家×六职责;需要真实 CLI/trust/#8 政策就绪;B45_T23_PROVIDERS 可分批)"]
async fn b45_live_four_provider_roles() {
    require_lc_gateway_e2e_switch();
    let providers = b45_t23_providers_from_env();
    assert!(
        !providers.is_empty(),
        "B45_T23_PROVIDERS 至少一家(claude/codex/pi/kimi)"
    );
    for provider in providers {
        // (a) 五阶段矩阵:M1/M2/M3/M7(Author/Coder/Reviewer/生命周期)。
        let matrix_root = b45_t23_evidence_root(&provider, "matrix");
        let matrix = match LiveLcGatewayHarness::new()
            .run_provider_matrix(provider.clone(), &matrix_root)
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
        // (b) 边界+失败关闭:M4/M5(group/repair 与 17 负向场景)。
        let boundary = match LiveLcGatewayHarness::new()
            .run_boundary_and_failure_matrix(provider.clone())
            .await
        {
            Ok(boundary) => boundary,
            Err(failure) => panic!(
                "run_boundary_and_failure_matrix 现场执行失败 [{}]{}: {} (环境不可运行须报告 BLOCKED,不删格)",
                failure.reason_code,
                failure
                    .stage
                    .map(|stage| format!("/{stage}"))
                    .unwrap_or_default(),
                failure.message
            ),
        };
        let provider_summary = b45_t23_assert_role_coverage(&provider, &matrix, &boundary);
        // 类1′ ①:每家立即落本家汇总(b45-t23-roles-matrix-<provider>.json,
        // 证据根含可选 run-id);不跨家合并、进程末尾不再整文件重写,
        // 异家并行/同家多轮互不覆盖。
        b45_t23_write_provider_summary(&provider, &provider_summary);
    }
}

// ---------------------------------------------------------------------------
// M6:codex danger-full-access 双审批真机探针(on-request/never)
// ---------------------------------------------------------------------------

/// codex danger 探针 fixture 产物根(计划:测试 fixture 在 /tmp/aria-bug45/t23/)。
const B45_T23_FIXTURE_ROOT: &str = "/tmp/aria-bug45/t23";

/// 受控 apply_patch 目标文件内容(前后标记,供 diff 观察)。
const PATCH_TARGET_BEFORE: &str = "before-b45-t23\n";
const PATCH_TARGET_AFTER: &str = "after-b45-t23\n";

/// 危险档位标签(固定 CLI 版本下分别执行)。
#[derive(Clone, Copy, PartialEq, Eq)]
enum B45DangerProfile {
    /// 岗位纪律档:deny 投影恒 on-request(T13 检测型纪律)。
    OnRequest,
    /// Coder 原生档:Auto 恒 never(REQ-LCG-04 改版)。
    Never,
}

impl B45DangerProfile {
    fn label(self) -> &'static str {
        match self {
            Self::OnRequest => "danger-onrequest",
            Self::Never => "danger-never",
        }
    }
    fn permission_mode(self) -> ProviderPermissionMode {
        match self {
            // 纪律档 approvalPolicy 恒 on-request(codex_launch_params);
            // permission_mode 取值不影响该档映射。never 档 Auto→never。
            Self::OnRequest | Self::Never => ProviderPermissionMode::Auto,
        }
    }
    fn tool_policy(self) -> Option<ProviderToolPolicy> {
        match self {
            Self::OnRequest => Some(ProviderToolPolicy::deny_file_write_builtins()),
            Self::Never => None,
        }
    }
}

/// 事件流观察记录(审批帧/fileChange/中断/durable 的原始事实)。
#[derive(Default)]
struct B45DangerObservation {
    permission_requests: Vec<String>,
    file_change_warnings: Vec<String>,
    tool_policy_decisions: Vec<String>,
    executions: usize,
    text_deltas: usize,
    terminal: Option<String>,
    event_log: Vec<Value>,
}

/// 建隔离 fixture:小 git 仓+受控 apply_patch 目标文件。
fn b45_danger_fixture(label: &str) -> PathBuf {
    let root = PathBuf::from(B45_T23_FIXTURE_ROOT).join(label);
    // 每次真机轮重建隔离 fixture(避免上次残留导致空提交失败)。
    if root.exists() {
        std::fs::remove_dir_all(&root).expect("b45 danger fixture 旧目录清理");
    }
    std::fs::create_dir_all(&root).expect("b45 danger fixture dir");
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).expect("b45 danger repo dir");
    std::fs::write(repo.join("patch_target.md"), PATCH_TARGET_BEFORE).expect("patch target before");
    for arguments in [
        vec![
            "init".to_string(),
            "-q".to_string(),
            "--initial-branch=main".to_string(),
        ],
        vec!["add".to_string(), "-A".to_string()],
        vec![
            "commit".to_string(),
            "-q".to_string(),
            "-m".to_string(),
            "b45 danger fixture baseline".to_string(),
        ],
    ] {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args([
                "-c",
                "user.email=b45-t23@example.invalid",
                "-c",
                "user.name=b45 t23 fixture",
            ])
            .args(&arguments)
            .status()
            .expect("b45 danger fixture git spawns");
        assert!(
            status.success(),
            "b45 danger fixture git {arguments:?} must succeed"
        );
    }
    repo
}

/// 解析 codex CLI 实际路径与 exact version(固定 CLI 版本记录)。
fn b45_codex_cli() -> (PathBuf, String) {
    let bin =
        PathBuf::from(std::env::var("B45_T23_CODEX_BIN").unwrap_or_else(|_| "codex".to_string()));
    let output = std::process::Command::new(&bin)
        .arg("--version")
        .output()
        .expect("codex --version spawns(需 codex CLI 在 PATH 或 B45_T23_CODEX_BIN)");
    assert!(
        output.status.success(),
        "codex --version 失败:{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert!(!version.is_empty(), "codex --version 输出为空");
    (bin, version)
}

/// 单档真实执行:danger-full-access 下受控 apply_patch,收集审批有无/
/// fileChange 事实/中断/durable 的原始帧。
async fn b45_run_danger_profile(
    profile: B45DangerProfile,
    bin: &Path,
) -> (B45DangerObservation, PathBuf, PathBuf, Option<String>) {
    let repo = b45_danger_fixture(profile.label());
    let fixture_root = repo.parent().expect("fixture root").to_path_buf();
    // durable 审计落 fixture 内 .aria(不触碰真实 HOME/store)。
    let app_paths = ProductAppPaths::new(fixture_root.join(".aria"));
    let lifecycle = Arc::new(LifecycleStore::new(app_paths.clone()));
    let workspace_session_id = format!("b45t23{}", profile.label().trim_start_matches("danger-"));
    let input = StreamingProviderInput {
        provider_type: ProviderType::Codex,
        // 纪律档=Author 岗位承载角色(Orchestrator,deny 守卫唯一合法面
        // 之一);never 档=Coder 原生 Executor。
        role: match profile {
            B45DangerProfile::OnRequest => AdapterRole::Orchestrator,
            B45DangerProfile::Never => AdapterRole::Executor,
        },
        prompt: "Use the apply_patch tool to edit patch_target.md in the current \
                 directory: replace the line 'before-b45-t23' with 'after-b45-t23'. \
                 Then reply with exactly DONE."
            .to_string(),
        working_dir: repo.clone(),
        working_directory: None,
        workspace_session_id: Some(workspace_session_id.clone()),
        resume_provider_session_id: None,
        permission_mode: profile.permission_mode(),
        tool_policy: profile.tool_policy(),
        audit_sink: Some(
            RoleRunBoundAuditSink::new(lifecycle.clone(), workspace_session_id.clone(), 1)
                .into_sink(),
        ),
        structured_output_contract: None,
        // adapter PATH 冻结契约(批2 Task 3):不回退宿主 PATH——显式
        // 注入当前进程 PATH,使 codex CLI 可解析。
        env_vars: BTreeMap::from([(
            "PATH".to_string(),
            std::env::var("PATH").unwrap_or_default(),
        )]),
        timeout_secs: 300,
        baseline_tree: None,
    };
    let adapter = CodexProvider::new(bin.to_path_buf());
    let cancel = CancellationToken::new();
    let start_outcome = tokio::time::timeout(
        std::time::Duration::from_secs(360),
        adapter.start(input, cancel),
    )
    .await
    .expect("b45 danger start 不得悬置(360s 界)");

    let mut observation = B45DangerObservation::default();
    let audit_partition = app_paths
        .root()
        .join(TOOL_POLICY_RUN_AUDIT_PARTITION)
        .join(&workspace_session_id);
    let start_error: Option<String> = match start_outcome {
        // 检测型纪律:fileChange 事实 → 事件 → durable → 中断 turn → start Err。
        Err(error) => {
            observation.terminal = Some(format!("start_error: {error}"));
            Some(format!("{error}"))
        }
        Ok(mut session) => {
            let deadline = std::time::Duration::from_secs(360);
            loop {
                let event = match tokio::time::timeout(deadline, session.events.recv()).await {
                    Ok(Some(event)) => event,
                    Ok(None) => break,
                    Err(_) => break,
                };
                let record = match &event {
                    ProviderEvent::TextDelta { content } => {
                        observation.text_deltas += 1;
                        json!({"kind": "text_delta", "chars": content.len()})
                    }
                    ProviderEvent::PermissionRequest(request) => {
                        observation
                            .permission_requests
                            .push(request.tool_name.clone());
                        json!({"kind": "permission_request", "tool": request.tool_name})
                    }
                    ProviderEvent::Execution(_) => {
                        observation.executions += 1;
                        json!({"kind": "execution"})
                    }
                    ProviderEvent::ToolCall(call) => {
                        json!({"kind": "tool_call", "tool": call.tool_name})
                    }
                    ProviderEvent::ToolResult(result) => {
                        json!({"kind": "tool_result", "tool_id": result.tool_use_id})
                    }
                    ProviderEvent::ToolPolicyWarning(warning) => {
                        observation
                            .file_change_warnings
                            .push(warning.reason_code.clone());
                        json!({
                            "kind": "tool_policy_warning",
                            "reason_code": warning.reason_code,
                            "method": warning.method,
                        })
                    }
                    ProviderEvent::ToolPolicyDecision(decision) => {
                        observation
                            .tool_policy_decisions
                            .push(decision.decision.to_string());
                        json!({
                            "kind": "tool_policy_decision",
                            "category": decision.category,
                            "decision": decision.decision,
                        })
                    }
                    ProviderEvent::ToolPolicyTerminated(termination) => {
                        json!({
                            "kind": "tool_policy_terminated",
                            "reason_code": termination.reason_code,
                        })
                    }
                    ProviderEvent::Completed(_) => {
                        observation.terminal = Some("completed".to_string());
                        json!({"kind": "completed"})
                    }
                    ProviderEvent::Failed { message } => {
                        observation.terminal = Some(format!("failed: {message}"));
                        json!({"kind": "failed", "message": message})
                    }
                    ProviderEvent::StatusChanged(status) => {
                        json!({"kind": "status", "status": format!("{status:?}")})
                    }
                    ProviderEvent::ProtocolError { code, message, .. } => {
                        json!({"kind": "protocol_error", "code": code, "message": message})
                    }
                    ProviderEvent::PermissionTimeout { permission_id } => {
                        json!({"kind": "permission_timeout", "permission_id": permission_id})
                    }
                    ProviderEvent::UsageReport(_) => json!({"kind": "usage"}),
                    ProviderEvent::TerminalActivity(_) => json!({"kind": "terminal_activity"}),
                    ProviderEvent::ChoiceRequest(choice) => {
                        json!({"kind": "choice_request", "source": format!("{:?}", choice.source)})
                    }
                };
                observation.event_log.push(record);
                if matches!(
                    event,
                    ProviderEvent::Completed(_)
                        | ProviderEvent::Failed { .. }
                        | ProviderEvent::ToolPolicyTerminated(_)
                ) {
                    break;
                }
            }
            None
        }
    };
    (observation, repo, audit_partition, start_error)
}

/// 读取 durable 审计分区全部 jsonl 行(原始 durable 事实)。
fn b45_read_audit_lines(partition: &Path) -> Vec<String> {
    let mut lines = Vec::new();
    let Ok(entries) = std::fs::read_dir(partition) else {
        return lines;
    };
    for entry in entries.flatten() {
        if entry.path().extension().is_some_and(|ext| ext == "jsonl")
            && let Ok(content) = std::fs::read_to_string(entry.path())
        {
            lines.extend(content.lines().map(str::to_string));
        }
    }
    lines
}

/// Step 4 真实执行:固定 CLI 版本,danger-full-access 下 on-request/never
/// 两个隔离 fixture 受控 apply_patch。
#[tokio::test]
#[ignore = "b45 真实现场:LC_GATEWAY_E2E=1 且 Step 4 命令执行(需要 codex CLI 登录态;固定实际 CLI 版本)"]
async fn b45_live_codex_danger_apply_patch() {
    require_lc_gateway_e2e_switch();
    let (bin, version) = b45_codex_cli();

    // ---- 场景 A:on-request(岗位纪律档)----
    let (onrequest, onrequest_repo, onrequest_audit, onrequest_start_error) =
        b45_run_danger_profile(B45DangerProfile::OnRequest, &bin).await;
    let onrequest_diff =
        std::fs::read_to_string(onrequest_repo.join("patch_target.md")).unwrap_or_default();
    let onrequest_audit_lines = b45_read_audit_lines(&onrequest_audit);

    // ---- 场景 B:never(Coder 原生档)----
    let (never, never_repo, never_audit, never_start_error) =
        b45_run_danger_profile(B45DangerProfile::Never, &bin).await;
    let never_diff =
        std::fs::read_to_string(never_repo.join("patch_target.md")).unwrap_or_default();
    let never_audit_lines = b45_read_audit_lines(&never_audit);

    // 证据落盘(审批帧/fileChange/diff/中断/durable 全量原始事实)。
    let evidence_root = PathBuf::from(B45_T23_FIXTURE_ROOT);
    std::fs::create_dir_all(&evidence_root).expect("b45 danger evidence dir");
    let evidence = json!({
        "cli": {"bin": bin.display().to_string(), "version": version},
        "sandbox": "danger-full-access",
        "on_request": {
            "approval_policy": "on-request(discipline)",
            "start_error": onrequest_start_error,
            "terminal": onrequest.terminal,
            "permission_requests": onrequest.permission_requests,
            "file_change_warnings": onrequest.file_change_warnings,
            "tool_policy_decisions": onrequest.tool_policy_decisions,
            "executions": onrequest.executions,
            "text_deltas": onrequest.text_deltas,
            "patch_target_after": onrequest_diff,
            "patch_applied": onrequest_diff == PATCH_TARGET_AFTER,
            "durable_audit_lines": onrequest_audit_lines,
            "events": onrequest.event_log,
        },
        "never": {
            "approval_policy": "never",
            "start_error": never_start_error,
            "terminal": never.terminal,
            "permission_requests": never.permission_requests,
            "file_change_warnings": never.file_change_warnings,
            "tool_policy_decisions": never.tool_policy_decisions,
            "executions": never.executions,
            "text_deltas": never.text_deltas,
            "patch_target_after": never_diff,
            "patch_applied": never_diff == PATCH_TARGET_AFTER,
            "durable_audit_lines": never_audit_lines,
            "events": never.event_log,
        },
    });
    std::fs::write(
        evidence_root.join("codex-danger-apply-patch.json"),
        serde_json::to_vec_pretty(&evidence).expect("danger evidence json"),
    )
    .expect("danger evidence 落盘");

    // ---- 判绿(红线:事件不可观测或不能可靠中断=机制未通过)----
    // 场景 A(on-request 纪律):fileChange 必须可观测(事件通道 warning 或
    // FileChange 审批请求至少其一;danger 下 apply_patch 可能无审批帧,
    // item 通知是检测型触发面)。
    let onrequest_file_change_observable = !onrequest.file_change_warnings.is_empty()
        || onrequest
            .tool_policy_decisions
            .iter()
            .any(|decision| decision == "decline")
        || !onrequest.permission_requests.is_empty();
    assert!(
        onrequest_file_change_observable,
        "on-request 档 fileChange 不可观测(无 warning/审批帧/决策事件)——\
         该 CLI 版本({version})apply_patch 机制未通过,不以「审批没来所以被拒\
         成功」交差"
    );
    // 场景 A:检测型纪律必须 durable(file_change_detected 落审计分区)。
    assert!(
        onrequest_audit_lines
            .iter()
            .any(|line| line.contains("file_change_detected")),
        "on-request 档 durable 审计缺 file_change_detected(事件→durable→中断\
         次序断裂)"
    );
    // 场景 A:turn 必须被中断(终态=start Err 携带 author_file_change_
    // detected,或事件流 ToolPolicyTerminated/Failed)——本轮不可采信。
    let onrequest_interrupted = onrequest_start_error
        .as_deref()
        .is_some_and(|message| message.contains(AUTHOR_FILE_CHANGE_DETECTED))
        || onrequest.event_log.iter().any(|event| {
            event.get("kind").and_then(Value::as_str) == Some("tool_policy_terminated")
        })
        || onrequest
            .terminal
            .as_deref()
            .is_some_and(|terminal| terminal.starts_with("failed:"));
    assert!(
        onrequest_interrupted,
        "on-request 档未可靠中断(turn 未失败)——检测型纪律红线;start_error=\
         {:?}, terminal={:?}",
        onrequest_start_error, onrequest.terminal
    );
    // 场景 B(never):审批必须缺席(never 档无审批请求;出现即映射失真)。
    assert!(
        never.permission_requests.is_empty(),
        "never 档出现审批请求({:?})——approvalPolicy=never 映射失真",
        never.permission_requests
    );
    // 场景 B:fileChange 可观测性——apply_patch 必须真实执行并生效:diff
    // 变更且事件流有工具/执行活动;仅"有事件"不构成 apply_patch 生效
    // 证据(文本输出恒有事件,不可证伪)。
    let never_observable = never_diff == PATCH_TARGET_AFTER && never.executions > 0;
    assert!(
        never_observable,
        "never 档 apply_patch 未真实执行生效(diff={}==after? {}, executions={})\
         ——该 CLI 版本({version})机制未通过",
        never_diff.trim_end(),
        never_diff == PATCH_TARGET_AFTER,
        never.executions
    );
    // 场景 B:非纪律会话不中断(终态完成;若 Failed 如实记录,由判读面
    // 裁决,此处要求事件事实完整落盘——已由 evidence 落盘承载)。
}

// ---------------------------------------------------------------------------
// 类 1 T23 断言加强负测(纯逻辑面;不需要真实 CLI/LC_GATEWAY_E2E)。
// 先红后绿:负测钉死「删必需格 / carried 冒充 resume / 单维 0 / 拒绝后
// audits 变化 / unknown 无 gate」的拒绝形态,实现转绿后由定向测试守护。
// ---------------------------------------------------------------------------

#[cfg(test)]
mod t23_assertion_tests {
    use super::*;
    use crate::web_lc_gateway_multi_provider::snapshot::{
        SNAPSHOT_MANIFEST_SCHEMA_VERSION, SNAPSHOT_POINT_PLAN_CONFIRMED, SnapshotManifest,
        snapshots_root,
    };

    fn test_manifest(source_root: &str) -> SnapshotManifest {
        SnapshotManifest {
            schema_version: SNAPSHOT_MANIFEST_SCHEMA_VERSION,
            snapshot_point: SNAPSHOT_POINT_PLAN_CONFIRMED.to_string(),
            provider_wire: "claude_code".to_string(),
            cli_version: "claude 2.1.283".to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            lc_id: "lc_0001".to_string(),
            plan_id: "plan_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            member_git_head: "deadbeef".to_string(),
            capability_digest: "sha256:cap".to_string(),
            policy_digest: "sha256:policy".to_string(),
            harness_revision: "rev-1".to_string(),
            workspace_digest: "sha256:ws".to_string(),
            aggregate_digest: "sha256:agg".to_string(),
            source_evidence_root: source_root.to_string(),
            quiescent: true,
            created_at: "2026-10-19T00:00:00Z".to_string(),
        }
    }

    /// 基线格:confirmed、真实执行、spawn=1、native 齐全(可做负测变体)。
    fn base_cell(stage: &str, entrypoint: &str, phase: &str) -> EvidenceCell {
        EvidenceCell {
            provider: ProviderName::ClaudeCode,
            exact_version: "claude 2.1.283".to_string(),
            stage: stage.to_string(),
            entrypoint: entrypoint.to_string(),
            fresh_or_resume: phase.to_string(),
            process_cwd: PathBuf::from("/tmp/t23-root"),
            target: PathBuf::from("/tmp/t23-root/alpha"),
            audit_projection_digest: "sha256:p".to_string(),
            frozen_projection_digest: "sha256:p".to_string(),
            native_resume_confirmed_id: None,
            native_resume_reattached: false,
            requested_resume_id: None,
            argv_or_wire_capture_exists: true,
            approval_and_tool_events_exist: true,
            completed_product_artifact_exists: true,
            run_ref: format!("run-{stage}-{phase}"),
            run_ref_is_unique_within_entrypoint: true,
            action: "coding_target_write".to_string(),
            role: "executor".to_string(),
            gateway_dialect: "claude_code_cli_v1".to_string(),
            wire_dialect: "claude-stream-json".to_string(),
            native_session_id: "native-1".to_string(),
            workspace_session_id: "ws-1".to_string(),
            argv: vec!["claude".to_string()],
            capability_state: "confirmed".to_string(),
            denied_reason: None,
            provider_pid: Some("4242".to_string()),
            pid_unavailable_reason: None,
            provider_spawn_count: 1,
            session_projection_digest: "sha256:s".to_string(),
            provider_events: Vec::new(),
            execution_origin: "full_chain".to_string(),
            carried_source_evidence_root: None,
            carried_workspace_digest: String::new(),
            carried_aggregate_digest: String::new(),
            carried_capability_digest: String::new(),
            carried_policy_digest: String::new(),
        }
    }

    fn carried_test_cell(
        stage: &str,
        entrypoint: &str,
        manifest: &SnapshotManifest,
    ) -> EvidenceCell {
        carried_cell(stage, &ProviderName::ClaudeCode, manifest, "snap-1")
            .tap_entrypoint(entrypoint)
    }

    /// 小助手:carried_cell 按 stage 决定入口,测试需要显式入口。
    trait TapEntrypoint {
        fn tap_entrypoint(self, entrypoint: &str) -> Self;
    }
    impl TapEntrypoint for EvidenceCell {
        fn tap_entrypoint(mut self, entrypoint: &str) -> Self {
            self.entrypoint = entrypoint.to_string();
            self
        }
    }

    fn executed_resume_cell(stage: &str) -> EvidenceCell {
        let mut cell = base_cell(stage, ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT, RESUME);
        cell.execution_origin = "snapshot_boot".to_string();
        cell.requested_resume_id = Some("native-7".to_string());
        cell.native_resume_confirmed_id = Some("native-7".to_string());
        cell
    }

    fn resume_round_cells(manifest: &SnapshotManifest) -> Vec<EvidenceCell> {
        let mut cells = vec![
            carried_test_cell("story", ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT, manifest),
            carried_test_cell(
                "design",
                ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT,
                manifest,
            ),
            carried_test_cell("plan", ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT, manifest),
            carried_test_cell("split", ENTRYPOINT_SPLIT_SYNC, manifest),
        ];
        for stage in ["coding", "review"] {
            let mut fresh = base_cell(stage, ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT, FRESH);
            fresh.execution_origin = "snapshot_boot".to_string();
            cells.push(fresh);
            cells.push(executed_resume_cell(stage));
        }
        cells
    }

    fn full_chain_cells() -> Vec<EvidenceCell> {
        FULL_CHAIN_FRESH_STAGES
            .iter()
            .flat_map(|(stage, entrypoint)| {
                [
                    base_cell(stage, entrypoint, FRESH),
                    base_cell(stage, entrypoint, RESUME),
                ]
            })
            .collect()
    }

    #[test]
    fn t23_carried_source_chain_requires_existing_root_and_matching_digests() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source_root = temp.path().join("evidence").join("matrix");
        std::fs::create_dir_all(&source_root).expect("source root");
        let snapshot_directory = temp
            .path()
            .join("evidence")
            .join("snapshots")
            .join("snap-1");
        std::fs::create_dir_all(&snapshot_directory).expect("snapshot dir");
        let manifest = test_manifest(&source_root.to_string_lossy());
        std::fs::write(
            snapshot_directory.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).expect("manifest json"),
        )
        .expect("write manifest");
        let canonical = PathBuf::from("/tmp/t23-root");
        let member = canonical.join("alpha");

        let cell = carried_test_cell(
            "story",
            ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT,
            &manifest,
        );
        assert!(
            cell.validate_against(&ProviderName::ClaudeCode, &canonical, &member)
                .is_ok(),
            "来源链可复核的承继格必须通过: {:?}",
            cell.validate_against(&ProviderName::ClaudeCode, &canonical, &member)
        );

        let mut tampered = cell.clone();
        tampered.carried_capability_digest = "sha256:tampered".to_string();
        assert!(
            tampered
                .validate_against(&ProviderName::ClaudeCode, &canonical, &member)
                .is_err(),
            "摘要不符必须拒绝"
        );

        let mut missing_root = cell.clone();
        missing_root.carried_source_evidence_root =
            Some(temp.path().join("absent").to_string_lossy().to_string());
        assert!(
            missing_root
                .validate_against(&ProviderName::ClaudeCode, &canonical, &member)
                .is_err(),
            "来源证据根不存在必须拒绝"
        );

        let mut missing_digest = cell.clone();
        missing_digest.carried_policy_digest = String::new();
        assert!(
            missing_digest
                .validate_against(&ProviderName::ClaudeCode, &canonical, &member)
                .is_err(),
            "承继格缺 manifest 摘要必须拒绝"
        );

        std::fs::remove_dir_all(&snapshot_directory).expect("remove snapshot");
        assert!(
            cell.validate_against(&ProviderName::ClaudeCode, &canonical, &member)
                .is_err(),
            "来源快照 manifest 不可复核必须拒绝"
        );
    }

    #[test]
    fn t23_resume_round_requires_carried_references_and_executed_resume() {
        let manifest = test_manifest("/tmp/unused-source-root");
        let cells = resume_round_cells(&manifest);
        assert!(
            verify_round_coverage(true, &cells).is_ok(),
            "合法 resume 轮必须通过: {:?}",
            verify_round_coverage(true, &cells)
        );

        // 删必需承继格 → 红。
        let mut missing_carried = cells.clone();
        missing_carried
            .retain(|cell| !(cell.stage == "split" && cell.capability_state == "carried"));
        assert!(
            verify_round_coverage(true, &missing_carried).is_err(),
            "删 split 承继格必须拒绝"
        );

        // carried 冒充 executed-resume → 红。
        let mut masquerade = cells.clone();
        for cell in &mut masquerade {
            if cell.stage == "coding" && cell.fresh_or_resume == RESUME {
                cell.execution_origin = "carried:snap-1".to_string();
            }
        }
        let error = verify_round_coverage(true, &masquerade).expect_err("carried 冒充必须拒绝");
        assert!(error.contains("冒充"), "{error}");

        // executed-resume native ID 不一致 → 红。
        let mut mismatch = cells.clone();
        for cell in &mut mismatch {
            if cell.stage == "review" && cell.fresh_or_resume == RESUME {
                cell.native_resume_confirmed_id = Some("native-other".to_string());
            }
        }
        assert!(
            verify_round_coverage(true, &mismatch).is_err(),
            "native ID 不一致必须拒绝"
        );

        // 承继格引用不可复核(缺来源根/摘要)→ 红。
        let mut incomplete = cells.clone();
        for cell in &mut incomplete {
            if cell.stage == "design" && cell.capability_state == "carried" {
                cell.carried_workspace_digest = String::new();
            }
        }
        assert!(
            verify_round_coverage(true, &incomplete).is_err(),
            "承继格缺摘要必须拒绝"
        );
    }

    #[test]
    fn t23_full_chain_requires_fresh_execution_per_stage() {
        let cells = full_chain_cells();
        assert!(
            verify_round_coverage(false, &cells).is_ok(),
            "合法 full_chain 轮必须通过"
        );

        let mut missing = cells.clone();
        missing.retain(|cell| {
            !(cell.stage == "plan"
                && cell.entrypoint == ENTRYPOINT_SPLIT_SYNC
                && cell.fresh_or_resume == FRESH)
        });
        assert!(
            verify_round_coverage(false, &missing).is_err(),
            "缺 plan/split_sync fresh 执行格必须拒绝"
        );

        // carried 冒充 fresh 执行格 → 红。
        let mut masquerade = cells.clone();
        for cell in &mut masquerade {
            if cell.stage == "coding" && cell.fresh_or_resume == FRESH {
                cell.capability_state = "carried".to_string();
                cell.execution_origin = "carried:snap-1".to_string();
            }
        }
        assert!(
            verify_round_coverage(false, &masquerade).is_err(),
            "carried 冒充 fresh 执行格必须拒绝"
        );

        // 只有 resume 格、无 fresh 执行格 → 红。
        let mut resume_only = cells.clone();
        for cell in &mut resume_only {
            if cell.fresh_or_resume == FRESH {
                cell.fresh_or_resume = RESUME.to_string();
            }
        }
        assert!(
            verify_round_coverage(false, &resume_only).is_err(),
            "无 fresh 执行格必须拒绝"
        );
    }

    fn zero_spawn_unknown_cell() -> EvidenceCell {
        let mut cell = base_cell("coding", ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT, FRESH);
        cell.capability_state = "unknown".to_string();
        cell.provider_pid = None;
        cell.pid_unavailable_reason = Some("零 spawn".to_string());
        cell.native_session_id = String::new();
        cell.argv = Vec::new();
        cell.argv_or_wire_capture_exists = false;
        cell.completed_product_artifact_exists = false;
        cell.provider_spawn_count = 0;
        cell
    }

    fn zero_spawn_event(before: u64, after: u64) -> Value {
        json!({
            "ts": "2026-10-19T00:00:00.000Z",
            "event": {
                "type": "zero_spawn_observation",
                "provider_start_audits_before": before,
                "provider_start_audits_after": after,
                "pid_observed": false,
                "argv_captured": false,
            }
        })
    }

    /// 失败格观测事件(t11 形态:显式 audit_window 口径标注)。
    fn failure_zero_spawn_event(before: u64, after: u64, audit_window: &str) -> Value {
        json!({
            "ts": "2026-10-19T00:00:00.000Z",
            "event": {
                "type": "zero_spawn_observation",
                "provider_start_audits_before": before,
                "provider_start_audits_after": after,
                "audit_window": audit_window,
                "pid_observed": false,
                "argv_captured": false,
            }
        })
    }

    /// 阶段格仪器缺口形态:阶段前审计基线未记录(before=null,无
    /// not_started 标记)——build_cell 不得以 after 冒充 before。
    fn unrecorded_zero_spawn_event(after: u64) -> Value {
        json!({
            "ts": "2026-10-19T00:00:00.000Z",
            "event": {
                "type": "zero_spawn_observation",
                "provider_start_audits_before": Value::Null,
                "provider_start_audits_after": after,
                "pid_observed": false,
                "argv_captured": false,
            }
        })
    }

    #[test]
    fn t23_spawn_instrumentation_requires_real_spawn_and_five_dim_accounting() {
        // fresh confirmed 有 spawn → 通过。
        let fresh = base_cell("coding", ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT, FRESH);
        assert!(verify_spawn_instrumentation(std::slice::from_ref(&fresh)).is_ok());

        // fresh 置 0 → 红(仪器缺失/真实未启动)。
        let mut zero_fresh = fresh.clone();
        zero_fresh.provider_spawn_count = 0;
        assert!(
            verify_spawn_instrumentation(&[zero_fresh]).is_err(),
            "fresh 置 0 必须拒绝"
        );

        // executed resume 置 0 → 红。
        let mut zero_resume = executed_resume_cell("coding");
        zero_resume.provider_spawn_count = 0;
        assert!(
            verify_spawn_instrumentation(&[zero_resume]).is_err(),
            "resume 置 0 必须拒绝"
        );

        // split_sync resume 非 0 → 红(契约零 spawn)。
        let mut split_resume = base_cell("plan", ENTRYPOINT_SPLIT_SYNC, RESUME);
        split_resume.capability_state = "unknown".to_string();
        split_resume.provider_spawn_count = 1;
        assert!(
            verify_spawn_instrumentation(&[split_resume]).is_err(),
            "split_sync resume 必须零 spawn"
        );

        // 单维 0、无五维/可证伪观测 → 红。
        let bare = zero_spawn_unknown_cell();
        assert!(
            verify_spawn_instrumentation(&[bare]).is_err(),
            "单维 0 无五维佐证必须拒绝"
        );

        // 前后审计相等 + 五维齐备 → 通过。
        let mut observed = zero_spawn_unknown_cell();
        observed.provider_events = vec![zero_spawn_event(2, 2)];
        assert!(
            verify_spawn_instrumentation(&[observed]).is_ok(),
            "五维 + zero_spawn_observation 必须通过"
        );

        // 前后审计变化 → 红。
        let mut drifted = zero_spawn_unknown_cell();
        drifted.provider_events = vec![zero_spawn_event(2, 3)];
        assert!(
            verify_spawn_instrumentation(&[drifted]).is_err(),
            "零 spawn 格审计前后变化必须拒绝"
        );
    }

    fn failure_cell_events(scenario: &str, audit_window: &str) -> Vec<Value> {
        vec![
            json!({
                "ts": "2026-10-19T00:00:00.000Z",
                "event": {
                    "type": "boundary_matrix_evidence",
                    "failure_scenario": scenario,
                }
            }),
            json!({
                "ts": "2026-10-19T00:00:00.000Z",
                "event": {
                    "type": "launch_rejected_probe",
                    "scenario": scenario,
                    "refusal_gate": "validate",
                    "refusal_text": "rejected",
                    "refused": true,
                }
            }),
            failure_zero_spawn_event(0, 0, audit_window),
        ]
    }

    fn failure_cell_scenario(state: &str, scenario: &str, audit_window: &str) -> EvidenceCell {
        let mut cell = zero_spawn_unknown_cell();
        cell.capability_state = state.to_string();
        cell.denied_reason = Some("真实拒绝/reason".to_string());
        cell.run_ref = format!("t11-failure-{scenario}");
        cell.provider_events = failure_cell_events(scenario, audit_window);
        cell
    }

    fn failure_cell(state: &str) -> EvidenceCell {
        // missing_readiness 把 launch 绑定到 sid(prepare 携带该会话身份)
        //——session 窗口缺省形态。
        failure_cell_scenario(state, "missing_readiness", "session")
    }

    #[test]
    fn t23_failure_cell_requires_refused_probe_unchanged_audits_and_unobservable_gate() {
        // 真实拒绝:refused=true + gate 非空 + audits 0→0 → 通过。
        assert!(
            verify_failure_cell(&failure_cell("denied")).is_ok(),
            "真实拒绝 + audits 不变必须通过"
        );

        // 拒绝但 audits 变化(session 绑定场景)→ 红。
        let mut changed = failure_cell("denied");
        changed.provider_events[2] = failure_zero_spawn_event(0, 1, "session");
        let error = verify_failure_cell(&changed).expect_err("audits 变化必须拒绝");
        assert!(error.contains("audits"), "{error}");

        // denied 但 refused=false → 红。
        let mut not_refused = failure_cell("denied");
        not_refused.provider_events[1]["event"]["refused"] = json!(false);
        assert!(
            verify_failure_cell(&not_refused).is_err(),
            "denied 且 refused=false 必须拒绝"
        );

        // unknown 无 unobservable gate → 红。
        let mut unknown_bad_gate = failure_cell("unknown");
        unknown_bad_gate.provider_events[1]["event"]["refused"] = json!(false);
        assert!(
            verify_failure_cell(&unknown_bad_gate).is_err(),
            "unknown 无 unobservable gate 必须拒绝"
        );

        // 合法 unknown:unobservable gate + 非空原因 → 通过。
        let mut allowed_unknown = failure_cell("unknown");
        allowed_unknown.provider_events[1]["event"]["refusal_gate"] = json!("unobservable");
        allowed_unknown.provider_events[1]["event"]["refusal_text"] = json!("OS 保护不可观察");
        allowed_unknown.provider_events[1]["event"]["refused"] = json!(false);
        assert!(
            verify_failure_cell(&allowed_unknown).is_ok(),
            "unobservable gate + 原因必须放行"
        );
    }

    /// F2(修复轮):阶段前审计基线未记录(build_cell 曾以 after 冒充
    /// before 构造恒等兜底)→ 必须红——仪器缺口可发现,不得伪造相等放行。
    #[test]
    fn t23_zero_spawn_requires_recorded_before_baseline() {
        // 断言侧:阶段前基线未记录(before=null,无 not_started 标记)
        // 不构成零 spawn 佐证 → 红——build_cell 不得以 after 冒充 before
        // 构造恒等兜底(F2 仪器缺口必须可发现)。
        let mut unrecorded = zero_spawn_unknown_cell();
        unrecorded.provider_events = vec![unrecorded_zero_spawn_event(2)];
        let error =
            verify_spawn_instrumentation(&[unrecorded]).expect_err("before 基线缺记(null)必须拒绝");
        assert!(error.contains("五维会计佐证"), "{error}");
    }

    /// M5/F4(修复轮):audits 相等仅对绑定 sid 的场景是零 spawn 证据;
    /// 未绑定场景(validate/能力分格面拒绝,launch 从不落在合成 sid 分区)
    /// 标注 not_applicable,判等不适用——审计漂移不得红、也不得计为证据,
    /// 零 spawn 由五维会计 + 拒绝探针承担;标注与冻结绑定清单交叉核对。
    #[test]
    fn t23_failure_cell_audit_window_scopes_audits_equality() {
        // 绑定 sid 场景(missing_readiness)声明 not_applicable → 红
        //(逃避审计对照)。
        let mut escape = failure_cell("denied");
        escape.provider_events[2] = failure_zero_spawn_event(2, 2, "not_applicable");
        let error = verify_failure_cell(&escape).expect_err("绑定场景不得标注 not_applicable");
        assert!(error.contains("not_applicable"), "{error}");

        // 未绑定场景(false_flag)声明 session → 红(伪造证据口径)。
        let fake_session = failure_cell_scenario("denied", "false_flag", "session");
        let error =
            verify_failure_cell(&fake_session).expect_err("未绑定场景不得声明 session 窗口");
        assert!(error.contains("不得声明 session"), "{error}");

        // 缺标注 → 红(必须如实声明窗口口径)。
        let mut unannotated = failure_cell("denied");
        unannotated.provider_events[2] = zero_spawn_event(0, 0);
        let error = verify_failure_cell(&unannotated).expect_err("缺 audit_window 标注必须拒绝");
        assert!(error.contains("audit_window"), "{error}");

        // 未绑定场景 + not_applicable + 审计漂移(0→3)→ 通过:
        // 0==0 无信息,漂移不作证据;零 spawn 证据改由五维+拒绝探针承担。
        let mut unbound = failure_cell_scenario("denied", "false_flag", "not_applicable");
        unbound.provider_events[2] = failure_zero_spawn_event(0, 3, "not_applicable");
        assert!(
            verify_failure_cell(&unbound).is_ok(),
            "未绑定场景审计漂移不是证据,不得红"
        );

        // 未绑定场景 + not_applicable + 五维破约(session child 观测在场)→ 红。
        let mut five_dim_broken = failure_cell_scenario("denied", "false_flag", "not_applicable");
        five_dim_broken.provider_events.push(json!({
            "ts": "2026-10-19T00:00:01.000Z",
            "event": {"type": "session_child_observed", "pid": 4242},
        }));
        let error = verify_failure_cell(&five_dim_broken)
            .expect_err("not_applicable 场景零 spawn 仍须五维会计自洽");
        assert!(error.contains("session_child_count"), "{error}");
    }

    #[test]
    fn t23_roles_matrix_counts_exactly_fixed_failure_scenarios() {
        let mut cells: Vec<EvidenceCell> = FIXED_FAILURE_SCENARIOS
            .iter()
            .map(|_| failure_cell("denied"))
            .collect();
        assert!(verify_failure_scenarios(&cells).is_ok());
        cells.pop();
        assert!(
            verify_failure_scenarios(&cells).is_err(),
            "删任一失败场景格必须拒绝"
        );
    }

    // ---- 类1′(证据完整性):per-provider 汇总 + run-id 证据根 ----

    #[test]
    fn t23_summary_paths_are_per_provider_and_run_id_scoped() {
        // 旧固定根(未设 run-id)逐字不变。
        assert_eq!(
            provider_evidence_root_with_run_id(&ProviderName::ClaudeCode, None)
                .expect("legacy root"),
            PathBuf::from("cadence/reports/lc-gateway-multi-provider/claude-code")
        );
        // run-id 作末段:同一 provider 不同 run-id 各自留证。
        let legacy = provider_evidence_root_with_run_id(&ProviderName::ClaudeCode, None)
            .expect("legacy root");
        let round_one =
            provider_evidence_root_with_run_id(&ProviderName::ClaudeCode, Some("round-1"))
                .expect("run root");
        assert_eq!(round_one, legacy.join("round-1"));
        // 汇总文件名按家分列,异家并行不互相覆盖。
        assert_eq!(
            b45_t23_summary_filename(&ProviderName::ClaudeCode),
            "b45-t23-roles-matrix-claude-code.json"
        );
        assert_ne!(
            b45_t23_summary_filename(&ProviderName::ClaudeCode),
            b45_t23_summary_filename(&ProviderName::Codex)
        );
        // 矩阵根布局逐字钉死(⚠5 补修:原断言两侧同一实现恒真):不带与带
        // run-id 的 b45-t23-roles/<facet> 都挂在 provider 证据根下,run-id 段
        // 插在 b45-t23-roles 之前(facet 结构不因 run-id 漂移)。
        assert_eq!(
            provider_evidence_root_with_run_id(&ProviderName::Pi, None)
                .expect("pi legacy root")
                .join("b45-t23-roles")
                .join("matrix"),
            PathBuf::from("cadence/reports/lc-gateway-multi-provider/pi/b45-t23-roles/matrix"),
            "T23 矩阵根必须钉死:facet 挂在 provider 证据根下"
        );
        assert_eq!(
            provider_evidence_root_with_run_id(&ProviderName::Pi, Some("r1"))
                .expect("pi run root")
                .join("b45-t23-roles")
                .join("matrix"),
            PathBuf::from("cadence/reports/lc-gateway-multi-provider/pi/r1/b45-t23-roles/matrix"),
            "run-id 段必须插在 b45-t23-roles 之前(不因 run-id 漂移 facet 结构)"
        );
        assert_ne!(
            b45_t23_evidence_root(&ProviderName::Codex, "matrix"),
            b45_t23_evidence_root(&ProviderName::Pi, "matrix"),
            "异家矩阵根必须互不相同"
        );
    }

    #[test]
    fn t23_summary_write_does_not_overwrite_other_provider_or_other_run() {
        let temp = tempfile::tempdir().expect("tempdir");
        let run_one = temp.path().join("reports/claude-code/round-1");
        let run_two = temp.path().join("reports/claude-code/round-2");
        let claude_path = b45_t23_write_provider_summary_at(
            &run_one,
            &ProviderName::ClaudeCode,
            Some("round-1"),
            &json!({"duties": {"m2_coder": ["coding"]}}),
        )
        .expect("write claude summary");
        let codex_path = b45_t23_write_provider_summary_at(
            &run_one,
            &ProviderName::Codex,
            Some("round-1"),
            &json!({"duties": {"m2_coder": ["coding"]}}),
        )
        .expect("write codex summary");
        assert_ne!(claude_path, codex_path, "异家汇总必须分文件");
        assert!(
            claude_path.ends_with("b45-t23-roles-matrix-claude-code.json"),
            "汇总文件名必须带 provider:{}",
            claude_path.display()
        );

        // 同家不同 run-id:各自留证,先落的不被覆盖。
        let claude_round_two = b45_t23_write_provider_summary_at(
            &run_two,
            &ProviderName::ClaudeCode,
            Some("round-2"),
            &json!({"duties": {"m2_coder": ["coding"]}}),
        )
        .expect("write claude round two");
        assert_ne!(claude_path, claude_round_two);
        assert!(claude_path.exists() && claude_round_two.exists());

        // 重写同家同 run-id:只覆盖本家文件,不删异家、不生成跨家合并文件。
        let rewritten = b45_t23_write_provider_summary_at(
            &run_one,
            &ProviderName::ClaudeCode,
            Some("round-1"),
            &json!({"duties": {"m2_coder": []}}),
        )
        .expect("rewrite claude summary");
        assert_eq!(rewritten, claude_path);
        assert!(codex_path.exists(), "重写单家不得动异家文件");
        assert!(
            !run_one.join("b45-t23-roles-matrix.json").exists(),
            "不得生成跨家共享汇总文件(会互相覆盖)"
        );

        // 文件内容只含本家(可逐格复算),run-id 如实记录。
        let written: Value =
            serde_json::from_slice(&std::fs::read(&claude_path).expect("read claude summary"))
                .expect("parse claude summary");
        assert_eq!(written["provider"], "claude-code");
        assert_eq!(written["run_id"], "round-1");
        assert!(written["providers"].get("ClaudeCode").is_some());
        assert!(
            written["providers"].get("Codex").is_none(),
            "单家汇总不得混入异家条目"
        );
    }

    #[test]
    fn t23_carried_source_chain_resolves_under_run_id_root() {
        let temp = tempfile::tempdir().expect("tempdir");
        // 带 run-id 的真实布局:<族>/<run-id>/b45-t23-roles/{matrix,snapshots}。
        let run_root = temp
            .path()
            .join("reports")
            .join("claude-code")
            .join("20261020T010203Z-capture");
        let matrix_root = run_root.join("b45-t23-roles").join("matrix");
        std::fs::create_dir_all(&matrix_root).expect("matrix root");
        let snapshot_directory = snapshots_root(&matrix_root).join("snap-1");
        assert_eq!(
            snapshots_root(&matrix_root),
            run_root.join("b45-t23-roles").join("snapshots"),
            "快照根必须是同 run-id 内的矩阵兄弟目录"
        );
        std::fs::create_dir_all(&snapshot_directory).expect("snapshot dir");
        let manifest = test_manifest(&matrix_root.to_string_lossy());
        std::fs::write(
            snapshot_directory.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).expect("manifest json"),
        )
        .expect("write manifest");

        // 承继格按来源证据根(带 run-id)复核仍通过。
        let cell = carried_test_cell(
            "story",
            ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT,
            &manifest,
        );
        let canonical = PathBuf::from("/tmp/t23-root");
        let member = canonical.join("alpha");
        assert!(
            cell.validate_against(&ProviderName::ClaudeCode, &canonical, &member)
                .is_ok(),
            "带 run-id 的来源链必须可复核: {:?}",
            cell.validate_against(&ProviderName::ClaudeCode, &canonical, &member)
        );

        // resume 轮消费者必须用同一 run-id 计算根:facet 结构一致时落在
        // 同一相对布局(claude-code/<run-id>/b45-t23-roles/matrix)。
        let resumed_root = provider_evidence_root_with_run_id(
            &ProviderName::ClaudeCode,
            Some("20261020T010203Z-capture"),
        )
        .expect("resume root")
        .join("b45-t23-roles")
        .join("matrix");
        assert!(
            resumed_root.ends_with(Path::new(
                "claude-code/20261020T010203Z-capture/b45-t23-roles/matrix"
            )),
            "resume 轮证据根布局必须与 capture 轮同源:{}",
            resumed_root.display()
        );
        // 不接 run-id 的旧根不会命中该快照(缺同一 run-id = 证据链断裂,
        // 因此 capture→resume 必须复用同一 run-id)。
        let legacy_matrix = provider_evidence_root_with_run_id(&ProviderName::ClaudeCode, None)
            .expect("legacy root")
            .join("b45-t23-roles")
            .join("matrix");
        assert_ne!(
            snapshots_root(&legacy_matrix),
            snapshots_root(&matrix_root),
            "旧固定根不得装作同轮快照根"
        );
    }
}
