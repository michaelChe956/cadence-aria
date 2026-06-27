use crate::product::models::{ProviderName, WorkspaceSessionRecord, WorkspaceType};

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
                "OpenSpec 已启用。必须覆盖 Issue 所表达的 proposal constraints；Markdown spec 中必须声明稳定 requirement IDs，供 daemon 在 review pass 后写回 OpenSpec 并编译 requirement_constraints。不要把 OpenSpec 当作 runtime truth。"
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
    let base = if session.superpowers_enabled {
        match session.workspace_type {
            WorkspaceType::Story | WorkspaceType::Design => {
                "必须遵守 using-superpowers 与 brainstorming。必须优先通过交互提问解决需求、范围、验收标准中的未决问题，并等待用户回答后继续；如果需要向用户提问，必须使用结构化 AskUserQuestion / requestUserInput 交互能力。不要把 A/B/C 选择题作为最终候选产物正文输出，也不要把文本选择题当作正常交互路径；不要把可通过当前用户确认解决的问题直接写入待确认项。只有用户明确要求保留、用户回答后仍需后续确认，或当前 provider 环境确实无法交互时，才允许在待确认项/open_items 中保留。若仍输出了可解析的文本选择题，daemon 只会作为 text_fallback 异常兜底暂停 reviewer 并转换为用户选择卡片，用户回答后仅追加 compact QA，不会重新灌入完整 prompt。"
            }
            WorkspaceType::WorkItem => {
                "必须遵守 using-superpowers 与 writing-plans；只使用 writing-plans 的计划结构要求来生成候选 Work Item artifact，不要执行该技能默认的落盘和执行交接流程。不得直接输出实现代码，先生成可确认的计划与任务拆分。不要创建 docs/superpowers/plans 文件，不要询问 Subagent-Driven 或 Inline Execution；daemon 会负责候选产物落盘和后续执行调度。"
            }
            WorkspaceType::WorkItemPlan => {
                "必须遵守 using-superpowers 与 writing-plans；只使用 writing-plans 的计划结构要求来生成候选 Issue 级 Work Item Plan artifact，不要执行该技能默认的落盘和执行交接流程。聚焦于任务拆分、依赖关系、验证计划与执行顺序；不要直接输出实现代码，不要创建 docs/superpowers/plans 文件，不要询问 Subagent-Driven 或 Inline Execution；daemon 会负责候选产物落盘和后续执行调度。"
            }
        }
        .to_string()
    } else {
        "Superpowers 未启用；仍需显式说明假设、风险、待确认项与下一步。".to_string()
    };

    match (&session.workspace_type, &session.author_provider) {
        (WorkspaceType::Story | WorkspaceType::Design, ProviderName::ClaudeCode) => {
            format!(
                "{base}\n当前 author provider 是 Claude Code；需要向用户确认时，必须使用结构化 AskUserQuestion，让同一个 Claude Code 进程等待用户回答后继续。禁止输出文本 A/B/C 选择题作为交互替代；若仍输出可解析的文本选择题，daemon 仅作为 text_fallback 异常兜底处理，并在用户回答后只追加 compact QA。"
            )
        }
        (WorkspaceType::Story | WorkspaceType::Design, ProviderName::Codex) => {
            format!(
                "{base}\n当前 author provider 是 Codex；需要向用户确认时，必须使用结构化 requestUserInput，让同一个 Codex turn 等待用户回答后继续。禁止输出文本 1/2/3 或 A/B/C 选择题作为交互替代；若仍输出可解析的文本选择题，daemon 仅作为 text_fallback 异常兜底处理，并在用户回答后只追加 compact QA。"
            )
        }
        _ => base,
    }
}

pub(super) fn output_schema_for(workspace_type: &WorkspaceType) -> &'static str {
    match workspace_type {
        WorkspaceType::Story => {
            "Markdown Story Spec 必须包含以下 heading：\n\
             - ## 范围\n\
             - ## 用户故事\n\
             - ## 功能需求\n\
             - ## 成功标准\n\
             - ## 待确认项\n\
             - ## 非功能需求\n\n\
             最终候选 Markdown 必须用 ```artifact fenced block 包裹，且 fenced block 内第一行必须是 Story Spec 一级标题，例如 # <名称> Story Spec；过程说明必须放在 fenced block 外。每条需求必须显式写稳定 ID，例如 [REQ-001]；每条验收标准必须显式写稳定 ID，例如 [AC-001]。如果通过交互已解决所有疑问，## 待确认项 写“无”；不要为了填充该 heading 编造未决问题。"
        }
        WorkspaceType::Design => {
            "Markdown Design Spec 必须用 ```artifact fenced block 包裹，且 fenced block 内第一行必须是 Design Spec 一级标题；内容必须包含以下 heading：\n\
             - ## 设计范围\n\
             - ## 设计决策\n\
             - ## 公共组件\n\
             - ## API 契约\n\
             - ## 数据模型\n\
             - ## 风险\n\
             - ## 追踪关系\n\n\
             设计决策使用 [DEC-001]，组件使用 [CMP-001]，API 使用 [API-001]。"
        }
        WorkspaceType::WorkItem => {
            "Markdown Work Item 必须用 ```artifact fenced block 包裹，且 fenced block 内第一行必须是 Work Item 一级标题；内容必须包含目标、范围、任务拆分、依赖、验证命令、风险和追踪关系；任务使用 [TASK-001]，并绑定来源 Story/Design。"
        }
        WorkspaceType::WorkItemPlan => {
            "Markdown Work Item Plan 必须用 ```artifact fenced block 包裹，且 fenced block 内第一行必须是 Work Item Plan 一级标题；内容必须包含计划范围、任务拆分（[TASK-001]）、依赖图、验证计划、执行顺序、风险与追踪关系；每个任务必须绑定来源 Story/Design。"
        }
    }
}

pub(super) fn completion_or_failure_for(session: &WorkspaceSessionRecord) -> &'static str {
    if session.openspec_enabled {
        "不要直接修改 OpenSpec。不要直接生成 projection。daemon 会做结构化落盘、OpenSpec 写回与约束编译。"
    } else {
        "不要直接生成 projection。daemon 会做结构化落盘与校验。"
    }
}
