use crate::product::app_paths::ProductAppPaths;

#[derive(Debug, Clone)]
pub struct WorkItemStore {
    paths: ProductAppPaths,
}

impl WorkItemStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    pub fn paths(&self) -> &ProductAppPaths {
        &self.paths
    }
}
