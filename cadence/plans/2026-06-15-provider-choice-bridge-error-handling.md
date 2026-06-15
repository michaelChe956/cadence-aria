# Provider Choice Bridge 错误处理实现方案

> **给 agentic worker：** 必须子技能：使用 `superpowers:subagent-driven-development`（推荐）或 `superpowers:executing-plans` 按任务逐步执行。每个步骤使用复选框 `- [ ]` 语法，方便跟踪进度。

**目标：** 让 Claude Code 的 `AskUserQuestion` 和 Codex 的 `requestUserInput` 在桥接失败时发出明确的 `ProviderEvent::ProtocolError` 错误码（`ask_user_question_unresolved` / `request_user_input_unresolved`），并修复 `run_streaming` 遇到交互式选择或权限请求时静默挂起的问题。

**架构思路：** 在两个 provider adapter 内部增加针对性的错误处理，复用已有的 `ProviderEvent::ProtocolError` 通道；同时修复 `StreamingProviderAdapter::run_streaming` 默认实现，自动拒绝交互请求，并删除 Claude/Codex 中重复的 `run_streaming` 覆盖。

**技术栈：** Rust（edition 2024）、Tokio、serde_json、cargo test。

---

## 文件映射

| 文件 | 职责 | 变更 |
|------|------|--------|
| `src/cross_cutting/claude_code_provider.rs` | Claude Code 流式 adapter | 包装 `AskUserQuestion` 的 choice 桥接错误；识别 `tool_result` 的 `is_error`。 |
| `src/cross_cutting/codex_provider.rs` | Codex JSON-RPC adapter | 包装 `requestUserInput` 的 choice 桥接错误与响应写入错误。 |
| `src/cross_cutting/streaming_provider.rs` | 共享流式抽象 | 修复默认 `run_streaming`，自动拒绝 `ChoiceRequest`/`PermissionRequest`；新增回归测试。 |
| `tests/fixtures/provider/claude_ask_user_question_tool_error_fixture.sh` | 测试 fixture | 模拟 Claude 发出 AskUserQuestion 后返回 `is_error: true` 的 `tool_result`。 |
| `tests/fixtures/provider/codex_request_user_input_peer_closes_fixture.sh` | 测试 fixture | 模拟 Codex 发出 `requestUserInput` 后立即关闭 app-server peer，使 JSON-RPC 响应写入失败。 |

---

## 任务 1：Claude Code — AskUserQuestion 桥接失败时发出 Protocol Error

**涉及文件：**
- 修改：`src/cross_cutting/claude_code_provider.rs:713-715`（control_request 分支）
- 修改：`src/cross_cutting/claude_code_provider.rs:775-777`（tool_use 分支）
- 测试：`src/cross_cutting/claude_code_provider.rs`（新增测试）

### 背景

在 `read_claude_stream` 中，两个 `AskUserQuestion` 路径目前这样写：

```rust
let choice_decision = bridge.request_choice(choice_request, cancel.clone()).await?;
```

如果 bridge 返回错误（例如 event receiver 被关闭、取消等），错误会直接冒泡，最终被外层转成通用的 `ProviderEvent::Failed`。我们需要在返回错误之前先发送 `ProviderEvent::ProtocolError { code: "ask_user_question_unresolved", ... }`。

- [ ] **步骤 1：编写失败测试**

把以下测试加到 `src/cross_cutting/claude_code_provider.rs` 的 `#[cfg(test)] mod tests` 块中：

```rust
#[tokio::test]
async fn claude_provider_ask_user_question_emits_protocol_error_on_bridge_failure() {
    let fixture = executable_fixture("tests/fixtures/provider/claude_ask_user_question_fixture.sh");
    let provider = ClaudeCodeProvider::new(fixture);
    let input = streaming_input(ProviderType::ClaudeCode, ProviderPermissionMode::Supervised);

    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .expect("start provider");

    let choice = loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit choice")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::ChoiceRequest(choice) => break choice,
            ProviderEvent::Failed { message } => {
                panic!("provider failed before choice: {message}")
            }
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error before choice: {message}")
            }
            _ => {}
        }
    };

    // 关闭 receiver，让 bridge 的 event_tx 发送失败。
    drop(session.events);

    // provider 应该抛出 ProtocolError，而不是通用的 Failed。
    let mut saw_protocol_error = false;
    while let Some(event) = tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
        .await
        .unwrap_or(None)
    {
        if matches!(event, ProviderEvent::ProtocolError { code, .. } if code == "ask_user_question_unresolved")
        {
            saw_protocol_error = true;
            break;
        }
    }
    assert!(
        saw_protocol_error,
        "expected ask_user_question_unresolved protocol error after bridge failure"
    );
}
```

- [ ] **步骤 2：运行测试，确认失败**

```bash
cargo test --locked --lib claude_code_provider::tests::claude_provider_ask_user_question_emits_protocol_error_on_bridge_failure -- --nocapture
```

预期：FAIL — provider 目前发出的是 `ProviderEvent::Failed`，不是 `ProtocolError`。

- [ ] **步骤 3：包装 control_request AskUserQuestion 的 choice 调用**

把第 713-715 行的 `let choice_decision = bridge.request_choice(...).await?;` 替换成：

```rust
let choice_decision = match bridge.request_choice(choice_request, cancel.clone()).await {
    Ok(decision) => decision,
    Err(error) => {
        let message = format!("AskUserQuestion control_request unresolved: {}", error.details);
        let _ = send_provider_event(
            &event_tx,
            ProviderEvent::ProtocolError {
                code: "ask_user_question_unresolved".to_string(),
                message: message.clone(),
                context: Some(json!({
                    "request_id": request.request_id,
                    "tool_use_id": request.tool_use_id,
                })),
            },
            &cancel,
        )
        .await;
        return Err(error);
    }
};
```

- [ ] **步骤 4：包装 tool_use AskUserQuestion 的 choice 调用**

把第 775-777 行的 `let choice_decision = bridge.request_choice(...).await?;` 替换成：

```rust
let choice_decision = match bridge.request_choice(choice_request, cancel.clone()).await {
    Ok(decision) => decision,
    Err(error) => {
        let message = format!("AskUserQuestion tool_use unresolved: {}", error.details);
        let _ = send_provider_event(
            &event_tx,
            ProviderEvent::ProtocolError {
                code: "ask_user_question_unresolved".to_string(),
                message: message.clone(),
                context: Some(json!({ "tool_use_id": tool_use.id })),
            },
            &cancel,
        )
        .await;
        return Err(error);
    }
};
```

- [ ] **步骤 5：运行测试，确认通过**

```bash
cargo test --locked --lib claude_code_provider::tests::claude_provider_ask_user_question_emits_protocol_error_on_bridge_failure -- --nocapture
```

预期：PASS。

- [ ] **步骤 6：提交**

```bash
git add src/cross_cutting/claude_code_provider.rs
git commit -m "fix(claude): 桥接失败时发出 ask_user_question_unresolved 协议错误"
```

---

## 任务 2：Claude Code — 识别 AskUserQuestion `tool_result` 的 `is_error`

**涉及文件：**
- 创建：`tests/fixtures/provider/claude_ask_user_question_tool_error_fixture.sh`
- 修改：`src/cross_cutting/claude_code_provider.rs:47-51`（`ToolResultBlock`）
- 修改：`src/cross_cutting/claude_code_provider.rs:173-203`（`parse_tool_result`）
- 修改：`src/cross_cutting/claude_code_provider.rs:829-854`（tool_result 处理）
- 测试：`src/cross_cutting/claude_code_provider.rs`（新增测试）

### 背景

Claude 可能返回带 `"is_error": true` 的 `tool_result`。当前解析器忽略该字段，handler 总是发出完成事件。对于 AskUserQuestion，这种情况必须视为选择未解析，并发出 `ask_user_question_unresolved`。

- [ ] **步骤 1：创建 fixture**

创建 `tests/fixtures/provider/claude_ask_user_question_tool_error_fixture.sh`：

```bash
#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  echo "claude 2.1.160"
  exit 0
fi

while IFS= read -r line; do
  if [[ "$line" == *'"initialize"'* ]]; then
    continue
  fi
  if [[ "$line" == *'"set_permission_mode"'* ]]; then
    continue
  fi
  if [[ "$line" == *'"user"'* ]]; then
    echo '{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_question","name":"AskUserQuestion","input":{"questions":[{"question":"Continue?","options":[{"label":"Yes"},{"label":"No"}]}]}}]}}'
    continue
  fi
  if [[ "$line" == *'"tool_result"'* ]]; then
    echo '{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_question","is_error":true,"content":"User refused to answer"}]}}'
    echo '{"type":"result","subtype":"success","is_error":false,"result":"should not reach here","session_id":"claude_error_session"}'
    exit 0
  fi
done
```

添加可执行权限：

```bash
chmod +x tests/fixtures/provider/claude_ask_user_question_tool_error_fixture.sh
```

- [ ] **步骤 2：编写失败测试**

在 `src/cross_cutting/claude_code_provider.rs` 中新增：

```rust
#[tokio::test]
async fn claude_provider_ask_user_question_emits_protocol_error_on_tool_result_error() {
    let fixture = executable_fixture(
        "tests/fixtures/provider/claude_ask_user_question_tool_error_fixture.sh",
    );
    let provider = ClaudeCodeProvider::new(fixture);
    let input = streaming_input(ProviderType::ClaudeCode, ProviderPermissionMode::Supervised);

    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .expect("start provider");

    let choice = loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit choice")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::ChoiceRequest(choice) => break choice,
            ProviderEvent::Failed { message } => panic!("provider failed before choice: {message}"),
            _ => {}
        }
    };
    assert_eq!(choice.id, "toolu_question");

    session
        .commands
        .send(ProviderCommand::ChoiceResponse {
            id: choice.id,
            selected_option_ids: vec!["opt_0".to_string()],
            free_text: None,
        })
        .await
        .expect("send choice response");

    let mut saw_protocol_error = false;
    while let Some(event) = tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
        .await
        .expect("provider should emit events")
    {
        match event {
            ProviderEvent::ProtocolError { code, .. } if code == "ask_user_question_unresolved" => {
                saw_protocol_error = true;
                break;
            }
            ProviderEvent::Completed { .. } => {
                panic!("provider should not complete after AskUserQuestion tool_result error")
            }
            _ => {}
        }
    }
    assert!(
        saw_protocol_error,
        "expected ask_user_question_unresolved protocol error on tool_result is_error"
    );
}
```

- [ ] **步骤 3：运行测试，确认失败**

```bash
cargo test --locked --lib claude_code_provider::tests::claude_provider_ask_user_question_emits_protocol_error_on_tool_result_error -- --nocapture
```

预期：FAIL — provider 会直接完成，而不是发出 protocol error。

- [ ] **步骤 4：给 `ToolResultBlock` 和 parser 增加 `is_error`**

把结构体（约第 47 行）改成：

```rust
#[derive(Debug, Clone)]
struct ToolResultBlock {
    tool_use_id: String,
    output: String,
    is_error: bool,
}
```

在 `parse_tool_result` 中（约第 181 行），计算完 `output` 后增加：

```rust
let is_error = item.get("is_error").and_then(Value::as_bool).unwrap_or(false);
Some(ToolResultBlock {
    tool_use_id,
    output,
    is_error,
})
```

- [ ] **步骤 5：在流循环中处理 AskUserQuestion tool_result 错误**

在 `if let Some(results) = ClaudeCodeProvider::parse_tool_result(&value)` 块中（约第 829 行），把内部处理替换成：

```rust
for result in results {
    if let Some(tool_use) = pending_tool_uses.remove(&result.tool_use_id) {
        if tool_use.name == "AskUserQuestion" && result.is_error {
            let _ = send_provider_event(
                &event_tx,
                ProviderEvent::ProtocolError {
                    code: "ask_user_question_unresolved".to_string(),
                    message: format!(
                        "AskUserQuestion tool_result error: {}",
                        result.output
                    ),
                    context: Some(json!({
                        "tool_use_id": tool_use.id,
                        "output": result.output,
                    })),
                },
                &cancel,
            )
            .await;
            return Err(ProviderAdapterError::execution_failed(
                None,
                result.output,
                "AskUserQuestion tool_result reported error",
                0,
            ));
        }

        // 原有的 execution event 逻辑保持不变
        let output_preview = output_preview(&result.output, TOOL_RESULT_PREVIEW_MAX_BYTES);
        let command = tool_use_command(&tool_use);
        send_provider_event(
            &event_tx,
            ProviderEvent::Execution(ProviderExecutionEvent {
                event_id: tool_use.id,
                kind: ProviderExecutionEventKind::Command,
                status: ProviderExecutionEventStatus::Completed,
                title: tool_use.name,
                detail: None,
                command,
                cwd: None,
                output: Some(output_preview),
                exit_code: Some(0),
            }),
            &cancel,
        )
        .await?;
    }
}
```

- [ ] **步骤 6：运行测试，确认通过**

```bash
cargo test --locked --lib claude_code_provider::tests::claude_provider_ask_user_question_emits_protocol_error_on_tool_result_error -- --nocapture
```

预期：PASS。

- [ ] **步骤 7：提交**

```bash
git add tests/fixtures/provider/claude_ask_user_question_tool_error_fixture.sh src/cross_cutting/claude_code_provider.rs
git commit -m "fix(claude): 识别 AskUserQuestion tool_result is_error 并发出协议错误"
```

---

## 任务 3：Codex — requestUserInput 桥接/写入失败时发出 Protocol Error

**涉及文件：**
- 创建：`tests/fixtures/provider/codex_request_user_input_peer_closes_fixture.sh`
- 修改：`src/cross_cutting/codex_provider.rs:452-470`
- 测试：`src/cross_cutting/codex_provider.rs`（新增测试）

### 背景

Codex 的 `item/tool/requestUserInput` 分支调用了 `bridge.request_choice(...).await?` 和 `write_user_input_response(...).await?`。两者失败时都需要发出 `ProviderEvent::ProtocolError { code: "request_user_input_unresolved", ... }`。

- [ ] **步骤 1：创建写入失败的 fixture**

创建 `tests/fixtures/provider/codex_request_user_input_peer_closes_fixture.sh`：

```bash
#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  echo "codex 0.133.0"
  exit 0
fi

while IFS= read -r line; do
  if [[ "$line" == *'"initialize"'* ]]; then
    id="$(printf '%s' "$line" | sed -n 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-1},\"result\":{\"userAgent\":\"cadence-aria-test\"}}"
  elif [[ "$line" == *'"thread/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-2},\"result\":{\"thread\":{\"id\":\"codex_input_thread\"},\"approvalPolicy\":\"never\"}}"
  elif [[ "$line" == *'"turn/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-3},\"result\":{\"turn\":{\"id\":\"codex_input_turn\",\"status\":\"inProgress\"}}}"
    echo '{"jsonrpc":"2.0","id":88,"method":"item/tool/requestUserInput","params":{"threadId":"codex_input_thread","turnId":"codex_input_turn","itemId":"ask_1","questions":[{"id":"confirm","header":"确认","question":"继续？","options":[{"label":"是"},{"label":"否"}]}]}}'
    # 立即关闭 peer，让 JSON-RPC 响应写入失败。
    exit 0
  fi
done
```

添加可执行权限：

```bash
chmod +x tests/fixtures/provider/codex_request_user_input_peer_closes_fixture.sh
```

- [ ] **步骤 2：编写失败测试**

在 `src/cross_cutting/codex_provider.rs` 中新增：

```rust
#[tokio::test]
async fn codex_provider_request_user_input_emits_protocol_error_on_bridge_failure() {
    let fixture = executable_fixture("tests/fixtures/provider/codex_app_server_user_input_fixture.sh");
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);

    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let choice = loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit choice")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::ChoiceRequest(choice) => break choice,
            ProviderEvent::Failed { message } => panic!("provider failed before choice: {message}"),
            _ => {}
        }
    };

    // 关闭 receiver 强制 bridge 失败。
    drop(session.events);

    let mut saw_protocol_error = false;
    while let Some(event) = tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
        .await
        .unwrap_or(None)
    {
        if matches!(event, ProviderEvent::ProtocolError { code, .. } if code == "request_user_input_unresolved")
        {
            saw_protocol_error = true;
            break;
        }
    }
    assert!(
        saw_protocol_error,
        "expected request_user_input_unresolved protocol error after bridge failure"
    );
}

#[tokio::test]
async fn codex_provider_request_user_input_emits_protocol_error_on_write_failure() {
    let fixture = executable_fixture(
        "tests/fixtures/provider/codex_request_user_input_peer_closes_fixture.sh",
    );
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);

    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let choice = loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit choice")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::ChoiceRequest(choice) => break choice,
            ProviderEvent::Failed { message } => panic!("provider failed before choice: {message}"),
            _ => {}
        }
    };

    session
        .commands
        .send(ProviderCommand::ChoiceResponse {
            id: choice.id,
            selected_option_ids: vec!["是".to_string()],
            free_text: None,
        })
        .await
        .expect("send choice response");

    let mut saw_protocol_error = false;
    while let Some(event) = tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
        .await
        .expect("provider should emit events")
    {
        if matches!(event, ProviderEvent::ProtocolError { code, .. } if code == "request_user_input_unresolved")
        {
            saw_protocol_error = true;
            break;
        }
    }
    assert!(
        saw_protocol_error,
        "expected request_user_input_unresolved protocol error when JSON-RPC response write fails"
    );
}
```

- [ ] **步骤 3：运行测试，确认失败**

```bash
cargo test --locked --lib codex_provider::tests::codex_provider_request_user_input_emits_protocol_error_on_bridge_failure -- --nocapture
cargo test --locked --lib codex_provider::tests::codex_provider_request_user_input_emits_protocol_error_on_write_failure -- --nocapture
```

预期：都 FAIL — provider 目前发出的是通用 `Failed`，不是 `ProtocolError`。

- [ ] **步骤 4：包装 requestUserInput 的 choice 和写入调用**

把 `if let Some(request) = parse_user_input_request(&incoming)` 块（约第 452 行）替换成：

```rust
if let Some(request) = parse_user_input_request(&incoming) {
    waiting_for_resume_progress = false;
    let decision = match bridge
        .request_choice(
            ChoiceRequestData {
                id: request.id,
                prompt: request.prompt,
                options: request.options,
                allow_multiple: false,
                allow_free_text: request.allow_free_text,
                source: ChoiceRequestSource::RequestUserInput,
            },
            cancel.clone(),
        )
        .await
    {
        Ok(decision) => decision,
        Err(error) => {
            let message = format!("requestUserInput choice bridge failed: {}", error.details);
            let _ = send_provider_event(
                &event_tx,
                ProviderEvent::ProtocolError {
                    code: "request_user_input_unresolved".to_string(),
                    message: message.clone(),
                    context: Some(json!({ "question_id": request.question_id })),
                },
                &cancel,
            )
            .await;
            return Err(error);
        }
    };
    if let Err(error) = write_user_input_response(
        &peer,
        request.rpc_id,
        &request.question_id,
        decision,
    )
    .await
    {
        let message = format!("requestUserInput response write failed: {}", error.details);
        let _ = send_provider_event(
            &event_tx,
            ProviderEvent::ProtocolError {
                code: "request_user_input_unresolved".to_string(),
                message: message.clone(),
                context: Some(json!({ "question_id": request.question_id })),
            },
            &cancel,
        )
        .await;
        return Err(error);
    }
    continue;
}
```

- [ ] **步骤 5：运行测试，确认通过**

```bash
cargo test --locked --lib codex_provider::tests::codex_provider_request_user_input_emits_protocol_error_on_bridge_failure -- --nocapture
cargo test --locked --lib codex_provider::tests::codex_provider_request_user_input_emits_protocol_error_on_write_failure -- --nocapture
```

预期：都 PASS。

- [ ] **步骤 6：提交**

```bash
git add tests/fixtures/provider/codex_request_user_input_peer_closes_fixture.sh src/cross_cutting/codex_provider.rs
git commit -m "fix(codex): requestUserInput 桥接/写入失败时发出协议错误"
```

---

## 任务 4：修复 `run_streaming` 默认实现并删除重复覆盖

**涉及文件：**
- 修改：`src/cross_cutting/streaming_provider.rs:330-370`
- 修改：`src/cross_cutting/claude_code_provider.rs:549-615`（删除）
- 修改：`src/cross_cutting/codex_provider.rs:154-220`（删除）
- 测试：`src/cross_cutting/streaming_provider.rs`（新增测试）

### 背景

默认 `run_streaming` 对 `PermissionRequest` 和 `ChoiceRequest` 使用 `continue` 并丢掉 `session.commands`。如果 provider 发出这类事件，它会永远等待回应。Claude 和 Codex 都重复了这个有问题的实现。

- [ ] **步骤 1：编写失败测试**

在 `src/cross_cutting/streaming_provider.rs` 的 `#[cfg(test)] mod tests` 块中新增：

```rust
use crate::cross_cutting::streaming_provider::{
    ChoiceRequestData, ChoiceRequestSource, ProviderCommand, ProviderEvent,
    ProviderSession, StreamingProviderAdapter, StreamingProviderInput,
};
use async_trait::async_trait;

struct ChoiceEmittingProvider;

#[async_trait]
impl StreamingProviderAdapter for ChoiceEmittingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, crate::cross_cutting::provider_adapter::ProviderAdapterError>
    {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::ChoiceRequest(ChoiceRequestData {
                    id: "choice_001".to_string(),
                    prompt: "Continue?".to_string(),
                    options: vec![],
                    allow_multiple: false,
                    allow_free_text: true,
                    source: ChoiceRequestSource::AskUserQuestion,
                }))
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

#[tokio::test]
async fn run_streaming_declines_choice_request_instead_of_hanging() {
    let provider = ChoiceEmittingProvider;
    let mut rx = provider
        .run_streaming(&make_input("test"), CancellationToken::new())
        .await
        .unwrap();

    let chunk = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("run_streaming 不应在 ChoiceRequest 上挂起")
        .expect("stream 应该发出错误块");

    assert!(
        matches!(chunk, StreamChunk::Error(msg) if msg.contains("choice")),
        "expected error chunk, got {chunk:?}"
    );
}
```

- [ ] **步骤 2：运行测试，确认失败/挂起**

```bash
cargo test --locked --lib streaming_provider::tests::run_streaming_declines_choice_request_instead_of_hanging -- --nocapture
```

预期：FAIL 或 TIMEOUT — 当前选择事件被丢弃，测试挂起。

- [ ] **步骤 3：修复默认 `run_streaming`**

在 `src/cross_cutting/streaming_provider.rs` 中，把事件循环里 `ProviderEvent::PermissionRequest(_) | ProviderEvent::ChoiceRequest(_)` 分支替换成：

```rust
ProviderEvent::PermissionRequest(request) => {
    let _ = session
        .commands
        .send(ProviderCommand::PermissionResponse {
            id: request.id,
            approved: false,
            reason: Some(
                "run_streaming does not support interactive permission requests".to_string(),
            ),
        })
        .await;
    let _ = tx
        .send(StreamChunk::Error(
            "interactive permission request is not supported in run_streaming".to_string(),
        ))
        .await;
    return;
}
ProviderEvent::ChoiceRequest(request) => {
    let _ = session
        .commands
        .send(ProviderCommand::ChoiceResponse {
            id: request.id,
            selected_option_ids: vec![],
            free_text: Some("aborted".to_string()),
        })
        .await;
    let _ = tx
        .send(StreamChunk::Error(
            "interactive choice request is not supported in run_streaming".to_string(),
        ))
        .await;
    return;
}
```

同时删除 `FakeStreamingProvider::run_streaming` 覆盖（第 437-511 行），因为修复后的默认实现已足够。

- [ ] **步骤 4：删除 Claude provider 的重复 `run_streaming`**

删除 `src/cross_cutting/claude_code_provider.rs` 中整个 `async fn run_streaming` 块（第 549-615 行）。保留 `AdapterInput` import，因为测试 helper `adapter_input` 还需要它。

- [ ] **步骤 5：删除 Codex provider 的重复 `run_streaming`**

删除 `src/cross_cutting/codex_provider.rs` 中整个 `async fn run_streaming` 块（第 154-220 行）。同时删除文件顶部不再使用的 `AdapterInput` import。

- [ ] **步骤 6：运行测试，确认通过**

```bash
cargo test --locked --lib streaming_provider::tests::run_streaming_declines_choice_request_instead_of_hanging -- --nocapture
```

预期：PASS。

- [ ] **步骤 7：提交**

```bash
git add src/cross_cutting/streaming_provider.rs src/cross_cutting/claude_code_provider.rs src/cross_cutting/codex_provider.rs
git commit -m "fix(streaming): run_streaming 自动拒绝交互请求并删除 provider 重复覆盖"
```

---

## 任务 5：全量回归验证

- [ ] **步骤 1：运行 provider 单元测试**

```bash
cargo test --locked --lib claude_code_provider
cargo test --locked --lib codex_provider
cargo test --locked --lib streaming_provider
```

预期：全部 PASS。

- [ ] **步骤 2：运行格式化和 clippy 检查**

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
```

预期：无格式化错误，无 clippy 警告。

- [ ] **步骤 3：运行完整测试套件**

```bash
cargo test --locked
```

预期：全部 PASS。

- [ ] **步骤 4：提交最终修复**

```bash
git add -A
git commit -m "chore: 修复 provider choice bridge 错误处理后的格式与 clippy"
```

---

## 自检清单

1. **需求覆盖：** 原始总结要求：
   - Claude Code `AskUserQuestion` 桥接失败 → protocol error → **任务 1**。
   - Claude Code `tool_result` `is_error` 处理 → **任务 2**。
   - Codex `requestUserInput` 桥接/写入失败 → protocol error → **任务 3**。
   - Codex 与 Claude 存在同样问题 → 任务 3 对称处理。
   - `run_streaming` 遇到交互请求挂起 → **任务 4**。

2. **占位符检查：** 无 TBD/TODO/"后续补充"/"类似任务 N" 等模糊描述。每一步都有精确文件路径、代码片段和命令。

3. **类型一致性：** 所有引用均使用 `src/cross_cutting/streaming_provider.rs` 中已定义的 `ProviderEvent::ProtocolError { code, message, context }` 形状，以及 `ProviderCommand::ChoiceResponse` / `ProviderCommand::PermissionResponse` 变体。

---

## 执行交接

方案已完成并保存到 `cadence/plans/2026-06-15-provider-choice-bridge-error-handling.md`。

两种执行方式可选：

1. **Subagent-Driven（推荐）** — 每个任务派一个独立子 agent，我逐 task 审核。必须使用子技能 `superpowers:subagent-driven-development`。
2. **Inline Execution** — 在当前会话按 plan 批量执行，中间设置 checkpoint 供你审核。必须使用子技能 `superpowers:executing-plans`。

你想用哪种方式？
