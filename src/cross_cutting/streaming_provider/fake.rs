use std::time::Duration;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_adapter::ProviderAdapterError;

use super::{
    ProviderCommand, ProviderEvent, ProviderSession, StreamingProviderAdapter,
    StreamingProviderInput,
};

const FAKE_STREAMING_STEP_DELAY: Duration = Duration::from_millis(10);

pub struct FakeStreamingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for FakeStreamingProvider {
    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(32);
        let (command_tx, mut command_rx) = mpsc::channel(8);
        let output = fake_workspace_markdown(&input.prompt);

        tokio::spawn(async move {
            let chunks = fake_stream_chunks(&output);
            let mut commands_open = true;

            for content in chunks {
                if fake_streaming_should_stop(&cancel, &mut command_rx, &mut commands_open).await {
                    return;
                }

                if !fake_streaming_send_event(
                    &event_tx,
                    ProviderEvent::TextDelta { content },
                    &cancel,
                    &mut command_rx,
                    &mut commands_open,
                )
                .await
                {
                    return;
                }
            }

            if fake_streaming_should_stop(&cancel, &mut command_rx, &mut commands_open).await {
                return;
            }
            let _ = fake_streaming_send_event(
                &event_tx,
                ProviderEvent::Completed {
                    full_output: output,
                    provider_session_id: None,
                },
                &cancel,
                &mut command_rx,
                &mut commands_open,
            )
            .await;
        });

        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

async fn fake_streaming_should_stop(
    cancel: &CancellationToken,
    command_rx: &mut mpsc::Receiver<ProviderCommand>,
    commands_open: &mut bool,
) -> bool {
    let delay = tokio::time::sleep(FAKE_STREAMING_STEP_DELAY);
    tokio::pin!(delay);

    loop {
        if *commands_open {
            tokio::select! {
                _ = cancel.cancelled() => return true,
                command = command_rx.recv() => {
                    match command {
                        Some(ProviderCommand::Abort) => return true,
                        Some(ProviderCommand::PermissionResponse { .. })
                        | Some(ProviderCommand::ChoiceResponse { .. })
                        | Some(ProviderCommand::ToolResult(_)) => {}
                        None => *commands_open = false,
                    }
                }
                _ = &mut delay => return false,
            }
        } else {
            tokio::select! {
                _ = cancel.cancelled() => return true,
                _ = &mut delay => return false,
            }
        }
    }
}

async fn fake_streaming_send_event(
    event_tx: &mpsc::Sender<ProviderEvent>,
    event: ProviderEvent,
    cancel: &CancellationToken,
    command_rx: &mut mpsc::Receiver<ProviderCommand>,
    commands_open: &mut bool,
) -> bool {
    loop {
        if *commands_open {
            tokio::select! {
                _ = cancel.cancelled() => return false,
                permit = event_tx.reserve() => {
                    match permit {
                        Ok(permit) => {
                            permit.send(event);
                            return true;
                        }
                        Err(_) => return false,
                    }
                }
                command = command_rx.recv() => {
                    match command {
                        Some(ProviderCommand::Abort) => return false,
                        Some(ProviderCommand::PermissionResponse { .. })
                        | Some(ProviderCommand::ChoiceResponse { .. })
                        | Some(ProviderCommand::ToolResult(_)) => {}
                        None => *commands_open = false,
                    }
                }
            }
        } else {
            tokio::select! {
                _ = cancel.cancelled() => return false,
                permit = event_tx.reserve() => {
                    match permit {
                        Ok(permit) => {
                            permit.send(event);
                            return true;
                        }
                        Err(_) => return false,
                    }
                }
            }
        }
    }
}

fn fake_workspace_markdown(prompt: &str) -> String {
    if prompt.contains("Tester Provider Runtime") && prompt.contains("Phase: plan_tests") {
        return serde_json::json!({
            "summary": "fake provider smoke test plan",
            "steps": [{
                "id": "fake_smoke",
                "title": "Fake provider smoke",
                "intent": "prove fake provider can satisfy provider-driven testing",
                "required": true,
                "tool": "provider_managed",
                "risk_level": "low",
                "command_or_tool_input": {},
                "evidence_expectation": "fake provider emits deterministic step evidence",
                "related_requirements": ["REQ-FAKE"],
                "related_design_constraints": ["DEC-FAKE"],
                "related_work_item_tasks": ["TASK-FAKE"]
            }]
        })
        .to_string();
    }
    if prompt.contains("Tester Provider Runtime") && prompt.contains("Phase: execute_test_plan") {
        return serde_json::json!({
            "step_results": [{
                "step_id": "fake_smoke",
                "status": "passed",
                "evidence_refs": ["fake-provider-smoke.log"],
                "provider_analysis": "fake provider deterministic testing passed"
            }]
        })
        .to_string();
    }
    if prompt.contains("Work Item Splitter") || prompt.contains("IssueWorkItemPlan") {
        let structured_output = if prompt.contains("局部重做（revision）") {
            serde_json::json!({
                "repository_profile": {
                    "confidence": "high",
                    "detected_layers": ["backend", "frontend"],
                    "split_recommendation": "frontend_backend",
                    "languages": ["rust"],
                    "frameworks": [],
                    "package_managers": ["cargo"],
                    "test_frameworks": ["cargo test"],
                    "build_systems": ["cargo"],
                    "verification_capabilities": ["unit_tests"],
                    "uncertainties": []
                },
                "work_items": [{
                    "title": "重做 Work Item Plan 后端流式 collector",
                    "kind": "backend",
                    "sequence_hint": 1,
                    "depends_on": [],
                    "exclusive_write_scopes": ["src/product/workspace_engine.rs"],
                    "forbidden_write_scopes": [],
                    "required_handoff_from": [],
                    "require_execution_plan_confirm": false
                }],
                "verification_plans": [{
                    "scope": "unit",
                    "commands": [{
                        "id": "cmd_001",
                        "label": "workspace engine unit test",
                        "command": "cargo test --locked --lib drive_work_item_plan_provider_session_returns_output_and_persists_stream",
                        "cwd": ".",
                        "purpose": "验证 Work Item Plan 后端流式 collector",
                        "required": true,
                        "timeout_seconds": 120,
                        "safety": "approved"
                    }],
                    "manual_checks": [],
                    "required_gates": [],
                    "risk_notes": [],
                    "confidence": "high",
                    "fallback_policy": "manual_gate"
                }]
            })
        } else {
            serde_json::json!({
                "repository_profile": {
                    "confidence": "high",
                    "detected_layers": ["backend", "frontend"],
                    "split_recommendation": "frontend_backend",
                    "languages": ["rust"],
                    "frameworks": [],
                    "package_managers": ["cargo"],
                    "test_frameworks": ["cargo test"],
                    "build_systems": ["cargo"],
                    "verification_capabilities": ["unit_tests"],
                    "uncertainties": []
                },
                "plan": {
                    "work_item_ids": ["wi_01", "wi_02", "wi_03"],
                    "dependency_graph": [
                        { "from_work_item_id": "wi_01", "to_work_item_id": "wi_02" },
                        { "from_work_item_id": "wi_02", "to_work_item_id": "wi_03" }
                    ]
                },
                "work_items": [
                    {
                        "title": "实现 Work Item Plan 后端流式 collector",
                        "kind": "backend",
                        "sequence_hint": 1,
                        "depends_on": [],
                        "exclusive_write_scopes": ["src/product/workspace_engine.rs"],
                        "forbidden_write_scopes": [],
                        "context_budget": {
                            "target_context_k": "30-50",
                            "max_summary_chars": 20000,
                            "max_handoff_chars": 12000,
                            "max_code_context_chars": 30000,
                            "max_context_file_refs": 80,
                            "max_traceability_refs": 40,
                            "max_dependency_handoffs": 3
                        },
                        "required_handoff_from": [],
                        "require_execution_plan_confirm": false
                    },
                    {
                        "title": "接入 Workbench Work Item Plan 前端流式展示",
                        "kind": "frontend",
                        "sequence_hint": 2,
                        "depends_on": [0],
                        "exclusive_write_scopes": ["web/src/state/workspace-ws-store.ts"],
                        "forbidden_write_scopes": [],
                        "required_handoff_from": [],
                        "require_execution_plan_confirm": false
                    },
                    {
                        "title": "补充 Work Item Plan 流式集成测试",
                        "kind": "integration",
                        "sequence_hint": 3,
                        "depends_on": [1],
                        "exclusive_write_scopes": ["tests/it_web/web_work_item_plan_author.rs"],
                        "forbidden_write_scopes": [],
                        "required_handoff_from": [],
                        "require_execution_plan_confirm": false
                    }
                ],
                "verification_plans": [
                    {
                        "scope": "unit",
                        "commands": [{
                            "id": "cmd_001",
                            "label": "workspace engine unit test",
                            "command": "cargo test --locked --lib drive_work_item_plan_provider_session_returns_output_and_persists_stream",
                            "cwd": ".",
                            "purpose": "验证 Work Item Plan 后端流式 collector",
                            "required": true,
                            "timeout_seconds": 120,
                            "safety": "approved"
                        }],
                        "manual_checks": [],
                        "required_gates": [],
                        "risk_notes": [],
                        "confidence": "high",
                        "fallback_policy": "manual_gate"
                    },
                    {
                        "scope": "unit",
                        "commands": [{
                            "id": "cmd_002",
                            "label": "workspace ws store test",
                            "command": "pnpm test --run src/state/workspace-ws-store.test.ts",
                            "cwd": "web",
                            "purpose": "验证前端状态可接收 Work Item Plan provider stream",
                            "required": true,
                            "timeout_seconds": 120,
                            "safety": "approved"
                        }],
                        "manual_checks": [],
                        "required_gates": [],
                        "risk_notes": [],
                        "confidence": "high",
                        "fallback_policy": "manual_gate"
                    },
                    {
                        "scope": "integration",
                        "commands": [{
                            "id": "cmd_003",
                            "label": "work item plan integration test",
                            "command": "cargo test --locked --test it_web work_item_plan_author_streams_provider_output_before_candidate_artifact",
                            "cwd": ".",
                            "purpose": "验证 provider stream 先于 candidate artifact 出现",
                            "required": true,
                            "timeout_seconds": 180,
                            "safety": "approved"
                        }],
                        "manual_checks": [],
                        "required_gates": [],
                        "risk_notes": [],
                        "confidence": "high",
                        "fallback_policy": "manual_gate"
                    }
                ]
            })
        };
        return format!(
            "Fake Work Item Plan streaming draft\n\n\
             - 分析 Story/Design 约束\n\
             - 拆分可执行 Work Item\n\n\
             <ARIA_STRUCTURED_OUTPUT>{}</ARIA_STRUCTURED_OUTPUT>",
            structured_output
        );
    }

    let issue = extract_prompt_field(prompt, "Issue")
        .or_else(|| extract_prompt_field(prompt, "Issue 描述"))
        .unwrap_or_else(|| "当前 Issue".to_string());
    let issue_source_id =
        extract_prompt_id(prompt, "Issue").unwrap_or_else(|| "issue_0001".to_string());
    let user_intent = latest_user_message(prompt)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "开始生成".to_string());

    if prompt.contains("Workspace 类型: Design Spec") {
        return format!(
            "# Design Spec\n\n\
             ## 设计范围\n\n\
             面向 {issue} 生成候选设计，响应用户指令：{user_intent}。\n\n\
             ## 设计决策\n\n\
             [DEC-001] 采用最小可验证实现，保持实现与测试边界清晰。\n\n\
             ## 公共组件\n\n\
             [CMP-001] 在现有代码结构中增加必要模块，不引入无关依赖。\n\n\
             ## API 契约\n\n\
             [API-001] 复用现有 Workspace 工作流入口，不新增外部 API。\n\n\
             ## 数据模型\n\n\
             [DATA-001] 不新增持久化实体；状态变更沿用现有生命周期记录。\n\n\
             ## 风险\n\n\
             [RISK-001] 需求边界不完整时，在待确认项中保留人工确认入口。\n\n\
             ## 追踪关系\n\n\
             - source ids: Story Spec story_spec_0001, Issue {issue_source_id}。\n\
             - [DEC-001] -> [REQ-001]。"
        );
    }

    if prompt.contains("Workspace 类型: Work Item Plan") {
        return format!(
            "# Work Item Plan\n\n\
             ## 计划范围\n\n\
             为 {issue} 生成 Issue 级任务计划，响应用户指令：{user_intent}。\n\n\
             ## 任务拆分\n\n\
             [TASK-001] 实现核心逻辑。\n\
             [TASK-002] 补充自动化测试。\n\n\
             ## 依赖图\n\n\
             [TASK-001] -> [TASK-002]。\n\n\
             ## 验证计划\n\n\
             - 运行项目现有测试命令。\n\n\
             ## 执行顺序\n\n\
             先执行 [TASK-001]，再执行 [TASK-002]。\n\n\
             ## 风险\n\n\
             输入约束变化时需重新确认计划。\n\n\
             ## 追踪关系\n\n\
             - source ids: Story Spec story_spec_0001, Design Spec design_spec_0001。\n\
             - [TASK-001] -> [REQ-001]。\n\
             - [TASK-002] -> [AC-001]。"
        );
    }

    if prompt.contains("Workspace 类型: Work Item") {
        return format!(
            "# Work Item\n\n\
             ## 目标\n\n\
             为 {issue} 拆分可执行任务，响应用户指令：{user_intent}。\n\n\
             ## 范围\n\n\
             覆盖实现、测试与验证命令。\n\n\
             ## 实现步骤\n\n\
             - 实现核心逻辑。\n\
             - 补充自动化测试。\n\n\
             ## 依赖\n\n\
             依赖已确认 Story Spec 与 Design Spec。\n\n\
             ## 验证命令\n\n\
             - 运行项目现有测试命令。\n\n\
             ## 风险\n\n\
             输入约束变化时需重新确认计划。\n\n\
             ## 追踪关系\n\n\
             - source ids: Story Spec story_spec_0001, Design Spec design_spec_0001。\n\
             - 绑定来源 [REQ-001] / [DEC-001]。"
        );
    }

    format!(
        "# Story Spec\n\n\
         ## 范围\n\n\
         来源 source id: Issue {issue_source_id}；覆盖 {issue} 的候选 Story Spec，响应用户指令：{user_intent}。\n\n\
         ## 用户故事\n\n\
         作为使用者，我希望系统能清晰解决该问题并提供可运行验证。\n\n\
         ## 功能需求\n\n\
         [REQ-001] 程序必须计算爬到第 n 步的走法数量，每次可走 1 或 2 步。\n\
         [REQ-002] 实现必须保持 O(n) 时间复杂度，并包含自动化测试用例。\n\n\
         ## 成功标准\n\n\
         [AC-001] n=1、n=2、n=3 等基础输入返回正确走法数量。\n\
         [AC-002] 测试覆盖边界输入和常规输入。\n\n\
         ## 待确认项\n\n\
         无。\n\n\
         ## 非功能需求\n\n\
         [NFR-001] 代码应保持可读、无额外运行时依赖。\n\n\
         ## 输入摘要\n\n\
         {user_intent}"
    )
}

fn extract_prompt_field(prompt: &str, field: &str) -> Option<String> {
    let prefix = format!("{field}:");
    prompt
        .lines()
        .find_map(|line| line.trim().strip_prefix(&prefix).map(str::trim))
        .filter(|value| !value.is_empty())
        .map(|value| value.split(" (").next().unwrap_or(value).trim().to_string())
}

fn extract_prompt_id(prompt: &str, field: &str) -> Option<String> {
    let prefix = format!("{field}:");
    prompt.lines().find_map(|line| {
        let value = line.trim().strip_prefix(&prefix)?.trim();
        let start = value.rfind('(')?;
        let end = value.rfind(')')?;
        (end > start + 1).then(|| value[start + 1..end].trim().to_string())
    })
}

fn latest_user_message(prompt: &str) -> Option<String> {
    prompt
        .lines()
        .rev()
        .find_map(|line| line.trim().strip_prefix("[user]:").map(str::trim))
        .map(ToString::to_string)
}

fn fake_stream_chunks(output: &str) -> Vec<String> {
    const MAX_PARTS_PER_CHUNK: usize = 16;
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut part_count = 0usize;
    let mut current_is_whitespace = None;

    for ch in output.chars() {
        let is_whitespace = ch.is_whitespace();
        if current_is_whitespace.is_some_and(|previous| previous != is_whitespace)
            && !current.is_empty()
        {
            part_count += 1;
            if part_count >= MAX_PARTS_PER_CHUNK && !is_whitespace {
                chunks.push(std::mem::take(&mut current));
                part_count = 0;
            }
        }
        current_is_whitespace = Some(is_whitespace);
        current.push(ch);
    }

    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}
