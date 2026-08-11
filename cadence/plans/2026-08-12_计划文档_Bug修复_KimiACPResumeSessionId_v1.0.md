# Kimi ACP Resume Session ID 兼容修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Kimi CLI 0.34 的 `session/load` 成功响应即使不含 `sessionId`，也能继续使用请求中的 provider session 恢复并完成当前 prompt。

**Architecture:** `session/new` 与 `session/load` 的 response contract 不同：前者必须提供新建 session ID，后者可只返回配置元数据。保持服务端对错误 JSON-RPC response 的 fail-closed 行为；仅在已成功的 `session/load` 且 response 缺少有效 ID 时复用已验证的请求 ID。

**Tech Stack:** Rust 2024、Tokio、JSON-RPC、Cargo 测试 fixture（Bash）。

## Global Constraints

- 已通过只读 ACP 探针确认：Kimi CLI `0.34.0` 的 `session/load` 返回 `result.configOptions` 与 `result.modes`，不返回 `result.sessionId`。
- `session/new` 缺少有效 `sessionId` 必须继续报错；不得把本修复扩展为 `session/new` fallback。
- `session/load` 返回 JSON-RPC error 时，仍不得发送 `session/new` 或 `session/prompt`。
- 新测试必须先在旧实现上失败，再改生产代码使其通过。
- 不新增依赖；Cargo 命令不得带 `-j 1`；不调用真实 Kimi prompt 或其他 Provider。
- 改动仅限 Kimi ACP adapter、其回归测试与 checked-in fixture。

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

## Plan 自检

- 覆盖范围：Task 1 覆盖真实 Kimi load response、严格 new response contract、成功恢复、已有 load error fail-closed 回归。
- 类型一致性：生产代码继续使用现有 `Option<String>` 的 `resume_id` 与 `session_id: String`，不新增接口或依赖。
- 占位扫描：没有未具体化的测试、实现或命令步骤。
