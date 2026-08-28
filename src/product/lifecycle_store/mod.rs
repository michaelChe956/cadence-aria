use crate::product::app_paths::ProductAppPaths;

pub mod inputs;
pub mod paths;
pub mod plan;
pub mod review;
pub mod spec;
pub mod utils;
pub mod verification;
pub mod work_item;
pub mod workspace;
pub mod worktree;

#[cfg(test)]
mod tests;

pub use inputs::*;
pub use spec::{ConfirmAggregateGateError, ConfirmGateViolation};
pub(crate) use utils::*;
pub use workspace::{
    CompileReservationError, PolicyRoutePersist, single_candidate_approval_attempt_id,
    single_candidate_compile_id,
};

#[derive(Debug, Clone)]
pub struct LifecycleStore {
    paths: ProductAppPaths,
}

impl LifecycleStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    pub fn app_paths(&self) -> ProductAppPaths {
        self.paths.clone()
    }
}
