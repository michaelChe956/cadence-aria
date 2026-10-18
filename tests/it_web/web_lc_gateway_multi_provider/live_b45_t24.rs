//! 批2 Task 24(b45):Pi 事故副本重放与真崩溃恢复的现场测试与 validator。
//!
//! - `b45_live_pi_review_replay`(Step 3):两独立场景各一轮真实现场——
//!   正常轮(合法验证→Accepted+worktree 恢复+Coder dirty 保留)与违规轮
//!   (主动 commit/写 `.git`/`.aria`→Invalidated+人工门+平台零推进),
//!   外加窗口外 Coder 正常提交/推送对照。
//! - `b45_live_review_crash_recovery`(Step 4):真实 `aria web` 服务进程
//!   kill -9/重启的完整演练(停旧 writer/清登记容器/原 manifest 续恢复/
//!   三域核对→再放行新材料;停等保持锁不误杀;静默 CHECK 跨 300s;
//!   其余三家各一取消/资源收口格)。
//!
//! validator(红测合同,Step 1):缺恢复前 journal、任一三域结果、dirty
//! 摘要、精确容器归属、人工门裁决或启动前零材料/零 provider 事实均 FAIL
//! ——`validator_replay_*`/`validator_crash_*` 单测钉死(合成夹具,红先)。
//!
//! 分批收证:`B45_T24_CANCEL_PROVIDERS`(逗号分隔 claude/codex/kimi 子集,
//! 缺省三家全跑)。

use std::path::PathBuf;

use serde_json::{Value, json};

use super::harness::{B45_T24_SILENT_CHECK_SECS, LiveLcGatewayHarness};
use super::live_matrix::require_lc_gateway_e2e_switch;

/// T24 证据根(计划 Files:cadence/reports/lc-gateway-multi-provider/pi/)。
fn b45_t24_evidence_root(facet: &str) -> PathBuf {
    PathBuf::from("cadence/reports/lc-gateway-multi-provider/pi").join(facet)
}

/// 取消格三家清集(env 缺省=三家全跑)。
fn b45_t24_cancel_providers_from_env() -> Vec<cadence_aria::product::models::ProviderName> {
    use cadence_aria::product::models::ProviderName;
    let raw = std::env::var("B45_T24_CANCEL_PROVIDERS").unwrap_or_default();
    let mut providers = Vec::new();
    for token in raw.split(',') {
        let token = token.trim().to_lowercase();
        let provider = match token.as_str() {
            "claude" => Some(ProviderName::ClaudeCode),
            "codex" => Some(ProviderName::Codex),
            "kimi" => Some(ProviderName::KimiCode),
            _ => None,
        };
        if let Some(provider) = provider {
            providers.push(provider);
        }
    }
    if providers.is_empty() {
        providers = vec![
            ProviderName::ClaudeCode,
            ProviderName::Codex,
            ProviderName::KimiCode,
        ];
    }
    providers
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
// 现场测试(#[ignore];LC_GATEWAY_E2E=1 + --ignored 命中)。
// ---------------------------------------------------------------------------

/// Step 3 真实执行:Pi 事故副本重放(两独立场景)。
#[tokio::test]
#[ignore = "b45 真实现场:LC_GATEWAY_E2E=1 且 Step 3 命令执行(需要 pi 登录态/podman;两场景各一轮全链)"]
async fn b45_live_pi_review_replay() {
    require_lc_gateway_e2e_switch();
    let harness = LiveLcGatewayHarness::new();
    for scenario in [
        super::harness::B45T24Scenario::ReplayNormal,
        super::harness::B45T24Scenario::ReplayViolation,
    ] {
        let evidence_root = b45_t24_evidence_root(&format!("b45-t24-{}", scenario.label()));
        let run = harness
            .run_b45_t24_replay(&evidence_root, scenario)
            .await
            .unwrap_or_else(|failure| {
                panic!("b45 t24 replay({})现场执行失败:{:?}", scenario.label(), failure)
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
}

/// Step 4 真实执行:真崩溃恢复(Pi 完整演练 + 三家取消格)。
#[tokio::test]
#[ignore = "b45 真实现场:LC_GATEWAY_E2E=1 且 Step 4 命令执行(需要 pi 登录态/podman;真实服务进程 kill/重启×4+静默 310s)"]
async fn b45_live_review_crash_recovery() {
    require_lc_gateway_e2e_switch();
    let harness = LiveLcGatewayHarness::new();
    let evidence_root = b45_t24_evidence_root("b45-t24-crash");
    let run = harness
        .run_b45_t24_crash(&evidence_root)
        .await
        .unwrap_or_else(|failure| panic!("b45 t24 crash 现场执行失败:{failure:?}"));
    let cancel = harness
        .run_b45_t24_cancel_cells(
            &evidence_root,
            &b45_t24_cancel_providers_from_env(),
        )
        .await
        .unwrap_or_else(|failure| panic!("b45 t24 取消格现场执行失败:{failure:?}"));
    let mut full = run;
    full["cancel_cells"] = cancel;
    std::fs::create_dir_all(&evidence_root).expect("证据目录");
    std::fs::write(
        evidence_root.join("run.json"),
        serde_json::to_vec_pretty(&full).expect("run json"),
    )
    .expect("run.json 落盘");
    if let Err(failures) = validate_b45_t24_crash_evidence(&full) {
        panic!("b45 t24 crash 证据 validator 拒绝:{failures:?}");
    }
    // 取消格判绿:每家 aborted。
    let cells = full
        .pointer("/cancel_cells/cells")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for (provider, cell) in cells {
        let aborted = cell
            .pointer("/cancel_cell/aborted")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        assert!(aborted, "{provider} 取消格未 aborted:{cell}");
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
