use crate::cross_cutting::document_ops::compute_sha256;

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

/// 顶层 fenced artifact 候选（无类型，只描述边界与正文）。
///
/// `artifact-candidate-selection` 的 Task 2/3/4 消费本结构与
/// [`scan_top_level_fenced_candidates`]；字段语义一经固定不得改名。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FencedArtifactCandidate {
    /// fence 内正文（不含 opening/closing fence 行本身，保留其间的原始字节）。
    pub markdown: String,
    /// opening fence 行号（1-based，按 content 内 `\n` 换行计数）。
    pub opening_line: usize,
    /// closing fence 行号（1-based）。
    pub closing_line: usize,
    /// opening fence 行首字节偏移。
    pub opening_byte: usize,
    /// closing fence 行尾字节偏移（含该行行尾换行符，若存在）。
    pub closing_byte_end: usize,
    /// `markdown` 正文的 sha256 hex。
    pub sha256: String,
}

/// 枚举顶层完整 fenced artifact 候选。
///
/// 只负责 fenced 形态：XML `<artifact>` marker 存在时由调用方沿用既有优先路径
/// （本次不扩 XML 多候选），`extract_artifact_content` 的无 marker heading fallback
/// 亦不在本函数职责内、保持不动。
///
/// 边界判定 fail-closed：同长度（或更长）非裸 fence 视为内层 fence 并配对；内层未
/// 成对闭合导致外层边界不可判定时，整块丢弃、不产出候选、不猜边界。
pub(crate) fn scan_top_level_fenced_candidates(output: &str) -> Vec<FencedArtifactCandidate> {
    let mut candidates = Vec::new();
    let mut open = None::<OpenArtifactCandidate>;
    let mut inner_fence = None::<(u8, usize)>;
    let mut offset = 0usize;

    for (index, raw_line) in output.split_inclusive('\n').enumerate() {
        let line_start = offset;
        let line_end = line_start + raw_line.len();
        offset = line_end;
        // 行尾 `\r`（CRLF）由 trim 吸收，行号与字节偏移一并保持稳定。
        let trimmed = raw_line.trim();

        let Some(current) = open else {
            if let Some((ch, len, rest)) = fence_run(trimmed)
                && rest.trim_start().starts_with("artifact")
            {
                open = Some(OpenArtifactCandidate {
                    ch,
                    len,
                    opening_line: index + 1,
                    opening_byte: line_start,
                    content_start: line_end,
                });
            }
            continue;
        };

        let Some((ch, len, rest)) = fence_run(trimmed) else {
            continue;
        };
        if let Some((inner_ch, inner_len)) = inner_fence {
            // 内层态：只有同字符、长度不短于内层的裸 fence 才闭合内层。
            if ch == inner_ch && len >= inner_len && rest.trim().is_empty() {
                inner_fence = None;
            }
            continue;
        }
        if ch != current.ch || len < current.len {
            continue;
        }
        if !rest.trim().is_empty() {
            // 非裸同长（或更长）fence：正文内层 fence 起界。
            inner_fence = Some((ch, len));
            continue;
        }

        let markdown = output[current.content_start..line_start].to_string();
        candidates.push(FencedArtifactCandidate {
            sha256: compute_sha256(markdown.as_bytes()),
            markdown,
            opening_line: current.opening_line,
            closing_line: index + 1,
            opening_byte: current.opening_byte,
            closing_byte_end: line_end,
        });
        open = None;
    }

    candidates
}

/// 扫描器内部游标：当前 opening fence 与其正文起点。
#[derive(Clone, Copy)]
struct OpenArtifactCandidate {
    ch: u8,
    len: usize,
    opening_line: usize,
    opening_byte: usize,
    content_start: usize,
}

/// 解析行首 fence：返回（fence 字符、连续长度、marker 之后的剩余文本）。
fn fence_run(trimmed_line: &str) -> Option<(u8, usize, &str)> {
    let first = trimmed_line.as_bytes().first().copied()?;
    if first != b'`' && first != b'~' {
        return None;
    }
    let len = trimmed_line
        .as_bytes()
        .iter()
        .take_while(|byte| **byte == first)
        .count();
    (len >= 3).then(|| (first, len, &trimmed_line[len..]))
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
        if trimmed.starts_with(fence) {
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
    use super::{extract_artifact_content, scan_top_level_fenced_candidates};

    /// F-46 形态样本（合成，行结构等价真实 content :198-:361）。
    const F46_LIKE: &str = "前言\n```artifact\n# 示意标题\n...\n```\n中间 <thinking>过程</thinking> 文本\n```artifact\n# 真实 Story Spec\n\n## 范围\n- x\n```\n尾随说明\n";

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
    fn strips_inline_process_text_after_artifact_closing_fence() {
        let input =
            "前缀\n```artifact\n# Story Spec\n\n## 范围\n正文\n```Story Spec 已生成完毕。过程说明";

        assert_eq!(
            extract_artifact_content(input),
            "# Story Spec\n\n## 范围\n正文"
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

    #[test]
    fn scan_finds_two_top_level_candidates() {
        let candidates = scan_top_level_fenced_candidates(F46_LIKE);

        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].markdown, "# 示意标题\n...\n");
        assert!(candidates[0].markdown.contains("示意标题"));
        assert_eq!(candidates[0].opening_line, 2);
        assert_eq!(candidates[0].closing_line, 5);
        assert_eq!(
            candidates[0].sha256,
            "7022a5fa2858f62dd5cc40dd104ef72d00138cb7db8829a640114c574d20be57"
        );
        assert_eq!(
            candidates[1].markdown,
            "# 真实 Story Spec\n\n## 范围\n- x\n"
        );
        assert!(candidates[1].markdown.contains("真实 Story Spec"));
        // opening_line 是 opening fence 行号（与计划 Task 4 诊断 JSON 示例
        // `{"opening_line":200,"closing_line":203}` 及 F-46 样本记法
        // 「真实候选 :270-361」一致）；计划 Task 1 骨架里写的 8 是该示例的
        // 正文首行号，实测扫描器返回 7（= fence 行）。
        assert_eq!(candidates[1].opening_line, 7);
        assert_eq!(candidates[1].closing_line, 12);
        assert_eq!(
            candidates[1].sha256,
            "e3fd54fa683bf0be16af73c84d94d28e6e26be606116a968a7c70aaa903ab9d9"
        );
    }

    #[test]
    fn scan_skips_nested_inner_fences() {
        let input = "前言\n```artifact\n# Story Spec\n\n```bash\necho hi\n```\n\n## 范围\n正文\n```\n尾部\n";

        let candidates = scan_top_level_fenced_candidates(input);

        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].markdown,
            "# Story Spec\n\n```bash\necho hi\n```\n\n## 范围\n正文\n"
        );
        assert_eq!(candidates[0].opening_line, 2);
        assert_eq!(candidates[0].closing_line, 11);
    }

    #[test]
    fn scan_four_backtick_outer_holds_inner_triple() {
        let input = "````artifact\n# Story Spec\n\n```\ncode\n```\n\n## 范围\n正文\n````\n";

        let candidates = scan_top_level_fenced_candidates(input);

        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].markdown,
            "# Story Spec\n\n```\ncode\n```\n\n## 范围\n正文\n"
        );
        assert_eq!(candidates[0].opening_line, 1);
        assert_eq!(candidates[0].closing_line, 10);
    }

    #[test]
    fn scan_crlf_line_numbers_stable() {
        let crlf = F46_LIKE.replace('\n', "\r\n");

        let lf_candidates = scan_top_level_fenced_candidates(F46_LIKE);
        let crlf_candidates = scan_top_level_fenced_candidates(&crlf);

        assert_eq!(crlf_candidates.len(), lf_candidates.len());
        for (lf, crlf_candidate) in lf_candidates.iter().zip(crlf_candidates.iter()) {
            assert_eq!(crlf_candidate.opening_line, lf.opening_line);
            assert_eq!(crlf_candidate.closing_line, lf.closing_line);
        }
        assert!(crlf[crlf_candidates[1].opening_byte..].starts_with("```artifact"));
        assert!(crlf[..crlf_candidates[1].closing_byte_end].ends_with("```\r\n"));
    }

    #[test]
    fn scan_empty_candidate_is_enumerated_not_panicking() {
        let candidates = scan_top_level_fenced_candidates("```artifact\n```\n尾随\n");

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].markdown, "");
        assert_eq!(candidates[0].opening_line, 1);
        assert_eq!(candidates[0].closing_line, 2);
        assert_eq!(
            candidates[0].sha256,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn scan_unclosed_inner_same_length_fence_yields_no_candidate() {
        // 外层 ```artifact 内出现同长度非裸 fence（```bash）会进入内层态；本输入只剩
        // 一个裸 fence，无法判定它闭合内层还是外层（同长度边界歧义）→ fail-closed，
        // 不猜边界、不产出候选。
        let input = "```artifact\n# Story Spec\n\n```bash\necho hi\n```\n";

        assert!(scan_top_level_fenced_candidates(input).is_empty());
    }
}
