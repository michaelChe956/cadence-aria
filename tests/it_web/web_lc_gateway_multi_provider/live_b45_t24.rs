//! 批2 Task 24(b45):Pi 事故副本重放与真崩溃恢复的现场测试与 validator。
//!
//! 四场景各自独立(各自证据根 + 各自 run.json + 各自 validator;一题红不
//! 阻断其它题):
//! - `b45_live_pi_replay_normal`(Step 3 正常轮):合法验证→Accepted+
//!   worktree 恢复+Coder dirty 保留;证据根 `pi/b45-t24-replay-normal`。
//! - `b45_live_pi_replay_violation`(Step 3 违规轮):主动 commit/写 `.git`/
//!   `.aria`→Invalidated+人工门+平台零推进(HEAD/index/裸远端),窗口外
//!   Coder 正常提交/推送对照;证据根 `pi/b45-t24-replay-violation`。
//! - `b45_live_review_crash`(Step 4):真实 `aria web` 服务进程 kill -9/
//!   重启的完整演练(停旧 writer/清登记容器/原 manifest 续恢复/三域核对
//!   →再放行新材料;停等保持锁不误杀;静默 CHECK 跨 300s);证据根
//!   `pi/b45-t24-crash`。**不含取消格**(cancel cell 的合并与统一 run.json
//!   已删除;crash validator 本身不引用 cancel 面)。
//! - `b45_live_review_cancel`(Step 4 取消格):按 `B45_T24_CANCEL_PROVIDERS`
//!   指定子集(缺省 claude,codex,kimi)各一格真实取消与资源收口;证据根
//!   `pi/b45-t24-cancel`。
//!
//! validator(红测合同,Step 1;缺项即 FAIL):replay 缺恢复前 journal、任一
//! 三域结果、dirty 摘要、精确容器归属、人工门裁决即 FAIL;crash 缺 kill
//! 事实、启动前零材料/零 provider 事实、停旧 writer、续恢复、静默
//! CHECK≥310s、停等臂即 FAIL;cancel 的 provider 集合不精确(空集合/缺家/
//! 多余)、未 aborted、进程/容器未终止、attempt 非终态、资源未收口、取消后
//! 新 spawn 即 FAIL——`validator_replay_*`/`validator_crash_*`/
//! `validator_cancel_*`/`cancel_providers_*` 单测钉死(合成夹具,红先)。
//!
//! 现场命令(每题独占;replay 两轮共用 `/tmp/aria-bug45/t24/common`,必须
//! 互斥串行;flock 仅包编译段,见计划 §2):
//! ```text
//! LC_GATEWAY_E2E=1 cargo test --locked --test it_web b45_live_pi_replay_normal    -- --ignored --nocapture --test-threads=1
//! LC_GATEWAY_E2E=1 cargo test --locked --test it_web b45_live_pi_replay_violation -- --ignored --nocapture --test-threads=1
//! LC_GATEWAY_E2E=1 cargo test --locked --test it_web b45_live_review_crash        -- --ignored --nocapture --test-threads=1
//! LC_GATEWAY_E2E=1 B45_T24_CANCEL_PROVIDERS=claude,codex,kimi \
//!   cargo test --locked --test it_web b45_live_review_cancel -- --ignored --nocapture --test-threads=1
//! ```

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{Value, json};

use super::harness::{
    B45_T24_SILENT_CHECK_SECS, B45T24Scenario, LiveLcGatewayHarness,
    b45_t24_cancel_provider_label,
};
use super::live_matrix::require_lc_gateway_e2e_switch;

/// T24 证据根(计划 Files:cadence/reports/lc-gateway-multi-provider/pi/)。
fn b45_t24_evidence_root(facet: &str) -> PathBuf {
    PathBuf::from("cadence/reports/lc-gateway-multi-provider/pi").join(facet)
}

/// 取消格默认三家(env 未设即此集合)。
fn b45_t24_cancel_default_providers() -> Vec<cadence_aria::product::models::ProviderName> {
    use cadence_aria::product::models::ProviderName;
    vec![
        ProviderName::ClaudeCode,
        ProviderName::Codex,
        ProviderName::KimiCode,
    ]
}

/// `B45_T24_CANCEL_PROVIDERS` 解析(逗号分隔 claude/codex/kimi;大小写与
/// 重复记号按集合收敛)。
///
/// 语义(集合精确口径的另一半):未设→默认三家;设了但**空集合**或未知
/// 记号→Err(必须红,不静默回退三家)。
fn b45_t24_parse_cancel_providers(
    raw: Option<&str>,
) -> Result<Vec<cadence_aria::product::models::ProviderName>, String> {
    use cadence_aria::product::models::ProviderName;
    let Some(raw) = raw else {
        return Ok(b45_t24_cancel_default_providers());
    };
    let mut providers = Vec::new();
    for token in raw.split(',') {
        let token = token.trim().to_lowercase();
        if token.is_empty() {
            continue;
        }
        let provider = match token.as_str() {
            "claude" => ProviderName::ClaudeCode,
            "codex" => ProviderName::Codex,
            "kimi" => ProviderName::KimiCode,
            other => {
                return Err(format!(
                    "未知 provider 记号 {other:?}(只认 claude/codex/kimi)"
                ));
            }
        };
        if !providers.contains(&provider) {
            providers.push(provider);
        }
    }
    if providers.is_empty() {
        return Err(format!(
            "空集合(B45_T24_CANCEL_PROVIDERS={raw:?});空集合必须红"
        ));
    }
    Ok(providers)
}

/// env 解析入口(非法 env 直接 panic=红,不降级)。
fn b45_t24_cancel_providers_from_env() -> Vec<cadence_aria::product::models::ProviderName> {
    b45_t24_parse_cancel_providers(std::env::var("B45_T24_CANCEL_PROVIDERS").ok().as_deref())
        .unwrap_or_else(|error| panic!("B45_T24_CANCEL_PROVIDERS 非法:{error}"))
}

/// env 集合 → 证据标签集合(validator 的期望集合口径)。
fn b45_t24_cancel_labels(
    providers: &[cadence_aria::product::models::ProviderName],
) -> Vec<&'static str> {
    providers.iter().map(b45_t24_cancel_provider_label).collect()
}

// ---------------------------------------------------------------------------
// validator(replay)——缺项即 FAIL 的可证伪面。
// ---------------------------------------------------------------------------

/// 窗口终态摘要(单窗;缺失字段→None 供可证伪)。
fn t24_window_of<'a>(windows: &'a [Value], attempt_id: &str) -> Vec<&'a Value> {
    windows
        .iter()
        .filter(|window| window["attempt_id"].as_str() == Some(attempt_id))
        .collect()
}

/// 违规 journal 条目集合(全部窗口)。
fn t24_violation_entries(evidence: &Value) -> Vec<&Value> {
    let mut entries = Vec::new();
    for journal in evidence
        .pointer("/windows_final/violation_journals")
        .and_then(Value::as_array)
        .into_iter().flatten()
    {
        if let Some(items) = journal["entries"].as_array() {
            entries.extend(items.iter());
        }
    }
    entries
}

/// replay validator:断言组(计划 Step 1 红测面的机器形态)。
///
/// 必备(缺任一=FAIL):
/// - 恢复事实:窗口 `restore`(正常轮 outcome=restored;违规轮 journal 在
///   恢复前落盘——`pre_restore_git_facts`/journal 条目在场);
/// - 三域结果:git 域(restore 绑定的 git_restored_mutation_paths/extra_
///   refs)、平台域(fixtures/aria 探针事实)、记录一致域(report_acceptance
///   投影与窗口一致);
/// - dirty 摘要:窗口开点与终局的 worktree 状态对照(coder dirty/staged/
///   ignored/build-out 存活);
/// - 精确容器归属:role run 事件含容器 exact_id+标签(正常轮);
/// - 人工门裁决(违规轮):ManualGate→resolve(invalidate)→ClosedInvalidated;
/// - 平台零推进(违规轮):worktree HEAD 终局=开点 HEAD(自提交被恢复)
///   + 窗口外对照 commit/push exit=0。
pub(crate) fn validate_b45_t24_replay_evidence(evidence: &Value) -> Result<(), Vec<String>> {
    let mut failures = Vec::new();
    let scenario = evidence["scenario"].as_str().unwrap_or_default().to_string();
    let attempt_id = evidence
        .pointer("/coding_main/attempt_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if attempt_id.is_empty() {
        failures.push("缺 attempt_id(coding 未启动=场景未执行)".to_string());
        return Err(failures);
    }
    let windows: Vec<Value> = evidence
        .pointer("/windows_final/windows")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if t24_window_of(&windows, &attempt_id).is_empty() {
        failures.push(format!("缺 attempt {attempt_id} 的 reviewer 窗口记录(开窗未发生=未达评审)"));
    }
    // 恢复事实(任一窗口必须携带 restore 绑定)。
    let restored_any = t24_window_of(&windows, &attempt_id)
        .iter()
        .any(|window| window.get("restore").is_some_and(|restore| {
            restore["outcome"].as_str().is_some_and(|outcome| outcome == "restored")
        }));
    if !restored_any {
        failures.push("缺恢复前 journal/恢复绑定:窗口无 restore{outcome=restored}(恢复未完成=不可采信)".to_string());
    }
    // 三域结果:git 域字段在场(restore 绑定含 git 域结构化事实)。
    let git_domain_any = t24_window_of(&windows, &attempt_id).iter().any(|window| {
        window["restore"].get("git_restored_mutation_paths").is_some()
            || window["restore"].get("git_extra_refs_observed").is_some()
    });
    if !git_domain_any {
        failures.push("缺三域结果之 git 域(restore 绑定无 git_restored_mutation_paths/git_extra_refs_observed)".to_string());
    }
    // 三域结果:平台域(fixtures 面在场)。
    if evidence.get("fixtures").map(Value::is_null).unwrap_or(true) {
        failures.push("缺三域结果之平台域(fixtures 摘要缺失)".to_string());
    }
    // 三域结果:记录一致域(报告采信投影在场且非空)。
    let reports = evidence
        .get("code_review_reports")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if reports.is_empty() {
        failures.push("缺三域结果之记录一致域(code review 报告/采信投影缺失)".to_string());
    }
    // dirty 摘要:窗口开点 worktree 状态 + 终局状态。
    if evidence
        .pointer("/window_open_facts/worktree_state")
        .map(Value::is_null)
        .unwrap_or(true)
    {
        failures.push("缺 dirty 摘要(窗口开点 worktree 状态缺失)".to_string());
    }
    if evidence.get("worktree_final").map(Value::is_null).unwrap_or(true) {
        failures.push("缺 dirty 摘要(终局 worktree 状态缺失)".to_string());
    }
    // 精确容器归属(正常轮):role run 事件含 exact_id+标签。
    let role_rows = evidence
        .pointer("/coding_main/role_run_rows")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let container_evidence = role_rows.iter().any(|row| {
        let payload = row["payload"].to_string();
        payload.contains("container exact_id=") && payload.contains("io.aria.b45.scene=")
    });
    match scenario.as_str() {
        "replay-normal" => {
            if !container_evidence {
                failures.push("缺精确容器归属(role run 事件无容器 exact_id+标签)".to_string());
            }
            // 正常轮:报告采信 Accepted + 窗口 ClosedAccepted。
            let accepted = reports.iter().any(|report| {
                report["acceptance"].as_str().is_some_and(|a| a.eq_ignore_ascii_case("accepted"))
            });
            if !accepted {
                failures.push("正常轮报告未 Accepted(合法验证必须可采信)".to_string());
            }
            let closed_accepted = t24_window_of(&windows, &attempt_id)
                .iter()
                .any(|window| window["state"].as_str() == Some("closed_accepted"));
            if !closed_accepted {
                failures.push("正常轮窗口未 ClosedAccepted".to_string());
            }
        }
        "replay-violation" => {
            // 违规 journal(恢复前 durable 违规)。
            let violations = t24_violation_entries(evidence);
            if violations.is_empty() {
                failures.push("缺恢复前 durable 违规 journal(违规轮必须落 journal)".to_string());
            }
            // 报告 Invalidated。
            let invalidated = reports.iter().any(|report| {
                report["acceptance"]
                    .as_str()
                    .is_some_and(|a| a.eq_ignore_ascii_case("invalidated"))
            });
            if !invalidated {
                failures.push("违规轮报告未 Invalidated(整轮作废)".to_string());
            }
            // 人工门裁决:resolve 前后状态。
            let resolve_status = evidence
                .pointer("/manual_gate/resolve_status")
                .and_then(Value::as_u64);
            if resolve_status != Some(200) {
                failures.push(format!("缺人工门裁决(resolve 状态非 200:{resolve_status:?})"));
            }
            let after_has_closed = evidence
                .pointer("/manual_gate/after/windows")
                .and_then(Value::as_array)
                .map(|windows| {
                    windows.iter().any(|window| {
                        window["attempt_id"].as_str() == Some(attempt_id.as_str())
                            && matches!(
                                window["state"].as_str(),
                                Some("closed_invalidated") | Some("new_review_required")
                            )
                    })
                })
                .unwrap_or(false);
            if !after_has_closed {
                failures.push("人工门裁决后窗口未释放(ClosedInvalidated/NewReviewRequired)".to_string());
            }
            // 平台零推进三面(F9):终局 HEAD=开点 HEAD(自提交被恢复)、
            // 裸远端 HEAD 前后一致(push 未推进)、index 摘要一致(stage 未推进)。
            let head_open = evidence
                .pointer("/window_open_facts/worktree_state/head")
                .and_then(Value::as_str)
                .map(str::to_string);
            let head_final = evidence
                .pointer("/worktree_final/head")
                .and_then(Value::as_str)
                .map(str::to_string);
            match (head_open, head_final) {
                (Some(open), Some(final_head)) if !open.is_empty() => {
                    if open != final_head {
                        failures.push(format!(
                            "平台零推进被违反:HEAD {open} → {final_head}(平台/评审推进了分支)"
                        ));
                    }
                }
                _ => failures.push("平台零推进对照缺 HEAD(开点/终局)".to_string()),
            }
            let bare_open = evidence.pointer("/window_open_facts/bare_head_open");
            let bare_final = evidence.pointer("/bare_head_final");
            match (bare_open, bare_final) {
                (Some(open), Some(final_head)) => match (open.as_str(), final_head.as_str()) {
                    (Some(open), Some(final_head)) if !open.is_empty() => {
                        if open != final_head {
                            failures.push(format!(
                                "平台零推进被违反:裸远端 HEAD {open} → {final_head}(平台推进了 push)"
                            ));
                        }
                    }
                    // 两键存在且为 null(远端分支从未创建;窗口外对照 push
                    // 在采样后)=零推进合法形态。缺键≠null:字段改名/漏发射
                    // 必须仍 FAIL(合成夹具缺项必须 FAIL 契约)。
                    (None, None) => {}
                    _ => failures.push("平台零推进对照裸远端 HEAD 值形态非法(非串/空串)".to_string()),
                },
                _ => failures.push("平台零推进对照缺裸远端 HEAD(开点/终局键缺失)".to_string()),
            }
            let index_open = evidence
                .pointer("/window_open_facts/index_digest_open")
                .and_then(Value::as_str)
                .map(str::to_string);
            let index_final = evidence
                .pointer("/index_digest_final")
                .and_then(Value::as_str)
                .map(str::to_string);
            match (index_open, index_final) {
                (Some(open), Some(final_digest)) if !open.is_empty() => {
                    if open != final_digest {
                        failures.push(format!(
                            "平台零推进被违反:index 摘要 {open} → {final_digest}(平台推进了 stage)"
                        ));
                    }
                }
                _ => failures.push("平台零推进对照缺 index 摘要(开点/终局)".to_string()),
            }
            // 窗口外对照:commit+push exit=0。
            let control_ok = ["add", "commit", "push"].into_iter().all(|step| {
                evidence
                    .pointer(&format!("/out_of_window_control/{step}/exit"))
                    .and_then(Value::as_i64)
                    == Some(0)
            });
            if !control_ok {
                failures.push("窗口外 Coder 正常提交/推送对照失败(对照必须成功)".to_string());
            }
        }
        other => failures.push(format!("未知场景标签:{other:?}")),
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures)
    }
}

// ---------------------------------------------------------------------------
// validator(crash)。
// ---------------------------------------------------------------------------

/// crash validator:断言组(计划 Step 4 红测面的机器形态)。
///
/// 必备(缺任一=FAIL):
/// - kill 事实:server_dead=true(kill -9 真实发生);
/// - 启动前零材料/零 provider 事实:kill 时窗口/资源/role run 快照在场
///   (与重启后事实可比);
/// - 停旧 writer:登记的旧 writer(确认身份 daemon)重启后不再存活;
/// - 清登记容器/资源:重启后登记面不再含 kill 时在场的旧 writer 记录;
/// - 原 manifest 续恢复:窗口 #1 达终态且 restore 绑定在场
///   (outcome=restored),且窗口释放时刻早于重启观测时刻(补偿先行);
/// - 三域核对:窗口 restore 的 git 域结构化事实在场;
/// - 再放行新材料:attempt #1 终态 completed(重启后续驱完成);
/// - 静默 CHECK:reviewer 静默跨度 ≥300s 且无 stalled(误杀)证据;
/// - 停等不误杀臂(drill B):park 态 recovery_required + kept locked 日志
///   + daemon/容器重启后存活 + 人工处置后释放;
/// - 取消格:三家格在场且 aborted。
pub(crate) fn validate_b45_t24_crash_evidence(evidence: &Value) -> Result<(), Vec<String>> {
    let mut failures = Vec::new();
    // kill 事实。
    if evidence.pointer("/kill/server_dead").and_then(Value::as_bool) != Some(true) {
        failures.push("缺 kill 事实(server 未死=真进程 kill 未发生)".to_string());
    }
    // 启动前零材料/零 provider 事实(kill 时快照三面在场)。
    for facet in ["/kill/windows_at_kill", "/kill/scopes_at_kill", "/kill/role_runs_at_kill"] {
        if evidence.pointer(facet).and_then(Value::as_array).is_none() {
            failures.push(format!("缺启动前零材料/零 provider 事实({facet} 快照缺失)"));
        }
    }
    // 旧 writer 停止 + 登记清理。
    let daemon_stopped =
        evidence.pointer("/restart/daemon_alive_after_restart").and_then(Value::as_bool) == Some(false);
    if !daemon_stopped {
        failures.push("旧 writer 未停止(登记 daemon 重启后仍存活)".to_string());
    }
    let scopes_at_kill = evidence
        .pointer("/kill/scopes_at_kill")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let old_writer_record = scopes_at_kill
        .iter()
        .find(|scope| {
            scope["record"]["resource_id"].as_str() == Some("old-writer-daemon")
        });
    if old_writer_record.is_none() {
        failures.push("缺旧 writer 登记记录(kill 时无 old-writer-daemon)".to_string());
    }
    let scopes_after = evidence
        .pointer("/restart/scopes_after_restart")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if old_writer_record.is_some()
        && scopes_after
            .iter()
            .any(|scope| scope["record"]["resource_id"].as_str() == Some("old-writer-daemon"))
    {
        failures.push("登记容器/资源未清(旧 writer 记录重启后仍在)".to_string());
    }
    // 原 manifest 续恢复 + 三域核对(窗口 #1)。
    let windows_one = evidence
        .pointer("/restart/window_one_terminal")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let restored_window = windows_one.iter().find(|window| {
        window["restore"]["outcome"].as_str() == Some("restored")
            && window["restore"]
                .get("git_restored_mutation_paths")
                .is_some()
    });
    if restored_window.is_none() {
        failures.push("缺原 manifest 续恢复/三域核对(窗口 #1 无 restore{restored}+git 域事实)".to_string());
    }
    // 补偿先行:窗口终态时刻早于重启观测时刻。
    let closed_at = restored_window
        .and_then(|window| window["updated_at"].as_str())
        .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok());
    let observed_at = evidence
        .pointer("/restart/restarted_observed_at")
        .and_then(Value::as_str)
        .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok());
    match (closed_at, observed_at) {
        (Some(closed), Some(observed)) => {
            if closed > observed {
                failures.push("补偿未先行:窗口收口晚于重启观测(材料先于补偿放行)".to_string());
            }
        }
        _ => failures.push("补偿先行对照缺时刻(窗口 updated_at/重启观测时刻)".to_string()),
    }
    // 再放行新材料:attempt #1 终态 completed。
    let attempt_status = evidence
        .pointer("/new_material/attempt_state/status")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if attempt_status != "completed" {
        failures.push(format!(
            "新材料未放行(attempt #1 终态={attempt_status:?},须 completed)"
        ));
    }
    // 静默 CHECK ≥300s 且不误杀。
    let silent_gap = evidence
        .pointer("/new_material/reviewer_silent_gap_secs")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if silent_gap < B45_T24_SILENT_CHECK_SECS {
        failures.push(format!(
            "静默 CHECK 未跨 {B45_T24_SILENT_CHECK_SECS}s(实测 {silent_gap}s)"
        ));
    }
    let stalled = evidence
        .pointer("/new_material/pump_resume/events")
        .and_then(Value::as_array)
        .map(|events| {
            events
                .iter()
                .any(|event| event.to_string().contains("stalled"))
        })
        .unwrap_or(false);
    if stalled {
        failures.push("静默 CHECK 被误杀(stalled 证据在场)".to_string());
    }
    // 停等不误杀臂。
    let drill_b = &evidence["drill_b"];
    if drill_b.get("skipped").is_some() {
        failures.push(format!(
            "停等不误杀臂未演练:{}",
            drill_b["skipped"].as_str().unwrap_or("未知")
        ));
    } else {
        let park_state = drill_b
            .pointer("/park_state/matched")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !park_state {
            failures.push("停等臂未 park(窗口未达 recovery_required)".to_string());
        }
        if drill_b["park_log_has_kept_locked"].as_bool() != Some(true) {
            failures.push("停等臂无 kept locked 日志(补偿未停等)".to_string());
        }
        if drill_b["daemon_alive_after_restart"].as_bool() != Some(true) {
            failures.push("未知 daemon 被误杀(重启后必须存活)".to_string());
        }
        if drill_b["container_alive_after_restart"].as_bool() != Some(true) {
            failures.push("共享/登记容器被误杀(重启后必须存活)".to_string());
        }
        let resolved = drill_b
            .pointer("/resolved_state/matched")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !resolved {
            failures.push("人工处置后未续接释放(resolved_state 未达终态)".to_string());
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures)
    }
}

// ---------------------------------------------------------------------------
// validator(cancel)。
// ---------------------------------------------------------------------------

/// cancel validator:取消格断言组(集合精确 + 真实终止 + 资源收口;缺项即
/// FAIL)。
///
/// `expected` = env 解析出的标签集合(`b45_t24_cancel_labels`)。
///
/// 必备(缺任一=FAIL):
/// - 集合精确:`providers` 声明集合与 `cells` 键集合都必须**恰好等于**期望
///   集合——空集合、缺家、多余任一即 FAIL;
/// - per-provider aborted:`cancel_cell.abort_sent=true` 且
///   `terminal_status="aborted"`(取消真发且真达取消终态);
/// - 进程/容器收口:`provider_pid_still_alive=false` 且
///   `lingering_fixture_containers` 为空;
/// - attempt 终态:HTTP 200 + `attempt.status="aborted"` +
///   `attempt.completed_at` 非空(终态回填时刻在场);
/// - `resource_scopes_after` 收口:取消后该 env 的 provider 资源登记面为空
///   (record 文件即登记事实,收口即无残留);
/// - 取消后无新 spawn:`role_run_rows_after_cancel` 非空(取消前已有 role
///   run 基线),且无任何 role run 的**首事件时刻**晚于 attempt 终态时刻
///   (role run = 一次 provider 启动;取消后不得再起)。
pub(crate) fn validate_b45_t24_cancel_evidence(
    evidence: &Value,
    expected: &[&str],
) -> Result<(), Vec<String>> {
    let mut failures = Vec::new();
    let mut expected_labels: Vec<&str> = expected.to_vec();
    expected_labels.sort_unstable();
    expected_labels.dedup();
    if expected_labels.is_empty() {
        failures.push("取消格期望集合为空(空集合必须红)".to_string());
        return Err(failures);
    }
    // 集合精确:声明集合与 cell 键集合都必须恰好等于期望集合。
    let declared: Vec<&str> = evidence
        .get("providers")
        .and_then(Value::as_array)
        .map(|values| values.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if declared.is_empty() {
        failures.push("缺取消格集合声明(providers 缺失/非数组/空)".to_string());
    }
    let cells = evidence
        .get("cells")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if cells.is_empty() {
        failures.push("缺取消格 cells(空集合=红)".to_string());
    }
    for (facet, actual) in [
        ("声明", declared),
        (
            "cells",
            cells.keys().map(String::as_str).collect::<Vec<_>>(),
        ),
    ] {
        let mut actual: Vec<&str> = actual;
        actual.sort_unstable();
        actual.dedup();
        if !actual.is_empty() && actual != expected_labels {
            failures.push(format!(
                "取消格集合不精确({facet}){actual:?} ≠ 期望 {expected_labels:?}(缺家/多余)"
            ));
        }
    }
    for label in &expected_labels {
        if let Some(cell) = cells.get(*label) {
            t24_cancel_cell_failures(label, cell, &mut failures);
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures)
    }
}

/// 单 provider 取消格的断言(未取消/未收口/新 spawn 即 FAIL)。
fn t24_cancel_cell_failures(label: &str, cell: &Value, failures: &mut Vec<String>) {
    if let Some(error) = cell.get("error") {
        failures.push(format!("取消格 {label} 未执行:{error}"));
        return;
    }
    let cancel = &cell["cancel_cell"];
    if cancel["abort_sent"].as_bool() != Some(true)
        || cancel["terminal_status"].as_str() != Some("aborted")
    {
        failures.push(format!(
            "取消格 {label} 未 aborted(abort_sent={:?},terminal_status={:?})",
            cancel["abort_sent"], cancel["terminal_status"]
        ));
    }
    if cancel["provider_pid_still_alive"].as_bool() != Some(false) {
        failures.push(format!(
            "取消格 {label} provider 进程未终止(provider_pid_still_alive={:?}/缺项)",
            cancel["provider_pid_still_alive"]
        ));
    }
    match cancel["lingering_fixture_containers"].as_array() {
        Some(containers) if containers.is_empty() => {}
        Some(containers) => {
            failures.push(format!("取消格 {label} 取消后遗留 fixture 容器:{containers:?}"));
        }
        None => failures.push(format!(
            "取消格 {label} 缺遗留容器对账(lingering_fixture_containers 缺失)"
        )),
    }
    // attempt 终态(status + 终态回填时刻)。
    let attempt = &cell["attempt_state"]["body"]["attempt"];
    let completed_at = attempt["completed_at"]
        .as_str()
        .filter(|text| !text.is_empty())
        .map(str::to_string);
    if cell["attempt_state"]["status"].as_u64() != Some(200)
        || attempt["status"].as_str() != Some("aborted")
        || completed_at.is_none()
    {
        failures.push(format!(
            "取消格 {label} attempt 未达终态(HTTP={:?},status={:?},completed_at={:?})",
            cell["attempt_state"]["status"], attempt["status"], attempt["completed_at"]
        ));
    }
    // 资源收口:取消后该 env 的 provider 资源登记面必须为空。
    match cell["resource_scopes_after"].as_array() {
        Some(scopes) if scopes.is_empty() => {}
        Some(scopes) => {
            let residue: Vec<&str> = scopes
                .iter()
                .map(|scope| {
                    scope["record"]["resource_id"]
                        .as_str()
                        .unwrap_or("<缺 resource_id>")
                })
                .collect();
            failures.push(format!("取消格 {label} 资源未收口:{residue:?}"));
        }
        None => failures.push(format!(
            "取消格 {label} 缺 resource_scopes_after 对账(数组缺失)"
        )),
    }
    // 取消后无新 spawn:role run 首事件时刻晚于 attempt 终态时刻即新 spawn。
    let rows = match cell["role_run_rows_after_cancel"].as_array() {
        Some(rows) if !rows.is_empty() => rows,
        _ => {
            failures.push(format!(
                "取消格 {label} 缺取消前 role run 基线(无法证明取消后无新 spawn)"
            ));
            return;
        }
    };
    let Some(completed_at) = completed_at else {
        return; // attempt 终态已红,不再二次报。
    };
    let Some(terminal_at) = chrono::DateTime::parse_from_rfc3339(&completed_at).ok() else {
        failures.push(format!(
            "取消格 {label} attempt 终态时刻不可解析:{completed_at:?}"
        ));
        return;
    };
    let mut run_started_at: BTreeMap<&str, chrono::DateTime<chrono::FixedOffset>> = BTreeMap::new();
    for row in rows {
        let (Some(run_id), Some(created_at)) =
            (row["role_run_id"].as_str(), row["created_at"].as_str())
        else {
            continue;
        };
        let Ok(created_at) = chrono::DateTime::parse_from_rfc3339(created_at) else {
            continue;
        };
        run_started_at
            .entry(run_id)
            .and_modify(|earliest| {
                if created_at < *earliest {
                    *earliest = created_at;
                }
            })
            .or_insert(created_at);
    }
    if run_started_at.is_empty() {
        failures.push(format!(
            "取消格 {label} 缺取消前 role run 基线(role run 行缺 role_run_id/created_at)"
        ));
        return;
    }
    let spawned_after: Vec<&str> = run_started_at
        .iter()
        .filter(|(_, started_at)| **started_at > terminal_at)
        .map(|(run_id, _)| *run_id)
        .collect();
    if !spawned_after.is_empty() {
        failures.push(format!("取消格 {label} 取消后出现新 spawn:{spawned_after:?}"));
    }
}

// ---------------------------------------------------------------------------
// 现场测试(#[ignore];LC_GATEWAY_E2E=1 + --ignored 命中)。
// ---------------------------------------------------------------------------

/// replay 两轮共同驱动(各自证据根/run.json/validator;两题共用
/// `/tmp/aria-bug45/t24/common`,必须互斥串行)。
async fn b45_t24_run_replay_scenario(scenario: B45T24Scenario) {
    require_lc_gateway_e2e_switch();
    let harness = LiveLcGatewayHarness::new();
    let evidence_root = b45_t24_evidence_root(&format!("b45-t24-{}", scenario.label()));
    let run = harness
        .run_b45_t24_replay(&evidence_root, scenario)
        .await
        .unwrap_or_else(|failure| {
            panic!(
                "b45 t24 replay({})现场执行失败:{failure:?}",
                scenario.label()
            )
        });
    // 证据落盘(脱敏:不含认证材料)。
    std::fs::create_dir_all(&evidence_root).expect("证据目录");
    std::fs::write(
        evidence_root.join("run.json"),
        serde_json::to_vec_pretty(&run).expect("run json"),
    )
    .expect("run.json 落盘");
    // validator 判绿(缺项=真实失败,不删格)。
    if let Err(failures) = validate_b45_t24_replay_evidence(&run) {
        panic!(
            "b45 t24 replay({})证据 validator 拒绝:{failures:?}",
            scenario.label()
        );
    }
}

/// Step 3 真实执行:重放·正常轮(合法验证→Accepted+dirty 保留)。
#[tokio::test]
#[ignore = "b45 真实现场:LC_GATEWAY_E2E=1 且 Step 3 正常轮执行(需要 pi 登录态/podman;一轮全链)→ 证据根 pi/b45-t24-replay-normal"]
async fn b45_live_pi_replay_normal() {
    b45_t24_run_replay_scenario(B45T24Scenario::ReplayNormal).await;
}

/// Step 3 真实执行:重放·违规轮(自提交→Invalidated+人工门+平台零推进)。
#[tokio::test]
#[ignore = "b45 真实现场:LC_GATEWAY_E2E=1 且 Step 3 违规轮执行(需要 pi 登录态/podman;一轮全链+窗口外对照)→ 证据根 pi/b45-t24-replay-violation"]
async fn b45_live_pi_replay_violation() {
    b45_t24_run_replay_scenario(B45T24Scenario::ReplayViolation).await;
}

/// Step 4 真实执行:真崩溃恢复(Pi 完整演练;不含取消格)。
#[tokio::test]
#[ignore = "b45 真实现场:LC_GATEWAY_E2E=1 且 Step 4 执行(需要 pi 登录态/podman;真实服务进程 kill/重启×4+静默 310s)→ 证据根 pi/b45-t24-crash"]
async fn b45_live_review_crash() {
    require_lc_gateway_e2e_switch();
    let harness = LiveLcGatewayHarness::new();
    let evidence_root = b45_t24_evidence_root("b45-t24-crash");
    let run = harness
        .run_b45_t24_crash(&evidence_root)
        .await
        .unwrap_or_else(|failure| panic!("b45 t24 crash 现场执行失败:{failure:?}"));
    std::fs::create_dir_all(&evidence_root).expect("证据目录");
    std::fs::write(
        evidence_root.join("run.json"),
        serde_json::to_vec_pretty(&run).expect("run json"),
    )
    .expect("run.json 落盘");
    if let Err(failures) = validate_b45_t24_crash_evidence(&run) {
        panic!("b45 t24 crash 证据 validator 拒绝:{failures:?}");
    }
}

/// Step 4 真实执行:取消格(`B45_T24_CANCEL_PROVIDERS` 子集;缺省三家)。
#[tokio::test]
#[ignore = "b45 真实现场:LC_GATEWAY_E2E=1 且 Step 4 取消格执行(需要各家 CLI 登录态;B45_T24_CANCEL_PROVIDERS 指定子集)→ 证据根 pi/b45-t24-cancel"]
async fn b45_live_review_cancel() {
    require_lc_gateway_e2e_switch();
    let harness = LiveLcGatewayHarness::new();
    let providers = b45_t24_cancel_providers_from_env();
    let expected = b45_t24_cancel_labels(&providers);
    let evidence_root = b45_t24_evidence_root("b45-t24-cancel");
    let run = harness
        .run_b45_t24_cancel_cells(&evidence_root, &providers)
        .await
        .unwrap_or_else(|failure| panic!("b45 t24 取消格现场执行失败:{failure:?}"));
    std::fs::create_dir_all(&evidence_root).expect("证据目录");
    std::fs::write(
        evidence_root.join("run.json"),
        serde_json::to_vec_pretty(&run).expect("run json"),
    )
    .expect("run.json 落盘");
    if let Err(failures) = validate_b45_t24_cancel_evidence(&run, &expected) {
        panic!("b45 t24 取消格证据 validator 拒绝:{failures:?}");
    }
}

// ---------------------------------------------------------------------------
// validator 单测(红测合同:合成夹具缺项必须 FAIL;完整夹具 PASS)。
// ---------------------------------------------------------------------------

#[cfg(test)]
mod validator_replay_tests {
    use super::*;

    fn window_record(state: &str, attempt_id: &str) -> Value {
        json!({
            "attempt_id": attempt_id,
            "state": state,
            "restore": {
                "outcome": "restored",
                "git_restored_mutation_paths": [],
                "git_extra_refs_observed": ["refs/heads/coder_probe"],
            },
            "updated_at": "2026-10-19T00:00:00Z",
        })
    }

    fn base_normal() -> Value {
        json!({
            "scenario": "replay-normal",
            "fixtures": {"fixture_root": "/tmp/aria-bug45/t24"},
            "coding_main": {
                "attempt_id": "coding_attempt_0001",
                "role_run_rows": [
                    {"payload": {"command": "bash fixtures/pi-incident/verify-normal.sh", "output": "[verify] container exact_id=abc123 --label io.aria.b45.scene=replay-normal"}},
                ],
            },
            "windows_final": {
                "windows": [window_record("closed_accepted", "coding_attempt_0001")],
                "violation_journals": [],
            },
            "window_open_facts": {"worktree_state": {"head": "sha-open"}},
            "worktree_final": {"head": "sha-open"},
            "code_review_reports": [
                {"id": "code_review_0001", "acceptance": "Accepted", "verdict": "Approve"},
            ],
        })
    }

    #[test]
    fn replay_validator_rejects_missing_restore_journal() {
        let mut evidence = base_normal();
        evidence["windows_final"]["windows"] = json!([{
            "attempt_id": "coding_attempt_0001",
            "state": "closed_accepted",
        }]);
        let failures = validate_b45_t24_replay_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("恢复前 journal")),
            "{failures:?}"
        );
    }

    #[test]
    fn replay_validator_rejects_missing_three_domain_git_facts() {
        let mut evidence = base_normal();
        evidence["windows_final"]["windows"] = json!([{
            "attempt_id": "coding_attempt_0001",
            "state": "closed_accepted",
            "restore": {"outcome": "restored"},
        }]);
        let failures = validate_b45_t24_replay_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("git 域")),
            "{failures:?}"
        );
    }

    #[test]
    fn replay_validator_rejects_missing_dirty_summary() {
        let mut evidence = base_normal();
        evidence["window_open_facts"] = json!(null);
        let failures = validate_b45_t24_replay_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("dirty 摘要")),
            "{failures:?}"
        );
    }

    #[test]
    fn replay_validator_rejects_missing_container_ownership() {
        let mut evidence = base_normal();
        evidence["coding_main"]["role_run_rows"] = json!([{"payload": {}}]);
        let failures = validate_b45_t24_replay_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("精确容器归属")),
            "{failures:?}"
        );
    }

    #[test]
    fn replay_validator_accepts_complete_normal_evidence() {
        assert!(validate_b45_t24_replay_evidence(&base_normal()).is_ok());
    }

    fn base_violation() -> Value {
        json!({
            "scenario": "replay-violation",
            "fixtures": {"fixture_root": "/tmp/aria-bug45/t24"},
            "coding_main": {"attempt_id": "coding_attempt_0001", "role_run_rows": []},
            "windows_final": {
                "windows": [window_record("manual_gate", "coding_attempt_0001")],
                "violation_journals": [
                    {"window_id": "w", "entries": [
                        {"kind": "git_self_commit", "path": "HEAD", "detail": "self-commit probe"},
                    ]},
                ],
            },
            "window_open_facts": {
                "worktree_state": {"head": "sha-open"},
                "bare_head_open": "bare-open",
                "index_digest_open": "idx-open",
            },
            "worktree_final": {"head": "sha-open"},
            "index_digest_final": "idx-open",
            "bare_head_final": "bare-open",
            "code_review_reports": [
                {"id": "code_review_0001", "acceptance": "Invalidated", "verdict": "Approve"},
            ],
            "manual_gate": {
                "resolve_status": 200,
                "after": {"windows": [window_record("closed_invalidated", "coding_attempt_0001")]},
            },
            "out_of_window_control": {
                "add": {"exit": 0},
                "commit": {"exit": 0},
                "push": {"exit": 0},
            },
        })
    }

    #[test]
    fn replay_validator_rejects_missing_manual_gate_verdict() {
        let mut evidence = base_violation();
        evidence["manual_gate"] = json!({"resolve_status": 404});
        let failures = validate_b45_t24_replay_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("人工门裁决")),
            "{failures:?}"
        );
    }

    #[test]
    fn replay_validator_rejects_platform_progression() {
        let mut evidence = base_violation();
        evidence["worktree_final"]["head"] = json!("sha-moved");
        let failures = validate_b45_t24_replay_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("平台零推进")),
            "{failures:?}"
        );
    }

    #[test]
    fn replay_validator_rejects_bare_head_or_index_progression() {
        let mut evidence = base_violation();
        evidence["bare_head_final"] = json!("bare-moved");
        let failures = validate_b45_t24_replay_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("裸远端")),
            "{failures:?}"
        );
        let mut evidence = base_violation();
        evidence["index_digest_final"] = json!("idx-moved");
        let failures = validate_b45_t24_replay_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("index 摘要")),
            "{failures:?}"
        );
    }

    #[test]
    fn replay_validator_allows_never_created_remote_branch() {
        let mut evidence = base_violation();
        evidence["window_open_facts"]["bare_head_open"] = Value::Null;
        evidence["bare_head_final"] = Value::Null;
        assert!(validate_b45_t24_replay_evidence(&evidence).is_ok());
    }

    #[test]
    fn replay_validator_rejects_missing_bare_head_keys() {
        // 缺键(字段改名/漏发射)≠ 存在且 null:必须 FAIL(缺项必须 FAIL 契约)。
        let mut evidence = base_violation();
        evidence["window_open_facts"]
            .as_object_mut()
            .expect("window_open_facts object")
            .remove("bare_head_open");
        evidence
            .as_object_mut()
            .expect("evidence object")
            .remove("bare_head_final");
        let failures = validate_b45_t24_replay_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("键缺失")),
            "{failures:?}"
        );
    }

    #[test]
    fn replay_validator_rejects_failed_out_of_window_control() {
        let mut evidence = base_violation();
        evidence["out_of_window_control"]["push"]["exit"] = json!(128);
        let failures = validate_b45_t24_replay_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("窗口外")),
            "{failures:?}"
        );
    }

    #[test]
    fn replay_validator_accepts_complete_violation_evidence() {
        assert!(validate_b45_t24_replay_evidence(&base_violation()).is_ok());
    }
}

#[cfg(test)]
mod validator_crash_tests {
    use super::*;

    fn crash_base() -> Value {
        json!({
            "kill": {
                "server_dead": true,
                "windows_at_kill": [{"state": "running"}],
                "scopes_at_kill": [
                    {"record": {"resource_id": "old-writer-daemon", "kind": "process"}},
                ],
                "role_runs_at_kill": [],
            },
            "restart": {
                "restarted_observed_at": "2026-10-19T00:10:00Z",
                "daemon_alive_after_restart": false,
                "scopes_after_restart": [],
                "window_one_terminal": [
                    {
                        "attempt_id": "a1",
                        "state": "closed_invalidated",
                        "updated_at": "2026-10-19T00:05:00Z",
                        "restore": {
                            "outcome": "restored",
                            "git_restored_mutation_paths": [],
                        },
                    },
                ],
            },
            "new_material": {
                "attempt_state": {"status": "completed"},
                "reviewer_silent_gap_secs": B45_T24_SILENT_CHECK_SECS,
                "pump_resume": {"events": []},
            },
            "drill_b": {
                "park_state": {"matched": true},
                "park_log_has_kept_locked": true,
                "daemon_alive_after_restart": true,
                "container_alive_after_restart": true,
                "resolved_state": {"matched": true},
            },
        })
    }

    #[test]
    fn crash_validator_rejects_missing_kill_fact() {
        let mut evidence = crash_base();
        evidence["kill"]["server_dead"] = json!(false);
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("kill 事实")),
            "{failures:?}"
        );
    }

    #[test]
    fn crash_validator_rejects_missing_pre_start_zero_facts() {
        let mut evidence = crash_base();
        evidence["kill"]["role_runs_at_kill"] = Value::Null;
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("零 provider")),
            "{failures:?}"
        );
    }

    #[test]
    fn crash_validator_rejects_live_old_writer() {
        let mut evidence = crash_base();
        evidence["restart"]["daemon_alive_after_restart"] = json!(true);
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("旧 writer")),
            "{failures:?}"
        );
    }

    #[test]
    fn crash_validator_rejects_uncleaned_registration() {
        let mut evidence = crash_base();
        evidence["restart"]["scopes_after_restart"] = json!([
            {"record": {"resource_id": "old-writer-daemon", "kind": "process"}},
        ]);
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("未清")),
            "{failures:?}"
        );
    }

    #[test]
    fn crash_validator_rejects_compensation_after_material() {
        let mut evidence = crash_base();
        evidence["restart"]["window_one_terminal"][0]["updated_at"] =
            json!("2026-10-19T00:20:00Z");
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("补偿未先行")),
            "{failures:?}"
        );
    }

    #[test]
    fn crash_validator_rejects_short_silence_or_kill() {
        let mut evidence = crash_base();
        evidence["new_material"]["reviewer_silent_gap_secs"] = json!(120);
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("静默 CHECK")),
            "{failures:?}"
        );
        let mut stalled = crash_base();
        stalled["new_material"]["pump_resume"]["events"] =
            json!([{"type": "coding_execution_event", "detail": "provider_idle_stalled"}]);
        let failures = validate_b45_t24_crash_evidence(&stalled).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("误杀")),
            "{failures:?}"
        );
    }

    #[test]
    fn crash_validator_rejects_missing_park_arm() {
        let mut evidence = crash_base();
        evidence["drill_b"] = json!({"skipped": "attempt #2 未建"});
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("停等")),
            "{failures:?}"
        );
    }

    #[test]
    fn crash_validator_rejects_killed_unknown_daemon() {
        let mut evidence = crash_base();
        evidence["drill_b"]["daemon_alive_after_restart"] = json!(false);
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("误杀")),
            "{failures:?}"
        );
    }

    #[test]
    fn crash_validator_accepts_complete_evidence() {
        assert!(validate_b45_t24_crash_evidence(&crash_base()).is_ok());
    }
}

#[cfg(test)]
mod cancel_providers_tests {
    use super::*;

    fn labels(raw: Option<&str>) -> Result<Vec<&'static str>, String> {
        b45_t24_parse_cancel_providers(raw).map(|providers| b45_t24_cancel_labels(&providers))
    }

    #[test]
    fn cancel_providers_default_to_three() {
        assert_eq!(
            labels(None).unwrap(),
            vec!["claude", "codex", "kimi"],
            "env 未设必须默认三家"
        );
    }

    #[test]
    fn cancel_providers_accept_subset() {
        assert_eq!(labels(Some("claude,kimi")).unwrap(), vec!["claude", "kimi"]);
    }

    #[test]
    fn cancel_providers_normalize_case_and_duplicates() {
        assert_eq!(
            labels(Some(" Kimi , claude ,kimi")).unwrap(),
            vec!["kimi", "claude"]
        );
    }

    #[test]
    fn cancel_providers_reject_empty_set() {
        let error = labels(Some("")).unwrap_err();
        assert!(error.contains("空集合"), "{error}");
        let error = labels(Some(" , ")).unwrap_err();
        assert!(error.contains("空集合"), "{error}");
    }

    #[test]
    fn cancel_providers_reject_unknown_token() {
        let error = labels(Some("claude,gemini")).unwrap_err();
        assert!(error.contains("gemini"), "{error}");
        let error = labels(Some("pi")).unwrap_err();
        assert!(error.contains("pi"), "{error}");
    }
}

#[cfg(test)]
mod validator_cancel_tests {
    use super::*;

    const THREE: [&str; 3] = ["claude", "codex", "kimi"];

    fn cell(label: &str, aborted: bool) -> Value {
        json!({
            "attempt_id": format!("coding_attempt_{label}"),
            "cancel_cell": {
                "abort_sent": true,
                "terminal_status": if aborted { "aborted" } else { "failed" },
                "provider_pid": "4242",
                "provider_pid_still_alive": !aborted,
                "lingering_fixture_containers": [],
            },
            "attempt_state": {
                "status": 200,
                "body": {"attempt": {
                    "status": if aborted { "aborted" } else { "running" },
                    "completed_at": if aborted {
                        json!("2026-10-19T01:00:00Z")
                    } else {
                        Value::Null
                    },
                }},
            },
            "resource_scopes_after": [],
            "role_run_rows_after_cancel": [
                {
                    "role_run_id": format!("run_{label}"),
                    "role": "coder",
                    "sequence": 0,
                    "created_at": "2026-10-19T00:59:00Z",
                },
                {
                    "role_run_id": format!("run_{label}"),
                    "role": "coder",
                    "sequence": 1,
                    "created_at": "2026-10-19T00:59:30Z",
                },
            ],
        })
    }

    fn base() -> Value {
        json!({
            "providers": THREE,
            "cells": {
                "claude": cell("claude", true),
                "codex": cell("codex", true),
                "kimi": cell("kimi", true),
            },
        })
    }

    #[test]
    fn cancel_validator_rejects_empty_expected_set() {
        let failures = validate_b45_t24_cancel_evidence(&base(), &[]).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("空集合")),
            "{failures:?}"
        );
    }

    #[test]
    fn cancel_validator_rejects_missing_provider() {
        let mut evidence = base();
        evidence["cells"]
            .as_object_mut()
            .expect("cells object")
            .remove("kimi");
        let failures = validate_b45_t24_cancel_evidence(&evidence, &THREE).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("缺家")),
            "{failures:?}"
        );
    }

    #[test]
    fn cancel_validator_rejects_extra_provider() {
        let mut evidence = base();
        evidence["providers"] = json!(["claude", "codex", "kimi", "pi"]);
        evidence["cells"]["pi"] = cell("pi", true);
        let failures = validate_b45_t24_cancel_evidence(&evidence, &THREE).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("多余")),
            "{failures:?}"
        );
    }

    #[test]
    fn cancel_validator_rejects_empty_cells() {
        // env 指定了集合(非空期望)但证据里没有任何 cell:空集合必须红。
        let evidence = json!({"providers": [], "cells": {}});
        let failures = validate_b45_t24_cancel_evidence(&evidence, &THREE).unwrap_err();
        assert!(
            failures
                .iter()
                .any(|f| f.contains("空集合") || f.contains("缺家")),
            "{failures:?}"
        );
    }

    #[test]
    fn cancel_validator_rejects_unaborted_provider() {
        let mut evidence = base();
        evidence["cells"]["codex"] = cell("codex", false);
        let failures = validate_b45_t24_cancel_evidence(&evidence, &THREE).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("未 aborted")),
            "{failures:?}"
        );
    }

    #[test]
    fn cancel_validator_rejects_live_provider_process() {
        let mut evidence = base();
        evidence["cells"]["claude"]["cancel_cell"]["provider_pid_still_alive"] = json!(true);
        let failures = validate_b45_t24_cancel_evidence(&evidence, &THREE).unwrap_err();
        assert!(
            failures
                .iter()
                .any(|f| f.contains("进程未终止") || f.contains("遗留")),
            "{failures:?}"
        );
    }

    #[test]
    fn cancel_validator_rejects_non_terminal_attempt() {
        let mut evidence = base();
        evidence["cells"]["kimi"]["attempt_state"]["body"]["attempt"]["completed_at"] = Value::Null;
        let failures = validate_b45_t24_cancel_evidence(&evidence, &THREE).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("attempt 未达终态")),
            "{failures:?}"
        );
    }

    #[test]
    fn cancel_validator_rejects_unclosed_resource_scope() {
        let mut evidence = base();
        evidence["cells"]["claude"]["resource_scopes_after"] = json!([
            {"path": "/tmp/x/resources/res_0001.json", "record": {"resource_id": "res_0001", "cleanup_cursor": "confirmed"}},
        ]);
        let failures = validate_b45_t24_cancel_evidence(&evidence, &THREE).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("资源未收口")),
            "{failures:?}"
        );
    }

    #[test]
    fn cancel_validator_rejects_missing_resource_snapshot() {
        let mut evidence = base();
        evidence["cells"]["codex"]
            .as_object_mut()
            .expect("cell object")
            .remove("resource_scopes_after");
        let failures = validate_b45_t24_cancel_evidence(&evidence, &THREE).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("resource_scopes_after")),
            "{failures:?}"
        );
    }

    #[test]
    fn cancel_validator_rejects_spawn_after_cancel() {
        let mut evidence = base();
        evidence["cells"]["kimi"]["role_run_rows_after_cancel"]
            .as_array_mut()
            .expect("role rows array")
            .push(json!({
                "role_run_id": "run_kimi_restart",
                "role": "coder",
                "sequence": 0,
                "created_at": "2026-10-19T01:00:30Z",
            }));
        let failures = validate_b45_t24_cancel_evidence(&evidence, &THREE).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("新 spawn")),
            "{failures:?}"
        );
    }

    #[test]
    fn cancel_validator_rejects_missing_spawn_baseline() {
        let mut evidence = base();
        evidence["cells"]["claude"]["role_run_rows_after_cancel"] = json!([]);
        let failures = validate_b45_t24_cancel_evidence(&evidence, &THREE).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("基线")),
            "{failures:?}"
        );
    }

    #[test]
    fn cancel_validator_rejects_unparsable_spawn_rows() {
        let mut evidence = base();
        evidence["cells"]["claude"]["role_run_rows_after_cancel"] =
            json!([{"role": "coder", "sequence": 0}]);
        let failures = validate_b45_t24_cancel_evidence(&evidence, &THREE).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("基线")),
            "{failures:?}"
        );
    }

    #[test]
    fn cancel_validator_rejects_failed_cell_start() {
        let mut evidence = base();
        evidence["cells"]["claude"] = json!({"error": "规划未 Confirmed,取消格无法启动"});
        let failures = validate_b45_t24_cancel_evidence(&evidence, &THREE).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("未执行")),
            "{failures:?}"
        );
    }

    #[test]
    fn cancel_validator_accepts_complete_evidence() {
        assert!(validate_b45_t24_cancel_evidence(&base(), &THREE).is_ok());
    }

    #[test]
    fn cancel_validator_accepts_single_provider_subset() {
        let evidence = json!({
            "providers": ["codex"],
            "cells": {"codex": cell("codex", true)},
        });
        assert!(validate_b45_t24_cancel_evidence(&evidence, &["codex"]).is_ok());
    }
}

