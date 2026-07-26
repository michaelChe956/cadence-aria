pub(crate) fn direct_cadence_routing_rules_reference() -> &'static str {
    const REFERENCE: &str = concat!(
        "[cadence_project_rules]\n",
        "当前目标仓库根目录的 AGENTS.md 与 CLAUDE.md 是本任务的流程规则依据，必须直接读取并遵守其中适用规则。\n",
        "[cadence_rule_read_gate]\n",
        "开始任务前必须使用当前 Provider 可用的原生文件读取工具完整读取 AGENTS.md 和 CLAUDE.md；不得以本 prompt 摘要、内部状态机、外部规则或声称已读替代工具读取。\n",
        "任一文件或工具不可用时，只报告阻塞；不得继续输出候选 artifact、代码、审查结论或 JSON。\n",
    );

    REFERENCE
}

#[cfg(test)]
mod tests {
    use super::direct_cadence_routing_rules_reference;

    #[test]
    fn direct_reference_requires_current_project_rule_files_and_fails_closed() {
        let prompt = direct_cadence_routing_rules_reference();

        assert_eq!(
            prompt,
            concat!(
                "[cadence_project_rules]\n",
                "当前目标仓库根目录的 AGENTS.md 与 CLAUDE.md 是本任务的流程规则依据，必须直接读取并遵守其中适用规则。\n",
                "[cadence_rule_read_gate]\n",
                "开始任务前必须使用当前 Provider 可用的原生文件读取工具完整读取 AGENTS.md 和 CLAUDE.md；不得以本 prompt 摘要、内部状态机、外部规则或声称已读替代工具读取。\n",
                "任一文件或工具不可用时，只报告阻塞；不得继续输出候选 artifact、代码、审查结论或 JSON。\n",
            )
        );
    }

    #[test]
    fn direct_reference_excludes_external_and_knowledge_base_constraints() {
        let prompt = direct_cadence_routing_rules_reference();

        for forbidden in [
            ["Cadence-", "skills/"].concat(),
            "KnowledgeBase".to_owned(),
            "唯一流程权威".to_owned(),
        ] {
            assert!(
                !prompt.contains(&forbidden),
                "unexpected {forbidden}: {prompt}"
            );
        }
    }
}
