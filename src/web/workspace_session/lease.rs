use crate::web::workspace_ws_types::HelloRole;

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

#[derive(Debug, Clone, Default)]
pub struct LeaseState {
    pub holder: Option<String>,
    pub epoch: u64,
}

impl LeaseState {
    pub const fn matches_epoch(&self, epoch: u64) -> bool {
        self.epoch == epoch
    }
}
