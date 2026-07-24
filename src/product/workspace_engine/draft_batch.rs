use super::*;

mod authoring;
mod compile_support;
mod decisions;
mod runs;

pub(crate) fn combine_draft_validation_feedback(
    user_feedback: Option<&str>,
    findings: &[ValidatorFindingDto],
) -> String {
    let mut parts = Vec::new();
    if let Some(feedback) = user_feedback
        .map(str::trim)
        .filter(|feedback| !feedback.is_empty())
    {
        parts.push(feedback.to_string());
    }
    if !findings.is_empty() {
        let mut lines: Vec<String> = findings
            .iter()
            .map(|finding| format!("{}: {}", finding.code, finding.message))
            .collect();
        lines.sort();
        parts.push(format!("[draft_validation_findings]\n{}", lines.join("\n")));
    }
    parts.join("\n\n")
}
