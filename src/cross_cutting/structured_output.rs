use serde::{Deserialize, Serialize};
use serde_json::Value;

const START_PREFIX: &str = "<ARIA_STRUCTURED_OUTPUT";
const END_PREFIX: &str = "</ARIA_STRUCTURED_OUTPUT";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredOutputContract {
    pub nonce: String,
    pub schema_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructuredOutputState {
    NotRequested,
    Parsed(Value),
    Failed(StructuredOutputError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredOutputError {
    pub code: StructuredOutputErrorCode,
    pub message: String,
    pub expected_nonce: Option<String>,
    pub observed_nonce: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recoverable_value: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuredOutputErrorCode {
    MissingStartTag,
    MissingEndTag,
    MissingEndNonce,
    NonceMismatch,
    InvalidJson,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredOutputParse {
    pub readable_output: String,
    pub state: StructuredOutputState,
}

impl StructuredOutputErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::MissingStartTag => "missing_start_tag",
            Self::MissingEndTag => "missing_end_tag",
            Self::MissingEndNonce => "missing_end_nonce",
            Self::NonceMismatch => "nonce_mismatch",
            Self::InvalidJson => "invalid_json",
        }
    }
}

pub fn parse_structured_output(
    output: &str,
    contract: &StructuredOutputContract,
) -> StructuredOutputParse {
    let expected_start = format!("{START_PREFIX} nonce=\"{}\">", contract.nonce);
    let Some(start) = output.rfind(&expected_start) else {
        return failed(
            output,
            None,
            StructuredOutputErrorCode::MissingStartTag,
            "missing structured output start tag for expected nonce",
            Some(contract.nonce.clone()),
            None,
            None,
        );
    };

    parse_block_at(output, start, Some(contract.nonce.as_str()))
}

pub fn parse_last_structured_output_value(
    output: &str,
) -> Result<Option<Value>, StructuredOutputError> {
    let Some(start) = output.rfind(START_PREFIX) else {
        return Ok(None);
    };
    let (nonce, _) = parse_start_tag(output, start, None)?;
    let parsed = parse_block_at(output, start, nonce.as_deref());

    match parsed.state {
        StructuredOutputState::Parsed(value) => Ok(Some(value)),
        StructuredOutputState::Failed(error) => Err(error),
        StructuredOutputState::NotRequested => Ok(None),
    }
}

fn parse_block_at(
    output: &str,
    start_index: usize,
    expected_nonce: Option<&str>,
) -> StructuredOutputParse {
    let (start_nonce, json_start) = match parse_start_tag(output, start_index, expected_nonce) {
        Ok(tag) => tag,
        Err(error) => {
            return StructuredOutputParse {
                readable_output: output[..start_index].trim().to_string(),
                state: StructuredOutputState::Failed(error),
            };
        }
    };
    if start_nonce.as_deref() != expected_nonce {
        return failed(
            output,
            Some(start_index),
            StructuredOutputErrorCode::NonceMismatch,
            "structured output start nonce mismatch",
            expected_nonce.map(str::to_string),
            start_nonce,
            None,
        );
    }

    let after_start = &output[json_start..];
    let Some(end_relative) = after_start.find(END_PREFIX) else {
        return failed(
            output,
            None,
            StructuredOutputErrorCode::MissingEndTag,
            "missing structured output end tag",
            expected_nonce.map(str::to_string),
            None,
            parse_json_candidate(after_start),
        );
    };
    let json_text = after_start[..end_relative].trim();
    let after_end_prefix = &after_start[end_relative + END_PREFIX.len()..];
    let Some(tag_close) = after_end_prefix.find('>') else {
        return failed(
            output,
            None,
            StructuredOutputErrorCode::MissingEndTag,
            "structured output end tag is not closed",
            expected_nonce.map(str::to_string),
            None,
            parse_json_candidate(json_text),
        );
    };

    let attrs = after_end_prefix[..tag_close].trim();
    let end_tag_len = END_PREFIX.len() + tag_close + 1;
    let block_end = json_start + end_relative + end_tag_len;
    let readable_output = format!("{}{}", &output[..start_index], &output[block_end..])
        .trim()
        .to_string();

    if attrs.is_empty() && expected_nonce.is_some() {
        return StructuredOutputParse {
            readable_output,
            state: StructuredOutputState::Failed(StructuredOutputError {
                code: StructuredOutputErrorCode::MissingEndNonce,
                message: "structured output end tag is missing nonce".to_string(),
                expected_nonce: expected_nonce.map(str::to_string),
                observed_nonce: None,
                recoverable_value: parse_json_candidate(json_text),
            }),
        };
    }

    let observed_nonce = match parse_nonce(attrs) {
        Ok(nonce) => nonce,
        Err(_) => {
            return StructuredOutputParse {
                readable_output,
                state: StructuredOutputState::Failed(StructuredOutputError {
                    code: StructuredOutputErrorCode::NonceMismatch,
                    message: "structured output nonce mismatch".to_string(),
                    expected_nonce: expected_nonce.map(str::to_string),
                    observed_nonce: None,
                    recoverable_value: parse_json_candidate(json_text),
                }),
            };
        }
    };
    if observed_nonce.as_deref() != expected_nonce {
        return StructuredOutputParse {
            readable_output,
            state: StructuredOutputState::Failed(StructuredOutputError {
                code: StructuredOutputErrorCode::NonceMismatch,
                message: "structured output nonce mismatch".to_string(),
                expected_nonce: expected_nonce.map(str::to_string),
                observed_nonce,
                recoverable_value: parse_json_candidate(json_text),
            }),
        };
    }

    match parse_json_candidate(json_text) {
        Some(value) => StructuredOutputParse {
            readable_output,
            state: StructuredOutputState::Parsed(value),
        },
        None => StructuredOutputParse {
            readable_output,
            state: StructuredOutputState::Failed(StructuredOutputError {
                code: StructuredOutputErrorCode::InvalidJson,
                message: "invalid structured output json".to_string(),
                expected_nonce: expected_nonce.map(str::to_string),
                observed_nonce,
                recoverable_value: None,
            }),
        },
    }
}

fn parse_start_tag(
    output: &str,
    start_index: usize,
    expected_nonce: Option<&str>,
) -> Result<(Option<String>, usize), StructuredOutputError> {
    let after_start_prefix = &output[start_index + START_PREFIX.len()..];
    let Some(tag_close) = after_start_prefix.find('>') else {
        return Err(StructuredOutputError {
            code: StructuredOutputErrorCode::MissingStartTag,
            message: "structured output start tag is not closed".to_string(),
            expected_nonce: expected_nonce.map(str::to_string),
            observed_nonce: None,
            recoverable_value: None,
        });
    };
    let attrs = after_start_prefix[..tag_close].trim();
    let nonce = parse_nonce(attrs).map_err(|message| StructuredOutputError {
        code: StructuredOutputErrorCode::MissingStartTag,
        message: format!("structured output start tag {message}"),
        expected_nonce: expected_nonce.map(str::to_string),
        observed_nonce: None,
        recoverable_value: None,
    })?;
    Ok((nonce, start_index + START_PREFIX.len() + tag_close + 1))
}

fn parse_nonce(attrs: &str) -> Result<Option<String>, &'static str> {
    if attrs.is_empty() {
        return Ok(None);
    }
    let Some(nonce) = attrs
        .strip_prefix("nonce=\"")
        .and_then(|value| value.strip_suffix('"'))
    else {
        return Err("has unsupported attributes");
    };
    if nonce.len() != 8 || !nonce.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        return Err("has invalid nonce");
    }
    Ok(Some(nonce.to_string()))
}

fn failed(
    output: &str,
    start_index: Option<usize>,
    code: StructuredOutputErrorCode,
    message: &str,
    expected_nonce: Option<String>,
    observed_nonce: Option<String>,
    recoverable_value: Option<Value>,
) -> StructuredOutputParse {
    let readable_output = start_index
        .map(|start| &output[..start])
        .unwrap_or(output)
        .trim()
        .to_string();
    StructuredOutputParse {
        readable_output,
        state: StructuredOutputState::Failed(StructuredOutputError {
            code,
            message: message.to_string(),
            expected_nonce,
            observed_nonce,
            recoverable_value,
        }),
    }
}

fn parse_json_candidate(text: &str) -> Option<Value> {
    serde_json::from_str(text).ok().or_else(|| {
        let candidate = extract_json_candidate(text)?;
        serde_json::from_str(candidate).ok()
    })
}

fn extract_json_candidate(text: &str) -> Option<&str> {
    let start = text.find(['{', '['])?;
    let close = match text.as_bytes()[start] {
        b'{' => '}',
        b'[' => ']',
        _ => return None,
    };
    let end = text.rfind(close)?;
    (end >= start).then_some(&text[start..=end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn contract() -> StructuredOutputContract {
        StructuredOutputContract {
            nonce: "96aca42f".to_string(),
            schema_name: "workspace_review".to_string(),
        }
    }

    #[test]
    fn parses_matching_nonce_and_removes_structured_block_from_readable_output() {
        let output = "审核说明\n<ARIA_STRUCTURED_OUTPUT nonce=\"96aca42f\">\n{\"verdict\":\"pass\"}\n</ARIA_STRUCTURED_OUTPUT nonce=\"96aca42f\">";

        let parsed = parse_structured_output(output, &contract());

        assert_eq!(parsed.readable_output, "审核说明");
        assert_eq!(
            parsed.state,
            StructuredOutputState::Parsed(json!({"verdict": "pass"}))
        );
    }

    #[test]
    fn classifies_missing_start_tag_without_recoverable_value() {
        let output = "审核说明\n{\"verdict\":\"pass\"}";

        let parsed = parse_structured_output(output, &contract());

        let StructuredOutputState::Failed(error) = parsed.state else {
            panic!("expected structured output failure");
        };
        assert_eq!(error.code, StructuredOutputErrorCode::MissingStartTag);
        assert!(error.recoverable_value.is_none());
        assert_eq!(parsed.readable_output, output);
    }

    #[test]
    fn classifies_missing_end_tag_and_keeps_recoverable_value() {
        let output =
            "审核说明\n<ARIA_STRUCTURED_OUTPUT nonce=\"96aca42f\">\n{\"verdict\":\"revise\"}";

        let parsed = parse_structured_output(output, &contract());

        let StructuredOutputState::Failed(error) = parsed.state else {
            panic!("expected structured output failure");
        };
        assert_eq!(error.code, StructuredOutputErrorCode::MissingEndTag);
        assert_eq!(error.recoverable_value, Some(json!({"verdict": "revise"})));
        assert_eq!(parsed.readable_output, output);
    }

    #[test]
    fn missing_or_unclosed_end_boundary_keeps_full_readable_output() {
        for output in [
            "审核说明\n<ARIA_STRUCTURED_OUTPUT nonce=\"96aca42f\">\n{\"verdict\":\"revise\"}\nReviewer 尾随说明仍需展示",
            "审核说明\n<ARIA_STRUCTURED_OUTPUT nonce=\"96aca42f\">\n{\"verdict\":\"revise\"}\n</ARIA_STRUCTURED_OUTPUT nonce=\"96aca42f\"\nReviewer 尾随说明仍需展示",
        ] {
            let parsed = parse_structured_output(output, &contract());

            let StructuredOutputState::Failed(error) = parsed.state else {
                panic!("expected structured output failure");
            };
            assert_eq!(error.code, StructuredOutputErrorCode::MissingEndTag);
            assert_eq!(error.recoverable_value, Some(json!({"verdict": "revise"})));
            assert_eq!(parsed.readable_output, output);
        }
    }

    #[test]
    fn classifies_missing_end_nonce_and_keeps_recoverable_value() {
        let output = "审核说明\n<ARIA_STRUCTURED_OUTPUT nonce=\"96aca42f\">\n{\"verdict\":\"revise\"}\n</ARIA_STRUCTURED_OUTPUT>";

        let parsed = parse_structured_output(output, &contract());

        let StructuredOutputState::Failed(error) = parsed.state else {
            panic!("expected structured output failure");
        };
        assert_eq!(error.code, StructuredOutputErrorCode::MissingEndNonce);
        assert_eq!(error.expected_nonce.as_deref(), Some("96aca42f"));
        assert_eq!(error.recoverable_value, Some(json!({"verdict": "revise"})));
        assert_eq!(parsed.readable_output, "审核说明");
    }

    #[test]
    fn classifies_nonce_mismatch() {
        let output = "<ARIA_STRUCTURED_OUTPUT nonce=\"96aca42f\">{\"verdict\":\"pass\"}</ARIA_STRUCTURED_OUTPUT nonce=\"deadbeef\">";

        let parsed = parse_structured_output(output, &contract());

        let StructuredOutputState::Failed(error) = parsed.state else {
            panic!("expected structured output failure");
        };
        assert_eq!(error.code, StructuredOutputErrorCode::NonceMismatch);
        assert_eq!(error.observed_nonce.as_deref(), Some("deadbeef"));
    }

    #[test]
    fn classifies_invalid_json_without_trusting_value() {
        let output = "<ARIA_STRUCTURED_OUTPUT nonce=\"96aca42f\">{invalid}</ARIA_STRUCTURED_OUTPUT nonce=\"96aca42f\">";

        let parsed = parse_structured_output(output, &contract());

        let StructuredOutputState::Failed(error) = parsed.state else {
            panic!("expected structured output failure");
        };
        assert_eq!(error.code, StructuredOutputErrorCode::InvalidJson);
        assert!(error.recoverable_value.is_none());
    }

    #[test]
    fn parses_fenced_json_inside_matching_nonce_block() {
        let output = "<ARIA_STRUCTURED_OUTPUT nonce=\"96aca42f\">\n```json\n{\"verdict\":\"pass\"}\n```\n</ARIA_STRUCTURED_OUTPUT nonce=\"96aca42f\">";

        let parsed = parse_structured_output(output, &contract());

        assert_eq!(
            parsed.state,
            StructuredOutputState::Parsed(json!({"verdict": "pass"}))
        );
    }

    #[test]
    fn legacy_parser_rejects_unsupported_end_attributes_without_start_nonce() {
        let output = "<ARIA_STRUCTURED_OUTPUT>{\"verdict\":\"pass\"}</ARIA_STRUCTURED_OUTPUT bogus=\"value\">";

        let error = parse_last_structured_output_value(output)
            .expect_err("unsupported end attributes must fail");

        assert_eq!(error.code, StructuredOutputErrorCode::NonceMismatch);
    }
}
