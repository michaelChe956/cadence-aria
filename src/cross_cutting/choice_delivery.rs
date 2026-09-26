//! P0 1.3：choice 应答两层回执（REQ-WIGA-05）。
//!
//! 语义：mpsc 入队仅推进到 `Resolving`；provider 等待者真正接收（bridge
//! `request_choice` 解出 `ChoiceDecision`、pi 对应命令等待者接收）才推进到
//! `Delivered`——mpsc accepted 不得冒充已交付。`Rejected` = 投递结构拒绝
//! （无 pending/oneshot 关闭）；`Expired` = run 结束/重启后旧命令失效
//! （Task 7/9 的 run 生命周期置位）。

use std::fmt;
use std::sync::Arc;

use tokio::sync::watch;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChoiceReplyState {
    Submitting,
    Resolving,
    Delivered,
    Rejected,
    Expired,
}

/// 提交方持有的回执句柄；可克隆（与命令一起透传），按 `Arc::ptr_eq` 判等
/// 以保持命令值比较语义。观察端（engine gate、REST 轮询）经 `subscribe()`
/// 获取 watch 接收器。
#[derive(Clone)]
pub struct ChoiceDeliverySignal(Arc<watch::Sender<ChoiceReplyState>>);

impl ChoiceDeliverySignal {
    pub fn new() -> (Self, watch::Receiver<ChoiceReplyState>) {
        let (sender, receiver) = watch::channel(ChoiceReplyState::Submitting);
        (Self(Arc::new(sender)), receiver)
    }

    /// 为后续观察者（coding gate 等待 Delivered、REST 状态查询）订阅状态。
    pub fn subscribe(&self) -> watch::Receiver<ChoiceReplyState> {
        self.0.subscribe()
    }

    /// 命令已被 mpsc 消费者领取，正在向 provider 等待者解析。
    pub fn mark_resolving(&self) {
        self.0.send_replace(ChoiceReplyState::Resolving);
    }

    /// provider 等待者真正接收（解出 ChoiceDecision）。
    pub fn deliver(&self) {
        self.0.send_replace(ChoiceReplyState::Delivered);
    }

    /// 投递结构拒绝（无 pending 等待者/oneshot 关闭/入队失败）。
    pub fn reject(&self) {
        self.0.send_replace(ChoiceReplyState::Rejected);
    }

    /// run 结束/重启后旧命令失效（Task 7/9 run 生命周期置位）。
    pub fn expire(&self) {
        self.0.send_replace(ChoiceReplyState::Expired);
    }
}

impl PartialEq for ChoiceDeliverySignal {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for ChoiceDeliverySignal {}

impl fmt::Debug for ChoiceDeliverySignal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChoiceDeliverySignal")
            .field("state", &*self.0.borrow())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choice_delivery_signal_starts_submitting_and_advances_on_each_transition() {
        let (receipt, mut status) = ChoiceDeliverySignal::new();
        assert_eq!(*status.borrow(), ChoiceReplyState::Submitting);

        receipt.mark_resolving();
        assert_eq!(*status.borrow(), ChoiceReplyState::Resolving);

        receipt.deliver();
        assert_eq!(*status.borrow(), ChoiceReplyState::Delivered);
    }

    #[test]
    fn choice_delivery_signal_reject_and_expire_are_terminal_for_their_paths() {
        let (rejected, mut rejected_status) = ChoiceDeliverySignal::new();
        rejected.reject();
        assert_eq!(*rejected_status.borrow(), ChoiceReplyState::Rejected);

        let (expired, mut expired_status) = ChoiceDeliverySignal::new();
        expired.expire();
        assert_eq!(*expired_status.borrow(), ChoiceReplyState::Expired);
    }

    #[test]
    fn choice_delivery_signal_late_subscriber_sees_latest_state_and_ptr_identity_compares() {
        let (receipt, _status) = ChoiceDeliverySignal::new();
        receipt.mark_resolving();

        let late = receipt.subscribe();
        assert_eq!(*late.borrow(), ChoiceReplyState::Resolving);

        let clone = receipt.clone();
        assert_eq!(clone, receipt);
        let (other, _other_status) = ChoiceDeliverySignal::new();
        assert_ne!(other, receipt);
    }
}
