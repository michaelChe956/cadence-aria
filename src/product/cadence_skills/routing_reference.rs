const AGENT_ROUTING_KERNEL_RULE_PATH: &str = "/home/michaelche/workspace/github/Cadence-skills/cadence-init/skills/rule-config/references/rules/agent-routing-kernel.md";
const OPENSPEC_SUPERPOWERS_WORKFLOW_RULE_PATH: &str = "/home/michaelche/workspace/github/Cadence-skills/cadence-init/skills/rule-config/references/rules/openspec-superpowers-workflow.md";

pub(crate) fn direct_cadence_routing_rules_reference() -> &'static str {
    const REFERENCE: &str = concat!(
        "[cadence_original_routing_rules]\n",
        "以下两份 Cadence 原始规则是唯一流程权威，必须直接读取并遵守；\n",
        "- /home/michaelche/workspace/github/Cadence-skills/cadence-init/skills/rule-config/references/rules/agent-routing-kernel.md\n",
        "- /home/michaelche/workspace/github/Cadence-skills/cadence-init/skills/rule-config/references/rules/openspec-superpowers-workflow.md\n",
        "不得以本 prompt 摘要、内部状态机或伪造 Skill 记录替代原规则。规则不可用时停止并报告；不得依赖外部编排系统、Hook、插件或阅读状态机。\n",
        "本项目不使用知识库：不得读取、调用或依赖任何知识库类 Skill、KnowledgeBase 的 manifest 或内容；只以当前任务直接提供的材料、仓库文件事实、项目规则和上述两份原始规则为依据。\n",
        "[cadence_rule_read_gate]\n",
        "首个用户可见文本只能是工作流路由回执；它不属于候选 artifact、审查结论或 schema payload。\n",
        "回执后必须先使用当前 Provider 可用的原生文件读取工具完整读取本阶段必调 Skill，再完整读取上列两个绝对路径；日志必须可审计到 Skill 与两条规则读取事件。\n",
        "不得凭路径、本 prompt 摘要或声称已读替代工具读取。任一 Skill、路径或工具不可用时，只报告阻塞；不得继续输出候选 artifact、审查结论或 JSON。\n",
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
                "本项目不使用知识库：不得读取、调用或依赖任何知识库类 Skill、KnowledgeBase 的 manifest 或内容；只以当前任务直接提供的材料、仓库文件事实、项目规则和上述两份原始规则为依据。\n",
                "[cadence_rule_read_gate]\n",
                "首个用户可见文本只能是工作流路由回执；它不属于候选 artifact、审查结论或 schema payload。\n",
                "回执后必须先使用当前 Provider 可用的原生文件读取工具完整读取本阶段必调 Skill，再完整读取上列两个绝对路径；日志必须可审计到 Skill 与两条规则读取事件。\n",
                "不得凭路径、本 prompt 摘要或声称已读替代工具读取。任一 Skill、路径或工具不可用时，只报告阻塞；不得继续输出候选 artifact、审查结论或 JSON。\n",
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

    #[test]
    fn direct_reference_requires_auditable_native_rule_reads_before_output() {
        let prompt = direct_cadence_routing_rules_reference();

        assert!(prompt.contains("[cadence_rule_read_gate]"));
        assert!(prompt.contains("首个用户可见文本只能是工作流路由回执"));
        assert!(prompt.contains("回执后必须先使用当前 Provider 可用的原生文件读取工具"));
        assert!(prompt.contains("Skill 与两条规则读取事件"));
        assert!(prompt.contains("不得继续输出候选 artifact、审查结论或 JSON"));
    }

    #[test]
    fn direct_reference_excludes_knowledge_base_skills_and_content() {
        let prompt = direct_cadence_routing_rules_reference();

        assert!(prompt.contains("不得读取、调用或依赖任何知识库类 Skill"));
        assert!(prompt.contains("KnowledgeBase 的 manifest 或内容"));
        assert!(prompt.contains("当前任务直接提供的材料"));
    }
}
