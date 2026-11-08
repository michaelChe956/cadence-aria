//! REQ-CG-10（gate-card-resolved-and-inflight-state，D3）：因取消 / 终止导致的
//! 人工修订失败记为**中止**而非 provider 故障。
//!
//! 复用既有中止语义，不引入第二套：节点 `TimelineNodeStatus::Aborted`
//! （REQ-WIGA-16）、turn `HumanGateTurnFailureClass::Cancelled`、取消结果三态
//! `RunAbortOutcome`（REQ-WIGA-17，本文件不触碰）。本文件只做失败分类与节点
//! 收口文案分流，放独立文件是因为 `conversational_gate.rs` 已逼近 1200 行门禁。

use crate::product::models::HumanGateTurnFailureClass;
use crate::web::workspace_ws_types::TimelineNodeStatus;

/// 修订 provider 驱动因取消返回的中止哨兵（`provider_drive/work_item_plan.rs`
/// 两处 `finish_aborted_run` 后的返回值共用，避免字面量漂移）。
pub(crate) const PROVIDER_RUN_ABORTED_MESSAGE: &str = "provider run aborted";
/// 事件通道先于取消臂关闭时驱动返回的消息（abort 杀 provider 进程的另一条竞态
/// 出口）；仅在 run 已被取消时才按取消归类。
pub(crate) const PROVIDER_EMPTY_OUTPUT_MESSAGE: &str = "provider completed without output";
/// 取消导致的修订失败：载体节点落 `Aborted` 的 summary（含 REQ-WIGA-16 要求的
/// 「已中止」表述 + 明说句）。
pub(crate) const REVISION_ABORTED_NODE_SUMMARY: &str = "运行已中止：修订被中断，你的选择未生效";
/// 真实失败：载体节点落 `Failed` 的 summary（本 change 前即有，逐字不变）。
pub(crate) const REVISION_FAILED_NODE_SUMMARY: &str = "人工修订失败，本轮反馈未采纳";
/// `human_gate_turn_failed{failure_class:"cancelled"}` 回显帧文案。
pub(crate) const REVISION_CANCELLED_FRAME_MESSAGE: &str =
    "修订被中断，你的选择未生效（运行已被取消或终止）；如需继续请重新提交反馈";

/// 修订驱动错误的分类结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RevisionDriveErrorOutcome {
    pub(crate) failure_class: HumanGateTurnFailureClass,
    /// 回显帧 `failure_class` 的 wire 标签。
    pub(crate) label: &'static str,
    pub(crate) frame_message: String,
}

/// 把修订 provider 驱动的 `Err(message)` 分类为 `Cancelled` 或 `ProviderErr`。
///
/// - 中止哨兵 ⇒ `Cancelled`（无论 `run_cancelled`：哨兵只由取消臂 / Abort 命令返回）。
/// - 空输出消息且 run 已被取消 ⇒ `Cancelled`（abort 杀进程后事件通道先关的竞态）。
/// - 其余（含 idle watchdog / 选择等待超时：它们也会 `cancel()` 同一 token，但消息
///   是各自的稳定前缀，不是 owner 终止）⇒ `ProviderErr`，消息原样透传。
pub(crate) fn revision_drive_error_outcome(
    message: &str,
    run_cancelled: bool,
) -> RevisionDriveErrorOutcome {
    let cancelled = message == PROVIDER_RUN_ABORTED_MESSAGE
        || (run_cancelled && message == PROVIDER_EMPTY_OUTPUT_MESSAGE);
    if cancelled {
        RevisionDriveErrorOutcome {
            failure_class: HumanGateTurnFailureClass::Cancelled,
            label: "cancelled",
            frame_message: REVISION_CANCELLED_FRAME_MESSAGE.to_string(),
        }
    } else {
        RevisionDriveErrorOutcome {
            failure_class: HumanGateTurnFailureClass::ProviderErr,
            label: "provider_err",
            frame_message: message.to_string(),
        }
    }
}

/// turn 终态化时修订 author 载体节点的收口：`Cancelled` ⇒ `Aborted`（不得为
/// `Failed`，REQ-WIGA-16）；其余失败类 ⇒ 既有 `Failed` + 既有文案。
pub(crate) fn revision_node_closure(
    failure_class: &HumanGateTurnFailureClass,
) -> (TimelineNodeStatus, &'static str) {
    if *failure_class == HumanGateTurnFailureClass::Cancelled {
        (TimelineNodeStatus::Aborted, REVISION_ABORTED_NODE_SUMMARY)
    } else {
        (TimelineNodeStatus::Failed, REVISION_FAILED_NODE_SUMMARY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 对话流 / 节点 summary / 回显帧共用的明说句（前端 `REVISION_INTERRUPTED_SENTENCE`
    /// 逐字相同，两侧各有断言钉住）。生产常量均为字面量（无法在 const 中拼接），
    /// 故本句只作为测试内的一致性锚点，钉住节点 summary 与回显帧文案。
    const REVISION_INTERRUPTED_SENTENCE: &str = "修订被中断，你的选择未生效";

    #[test]
    fn abort_sentinel_is_cancelled_regardless_of_token_state() {
        for run_cancelled in [true, false] {
            let outcome = revision_drive_error_outcome(PROVIDER_RUN_ABORTED_MESSAGE, run_cancelled);
            assert_eq!(outcome.failure_class, HumanGateTurnFailureClass::Cancelled);
            assert_eq!(outcome.label, "cancelled");
            assert!(outcome.frame_message.contains("你的选择未生效"));
        }
    }

    #[test]
    fn empty_output_is_cancelled_only_when_the_run_was_cancelled() {
        let cancelled = revision_drive_error_outcome(PROVIDER_EMPTY_OUTPUT_MESSAGE, true);
        assert_eq!(
            cancelled.failure_class,
            HumanGateTurnFailureClass::Cancelled
        );

        let genuine = revision_drive_error_outcome(PROVIDER_EMPTY_OUTPUT_MESSAGE, false);
        assert_eq!(
            genuine.failure_class,
            HumanGateTurnFailureClass::ProviderErr
        );
        assert_eq!(genuine.label, "provider_err");
        assert_eq!(genuine.frame_message, PROVIDER_EMPTY_OUTPUT_MESSAGE);
    }

    #[test]
    fn watchdog_and_choice_timeout_stay_provider_err_even_though_they_cancel_the_token() {
        for message in [
            "provider_idle_watchdog: work item plan provider 会话零活动楔死，运行已中止",
            "provider_choice_wait_timeout: work item plan provider 等待选择应答超时，运行已中止",
            "permission timeout: perm_1",
            "transport died",
        ] {
            let outcome = revision_drive_error_outcome(message, true);
            assert_eq!(
                outcome.failure_class,
                HumanGateTurnFailureClass::ProviderErr,
                "{message}"
            );
            assert_eq!(outcome.frame_message, message);
        }
    }

    #[test]
    fn node_closure_aborts_on_cancelled_and_fails_otherwise() {
        assert_eq!(
            revision_node_closure(&HumanGateTurnFailureClass::Cancelled),
            (TimelineNodeStatus::Aborted, REVISION_ABORTED_NODE_SUMMARY)
        );
        for class in [
            HumanGateTurnFailureClass::ProviderErr,
            HumanGateTurnFailureClass::ValidationReject,
            HumanGateTurnFailureClass::Timeout,
            HumanGateTurnFailureClass::BudgetExhausted,
        ] {
            assert_eq!(
                revision_node_closure(&class),
                (TimelineNodeStatus::Failed, REVISION_FAILED_NODE_SUMMARY)
            );
        }
        assert!(REVISION_ABORTED_NODE_SUMMARY.contains("已中止"));
        assert!(REVISION_ABORTED_NODE_SUMMARY.ends_with(REVISION_INTERRUPTED_SENTENCE));
        assert!(REVISION_CANCELLED_FRAME_MESSAGE.starts_with(REVISION_INTERRUPTED_SENTENCE));
    }
}
