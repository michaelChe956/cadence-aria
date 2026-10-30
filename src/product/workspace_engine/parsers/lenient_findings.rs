use super::*;

/// REQ-WSC-11 相关改动（F1 下界）落在此文件；宽松 findings 解析族独立成模块，
/// 让 `parsers.rs` 保持行数门禁余量。
pub(crate) fn parse_review_findings(value: Option<&serde_json::Value>) -> ParsedReviewFindings {
    let Some(value) = value else {
        return ParsedReviewFindings {
            findings: Vec::new(),
            malformed: false,
            structured_error: None,
        };
    };
    let Some(items) = value.as_array() else {
        return ParsedReviewFindings {
            findings: Vec::new(),
            malformed: true,
            structured_error: None,
        };
    };

    let mut findings = Vec::new();
    let mut malformed = false;
    for (index, item) in items.iter().enumerate() {
        let Some(object) = item.as_object() else {
            malformed = true;
            continue;
        };
        if object.contains_key("impact") {
            malformed = true;
            continue;
        }
        let Some(severity) = object
            .get("severity")
            .and_then(serde_json::Value::as_str)
            .and_then(parse_live_review_finding_severity)
        else {
            malformed = true;
            continue;
        };
        let Some(message) = object.get("message").and_then(serde_json::Value::as_str) else {
            malformed = true;
            continue;
        };
        let prefix = format!("findings[{index}]");
        let category = match parse_optional_finding_category(object, &prefix) {
            Ok(category) => category,
            Err(error) => {
                return ParsedReviewFindings {
                    findings: Vec::new(),
                    malformed: false,
                    structured_error: Some(error),
                };
            }
        };
        let class_hint = match parse_optional_finding_class_hint(object, &prefix) {
            Ok(class_hint) => class_hint,
            Err(error) => {
                return ParsedReviewFindings {
                    findings: Vec::new(),
                    malformed: false,
                    structured_error: Some(error),
                };
            }
        };
        let contract_field = match parse_optional_contract_field(object, &prefix) {
            Ok(contract_field) => contract_field,
            Err(error) => {
                return ParsedReviewFindings {
                    findings: Vec::new(),
                    malformed: false,
                    structured_error: Some(error),
                };
            }
        };
        let owner_question = match parse_optional_owner_question(object, &prefix) {
            Ok(owner_question) => owner_question,
            Err(error) => {
                return ParsedReviewFindings {
                    findings: Vec::new(),
                    malformed: false,
                    structured_error: Some(error),
                };
            }
        };
        let owner_options = match parse_optional_owner_options(object, &prefix) {
            Ok(owner_options) => owner_options,
            Err(error) => {
                return ParsedReviewFindings {
                    findings: Vec::new(),
                    malformed: false,
                    structured_error: Some(error),
                };
            }
        };

        findings.push(ReviewFinding {
            severity,
            message: message.to_string(),
            evidence: object
                .get("evidence")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string(),
            required_action: object
                .get("required_action")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string(),
            category,
            class_hint,
            contract_field,
            // SG7（§0.6.3）：Raw 宽松入口对伪造 machine_repair 一律丢弃，
            // 不从 provider 输入建立修复许可。
            machine_repair: None,
            owner_question,
            owner_options,
        });
    }

    ParsedReviewFindings {
        findings,
        malformed,
        structured_error: None,
    }
}

pub(crate) fn parse_optional_finding_category(
    object: &serde_json::Map<String, serde_json::Value>,
    prefix: &str,
) -> Result<
    Option<crate::product::work_item_plan_policy::ReviewFindingCategory>,
    ReviewStructuredOutputError,
> {
    match object.get("category") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(value)) => parse_finding_category(value).map(Some),
        Some(_) => Err(invalid_finding_field(
            format!("{prefix}.category"),
            "must be a string or null",
        )),
    }
}

pub(crate) fn parse_optional_finding_class_hint(
    object: &serde_json::Map<String, serde_json::Value>,
    prefix: &str,
) -> Result<
    Option<crate::product::work_item_plan_policy::FindingClassHint>,
    ReviewStructuredOutputError,
> {
    match object.get("class_hint") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(value)) => parse_finding_class_hint(value).map(Some),
        Some(_) => Err(invalid_finding_field(
            format!("{prefix}.class_hint"),
            "must be a string or null",
        )),
    }
}

pub(crate) fn parse_optional_contract_field(
    object: &serde_json::Map<String, serde_json::Value>,
    prefix: &str,
) -> Result<Option<String>, ReviewStructuredOutputError> {
    match object.get("contract_field") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(invalid_finding_field(
            format!("{prefix}.contract_field"),
            "must be a string or null",
        )),
    }
}
