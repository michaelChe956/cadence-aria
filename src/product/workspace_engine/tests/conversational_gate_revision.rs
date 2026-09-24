use crate::product::work_item_split_engine::prompts::WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES;
use crate::product::workspace_engine::conversational_gate::trim_provider_preamble;
use crate::product::workspace_engine::prompts::{
    SC_MANUAL_REVISION_FEEDBACK_MAX_BYTES, SC_MANUAL_REVISION_PROMPT_QUALITY_BUDGET_BYTES,
    ScManualRevisionPromptInput, build_sc_manual_revision_prompt,
};
use crate::product::workspace_engine::{
    HumanGateCloseDecision, HumanGateCommandOutcome, HumanGateFeedbackInput,
};

const LANGUAGE_RULE_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/.claude/rules/language.md"
));
const PRIORITY_RULE_MARKER: &str =
    "结构标题(##/### section 名)、字段 key、ID(WI-*/CT-*/TASK-*/AC-*/REQ-*/CHECK-* 等)、枚举值";

include!("conversational_gate_revision/part_01.rs");
include!("conversational_gate_revision/part_02.rs");
include!("conversational_gate_revision/part_03.rs");
include!("conversational_gate_revision/part_04.rs");
