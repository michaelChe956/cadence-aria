use super::*;

pub(crate) fn detect_author_choice_request(
    content: &str,
    workspace_type: &WorkspaceType,
) -> Option<(String, Vec<ChoiceOptionData>)> {
    if !matches!(workspace_type, WorkspaceType::Story | WorkspaceType::Design) {
        return None;
    }
    if content_has_complete_workspace_artifact(content, workspace_type) {
        return None;
    }
    if !looks_like_user_question(content) {
        return None;
    }

    if let Some(choice) = detect_explicit_choice_request(content) {
        return Some(choice);
    }

    detect_recommendation_choice_request(content)
}

pub(crate) fn detect_explicit_choice_request(
    content: &str,
) -> Option<(String, Vec<ChoiceOptionData>)> {
    let lines = content.lines().collect::<Vec<_>>();
    let mut candidates = Vec::new();
    let mut index = 0;

    while index < lines.len() {
        let trimmed = lines[index].trim();
        let Some(first_option) = parse_choice_option_line(trimmed) else {
            index += 1;
            continue;
        };

        let option_start = index;
        let mut options = vec![first_option];
        index += 1;
        while index < lines.len() {
            let trimmed = lines[index].trim();
            if trimmed.is_empty() {
                break;
            }
            let Some(option) = parse_choice_option_line(trimmed) else {
                break;
            };
            options.push(option);
            index += 1;
        }

        if options.len() >= 2 {
            candidates.push((choice_prompt_before_options(&lines, option_start), options));
        }
    }

    candidates.into_iter().last()
}

pub(crate) fn choice_prompt_before_options(lines: &[&str], option_start: usize) -> String {
    let Some((prompt_start, prompt_lines)) = previous_non_empty_block(lines, option_start) else {
        return default_choice_prompt();
    };

    let mut prompt_parts = Vec::new();
    if let Some((_, heading_lines)) = previous_non_empty_block(lines, prompt_start)
        && heading_lines.len() == 1
        && looks_like_choice_question_heading(&heading_lines[0])
    {
        prompt_parts.extend(heading_lines);
    }
    prompt_parts.extend(prompt_lines);

    let prompt = prompt_parts.join("\n");
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        default_choice_prompt()
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn previous_non_empty_block(
    lines: &[&str],
    before: usize,
) -> Option<(usize, Vec<String>)> {
    if before == 0 {
        return None;
    }

    let mut index = before;
    loop {
        index -= 1;
        if !lines[index].trim().is_empty() {
            break;
        }
        if index == 0 {
            return None;
        }
    }

    let end = index + 1;
    while index > 0 && !lines[index - 1].trim().is_empty() {
        index -= 1;
    }

    Some((
        index,
        lines[index..end]
            .iter()
            .map(|line| line.trim().to_string())
            .collect(),
    ))
}

pub(crate) fn looks_like_choice_question_heading(line: &str) -> bool {
    let normalized = line.trim().trim_matches('*').trim_matches('_').trim();
    (normalized.starts_with("问题") || normalized.starts_with("Question"))
        && (normalized.contains('：') || normalized.contains(':'))
}

pub(crate) fn default_choice_prompt() -> String {
    "请选择下一步处理方式。".to_string()
}

pub(crate) fn detect_recommendation_choice_request(
    content: &str,
) -> Option<(String, Vec<ChoiceOptionData>)> {
    let mut prompt_lines = Vec::new();
    let mut option_texts = Vec::new();
    let mut seen_option_line = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Some(text) = strip_choice_prefix(trimmed, &["推荐选项：", "推荐选项:"]) {
            seen_option_line = true;
            push_choice_text(&mut option_texts, text);
            continue;
        }

        if let Some(text) = strip_choice_prefix(
            trimmed,
            &["其他可选：", "其他可选:", "其他选项：", "其他选项:"],
        ) {
            seen_option_line = true;
            for choice_text in split_inline_choices(text) {
                push_choice_text(&mut option_texts, choice_text);
            }
            continue;
        }

        if !seen_option_line {
            prompt_lines.push(trimmed.to_string());
        }
    }

    if option_texts.len() < 2 {
        return None;
    }

    let options = option_texts
        .into_iter()
        .enumerate()
        .map(|(idx, text)| {
            let id = ((b'A' + idx as u8) as char).to_string();
            ChoiceOptionData {
                id: id.clone(),
                label: format!("{id}. {text}"),
                description: None,
            }
        })
        .collect();
    let prompt = prompt_lines.join("\n");
    let prompt = if prompt.trim().is_empty() {
        "请选择下一步处理方式。".to_string()
    } else {
        prompt.trim().to_string()
    };
    Some((prompt, options))
}

pub(crate) fn strip_choice_prefix<'a>(line: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    prefixes
        .iter()
        .find_map(|prefix| line.strip_prefix(prefix))
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

pub(crate) fn split_inline_choices(text: &str) -> Vec<&str> {
    text.split(['；', ';'])
        .flat_map(|part| part.split(" 或 "))
        .flat_map(|part| part.split("或"))
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect()
}

pub(crate) fn push_choice_text(option_texts: &mut Vec<String>, text: &str) {
    let normalized = text
        .trim()
        .trim_start_matches("或")
        .trim()
        .trim_end_matches(['。', '.', '；', ';'])
        .trim();
    if !normalized.is_empty() {
        option_texts.push(normalized.to_string());
    }
}

pub(crate) fn looks_like_user_question(content: &str) -> bool {
    content.contains('?')
        || content.contains('？')
        || content.contains("需要确认")
        || content.contains("需要先确认")
        || content.contains("请选择")
        || content.contains("如何处理")
}

pub(crate) fn content_has_complete_workspace_artifact(
    content: &str,
    workspace_type: &WorkspaceType,
) -> bool {
    validate_workspace_artifact_constraints(content, workspace_type).passed
}

pub(crate) fn normalize_workspace_heading_line(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let heading_level = trimmed.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&heading_level) {
        return None;
    }

    let heading_text = trimmed.get(heading_level..)?.trim();
    if heading_text.is_empty() {
        return None;
    }

    Some(strip_heading_number_prefix(heading_text).trim().to_string())
}

pub(crate) fn strip_heading_number_prefix(text: &str) -> &str {
    let trimmed = text.trim_start();
    let Some(split_index) = trimmed
        .char_indices()
        .find(|(_, ch)| ch.is_whitespace())
        .map(|(index, _)| index)
    else {
        return trimmed;
    };

    let token = &trimmed[..split_index];
    if is_heading_number_token(token) {
        trimmed[split_index..].trim_start()
    } else {
        trimmed
    }
}

pub(crate) fn is_heading_number_token(token: &str) -> bool {
    if !token
        .chars()
        .any(|ch| matches!(ch, '.' | '、' | ')' | '）'))
    {
        return false;
    }

    let number = token.trim_end_matches(['.', '、', ')', '）']);
    !number.is_empty()
        && number
            .split('.')
            .all(|part| !part.is_empty() && part.chars().all(|ch| ch.is_ascii_digit()))
}

pub(crate) fn parse_choice_option_line(line: &str) -> Option<ChoiceOptionData> {
    let line = normalize_choice_option_line(line);
    let mut chars = line.char_indices();
    let (_, first) = chars.next()?;
    if !first.is_ascii_alphanumeric() {
        return None;
    }
    let (delimiter_index, delimiter) = chars.next()?;
    if !matches!(delimiter, '.' | '、' | ')' | '）' | '．') {
        return None;
    }

    let label_start = delimiter_index + delimiter.len_utf8();
    let raw_label = line
        .get(label_start..)?
        .trim()
        .trim_start_matches('*')
        .trim_start_matches('_')
        .trim();
    if raw_label.is_empty() {
        return None;
    }

    let id = first.to_string().to_ascii_uppercase();
    Some(ChoiceOptionData {
        id: id.clone(),
        label: format!("{id}. {raw_label}"),
        description: None,
    })
}

pub(crate) fn normalize_choice_option_line(line: &str) -> String {
    let mut candidate = line.trim();
    if let Some(rest) = strip_markdown_list_marker(candidate) {
        candidate = rest;
    }
    candidate = candidate.trim_start();
    if let Some(rest) = candidate.strip_prefix("**") {
        candidate = rest;
    } else if let Some(rest) = candidate.strip_prefix("__") {
        candidate = rest;
    }
    candidate.trim_start().to_string()
}

pub(crate) fn strip_markdown_list_marker(line: &str) -> Option<&str> {
    let mut chars = line.char_indices();
    let (_, marker) = chars.next()?;
    if !matches!(marker, '-' | '*' | '+') {
        return None;
    }
    let (space_index, space) = chars.next()?;
    space
        .is_whitespace()
        .then(|| line[space_index + space.len_utf8()..].trim_start())
}
