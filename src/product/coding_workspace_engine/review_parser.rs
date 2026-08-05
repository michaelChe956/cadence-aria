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
    #[serde(default, deserialize_with = "deserialize_review_finding_source_stage")]
    pub(crate) source_stage: Option<CodingExecutionStage>,
    #[serde(default)]
    pub(crate) title: Option<String>,
    #[serde(default)]
    pub(crate) evidence: Vec<RawReviewEvidence>,
    #[serde(default)]
    pub(crate) related_requirements: Vec<String>,
    #[serde(default)]
    pub(crate) related_design_constraints: Vec<String>,
    #[serde(default)]
    pub(crate) related_work_item_tasks: Vec<String>,
    #[serde(default)]
    pub(crate) defect_class: Option<crate::product::models::PlanDefectClass>,
    #[serde(default)]
    pub(crate) reason_code: Option<String>,
    #[serde(default)]
    pub(crate) contract_refs: Vec<String>,
    #[serde(default)]
    pub(crate) capability_refs: Vec<String>,
    #[serde(default)]
    pub(crate) repair_target: Option<crate::product::models::RepairTarget>,
    #[serde(default)]
    pub(crate) recommended_route: Option<crate::product::models::PlanDefectRoute>,
    #[serde(default)]
    pub(crate) confidence: Option<crate::product::plan_repair::PlanDefectConfidence>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum RawReviewEvidence {
    Reference(String),
    Canonical(crate::product::models::PlanDefectEvidence),
}

fn deserialize_review_finding_source_stage<'de, D>(
    deserializer: D,
) -> Result<Option<CodingExecutionStage>, D::Error>
where
    D: Deserializer<'de>,
{
    let Some(value) = Option::<String>::deserialize(deserializer)? else {
        return Ok(None);
    };
    match value.trim() {
        "prepare_context" => Ok(Some(CodingExecutionStage::PrepareContext)),
        "worktree_prepare" => Ok(Some(CodingExecutionStage::WorktreePrepare)),
        "coding" => Ok(Some(CodingExecutionStage::Coding)),
        "code_review" => Ok(Some(CodingExecutionStage::CodeReview)),
        "review_request" => Ok(Some(CodingExecutionStage::ReviewRequest)),
        "internal_pr_review" | "group_final_review" => {
            Ok(Some(CodingExecutionStage::InternalPrReview))
        }
        "final_confirm" => Ok(Some(CodingExecutionStage::FinalConfirm)),
        other => Err(serde::de::Error::unknown_variant(
            other,
            &[
                "prepare_context",
                "worktree_prepare",
                "coding",
                "code_review",
                "review_request",
                "internal_pr_review",
                "group_final_review",
                "final_confirm",
            ],
        )),
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum GroupReviewParseError {
    #[error("group_review_parse: {0}")]
    Json(#[from] serde_json::Error),
}

/// 组级审查专用的严格解析入口。
///
/// 普通 review 路径保留 `parse_review_payload` 的 blocked 合成行为；组级路径
/// 必须把 provider 输出解析失败显式暴露给编排器，以便先做一次受限修复。
pub(crate) fn parse_group_review_payload(
    full_output: &str,
    default_source_stage: CodingExecutionStage,
) -> Result<CodeReviewProviderPayload, GroupReviewParseError> {
    let candidates = extract_json_object_candidates(full_output);
    if candidates.is_empty() {
        return serde_json::from_str::<RawCodeReviewProviderPayload>(full_output)
            .map(|raw| raw.into_payload(default_source_stage))
            .map_err(GroupReviewParseError::Json);
    }
    let mut last_error = None;
    for json in candidates {
        match serde_json::from_str::<RawCodeReviewProviderPayload>(json) {
            Ok(raw) => return Ok(raw.into_payload(default_source_stage)),
            Err(error) => last_error = Some(error),
        }
    }
    Err(GroupReviewParseError::Json(
        last_error.expect("candidates 非空时必然记录过反序列化错误"),
    ))
}
pub(crate) fn parse_review_payload(
    full_output: &str,
    default_source_stage: CodingExecutionStage,
) -> CodeReviewProviderPayload {
    let candidates = extract_json_object_candidates(full_output);
    if candidates.is_empty() {
        // 没有任何平衡 JSON 对象时保持原有诊断行为：按整体输出解析以区分语法错误。
        return match serde_json::from_str::<RawCodeReviewProviderPayload>(full_output) {
            Ok(raw) => raw.into_payload(default_source_stage),
            Err(error) => blocked_review_payload(full_output, &error),
        };
    }
    // 输出可能夹带与结论无关的 JSON 片段；逐个候选尝试 Schema 校验，
    // 取第一个通过校验的对象，全部失败才按最后一个候选（契约要求结论位于末尾）报错。
    let mut last_error = None;
    for json in candidates {
        match serde_json::from_str::<RawCodeReviewProviderPayload>(json) {
            Ok(raw) => return raw.into_payload(default_source_stage),
            Err(error) => last_error = Some(error),
        }
    }
    let error = last_error.expect("candidates 非空时必然记录过反序列化错误");
    blocked_review_payload(full_output, &error)
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
        let mut evidence = Vec::new();
        let mut plan_defect_evidence = Vec::new();
        for item in self.evidence {
            match item {
                RawReviewEvidence::Reference(reference) => evidence.push(reference),
                RawReviewEvidence::Canonical(canonical) => plan_defect_evidence.push(canonical),
            }
        }
        let defect_class = self
            .defect_class
            .unwrap_or(crate::product::models::PlanDefectClass::ImplementationDefect);
        let recommended_route = self.recommended_route.unwrap_or_else(|| {
            if defect_class == crate::product::models::PlanDefectClass::ImplementationDefect {
                crate::product::models::PlanDefectRoute::CoderRework
            } else {
                crate::product::models::PlanDefectRoute::HumanTriage
            }
        });
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
            evidence,
            plan_defect_evidence,
            related_requirements: self.related_requirements,
            related_design_constraints: self.related_design_constraints,
            related_work_item_tasks: self.related_work_item_tasks,
            defect_class,
            reason_code: self.reason_code,
            contract_refs: self.contract_refs,
            capability_refs: self.capability_refs,
            repair_target: self.repair_target,
            recommended_route,
            confidence: self.confidence,
        }
    }
}

pub(crate) fn blocked_review_payload(
    full_output: &str,
    error: &serde_json::Error,
) -> CodeReviewProviderPayload {
    let prefix = match error.classify() {
        serde_json::error::Category::Syntax | serde_json::error::Category::Eof => {
            "review 输出不是有效 JSON"
        }
        serde_json::error::Category::Data => "review JSON Schema 校验失败",
        serde_json::error::Category::Io => "review JSON 解析失败",
    };
    CodeReviewProviderPayload {
        verdict: ReviewVerdict::Blocked,
        summary: format!(
            "{prefix}，已阻塞并等待人工确认: {error}; 原始输出: {}",
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
