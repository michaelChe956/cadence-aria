//! P0 1.3：workspace/coding choice 应答共用请求/状态 DTO（REQ-WIGA-05）。
//!
//! 复用 cross_cutting 的同一 `ChoiceReplyState`，不另造同义状态枚举；
//! `ChoiceResponseRequest` 的完整 `answers` 原样透传给 provider，禁止
//! 多题扁平化。HTTP `202` 体同 `ChoiceReplyStatus`，`200` 仅 Delivered。

use serde::{Deserialize, Serialize};

use crate::cross_cutting::choice_delivery::ChoiceReplyState;
use crate::cross_cutting::streaming_provider::ChoiceAnswerData;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ChoiceResponseRequest {
    pub command_id: String,
    pub expected_run_id: String,
    pub answers: Vec<ChoiceAnswerData>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ChoiceReplyStatus {
    pub command_id: String,
    pub expected_run_id: String,
    pub choice_id: String,
    pub state: ChoiceReplyState,
}
