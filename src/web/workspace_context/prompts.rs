use crate::product::cadence_skills::routing_reference::direct_cadence_routing_rules_reference;
use crate::product::models::{ProviderName, WorkspaceSessionRecord, WorkspaceType};
use crate::product::workspace_engine::{
    allowed_outputs_for, author_artifact_schema_contract_for, forbidden_outputs_for,
};

pub(super) fn workspace_type_label(workspace_type: &WorkspaceType) -> &'static str {
    match workspace_type {
        WorkspaceType::Story => "Story Spec",
        WorkspaceType::Design => "Design Spec",
        WorkspaceType::WorkItem => "Work Item",
        WorkspaceType::WorkItemPlan => "Work Item Plan",
    }
}

pub(super) fn node_id_for(workspace_type: &WorkspaceType) -> &'static str {
    match workspace_type {
        WorkspaceType::Story => "N05",
        WorkspaceType::Design => "N07",
        WorkspaceType::WorkItem => "WORK_ITEM",
        WorkspaceType::WorkItemPlan => "WORK_ITEM_PLAN",
    }
}

pub(super) fn workspace_runtime_role(workspace_type: &WorkspaceType) -> &'static str {
    match workspace_type {
        WorkspaceType::Story => "story_spec",
        WorkspaceType::Design => "design_spec",
        WorkspaceType::WorkItem => "work_item",
        WorkspaceType::WorkItemPlan => "work_item_plan",
    }
}

pub(super) fn system_prompt_for(workspace_type: &WorkspaceType) -> &'static str {
    match workspace_type {
        WorkspaceType::Story => {
            "你是 Aria 的候选 spec 生成器。你负责基于 Issue、Repository 代码上下文和项目规则生成用户可读 Markdown Story Spec 候选；daemon 负责校验、落盘、编译 SpecProjection。"
        }
        WorkspaceType::Design => {
            "你是 Aria 的候选 design 生成器。你负责基于已确认 Story Spec、Repository 代码上下文和项目规则生成候选设计文档；daemon 负责 canonical 校验、落盘与 DesignProjection 编译。"
        }
        WorkspaceType::WorkItem => {
            "你是 Aria 的候选 work item 生成器。你负责基于已确认 Story Spec、Design Spec、Repository 代码上下文和项目规则生成候选工作项与计划输入；daemon 负责校验、落盘与后续执行调度。"
        }
        WorkspaceType::WorkItemPlan => {
            "你是 Aria 的候选 work item plan 生成器。你负责基于已确认 Story Spec、Design Spec、Repository 代码上下文和项目规则生成候选 Issue 级 Work Item Plan；daemon 负责校验、落盘与 Work Item Plan 编译。"
        }
    }
}

pub(super) fn constraint_summary_for(session: &WorkspaceSessionRecord) -> String {
    if session.openspec_enabled {
        match session.workspace_type {
            WorkspaceType::Story => {
                "OpenSpec 已启用。必须覆盖 Issue 所表达的 proposal constraints；Markdown spec 中必须声明稳定 requirement IDs，供 daemon 在 review pass 后写回 OpenSpec 并编译 requirement_constraints。若 Issue 明示文件/模块归属、公开 API、复用/依赖关系或自动化验证责任，必须把每项约束分别固化为带 source id 的稳定 [REQ-*]；文件/模块归属覆盖具体 API/行为时，需求必须将该具体 API/行为与所属文件/模块直接绑定，声明、重导出或范围概述不能替代。若明示规则之间存在例外、优先级或集合包含关系，需求必须保留该关系及适用范围，不得把带例外的规则写成无条件绝对规则。仅在范围段提及不足以覆盖这些约束。可观察行为的验收必须用 [AC-*] 关联相应 [REQ-*]。不要把 OpenSpec 当作 runtime truth。"
                    .to_string()
            }
            WorkspaceType::Design => {
                "OpenSpec 已启用。必须覆盖已确认 Story Spec 的 requirement constraints；设计决策、组件/API 与风险必须可追踪，供 daemon 写回 OpenSpec design constraints。不要把 OpenSpec 当作 runtime truth。"
                    .to_string()
            }
            WorkspaceType::WorkItem => {
                "OpenSpec 已启用。必须覆盖已确认 Story/Design 约束，并产生可追踪的 task/routing 候选，供 daemon 写回 OpenSpec tasks constraints。不要把 OpenSpec 当作 runtime truth。"
                    .to_string()
            }
            WorkspaceType::WorkItemPlan => {
                "OpenSpec 已启用。必须基于已确认 Story/Design 约束生成可追踪的 Issue 级 Work Item Plan，明确任务拆分、依赖与验证计划；供 daemon 写回 OpenSpec tasks constraints。不要把 OpenSpec 当作 runtime truth。"
                    .to_string()
            }
        }
    } else {
        "OpenSpec 未启用；仍需保持产物结构化、可追踪，并明确记录假设与待确认项。".to_string()
    }
}

pub(super) fn workflow_discipline_for(session: &WorkspaceSessionRecord) -> String {
    let routing = match session.workspace_type {
        WorkspaceType::Story | WorkspaceType::Design => {
            "当前阶段：新功能、行为变化或方案讨论的候选产物 author。\n必调 Skill：using-superpowers → brainstorming。\n前置 gate：Aria 现有 author-confirmation/human-confirmation gate 承接人工确认；不得由 Provider 写入 canonical artifact。\n"
        }
        WorkspaceType::WorkItem | WorkspaceType::WorkItemPlan => {
            "当前阶段：已确认 Story/Design 与 OpenSpec 契约后的候选规划。\n必调 Skill：using-superpowers → writing-plans。\n前置 gate：仅在确认范围内规划；Aria daemon 的现有 human-confirmation gate 承接人工确认，Provider 不得写入 canonical artifact。\n"
        }
    };
    let base = if session.superpowers_enabled {
        match session.workspace_type {
            WorkspaceType::Story | WorkspaceType::Design => {
                "必须遵守 using-superpowers 与 brainstorming 的纪律。必须优先通过可用交互机制解决需求、范围、验收标准中的未决问题，并等待用户回答后继续；若当前 provider 环境没有原生结构化交互能力，必须输出 daemon 可识别的暂停信号并交给 text_fallback，而不是伪造工具调用。不要把 A/B/C 选择题作为最终候选产物正文输出，也不要把文本选择题当作正常交互路径；不要把可通过当前用户确认解决的问题直接写入待确认项。只有用户明确要求保留、用户回答后仍需后续确认，或当前 provider 环境确实无法交互时，才允许在待确认项/open_items 中保留。若仍输出了可解析的文本选择题，daemon 只会作为 text_fallback 异常兜底暂停 reviewer 并转换为用户选择卡片，用户回答后仅追加 compact QA，不会重新灌入完整 prompt。"
            }
            WorkspaceType::WorkItem => {
                "必须遵守 using-superpowers 与 writing-plans；只使用 writing-plans 的计划结构要求来生成候选 Work Item artifact，不要执行该技能默认的落盘和执行交接流程。不得直接输出实现代码，先生成可确认的单个可执行任务说明；允许包含子步骤，但不得生成兄弟任务或 Issue 级拆分。不要创建 docs/superpowers/plans 文件，不要询问 Subagent-Driven 或 Inline Execution；daemon 会负责候选产物落盘和后续执行调度。"
            }
            WorkspaceType::WorkItemPlan => {
                "必须遵守 using-superpowers 与 writing-plans；只使用 writing-plans 的计划结构要求来生成候选 Issue 级 Work Item Plan artifact，不要执行该技能默认的落盘和执行交接流程。聚焦于任务拆分、依赖关系、验证计划与执行顺序；不要直接输出实现代码，不要创建 docs/superpowers/plans 文件，不要询问 Subagent-Driven 或 Inline Execution；daemon 会负责候选产物落盘和后续执行调度。"
            }
        }
        .to_string()
    } else {
        "当前 provider 环境未启用必调 Superpowers Skill；必须停止并报告，等待 Aria 恢复可用的原生 Skill 环境后再继续。不得以本 prompt 摘要、文本伪造或待确认项替代必调 Skill。".to_string()
    };

    let workflow = if matches!(
        session.workspace_type,
        WorkspaceType::Story | WorkspaceType::Design
    ) {
        format!(
            "{base}\n{}",
            structured_interaction_guidance_for(&session.author_provider)
        )
    } else {
        base
    };

    format!("{}\n{}", direct_cadence_routing_rules_reference(), routing) + &workflow
}

fn structured_interaction_guidance_for(provider: &ProviderName) -> &'static str {
    match provider {
        ProviderName::ClaudeCode => {
            "当前 author provider 是 Claude Code；需要向用户确认时，必须使用结构化 AskUserQuestion，让同一个 Claude Code 进程等待用户回答后继续。禁止输出文本 A/B/C 选择题作为交互替代；若仍输出可解析的文本选择题，daemon 仅作为 text_fallback 异常兜底处理，并在用户回答后只追加 compact QA。"
        }
        ProviderName::Codex => {
            "当前 author provider 是 Codex；需要向用户确认时，必须使用结构化 requestUserInput，让同一个 Codex turn 等待用户回答后继续。禁止输出文本 1/2/3 或 A/B/C 选择题作为交互替代；若仍输出可解析的文本选择题，daemon 仅作为 text_fallback 异常兜底处理，并在用户回答后只追加 compact QA。"
        }
        ProviderName::Pi => {
            "当前 author provider 是 Pi；需要向用户确认时，使用 `ask_user` 工具提问并等待回答。禁止输出文本 A/B/C 选择题作为交互替代；`ask_user` 会经 Aria 弹出选择卡片，用户回答后同一 Pi 进程继续。"
        }
        ProviderName::Fake => {
            "当前 author provider 未声明原生结构化交互能力；需要向用户确认时，必须输出 daemon 可识别的暂停信号并交给 text_fallback。禁止伪造 AskUserQuestion 或 requestUserInput 工具调用，也不要把文本选择题作为正常交互路径。"
        }
    }
}

pub(super) fn output_schema_for(workspace_type: &WorkspaceType) -> String {
    let mut schema = match workspace_type {
        WorkspaceType::Story => {
            "最终候选 Markdown 必须用 ```artifact fenced block 包裹，且 fenced block 内第一行必须是 Story Spec 一级标题，例如 # <名称> Story Spec；过程说明必须放在 fenced block 外。保留与上游 Issue 的需求追踪叙述；下方 parser schema 是唯一的 Markdown 结构、稳定 ID、追踪与禁止项合同。如果通过 AskUserQuestion、requestUserInput 或 text_fallback 结构化交互解决了影响范围、需求、成功标准或验收口径的问题，必须在 artifact 正文加入 ## 用户确认决策，使用稳定 ID（例如 author-decision-001）记录问题、用户选择、来源机制，并把每条决策绑定到受影响的 parser-required 需求/验收 ID；已解决的选择不得再写入 ## 待确认项。实现细节类选择只记录为 Design 阶段输入，不要固化成 Story 范围或验收标准。如果通过交互已解决所有疑问，## 待确认项 写“无”；不要为了填充该 heading 编造未决问题。"
                .to_string()
        }
        WorkspaceType::Design => {
            "Markdown Design Spec 必须用 ```artifact fenced block 包裹，且 fenced block 内第一行必须是 Design Spec 一级标题。基于上游 Story Spec 保留设计追踪关系；下方 parser schema 是唯一的 Markdown 结构、稳定 ID、追踪与禁止项合同。如果上游 Story 或本轮 author 通过结构化交互形成用户确认决策，必须在 ## 设计决策 中记录对应 author-decision-* 或将其映射到 parser-required 设计决策 ID，并说明 AskUserQuestion/requestUserInput/text_fallback 来源；必须在 ## 追踪关系 中把这些用户确认决策绑定到来源需求、验收或设计决策 ID。"
                .to_string()
        }
        WorkspaceType::WorkItem => {
            "Markdown Work Item 必须用 ```artifact fenced block 包裹，且 fenced block 内第一行必须是 Work Item 一级标题；内容必须描述基于上游 Story/Design 的单个可执行任务。下方 parser schema 是唯一的 Markdown 结构、追踪与禁止项合同。如果来源 Story/Design 或本轮 author 包含结构化交互形成的用户确认决策，必须在目标、范围或追踪关系中写明对应 author-decision-*，并绑定到来源需求/设计/验收 ID。内容规模不超过 40k 属正常范围，40001..=50000 必须仍能由单个会话完成编码、返修与验证，超过 50k 不得作为单个 Work Item；禁止跨任务内容、兄弟任务、Issue 级完整计划和其它任务的交叉内容。"
                .to_string()
        }
        WorkspaceType::WorkItemPlan => {
            "Markdown Work Item Plan 必须用 ```artifact fenced block 包裹，且 fenced block 内第一行必须是 Work Item Plan 一级标题；内容必须包含计划范围、任务拆分（[TASK-001]）、依赖图、验证计划、执行顺序、风险与追踪关系；每个任务必须显式写出并绑定来源 Story/Design source ids，例如 Story Spec story_spec_0001、Design Spec design_spec_0001。拆分目标是在单个 Claude Code 或 Codex 会话可完成的前提下最少拆分。每个任务必须最大内聚，优先合并目标一致、范围重叠且可在同一会话闭环的工作；不超过 40k 属正常范围，40001..=50000 需经 Reviewer 判断，超过 50k 必须继续拆分。"
                .to_string()
        }
    };
    if let Some(contract) = author_artifact_schema_contract_for(workspace_type) {
        schema.push_str(&contract);
    }
    schema
}

pub(super) fn runtime_contract_for(session: &WorkspaceSessionRecord) -> String {
    let openspec = if session.openspec_enabled {
        "[openspec_contract]\n\
         - 必须保持 Story/Design/Work Item 追踪关系。\n\
         - 不得忽略 source ids、verification commands 或 planned context。\n\
         - 不要直接修改 OpenSpec；由 daemon 负责后续写回与 projection。"
    } else {
        "[openspec_contract]\n\
         - OpenSpec 未启用，但仍需保持产物可追踪。"
    };
    let discipline = match (&session.workspace_type, session.superpowers_enabled) {
        (WorkspaceType::Story | WorkspaceType::Design, true) => {
            "- 必须遵守 using-superpowers 与 brainstorming 的纪律；优先澄清需求、范围、验收标准与风险。\n\
             - 若 provider 环境没有对应技能文件，也必须遵守本 prompt 内嵌的纪律摘要。"
        }
        (WorkspaceType::WorkItem | WorkspaceType::WorkItemPlan, true) => {
            "- 必须遵守 using-superpowers 与 writing-plans 的计划结构纪律。\n\
             - 生成候选计划或任务说明，不执行代码修改。"
        }
        (_, false) => "- Superpowers 未启用，但仍需明确假设、风险、验证与下一步。",
    };
    let code_reading = "\
         [code_reading_contract]\n\
         - 大范围理解 Repository 代码、调用链或影响面时，必须优先使用 CodeGraph MCP（mcp__codegraph__codegraph_explore）或等价的 codegraph explore。\n\
         - 精确结构阅读优先使用 ast-grep outline，再按需读取目标符号或文件片段。\n\
         - 只有 CodeGraph/MCP 或 ast-grep 不可用时才降级到 rg/find/ls/cat，并在输出中说明降级原因。\n\
         - 如果使用 MCP/CodeGraph 工具，daemon 会将 mcp__... tool_use 记录为 execution event，供用户审计。";
    format!(
        "{openspec}\n\n\
         {code_reading}\n\n\
         [allowed_outputs]\n\
         - {}\n\n\
         [forbidden_outputs]\n\
         - {}\n\n\
         [superpowers_contract]\n\
         {discipline}",
        allowed_outputs_for(&session.workspace_type),
        forbidden_outputs_for(&session.workspace_type)
    )
}

pub(super) fn completion_or_failure_for(session: &WorkspaceSessionRecord) -> &'static str {
    if session.openspec_enabled {
        "不要直接修改 OpenSpec。不要直接生成 projection。daemon 会做结构化落盘、OpenSpec 写回与约束编译。"
    } else {
        "不要直接生成 projection。daemon 会做结构化落盘与校验。"
    }
}
