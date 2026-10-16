mod journal;
mod lease;
mod manager;
mod registry;
mod router;
pub use lease::{ConnectionRole, LeaseState, is_write_message};
pub(crate) use manager::ChoiceReplyError;
pub(crate) use manager::lease_diagnostics_path;
// 批 2 T3：统一终止服务调用 T2 引擎侧专用取消契约（manager 为私有模块，
// 经此 crate 内出口；接线后 terminate_plan_engine_side 不再 dead_code）。
pub(crate) use manager::terminate_plan_engine_side;
pub use manager::{ActiveRun, WorkspaceSessionManager};

#[cfg(test)]
pub(crate) use manager::test_session_record;
pub use registry::WorkspaceSessionRegistry;

#[cfg(test)]
mod tests;
