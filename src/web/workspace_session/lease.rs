#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionRole {
    Driver,
    Observer,
}

impl ConnectionRole {
    /// Task 6 将根据 wire role 显式归一；本阶段保持 legacy driver 等价。
    pub const fn normalize(_role: Option<Self>) -> Self {
        Self::Driver
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
