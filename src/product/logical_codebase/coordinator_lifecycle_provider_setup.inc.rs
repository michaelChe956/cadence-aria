// 片（REQ-BOOT-07 / REQ-BOOT-09）：provider 选择前置与环境前置的构造/入口分片。
// 本文件通过 `include!` 进入 aggregate_initialization_coordinator.rs 的同一模块，
// 拆分只为满足 large_file_guard 的 1200 行上限；函数体、签名、可见性与注释
// 逐字保持不变（纯移动，零行为差异）。

impl AggregateInitializationCoordinator {
    /// REQ-BOOT-09：注入 codegraph 前置探针。生产装配用与索引操作同一份
    /// CLI 配置的 `CodeGraphEnvironmentProbe`；未注入时不做探测（既有测试
    /// 构造点零迁移），快照记录 `codegraph_available=false`。
    pub fn with_environment_probe(
        mut self,
        probe: Arc<dyn AggregateEnvironmentProbe>,
    ) -> Self {
        self.environment_probe = Some(probe);
        self
    }

    /// `machine_skills` 成功后校验四条命令的链接层（shared / codex /
    /// claude 三层的 `SKILL.md`）。`home` 必须与生产 skills manager 的
    /// HOME 同源；未注入时跳过校验（既有测试构造点零迁移）。
    pub fn with_skills_home(mut self, home: impl Into<PathBuf>) -> Self {
        self.skills_home = Some(home.into());
        self
    }

    /// REQ-BOOT-07：把冻结的 provider 选择结果落到 operation（候选表按
    /// owner 顺序）。既有 [`Self::begin`] / [`Self::begin_reinitialization`]
    /// 语义与签名保持不动（等价 selection = None），现有调用点零迁移。
    pub fn begin_with_selection(
        &self,
        operation_id: String,
        project_id: &str,
        input: AggregateInitializationOperationInput,
        selection: AggregateProviderSelection,
        full_recipe_requested: bool,
    ) -> Result<AggregateInitializationOperation, AggregateInitializationError> {
        self.begin_with(operation_id, project_id, input, Some(selection), full_recipe_requested)
    }

    /// 创建期「整条链不需要 provider」的操作创建入口（owner 2026-11-04
    /// 判定前移）：冻结「零 provider turn」事实，**不落候选表**——没有候选，
    /// 因为整条链一个 provider turn 都不跑（两个判定文件都在 ⇒ 四条
    /// Cadence-skills 命令没必要跑）。
    ///
    /// 语义等价于 [`Self::begin_with_selection`] 的 `selection = None` 分支，
    /// 但把「无 provider」这件事在创建期就写进 `input.provider`
    /// （`AGGREGATE_PROVIDER_NOT_REQUIRED`），使 durable 记录自证「这条链
    /// 没有 provider」。
    pub fn begin_without_provider(
        &self,
        operation_id: String,
        project_id: &str,
        input: AggregateInitializationOperationInput,
        full_recipe_requested: bool,
    ) -> Result<AggregateInitializationOperation, AggregateInitializationError> {
        self.begin_with(operation_id, project_id, input, None, full_recipe_requested)
    }
}
