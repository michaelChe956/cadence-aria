//! 解析 AI 产出的 structured output（`<ARIA_STRUCTURED_OUTPUT nonce>` 标签）。
//!
//! 复用 `extract_structured_json`（parsers.rs 既有）完成标签提取，得到
//! `(comments, json_str)` 后再以 serde 反序列化，提取：
//! - Story：`involved_repository_ids` + `focus_repository_id`
//! - Design：`involved_repository_ids` + `change_order`
//!
//! 缺标签 → `MissingStructuredOutput`（按 REQ-PLN-04「AI 不确定即 blocker」）；
//! JSON schema / UUID 非法 → `InvalidSchema`。

use uuid::Uuid;

use crate::product::logical_codebase::LogicalRepositoryId;
use crate::product::workspace_engine::parsers::extract_structured_json;

/// Story 聚合输出的结构化结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoryAggregateOutput {
    pub involved_repository_ids: Vec<LogicalRepositoryId>,
    pub focus_repository_id: Option<LogicalRepositoryId>,
}

/// Design 聚合输出的结构化结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesignAggregateOutput {
    pub involved_repository_ids: Vec<LogicalRepositoryId>,
    pub change_order: Vec<LogicalRepositoryId>,
}

/// 解析 aggregate structured output 过程中的错误。
#[derive(Debug)]
pub enum AggregateOutputError {
    /// 未找到 `<ARIA_STRUCTURED_OUTPUT nonce>` 标签（AI 不确定即 blocker）。
    MissingStructuredOutput,
    /// JSON schema 非法或 UUID 无法解析。
    InvalidSchema(String),
}

/// 从 AI 产出的 Story markdown 中解析 `involved_repository_ids` 与 `focus_repository_id`。
pub fn parse_story_aggregate_output(
    content: &str,
) -> Result<StoryAggregateOutput, AggregateOutputError> {
    let (_, json) =
        extract_structured_json(content).ok_or(AggregateOutputError::MissingStructuredOutput)?;

    #[derive(serde::Deserialize)]
    struct Schema {
        involved_repository_ids: Vec<String>,
        #[serde(default)]
        focus_repository_id: Option<String>,
    }

    let parsed: Schema = serde_json::from_str(&json)
        .map_err(|error| AggregateOutputError::InvalidSchema(error.to_string()))?;
    let involved = parsed
        .involved_repository_ids
        .iter()
        .map(|value| parse_repository_id(value))
        .collect::<Result<Vec<_>, _>>()?;
    let focus = parsed
        .focus_repository_id
        .as_deref()
        .map(parse_repository_id)
        .transpose()?;
    Ok(StoryAggregateOutput {
        involved_repository_ids: involved,
        focus_repository_id: focus,
    })
}

/// 从 AI 产出的 Design markdown 中解析 `involved_repository_ids` 与 `change_order`。
pub fn parse_design_aggregate_output(
    content: &str,
) -> Result<DesignAggregateOutput, AggregateOutputError> {
    let (_, json) =
        extract_structured_json(content).ok_or(AggregateOutputError::MissingStructuredOutput)?;

    #[derive(serde::Deserialize)]
    struct Schema {
        involved_repository_ids: Vec<String>,
        #[serde(default)]
        change_order: Vec<String>,
    }

    let parsed: Schema = serde_json::from_str(&json)
        .map_err(|error| AggregateOutputError::InvalidSchema(error.to_string()))?;
    let involved = parsed
        .involved_repository_ids
        .iter()
        .map(|value| parse_repository_id(value))
        .collect::<Result<Vec<_>, _>>()?;
    let change_order = parsed
        .change_order
        .iter()
        .map(|value| parse_repository_id(value))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(DesignAggregateOutput {
        involved_repository_ids: involved,
        change_order,
    })
}

fn parse_repository_id(value: &str) -> Result<LogicalRepositoryId, AggregateOutputError> {
    Uuid::parse_str(value)
        .map(LogicalRepositoryId)
        .map_err(|error| AggregateOutputError::InvalidSchema(error.to_string()))
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use crate::product::logical_codebase::LogicalRepositoryId;
    use crate::product::workspace_engine::aggregate_output_parser::{
        AggregateOutputError, parse_design_aggregate_output, parse_story_aggregate_output,
    };

    /// Constructs the repository sentinel protocol with a JSON envelope nonce.
    fn wrapped(nonce: &str, json: &str) -> String {
        let mut value: serde_json::Value = serde_json::from_str(json).expect("fixture JSON");
        value
            .as_object_mut()
            .expect("fixture JSON object")
            .insert("nonce".to_string(), serde_json::json!(nonce));
        format!("<ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">{value}</ARIA_STRUCTURED_OUTPUT>")
    }

    #[test]
    fn parse_story_aggregate_output_extracts_involved_with_nonce() {
        let nonce = "abcd1234";
        let content = format!(
            "Story 内容...\n{}",
            wrapped(
                nonce,
                "{\"involved_repository_ids\":[\"00000000-0000-0000-0000-000000000001\"],\"focus_repository_id\":null}"
            )
        );
        let parsed = parse_story_aggregate_output(&content).unwrap();
        assert_eq!(
            parsed.involved_repository_ids,
            vec![LogicalRepositoryId(Uuid::from_u128(1))]
        );
        assert_eq!(parsed.focus_repository_id, None);
    }

    #[test]
    fn parse_story_aggregate_output_rejects_missing_structured_tag() {
        // 无 ARIA_STRUCTURED_OUTPUT 标签 → 按 REQ-PLN-04「AI 不确定即 blocker」
        assert!(parse_story_aggregate_output("无结构化输出的内容").is_err());
    }

    #[test]
    fn parse_story_aggregate_output_extracts_focus_repository() {
        let content = format!(
            "Story 内容...\n{}",
            wrapped(
                "abcd1234",
                "{\"involved_repository_ids\":[\"00000000-0000-0000-0000-000000000001\",\"00000000-0000-0000-0000-000000000002\"],\"focus_repository_id\":\"00000000-0000-0000-0000-000000000002\"}"
            )
        );
        let parsed = parse_story_aggregate_output(&content).unwrap();
        assert_eq!(
            parsed.involved_repository_ids,
            vec![
                LogicalRepositoryId(Uuid::from_u128(1)),
                LogicalRepositoryId(Uuid::from_u128(2)),
            ]
        );
        assert_eq!(
            parsed.focus_repository_id,
            Some(LogicalRepositoryId(Uuid::from_u128(2)))
        );
    }

    #[test]
    fn parse_story_aggregate_output_rejects_invalid_uuid() {
        let content = format!(
            "Story 内容...\n{}",
            wrapped(
                "abcd1234",
                "{\"involved_repository_ids\":[\"not-a-uuid\"],\"focus_repository_id\":null}"
            )
        );
        let error = parse_story_aggregate_output(&content).unwrap_err();
        assert!(matches!(error, AggregateOutputError::InvalidSchema(_)));
    }

    #[test]
    fn parse_design_aggregate_output_extracts_change_order() {
        // 同模式，含 change_order 字段
        let content = format!(
            "Design 内容...\n{}",
            wrapped(
                "abcd1234",
                "{\"involved_repository_ids\":[\"00000000-0000-0000-0000-000000000001\"],\"change_order\":[\"00000000-0000-0000-0000-000000000001\",\"00000000-0000-0000-0000-000000000002\"]}"
            )
        );
        let parsed = parse_design_aggregate_output(&content).unwrap();
        assert_eq!(
            parsed.involved_repository_ids,
            vec![LogicalRepositoryId(Uuid::from_u128(1))]
        );
        assert_eq!(
            parsed.change_order,
            vec![
                LogicalRepositoryId(Uuid::from_u128(1)),
                LogicalRepositoryId(Uuid::from_u128(2)),
            ]
        );
    }

    #[test]
    fn parse_design_aggregate_output_rejects_missing_structured_tag() {
        assert!(parse_design_aggregate_output("无结构化输出的内容").is_err());
    }
}
