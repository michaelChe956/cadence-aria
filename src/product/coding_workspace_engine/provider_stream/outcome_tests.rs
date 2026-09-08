//! ProviderStreamOutcome 的结构化输出保持性测试。

use super::*;
use serde_json::json;

#[test]
fn provider_stream_outcome_keeps_completion_structured_output() {
    let outcome = ProviderStreamOutcome {
        full_output: "可读审查回执".to_string(),
        structured_output: StructuredOutputState::Parsed(json!({
            "verdict": "approve",
            "findings": []
        })),
    };

    assert_eq!(outcome.full_output, "可读审查回执");
    assert_eq!(
        outcome.structured_output,
        StructuredOutputState::Parsed(json!({
            "verdict": "approve",
            "findings": []
        }))
    );
}
