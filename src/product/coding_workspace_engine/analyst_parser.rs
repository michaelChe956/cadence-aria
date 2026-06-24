use super::*;

pub(crate) struct AnalystDecision {
    pub(crate) verdict: AnalystVerdict,
    pub(crate) structured_verdict: AnalystDecisionVerdict,
    pub(crate) next_stage: Option<AnalystDecisionNextStage>,
    pub(crate) summary: String,
    pub(crate) reason: String,
    pub(crate) evidence_refs: Vec<String>,
    pub(crate) raw_provider_output_refs: Vec<String>,
    pub(crate) rework_instructions: Option<AnalystReworkInstructions>,
    pub(crate) human_gate: Option<AnalystHumanGateRecommendation>,
    pub(crate) fix_hints: Vec<String>,
    pub(crate) questions: Vec<String>,
    pub(crate) parse_error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AnalystProviderPayload {
    pub(crate) verdict: AnalystProviderVerdict,
    #[serde(default)]
    pub(crate) next_stage: Option<AnalystDecisionNextStage>,
    #[serde(default)]
    pub(crate) reason: Option<String>,
    #[serde(default)]
    pub(crate) summary: Option<String>,
    #[serde(default)]
    pub(crate) evidence_refs: Vec<String>,
    #[serde(default)]
    pub(crate) raw_provider_output_refs: Vec<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_analyst_rework_instructions"
    )]
    pub(crate) rework_instructions: Option<AnalystReworkInstructions>,
    #[serde(default)]
    pub(crate) human_gate: Option<AnalystHumanGateRecommendation>,
    #[serde(default)]
    pub(crate) fix_hints: Vec<String>,
    #[serde(default)]
    pub(crate) questions: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum AnalystReworkInstructionsInput {
    Structured(AnalystReworkInstructions),
    Summary(String),
}

pub(crate) fn deserialize_optional_analyst_rework_instructions<'de, D>(
    deserializer: D,
) -> Result<Option<AnalystReworkInstructions>, D::Error>
where
    D: Deserializer<'de>,
{
    let Some(input) = Option::<AnalystReworkInstructionsInput>::deserialize(deserializer)? else {
        return Ok(None);
    };
    match input {
        AnalystReworkInstructionsInput::Structured(instructions) => Ok(Some(instructions)),
        AnalystReworkInstructionsInput::Summary(summary) => {
            let summary = summary.trim();
            if summary.is_empty() {
                Ok(None)
            } else {
                Ok(Some(AnalystReworkInstructions {
                    summary: summary.to_string(),
                    required_changes: vec![summary.to_string()],
                    verification_expectations: Vec::new(),
                }))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AnalystProviderVerdict {
    NeedsFix,
    NeedsHumanInput,
    NoIssue,
    RerunTesting,
    Proceed,
    HumanRequired,
    Blocked,
}

impl AnalystProviderVerdict {
    pub(crate) fn structured(&self) -> AnalystDecisionVerdict {
        match self {
            Self::NeedsFix => AnalystDecisionVerdict::NeedsFix,
            Self::NeedsHumanInput => AnalystDecisionVerdict::HumanRequired,
            Self::NoIssue => AnalystDecisionVerdict::Proceed,
            Self::RerunTesting => AnalystDecisionVerdict::RerunTesting,
            Self::Proceed => AnalystDecisionVerdict::Proceed,
            Self::HumanRequired => AnalystDecisionVerdict::HumanRequired,
            Self::Blocked => AnalystDecisionVerdict::Blocked,
        }
    }
}

pub(crate) fn default_next_stage_for_legacy_verdict(
    verdict: &AnalystDecisionVerdict,
    source_stage: &CodingExecutionStage,
) -> AnalystDecisionNextStage {
    match verdict {
        AnalystDecisionVerdict::NeedsFix => AnalystDecisionNextStage::Coding,
        AnalystDecisionVerdict::RerunTesting => AnalystDecisionNextStage::Testing,
        AnalystDecisionVerdict::HumanRequired | AnalystDecisionVerdict::Blocked => {
            AnalystDecisionNextStage::HumanGate
        }
        AnalystDecisionVerdict::Proceed => match source_stage {
            CodingExecutionStage::Testing => AnalystDecisionNextStage::CodeReview,
            CodingExecutionStage::CodeReview => AnalystDecisionNextStage::ReviewRequest,
            CodingExecutionStage::InternalPrReview => AnalystDecisionNextStage::FinalConfirm,
            _ => AnalystDecisionNextStage::CodeReview,
        },
    }
}

pub(crate) fn decision_reason(summary: &str, reason: Option<&str>) -> String {
    reason
        .and_then(non_empty_trimmed)
        .unwrap_or_else(|| summary.to_string())
}

pub(crate) fn parse_analyst_verdict(
    full_output: &str,
    source_stage: &CodingExecutionStage,
) -> AnalystDecision {
    let Some(json_text) = extract_json_object(full_output) else {
        let summary = "Analyst 输出不是有效 JSON，已转人工确认。".to_string();
        return AnalystDecision {
            verdict: AnalystVerdict::NeedsHumanInput,
            structured_verdict: AnalystDecisionVerdict::HumanRequired,
            next_stage: Some(AnalystDecisionNextStage::HumanGate),
            summary: summary.clone(),
            reason: summary,
            evidence_refs: Vec::new(),
            raw_provider_output_refs: Vec::new(),
            rework_instructions: None,
            human_gate: None,
            fix_hints: Vec::new(),
            questions: vec!["请人工确认 Analyst 输出并补充下一步处理意见。".to_string()],
            parse_error: Some("missing_json_object".to_string()),
        };
    };

    match serde_json::from_str::<AnalystProviderPayload>(json_text) {
        Ok(payload) => {
            let structured_verdict = payload.verdict.structured();
            let summary = payload
                .summary
                .as_deref()
                .and_then(non_empty_trimmed)
                .or_else(|| {
                    payload
                        .rework_instructions
                        .as_ref()
                        .and_then(|instruction| non_empty_trimmed(&instruction.summary))
                })
                .unwrap_or_else(|| default_analyst_decision_summary(&structured_verdict));
            let next_stage = payload.next_stage.unwrap_or_else(|| {
                default_next_stage_for_legacy_verdict(&structured_verdict, source_stage)
            });
            let reason = decision_reason(&summary, payload.reason.as_deref());
            AnalystDecision {
                verdict: structured_verdict.legacy_chat_verdict(),
                structured_verdict,
                next_stage: Some(next_stage),
                summary,
                reason,
                evidence_refs: payload.evidence_refs,
                raw_provider_output_refs: payload.raw_provider_output_refs,
                rework_instructions: payload.rework_instructions,
                human_gate: payload.human_gate,
                fix_hints: payload.fix_hints,
                questions: payload.questions,
                parse_error: None,
            }
        }
        Err(error) => {
            let summary = "Analyst 输出不是有效 JSON，已转人工确认。".to_string();
            AnalystDecision {
                verdict: AnalystVerdict::NeedsHumanInput,
                structured_verdict: AnalystDecisionVerdict::HumanRequired,
                next_stage: Some(AnalystDecisionNextStage::HumanGate),
                summary: summary.clone(),
                reason: summary,
                evidence_refs: Vec::new(),
                raw_provider_output_refs: Vec::new(),
                rework_instructions: None,
                human_gate: None,
                fix_hints: Vec::new(),
                questions: vec!["请人工确认 Analyst 输出并补充下一步处理意见。".to_string()],
                parse_error: Some(error.to_string()),
            }
        }
    }
}

pub(crate) fn extract_json_object(value: &str) -> Option<&str> {
    let start = value.find('{')?;
    let end = value.rfind('}')?;
    (start <= end).then(|| &value[start..=end])
}

pub(crate) fn default_analyst_decision_summary(verdict: &AnalystDecisionVerdict) -> String {
    match verdict {
        AnalystDecisionVerdict::NeedsFix => "Analyst 判定需要自动修复".to_string(),
        AnalystDecisionVerdict::RerunTesting => "Analyst 判定需要重跑测试".to_string(),
        AnalystDecisionVerdict::Proceed => "Analyst 未发现阻塞问题".to_string(),
        AnalystDecisionVerdict::HumanRequired => "Analyst 判定需要人工补充信息".to_string(),
        AnalystDecisionVerdict::Blocked => "Analyst 判定当前流程被阻塞".to_string(),
    }
}
