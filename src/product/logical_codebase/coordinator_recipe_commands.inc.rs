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
    /// 命令逐字前缀+scope 纪律线追加+无人值守指令追加(不经 gateway 即可直接测试)。
    ///
    /// 无人值守指令：聚合初始化的 provider turn 同样**没有应答通道**，provider
    /// 一旦发问/请求授权，本次 turn 直接判失败（真机：Pi 在 pre_check 里发问
    /// ⇒ `aggregate_pre_check_failed`）。指令只约束**交互行为**（不提问、不
    /// 等回答），不改变 skill 的业务规则：需要用户授权的破坏性分支仍按 skill
    /// 规定停止并记录。
    fn recipe_prompt(
        step: AggregateInitializationStepKind,
        aggregate_root: &std::path::Path,
        provider: crate::product::models::ProviderName,
    ) -> String {
        format!(
            "{}{}{}",
            Self::recipe_commands(step, provider).join("\n"),
            Self::scope_discipline_section(aggregate_root),
            Self::unattended_section()
        )
    }

    /// 无人值守指令段（聚合初始化与知识库操作共用同一口径）。
    fn unattended_section() -> &'static str {
        "\n\n（无人值守执行：本次操作没有交互通道。不要向用户提问，也不要请求授权\
         或等待回答；遇到需要用户决策的分支，按 skill 规定的非破坏性默认路径继续\
         （无法安全继续时停止并在产物中写明），不要停在等待回答上。）\n"
    }
}
