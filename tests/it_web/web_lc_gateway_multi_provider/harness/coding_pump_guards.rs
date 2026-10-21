//! coding 泵到点归因与 blocked 门有界守卫(kimi r2 现场,诊断
//! diag-kimi-coding-cell §3 R3)。——大文件守卫:harness 已按祖父条款
//! 登记 14966 行上限,到点收口与门守卫的决策表/留痕整体落本文件,
//! harness 只留薄接线。
//!
//! 现场事实(产品/评审行为全程正确,红因在计划不可满足 + 测试归因):
//! - 到点分支只 push `matrix_stage_timeout`、不落 run_failure → 格以
//!   confirmed 进产品确认门,被改写成「缺完成产物(产品确认门未通过)」,
//!   真实原因(3600s 未达终门)被这个标签掩盖。workspace 泵 r21 C1 已修
//!   同族盲区,coding 泵这一分支是漏网处。
//! - blocked 门守卫按 gate_id 记账,而产品每轮发新 gate_id(现场
//!   coding_blocked_gate_0001..0012,12 个门 12 个 id),守卫永不触发;
//!   `reviewer_rework_limit_reached` 恒答 send_to_coder,确定性死局
//!   空转满 3600s。gate 帧只带 attempt/stage/role,不带 work item id
//!   (`CreateBlockedGateInput`),故同因判定维度取 reason_code。

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{DriveOutcome, LiveWs, StageObservation, fp};

/// 返修上限门的 reason_code(产品 `coding_workspace_engine/rework.rs`
/// 冻结值)。
const REWORK_LIMIT_REASON: &str = "reviewer_rework_limit_reached";

/// 阶段超时未达终门(现场 3600s)的格失败原因——纯函数便于钉住文案:
/// 断言侧据此按「阶段超时」归因,不再被产品确认门改写成「缺完成产物」。
pub(super) fn coding_pump_timeout_failure(timeout: Duration) -> String {
    format!("coding 阶段超时未达终门({timeout:?};疑似返修循环无进展)")
}

/// coding 泵到点收口(与 workspace 泵 r21 C1 同名分支逐字对齐):阶段
/// 超时=驱动未达终态/终门,必须落格失败原因 + 足迹,不得让格以
/// confirmed 建格后被产品确认门覆盖成伪归因。
pub(super) fn coding_pump_stage_timeout(
    observation: &mut StageObservation,
    outcome: DriveOutcome,
    timeout: Duration,
    started: Instant,
    events_seen: u64,
) -> DriveOutcome {
    observation.push_event(json!({"type": "matrix_stage_timeout"}));
    observation.run_failure = Some(coding_pump_timeout_failure(timeout));
    fp(
        "coding_pump_stage_timeout",
        format_args!("elapsed={:?} events={events_seen}", started.elapsed()),
    );
    outcome
}

/// blocked 门有界守卫状态(跨帧保持)。
#[derive(Default)]
pub(super) struct BlockedGateGuardState {
    by_gate: BTreeMap<String, u32>,
    by_reason: BTreeMap<String, u32>,
}

/// 守卫判定结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum BlockedGateStop {
    /// 同 gate_id 第 2 次出现:重试动作救不回,直接收口(不答门)。
    Stop(String),
    /// 同因第 2 次出现(现场返修上限门):先按门语义答 abort 终止
    /// attempt,再收口——reviewer 已明说「本工作项边界内不可满足」,自动
    /// 驱动不得靠 send_to_coder 硬续。
    AbortAndStop(String),
}

/// blocked 门有界判定(纯函数,可单测)。
///
/// 第一维=同 gate_id(原有语义:retry 后同门重开);第二维=同
/// reason_code 下的返修上限门——产品每轮新建 gate_id,单靠第一维永远
/// 到不了 2(现场 12 门 12 id),第二维把确定性死局压到第 2 次(现场
/// 3600s → 约 24min,单测毫秒级)。三态门/配置缺失类的 triage 重试
/// 不进第二维,保持原有「按门语义应答」路径。
pub(super) fn blocked_gate_stop(
    state: &mut BlockedGateGuardState,
    gate_id: &str,
    reason_code: &str,
) -> Option<BlockedGateStop> {
    if count(&mut state.by_gate, gate_id) >= 2 {
        return Some(BlockedGateStop::Stop(format!(
            "coding blocked 门重试无效(gate={gate_id} reason={reason_code});真人工恢复门,不空转"
        )));
    }
    let reason_count = count(&mut state.by_reason, reason_code);
    (reason_code == REWORK_LIMIT_REASON && reason_count >= 2).then(|| {
        BlockedGateStop::AbortAndStop(format!(
            "coding 返修上限门重复出现(reason={reason_code} 第 {reason_count} 次,review 前沿未变);\
人工门不可自动推进,答 abort 收口"
        ))
    })
}

fn count(map: &mut BTreeMap<String, u32>, key: &str) -> u32 {
    let seen = map.entry(key.to_string()).or_default();
    *seen += 1;
    *seen
}

/// blocked 门处置(harness 泵调用):判定 + 足迹 + 必要时答 abort。
/// 返回 Some(格失败原因)=泵收口;None=照常走门应答。
pub(super) async fn blocked_gate_guard(
    state: &mut BlockedGateGuardState,
    ws: &mut LiveWs,
    frame: &Value,
) -> Option<String> {
    let gate_id = frame
        .pointer("/gate/id")
        .or_else(|| frame.pointer("/gate/gate_id"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let reason_code = frame
        .pointer("/gate/reason_code")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match blocked_gate_stop(state, gate_id, reason_code)? {
        BlockedGateStop::Stop(reason) => {
            fp(
                "coding_pump_blocked_unrecoverable",
                format_args!("gate={gate_id} reason={reason_code}"),
            );
            Some(reason)
        }
        BlockedGateStop::AbortAndStop(reason) => {
            fp(
                "coding_pump_rework_limit_abort",
                format_args!("gate={gate_id} reason={reason_code}"),
            );
            let _ = ws
                .send_json(&json!({
                    "type": "gate_response",
                    "gate_id": gate_id,
                    "action_id": abort_action_id(frame),
                    "extra_context": null
                }))
                .await;
            Some(reason)
        }
    }
}

/// 门上的 abort 动作 id(产品冻结动作名;缺失时按同名兜底)。
fn abort_action_id(frame: &Value) -> String {
    frame
        .pointer("/gate/available_actions")
        .and_then(Value::as_array)
        .and_then(|actions| {
            actions.iter().find(|action| {
                action
                    .get("action_id")
                    .or_else(|| action.get("id"))
                    .and_then(Value::as_str)
                    == Some("abort")
            })
        })
        .and_then(|action| action.get("action_id").or_else(|| action.get("id")))
        .and_then(Value::as_str)
        .unwrap_or("abort")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate_frame(gate_id: &str, reason_code: &str) -> Value {
        json!({
            "type": "coding_gate_required",
            "gate": {
                "gate_id": gate_id,
                "kind": "blocked",
                "reason_code": reason_code,
                "available_actions": [
                    {"action_id": "provide_context"},
                    {"action_id": "send_to_coder"},
                    {"action_id": "abort"},
                ],
            },
        })
    }

    /// kimi r2 现场 13:41:31:泵到点必须落「阶段超时未达终门」失败原因 +
    /// 足迹,而不是无声返回让格以 confirmed 进产品确认门。
    #[test]
    fn coding_timeout_failure_names_stage_timeout_not_missing_artifact() {
        let reason = coding_pump_timeout_failure(Duration::from_secs(3600));
        assert_eq!(reason, "coding 阶段超时未达终门(3600s;疑似返修循环无进展)");
        assert!(!reason.contains("缺完成产物"));
    }

    /// 原有语义不变:同一 gate_id 第 2 次出现 = retry 救不回,直接收口。
    #[test]
    fn same_gate_id_twice_stops() {
        let mut state = BlockedGateGuardState::default();
        assert!(blocked_gate_stop(&mut state, "coding_blocked_gate_0001", "x").is_none());
        let stop = blocked_gate_stop(&mut state, "coding_blocked_gate_0001", "x")
            .expect("同门第 2 次必须收口");
        assert!(matches!(stop, BlockedGateStop::Stop(_)));
    }

    /// 现场主因:返修上限门每轮新 gate_id(0003/0004/0006/…),同因第 2
    /// 次即答 abort 收口——把 60min 死局压到分钟级。
    #[test]
    fn rework_limit_reopens_with_new_gate_id_aborts_on_second() {
        let mut state = BlockedGateGuardState::default();
        let first = gate_frame("coding_blocked_gate_0003", REWORK_LIMIT_REASON);
        assert!(
            blocked_gate_stop(&mut state, "coding_blocked_gate_0003", REWORK_LIMIT_REASON)
                .is_none(),
            "首次返修上限门仍按门语义应答(send_to_coder)"
        );
        assert_eq!(abort_action_id(&first), "abort");
        let stop = blocked_gate_stop(&mut state, "coding_blocked_gate_0004", REWORK_LIMIT_REASON)
            .expect("同因第 2 门必须收口");
        match stop {
            BlockedGateStop::AbortAndStop(reason) => {
                assert!(
                    reason.contains("review 前沿未变"),
                    "原因须说明死局:{reason}"
                );
            }
            other => panic!("返修上限门第 2 次必须 AbortAndStop,实际:{other:?}"),
        }
    }

    /// triage 门(现场 6 次 code_review_output_human_triage)不属于同因
    /// 死局判定——review 前沿仍在推进,必须继续按门语义应答。
    #[test]
    fn triage_gates_are_not_treated_as_rework_deadlock() {
        let mut state = BlockedGateGuardState::default();
        for gate_id in [
            "coding_blocked_gate_0001",
            "coding_blocked_gate_0002",
            "coding_blocked_gate_0005",
        ] {
            assert!(
                blocked_gate_stop(&mut state, gate_id, "code_review_output_human_triage").is_none(),
                "triage 门({gate_id})不得被同因守卫拦截"
            );
        }
    }

    /// abort 动作缺失时兜底冻结动作名,不因门形状漂移丢掉收口动作。
    #[test]
    fn abort_action_falls_back_to_frozen_name() {
        let frame = json!({"gate": {"available_actions": [{"action_id": "send_to_coder"}]}});
        assert_eq!(abort_action_id(&frame), "abort");
    }
}
