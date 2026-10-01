/// Aggregate initialization coordinator dependencies that can be injected by
/// tests or built once from the web app state.
#[derive(Clone)]
pub struct AggregateInitializationDependencies {
    pub(crate) coordinator: Arc<AggregateInitializationCoordinator>,
    pub(crate) runs: InitializationRunRegistry,
    pub(crate) index: Arc<AggregateIndexOperation>,
    /// Task 1.3（REQ-REG-14）：Codex/Kimi trust 硬前置门。registry 的
    /// durable facts 按 (project_id, lc_id) 调用点 scope，因此 state 装配
    /// 期即可冻结同一实例；`None` 表示该依赖图未装配 trust 门（测试默认，
    /// 生产由 `production` 注入、handler 接线由 Task 1.8 完成）。
    pub(crate) trust: Option<Arc<dyn crate::product::logical_codebase::ProviderTrustPrecondition>>,
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
            trust: None,
        }
    }

    /// Task 1.3（REQ-REG-14）：注入 Codex/Kimi trust 前置门（生产装配见
    /// production_dependencies.inc.rs；handler 消费由 Task 1.8 接线）。
    pub fn with_trust(
        mut self,
        trust: Arc<dyn crate::product::logical_codebase::ProviderTrustPrecondition>,
    ) -> Self {
        self.trust = Some(trust);
        self
    }

    pub fn trust(
        &self,
    ) -> Option<&Arc<dyn crate::product::logical_codebase::ProviderTrustPrecondition>> {
        self.trust.as_ref()
    }

    /// Derives a per-LC view of these dependencies: the same skills/preflight/
    /// provider/clock components and shared run registry, but the coordinator
    /// and index operation are re-scoped to one logical codebase subtree.
    /// trust 前置门原样透传——registry 按 (project_id, lc_id) 调用点 scope。
    pub fn for_lc(&self, lc_id: impl Into<String>) -> Self {
        let lc_id = lc_id.into();
        let derived = Self::with_index(
            Arc::new(self.coordinator.for_lc(lc_id.clone())),
            self.runs.clone(),
            Arc::new(self.index.for_lc(lc_id)),
        );
        match self.trust.clone() {
            Some(trust) => derived.with_trust(trust),
            None => derived,
        }
    }

    #[allow(dead_code)]
    pub fn coordinator(&self) -> &AggregateInitializationCoordinator {
        &self.coordinator
    }

    pub fn index(&self) -> &AggregateIndexOperation {
        &self.index
    }

    /// C4 Task 6：bootstrap action 的 member-index run 活跃探针数据源。
    pub fn runs(&self) -> &InitializationRunRegistry {
        &self.runs
    }
}
