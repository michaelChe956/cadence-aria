use super::super::SessionMessage;

pub(super) fn reviewer_context_content(message: &SessionMessage) -> Option<String> {
    if matches!(message.role.as_str(), "assistant" | "provider") {
        return None;
    }

    if message.role == "system" && message.content.starts_with("Workspace 生成任务已准备\n\n")
    {
        return reviewer_generation_context(&message.content);
    }

    Some(message.content.clone())
}

fn reviewer_generation_context(generation_brief: &str) -> Option<String> {
    let sections = ["canonical_inputs", "constraint_summary"]
        .into_iter()
        .filter_map(|section_name| {
            generation_brief_section(generation_brief, section_name)
                .filter(|content| !content.is_empty())
                .map(|content| format!("[{section_name}]\n{content}"))
        })
        .collect::<Vec<_>>();

    (!sections.is_empty()).then(|| {
        format!(
            "生成阶段已确认的审核上下文（不包含 author 工作流指令）：\n\n{}",
            sections.join("\n\n")
        )
    })
}

fn generation_brief_section<'a>(generation_brief: &'a str, section_name: &str) -> Option<&'a str> {
    let heading = format!("[{section_name}]\n");
    let section_start = generation_brief.find(&heading)? + heading.len();
    let remaining = &generation_brief[section_start..];
    let section_end = remaining.find("\n\n[").unwrap_or(remaining.len());

    Some(remaining[..section_end].trim_end())
}
