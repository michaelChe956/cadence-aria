# Kimi ACP Resume Session ID 兼容修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Kimi CLI 0.34 的 `session/load` 成功响应即使不含 `sessionId`，也能在历史回放较多时继续使用请求中的 provider session 恢复并完成当前 prompt。

**Architecture:** `session/new` 与 `session/load` 的 response contract 不同：前者必须提供新建 session ID，后者可只返回配置元数据。保持服务端对错误 JSON-RPC response 的 fail-closed 行为；仅在已成功的 `session/load` 且 response 缺少有效 ID 时复用已验证的请求 ID。等待控制响应期间，读取器必须继续消费 Kimi 的历史通知，避免有界通知队列反压阻塞后续 JSON-RPC response；历史回放只用于完成恢复握手，不得伪装成当前 prompt 的新输出。

**Tech Stack:** Rust 2024、Tokio、JSON-RPC、Cargo 测试 fixture（Bash）。

## Global Constraints

- 已通过只读 ACP 探针确认：Kimi CLI `0.34.0` 的 `session/load` 返回 `result.configOptions` 与 `result.modes`，不返回 `result.sessionId`。
- `session/new` 缺少有效 `sessionId` 必须继续报错；不得把本修复扩展为 `session/new` fallback。
- `session/load` 返回 JSON-RPC error 时，仍不得发送 `session/new` 或 `session/prompt`。
- 新测试必须先在旧实现上失败，再改生产代码使其通过。
- 不新增依赖；Cargo 命令不得带 `-j 1`；不调用真实 Kimi prompt 或其他 Provider。
- 改动仅限 Kimi ACP adapter、其回归测试与 checked-in fixture。
- `session/load` 的历史回放不得写入当前运行的输出或 timeline；`session/prompt` 开始后收到的新通知仍必须按现有逻辑完整处理。

---

### Task 1: Kimi ACP 无 sessionId 的恢复响应兼容

**Files:**

- Create: `tests/fixtures/provider/kimi_acp_resume_without_session_id_fixture.sh`
- Modify: `src/cross_cutting/kimi_code_provider/tests.rs`
- Modify: `src/cross_cutting/kimi_code_provider/session.rs`

**Interfaces:**

- Consumes: `StreamingProviderInput.resume_provider_session_id: Option<String>` 与 Kimi ACP 的 `session/load` response。
- Produces: 成功完成时 `ProviderCompletion.provider_session_id`；无返回 ID 的 load 路径必须保留请求 ID `existing_session`。

- [ ] **Step 1: 添加失败回归 fixture 与测试**

创建 `kimi_acp_resume_without_session_id_fixture.sh`：对 `initialize` 返回协议版本 1 及 `loadSession/resume` capability；对 `session/load` 返回 `{"configOptions":[],"modes":[]}`；仅在收到 `session/prompt` 后发送 `agent_message_chunk`（`sessionId` 为 `existing_session`）和 `stopReason: end_turn`。

在 `tests.rs` 新增以下测试，使用该 fixture；断言完整输出为 `resumed`，且 completion 的 provider session ID 为请求 ID：

```rust
#[tokio::test]
async fn resume_load_without_session_id_reuses_requested_session_id() {
    let provider = KimiCodeProvider::new(fixture_command(
        "kimi_acp_resume_without_session_id_fixture.sh",
    ));
    let mut session = provider
        .start(input(Some("existing_session"), 10), CancellationToken::new())
        .await
        .expect("start");
    let events = terminal_events(&mut session).await;
    let completion = events
        .iter()
        .find_map(|event| match event {
            ProviderEvent::Completed(completion) => Some(completion),
            _ => None,
        })
        .expect("completion");
    assert_eq!(completion.full_output, "resumed");
    assert_eq!(
        completion.provider_session_id.as_deref(),
        Some("existing_session")
    );
}
```

- [ ] **Step 2: 运行定向测试并确认 RED**

运行：`cargo test --locked --lib resume_load_without_session_id_reuses_requested_session_id`

预期：测试因为当前 adapter 对 `session/load` response 强制要求 `sessionId` 而找不到 `ProviderEvent::Completed` 失败；不得因 fixture 语法或测试编译错误失败。

- [ ] **Step 3: 以最小逻辑实现恢复 ID fallback**

在 `run_kimi_session_inner` 解析 session response 时，保留优先使用 response 中非空 `sessionId` 的逻辑。仅当该值缺失且 `resume_id` 存在时，以 `resume_id.clone()` 作为 `session_id`；若两者皆无，保留现有 `Kimi ACP {session_method} response did not contain sessionId` 错误。

```rust
let session_id = session_response
    .get("sessionId")
    .and_then(Value::as_str)
    .filter(|id| !id.trim().is_empty())
    .map(ToString::to_string)
    .or_else(|| resume_id.clone())
    .ok_or_else(|| {
        provider_error(format!(
            "Kimi ACP {session_method} response did not contain sessionId"
        ))
    })?;
```

- [ ] **Step 4: 运行 GREEN 与相邻回归测试**

运行：

```bash
cargo test --locked --lib resume_load_without_session_id_reuses_requested_session_id
cargo test --locked --lib resume_uses_session_load_when_resume_id_present
cargo test --locked --lib resume_load_failure_never_falls_back_to_new_or_prompt
cargo fmt --check
```

预期：所有命令退出码为 0；新建测试通过，原有 resume 成功与 load error fail-closed 行为不变。

- [ ] **Step 5: 原子提交**

```bash
git add src/cross_cutting/kimi_code_provider/session.rs \
  src/cross_cutting/kimi_code_provider/tests.rs \
  tests/fixtures/provider/kimi_acp_resume_without_session_id_fixture.sh
git commit -m "fix(kimi): reuse requested session id after load"
```

提交不得包含 `.aria/`、`target/`、前端构建产物、临时 probe 或无关文件。

### Task 2: Kimi ACP session/load 历史回放背压兼容

**Files:**

- Modify: `src/cross_cutting/json_rpc_peer.rs`
- Modify: `src/cross_cutting/kimi_code_provider/session.rs`
- Modify: `src/cross_cutting/kimi_code_provider/tests.rs`

**Interfaces:**

- Consumes: Kimi ACP `session/load` 在 terminal JSON-RPC response 前发送的历史 `session/update` 通知。
- Produces: 控制请求在通知回放期间仍能收到匹配的 JSON-RPC response；恢复完成后当前 prompt 的新通知继续进入现有解析与事件流。

- [ ] **Step 1: 添加超过通知队列容量的失败回归测试**

在 `tests.rs` 添加一个真实 `JsonRpcPeer`/Kimi session fixture 测试：`session/load` response 前发送至少 33 条历史 `session/update` 通知，再发送 `id=2` 的成功 response，随后发送当前 prompt 的一条新 `agent_message_chunk` 与 `stopReason=end_turn`。测试必须断言：

1. session/load 在短测试超时内成功完成，而不是 `JSON-RPC request session/load timed out`；
2. 历史回放不会出现在本次 completion 的 `full_output` 中；
3. 当前 prompt 的新消息仍出现在 completion 输出中，且 provider session ID 仍为请求的 resume ID。

- [ ] **Step 2: 运行定向测试并确认 RED**

运行：`cargo test --locked --lib resume_load_replay_over_queue_capacity_does_not_timeout`

预期：旧实现因 `JsonRpcPeer` 的 32 条 incoming 队列在第 33 条历史通知处反压，测试按预期失败；不得因测试编译错误或 fixture 错误失败。

- [ ] **Step 3: 实现最小的控制阶段通知消费逻辑**

修改 `request_control` 或其紧邻抽象，使等待 `initialize`/`session/load` 控制响应时，JSON-RPC reader 不会因历史通知填满有界队列而停读。对 `session/load` 等待阶段消费并丢弃历史回放通知；不得把它们转发为当前 prompt 的 `ProviderEvent`。保持 `session/prompt` 之后现有 `handle_incoming` 流程不变。

实现不得仅把 mpsc 容量机械调大；必须保留有界内存与 response 按 request id 匹配语义。`session/load` error 仍 fail-closed，不发送 `session/new` 或 `session/prompt`。

- [ ] **Step 4: 运行 GREEN 与相邻回归测试**

运行：

```bash
cargo test --locked --lib resume_load_replay_over_queue_capacity_does_not_timeout
cargo test --locked --lib resume_load_without_session_id_reuses_requested_session_id
cargo test --locked --lib resume_uses_session_load_when_resume_id_present
cargo test --locked --lib resume_load_failure_never_falls_back_to_new_or_prompt
```

预期：新增背压回归、无 sessionId fallback、正常 resume 与 load error fail-closed 均通过；历史通知不污染当前 prompt 输出。

- [ ] **Step 5: 原子提交**

```bash
git add src/cross_cutting/json_rpc_peer.rs \
  src/cross_cutting/kimi_code_provider/session.rs \
  src/cross_cutting/kimi_code_provider/tests.rs
git commit -m "fix(kimi): drain load replay before response"
```

提交不得包含 `.aria/`、`target/`、前端构建产物、临时 probe 或无关文件。

## Plan 自检

- 覆盖范围：Task 1 覆盖真实 Kimi load response、严格 new response contract、成功恢复、已有 load error fail-closed 回归；Task 2 覆盖超过队列容量的 load 历史回放、当前 prompt 输出隔离与后续 response 可达性。
- 类型一致性：生产代码继续使用现有 `Option<String>` 的 `resume_id` 与 `session_id: String`，不新增接口或依赖。
- 占位扫描：没有未具体化的测试、实现或命令步骤。
