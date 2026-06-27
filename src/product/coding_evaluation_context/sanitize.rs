use super::{MAX_CONTEXT_SECTION_CHARS, MAX_DIFF_CONTEXT_CHARS};

pub(super) fn sanitize_context_text(input: &str) -> (String, bool) {
    let mut lines = Vec::new();
    let mut in_private_key_block = false;
    for line in input.lines() {
        let lower = line.to_ascii_lowercase();
        if in_private_key_block {
            if lower.contains("-----end") && lower.contains("private key") {
                in_private_key_block = false;
            }
            continue;
        }
        if lower.contains("-----begin") && lower.contains("private key") {
            lines.push("[REDACTED_PRIVATE_KEY]".to_string());
            in_private_key_block = true;
            continue;
        }
        if contains_sensitive_keyword(&lower) {
            lines.push("[REDACTED]".to_string());
        } else {
            lines.push(line.to_string());
        }
    }

    let sanitized = lines.join("\n");
    if sanitized.len() <= MAX_CONTEXT_SECTION_CHARS {
        return (sanitized, false);
    }
    (
        truncate_to_char_boundary(&sanitized, MAX_CONTEXT_SECTION_CHARS),
        true,
    )
}

fn contains_sensitive_keyword(lower_line: &str) -> bool {
    [
        "api_key",
        "token",
        "secret",
        "password",
        "authorization",
        "private key",
    ]
    .iter()
    .any(|keyword| lower_line.contains(keyword))
}

fn truncate_to_char_boundary(value: &str, max_len: usize) -> String {
    if value.len() <= max_len {
        return value.to_string();
    }
    let mut end = max_len;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

pub(super) fn push_warning_once(warnings: &mut Vec<String>, warning: &str) {
    if !warnings.iter().any(|existing| existing == warning) {
        warnings.push(warning.to_string());
    }
}

pub(super) fn sanitize_diff_text(input: &str) -> (String, bool) {
    let (sanitized, redaction_truncated) = sanitize_context_text(input);
    if sanitized.len() <= MAX_DIFF_CONTEXT_CHARS {
        return (sanitized, redaction_truncated);
    }
    (
        truncate_to_char_boundary(&sanitized, MAX_DIFF_CONTEXT_CHARS),
        true,
    )
}
