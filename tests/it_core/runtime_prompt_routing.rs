use super::context_builder::{builder_input, p4_builder_input};
use cadence_aria::cross_cutting::provider_context_builder::build_provider_context;
use cadence_aria::runtime_units::prompt_template_registry::all_planning_node_ids;

#[test]
fn runtime_prompts_directly_reference_rules_and_declare_their_actual_stage() {
    for (node_id, required_stage_fragments) in [
        ("N04", &["候选澄清", "brainstorming"][..]),
        ("N05", &["Story Spec 候选方案", "brainstorming"][..]),
        (
            "N06",
            &["Spec 候选只读 advisory review", "using-superpowers"][..],
        ),
        ("N07", &["Design Spec 候选方案", "brainstorming"][..]),
        (
            "N08",
            &["Design Spec 候选只读审查", "using-superpowers"][..],
        ),
        ("N09", &["bounded Design revision", "using-superpowers"][..]),
        ("N10", &["Plan readiness 只读检查", "using-superpowers"][..]),
        (
            "N11",
            &["已确认 OpenSpec 后的候选计划", "writing-plans"][..],
        ),
        (
            "N12",
            &["已确认 Plan 后的 dispatch 候选", "using-superpowers"][..],
        ),
        (
            "N16",
            &[
                "已确认 Plan/WorkTask 范围内实施",
                "executing-plans",
                "test-driven-development",
            ][..],
        ),
        (
            "N17",
            &["测试执行与新鲜验证证据", "verification-before-completion"][..],
        ),
        ("N18", &["代码审查", "requesting-code-review"][..]),
        (
            "N19",
            &[
                "bounded rework",
                "using-superpowers → executing-plans",
                "test-driven-development",
            ][..],
        ),
        ("N20", &["ready advisory 只读复核", "using-superpowers"][..]),
        (
            "N24",
            &["集成验证与新鲜证据", "verification-before-completion"][..],
        ),
        ("N25", &["最终审查", "requesting-code-review"][..]),
        ("N26", &["sync/archive gate"][..]),
        (
            "N27",
            &["归档后的分支收尾", "finishing-a-development-branch"][..],
        ),
    ] {
        let input = if all_planning_node_ids().contains(&node_id) {
            builder_input(node_id)
        } else {
            p4_builder_input(node_id)
        };
        let prompt = build_provider_context(input)
            .expect("runtime context package")
            .adapter_input
            .prompt;

        assert!(
            prompt.contains("[cadence_project_rules]"),
            "{node_id}: {prompt}"
        );
        assert!(
            prompt.contains("AGENTS.md") && prompt.contains("CLAUDE.md"),
            "{node_id}: {prompt}"
        );
        assert!(
            !prompt.contains(&["Cadence-", "skills/"].concat()),
            "{node_id}: {prompt}"
        );
        assert!(!prompt.contains("cadence-workflow"));
        for fragment in required_stage_fragments {
            assert!(
                prompt.contains(fragment),
                "{node_id} must declare its current routing stage: {fragment}"
            );
        }
    }
}
