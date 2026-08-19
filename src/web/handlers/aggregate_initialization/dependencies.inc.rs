/// Aggregate initialization coordinator dependencies that can be injected by
/// tests or built once from the web app state.
#[derive(Clone)]
pub struct AggregateInitializationDependencies {
    pub(crate) coordinator: Arc<AggregateInitializationCoordinator>,
    pub(crate) runs: InitializationRunRegistry,
    pub(crate) index: Arc<AggregateIndexOperation>,
}

impl AggregateInitializationDependencies {
    pub fn new(
        coordinator: Arc<AggregateInitializationCoordinator>,
        runs: InitializationRunRegistry,
    ) -> Self {
        Self::with_index(
            coordinator,
            runs,
            Arc::new(AggregateIndexOperation::new(
                ProductAppPaths::new(std::env::temp_dir().join("aria-aggregate-index")),
                CodeGraphCli::new(Arc::new(TokioBoundedCommandRunner), "codegraph".to_string()),
                CodeGraphExcludeGenerator,
            )),
        )
    }

    pub fn with_index(
        coordinator: Arc<AggregateInitializationCoordinator>,
        runs: InitializationRunRegistry,
        index: Arc<AggregateIndexOperation>,
    ) -> Self {
        Self {
            coordinator,
            runs,
            index,
        }
    }

    /// Derives a per-LC view of these dependencies: the same skills/preflight/
    /// provider/clock components and shared run registry, but the coordinator
    /// and index operation are re-scoped to one logical codebase subtree.
    pub fn for_lc(&self, lc_id: impl Into<String>) -> Self {
        let lc_id = lc_id.into();
        Self::with_index(
            Arc::new(self.coordinator.for_lc(lc_id.clone())),
            self.runs.clone(),
            Arc::new(self.index.for_lc(lc_id)),
        )
    }

    #[allow(dead_code)]
    pub fn coordinator(&self) -> &AggregateInitializationCoordinator {
        &self.coordinator
    }

    pub fn index(&self) -> &AggregateIndexOperation {
        &self.index
    }
}
