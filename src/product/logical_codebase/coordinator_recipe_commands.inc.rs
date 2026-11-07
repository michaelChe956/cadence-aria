// root recipe 的 provider turn 命令生成（按 provider 的 skill 调用语法）。
//
// 纯搬移：原内联在 `coordinator_provider_turn.inc.rs` 的 impl 块里，为满足
// `tests/it_core/large_file_guard.rs` 的行数门禁（该文件的登记上限不可增长）
// 而拆出——方法体逐字不变，只换承载文件。

impl GatewayBackedAggregateProviderTurnDriver {
    /// Task 1.4（REQ-BOOT-03）：五步 root recipe 的三个 provider turn 到四条
    /// 无中断命令的固定映射。命令文本唯一来源是
    /// `RepositoryInitializationStepKind::command()`（Task 1.1 隔离锁显式豁免
    /// 该消费）：`PreCheck`=命令 1，`RuleAndMcpConfig`=命令 2+3（同一 Claude
    /// turn 内顺序执行），`OpenspecAndExamples`=命令 4。确定性 step 不映射任何
    /// 命令（空表=fail-closed，调用方拒绝该 turn）。
    fn recipe_commands(
        step: AggregateInitializationStepKind,
        provider: crate::product::models::ProviderName,
    ) -> Vec<String> {
        use crate::product::repository_store::RepositoryInitializationStepKind as RepoStep;
        let steps: &[RepoStep] = match step {
            AggregateInitializationStepKind::PreCheck => &[RepoStep::PreCheck],
            AggregateInitializationStepKind::RuleAndMcpConfig => {
                &[RepoStep::RuleConfig, RepoStep::McpConfiguration]
            }
            AggregateInitializationStepKind::OpenspecAndExamples => {
                &[RepoStep::ProjectRulesExamples]
            }
            _ => &[],
        };
        steps
            .iter()
            .filter_map(|kind| kind.command_for(provider.clone()))
            .collect()
    }

    /// 本次 turn 的 provider 名（由冻结的 `ProviderRef` 派生；四家显式映射、
    /// Fake fail-closed）。
    fn provider_name(&self) -> crate::product::models::ProviderName {
        crate::product::logical_codebase::provider_capability_revalidate::provider_name_for_type(
            self.provider.provider_type,
        )
    }

    /// OracleAggInitScope：root recipe turn prompt 拼接纯函数——recipe
    /// 命令逐字前缀+scope 纪律线追加(不经 gateway 即可直接测试)。
    fn recipe_prompt(
        step: AggregateInitializationStepKind,
        aggregate_root: &std::path::Path,
        provider: crate::product::models::ProviderName,
    ) -> String {
        format!(
            "{}{}",
            Self::recipe_commands(step, provider).join("\n"),
            Self::scope_discipline_section(aggregate_root)
        )
    }
}
