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
pub(crate) use utils::*;

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
