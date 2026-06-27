use serde::{Deserialize, Serialize};

use super::analyst::AnalystVerdict;
use super::execution::{CodingAgentRole, CodingExecutionStage};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CodingEntryType {
    UserMessage,
    AssistantMessage,
    ToolCall {
        tool_name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        output: String,
        is_error: bool,
    },
    StageGate {
        stage: CodingExecutionStage,
        countdown_seconds: u8,
    },
    AnalystVerdict {
        verdict: AnalystVerdict,
    },
    StageSummary {
        stage: CodingExecutionStage,
        summary: String,
    },
    SystemEvent {
        event_type: String,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingChatEntry {
    pub id: String,
    pub attempt_id: String,
    pub node_id: Option<String>,
    pub role: CodingAgentRole,
    pub entry_type: CodingEntryType,
    pub content: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingContextNote {
    pub id: String,
    pub attempt_id: String,
    pub content: String,
    pub created_at: String,
    pub consumed_by_rework_round: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingReworkInstruction {
    pub id: String,
    pub attempt_id: String,
    pub source_stage: CodingExecutionStage,
    pub rework_round: u32,
    pub summary: String,
    pub fix_hints: Vec<String>,
    pub questions: Vec<String>,
    pub created_at: String,
    pub consumed_by_node_id: Option<String>,
    pub consumed_at: Option<String>,
}
