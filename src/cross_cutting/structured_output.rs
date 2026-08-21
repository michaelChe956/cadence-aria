use serde::{Deserialize, Serialize};
use serde_json::Value;

const START_PREFIX: &str = "<ARIA_STRUCTURED_OUTPUT";
const END_PREFIX: &str = "</ARIA_STRUCTURED_OUTPUT";
pub const MAX_JSON_BYTES: usize = 65_536;
pub const MAX_JSON_DEPTH: usize = 32;

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
    NonceMismatch,
    MissingJsonNonce,
    JsonNonceMismatch,
    InvalidEndTag,
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
            Self::NonceMismatch => "nonce_mismatch",
            Self::MissingJsonNonce => "missing_json_nonce",
            Self::JsonNonceMismatch => "json_nonce_mismatch",
            Self::InvalidEndTag => "invalid_end_tag",
            Self::InvalidJson => "invalid_json",
        }
    }
}

pub fn parse_structured_output(
    output: &str,
    contract: &StructuredOutputContract,
) -> StructuredOutputParse {
    let Some(start) = output.rfind(START_PREFIX) else {
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

/// Parses the final sentinel under the same protocol used by contract-bound consumers.
///
/// This is intended for legacy workspace call sites that discover the nonce from the
/// sentinel itself. The JSON envelope nonce is still required and removed before the
/// value is returned.
pub fn parse_last_structured_output(
    output: &str,
) -> Result<Option<(String, Value)>, StructuredOutputError> {
    let Some(start) = output.rfind(START_PREFIX) else {
        return Ok(None);
    };
    let parsed = parse_block_at(output, start, None);
    match parsed.state {
        StructuredOutputState::Parsed(value) => Ok(Some((parsed.readable_output, value))),
        StructuredOutputState::Failed(error) => Err(error),
        StructuredOutputState::NotRequested => Ok(None),
    }
}

pub fn parse_last_structured_output_value(
    output: &str,
) -> Result<Option<Value>, StructuredOutputError> {
    parse_last_structured_output(output).map(|parsed| parsed.map(|(_, value)| value))
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
    if expected_nonce.is_some_and(|expected| start_nonce != expected) {
        return failed(
            output,
            Some(start_index),
            StructuredOutputErrorCode::NonceMismatch,
            "structured output start nonce mismatch",
            expected_nonce.map(str::to_string),
            Some(start_nonce),
            None,
        );
    }

    let expected_json_nonce = expected_nonce.unwrap_or(start_nonce.as_str());
    let after_start = &output[json_start..];
    let Some(end_relative) = find_end_tag_outside_json_strings(after_start) else {
        return failed(
            output,
            None,
            StructuredOutputErrorCode::MissingEndTag,
            "missing structured output end tag",
            Some(expected_json_nonce.to_string()),
            None,
            recoverable_value(after_start, expected_json_nonce),
        );
    };
    let json_text = &after_start[..end_relative];
    let after_end_prefix = &after_start[end_relative + END_PREFIX.len()..];
    let Some(tag_close) = after_end_prefix.find('>') else {
        return failed(
            output,
            None,
            StructuredOutputErrorCode::MissingEndTag,
            "structured output end tag is not closed",
            Some(expected_json_nonce.to_string()),
            None,
            recoverable_value(json_text, expected_json_nonce),
        );
    };

    let end_attrs = after_end_prefix[..tag_close].trim();
    let end_tag_len = END_PREFIX.len() + tag_close + 1;
    let block_end = json_start + end_relative + end_tag_len;
    let readable_output = format!("{}{}", &output[..start_index], &output[block_end..])
        .trim()
        .to_string();
    if !end_attrs.is_empty() {
        return parse_failure(
            readable_output,
            StructuredOutputErrorCode::InvalidEndTag,
            "structured output end tag must not have attributes",
            Some(expected_json_nonce.to_string()),
            None,
            recoverable_value(json_text, expected_json_nonce),
        );
    }

    let mut value = match recover_json_object(json_text) {
        Ok(value) => value,
        Err(()) => {
            return parse_failure(
                readable_output,
                StructuredOutputErrorCode::InvalidJson,
                "invalid structured output json",
                Some(expected_json_nonce.to_string()),
                None,
                None,
            );
        }
    };
    let recoverable_value = business_payload_without_nonce(&value);
    match strip_and_validate_json_nonce(&mut value, expected_json_nonce) {
        Ok(()) => StructuredOutputParse {
            readable_output,
            state: StructuredOutputState::Parsed(value),
        },
        Err(mut error) => {
            // A nonce failure is never accepted as business output. Retaining only the
            // nonce-stripped object lets the existing repair turn restore an otherwise
            // complete verdict without granting the invalid envelope any authority.
            error.recoverable_value = recoverable_value;
            StructuredOutputParse {
                readable_output,
                state: StructuredOutputState::Failed(error),
            }
        }
    }
}

fn parse_start_tag(
    output: &str,
    start_index: usize,
    expected_nonce: Option<&str>,
) -> Result<(String, usize), StructuredOutputError> {
    let after_start_prefix = &output[start_index + START_PREFIX.len()..];
    let Some(tag_close) = after_start_prefix.find('>') else {
        return Err(error(
            StructuredOutputErrorCode::MissingStartTag,
            "structured output start tag is not closed",
            expected_nonce,
            None,
            None,
        ));
    };
    let attrs = after_start_prefix[..tag_close].trim();
    let nonce = parse_nonce(attrs).ok_or_else(|| {
        error(
            StructuredOutputErrorCode::MissingStartTag,
            "structured output start tag must contain a valid nonce attribute",
            expected_nonce,
            None,
            None,
        )
    })?;
    Ok((nonce, start_index + START_PREFIX.len() + tag_close + 1))
}

fn parse_nonce(attrs: &str) -> Option<String> {
    let nonce = attrs
        .strip_prefix("nonce=\"")
        .and_then(|value| value.strip_suffix('"'))?;
    // The contract generates compact random nonces, while its intentional few-shot
    // placeholder is `EXAMPLE_NONCE`; format is not the trust boundary. The exact
    // value comparison below is, so retain any non-empty attribute value here.
    (!nonce.is_empty()).then(|| nonce.to_string())
}

fn recoverable_value(text: &str, expected_nonce: &str) -> Option<Value> {
    let mut value = recover_json_object(text).ok()?;
    strip_and_validate_json_nonce(&mut value, expected_nonce)
        .ok()
        .map(|()| value)
}

fn business_payload_without_nonce(value: &Value) -> Option<Value> {
    let mut value = value.clone();
    value.as_object_mut()?.remove("nonce");
    Some(value)
}

fn strip_and_validate_json_nonce(
    value: &mut Value,
    expected_nonce: &str,
) -> Result<(), StructuredOutputError> {
    let Some(object) = value.as_object_mut() else {
        return Err(error(
            StructuredOutputErrorCode::InvalidJson,
            "structured output json must be an object",
            Some(expected_nonce),
            None,
            None,
        ));
    };
    let Some(nonce) = object.remove("nonce") else {
        return Err(error(
            StructuredOutputErrorCode::MissingJsonNonce,
            "structured output json is missing envelope nonce",
            Some(expected_nonce),
            None,
            None,
        ));
    };
    let observed_nonce = nonce.as_str().map(str::to_string);
    if observed_nonce.as_deref() != Some(expected_nonce) {
        return Err(error(
            StructuredOutputErrorCode::JsonNonceMismatch,
            "structured output json envelope nonce mismatch",
            Some(expected_nonce),
            observed_nonce.as_deref(),
            None,
        ));
    }
    Ok(())
}

/// Recovers exactly one top-level JSON object from the sentinel body. The state
/// machine deliberately understands strings and escapes, but it does not repair
/// JSON; serde remains the final authority on JSON validity.
fn find_end_tag_outside_json_strings(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut in_string = false;
    let mut escaped = false;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        if byte == b'"' {
            in_string = true;
        } else if bytes[index..].starts_with(END_PREFIX.as_bytes()) {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn recover_json_object(text: &str) -> Result<Value, ()> {
    let candidate = sentinel_json_candidate(text)?;
    let object = extract_unique_json_object(candidate)?;
    if object.len() > MAX_JSON_BYTES {
        return Err(());
    }
    serde_json::from_str(object).map_err(|_| ())
}

fn sentinel_json_candidate(text: &str) -> Result<&str, ()> {
    let trimmed = text.trim();
    if !trimmed.starts_with("```") {
        return Ok(trimmed);
    }

    let after_open = &trimmed[3..];
    let Some(newline) = after_open.find('\n') else {
        return Err(());
    };
    let inner_with_end = &after_open[newline + 1..];
    let Some(inner) = inner_with_end.strip_suffix("```") else {
        return Err(());
    };
    Ok(inner.trim())
}

fn extract_unique_json_object(text: &str) -> Result<&str, ()> {
    let bytes = text.as_bytes();
    let start = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .ok_or(())?;
    if bytes[start] != b'{' {
        return Err(());
    }

    let mut stack = Vec::with_capacity(MAX_JSON_DEPTH);
    let mut in_string = false;
    let mut escaped = false;
    for (index, byte) in bytes.iter().copied().enumerate().skip(start) {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }

        match byte {
            b'"' => in_string = true,
            b'{' => {
                stack.push(b'}');
                if stack.len() > MAX_JSON_DEPTH {
                    return Err(());
                }
            }
            b'[' => {
                stack.push(b']');
                if stack.len() > MAX_JSON_DEPTH {
                    return Err(());
                }
            }
            b'}' | b']' => {
                if stack.pop() != Some(byte) {
                    return Err(());
                }
                if stack.is_empty() {
                    if bytes[index + 1..]
                        .iter()
                        .any(|byte| !byte.is_ascii_whitespace())
                    {
                        return Err(());
                    }
                    return Ok(&text[start..=index]);
                }
            }
            _ => {}
        }
    }
    Err(())
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
    parse_failure(
        readable_output,
        code,
        message,
        expected_nonce,
        observed_nonce,
        recoverable_value,
    )
}

fn parse_failure(
    readable_output: String,
    code: StructuredOutputErrorCode,
    message: &str,
    expected_nonce: Option<String>,
    observed_nonce: Option<String>,
    recoverable_value: Option<Value>,
) -> StructuredOutputParse {
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

fn error(
    code: StructuredOutputErrorCode,
    message: &str,
    expected_nonce: Option<&str>,
    observed_nonce: Option<&str>,
    recoverable_value: Option<Value>,
) -> StructuredOutputError {
    StructuredOutputError {
        code,
        message: message.to_string(),
        expected_nonce: expected_nonce.map(str::to_string),
        observed_nonce: observed_nonce.map(str::to_string),
        recoverable_value,
    }
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

    fn block(json: &str) -> String {
        format!("<ARIA_STRUCTURED_OUTPUT nonce=\"96aca42f\">{json}</ARIA_STRUCTURED_OUTPUT>")
    }

    fn failed_error(output: &str) -> StructuredOutputError {
        let parsed = parse_structured_output(output, &contract());
        let StructuredOutputState::Failed(error) = parsed.state else {
            panic!("expected structured output failure: {parsed:?}");
        };
        error
    }

    #[test]
    fn parses_new_protocol_strips_envelope_and_trailing_text_into_readable_output() {
        let output = format!(
            "审核说明\n{}\n闭合后的可读说明",
            block(r#"{"nonce":"96aca42f","verdict":"pass"}"#)
        );

        let parsed = parse_structured_output(&output, &contract());

        assert_eq!(parsed.readable_output, "审核说明\n\n闭合后的可读说明");
        assert_eq!(
            parsed.state,
            StructuredOutputState::Parsed(json!({"verdict": "pass"}))
        );
    }

    #[test]
    fn classifies_missing_and_mismatched_json_nonce_with_distinct_stable_codes() {
        let missing = failed_error(&block(r#"{"verdict":"pass"}"#));
        let mismatch = failed_error(&block(r#"{"nonce":"deadbeef","verdict":"pass"}"#));

        assert_eq!(missing.code, StructuredOutputErrorCode::MissingJsonNonce);
        assert_eq!(missing.code.as_str(), "missing_json_nonce");
        assert_eq!(mismatch.code, StructuredOutputErrorCode::JsonNonceMismatch);
        assert_eq!(mismatch.code.as_str(), "json_nonce_mismatch");
        assert_eq!(mismatch.observed_nonce.as_deref(), Some("deadbeef"));
    }

    #[test]
    fn rejects_old_double_closed_protocol_with_distinct_stable_code() {
        let output = format!(
            "<ARIA_STRUCTURED_OUTPUT nonce=\"96aca42f\">{{\"nonce\":\"96aca42f\",\"verdict\":\"pass\"}}</ARIA_STRUCTURED_OUTPUT{}>",
            " nonce=\"96aca42f\""
        );

        let error = failed_error(&output);

        assert_eq!(error.code, StructuredOutputErrorCode::InvalidEndTag);
        assert_eq!(error.code.as_str(), "invalid_end_tag");
    }

    #[test]
    fn rejects_copied_example_nonce() {
        let copied_example = "<ARIA_STRUCTURED_OUTPUT nonce=\"EXAMPLE_NONCE\">\
            {\"nonce\":\"EXAMPLE_NONCE\",\"verdict\":\"pass\"}\
            </ARIA_STRUCTURED_OUTPUT>";
        let error = failed_error(copied_example);

        assert_eq!(error.code, StructuredOutputErrorCode::NonceMismatch);
        assert_eq!(error.observed_nonce.as_deref(), Some("EXAMPLE_NONCE"));
    }

    #[test]
    fn rejects_non_whitespace_between_json_and_closing_tag() {
        let error = failed_error(&block(r#"{"nonce":"96aca42f","verdict":"pass"}说明"#));

        assert_eq!(error.code, StructuredOutputErrorCode::InvalidJson);
    }

    #[test]
    fn recovers_single_json_object_inside_one_complete_fence() {
        let parsed = parse_structured_output(
            &block("\n```json\n{\"nonce\":\"96aca42f\",\"verdict\":\"pass\"}\n```\n"),
            &contract(),
        );

        assert_eq!(
            parsed.state,
            StructuredOutputState::Parsed(json!({"verdict": "pass"}))
        );
    }

    #[test]
    fn rejects_fence_external_text_multiple_candidates_and_trailing_commas() {
        for json in [
            "explanation\n```json\n{\"nonce\":\"96aca42f\"}\n```",
            "{\"nonce\":\"96aca42f\"}{\"nonce\":\"96aca42f\"}",
            "{\"nonce\":\"96aca42f\",}",
        ] {
            assert_eq!(
                failed_error(&block(json)).code,
                StructuredOutputErrorCode::InvalidJson,
                "must reject {json}"
            );
        }
    }

    #[test]
    fn recovery_is_string_and_escape_aware_and_rejects_mismatched_brackets() {
        let valid = block(
            r#"{"nonce":"96aca42f","message":"literal { } and escaped quote: \" plus slash: \\"}"#,
        );
        let parsed = parse_structured_output(&valid, &contract());
        assert_eq!(
            parsed.state,
            StructuredOutputState::Parsed(
                json!({"message": "literal { } and escaped quote: \" plus slash: \\"})
            )
        );

        let invalid = block(r#"{"nonce":"96aca42f","items":[}"#);
        assert_eq!(
            failed_error(&invalid).code,
            StructuredOutputErrorCode::InvalidJson
        );
    }

    #[test]
    fn accepts_json_depth_and_byte_limits_at_boundary_and_rejects_overflow() {
        let at_depth_limit = nested_payload(MAX_JSON_DEPTH - 1);
        let over_depth_limit = nested_payload(MAX_JSON_DEPTH);
        assert!(matches!(
            parse_structured_output(&block(&at_depth_limit), &contract()).state,
            StructuredOutputState::Parsed(_)
        ));
        assert_eq!(
            failed_error(&block(&over_depth_limit)).code,
            StructuredOutputErrorCode::InvalidJson
        );

        let base = r#"{"nonce":"96aca42f","payload":""}"#;
        let at_byte_limit = format!(
            r#"{{"nonce":"96aca42f","payload":"{}"}}"#,
            "x".repeat(MAX_JSON_BYTES - base.len())
        );
        assert_eq!(at_byte_limit.len(), MAX_JSON_BYTES);
        assert!(matches!(
            parse_structured_output(&block(&at_byte_limit), &contract()).state,
            StructuredOutputState::Parsed(_)
        ));
        // Insert one byte into the JSON string, not after its object boundary.
        let over_byte_limit = format!(
            r#"{{"nonce":"96aca42f","payload":"{}"}}"#,
            "x".repeat(MAX_JSON_BYTES - base.len() + 1)
        );
        assert_eq!(over_byte_limit.len(), MAX_JSON_BYTES + 1);
        assert_eq!(
            failed_error(&block(&over_byte_limit)).code,
            StructuredOutputErrorCode::InvalidJson
        );
    }

    fn nested_payload(nested_object_count: usize) -> String {
        let mut payload = "{\"nonce\":\"96aca42f\",\"value\":".to_string();
        for _ in 0..nested_object_count {
            payload.push_str("{\"nested\":");
        }
        payload.push('0');
        payload.push_str(&"}".repeat(nested_object_count));
        payload.push('}');
        payload
    }

    #[test]
    fn parse_last_value_uses_the_same_protocol_and_strips_nonce() {
        let output = format!(
            "prefix {} suffix",
            block(r#"{"nonce":"96aca42f","verdict":"pass"}"#)
        );

        assert_eq!(
            parse_last_structured_output_value(&output).expect("new protocol"),
            Some(json!({"verdict": "pass"}))
        );
    }

    #[test]
    fn ignores_end_tag_text_inside_a_json_string_and_serializes_new_codes_in_snake_case() {
        let parsed = parse_structured_output(
            &block(r#"{"nonce":"96aca42f","message":"literal </ARIA_STRUCTURED_OUTPUT> text"}"#),
            &contract(),
        );
        assert_eq!(
            parsed.state,
            StructuredOutputState::Parsed(
                json!({"message": "literal </ARIA_STRUCTURED_OUTPUT> text"})
            )
        );
        assert_eq!(
            serde_json::to_string(&StructuredOutputErrorCode::MissingJsonNonce).expect("serialize"),
            "\"missing_json_nonce\""
        );
        assert_eq!(
            serde_json::to_string(&StructuredOutputErrorCode::InvalidEndTag).expect("serialize"),
            "\"invalid_end_tag\""
        );
    }
}
