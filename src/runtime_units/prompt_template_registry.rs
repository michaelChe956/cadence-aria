use crate::protocol::contracts::{
    NodePromptTemplateRef, PromptSection, execution_contract_for_node, workflow_discipline_for_node,
};
use crate::protocol::prompt_manifest::{PromptTemplate, required_prompt_sections};
use std::collections::BTreeMap;

pub fn all_planning_node_ids() -> Vec<&'static str> {
    vec![
        "N04", "N05", "N06", "N07", "N08", "N09", "N10", "N11", "N12",
    ]
}

pub fn all_phase1_provider_node_ids() -> Vec<&'static str> {
    let mut nodes = all_planning_node_ids();
    nodes.extend([
        "N16", "N17", "N18", "N19", "N20", "N24", "N25", "N26", "N27",
    ]);
    nodes
}

pub fn prompt_template_for_node(node_id: &str) -> Option<PromptTemplate> {
    let contract = execution_contract_for_node(node_id)?;
    let _workflow = workflow_discipline_for_node(node_id)?;
    let sections = match node_id {
        "N04" => n04_sections(),
        "N05" => n05_sections(),
        "N07" => n07_sections(),
        "N11" => n11_sections(),
        _ => generic_sections(system_delta(node_id), artifact_kind(node_id)),
    };

    Some(PromptTemplate {
        template_ref: NodePromptTemplateRef {
            template_id: contract.prompt_template_id,
            template_version: "v1".to_string(),
            system_instruction_ref: format!("system://aria/{node_id}/v1"),
            render_order: required_prompt_sections(),
            required_sections: required_prompt_sections(),
            output_schema_ref: contract.output_schema_ref,
            output_instruction_ref: format!("output://aria/{node_id}/v1"),
            failure_instruction_ref: "failure://aria/provider_failure/v1".to_string(),
        },
        sections,
    })
}

fn n04_sections() -> BTreeMap<PromptSection, String> {
    section_map(
        "[system]\n你是 Aria 的候选澄清产物生成器。Aria daemon 是唯一运行时真相源；你只能输出候选 clarification_record。",
        "[node_contract]\nnode_id={{node_id}}\nruntime_role={{runtime_role}}\nadapter_role={{adapter_role}}\nadvisory_only=false\nallowed_write_scope=[]",
        "[canonical_inputs]\n读取 intake_brief 与 effective_policy 摘要：\n{{canonical_input_summary}}\n\n完整 canonical_inputs（JSON）：\n{{canonical_inputs_json}}",
        "[projection_summary]\n本节点通常没有 projection 输入。如存在历史 projection，只能作为上下文引用：\n{{projection_summary}}",
        "[constraint_summary]\n必须遵守 proposal_constraints：\n{{constraint_summary}}",
        "[workflow_discipline]\n本节点保留 using-superpowers 与 brainstorming 的纪律约束，但这是 Aria 非交互运行：不得向用户提问或等待确认，不得启动交互式 Todo/确认流程。未决问题必须写入候选产物的 open_questions 或待确认项。{{workflow_discipline_summary}}",
        "[output_schema]\n{{output_schema_summary}}\n最终 stdout 必须包含 <ARIA_STRUCTURED_OUTPUT> JSON block，且 artifact_kind 必须等于 clarification_record。\n</ARIA_STRUCTURED_OUTPUT> 只能作为结束标签出现。",
        "[completion_or_failure]\n不要推进节点状态。不要写文件。只输出候选结果。",
    )
}

fn n05_sections() -> BTreeMap<PromptSection, String> {
    section_map(
        "[system]\n你是 Aria 的候选 spec 生成器。daemon 负责校验、落盘、编译 SpecProjection。",
        "[node_contract]\nnode_id={{node_id}}\nruntime_role={{runtime_role}}\nadapter_role={{adapter_role}}\nadvisory_only={{advisory_only}}\nallowed_write_scope={{allowed_write_scope}}\ntimeout_sec={{timeout_sec}}\nmax_retries={{max_retries}}",
        "[canonical_inputs]\n输入 artifact：\n{{canonical_input_summary}}\n\n完整 canonical_inputs（JSON）：\n{{canonical_inputs_json}}",
        "[projection_summary]\n如已有 projection，只能作为一致性参考：\n{{projection_summary}}",
        "[constraint_summary]\n必须覆盖 proposal_constraints，并声明稳定 requirement IDs：\n{{constraint_summary}}",
        "[workflow_discipline]\n本节点保留 using-superpowers 与 brainstorming 的纪律约束，但这是 Aria 非交互运行：不得向用户提问或等待确认，不得启动交互式 Todo/确认流程。未决问题必须写入候选产物的 open_questions 或待确认项。{{workflow_discipline_summary}}",
        "[output_schema]\n{{output_schema_summary}}\n最终 stdout 必须包含 <ARIA_STRUCTURED_OUTPUT> JSON block，且 artifact_kind 必须等于 spec。JSON 内的 markdown 字段必须包含范围、用户故事、功能需求、成功标准、待确认项和非功能需求。\n</ARIA_STRUCTURED_OUTPUT> 只能作为结束标签出现。",
        "[completion_or_failure]\n不要直接修改 OpenSpec。不要直接生成 projection。daemon 会做结构化落盘。",
    )
}

fn n07_sections() -> BTreeMap<PromptSection, String> {
    section_map(
        "[system]\n你是 Aria 的候选 design 生成器。你只生成候选设计文档，daemon 负责 canonical 校验、落盘与 DesignProjection 编译。",
        "[node_contract]\nnode_id={{node_id}}\nruntime_role={{runtime_role}}\nadapter_role={{adapter_role}}\nadvisory_only={{advisory_only}}\nallowed_write_scope={{allowed_write_scope}}\ntimeout_sec={{timeout_sec}}\nmax_retries={{max_retries}}",
        "[canonical_inputs]\n输入 spec 与 spec_gate_decision：\n{{canonical_input_summary}}\n\n完整 canonical_inputs（JSON）：\n{{canonical_inputs_json}}",
        "[projection_summary]\nSpecProjection 摘要：\n{{projection_summary}}",
        "[constraint_summary]\n必须覆盖 requirement_constraints：\n{{constraint_summary}}",
        "[workflow_discipline]\n本节点保留 using-superpowers 与 brainstorming 的纪律约束，但这是 Aria 非交互运行：不得向用户提问或等待确认，不得启动交互式 Todo/确认流程。未决问题必须写入候选产物的 open_questions 或待确认项。风险必须显式写入“## 风险”。{{workflow_discipline_summary}}",
        "[output_schema]\n{{output_schema_summary}}\n最终 stdout 必须包含 <ARIA_STRUCTURED_OUTPUT> JSON block，且 artifact_kind 必须等于 design。JSON 内的 markdown 字段必须包含架构摘要、设计决策、公共组件、数据模型、API 契约、风险和待确认项。\n</ARIA_STRUCTURED_OUTPUT> 只能作为结束标签出现。",
        "[completion_or_failure]\n不要直接修改 plan。不要生成 dispatch_package。",
    )
}

fn n11_sections() -> BTreeMap<PromptSection, String> {
    section_map(
        "[system]\n你是 Aria 的候选 plan 生成器。你只生成可编译为 PlanProjection 的候选计划文档；daemon 负责 canonical 校验、落盘、OpenSpec tasks 写回和 dispatch_package 生成。",
        "[node_contract]\nnode_id={{node_id}}\nruntime_role={{runtime_role}}\nadapter_role={{adapter_role}}\nadvisory_only={{advisory_only}}\nallowed_write_scope={{allowed_write_scope}}\ntimeout_sec={{timeout_sec}}\nmax_retries={{max_retries}}",
        "[canonical_inputs]\n输入 spec_projection、design_projection、readiness_check 与 constraint bundle：\n{{canonical_input_summary}}\n\n完整 canonical_inputs（JSON）：\n{{canonical_inputs_json}}",
        "[projection_summary]\n{{projection_summary}}",
        "[constraint_summary]\n必须覆盖 requirement_constraints 与 design_constraints：\n{{constraint_summary}}",
        "[workflow_discipline]\n本节点保留 using-superpowers 与 writing-plans 的纪律约束，但这是 Aria 非交互运行：不得向用户提问或等待确认，不得启动交互式 Todo/确认流程。未决问题必须写入候选产物。不得输出 superpowers 实施计划、文件写入步骤、commit 步骤、代码块或安装命令。你必须只输出 Aria PlanProjection 可消费的短 markdown。",
        "[output_schema]\n{{output_schema_summary}}\n最终 stdout 必须包含 <ARIA_STRUCTURED_OUTPUT> JSON block，且 artifact_kind 必须等于 plan。JSON 内 markdown 字段必须严格包含以下 heading 与表格结构：\n# Plan\n\n## 工作包\n\n| ID | Description | Execution Mode | Human Reason | Traceability | Acceptance |\n|----|-------------|----------------|--------------|--------------|------------|\n| WT-001 | ... | agent_only | | REQ-001, DEC-001 | AC-001 |\n\n## 依赖关系\n\n| From | To | Type |\n|------|----|------|\n\n规则：工作包 ID 必须使用 WT-001、WT-002 这类 WT 前缀；Execution Mode 只能使用 agent_only 或 human_required；Traceability 必须引用已知 REQ/DEC；Acceptance 必须引用成功标准 AC/SC。\n</ARIA_STRUCTURED_OUTPUT> 只能作为结束标签出现。",
        "[completion_or_failure]\n不要直接修改 OpenSpec。不要生成 dispatch_package。不要生成逐文件实现步骤；daemon 会从 PlanProjection 自动写回 tasks.md。",
    )
}

fn generic_sections(system_delta: &str, artifact_kind: &str) -> BTreeMap<PromptSection, String> {
    section_map(
        &format!(
            "[system]\n你是 Aria 的 {artifact_kind} 候选产物生成器或 advisory reviewer。Aria daemon 是唯一运行时真相源；你只能输出候选结果或 advisory 结果。\n{system_delta}"
        ),
        "[node_contract]\nnode_id={{node_id}}\nruntime_role={{runtime_role}}\nadapter_role={{adapter_role}}\nadvisory_only={{advisory_only}}\nallowed_write_scope={{allowed_write_scope}}\ntimeout_sec={{timeout_sec}}\nmax_retries={{max_retries}}",
        "[canonical_inputs]\n{{canonical_input_summary}}\n\n完整 canonical_inputs（JSON）：\n{{canonical_inputs_json}}",
        "[projection_summary]\n{{projection_summary}}",
        "[constraint_summary]\n{{constraint_summary}}",
        "[workflow_discipline]\n{{workflow_discipline_summary}}",
        &format!(
            "[output_schema]\n最终 stdout 必须包含 <ARIA_STRUCTURED_OUTPUT> JSON block，且 artifact_kind 必须等于 {artifact_kind}。\n</ARIA_STRUCTURED_OUTPUT> 只能作为结束标签出现。\n{{{{output_schema_summary}}}}"
        ),
        "[completion_or_failure]\nforbidden_actions={{forbidden_actions}}\ncompletion_criteria={{completion_criteria}}\nverification_commands={{verification_commands}}\n不要推进节点状态。不要绕过 daemon 写入 canonical artifact。失败时按 output schema 返回 failure summary。",
    )
}

fn section_map(
    system: &str,
    node_contract: &str,
    canonical_inputs: &str,
    projection_summary: &str,
    constraint_summary: &str,
    workflow_discipline: &str,
    output_schema: &str,
    completion_or_failure: &str,
) -> BTreeMap<PromptSection, String> {
    BTreeMap::from([
        (PromptSection::System, system.to_string()),
        (PromptSection::NodeContract, node_contract.to_string()),
        (PromptSection::CanonicalInputs, canonical_inputs.to_string()),
        (
            PromptSection::ProjectionSummary,
            projection_summary.to_string(),
        ),
        (
            PromptSection::ConstraintSummary,
            constraint_summary.to_string(),
        ),
        (
            PromptSection::WorkflowDiscipline,
            workflow_discipline.to_string(),
        ),
        (PromptSection::OutputSchema, output_schema.to_string()),
        (
            PromptSection::CompletionOrFailure,
            completion_or_failure.to_string(),
        ),
    ])
}

fn artifact_kind(node_id: &str) -> &'static str {
    match node_id {
        "N06" => "advisory_review",
        "N08" => "design_review",
        "N09" => "design_revision_record",
        "N10" => "readiness_check",
        "N11" => "plan",
        "N12" => "dispatch_package",
        "N16" => "coding_report",
        "N17" => "testing_report",
        "N18" => "code_review_report",
        "N19" => "coding_report",
        "N20" => "ready_advisory",
        "N24" => "integration_verify_advisory",
        "N25" => "final_review",
        "N26" => "dispatch_package",
        "N27" => "final_summary",
        _ => "unknown",
    }
}

fn system_delta(node_id: &str) -> &'static str {
    match node_id {
        "N06" => {
            "你是 advisory reviewer，只能指出 spec gate 风险与建议；daemon 才能生成 spec_gate_decision。不得要求 requirement_constraints 已存在。"
        }
        "N08" => {
            "你是设计评审 reviewer，优先输出阻塞性 findings、风险与明确 review_decision；不得直接修改 design。"
        }
        "N09" => {
            "你是设计修订候选生成器，只能基于 spec_projection、当前 design 和 design_review.findings 产出修订记录和候选 revised_design_markdown。"
        }
        "N10" => {
            "你是 plan readiness checker，只判断 spec/design/projection/bundle 是否足以进入计划，不生成 plan。"
        }
        "N11" => {
            "你是计划候选生成器，必须产出带显式 work_package_id、traceability_refs 和 acceptance_targets 的 plan。"
        }
        "N12" => {
            "你是 dispatch 候选生成器，必须把 PlanProjection work_package 映射为 WorkTask routing，并保留 source_work_package_id。"
        }
        "N16" => {
            "你是 Codex executor，只能在当前 worktask routing 授权的 worktree 写范围内完成 coding_report 候选输出；不得执行 git commit。"
        }
        "N17" => {
            "你是 Codex executor，默认只运行验证并产出 testing_report；除非 route 明确授权，不得修改生产代码。"
        }
        "N18" => {
            "你是 Codex reviewer，只读检查 worktask 改动并输出 code_review_report；不得修改文件。"
        }
        "N19" => {
            "你是 Codex executor，只能按失败报告或 review findings 做 bounded rework，并产出更新后的 coding_report。"
        }
        "N20" => {
            "你是 ready advisory reviewer，只能给出 ready/block/rework 候选建议；最终决策由 daemon 生成。"
        }
        "N24" => {
            "你是 integration verify advisory reviewer，只能给出 verify/rollback 候选建议；rollback 决策由 daemon 生成。"
        }
        "N25" => {
            "你是 Claude Code orchestrator，基于已有事实生成 final_review 和 coverage_summary 候选输出。"
        }
        "N26" => {
            "你是 Claude Code orchestrator，只有 approval gate 明确确认后才输出候选 patch task delta 或 dispatch routing 意图。"
        }
        "N27" => {
            "你是 Claude Code orchestrator，只引用 final_review 中已经存在的验证事实生成 final_summary。"
        }
        _ => "",
    }
}
