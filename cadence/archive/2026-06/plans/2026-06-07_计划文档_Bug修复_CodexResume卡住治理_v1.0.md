# Codex Resume 卡住治理 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 Coding Workspace 第二轮 `Coder · Codex` resume 卡住问题，并将非 Coder 节点默认改为 fresh provider thread。

**Architecture:** Codex provider 在有 `resume_provider_session_id` 时先调用 app-server `thread/resume`，再调用 `turn/start`。JSON-RPC peer 增加 request timeout，Coding Workspace 增加角色级 resume policy，只允许 Coder 默认 resume，其余角色仍记录 provider session 但启动时不续接。

**Tech Stack:** Rust 1.95、Tokio、serde_json、Codex app-server JSON-RPC、Coding Workspace engine、Cargo 测试套件。

---

## File Structure

- Modify: `tests/fixtures/provider/codex_app_server_resume_fixture.sh`
  - 将 resume fixture 从“直接允许 `turn/start`”改为“必须先收到 `thread/resume`”。
- Modify: `src/cross_cutting/codex_provider.rs`
  - resume 分支发送 `thread/resume`。
  - 关键 JSON-RPC request 使用 timeout。
- Modify: `src/cross_cutting/json_rpc_peer.rs`
  - 新增 `request_with_timeout`。
  - timeout 后清理 pending request。
- Modify: `src/cross_cutting/provider_adapter.rs`
  - 增加可自定义 details 的 timeout 构造器，便于 UI 显示具体 JSON-RPC 方法。
- Modify: `src/product/coding_workspace_engine.rs`
  - 增加 `should_resume_provider_conversation`。
  - `provider_resume_session_id_for_attempt` 按策略过滤，默认只允许 Coder。
- Modify: `tests/it_product/product_coding_workspace_engine.rs`
  - 更新 tester、analyst、code reviewer、internal reviewer 的 resume 断言。

## Task 1: Codex app-server resume 使用 `thread/resume`

**Files:**
- Modify: `tests/fixtures/provider/codex_app_server_resume_fixture.sh`
- Modify: `src/cross_cutting/codex_provider.rs`

- [ ] **Step 1: 写 failing fixture**

将 `tests/fixtures/provider/codex_app_server_resume_fixture.sh` 改为：

````bash
#!/usr/bin/env bash
set -euo pipefail

saw_resume=0

while IFS= read -r line; do
  if [[ "$line" == *'"method":"initialize"'* ]]; then
    echo '{"id":1,"result":{"capabilities":{}}}'
  elif [[ "$line" == *'"method":"initialized"'* ]]; then
    :
  elif [[ "$line" == *'"method":"thread/start"'* ]]; then
    echo '{"id":999,"error":{"code":-32000,"message":"thread/start must not be called during resume"}}' >&2
    exit 1
  elif [[ "$line" == *'"method":"thread/resume"'* ]]; then
    if [[ "$line" != *'"threadId":"codex-thread-123"'* ]]; then
      echo '{"id":2,"error":{"code":-32001,"message":"unexpected resume threadId"}}' >&2
      exit 1
    fi
    saw_resume=1
    echo '{"id":2,"result":{"thread":{"id":"codex-thread-123"}}}'
  elif [[ "$line" == *'"method":"turn/start"'* ]]; then
    if [[ "$saw_resume" != "1" ]]; then
      echo '{"id":3,"error":{"code":-32002,"message":"turn/start before thread/resume"}}' >&2
      exit 1
    fi
    if [[ "$line" != *'"threadId":"codex-thread-123"'* ]]; then
      echo '{"id":3,"error":{"code":-32003,"message":"unexpected turn threadId"}}' >&2
      exit 1
    fi
    echo '{"id":3,"result":{"turn":{"id":"turn-1"}}}'
    echo '{"method":"item/completed","params":{"item":{"id":"msg-1","type":"agentMessage","text":"resumed done"}}}'
    echo '{"method":"turn/completed","params":{"turnId":"turn-1"}}'
    exit 0
  fi
done
````

- [ ] **Step 2: 运行测试确认失败**

Run:

````bash
cargo test --locked --lib codex_resume_uses_existing_thread_without_starting_new_thread
````

Expected: FAIL，fixture 报 `turn/start before thread/resume` 或 provider failed。

- [ ] **Step 3: 修改 CodexProvider resume 分支**

在 `run_codex_session` 中：

````rust
let thread_id = if let Some(session_id) = input
    .resume_provider_session_id
    .as_deref()
    .map(str::trim)
    .filter(|session_id| !session_id.is_empty())
{
    let resume_response = peer
        .request_with_timeout(json!({
            "jsonrpc": "2.0",
            "method": "thread/resume",
            "params": {
                "threadId": session_id,
                "cwd": input.working_dir.clone(),
                "approvalPolicy": match input.permission_mode {
                    ProviderPermissionMode::Auto => "never",
                    ProviderPermissionMode::Supervised => "on-request",
                },
            },
        }), CODEX_RPC_REQUEST_TIMEOUT)
        .await?;
    resume_response
        .pointer("/thread/id")
        .or_else(|| resume_response.pointer("/id"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or_else(|| Some(session_id.to_string()))
} else {
    ...
};
````

- [ ] **Step 4: 运行测试确认通过**

Run:

````bash
cargo test --locked --lib codex_resume_uses_existing_thread_without_starting_new_thread
````

Expected: PASS。

## Task 2: JSON-RPC request timeout

**Files:**
- Modify: `src/cross_cutting/json_rpc_peer.rs`
- Modify: `src/cross_cutting/provider_adapter.rs`
- Modify: `src/cross_cutting/codex_provider.rs`

- [ ] **Step 1: 写 failing 单测**

在 `src/cross_cutting/json_rpc_peer.rs` tests 中增加：

````rust
#[tokio::test]
async fn json_rpc_peer_times_out_and_removes_pending_request() {
    let (client_io, server_io) = tokio::io::duplex(4096);
    let (reader, writer) = tokio::io::split(client_io);
    let peer = JsonRpcPeer::new(reader, writer);

    tokio::spawn(async move {
        let (server_reader, _server_writer) = tokio::io::split(server_io);
        let mut line = String::new();
        let mut reader = tokio::io::BufReader::new(server_reader);
        reader.read_line(&mut line).await.unwrap();
    });

    let result = peer
        .request_with_timeout(
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "turn/start",
                "params": {},
            }),
            Duration::from_millis(10),
        )
        .await;

    let error = result.expect_err("request should time out");
    assert!(error.details.contains("turn/start"));
    assert!(peer.pending.lock().await.is_empty());
}
````

- [ ] **Step 2: 运行测试确认失败**

Run:

````bash
cargo test --locked --lib json_rpc_peer_times_out_and_removes_pending_request
````

Expected: 编译失败，`request_with_timeout` 不存在。

- [ ] **Step 3: 实现 timeout error 构造器**

在 `ProviderAdapterError` 增加：

````rust
pub fn timeout_with_details(
    details: impl Into<String>,
    stdout: impl Into<String>,
    stderr: impl Into<String>,
    duration_ms: u64,
) -> Self {
    Self::with_output(
        ProviderErrorCode::ProviderTimeout,
        details,
        stdout,
        stderr,
        None,
        TimeoutStatus::HardTimeoutKilled,
        duration_ms,
    )
}
````

- [ ] **Step 4: 实现 `request_with_timeout`**

在 `JsonRpcPeer` 中抽取内部 request 实现，新增：

````rust
pub async fn request_with_timeout(
    &self,
    payload: Value,
    timeout: Duration,
) -> Result<Value, ProviderAdapterError>
````

timeout 后删除 pending request，并返回包含 method 的 timeout error。

- [ ] **Step 5: CodexProvider 使用 request timeout**

增加：

````rust
const CODEX_RPC_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
````

并将 `initialize`、`thread/start`、`thread/resume`、`turn/start` 改为 `request_with_timeout(..., CODEX_RPC_REQUEST_TIMEOUT)`。

- [ ] **Step 6: 运行 timeout 测试**

Run:

````bash
cargo test --locked --lib json_rpc_peer_times_out_and_removes_pending_request
cargo test --locked --lib codex_resume_uses_existing_thread_without_starting_new_thread
````

Expected: PASS。

## Task 3: Coding Workspace 只默认 resume Coder

**Files:**
- Modify: `src/product/coding_workspace_engine.rs`
- Modify: `tests/it_product/product_coding_workspace_engine.rs`

- [ ] **Step 1: 更新现有 failing 测试断言**

将以下测试中的期望改为 `None`：

- `coding_tester_does_not_resume_coder_provider_session`
- `coding_code_reviewer_run_uses_fresh_provider_session`
- `coding_analyst_rework_uses_fresh_provider_session`
- `coding_internal_reviewer_uses_fresh_provider_session`

Coder 测试 `coding_coder_run_resumes_previous_coder_provider_session` 保持不变。

- [ ] **Step 2: 运行测试确认失败**

Run:

````bash
cargo test --locked --test it_product coding_tester_does_not_resume_coder_provider_session
cargo test --locked --test it_product coding_code_reviewer_run_uses_fresh_provider_session
cargo test --locked --test it_product coding_analyst_rework_uses_fresh_provider_session
cargo test --locked --test it_product coding_internal_reviewer_uses_fresh_provider_session
````

Expected: FAIL，当前实现仍会传入各自旧 session id。

- [ ] **Step 3: 增加 resume policy**

在 `src/product/coding_workspace_engine.rs` 增加：

````rust
fn should_resume_provider_conversation(role: &CodingProviderRole) -> bool {
    matches!(role, CodingProviderRole::Coder)
}
````

在 `provider_resume_session_id_for_attempt` 开头增加：

````rust
if !should_resume_provider_conversation(role) {
    return None;
}
````

- [ ] **Step 4: 更新单元测试**

将 `coding_provider_resume_session_id_is_isolated_by_role_and_provider` 的 tester 断言改为 `None`，并增加说明 Coder 仍可 resume。

- [ ] **Step 5: 运行策略测试**

Run:

````bash
cargo test --locked --lib coding_provider_resume_session_id_is_isolated_by_role_and_provider
cargo test --locked --test it_product coding_coder_run_resumes_previous_coder_provider_session
cargo test --locked --test it_product coding_tester_does_not_resume_coder_provider_session
cargo test --locked --test it_product coding_code_reviewer_run_uses_fresh_provider_session
cargo test --locked --test it_product coding_analyst_rework_uses_fresh_provider_session
cargo test --locked --test it_product coding_internal_reviewer_uses_fresh_provider_session
````

Expected: PASS。

## Task 4: 最终验证

**Files:**
- All changed Rust and fixture files.

- [ ] **Step 1: 格式化**

Run:

````bash
cargo fmt
````

- [ ] **Step 2: 快速检查**

Run:

````bash
cargo check --locked
````

Expected: PASS。

- [ ] **Step 3: 关键回归测试**

Run:

````bash
cargo test --locked --lib codex_resume_uses_existing_thread_without_starting_new_thread
cargo test --locked --lib json_rpc_peer_times_out_and_removes_pending_request
cargo test --locked --lib coding_provider_resume_session_id_is_isolated_by_role_and_provider
cargo test --locked --test it_product coding_coder_run_resumes_previous_coder_provider_session
cargo test --locked --test it_product coding_tester_does_not_resume_coder_provider_session
cargo test --locked --test it_product coding_code_reviewer_run_uses_fresh_provider_session
cargo test --locked --test it_product coding_analyst_rework_uses_fresh_provider_session
cargo test --locked --test it_product coding_internal_reviewer_uses_fresh_provider_session
````

Expected: PASS。

- [ ] **Step 4: 标准验证**

Run:

````bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
````

Expected: PASS。
