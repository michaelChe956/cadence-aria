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
//!   已删除;crash validator 本身不引用 cancel 面)。正向臂=生产 adapter
//!   (B1)真实登记:kill 时采集 kind=process/confirmed + 真实 PID+`/proc`
//!   starttime + 窗口/attempt 业务归属的登记记录,补偿后同一记录注销且
//!   `/proc/<pid>` 消失(不再预置合成 `old-writer-daemon`)。另含 A7 两
//!   场景断言:(a) mid-commit journal 不抢跑——kill 时 ReviewRequest
//!   journal 停在 `commit_started`,重启后收敛到终态且收敛时刻不早于窗口
//!   恢复完成;(b) 停等双臂(各占**独立 scope**,不混排)——writer 臂:
//!   预置未确认 writer(无容器记录)由 quiesce 未知 writer 分支触发
//!   RecoveryRequired,park 保锁、登记记录不注销、daemon 不误杀、停等期
//!   登记面 audits 逐字不变;容器臂:已登记容器在独立 scope 内不被清理/
//!   不 deregister(登记记录在场 + audits 不变 + 人工处置后才移除),但
//!   生产恢复路径无 engine observer(call site 传 None),容器实例存活
//!   只记录不断言(`observability="no_production_observer"`);无登记记录
//!   的未知容器对平台不可观测(quiesce 只消费登记面)——如实标注
//!   `unobservable_by_platform`,存活字段只记录不断言。
//! - `b45_live_review_cancel`(Step 4 取消格):按 `B45_T24_CANCEL_PROVIDERS`
//!   指定子集(缺省 claude,codex,kimi)各一格真实取消与资源收口;证据根
//!   `pi/b45-t24-cancel`。
//!
//! validator(红测合同,Step 1;缺项即 FAIL):replay 缺恢复前 journal、任一
//! 三域结果、dirty 摘要、精确容器归属、人工门裁决即 FAIL;crash 缺 kill
//! 事实、启动前零材料/零 provider 事实、真实登记(PID+starttime 与
//! 窗口/attempt 归属)、补偿后登记注销与 `/proc/<pid>` 消失、续恢复、静默
//! CHECK≥310s、A7(a) journal 在途相位与收敛先后关系、A7(b) 双臂独立
//! scope/writer 停等原因归属/停等期 audits 不变/已登记容器不被清理、
//! 停等臂即 FAIL;cancel 的 provider 集合不精确
//! (空集合/缺家/多余)、未 aborted、进程/容器未终止、attempt 非终态、资源
//! 未收口、取消后新 spawn 即 FAIL——`validator_replay_*`/`validator_crash_*`/
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
    B45_T24_SILENT_CHECK_SECS, B45T24Scenario, LiveLcGatewayHarness, b45_t24_cancel_provider_label,
};
use super::live_matrix::require_lc_gateway_e2e_switch;

/// A7(b) 停等双臂契约(harness 落证据与 validator 断言共用同一组常量,
/// 防两侧字面量漂移):
/// - 两臂各占**独立 scope**:同 scope 混排时 `list_scope` 按 resource_id
///   排序,先命中的记录会抢占 quiesce 停等原因,另一臂的触发者不会被
///   演练(F1——drill B 曾因此"通过但证明错对象");
/// - writer 臂的停等原因必须是未确认 writer 类;
/// - 容器臂在**生产恢复路径**必然 observer_missing:所有
///   `open_for_recovery` call site 传 `engine_observer=None`,生产无
///   `OwnedEngineObserver` 实现——容器实例存活不是平台行为证据(只记录
///   不断言,见 `B45_T24_CONTAINER_OBSERVABILITY`)。
pub(crate) const B45_T24_WRITER_SCOPE_ID: &str = "b45-t24-park-writer";
pub(crate) const B45_T24_CONTAINER_SCOPE_ID: &str = "b45-t24-park-container";
pub(crate) const B45_T24_WRITER_RESOURCE_ID: &str = "unconfirmed-writer";
pub(crate) const B45_T24_CONTAINER_RESOURCE_ID: &str = "park-container";
/// 补偿驱动停等落账的稳定原因码(`park_window_recovery` 调用点)。
pub(crate) const B45_T24_PARK_QUIESCE_REASON: &str = "startup_compensation_quiesce_not_proven";
/// quiesce 未知 writer 停等码(`quiesce_scope` Process 未确认分支)。
pub(crate) const B45_T24_UNCONFIRMED_WRITER_QUIESCE_REASON: &str =
    "provider_resources_quiesce_unconfirmed_process_unknown";
/// quiesce 引擎观察通道缺失停等码(`quiesce_scope` Container/Network
/// 分支 observer=None)。
pub(crate) const B45_T24_OBSERVER_MISSING_QUIESCE_REASON: &str =
    "provider_resources_quiesce_observer_missing";
/// 已登记容器臂的可观测口径:生产恢复路径无 observer,容器实例存活
/// 只记录不断言。
pub(crate) const B45_T24_CONTAINER_OBSERVABILITY: &str = "no_production_observer";
pub(crate) const B45_T24_CONTAINER_OBSERVABILITY_NOTE: &str = "生产恢复路径 open_for_recovery 全部 call site 传 engine_observer=None(code_review.rs/provider_stream.rs),全仓无生产 OwnedEngineObserver 实现——平台永不 stop_exact,容器实例存活只记录不断言;可红面=登记记录在场+登记面 audits 逐字不变+人工处置后才移除";

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
    providers
        .iter()
        .map(b45_t24_cancel_provider_label)
        .collect()
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
        .into_iter()
        .flatten()
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
    let scenario = evidence["scenario"]
        .as_str()
        .unwrap_or_default()
        .to_string();
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
        failures.push(format!(
            "缺 attempt {attempt_id} 的 reviewer 窗口记录(开窗未发生=未达评审)"
        ));
    }
    // 恢复事实(任一窗口必须携带 restore 绑定)。
    let restored_any = t24_window_of(&windows, &attempt_id).iter().any(|window| {
        window.get("restore").is_some_and(|restore| {
            restore["outcome"]
                .as_str()
                .is_some_and(|outcome| outcome == "restored")
        })
    });
    if !restored_any {
        failures.push(
            "缺恢复前 journal/恢复绑定:窗口无 restore{outcome=restored}(恢复未完成=不可采信)"
                .to_string(),
        );
    }
    // 三域结果:git 域字段在场(restore 绑定含 git 域结构化事实)。
    let git_domain_any = t24_window_of(&windows, &attempt_id).iter().any(|window| {
        window["restore"]
            .get("git_restored_mutation_paths")
            .is_some()
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
    if evidence
        .get("worktree_final")
        .map(Value::is_null)
        .unwrap_or(true)
    {
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
                report["acceptance"]
                    .as_str()
                    .is_some_and(|a| a.eq_ignore_ascii_case("accepted"))
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
                failures.push(format!(
                    "缺人工门裁决(resolve 状态非 200:{resolve_status:?})"
                ));
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
                failures.push(
                    "人工门裁决后窗口未释放(ClosedInvalidated/NewReviewRequired)".to_string(),
                );
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
                    _ => {
                        failures.push("平台零推进对照裸远端 HEAD 值形态非法(非串/空串)".to_string())
                    }
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
/// - 正向臂=真实登记(B1 生产 adapter 写入,不是 fixture preset):kill 时
///   登记面含 kind=process/cleanup_cursor=confirmed 的记录,携带真实
///   PID+`/proc` starttime(与 `/proc` 逐字核对)与窗口/attempt/run/
///   worktree 业务归属;补偿后同一登记项注销且 `/proc/<pid>` 消失;
/// - 原 manifest 续恢复:窗口 #1 达终态且 restore 绑定在场
///   (outcome=restored),且窗口释放时刻早于重启观测时刻(补偿先行);
/// - 三域核对:窗口 restore 的 git 域结构化事实在场;
/// - 再放行新材料:attempt #1 终态 completed(重启后续驱完成);
/// - 静默 CHECK:reviewer 静默跨度 ≥300s 且无 stalled(误杀)证据;
/// - A7(a) mid-commit journal 不抢跑:kill 时 ReviewRequest journal 停在
///   `commit_started`(提交进行中),重启后收敛到终态,且收敛时刻不早于
///   窗口恢复完成时刻(journal 在途/post-restore 先后关系可证);
/// - A7(b) 停等双臂(各占独立 scope,禁止同 scope 混排——同 scope 时
///   `list_scope` 按 resource_id 排序会抢占停等触发者):
///   - writer 臂:未确认 writer(无容器记录)必须由 quiesce 未知 writer
///     分支触发 RecoveryRequired——窗口 recovery.detail 须含
///     `provider_resources_quiesce_unconfirmed_process_unknown` 且
///     `resource=unconfirmed-writer`(若 detail 来自容器 observer_missing,
///     说明"通过但证明错对象"→ FAIL);登记记录与 daemon 原样保留到人工
///     处置;
///   - 已登记容器臂(**可观测半边=登记面**;实例存活不可观测):独立
///     scope 内登记记录不得被清理/注销
///     (record_present_after_restart + 停等期 audits 逐字不变 + 人工
///     处置后才移除),停等原因须为本 scope 的
///     `provider_resources_quiesce_observer_missing`;生产恢复路径无
///     engine observer,容器实例存活只记录不断言(证据须标注
///     `observability="no_production_observer"`);
///   - 未知容器臂(**不可观测半边**):无登记记录 → 平台不可观测(quiesce
///     只消费登记面),如实标注 `unobservable_by_platform`,存活字段只
///     记录不断言,人工处置完成仍须可证;
/// - 停等不误杀臂(drill B):park 态 recovery_required + kept locked 日志
///   (每臂各自的重启)。
pub(crate) fn validate_b45_t24_crash_evidence(evidence: &Value) -> Result<(), Vec<String>> {
    let mut failures = Vec::new();
    // kill 事实。
    if evidence
        .pointer("/kill/server_dead")
        .and_then(Value::as_bool)
        != Some(true)
    {
        failures.push("缺 kill 事实(server 未死=真进程 kill 未发生)".to_string());
    }
    // 启动前零材料/零 provider 事实(kill 时快照三面在场)。
    for facet in [
        "/kill/windows_at_kill",
        "/kill/scopes_at_kill",
        "/kill/role_runs_at_kill",
    ] {
        if evidence.pointer(facet).and_then(Value::as_array).is_none() {
            failures.push(format!("缺启动前零材料/零 provider 事实({facet} 快照缺失)"));
        }
    }
    // 正向臂:真实登记(生产 adapter 写入;fixture preset/合成记录不满足
    // 本形状——缺 PID/starttime/业务归属即红)。
    let registrations = evidence
        .pointer("/kill/real_registrations_at_kill")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if registrations.is_empty() {
        failures.push(
            "缺真实登记(kill 时无生产 adapter 登记的 writer;合成记录不算正向前提)".to_string(),
        );
    }
    let windows_at_kill = evidence
        .pointer("/kill/windows_at_kill")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let worktree_tree_key = evidence
        .pointer("/kill/worktree_tree_key")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let mut attributed = 0usize;
    let mut run_matched = 0usize;
    for record in &registrations {
        let resource_id = record["resource_id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let labels = &record["labels"];
        let pid = record["pid"].as_u64().unwrap_or(0);
        let start_ticks = record["start_time_ticks"].as_u64().unwrap_or(0);
        let identity_ok = record["kind"].as_str() == Some("process")
            && labels["kind"].as_str() == Some("provider_process")
            && labels["aria.scope_id"]
                .as_str()
                .is_some_and(|value| !value.trim().is_empty())
            && labels["aria.run_id"]
                .as_str()
                .is_some_and(|value| !value.trim().is_empty())
            && pid > 0
            && start_ticks > 0
            && record["pid_starttime_matches_proc"].as_bool() == Some(true)
            && record["cleanup_cursor"].as_str() == Some("confirmed");
        if !identity_ok {
            failures.push(format!(
                "真实登记缺真实 PID+starttime/confirmed 身份:{resource_id:?}(fixture preset 或未回填不算)"
            ));
            continue;
        }
        // 业务归属:窗口类登记(aria.window_id)必须是 kill 时真实窗口,
        // 窗口→attempt 关联在位,run/worktree 归属逐字一致;非窗口登记
        // (如同树 coder delivery)只要求 aria.worktree 同树归属。正向臂
        // 至少要有一条窗口/attempt 归属的登记(不能只凭进程名/前缀)。
        let window_id = record["window_id"]
            .as_str()
            .or_else(|| labels["aria.window_id"].as_str())
            .unwrap_or_default();
        let same_tree = worktree_tree_key.is_empty()
            || labels["aria.worktree"].as_str() == Some(worktree_tree_key.as_str());
        if window_id.trim().is_empty() {
            if !same_tree {
                failures.push(format!(
                    "真实登记缺窗口/attempt 归属:{resource_id:?} 无 aria.window_id 且非本树归属"
                ));
            }
            continue;
        }
        let Some(window) = windows_at_kill
            .iter()
            .find(|window| window["window_id"].as_str() == Some(window_id))
        else {
            failures.push(format!(
                "真实登记缺窗口/attempt 归属:{resource_id:?} 的 window_id={window_id:?} 不在 kill 时窗口面"
            ));
            continue;
        };
        let attempt_attributed = window["attempt_id"]
            .as_str()
            .is_some_and(|value| !value.trim().is_empty());
        // scope 内 run 一致性(登记自身身份)+ 至少一条 run 与窗口 run 同源
        // (窗口开窗 run=reviewer role run,launch 归属同源)。
        let run_consistent = labels["aria.run_id"]
            .as_str()
            .is_some_and(|value| !value.trim().is_empty())
            && record["run_id"].as_str() == labels["aria.run_id"].as_str();
        if !(attempt_attributed && run_consistent && same_tree) {
            failures.push(format!(
                "真实登记缺窗口/attempt 归属:{resource_id:?} 窗口={window_id:?} 的 attempt/run/worktree 归属不一致"
            ));
            continue;
        }
        if window["run_id"]
            .as_str()
            .is_some_and(|value| !value.trim().is_empty())
            && record["run_id"].as_str() == window["run_id"].as_str()
        {
            run_matched += 1;
        }
        attributed += 1;
    }
    if !registrations.is_empty() && attributed == 0 {
        failures.push("真实登记无一条带窗口/attempt 归属(业务归属不可证)".to_string());
    } else if attributed > 0 && run_matched == 0 {
        failures
            .push("真实登记 run 与 kill 时窗口 run 不同源(窗口/attempt 归属不可证)".to_string());
    }
    // 补偿后:登记项注销(durable 记录消失)。
    let registrations_after = evidence
        .pointer("/restart/real_registrations_after_restart")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let registration_key = |record: &Value| {
        (
            record["scope_id"].as_str().unwrap_or_default().to_string(),
            record["run_id"].as_str().unwrap_or_default().to_string(),
            record["boot_identity"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            record["resource_id"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        )
    };
    let surviving: Vec<String> = registrations
        .iter()
        .map(registration_key)
        .filter(|key| {
            registrations_after
                .iter()
                .map(registration_key)
                .any(|other| &other == key)
        })
        .map(|(_, _, _, resource_id)| resource_id)
        .collect();
    if !surviving.is_empty() {
        failures.push(format!(
            "登记未清(重启后仍含 kill 时真实登记记录):{surviving:?}"
        ));
    }
    // 补偿后 `/proc/<pid>` 消失(逐条按登记身份观察)。
    let compensated = evidence
        .pointer("/restart/compensated_processes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for record in &registrations {
        let pid = record["pid"].as_u64().unwrap_or(0);
        if pid == 0 {
            continue;
        }
        let gone = compensated
            .iter()
            .find(|entry| entry["pid"].as_u64() == Some(pid))
            .is_some_and(|entry| entry["proc_alive"].as_bool() == Some(false));
        if !gone {
            failures.push(format!(
                "登记 PID 未消失(补偿后 /proc/{pid} 仍存活或未观察——旧 writer 未停)"
            ));
        }
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
        failures.push(
            "缺原 manifest 续恢复/三域核对(窗口 #1 无 restore{restored}+git 域事实)".to_string(),
        );
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
    // A7(a):mid-commit journal 不抢跑。崩溃窗口内 ReviewRequest journal
    // 停在 commit_started(提交进行中);重启补偿必须等窗口恢复完成后才
    // 收敛 journal(才动 worktree)。先后时刻全部来自 durable 事实:
    // journal 在途时刻 ≤ 窗口恢复完成时刻 ≤ journal 收敛终态时刻。
    match evidence
        .get("kill")
        .and_then(|kill| kill.get("mid_commit_journal"))
    {
        None => failures
            .push("缺 mid-commit journal 在途事实(kill 时未造出提交进行中的 journal)".to_string()),
        Some(journal) => {
            let phase_at_kill = journal["phase"].as_str().unwrap_or_default();
            let in_flight_at = journal["in_flight_at"]
                .as_str()
                .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok());
            let attempt_id = journal["attempt_id"].as_str().unwrap_or_default();
            let before_head = journal["before_head"].as_str().unwrap_or_default();
            if phase_at_kill != "commit_started" {
                failures.push(format!(
                    "缺 mid-commit journal 在途事实(kill 时相位={phase_at_kill:?},须 commit_started)"
                ));
            }
            if in_flight_at.is_none()
                || attempt_id.trim().is_empty()
                || before_head.trim().is_empty()
            {
                failures.push(
                    "mid-commit journal 在途事实缺时刻/归属(attempt/in_flight_at/before_head)"
                        .to_string(),
                );
            }
            match evidence
                .get("restart")
                .and_then(|restart| restart.get("mid_commit_journal"))
            {
                None => failures
                    .push("缺 mid-commit journal 收敛事实(重启后未观察 journal 终态)".to_string()),
                Some(restart_journal) => {
                    let converged = restart_journal["converged"].as_bool() == Some(true);
                    let phase_after = restart_journal["phase_after"].as_str().unwrap_or_default();
                    let converged_at = restart_journal["converged_at"]
                        .as_str()
                        .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok());
                    if !converged
                        || !matches!(phase_after, "compensated" | "completed")
                        || converged_at.is_none()
                    {
                        failures.push(format!(
                            "mid-commit journal 未收敛(重启后 converged={converged} phase={phase_after:?})"
                        ));
                    }
                    let released_at = restart_journal["window_one_released_at"]
                        .as_str()
                        .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok());
                    match (in_flight_at, released_at, converged_at) {
                        (Some(in_flight), Some(released), Some(converged_at)) => {
                            if in_flight > released {
                                failures.push(format!(
                                    "mid-commit journal 在途时刻晚于窗口恢复完成(时序矛盾:{in_flight} > {released})"
                                ));
                            }
                            if converged_at < released {
                                failures.push(format!(
                                    "补偿抢跑:journal 收敛({converged_at})先于窗口恢复完成({released})"
                                ));
                            }
                        }
                        _ => failures.push(
                            "mid-commit journal 先后关系缺时刻(在途/恢复完成/收敛)".to_string(),
                        ),
                    }
                    if restart_journal["attempt_id"].as_str() != Some(attempt_id) {
                        failures
                            .push("mid-commit journal attempt 归属不一致(kill/重启)".to_string());
                    }
                    if restart_journal["before_head"].as_str() != Some(before_head) {
                        failures
                            .push("mid-commit journal before_head 不一致(kill/重启)".to_string());
                    }
                    let head_at_kill = restart_journal["head_at_kill"].as_str().unwrap_or_default();
                    if !before_head.trim().is_empty() && head_at_kill != before_head {
                        failures.push(format!(
                            "mid-commit journal 基线不一致:before_head={before_head:?} 现场 HEAD={head_at_kill:?}"
                        ));
                    }
                }
            }
        }
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
    // 停等不误杀臂(drill B)。A7(b) 双臂各占独立 scope;停等原因必须
    // 逐字来自本臂资源——否则是"通过但证明错对象"。
    let drill_b = &evidence["drill_b"];
    if drill_b.get("skipped").is_some() {
        failures.push(format!(
            "停等不误杀臂未演练:{}",
            drill_b["skipped"].as_str().unwrap_or("未知")
        ));
    } else {
        // 登记面 audits(provider-resources 记录 path+摘要)在停等前后
        // 必须逐字不变且基线非空(空基线不得冒充"未变")。
        let audits_unchanged = |arm: &Value| -> bool {
            let audits = &arm["audits"];
            let before = audits["before"].as_array().cloned().unwrap_or_default();
            let after = audits["after"].as_array().cloned().unwrap_or_default();
            audits["unchanged"].as_bool() == Some(true) && before == after && !before.is_empty()
        };
        let writer_arm = &drill_b["writer_arm"];
        let container_arm = &drill_b["container_arm"];
        let writer_scope = writer_arm["scope_id"].as_str().unwrap_or_default();
        let container_scope = container_arm["scope_id"].as_str().unwrap_or_default();
        if writer_scope.trim().is_empty()
            || container_scope.trim().is_empty()
            || writer_scope == container_scope
        {
            failures.push(format!(
                "A7(b) 停等双臂必须各占独立 scope(writer={writer_scope:?} container={container_scope:?}:\
                 同 scope 时 list_scope 按 resource_id 排序会抢占停等触发者,另一臂不会被演练)"
            ));
        }
        // 臂 a:未确认 writer(该 scope 无容器记录)——quiesce 未知 writer
        // 分支触发 RecoveryRequired;park + 保锁 + audits 不变;登记记录
        // 不注销,daemon 不误杀。
        let writer_recovery = &writer_arm["recovery"];
        let writer_detail = writer_recovery["detail"].as_str().unwrap_or_default();
        let writer_arm_ok = writer_arm["record_present_after_restart"].as_bool() == Some(true)
            && writer_arm["daemon_alive_after_kill"].as_bool() == Some(true)
            && writer_arm["daemon_alive_after_restart"].as_bool() == Some(true)
            && writer_recovery["reason"].as_str() == Some(B45_T24_PARK_QUIESCE_REASON)
            && writer_detail.contains(B45_T24_UNCONFIRMED_WRITER_QUIESCE_REASON)
            && writer_detail.contains(&format!("resource={B45_T24_WRITER_RESOURCE_ID}"))
            && !writer_detail.contains(&format!("resource={B45_T24_CONTAINER_RESOURCE_ID}"))
            && writer_arm["parked"].as_bool() == Some(true)
            && writer_arm["park_log_has_kept_locked"].as_bool() == Some(true)
            && audits_unchanged(writer_arm)
            && writer_arm["removed_by_manual_disposition"].as_bool() == Some(true);
        if !writer_arm_ok {
            failures.push(format!(
                "未确认 writer 臂不成立(须由 writer 资源触发未知 writer 类 RecoveryRequired:\
                 recovery.detail 含 {B45_T24_UNCONFIRMED_WRITER_QUIESCE_REASON} 与 \
                 resource={B45_T24_WRITER_RESOURCE_ID};park+kept locked;登记记录与 daemon 原样保留\
                 到人工处置;停等期 audits 逐字不变)"
            ));
        }
        // 臂 b:已登记容器(独立 scope)——锁定期不得被清理/注销:登记
        // 记录在场 + audits 逐字不变 + 停等原因属本 scope(生产恢复路径
        // 无 observer ⇒ observer_missing)+ 人工处置后才移除。容器实例
        // 存活只记录不断言(见 observability 口径)。
        let container_recovery = &container_arm["recovery"];
        let container_detail = container_recovery["detail"].as_str().unwrap_or_default();
        let container_arm_ok = container_arm["registered"].as_bool() == Some(true)
            && container_arm["record_present_after_restart"].as_bool() == Some(true)
            && container_recovery["reason"].as_str() == Some(B45_T24_PARK_QUIESCE_REASON)
            && container_detail.contains(B45_T24_OBSERVER_MISSING_QUIESCE_REASON)
            && container_detail.contains(&format!("resource={B45_T24_CONTAINER_RESOURCE_ID}"))
            && !container_detail.contains(&format!("resource={B45_T24_WRITER_RESOURCE_ID}"))
            && container_arm["parked"].as_bool() == Some(true)
            && container_arm["park_log_has_kept_locked"].as_bool() == Some(true)
            && audits_unchanged(container_arm)
            && container_arm["removed_by_manual_disposition"].as_bool() == Some(true)
            && container_arm["observability"].as_str() == Some(B45_T24_CONTAINER_OBSERVABILITY)
            && !container_arm["observability_note"]
                .as_str()
                .unwrap_or_default()
                .trim()
                .is_empty();
        if !container_arm_ok {
            failures.push(format!(
                "已登记容器臂不成立(独立 scope 内不得被清理/注销:登记记录在场、停等期 audits 逐字\
                 不变、停等原因为本 scope 的 {B45_T24_OBSERVER_MISSING_QUIESCE_REASON}、人工处置后\
                 才移除;生产无 engine observer ⇒ 容器实例存活只记录不断言,\
                 observability 须为 {B45_T24_CONTAINER_OBSERVABILITY} 且带非空说明)"
            ));
        }
        // 臂 c:不可观测半边——无登记容器。平台 quiesce 只消费登记面
        // (quiesce_scope 入口 list_scope),无枚举通道,容器存活性与平台
        // 行为无因果:如实标注口径(registered=false +
        // unobservable_by_platform + 非空说明 + 人工处置完成);存活字段
        // 只记录不断言。
        let unknown_container = &drill_b["unknown_container"];
        let unknown_annotated = unknown_container["registered"].as_bool() == Some(false)
            && unknown_container["observability"].as_str() == Some("unobservable_by_platform")
            && !unknown_container["observability_note"]
                .as_str()
                .unwrap_or_default()
                .trim()
                .is_empty()
            && unknown_container["removed_by_manual_disposition"].as_bool() == Some(true);
        if !unknown_annotated {
            failures.push(
                "未知容器证据口径不符(平台无枚举通道:须如实标注 unobservable_by_platform+\
                 registered=false+非空说明,人工处置完成;存活性字段只记录不断言)"
                    .to_string(),
            );
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
            failures.push(format!(
                "取消格 {label} 取消后遗留 fixture 容器:{containers:?}"
            ));
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
        failures.push(format!(
            "取消格 {label} 取消后出现新 spawn:{spawned_after:?}"
        ));
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

    /// 真实登记记录(B1 生产 adapter 形状:process + PID/starttime + 业务
    /// 归属标签;`pid_starttime_matches_proc` = 采集时与 `/proc` 逐字核对)。
    fn real_registration(pid: u64, window_id: &str, run_id: &str) -> Value {
        json!({
            "path": format!(
                "/tmp/x/provider-resources/scopes/reviewer-window-a1/{run_id}/aria-engine-1/resources/launch_1.json"
            ),
            "scope_id": "reviewer-window-a1",
            "run_id": run_id,
            "boot_identity": "aria-engine-1",
            "resource_id": "launch_1",
            "kind": "process",
            "cleanup_cursor": "confirmed",
            "labels": {
                "aria.scope_id": "reviewer-window-a1",
                "aria.run_id": run_id,
                "aria.window_id": window_id,
                "aria.worktree": "tree-key-1",
                "kind": "provider_process",
            },
            "pid": pid,
            "start_time_ticks": 123456,
            "proc_start_time_ticks": 123456,
            "pid_starttime_matches_proc": true,
            "window_id": window_id,
            "worktree_label": "tree-key-1",
        })
    }

    fn crash_base() -> Value {
        json!({
            "kill": {
                "server_dead": true,
                "worktree_tree_key": "tree-key-1",
                "windows_at_kill": [{
                    "window_id": "w1",
                    "attempt_id": "a1",
                    "run_id": "role-run-reviewer-1",
                    "state": "running",
                }],
                "scopes_at_kill": [],
                "role_runs_at_kill": [],
                "real_registrations_at_kill": [
                    real_registration(4242, "w1", "role-run-reviewer-1"),
                ],
                "mid_commit_journal": {
                    "path": "/tmp/x/coding-attempts/a1/git-operation.json",
                    "fixture_preset": true,
                    "kind": "review_request",
                    "phase": "commit_started",
                    "attempt_id": "a1",
                    "worktree_path": "/tmp/x/worktree",
                    "before_head": "sha-base",
                    "in_flight_at": "2026-10-19T00:04:00Z",
                    "observed_at": "2026-10-19T00:04:01Z",
                },
            },
            "restart": {
                "restarted_observed_at": "2026-10-19T00:10:00Z",
                "real_registrations_after_restart": [],
                "scopes_after_restart": [],
                "compensated_processes": [{
                    "pid": 4242,
                    "start_time_ticks": 123456,
                    "proc_alive": false,
                    "proc_start_time_ticks": Value::Null,
                }],
                "mid_commit_journal": {
                    "in_flight_at": "2026-10-19T00:04:00Z",
                    "phase_at_kill": "commit_started",
                    "attempt_id": "a1",
                    "before_head": "sha-base",
                    "converged": true,
                    "converged_at": "2026-10-19T00:06:00Z",
                    "phase_after": "completed",
                    "window_one_released_at": "2026-10-19T00:05:00Z",
                    "window_one_terminal_state": "new_review_required",
                    "head_at_kill": "sha-base",
                    "head_after_restart": "sha-base",
                    "fixture_preset": true,
                },
                "window_one_terminal": [
                    {
                        "attempt_id": "a1",
                        "state": "new_review_required",
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
                // A7(b) 停等双臂:各自独立 scope(同 scope 混排会被
                // resource_id 排序抢占停等触发者)。
                "writer_arm": {
                    "scope_id": B45_T24_WRITER_SCOPE_ID,
                    "run_id": "run-a2",
                    "boot_identity": "b45-t24-fixture-77",
                    "record": "/tmp/x/provider-resources/scopes/b45-t24-park-writer/run-a2/boot/resources/unconfirmed-writer.json",
                    "record_present_after_restart": true,
                    "daemon_pid": 77,
                    "daemon_alive_after_kill": true,
                    "daemon_alive_after_restart": true,
                    "park_state": {"matched": true},
                    "parked": true,
                    "park_log_has_kept_locked": true,
                    "recovery": {
                        "reason": B45_T24_PARK_QUIESCE_REASON,
                        "detail": "provider_resources_quiesce_unconfirmed_process_unknown: resource=unconfirmed-writer launch=bash -c 'sleep 1800' (b45-t24 unconfirmed writer; pid=77): 预留未确认(崩溃窗口),是否已 spawn 不可证——可识别未知 writer 停等",
                    },
                    "audits": {
                        "before": [
                            "/tmp/x/provider-resources/scopes/b45-t24-park-writer/run-a2/boot/resources/unconfirmed-writer.json sha256:aaa",
                        ],
                        "after": [
                            "/tmp/x/provider-resources/scopes/b45-t24-park-writer/run-a2/boot/resources/unconfirmed-writer.json sha256:aaa",
                        ],
                        "unchanged": true,
                    },
                    "removed_by_manual_disposition": true,
                },
                "container_arm": {
                    "scope_id": B45_T24_CONTAINER_SCOPE_ID,
                    "run_id": "run-a2",
                    "boot_identity": "b45-t24-fixture-container-77",
                    "exact_id": "ctr-registered",
                    "name": "b45-t24-park-registered",
                    "registered": true,
                    "record": "/tmp/x/provider-resources/scopes/b45-t24-park-container/run-a2/boot/resources/park-container.json",
                    "record_present_after_restart": true,
                    "park_state": {"matched": true},
                    "parked": true,
                    "park_log_has_kept_locked": true,
                    "recovery": {
                        "reason": B45_T24_PARK_QUIESCE_REASON,
                        "detail": "provider_resources_quiesce_observer_missing: resource=park-container: 无引擎可观察通道,无法证明静止",
                    },
                    "audits": {
                        "before": [
                            "/tmp/x/provider-resources/scopes/b45-t24-park-container/run-a2/boot/resources/park-container.json sha256:bbb",
                        ],
                        "after": [
                            "/tmp/x/provider-resources/scopes/b45-t24-park-container/run-a2/boot/resources/park-container.json sha256:bbb",
                        ],
                        "unchanged": true,
                    },
                    "removed_by_manual_disposition": true,
                    "observability": B45_T24_CONTAINER_OBSERVABILITY,
                    "observability_note": B45_T24_CONTAINER_OBSERVABILITY_NOTE,
                    // 存活字段只记录不断言(生产无 observer)。
                    "alive_before_restart": true,
                    "alive_after_restart": true,
                },
                "unknown_container": {
                    "exact_id": "ctr-1",
                    "name": "b45-t24-park-unknown",
                    "registered": false,
                    "observability": "unobservable_by_platform",
                    "observability_note": "平台 quiesce 只枚举登记面(provider_resources::quiesce_scope→list_scope);无登记容器对平台不可见,存活性不构成清理证据(只记录不断言)",
                    "alive_after_kill": true,
                    "alive_after_restart": true,
                    "removed_by_manual_disposition": true,
                },
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

    /// 合成 `old-writer-daemon` 前提必须红(kill 时只有合成记录、无生产
    /// 登记 → 正向臂不得以 fixture preset 冒充真实登记)。
    #[test]
    fn crash_validator_rejects_synthetic_old_writer_premise() {
        let mut evidence = crash_base();
        evidence["kill"]["real_registrations_at_kill"] = json!([]);
        evidence["kill"]["scopes_at_kill"] = json!([
            {"record": {"resource_id": "old-writer-daemon", "kind": "process"}},
        ]);
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("真实登记")),
            "{failures:?}"
        );
    }

    /// 登记无真实 PID+`/proc` starttime(未回填/身份不可证)→ 红。
    #[test]
    fn crash_validator_rejects_registration_without_proc_identity() {
        let mut evidence = crash_base();
        evidence["kill"]["real_registrations_at_kill"][0]["pid_starttime_matches_proc"] =
            json!(false);
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("PID+starttime")),
            "{failures:?}"
        );
        let mut evidence = crash_base();
        evidence["kill"]["real_registrations_at_kill"][0]["start_time_ticks"] = json!(0);
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("PID+starttime")),
            "{failures:?}"
        );
    }

    /// 登记无窗口/attempt 业务归属(只有 scope/run 不够)→ 红。
    #[test]
    fn crash_validator_rejects_registration_without_window_attribution() {
        let mut evidence = crash_base();
        evidence["kill"]["real_registrations_at_kill"][0]["window_id"] = Value::Null;
        evidence["kill"]["real_registrations_at_kill"][0]["labels"]
            .as_object_mut()
            .expect("labels")
            .remove("aria.window_id");
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("窗口/attempt 归属")),
            "{failures:?}"
        );
        // 窗口 id 不在 kill 时窗口面(伪造归属)→ 红。
        let mut evidence = crash_base();
        evidence["kill"]["real_registrations_at_kill"][0]["window_id"] = json!("w-unknown");
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("窗口/attempt 归属")),
            "{failures:?}"
        );
    }

    /// 补偿后登记 PID 仍存活(未停旧 writer)→ 红。
    #[test]
    fn crash_validator_rejects_surviving_registered_process() {
        let mut evidence = crash_base();
        evidence["restart"]["compensated_processes"][0]["proc_alive"] = json!(true);
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("PID 未消失")),
            "{failures:?}"
        );
    }

    /// 补偿后登记项未清(/proc 已消失但 durable 记录仍在)→ 红。
    #[test]
    fn crash_validator_rejects_uncleaned_registration() {
        let mut evidence = crash_base();
        let record = evidence["kill"]["real_registrations_at_kill"][0].clone();
        evidence["restart"]["real_registrations_after_restart"] = json!([record]);
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(failures.iter().any(|f| f.contains("未清")), "{failures:?}");
    }

    /// A7(a):缺 journal 在途事实(未造出“提交进行中”状态)→ 红。
    #[test]
    fn crash_validator_rejects_missing_mid_commit_journal() {
        let mut evidence = crash_base();
        evidence["kill"]
            .as_object_mut()
            .expect("kill")
            .remove("mid_commit_journal");
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("journal 在途")),
            "{failures:?}"
        );
        // kill 时相位不是 commit_started(不是“提交进行中”)→ 红。
        let mut evidence = crash_base();
        evidence["kill"]["mid_commit_journal"]["phase"] = json!("before");
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("journal 在途")),
            "{failures:?}"
        );
    }

    /// A7(a):journal 未收敛(重启后仍非终态)→ 红。
    #[test]
    fn crash_validator_rejects_unconverged_mid_commit_journal() {
        let mut evidence = crash_base();
        evidence["restart"]["mid_commit_journal"]["converged"] = json!(false);
        evidence["restart"]["mid_commit_journal"]["converged_at"] = Value::Null;
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("未收敛")),
            "{failures:?}"
        );
        let mut evidence = crash_base();
        evidence["restart"]["mid_commit_journal"]["converged"] = json!(true);
        evidence["restart"]["mid_commit_journal"]["converged_at"] = json!("2026-10-19T00:06:00Z");
        evidence["restart"]["mid_commit_journal"]["phase_after"] = json!("commit_started");
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("未收敛")),
            "{failures:?}"
        );
    }

    /// A7(a) 逆序负测:补偿(journal 收敛)先于恢复完成 → 抢跑,红。
    #[test]
    fn crash_validator_rejects_compensation_before_restore() {
        let mut evidence = crash_base();
        evidence["restart"]["mid_commit_journal"]["converged_at"] = json!("2026-10-19T00:04:30Z");
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(failures.iter().any(|f| f.contains("抢跑")), "{failures:?}");
    }

    /// A7(a) 缺失负测:恢复完成时刻缺失(先后关系不可证)→ 红。
    #[test]
    fn crash_validator_rejects_missing_restore_timestamp() {
        let mut evidence = crash_base();
        evidence["restart"]["mid_commit_journal"]["window_one_released_at"] = Value::Null;
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(failures.iter().any(|f| f.contains("时刻")), "{failures:?}");
    }

    /// A7(b):缺 parked 证据(任一臂未 park)→ 红。
    #[test]
    fn crash_validator_rejects_missing_parked_evidence() {
        let mut evidence = crash_base();
        evidence["drill_b"]["writer_arm"]["parked"] = json!(false);
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("未确认 writer 臂")),
            "{failures:?}"
        );
        let mut evidence = crash_base();
        evidence["drill_b"]["container_arm"]
            .as_object_mut()
            .expect("container_arm")
            .remove("parked");
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("已登记容器臂")),
            "{failures:?}"
        );
    }

    /// A7(b) 容器臂:登记记录在锁定期被清理/注销 → 红(可观测半边的
    /// 真实断言:平台可枚举登记面,park 而非 remove)。容器实例存活
    /// 不在此列——生产恢复路径无 observer,存活只记录(见下一用例)。
    #[test]
    fn crash_validator_rejects_cleaned_registered_container() {
        let mut evidence = crash_base();
        evidence["drill_b"]["container_arm"]["record_present_after_restart"] = json!(false);
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("已登记容器臂")),
            "{failures:?}"
        );

        // 登记面 audits 被改写(清理游标推进/记录删除)→ 红。
        let mut evidence = crash_base();
        evidence["drill_b"]["container_arm"]["audits"]["after"] =
            json!([".../park-container.json sha256:ccc"]);
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("已登记容器臂")),
            "{failures:?}"
        );

        // 停等原因不是本 scope 的 observer_missing(例如被同 scope 抢占成
        // writer 停等)→ 红:该臂没有被真正演练。
        let mut evidence = crash_base();
        evidence["drill_b"]["container_arm"]["recovery"]["detail"] = json!(
            "provider_resources_quiesce_unconfirmed_process_unknown: resource=unconfirmed-writer launch=x: 预留未确认(崩溃窗口)"
        );
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("已登记容器臂")),
            "{failures:?}"
        );
    }

    /// A7(b) 容器臂:生产恢复路径无 engine observer(call site 传 None),
    /// 容器实例存活与平台行为无因果——存活字段只记录不断言;口径标注
    /// `observability="no_production_observer"` 缺失 → 红。
    #[test]
    fn crash_validator_records_but_never_asserts_registered_container_aliveness() {
        let mut evidence = crash_base();
        evidence["drill_b"]["container_arm"]["alive_before_restart"] = json!(false);
        evidence["drill_b"]["container_arm"]["alive_after_restart"] = json!(false);
        assert!(
            validate_b45_t24_crash_evidence(&evidence).is_ok(),
            "容器存活字段不是断言面,改写不得红"
        );

        let mut evidence = crash_base();
        evidence["drill_b"]["container_arm"]
            .as_object_mut()
            .expect("container_arm")
            .remove("observability");
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("已登记容器臂")),
            "{failures:?}"
        );

        let mut evidence = crash_base();
        evidence["drill_b"]["container_arm"]["observability_note"] = json!("");
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("已登记容器臂")),
            "{failures:?}"
        );
    }

    /// F1:A7(b) writer 臂缺失(未确认 writer 记录被去掉/记录被注销)→
    /// 红——writer 停等必须由真实记录触发,不能空白通过。
    #[test]
    fn crash_validator_rejects_missing_or_deregistered_writer_arm() {
        let mut evidence = crash_base();
        evidence["drill_b"]
            .as_object_mut()
            .expect("drill_b")
            .remove("writer_arm");
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("未确认 writer 臂")),
            "{failures:?}"
        );

        let mut evidence = crash_base();
        evidence["drill_b"]["writer_arm"]["record_present_after_restart"] = json!(false);
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("未确认 writer 臂")),
            "{failures:?}"
        );
    }

    /// F1:writer 臂的停等原因必须逐字来自 writer——若 detail 是容器
    /// observer_missing(即停在错对象上),必须红。
    #[test]
    fn crash_validator_rejects_writer_park_attributed_to_container() {
        let mut evidence = crash_base();
        evidence["drill_b"]["writer_arm"]["recovery"]["detail"] = json!(
            "provider_resources_quiesce_observer_missing: resource=park-container: 无引擎可观察通道"
        );
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("未确认 writer 臂")),
            "{failures:?}"
        );

        // 两臂同 scope(修复轮回归:同 scope 混排)→ 红。
        let mut evidence = crash_base();
        evidence["drill_b"]["container_arm"]["scope_id"] = json!(B45_T24_WRITER_SCOPE_ID);
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("独立 scope")),
            "{failures:?}"
        );
    }

    /// A7(b):未知容器半边如实标注不可观测——缺标注/错标注/偷偷登记
    /// (冒充可观测)/标注无说明/人工处置未完成 → 红。
    #[test]
    fn crash_validator_requires_unobservable_annotation_on_unknown_container() {
        let mut evidence = crash_base();
        evidence["drill_b"]["unknown_container"]
            .as_object_mut()
            .expect("unknown_container")
            .remove("observability");
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures
                .iter()
                .any(|f| f.contains("unobservable_by_platform")),
            "{failures:?}"
        );

        let mut evidence = crash_base();
        evidence["drill_b"]["unknown_container"]["observability"] = json!("observable");
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("未知容器证据口径")),
            "{failures:?}"
        );

        let mut evidence = crash_base();
        evidence["drill_b"]["unknown_container"]["registered"] = json!(true);
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("未知容器证据口径")),
            "{failures:?}"
        );

        let mut evidence = crash_base();
        evidence["drill_b"]["unknown_container"]["observability_note"] = json!("");
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("未知容器证据口径")),
            "{failures:?}"
        );

        let mut evidence = crash_base();
        evidence["drill_b"]["unknown_container"]["removed_by_manual_disposition"] = json!(false);
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("未知容器证据口径")),
            "{failures:?}"
        );
    }

    /// A7(b):未知容器存活性字段只记录不断言——改写存活值不得改变判定
    ///(旧恒真 alive 断言已删除;真实现场的清理证据由已登记半边与
    /// park_audits 承担)。
    #[test]
    fn crash_validator_records_but_never_asserts_unknown_container_aliveness() {
        let mut evidence = crash_base();
        evidence["drill_b"]["unknown_container"]["alive_after_kill"] = json!(false);
        evidence["drill_b"]["unknown_container"]["alive_after_restart"] = json!(false);
        assert!(
            validate_b45_t24_crash_evidence(&evidence).is_ok(),
            "未知容器存活字段不是断言面,改写不得红"
        );
    }

    /// A7(b):停等期 audits 变化(任一臂登记面被改/被清)→ 红,且空基线
    /// 不得冒充"未变"。
    #[test]
    fn crash_validator_rejects_changed_park_audits() {
        let mut evidence = crash_base();
        evidence["drill_b"]["writer_arm"]["audits"]["after"] = json!([]);
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("未确认 writer 臂")),
            "{failures:?}"
        );
        let mut evidence = crash_base();
        evidence["drill_b"]["container_arm"]["audits"]["before"] = json!([]);
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("已登记容器臂")),
            "{failures:?}"
        );
    }

    #[test]
    fn crash_validator_rejects_compensation_after_material() {
        let mut evidence = crash_base();
        evidence["restart"]["window_one_terminal"][0]["updated_at"] = json!("2026-10-19T00:20:00Z");
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
        assert!(failures.iter().any(|f| f.contains("误杀")), "{failures:?}");
    }

    #[test]
    fn crash_validator_rejects_missing_park_arm() {
        let mut evidence = crash_base();
        evidence["drill_b"] = json!({"skipped": "attempt #2 未建"});
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(failures.iter().any(|f| f.contains("停等")), "{failures:?}");
    }

    #[test]
    fn crash_validator_rejects_killed_unknown_daemon() {
        let mut evidence = crash_base();
        evidence["drill_b"]["writer_arm"]["daemon_alive_after_restart"] = json!(false);
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("未确认 writer 臂")),
            "{failures:?}"
        );
        let mut evidence = crash_base();
        evidence["drill_b"]["writer_arm"]["daemon_alive_after_kill"] = json!(false);
        let failures = validate_b45_t24_crash_evidence(&evidence).unwrap_err();
        assert!(
            failures.iter().any(|f| f.contains("未确认 writer 臂")),
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
        assert!(failures.iter().any(|f| f.contains("缺家")), "{failures:?}");
    }

    #[test]
    fn cancel_validator_rejects_extra_provider() {
        let mut evidence = base();
        evidence["providers"] = json!(["claude", "codex", "kimi", "pi"]);
        evidence["cells"]["pi"] = cell("pi", true);
        let failures = validate_b45_t24_cancel_evidence(&evidence, &THREE).unwrap_err();
        assert!(failures.iter().any(|f| f.contains("多余")), "{failures:?}");
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
        assert!(failures.iter().any(|f| f.contains("基线")), "{failures:?}");
    }

    #[test]
    fn cancel_validator_rejects_unparsable_spawn_rows() {
        let mut evidence = base();
        evidence["cells"]["claude"]["role_run_rows_after_cancel"] =
            json!([{"role": "coder", "sequence": 0}]);
        let failures = validate_b45_t24_cancel_evidence(&evidence, &THREE).unwrap_err();
        assert!(failures.iter().any(|f| f.contains("基线")), "{failures:?}");
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
