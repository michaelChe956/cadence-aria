use std::path::{Component, Path};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestCommandSpec {
    pub id: String,
    pub command: Vec<String>,
}

pub fn planned_test_commands_from_markdown(markdown: &str) -> Vec<TestCommandSpec> {
    let mut commands = Vec::new();
    let mut in_verification_block = false;
    let mut in_command_fence: Option<String> = None;
    for line in markdown.lines() {
        let trimmed = line.trim();
        if let Some(fence) = in_command_fence.as_deref() {
            if is_closing_fence(trimmed, fence) {
                in_command_fence = None;
                continue;
            }
            let command = trimmed.strip_prefix("$ ").unwrap_or(trimmed).to_string();
            if let Some(command) = normalize_planned_command(&command)
                && !commands.contains(&command)
            {
                commands.push(command);
            }
            continue;
        }
        if trimmed.starts_with('#') {
            in_verification_block = trimmed.contains("验证命令");
            continue;
        }
        if is_verification_label(trimmed) {
            in_verification_block = true;
            continue;
        }
        if in_verification_block && is_non_verification_label(trimmed) {
            in_verification_block = false;
            continue;
        }
        if !in_verification_block {
            continue;
        }
        if let Some(fence) = code_fence_marker(trimmed) {
            in_command_fence = Some(fence);
            continue;
        }
        for command in inline_code_spans(trimmed) {
            let Some(command) = normalize_planned_command(&command) else {
                continue;
            };
            if commands.contains(&command) {
                continue;
            }
            commands.push(command);
        }
    }

    commands
        .into_iter()
        .enumerate()
        .map(|(index, command)| TestCommandSpec {
            id: format!("planned_{:03}", index + 1),
            command,
        })
        .collect()
}

fn is_verification_label(line: &str) -> bool {
    line.starts_with("验证命令")
        || line.starts_with("主验证命令")
        || line.starts_with("辅助检查命令")
}

fn is_non_verification_label(line: &str) -> bool {
    line.ends_with('：')
        && !line.starts_with('-')
        && !line.starts_with('*')
        && !line.contains("命令")
        && !is_verification_label(line)
}

fn inline_code_spans(line: &str) -> Vec<String> {
    let mut spans = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find('`') {
        let after_start = &rest[start + 1..];
        let Some(end) = after_start.find('`') else {
            break;
        };
        let value = after_start[..end].trim();
        if !value.is_empty() {
            spans.push(value.to_string());
        }
        rest = &after_start[end + 1..];
    }
    spans
}

fn normalize_planned_command(command: &str) -> Option<Vec<String>> {
    let parts = split_simple_command(command)?;

    if let Some(parts) = normalize_cd_pnpm_command(&parts) {
        return Some(parts);
    }

    allowed_planned_command_parts(&parts).then_some(parts)
}

fn normalize_cd_pnpm_command(parts: &[String]) -> Option<Vec<String>> {
    if parts.len() < 4
        || parts.first().map(String::as_str) != Some("cd")
        || parts.get(2).map(String::as_str) != Some("&&")
        || parts.get(3).map(String::as_str) != Some("pnpm")
    {
        return None;
    }
    let package_dir = parts.get(1)?;
    if !is_safe_package_dir_argument(package_dir) {
        return None;
    }
    let mut normalized = vec![
        "pnpm".to_string(),
        "-C".to_string(),
        package_dir.to_string(),
    ];
    normalized.extend(parts.iter().skip(4).cloned());
    allowed_planned_command_parts(&normalized).then_some(normalized)
}

fn allowed_planned_command_parts(parts: &[String]) -> bool {
    match parts.first().map(String::as_str) {
        Some("cargo" | "uv" | "pnpm" | "node" | "python" | "python3" | "pytest") => true,
        Some("git") => parts.get(1).is_some_and(|subcommand| subcommand == "diff"),
        _ => false,
    }
}

fn is_safe_package_dir_argument(value: &str) -> bool {
    let path = Path::new(value);
    !value.is_empty()
        && !value.starts_with('-')
        && path.is_relative()
        && path.components().all(|component| match component {
            Component::Normal(value) => value
                .to_string_lossy()
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.')),
            _ => false,
        })
}

fn split_simple_command(command: &str) -> Option<Vec<String>> {
    let parts: Vec<String> = command
        .split_whitespace()
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToString::to_string)
        .collect();
    (!parts.is_empty()).then_some(parts)
}

fn code_fence_marker(line: &str) -> Option<String> {
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

fn is_closing_fence(line: &str, fence: &str) -> bool {
    line.starts_with(fence) && line[fence.len()..].trim().is_empty()
}
