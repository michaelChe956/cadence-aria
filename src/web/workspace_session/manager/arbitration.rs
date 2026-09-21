//! 连接角色与写面 lease 仲裁。

use tokio::sync::mpsc;

use super::WorkspaceSessionManager;
use crate::web::workspace_session::{ConnectionRole, is_write_message};
use crate::web::workspace_ws_handler::OutboundControl;
use crate::web::workspace_ws_types::WsInMessage;

impl WorkspaceSessionManager {
    pub(crate) fn lease_epoch_for_connection(&self, connection_id: &str) -> Option<u64> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state
            .attachments
            .get(connection_id)
            .or_else(|| state.pending_attachments.get(connection_id))
            .map(|attachment| attachment.lease_epoch)
    }

    /// Hello 入口完成 wire role 的一次归一。pending 期间的连接也必须先归一，
    /// 随后 cursor 分支会在同一状态锁内将其转为直播 attachment。
    pub(crate) fn bind_role(
        &self,
        outbound_tx: &mpsc::Sender<OutboundControl>,
        role: ConnectionRole,
        after_event_seq: Option<u64>,
    ) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let connection_id = state
            .pending_attachments
            .iter()
            .find_map(|(connection_id, attachment)| {
                attachment
                    .outbound_tx
                    .same_channel(outbound_tx)
                    .then(|| connection_id.clone())
            })
            .or_else(|| {
                state
                    .attachments
                    .iter()
                    .find_map(|(connection_id, attachment)| {
                        attachment
                            .outbound_tx
                            .same_channel(outbound_tx)
                            .then(|| connection_id.clone())
                    })
            });
        let Some(connection_id) = connection_id else {
            return;
        };
        let provisional_lease =
            if let Some(attachment) = state.pending_attachments.get_mut(&connection_id) {
                attachment.role = role;
                attachment.after_event_seq = after_event_seq;
                attachment.provisional_lease.take()
            } else if let Some(attachment) = state.attachments.get_mut(&connection_id) {
                attachment.role = role;
                attachment.after_event_seq = after_event_seq;
                attachment.provisional_lease.take()
            } else {
                return;
            };
        if role == ConnectionRole::Observer {
            if let Some(rollback) = provisional_lease
                && state.lease.holder.as_deref() == Some(connection_id.as_str())
            {
                state.lease = rollback;
            }
            return;
        }
        state.lease.acquire(&connection_id);
        let lease_epoch = state.lease.epoch;
        if let Some(attachment) = state.pending_attachments.get_mut(&connection_id) {
            attachment.lease_epoch = lease_epoch;
        } else if let Some(attachment) = state.attachments.get_mut(&connection_id) {
            attachment.lease_epoch = lease_epoch;
        }
    }

    pub(crate) fn connection_role(&self, connection_id: &str) -> ConnectionRole {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state
            .attachments
            .get(connection_id)
            .or_else(|| state.pending_attachments.get(connection_id))
            .map(|attachment| attachment.role)
            .unwrap_or(ConnectionRole::Driver)
    }

    /// 读循环在取得 engine 锁之前调用。无 role 已在 Hello 入口归一为 Driver；任何
    /// 非 holder 的 Driver 写面都必须由 lease 以 STALE_DRIVER_LEASE 拒绝。
    pub(crate) fn arbitrate(
        &self,
        connection_id: &str,
        message: &WsInMessage,
    ) -> Result<(), ConnectionRole> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (role, lease_epoch) = state
            .attachments
            .get(connection_id)
            .or_else(|| state.pending_attachments.get(connection_id))
            .map(|attachment| (attachment.role, attachment.lease_epoch))
            // attachment 只会在 socket 收尾时摘除；若发生内部竞态，写面保守拒绝。
            .unwrap_or((ConnectionRole::Observer, 0));
        if !is_write_message(message) {
            return Ok(());
        }
        if role == ConnectionRole::Observer {
            return Err(ConnectionRole::Observer);
        }
        if state.lease.holder.as_deref() != Some(connection_id)
            || !state.lease.matches_epoch(lease_epoch)
        {
            return Err(ConnectionRole::Driver);
        }
        Ok(())
    }
}
