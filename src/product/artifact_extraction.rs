pub fn extract_artifact_content(full_output: &str) -> String {
    let trimmed = full_output.trim();
    if let Some(content) = extract_between_artifact_tags(trimmed) {
        return content.trim().to_string();
    }
    if let Some(content) = extract_between_fenced_artifact_tags(trimmed) {
        return content.trim().to_string();
    }
    if let Some(content) = extract_after_open_artifact_tag(trimmed) {
        return content.trim().to_string();
    }
    if let Some(heading_index) = first_markdown_heading_index(trimmed) {
        return trimmed[heading_index..].trim().to_string();
    }
    trimmed.to_string()
}

fn extract_between_artifact_tags(input: &str) -> Option<&str> {
    let start = input.find("<artifact>")?;
    let content_start = start + "<artifact>".len();
    let end = input[content_start..].find("</artifact>")?;
    Some(&input[content_start..content_start + end])
}

fn extract_between_fenced_artifact_tags(input: &str) -> Option<&str> {
    let start = input.find("```artifact")?;
    let after_marker = &input[start + "```artifact".len()..];
    let content_start = after_marker
        .find('\n')
        .map(|index| start + "```artifact".len() + index + 1)
        .unwrap_or(start + "```artifact".len());
    let end = input[content_start..].find("\n```")?;
    Some(&input[content_start..content_start + end])
}

fn extract_after_open_artifact_tag(input: &str) -> Option<&str> {
    let start = input.find("<artifact>")?;
    Some(&input[start + "<artifact>".len()..])
}

fn first_markdown_heading_index(input: &str) -> Option<usize> {
    if input.starts_with("# ") {
        return Some(0);
    }
    input
        .find("\n# ")
        .map(|index| index + 1)
        .or_else(|| first_workspace_heading_index(input))
}

fn first_workspace_heading_index(input: &str) -> Option<usize> {
    ["Story Spec", "Design Spec", "Work Item"]
        .iter()
        .filter_map(|marker| input.find(marker))
        .filter_map(|marker_index| {
            input[..marker_index]
                .rfind("# ")
                .filter(|heading_index| heading_index + 2 < marker_index)
        })
        .min()
}

#[cfg(test)]
mod tests {
    use super::extract_artifact_content;

    #[test]
    fn extracts_content_between_complete_artifact_tags() {
        let input = "思考过程\n<artifact>\n# Story Spec\n\n正文\n</artifact>\n尾部";

        assert_eq!(extract_artifact_content(input), "# Story Spec\n\n正文");
    }

    #[test]
    fn extracts_content_after_unclosed_artifact_tag() {
        let input = "前缀\n<artifact>\n# Design Spec\n\n正文";

        assert_eq!(extract_artifact_content(input), "# Design Spec\n\n正文");
    }

    #[test]
    fn extracts_content_between_fenced_artifact_tags() {
        let input = "前缀\n```artifact\n# Story Spec\n\n正文\n```\n尾部";

        assert_eq!(extract_artifact_content(input), "# Story Spec\n\n正文");
    }

    #[test]
    fn falls_back_to_first_markdown_heading() {
        let input = "分析过程\n\n# Work Item\n\n- step";

        assert_eq!(extract_artifact_content(input), "# Work Item\n\n- step");
    }

    #[test]
    fn falls_back_to_localized_story_spec_heading_without_preceding_newline() {
        let input = "分析过程。# 爬楼梯问题 Story Spec\n\n## 范围\n正文";

        assert_eq!(
            extract_artifact_content(input),
            "# 爬楼梯问题 Story Spec\n\n## 范围\n正文"
        );
    }

    #[test]
    fn trims_original_content_when_no_marker_or_heading_exists() {
        assert_eq!(extract_artifact_content("  plain output  "), "plain output");
    }
}
