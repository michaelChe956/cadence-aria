use super::*;

mod authoring;
pub(crate) mod compile_support;
mod decisions;
mod runs;

pub(crate) fn combine_draft_validation_feedback(
    user_feedback: Option<&str>,
    findings: &[ValidatorFindingDto],
) -> String {
    let mut parts = Vec::new();
    if let Some(feedback) = user_feedback.filter(|feedback| !feedback.trim().is_empty()) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combine_draft_validation_feedback_preserves_nonempty_user_text_verbatim() {
        let feedback = combine_draft_validation_feedback(
            Some("  保留首尾空白  "),
            &[
                ValidatorFindingDto {
                    severity: "error".to_string(),
                    code: "z_code".to_string(),
                    message: "z message".to_string(),
                    work_item_ids: vec![],
                },
                ValidatorFindingDto {
                    severity: "error".to_string(),
                    code: "a_code".to_string(),
                    message: "a message".to_string(),
                    work_item_ids: vec![],
                },
            ],
        );

        assert_eq!(
            feedback,
            "  保留首尾空白  \n\n[draft_validation_findings]\na_code: a message\nz_code: z message"
        );
    }
}
