use super::*;

pub(crate) struct CodeReviewProviderPayload {
    pub(crate) verdict: ReviewVerdict,
    pub(crate) summary: String,
    pub(crate) findings: Vec<ReviewFinding>,
    pub(crate) impact_scope: Vec<String>,
    pub(crate) pr_description: String,
    pub(crate) commit_message_suggestion: String,
    pub(crate) tested_evidence_refs: Vec<String>,
    pub(crate) diff_refs: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RawCodeReviewProviderPayload {
    pub(crate) verdict: ReviewVerdict,
    #[serde(default)]
    pub(crate) summary: String,
    #[serde(default)]
    pub(crate) findings: Vec<RawReviewFinding>,
    #[serde(default)]
    pub(crate) impact_scope: Vec<String>,
    #[serde(default)]
    pub(crate) pr_description: String,
    #[serde(default)]
    pub(crate) commit_message_suggestion: String,
    #[serde(default)]
    pub(crate) tested_evidence_refs: Vec<String>,
    #[serde(default)]
    pub(crate) diff_refs: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RawReviewFinding {
    #[serde(default)]
    pub(crate) severity: Option<crate::product::coding_models::FindingSeverity>,
    #[serde(default, alias = "file")]
    pub(crate) file_path: Option<String>,
    #[serde(default)]
    pub(crate) line: Option<u32>,
    #[serde(default, alias = "description", alias = "failure_scenario")]
    pub(crate) message: Option<String>,
    #[serde(default, alias = "recommendation", alias = "fix")]
    pub(crate) required_action: Option<String>,
    #[serde(default)]
    pub(crate) source_stage: Option<CodingExecutionStage>,
    #[serde(default)]
    pub(crate) title: Option<String>,
    #[serde(default)]
    pub(crate) evidence: Vec<String>,
    #[serde(default)]
    pub(crate) related_requirements: Vec<String>,
    #[serde(default)]
    pub(crate) related_design_constraints: Vec<String>,
    #[serde(default)]
    pub(crate) related_work_item_tasks: Vec<String>,
}

pub(crate) fn parse_review_payload(
    full_output: &str,
    default_source_stage: CodingExecutionStage,
) -> CodeReviewProviderPayload {
    let json = extract_json_object(full_output).unwrap_or(full_output);
    match serde_json::from_str::<RawCodeReviewProviderPayload>(json) {
        Ok(raw) => raw.into_payload(default_source_stage),
        Err(_) => blocked_review_payload(full_output),
    }
}

impl RawCodeReviewProviderPayload {
    pub(crate) fn into_payload(
        self,
        default_source_stage: CodingExecutionStage,
    ) -> CodeReviewProviderPayload {
        let verdict = self.verdict;
        CodeReviewProviderPayload {
            summary: non_empty_trimmed(&self.summary)
                .unwrap_or_else(|| default_review_summary(&verdict)),
            verdict,
            findings: self
                .findings
                .into_iter()
                .map(|finding| finding.into_review_finding(default_source_stage.clone()))
                .collect(),
            impact_scope: self.impact_scope,
            pr_description: self.pr_description,
            commit_message_suggestion: self.commit_message_suggestion,
            tested_evidence_refs: self.tested_evidence_refs,
            diff_refs: self.diff_refs,
        }
    }
}

impl RawReviewFinding {
    pub(crate) fn into_review_finding(
        self,
        default_source_stage: CodingExecutionStage,
    ) -> ReviewFinding {
        ReviewFinding {
            severity: self
                .severity
                .unwrap_or(crate::product::coding_models::FindingSeverity::Warning),
            file_path: self.file_path,
            line: self.line,
            message: self
                .message
                .or(self.title)
                .unwrap_or_else(|| "review finding".to_string()),
            required_action: self.required_action,
            source_stage: self.source_stage.unwrap_or(default_source_stage),
            evidence: self.evidence,
            related_requirements: self.related_requirements,
            related_design_constraints: self.related_design_constraints,
            related_work_item_tasks: self.related_work_item_tasks,
        }
    }
}

pub(crate) fn blocked_review_payload(full_output: &str) -> CodeReviewProviderPayload {
    CodeReviewProviderPayload {
        verdict: ReviewVerdict::Blocked,
        summary: format!(
            "review 输出不是有效 JSON，已阻塞并等待人工确认: {}",
            non_empty_trimmed(full_output).unwrap_or_else(|| "<empty>".to_string())
        ),
        findings: Vec::new(),
        impact_scope: Vec::new(),
        pr_description: String::new(),
        commit_message_suggestion: String::new(),
        tested_evidence_refs: Vec::new(),
        diff_refs: Vec::new(),
    }
}

pub(crate) fn default_review_summary(verdict: &ReviewVerdict) -> String {
    match verdict {
        ReviewVerdict::Approve => "review 通过".to_string(),
        ReviewVerdict::RequestChanges => "review 要求修改".to_string(),
        ReviewVerdict::Blocked => "review 被阻塞".to_string(),
    }
}

pub(crate) fn non_empty_trimmed(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

pub(crate) fn truncate_prompt_section(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut truncated: String = value.chars().take(max_chars).collect();
    truncated.push_str("\n[truncated]");
    truncated
}
