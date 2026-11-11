//! provider 交互面（提问 / 授权请求）的诊断文本渲染。
//!
//! 无人值守场景（聚合初始化 / 知识库 operation）没有应答通道：provider 一旦
//! 发起提问或请求授权，本次 turn 只能中止（`provider_interaction_required`）。
//! 中止前必须把 **provider 问了什么** 写进诊断——否则界面只给一句
//! 「provider interaction required」的无信息量兜底，用户既不知道问的是什么，
//! 也不知道该改什么（真机：Pi 在 bootstrap 里发问，失败面看不出原因）。
//!
//! 放在产品层而不是 `cross_cutting::streaming_provider`：那层的事件类型文件
//! 已登记在「单文件行数红线」豁免表里，**不得增长**（豁免只放松「是否超
//! 1200」，不放宽「不得继续增长」）。

use crate::cross_cutting::streaming_provider::{ChoiceRequestData, PermissionRequestData};

/// 授权请求的诊断文本：provider 想跑什么工具、为什么。
pub(crate) fn permission_interaction_text(request: &PermissionRequestData) -> String {
    format!(
        "\n[provider 请求授权] tool={} desc={}",
        request.tool_name, request.description
    )
}

/// 提问的诊断文本：问题原文 + 可选项（用户要能照着回答/补输入）。
pub(crate) fn choice_interaction_text(request: &ChoiceRequestData) -> String {
    let questions = request
        .effective_questions()
        .iter()
        .map(|question| {
            let options = question
                .options
                .iter()
                .map(|option| option.label.as_str())
                .collect::<Vec<_>>()
                .join(" / ");
            if options.is_empty() {
                question.prompt.clone()
            } else {
                format!("{}（可选：{}）", question.prompt, options)
            }
        })
        .collect::<Vec<_>>()
        .join("；");
    format!("\n[provider 提问] {questions}")
}
