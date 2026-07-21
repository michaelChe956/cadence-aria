# Coding Provider 无响应恢复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Coding Coder 的 Codex continuation 无进展时自动使用完整 Prompt 新建会话重试一次，重复失败时进入可继续操作的恢复 Gate，并在修复后把 `coding_attempt_0001` 回退到 Draft 4 尚未启动。

**Architecture:** Coder 调用同时准备 continuation 输入和一次性 Fresh Retry 输入，Provider stream 在持久化失败前识别稳定的 resume-stall 标记并切换输入。若最终仍失败，Coding Workspace 将 Attempt 置为 `blocked` 并创建 `retry_coding` Gate；Runner 对已持久化的可恢复状态不再发送致命 `coding_start_failed`。

**Tech Stack:** Rust 2024、Tokio、Axum WebSocket、Serde JSON、React 19、TypeScript、Vitest、pnpm。

## Global Constraints

- 所有实现位于 `.worktrees/feat-b-0709`，不得覆盖该 worktree 现有未提交修改。
- Bug 修复严格执行 TDD：先看到失败测试，再写最小实现。
- Rust 命令必须使用宿主机 Cargo 和 `--locked`；禁止 `-j 1`。
- 前端必须使用 pnpm；禁止 npm 和 yarn。
- 自动 Fresh Retry 最多一次，仅适用于 Codex Coder 且首次输入带 continuation。
- Fresh Retry 必须使用完整 Coding Prompt，并清除 Coder continuation。
- 重复失败必须保留 worktree 修改，Attempt 进入 `blocked`，不得进入不可操作的 `failed`。
- `shared_worktree_dirty_manual_gate` 仍保留在完成、终止、删除和释放共享锁流程。
- 设计依据：`cadence/designs/2026-07-13_技术方案_CodingProvider无响应恢复_v1.0.md`。

---

## 文件职责映射

- `src/cross_cutting/codex_provider/mod.rs`：提供共享的 Codex resume-stall 稳定判定。
- `src/product/coding_workspace_engine/lifecycle.rs`：按角色和 Provider 清除单个 conversation。
- `src/product/coding_workspace_engine/types.rs`：定义一次性 Fresh Retry 输入。
- `src/product/coding_workspace_engine/coding.rs`：同时构造 continuation 与完整 Prompt Fresh Retry 输入。
- `src/product/coding_workspace_engine/rework.rs`：在 Reviewer 驱动的 Coder 返修路径同时构造 continuation 与完整 Prompt Fresh Retry 输入。
- `src/product/coding_workspace_engine/provider_stream.rs`：在同一逻辑 role run 中切换到 Fresh Retry。
- `src/product/coding_workspace_engine/code_review.rs`、`internal_pr_review.rs`、`testing_provider/plan.rs`、`testing_provider/report.rs`：非 Coder 调用点显式关闭 Fresh Retry。
- `src/product/coding_workspace_engine/gates.rs`：把最终 Coder Provider 中断转为可恢复 Gate。
- `src/product/coding_workspace_engine/testing_parser.rs`：提供 `retry_coding` Gate action。
- `src/product/coding_models/gate.rs`：增加 `RetryCoding` action type。
- `src/web/coding_ws_handler/runner.rs`：允许 Gate 恢复 Runner，并抑制可恢复状态的致命 Protocol Error。
- `web/src/api/types/coding.ts`：同步 `retry_coding` 前端类型。
- `web/src/pages/CodingWorkspacePage.gates.test.tsx`：验证恢复按钮无需额外上下文即可发送。
- `src/product/coding_workspace_engine/tests/coder_resume_recovery.rs`：覆盖自动 Fresh Retry 和重复失败。
- `src/product/coding_workspace_engine/tests/provider_failure_recovery.rs`：覆盖 Coder interruption Gate 与 GateResponse。
- `src/web/coding_ws_handler/tests.rs`：覆盖 Runner 恢复与错误抑制判定。

---

### Task 1: 共享 resume-stall 判定与 Coder conversation 清理

**Files:**
- Modify: `src/cross_cutting/codex_provider/mod.rs:32`
- Modify: `src/product/coding_workspace_engine/lifecycle.rs:29-90`
- Test: `src/product/coding_workspace_engine/tests/provider_driven.rs:230-299`

**Interfaces:**
- Produces: `crate::cross_cutting::codex_provider::is_resume_stall_failure(message: &str) -> bool`
- Produces: `CodingWorkspaceEngine::clear_attempt_provider_conversation(attempt, role, provider) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError>`
- Consumes: `CodingAttemptStore::replace_attempt_provider_conversations`

- [ ] **Step 1: 写 conversation 窄删除失败测试**

在 `provider_driven.rs` 增加测试，构造 Coder 与 Code Reviewer 两个 conversation，只删除 Coder：

```rust
#[test]
fn clearing_coder_conversation_preserves_other_roles() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let attempt = store
        .replace_attempt_provider_conversations(
            &attempt.id,
            vec![
                ProviderConversationRef {
                    role: ProviderConversationRole::Coder,
                    provider: ProviderName::Codex,
                    provider_session_id: "stale-coder-thread".to_string(),
                    updated_at: "2026-07-13T00:00:00Z".to_string(),
                    last_node_id: Some("coding_node_0001".to_string()),
                },
                ProviderConversationRef {
                    role: ProviderConversationRole::CodeReviewer,
                    provider: ProviderName::ClaudeCode,
                    provider_session_id: "review-thread".to_string(),
                    updated_at: "2026-07-13T00:00:01Z".to_string(),
                    last_node_id: Some("coding_node_0002".to_string()),
                },
            ],
        )
        .expect("seed conversations");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);

    let updated = engine
        .clear_attempt_provider_conversation(
            &attempt,
            &CodingProviderRole::Coder,
            &ProviderName::Codex,
        )
        .expect("clear coder conversation");

    assert_eq!(updated.provider_conversations.len(), 1);
    assert_eq!(
        updated.provider_conversations[0].role,
        ProviderConversationRole::CodeReviewer
    );
}
```

- [ ] **Step 2: 运行测试确认 RED**

Run: `cargo test --locked --lib clearing_coder_conversation_preserves_other_roles`

Expected: 编译失败，提示 `clear_attempt_provider_conversation` 不存在。

- [ ] **Step 3: 实现共享判定和 conversation 清理**

在 `codex_provider/mod.rs` 增加：

```rust
pub(crate) fn is_resume_stall_failure(message: &str) -> bool {
    message.contains(CODEX_RESUME_STALL_ERROR)
}
```

Coding Workspace 后续直接调用该函数。`src/product/workspace_engine/mod.rs` 保持当前实现不变，避免覆盖该文件中的用户未提交修改。

在 `coding_workspace_engine/lifecycle.rs` 增加：

```rust
pub(crate) fn clear_attempt_provider_conversation(
    &self,
    attempt: &CodingExecutionAttempt,
    role: &CodingProviderRole,
    provider: &ProviderName,
) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
    let conversation_role = provider_conversation_role_for_coding_role(role);
    let conversations = attempt
        .provider_conversations
        .iter()
        .filter(|conversation| {
            conversation.role != conversation_role || &conversation.provider != provider
        })
        .cloned()
        .collect();
    self.store
        .replace_attempt_provider_conversations(&attempt.id, conversations)
        .map_err(CodingWorkspaceEngineError::from)
}
```

- [ ] **Step 4: 验证 GREEN**

Run:

```bash
cargo test --locked --lib clearing_coder_conversation_preserves_other_roles
```

Expected: 命令退出码为 0。

- [ ] **Step 5: 独立提交**

```bash
git add src/cross_cutting/codex_provider/mod.rs src/product/coding_workspace_engine/lifecycle.rs src/product/coding_workspace_engine/tests/provider_driven.rs
git commit -m "refactor: share codex resume stall detection"
```

---

### Task 2: Coder continuation 自动 Fresh Retry

**Files:**
- Create: `src/product/coding_workspace_engine/tests/coder_resume_recovery.rs`
- Modify: `src/product/coding_workspace_engine/tests.rs:20-30`
- Modify: `src/product/coding_workspace_engine/types.rs:64-78`
- Modify: `src/product/coding_workspace_engine/coding.rs:36-175`
- Modify: `src/product/coding_workspace_engine/rework.rs:3-205`
- Modify: `src/product/coding_workspace_engine/provider_stream.rs:48-285`
- Modify: `src/product/coding_workspace_engine/code_review.rs:105-120`
- Modify: `src/product/coding_workspace_engine/internal_pr_review.rs:228-245`
- Modify: `src/product/coding_workspace_engine/testing_provider/plan.rs:104-122,197-215`
- Modify: `src/product/coding_workspace_engine/testing_provider/report.rs:116-134`

**Interfaces:**
- Consumes: `is_resume_stall_failure` 与 `clear_attempt_provider_conversation`
- Produces: `CodingProviderFreshRetry { legacy_input: AdapterInput, input: StreamingProviderInput }`
- Extends: `CodingProviderStreamRun::fresh_retry: Option<CodingProviderFreshRetry>`

- [ ] **Step 1: 写初始 Coding 与 Reviewer 返修两条 stale continuation 自动重试失败测试**

新建 `coder_resume_recovery.rs`，定义脚本 Provider：首次收到 resume ID 时发送稳定 stall 错误，第二次无 resume ID 时完成。分别通过 `execute_coding_with_commands` 和 `execute_coder_fix_from_review` 驱动，确保现场实际经过的 `rework.rs` 路径也被覆盖。

```rust
#[derive(Default)]
struct ResumeStallThenFreshSuccessProvider {
    inputs: Mutex<Vec<StreamingProviderInput>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ResumeStallThenFreshSuccessProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let resumed = input.resume_provider_session_id.is_some();
        self.inputs.lock().expect("inputs").push(input);
        let (event_tx, event_rx) = mpsc::channel(4);
        tokio::spawn(async move {
            if resumed {
                let _ = event_tx
                    .send(ProviderEvent::Failed {
                        message: "Codex resume stalled before provider progress for thread stale-thread"
                            .to_string(),
                    })
                    .await;
            } else {
                let _ = event_tx
                    .send(ProviderEvent::Completed(ProviderCompletion {
                        full_output: "fresh coder completed".to_string(),
                        provider_session_id: Some("fresh-thread".to_string()),
                    }))
                    .await;
            }
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: mpsc::channel(4).0,
        })
    }
}
```

两条测试共同断言：

```rust
assert_eq!(inputs.len(), 2);
assert_eq!(inputs[0].resume_provider_session_id.as_deref(), Some("stale-thread"));
assert_eq!(inputs[1].resume_provider_session_id, None);
assert!(inputs[0].prompt.contains("增量代码编写指令"));
assert!(inputs[1].prompt.contains("已确认 Work Item"));
assert_eq!(persisted.status, CodingAttemptStatus::Running);
assert!(open_gates.is_empty());
```

返修测试还必须断言 Fresh Retry 完整 Prompt 包含 Reviewer 的 summary 与 finding fix hint，证明新会话没有丢失本轮返修材料。

- [ ] **Step 2: 运行测试确认 RED**

Run:

```bash
cargo test --locked --lib initial_coder_resume_stall_retries_once_with_fresh_full_prompt
cargo test --locked --lib coder_rework_resume_stall_retries_once_with_fresh_full_prompt
```

Expected: 两条测试均失败，实际只启动一次 Provider，并将 Attempt 置为失败或返回 ProviderStream。

- [ ] **Step 3: 增加 Fresh Retry 输入契约**

在 `types.rs` 增加：

```rust
pub(crate) struct CodingProviderFreshRetry {
    pub(crate) legacy_input: AdapterInput,
    pub(crate) input: StreamingProviderInput,
}
```

并为 `CodingProviderStreamRun` 增加：

```rust
pub(crate) fresh_retry: Option<CodingProviderFreshRetry>,
```

所有非 Coder 调用点显式传 `fresh_retry: None`：

- `code_review.rs`
- `internal_pr_review.rs`
- `testing_provider/plan.rs` 的测试计划与修复两处
- `testing_provider/report.rs`

`testing_provider/execution.rs` 当前不构造 `CodingProviderStreamRun`，只做影响面复核，不修改。完成字段新增后运行 `rg -n 'CodingProviderStreamRun \{' src/product/coding_workspace_engine`，确认所有构造点都显式赋值。

- [ ] **Step 4: 初始 Coding 与 Reviewer 返修同时构造 continuation 与完整 Prompt**

在 `coding.rs` 始终生成 `full_prompt`；有 resume ID 时初始输入使用 `delta_prompt`，同时创建一次性 Fresh Retry：

```rust
let full_prompt = build_coding_prompt(
    &attempt,
    context,
    rework_instruction.as_ref(),
    coding_context_notes,
);
let initial_prompt = if resume_provider_session_id.is_some() {
    build_coding_delta_prompt(
        &attempt,
        context,
        rework_instruction.as_ref(),
        coding_context_notes,
    )
} else {
    full_prompt.clone()
};

let fresh_retry = resume_provider_session_id.as_ref().map(|_| {
    let legacy_input = AdapterInput {
        prompt: full_prompt.clone(),
        ..legacy_input.clone()
    };
    let mut input = streaming_input_from_adapter(&legacy_input, worktree_path.clone());
    input.workspace_session_id = Some(attempt.id.clone());
    input.resume_provider_session_id = None;
    input.permission_mode = permission_mode.clone();
CodingProviderFreshRetry { legacy_input, input }
});
```

在 `rework.rs` 先读取 `resume_provider_session_id`，再使用同一规则选择 Prompt：有 continuation 时首次使用 `build_coding_delta_prompt`，无 continuation 时直接使用 `build_coding_prompt`；若有 continuation，再额外构造一次无 resume ID、携带完整 Reviewer 返修材料的 `CodingProviderFreshRetry`。两条路径都把 `fresh_retry` 传入 `CodingProviderStreamRun`。

- [ ] **Step 5: Provider stream 在失败持久化前切换一次输入**

在 `provider_stream.rs` 使用外层 `'provider_attempt` 循环，持有 `active_legacy_input`、`active_input` 和 `fresh_retry.take()`。当事件是稳定 resume stall 时：

```rust
if provider_role == CodingProviderRole::Coder
    && provider_name == &ProviderName::Codex
    && is_resume_stall_failure(&message)
    && let Some(fresh) = fresh_retry.take()
{
    self.record_role_run_event(
        attempt,
        role_run,
        CodingRoleRunEventType::ProviderFailed,
        json!({
            "code": "codex_resume_stall_fresh_retry",
            "message": message,
            "resume_provider_session_id": active_input.resume_provider_session_id,
        }),
    );
    let _ = self.clear_attempt_provider_conversation(
        attempt,
        &CodingProviderRole::Coder,
        provider_name,
    )?;
    active_legacy_input = fresh.legacy_input;
    active_input = fresh.input;
    continue 'provider_attempt;
}
```

第二次失败因 `fresh_retry` 已取空，进入现有失败处理。

- [ ] **Step 6: 验证 GREEN 和非重试分支**

Run:

```bash
cargo test --locked --lib coder_resume_stall_retries_once_with_fresh_full_prompt
cargo test --locked --lib coder_rework_resume_stall_retries_once_with_fresh_full_prompt
cargo test --locked --lib coder_resume_stall_does_not_retry_without_resume_id
cargo test --locked --lib coder_resume_stall_does_not_retry_non_codex_provider
```

Expected: 四条命令退出码均为 0；两个自动重试测试的 Provider 调用次数均为 2，两个非重试测试均为 1。

- [ ] **Step 7: 独立提交**

```bash
git add src/product/coding_workspace_engine/types.rs src/product/coding_workspace_engine/coding.rs src/product/coding_workspace_engine/rework.rs src/product/coding_workspace_engine/provider_stream.rs src/product/coding_workspace_engine/code_review.rs src/product/coding_workspace_engine/internal_pr_review.rs src/product/coding_workspace_engine/testing_provider/plan.rs src/product/coding_workspace_engine/testing_provider/report.rs src/product/coding_workspace_engine/tests.rs src/product/coding_workspace_engine/tests/coder_resume_recovery.rs
git commit -m "fix: retry stalled coder continuation with fresh session"
```

---

### Task 3: 最终 Coder 中断进入可恢复 Gate

**Files:**
- Modify: `src/product/coding_models/gate.rs:87-105`
- Modify: `src/product/coding_workspace_engine/testing_parser.rs:326-375`
- Modify: `src/product/coding_workspace_engine/gates.rs:15-145, 625-715`
- Modify: `src/product/coding_workspace_engine/tests/provider_failure_recovery.rs`
- Modify: `src/product/coding_workspace_engine/tests/coder_resume_recovery.rs`

**Interfaces:**
- Produces: `CodingGateActionType::RetryCoding`
- Produces: Gate action ID `retry_coding`
- Produces: reason code `coder_provider_interrupted`

- [ ] **Step 1: 写重复失败保留 dirty worktree 的失败测试**

在 `coder_resume_recovery.rs` 增加 Provider，使 resume 和 Fresh Retry 都发送失败；测试先在 worktree 写入未提交文件，并断言：

```rust
assert_eq!(persisted.status, CodingAttemptStatus::Blocked);
assert_eq!(persisted.stage, CodingExecutionStage::Coding);
assert_eq!(gate.reason_code.as_deref(), Some("coder_provider_interrupted"));
assert_eq!(
    gate.available_actions
        .iter()
        .map(|action| action.action_id.as_str())
        .collect::<Vec<_>>(),
    vec!["retry_coding", "abort"]
);
assert!(open_gates.iter().all(|gate| {
    gate.reason_code.as_deref() != Some("shared_worktree_dirty_manual_gate")
}));
assert!(!git_status.is_empty());
```

- [ ] **Step 2: 运行测试确认 RED**

Run: `cargo test --locked --lib repeated_coder_failure_blocks_with_retry_gate_and_preserves_worktree`

Expected: 当前实现把 Attempt 置为 `failed`，并创建 `shared_worktree_dirty_manual_gate`。

- [ ] **Step 3: 增加 RetryCoding action 契约**

在 `gate.rs` 增加 `RetryCoding`，在 `testing_parser.rs` 增加：

```rust
"retry_coding" => Some(CodingGateAction {
    action_id: "retry_coding".to_string(),
    label: "重新启动 Coder".to_string(),
    action_type: CodingGateActionType::RetryCoding,
}),
```

- [ ] **Step 4: Coding 失败转为 Blocked Gate**

在 `fail_provider_stream` 的 CodeReview 分支之后增加 Coding 分支：

```rust
if attempt.stage == CodingExecutionStage::Coding {
    self.complete_timeline_node(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        node_id,
        CodingTimelineNodeStatus::Failed,
        Some(message.clone()),
    )
    .await?;
    if let Some(role_run) = self.store.latest_role_run(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        CodingExecutionStage::Coding,
        CodingProviderRole::Coder,
    )? {
        self.store.update_role_run_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
            CodingRoleRunStatus::Failed,
            Some("coder_provider_interrupted".to_string()),
        )?;
    }
    let _ = self.store.update_attempt_status(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        CodingAttemptStatus::Blocked,
    )?;
    let gate = self.store.create_blocked_gate(CreateBlockedGateInput {
        attempt_id: attempt.id.clone(),
        stage: CodingExecutionStage::Coding,
        node_id: Some(node_id.to_string()),
        role: Some(CodingProviderRole::Coder),
        title: "Coder 执行中断".to_string(),
        description: message.clone(),
        reason_code: Some("coder_provider_interrupted".to_string()),
        evidence_refs: Vec::new(),
        raw_provider_output_ref: None,
        available_actions: vec![
            coding_gate_action_for_id("retry_coding").expect("retry coding action"),
            coding_gate_action_for_id("abort").expect("abort action"),
        ],
    })?;
    let _ = self.event_tx.send(CodingWsOutMessage::CodingGateRequired { gate }).await;
    return Err(CodingWorkspaceEngineError::ProviderStream(message));
}
```

该分支不得调用 `handle_attempt_failed`。

- [ ] **Step 5: 处理 retry_coding GateResponse**

在 `handle_blocked_gate_response` 增加：

```rust
CodingGateActionType::RetryCoding => {
    let coder_provider = self
        .store
        .get_role_provider_config_snapshot(
            &current.project_id,
            &current.issue_id,
            &current.id,
        )?
        .coder;
    let cleared = self.clear_attempt_provider_conversation(
        &current,
        &CodingProviderRole::Coder,
        &coder_provider,
    )?;
    self.resume_blocked_attempt_at_stage(&cleared, CodingExecutionStage::Coding)?
}
```

测试需把旧 `provider_config_snapshot.author` 与最新 role-provider `coder` 设置成不同 Provider，并断言 Gate 被解析、Attempt 为 `running / coding`、最新 Coder Provider conversation 被删除、旧 Provider 与 Reviewer conversation 保留，防止清理错误会话。

- [ ] **Step 6: 验证 GREEN**

Run:

```bash
cargo test --locked --lib repeated_coder_failure_blocks_with_retry_gate_and_preserves_worktree
cargo test --locked --lib retry_coding_gate_clears_stale_coder_conversation
cargo test --locked --lib code_review_provider_failure_blocks_attempt_without_cleaning_shared_worktree
```

Expected: 三条命令退出码均为 0。

- [ ] **Step 7: 独立提交**

```bash
git add src/product/coding_models/gate.rs src/product/coding_workspace_engine/testing_parser.rs src/product/coding_workspace_engine/gates.rs src/product/coding_workspace_engine/tests/provider_failure_recovery.rs src/product/coding_workspace_engine/tests/coder_resume_recovery.rs
git commit -m "fix: keep interrupted coder attempts recoverable"
```

---

### Task 4: WebSocket Runner 与前端 Gate 契约

**Files:**
- Modify: `src/web/coding_ws_handler/runner.rs:180-240`
- Modify: `src/web/coding_ws_handler/tests.rs:441-505`
- Modify: `web/src/api/types/coding.ts:361-375`
- Modify: `web/src/pages/CodingWorkspacePage.gates.test.tsx`

**Interfaces:**
- Consumes: action ID `retry_coding`
- Produces: `should_emit_coding_runner_protocol_error(status: &CodingAttemptStatus) -> bool`

- [ ] **Step 1: 写 Runner 判定失败测试**

在 `src/web/coding_ws_handler/tests.rs` 增加：

```rust
#[test]
fn recoverable_attempt_status_suppresses_coding_start_failed() {
    assert!(!should_emit_coding_runner_protocol_error(
        &CodingAttemptStatus::Blocked
    ));
    assert!(!should_emit_coding_runner_protocol_error(
        &CodingAttemptStatus::WaitingForHuman
    ));
    assert!(should_emit_coding_runner_protocol_error(
        &CodingAttemptStatus::Failed
    ));
}
```

并扩展 `manual_continue_gate_response_does_not_auto_resume_runner`：

```rust
assert!(should_resume_runner_after_gate_response("retry_coding", &attempt));
```

- [ ] **Step 2: 运行 Rust 测试确认 RED**

Run: `cargo test --locked --lib recoverable_attempt_status_suppresses_coding_start_failed`

Expected: 编译失败，提示判定函数不存在。

- [ ] **Step 3: 实现 Runner 可恢复错误抑制**

在 `runner.rs` 增加：

```rust
pub(crate) fn should_emit_coding_runner_protocol_error(
    status: &CodingAttemptStatus,
) -> bool {
    !matches!(
        status,
        CodingAttemptStatus::Blocked | CodingAttemptStatus::WaitingForHuman
    )
}
```

Runner 执行失败后重新读取 Attempt；若函数返回 false，则发送最新 Session State，不发送 `CodingProtocolError`。同时将 `retry_coding` 加入 `should_resume_runner_after_gate_response` 的 action 列表。

- [ ] **Step 4: 写前端恢复 Gate 失败测试**

在 `CodingWorkspacePage.gates.test.tsx` 增加：

```tsx
it("restarts an interrupted coder without requiring extra context", async () => {
  const api = mockCodingWs();
  useCodingWorkspaceStore.setState({
    attemptId: "coding_attempt_0001",
    status: "blocked",
    stage: "coding",
    pendingGates: [
      {
        gate_id: "coding_blocked_gate_0001",
        kind: "blocked",
        title: "Coder 执行中断",
        description: "Codex resume stalled before provider progress",
        stage: "coding",
        role: "coder",
        reason_code: "coder_provider_interrupted",
        available_actions: [
          {
            action_id: "retry_coding",
            label: "重新启动 Coder",
            action_type: "retry_coding",
          },
        ],
      },
    ],
  });

  render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);
  await userEvent.click(screen.getByRole("button", { name: "重新启动 Coder" }));

  expect(api.respondGate).toHaveBeenCalledWith(
    "coding_blocked_gate_0001",
    "retry_coding",
    undefined,
  );
});
```

- [ ] **Step 5: 运行前端测试确认 RED**

Run: `cd web && pnpm test -- CodingWorkspacePage.gates.test.tsx`

Expected: TypeScript/Vitest 因 `retry_coding` 不在 action union 中失败。

- [ ] **Step 6: 同步前端 action union 并验证 GREEN**

在 `web/src/api/types/coding.ts` 的 `CodingGateActionType` 增加：

```typescript
| "retry_coding"
```

Run:

```bash
cargo test --locked --lib coding_ws_handler
cd web && pnpm test -- CodingWorkspacePage.gates.test.tsx
cd web && pnpm tsc -b
```

Expected: 三条命令退出码均为 0。

- [ ] **Step 7: 独立提交**

```bash
git add src/web/coding_ws_handler/runner.rs src/web/coding_ws_handler/tests.rs web/src/api/types/coding.ts web/src/pages/CodingWorkspacePage.gates.test.tsx
git commit -m "fix: resume coder from recoverable provider gate"
```

---

### Task 5: 全量验证与范围自检

**Files:**
- Verify only; no unrelated edits.

**Interfaces:**
- Consumes: Tasks 1-4 的全部行为。
- Produces: 可交付的验证证据。

- [ ] **Step 1: 运行相关 Rust 定向回归**

```bash
cargo test --locked --lib coder_resume
cargo test --locked --lib coder_provider_interrupted
cargo test --locked --lib coding_ws_handler
cargo test --locked --lib workspace_engine
```

Expected: 全部退出码为 0，无失败测试。

- [ ] **Step 2: 运行前端定向回归**

```bash
cd web
pnpm test -- CodingWorkspacePage.gates.test.tsx
pnpm tsc -b
pnpm build
```

Expected: 全部退出码为 0；Vite 仅允许既有 chunk-size warning。

- [ ] **Step 3: 运行 Rust 标准全量门禁**

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
```

Expected: 全部退出码为 0；禁止使用 `-j 1`。

- [ ] **Step 4: 检查改动范围**

```bash
git diff --check
git status --short
git diff --stat
```

Expected: 无 whitespace error；新增改动仅覆盖本计划文件，`feat-b-0709` 原有未提交文件保持原样。

- [ ] **Step 5: 提交必要的验证修正**

仅当格式化产生本任务文件变化时：

```bash
git add src/cross_cutting/codex_provider/mod.rs src/product/coding_models/gate.rs src/product/coding_workspace_engine src/web/coding_ws_handler web/src/api/types/coding.ts web/src/pages/CodingWorkspacePage.gates.test.tsx
git commit -m "test: verify coding provider recovery flow"
```

---

### Task 6: 回退 Attempt 到 Draft 4 尚未启动并重启服务

**Files:**
- Runtime state: `.aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001*`
- Target worktree: `.worktrees/aria-issues/issue_0001`
- Backup only: `/tmp/coding_attempt_0001-before-final-reset.tgz`
- Backup only: `/tmp/aria-issue_0001-before-final-reset.patch`

**Interfaces:**
- Consumes: 已验证的修复二进制和 Draft 3 commit `58ec0db92dd5a9c874feab9447194bd657f22e35`。
- Produces: `running / prepare_context` 的 Draft 4 预启动状态。

- [ ] **Step 1: 停止前后端开发服务，避免并发写状态**

使用当前后台任务会话发送 `Ctrl-C`，确认 `4317` 和 `5173` 不再响应。

- [ ] **Step 2: 备份最新 Attempt 和 Draft 4 diff**

```bash
tar -czf /tmp/coding_attempt_0001-before-final-reset.tgz \
  .aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001 \
  .aria/projects/project_0001/issues/issue_0001/issue-shared-worktree.json

git -C .worktrees/aria-issues/issue_0001 diff --binary \
  --output=/tmp/aria-issue_0001-before-final-reset.patch
```

Expected: 两个备份文件存在且非空。

- [ ] **Step 3: 清理 Draft 4 worktree 修改**

```bash
git -C .worktrees/aria-issues/issue_0001 restore --source=HEAD --staged --worktree .
git -C .worktrees/aria-issues/issue_0001 status --short --branch
```

Expected: 仅输出 `## aria/issues/issue_0001`，HEAD 为 `58ec0db`。

- [ ] **Step 4: 精确恢复 Attempt 快照**

恢复以下字段：

```json
{
  "status": "running",
  "stage": "prepare_context",
  "rework_count": 5,
  "current_work_item_id": "work_item_compile_20260712024139064_004",
  "active_unit_id": "coding_unit_0004",
  "head_commit": "58ec0db92dd5a9c874feab9447194bd657f22e35",
  "provider_conversations": []
}
```

保留 Unit 1-3 completed、Unit 4 running、Unit 5-10 pending。删除 Draft 4 对应的 timeline nodes 20 及以后、role runs 19 及以后、stage gates 17 及以后、code reviews 9 及以后、chat entries、blocked gates、context notes、choice gates、rework instructions、raw outputs 和 role-run event artifacts。

- [ ] **Step 5: 验证状态一致性**

```bash
jq -e '.status == "running" and .stage == "prepare_context" and .provider_conversations == []' \
  .aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001.json

jq -e 'length == 19 and .[-1].id == "coding_node_0019" and .[-1].status == "completed"' \
  .aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001/timeline-nodes.json
```

Expected: 两条命令输出 `true`，退出码为 0。

- [ ] **Step 6: 重启后端和前端**

后端：

```bash
cargo watch -w src -w Cargo.toml -w Cargo.lock \
  -x "run --locked -- web --workspace . --host 127.0.0.1 --port 4317"
```

确认后端健康后启动前端：

```bash
cd web
pnpm dev --port 5173
```

- [ ] **Step 7: 最终只读验收**

```bash
curl --noproxy '*' -sS http://127.0.0.1:4317/api/health
curl --noproxy '*' -sS -I http://127.0.0.1:5173/
curl --noproxy '*' -sS http://127.0.0.1:5173/api/health
```

Expected: 后端和代理返回 `{"status":"ok"}`，前端返回 HTTP 200。只读 Attempt 快照必须显示 Draft 4、`prepare_context`、无 pending gate、最后 timeline node 为 Draft 3 review passed。

---

## 计划自审结果

- 设计目标均映射到 Tasks 1-6，无未覆盖需求。
- 自动重试限定为一次，重复失败进入人工 Gate，无无限重试。
- 初始 Coding 与 Reviewer 驱动返修两条 Coder 路径都构造了 Fresh Retry；现场故障路径 `rework.rs` 有独立回归测试。
- `CodingProviderStreamRun` 的全部实际构造点都显式设置 `fresh_retry`；`testing_provider/execution.rs` 经复核不构造该类型。
- `retry_coding` 使用最新 role-provider 配置中的 `coder` 清理会话，不读取旧 `provider_config_snapshot.author`。
- 类型名 `RetryCoding`、action ID `retry_coding`、reason code `coder_provider_interrupted` 在后端、WebSocket 与前端保持一致。
- 所有生产行为均有先失败再实现的测试步骤。
- 计划不修改或提交 `src/product/workspace_engine/mod.rs` 及 `feat-b-0709` 其他现有无关未提交文件。
