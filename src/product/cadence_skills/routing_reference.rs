const AGENT_ROUTING_KERNEL_RULE_PATH: &str = "/home/michaelche/workspace/github/Cadence-skills/cadence-init/skills/rule-config/references/rules/agent-routing-kernel.md";
const OPENSPEC_SUPERPOWERS_WORKFLOW_RULE_PATH: &str = "/home/michaelche/workspace/github/Cadence-skills/cadence-init/skills/rule-config/references/rules/openspec-superpowers-workflow.md";

pub(crate) fn direct_cadence_routing_rules_reference() -> &'static str {
    const REFERENCE: &str = concat!(
        "[cadence_original_routing_rules]\n",
        "以下两份 Cadence 原始规则是唯一流程权威，必须直接读取并遵守；\n",
        "- /home/michaelche/workspace/github/Cadence-skills/cadence-init/skills/rule-config/references/rules/agent-routing-kernel.md\n",
        "- /home/michaelche/workspace/github/Cadence-skills/cadence-init/skills/rule-config/references/rules/openspec-superpowers-workflow.md\n",
        "不得以本 prompt 摘要、内部状态机或伪造 Skill 记录替代原规则。规则不可用时停止并报告；不得依赖外部编排系统、Hook、插件或阅读状态机。\n",
    );

    debug_assert!(REFERENCE.contains(AGENT_ROUTING_KERNEL_RULE_PATH));
    debug_assert!(REFERENCE.contains(OPENSPEC_SUPERPOWERS_WORKFLOW_RULE_PATH));

    REFERENCE
}

#[cfg(test)]
mod tests {
    use super::direct_cadence_routing_rules_reference;

    #[test]
    fn direct_reference_points_to_the_two_authoritative_rules_only() {
        let prompt = direct_cadence_routing_rules_reference();

        assert_eq!(
            prompt,
            concat!(
                "[cadence_original_routing_rules]\n",
                "以下两份 Cadence 原始规则是唯一流程权威，必须直接读取并遵守；\n",
                "- /home/michaelche/workspace/github/Cadence-skills/cadence-init/skills/rule-config/references/rules/agent-routing-kernel.md\n",
                "- /home/michaelche/workspace/github/Cadence-skills/cadence-init/skills/rule-config/references/rules/openspec-superpowers-workflow.md\n",
                "不得以本 prompt 摘要、内部状态机或伪造 Skill 记录替代原规则。规则不可用时停止并报告；不得依赖外部编排系统、Hook、插件或阅读状态机。\n",
            )
        );
        assert!(prompt.contains("agent-routing-kernel.md"));
        assert!(prompt.contains("openspec-superpowers-workflow.md"));
        assert!(prompt.contains("/home/michaelche/workspace/github/Cadence-skills/"));
        assert!(prompt.contains("直接读取并遵守"));
        assert!(prompt.contains("停止并报告"));
        assert!(!prompt.contains("cadence-workflow"));
        assert!(!prompt.contains("Cadence Workflow"));
        assert!(!prompt.contains("cadence workflow"));
        assert!(!prompt.contains("CADENCE-WORKFLOW"));
        assert!(!prompt.contains("# OpenSpec 与 Superpowers 协作规则"));
        assert!(!prompt.contains("WorkflowDisciplineSpec"));
        assert!(!prompt.contains("state_machine"));
        assert!(!prompt.contains('\\'));
    }
}
