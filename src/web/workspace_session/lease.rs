use crate::web::workspace_ws_types::HelloRole;

use crate::web::workspace_ws_types::WsInMessage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionRole {
    Driver,
    Observer,
}

impl ConnectionRole {
    /// 缺席 role 的唯一归一目标，保持旧客户端为 driver 的既有语义。
    const LEGACY_DEFAULT: Self = Self::Driver;

    /// 缺席 role 在协议入口归一为 legacy driver，后续仲裁仅消费内部角色。
    pub const fn normalize(role: Option<HelloRole>) -> Self {
        match role {
            None => Self::LEGACY_DEFAULT,
            Some(HelloRole::Driver) => Self::Driver,
            Some(HelloRole::Observer) => Self::Observer,
        }
    }
}

impl ConnectionRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Driver => "driver",
            Self::Observer => "observer",
        }
    }
}

/// 协议读白名单：显式 observer 只允许完成握手或保持连接；其余消息全部视为写，
/// 以便新增消息类型默认落入拒绝面。
pub const fn is_write_message(message: &WsInMessage) -> bool {
    !matches!(message, WsInMessage::Hello { .. } | WsInMessage::Ping)
}

/// 会话级单活性 driver lease。它绑定于 WebSocket attachment，不设超时定时器：
/// lease 的唯一过期条件是当前 holder 的连接关闭。
#[derive(Debug, Clone, Default)]
pub struct LeaseState {
    pub holder: Option<String>,
    pub epoch: u64,
    pub last_holder: Option<String>,
}

impl LeaseState {
    /// Driver 在 Hello 绑定时显式获取 lease；已有另一 holder 时立即接管并推进 epoch。
    pub fn acquire(&mut self, connection_id: &str) {
        if self.holder.as_deref() != Some(connection_id) {
            self.epoch += 1;
            self.holder = Some(connection_id.to_string());
        }
    }

    /// REQ-DLS-01 写时自愈：仅当租约悬空（holder=None）时授予该连接。调用方必须在
    /// manager 状态锁内使用，使「判空 + 授予 + attachment epoch 刷新」成为同一
    /// 原子临界区；返回是否实际发生授予（幂等：已持有者不重复推进）。
    pub fn acquire_if_vacant(&mut self, connection_id: &str) -> bool {
        if self.holder.is_some() {
            return false;
        }
        self.acquire(connection_id);
        true
    }

    /// 仅 holder 的连接关闭才撤销 lease；运行的生命周期完全不受影响。
    pub fn revoke_if_holder(&mut self, connection_id: &str) -> bool {
        if self.holder.as_deref() != Some(connection_id) {
            return false;
        }
        self.last_holder = self.holder.take();
        true
    }

    pub const fn matches_epoch(&self, epoch: u64) -> bool {
        self.epoch == epoch
    }
}
