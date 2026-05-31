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
    let mut offset = 0;
    for line in input.split_inclusive('\n') {
        let line_start = offset;
        let line_end = line_start + line.len();
        let trimmed = line.trim();
        if let Some(fence) = artifact_fence_marker(trimmed) {
            let content_start = line_end;
            let close_start = last_closing_fence_start(input, content_start, &fence)?;
            return Some(&input[content_start..close_start]);
        }
        offset = line_end;
    }
    None
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

fn artifact_fence_marker(line: &str) -> Option<String> {
    let marker = fence_marker(line)?;
    let rest = line[marker.len()..].trim_start();
    rest.starts_with("artifact").then_some(marker)
}

fn fence_marker(line: &str) -> Option<String> {
    let first = line.as_bytes().first().copied()?;
    if first != b'`' && first != b'~' {
        return None;
    }
    let len = line
        .as_bytes()
        .iter()
        .take_while(|byte| **byte == first)
        .count();
    (len >= 3).then(|| std::iter::repeat_n(char::from(first), len).collect())
}

fn last_closing_fence_start(input: &str, content_start: usize, fence: &str) -> Option<usize> {
    let mut offset = content_start;
    let mut last = None;
    for line in input[content_start..].split_inclusive('\n') {
        let line_start = offset;
        let trimmed = line.trim();
        if trimmed.starts_with(fence) && trimmed[fence.len()..].trim().is_empty() {
            last = Some(line_start);
        }
        offset += line.len();
    }
    last
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
    fn extracts_fenced_artifact_content_with_inner_code_blocks() {
        let input = "前缀\n```artifact\n# Work Item\n\n## 验证命令\n\n```bash\nuv run python -m unittest discover -s tests -v\n```\n\n## 风险\n\n- 无\n```\n尾部";

        assert_eq!(
            extract_artifact_content(input),
            "# Work Item\n\n## 验证命令\n\n```bash\nuv run python -m unittest discover -s tests -v\n```\n\n## 风险\n\n- 无"
        );
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
