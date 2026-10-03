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

use crate::cross_cutting::structured_output::parse_all_structured_output_blocks;
use crate::product::logical_codebase::LogicalRepositoryId;
use crate::product::workspace_engine::parsers::extract_markdown_fence_json;

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

/// 从 AI 产出的 Story markdown 中解析 `involved_repository_ids` 与
/// `focus_repository_id`。
///
/// 缺陷 #3（2026-10-02 E2E）：AI 产物常含**提示词模板回声**（同 nonce、
/// `<logical_repository_id>` 占位）与**真值**两份标签。这里扫描全部标签取
/// 最后一个**整体可用**（JSON+schema+UUID 全过）的块——模板回声天然被跳过；
/// 全部不可用时上报**真实**标签错误（不再落 `extract_structured_json` 的
/// 围栏兜底——其“expected ident”类错误掩蔽了真因）；完全无标签时保留既有
/// 围栏兜底与 `MissingStructuredOutput`（REQ-PLN-04 blocker）语义。
pub fn parse_story_aggregate_output(
    content: &str,
) -> Result<StoryAggregateOutput, AggregateOutputError> {
    #[derive(serde::Deserialize)]
    struct Schema {
        involved_repository_ids: Vec<String>,
        #[serde(default)]
        focus_repository_id: Option<String>,
    }

    let mut last_success = None;
    let mut last_block_error = None;
    for block in parse_all_structured_output_blocks(content) {
        let value = match block {
            Ok((_, value)) => value,
            Err(error) => {
                last_block_error = Some(error.message);
                continue;
            }
        };
        match serde_json::from_value::<Schema>(value) {
            Ok(parsed) => {
                let involved = parsed
                    .involved_repository_ids
                    .iter()
                    .map(|value| parse_repository_id(value))
                    .collect::<Result<Vec<_>, _>>();
                let focus = parsed
                    .focus_repository_id
                    .as_deref()
                    .map(parse_repository_id)
                    .transpose();
                match (involved, focus) {
                    (Ok(involved), Ok(focus)) => {
                        last_success = Some(StoryAggregateOutput {
                            involved_repository_ids: involved,
                            focus_repository_id: focus,
                        });
                    }
                    (Err(error), _) | (_, Err(error)) => {
                        last_block_error = Some(match error {
                            AggregateOutputError::InvalidSchema(message) => message,
                            other => format!("{other:?}"),
                        });
                    }
                }
            }
            Err(error) => last_block_error = Some(error.to_string()),
        }
    }
    if let Some(success) = last_success {
        return Ok(success);
    }
    if let Some(message) = last_block_error {
        return Err(AggregateOutputError::InvalidSchema(message));
    }
    if let Some((_, json)) = extract_markdown_fence_json(content) {
        if let Ok(parsed) = serde_json::from_str::<Schema>(&json) {
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
            return Ok(StoryAggregateOutput {
                involved_repository_ids: involved,
                focus_repository_id: focus,
            });
        }
    }
    Err(AggregateOutputError::MissingStructuredOutput)
}

/// 从 AI 产出的 Design markdown 中解析 `involved_repository_ids` 与
/// `change_order`（标签择块口径同 [`parse_story_aggregate_output`]，缺陷 #3）。
pub fn parse_design_aggregate_output(
    content: &str,
) -> Result<DesignAggregateOutput, AggregateOutputError> {
    #[derive(serde::Deserialize)]
    struct Schema {
        involved_repository_ids: Vec<String>,
        #[serde(default)]
        change_order: Vec<String>,
    }

    let mut last_success = None;
    let mut last_block_error = None;
    for block in parse_all_structured_output_blocks(content) {
        let value = match block {
            Ok((_, value)) => value,
            Err(error) => {
                last_block_error = Some(error.message);
                continue;
            }
        };
        match serde_json::from_value::<Schema>(value) {
            Ok(parsed) => {
                let involved = parsed
                    .involved_repository_ids
                    .iter()
                    .map(|value| parse_repository_id(value))
                    .collect::<Result<Vec<_>, _>>();
                let change_order = parsed
                    .change_order
                    .iter()
                    .map(|value| parse_repository_id(value))
                    .collect::<Result<Vec<_>, _>>();
                match (involved, change_order) {
                    (Ok(involved), Ok(change_order)) => {
                        last_success = Some(DesignAggregateOutput {
                            involved_repository_ids: involved,
                            change_order,
                        });
                    }
                    (Err(error), _) | (_, Err(error)) => {
                        last_block_error = Some(match error {
                            AggregateOutputError::InvalidSchema(message) => message,
                            other => format!("{other:?}"),
                        });
                    }
                }
            }
            Err(error) => last_block_error = Some(error.to_string()),
        }
    }
    if let Some(success) = last_success {
        return Ok(success);
    }
    if let Some(message) = last_block_error {
        return Err(AggregateOutputError::InvalidSchema(message));
    }
    if let Some((_, json)) = extract_markdown_fence_json(content) {
        if let Ok(parsed) = serde_json::from_str::<Schema>(&json) {
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
            return Ok(DesignAggregateOutput {
                involved_repository_ids: involved,
                change_order,
            });
        }
    }
    Err(AggregateOutputError::MissingStructuredOutput)
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

    /// 缺陷 #3 红→绿：AI 产物「真值标签在前、提示词模板回声在后」时取真值
    ///（实测形态：timeline_node_002——模板与真值同 nonce，模板占位 UUID）。
    #[test]
    fn parse_story_output_picks_real_tag_over_trailing_template_echo() {
        let nonce = "61e31b5a";
        let member = "df2303b9-f2d3-42e1-946f-7458885f41be";
        let real = wrapped(
            nonce,
            &format!(
                "{{\"nonce\":\"{nonce}\",\"involved_repository_ids\":[\"{member}\"],\"focus_repository_id\":\"{member}\"}}"
            ),
        );
        let template = wrapped(
            nonce,
            "{\"nonce\":\"61e31b5a\",\"involved_repository_ids\":[\"<logical_repository_id>\"],\"focus_repository_id\":\"<logical_repository_id>|null\"}",
        );
        let content = format!("# Story\n...\n{real}\n格式说明（回声）：\n{template}");
        let parsed = parse_story_aggregate_output(&content).expect("real tag must win");
        assert_eq!(
            parsed.involved_repository_ids,
            vec![LogicalRepositoryId(uuid::Uuid::parse_str(member).unwrap())]
        );
        assert_eq!(
            parsed.focus_repository_id,
            Some(LogicalRepositoryId(uuid::Uuid::parse_str(member).unwrap()))
        );
    }

    /// 缺陷 #3 负例：仅模板回声、无真值 → 上报**真实** InvalidSchema（占位
    /// UUID 无法解析），不得掩蔽为围栏兜底的语法错误。
    #[test]
    fn parse_story_output_template_only_reports_real_invalid_schema() {
        let template = wrapped(
            "61e31b5a",
            "{\"nonce\":\"61e31b5a\",\"involved_repository_ids\":[\"<logical_repository_id>\"],\"focus_repository_id\":\"<logical_repository_id>|null\"}",
        );
        let content = format!("# Story\n...\n{template}");
        let error = parse_story_aggregate_output(&content).unwrap_err();
        match error {
            AggregateOutputError::InvalidSchema(message) => {
                assert!(
                    message.contains("found `<`") || message.contains("UUID"),
                    "real placeholder/UUID error must surface (not fence fallback), got: {message}"
                );
            }
            other => panic!("expected InvalidSchema, got {other:?}"),
        }
    }

    /// Design 同口径：模板回声在后取真值。
    #[test]
    fn parse_design_output_picks_real_tag_over_trailing_template_echo() {
        let nonce = "61e31b5a";
        let member = "df2303b9-f2d3-42e1-946f-7458885f41be";
        let real = wrapped(
            nonce,
            &format!(
                "{{\"nonce\":\"{nonce}\",\"involved_repository_ids\":[\"{member}\"],\"change_order\":[\"{member}\"]}}"
            ),
        );
        let template = wrapped(
            nonce,
            "{\"nonce\":\"61e31b5a\",\"involved_repository_ids\":[\"<logical_repository_id>\"],\"change_order\":[\"<logical_repository_id>\"]}",
        );
        let content = format!("# Design\n...\n{real}\n格式说明（回声）：\n{template}");
        let parsed = parse_design_aggregate_output(&content).expect("real tag must win");
        assert_eq!(
            parsed.involved_repository_ids,
            vec![LogicalRepositoryId(uuid::Uuid::parse_str(member).unwrap())]
        );
        assert_eq!(parsed.change_order.len(), 1);
    }

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
