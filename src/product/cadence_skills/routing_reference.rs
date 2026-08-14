#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RoutingReferenceContext {
    #[default]
    Legacy,
    Logical(LogicalPolicyReference),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicalPolicyReference {
    pub policy_id: String,
    pub policy_revision: u64,
    pub policy_digest: String,
    pub authority_root: String,
}

/// 把 gateway 校验后的 `ValidatedSessionLaunchPolicy` 映射为 `RoutingReferenceContext::Logical`。
///
/// 单一归属：WorkItemPlan 流式路径（T3）与 coding 路径（T2）共用本转换，不复制逻辑。
/// envelope 是 policy 的冻结快照，`authority_root` 取 `manifest.provider_context_root`。
pub(crate) fn routing_reference_context_from_policy(
    policy: &crate::product::logical_codebase::provider_gateway::ValidatedSessionLaunchPolicy,
) -> RoutingReferenceContext {
    let envelope = policy.envelope();
    RoutingReferenceContext::Logical(LogicalPolicyReference {
        policy_id: envelope.policy_id.clone(),
        policy_revision: envelope.policy_revision,
        policy_digest: envelope.policy_digest.clone(),
        authority_root: envelope.authority_root.to_string_lossy().to_string(),
    })
}

const LEGACY_REFERENCE: &str = concat!(
    "[cadence_project_rules]\n",
    "当前目标仓库根目录的 AGENTS.md 与 CLAUDE.md 是本任务的流程规则依据，必须直接读取并遵守其中适用规则。\n",
    "[cadence_rule_read_gate]\n",
    "开始任务前必须使用当前 Provider 可用的原生文件读取工具完整读取 AGENTS.md 和 CLAUDE.md；不得以本 prompt 摘要、内部状态机、外部规则或声称已读替代工具读取。\n",
    "任一文件或工具不可用时，只报告阻塞；不得继续输出候选 artifact、代码、审查结论或 JSON。\n",
);

pub(crate) fn direct_cadence_routing_rules_reference(context: &RoutingReferenceContext) -> String {
    match context {
        RoutingReferenceContext::Legacy => LEGACY_REFERENCE.to_string(),
        RoutingReferenceContext::Logical(policy) => logical_cadence_routing_rules_reference(policy),
    }
}

fn logical_cadence_routing_rules_reference(policy: &LogicalPolicyReference) -> String {
    format!(
        concat!(
            "[cadence_project_rules]\n",
            "本任务属于逻辑代码库（logical codebase）多仓流程。规则依据是 envelope 校验过的聚合政策正文：\n",
            "authority_root: {}\n",
            "policy_id: {}\n",
            "policy_revision: {}\n",
            "policy_digest: {}\n",
            "目标仓根目录的 AGENTS.md/CLAUDE.md 若含 aria 指针块，仅用于发现上述政策 locator 与路由，不作为政策正文执行。\n",
            "[cadence_rule_read_gate]\n",
            "开始任务前必须通过原生文件读取工具加载权威根下的聚合政策正文；不得以本 prompt 摘要、内部状态机、外部规则或声称已读替代工具读取；不得在 target worktree 下猜测同名路径。\n",
            "未加载有效聚合政策时，只报告阻塞；不得继续输出候选 artifact、代码、审查结论或 JSON。\n",
        ),
        policy.authority_root, policy.policy_id, policy.policy_revision, policy.policy_digest,
    )
}

#[cfg(test)]
mod tests {
    use super::{
        LogicalPolicyReference, RoutingReferenceContext, direct_cadence_routing_rules_reference,
    };

    fn policy_fixture() -> LogicalPolicyReference {
        LogicalPolicyReference {
            policy_id: "pol_1".into(),
            policy_revision: 3,
            policy_digest: "abc123".into(),
            authority_root: "/data/aria/aggregate/policy".into(),
        }
    }

    fn logical() -> RoutingReferenceContext {
        RoutingReferenceContext::Logical(policy_fixture())
    }

    #[test]
    fn direct_reference_requires_current_project_rule_files_and_fails_closed() {
        let prompt = direct_cadence_routing_rules_reference(&RoutingReferenceContext::Legacy);

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
        let prompt = direct_cadence_routing_rules_reference(&RoutingReferenceContext::Legacy);

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

    #[test]
    fn logical_reference_declares_authority_root_and_digest() {
        let ctx = RoutingReferenceContext::Logical(LogicalPolicyReference {
            policy_id: "pol_1".into(),
            policy_revision: 3,
            policy_digest: "abc123".into(),
            authority_root: "/data/aria/aggregate/policy".into(),
        });
        let prompt = direct_cadence_routing_rules_reference(&ctx);
        assert!(prompt.contains("/data/aria/aggregate/policy"));
        assert!(prompt.contains("pol_1"));
        assert!(prompt.contains("policy_revision: 3"));
        assert!(prompt.contains("abc123"));
        assert!(prompt.contains("仅用于发现"));
        assert!(prompt.contains("不作为政策正文"));
        assert!(prompt.contains("只报告阻塞"));
    }

    #[test]
    fn logical_reference_forbids_guessing_paths_and_external_deps() {
        let prompt = direct_cadence_routing_rules_reference(&logical());
        assert!(!prompt.contains(&["Cadence-", "skills/"].concat()));
        assert!(!prompt.contains("KnowledgeBase"));
        assert!(!prompt.contains("唯一流程权威"));
        assert!(prompt.contains("不得在 target worktree 下猜测同名路径"));
    }

    #[test]
    fn legacy_reference_bytes_unchanged_after_context_parameterization() {
        let prompt = direct_cadence_routing_rules_reference(&RoutingReferenceContext::Legacy);
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
    fn routing_reference_context_defaults_to_legacy() {
        assert!(matches!(
            RoutingReferenceContext::default(),
            RoutingReferenceContext::Legacy
        ));
    }
}
