# Workspace 断线手动恢复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 Workspace 增加显式的断线手动恢复入口，安全恢复 Work Item Draft 生成或 Review，而不重新生成已确认 Outline 或覆盖已保存产物。

**Architecture:** 后端 WorkspaceEngine 是恢复状态的唯一判断源，根据 artifact version、Work Item active index、Draft record 和 Timeline 计算 `RecoverableInterruptedRun`。前端只展示后端返回的恢复描述，点击后发送专用 WebSocket 动作；后端再次校验并创建带 `TimelineNodeRetry` 的新节点，再启动对应 provider run。

**Tech Stack:** Rust 2024、Tokio、Serde、React、TypeScript、Zustand、Vitest、Testing Library。

## Global Constraints

- 使用宿主机 Rust 工具链和仓库 `rust-toolchain.toml`。
- Cargo 命令必须带 `--locked`，禁止 `-j 1`。
- 前端包管理器只使用 `pnpm`。
- 所有生产代码遵循 TDD：先写失败测试并确认失败，再写最小实现。
- 不覆盖当前工作区已有结构化审核恢复改动。
- 不自动重试，不复用 partial output，不删除失败 Timeline 节点。
- Story、Design、Work Item 的普通 Reviewer Run 使用共享恢复逻辑；Work Item Plan 额外覆盖 Draft Run 和 Draft Review。

---

### Task 1: 后端恢复状态识别与 Engine 动作

**Files:**
- Create: `src/product/workspace_engine/interrupted_run_recovery.rs`
- Modify: `src/product/workspace_engine/mod.rs`
- Modify: `src/product/workspace_engine/session_state.rs`
- Modify: `src/web/workspace_ws_types/out.rs`
- Modify: `src/product/workspace_engine/tests.rs`
- Create: `src/product/workspace_engine/tests/part_19.rs`

**Interfaces:**
- Produces: `RecoverableInterruptedRun { failed_node_id, operation, label }`
- Produces: `WorkspaceEngine::recoverable_interrupted_run(&self) -> Option<RecoverableInterruptedRun>`
- Produces: `WorkspaceEngine::retry_interrupted_run(&mut self, failed_node_id: &str) -> Result<InterruptedRunRecoveryOutcome, InterruptedRunRecoveryError>`
- Produces: `InterruptedRunRecoveryOutcome::{Review, WorkItemDraftGeneration}`

- [ ] **Step 1: 写 Work Item Draft Review 恢复失败测试**

在 `part_19.rs` 构造 confirmed Outline、accepted Draft、失败 `work_item_draft_review`、`aborted_by_disconnect`，并追加一个未提交 artifact 的失败 Outline Run。断言：

```rust
let recovery = engine.recoverable_interrupted_run().expect("recoverable review");
assert_eq!(recovery.failed_node_id, "timeline_node_054");
assert_eq!(recovery.operation, RecoverableInterruptedOperation::Review);
```

- [ ] **Step 2: 运行测试并确认因接口不存在而失败**

Run: `cargo test --locked --lib interrupted_run_recovery`

Expected: FAIL，缺少恢复类型或方法。

- [ ] **Step 3: 写 Draft 生成与普通 Review 失败测试**

新增三个具名测试：

- `interrupted_work_item_draft_run_retries_same_active_outline`：active index 指向 `outline_frontend_provider_controls_repository`，失败节点 summary 也绑定该 ID；调用恢复后断言新节点仍绑定同一 outline，outcome 为 `WorkItemDraftGeneration`。
- `interrupted_shared_reviewer_run_is_recoverable_for_story_design_and_work_item`：使用 `[Story, Design, WorkItem]` 表驱动构造当前 Markdown artifact、失败 `reviewer_run` 和断线节点；逐项断言 operation 为 `Review`。
- `successful_new_artifact_supersedes_old_interrupted_review`：在失败 Review 后追加一个 source node 不同的新 current artifact version；断言 `recoverable_interrupted_run()` 返回 `None`。

- [ ] **Step 4: 实现恢复描述类型和状态识别**

在 `out.rs` 增加：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoverableInterruptedOperation {
    Review,
    WorkItemDraftGeneration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoverableInterruptedRun {
    pub failed_node_id: String,
    pub operation: RecoverableInterruptedOperation,
    pub label: String,
}
```

`recoverable_interrupted_run` 必须以当前 artifact 和 active index 为准：accepted `WorkItemDraftCandidate` 优先寻找相同 outline 的失败 Draft Review；active outline 没有可确认 Draft 时寻找相同 outline 的失败 Draft Run；普通 Workspace 只接受当前 artifact 对应的失败 `ReviewerRun`。

- [ ] **Step 5: 实现重试节点创建**

`retry_interrupted_run` 再次计算恢复描述并校验 `failed_node_id`，然后调用 `create_timeline_node_with_retry`：

```rust
TimelineNodeRetry {
    retry_of_node_id: failed_node_id.to_string(),
    retry_attempt,
    retry_reason: "aborted_by_disconnect".to_string(),
    retry_error: TimelineNodeRetryError {
        code: "provider_run_aborted_by_disconnect".to_string(),
        message: "连接断开，运行已中止".to_string(),
    },
}
```

Review 创建与原节点同类型的新 `cross_review` 节点；Draft 创建同一 active outline 的新 `work_item_draft_run` 节点。

- [ ] **Step 6: 将恢复描述加入 SessionState**

`WsOutMessage::SessionState` 增加：

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
recoverable_interrupted_run: Option<RecoverableInterruptedRun>,
```

`build_session_state` 使用 `self.recoverable_interrupted_run()` 填充。

- [ ] **Step 7: 运行 Engine 定向测试**

Run: `cargo test --locked --lib interrupted_run_recovery`

Expected: PASS。

---

### Task 2: WebSocket 恢复协议与 Provider 调度

**Files:**
- Modify: `src/web/workspace_ws_types/in_.rs`
- Modify: `src/web/workspace_ws_handler/protocol.rs`
- Modify: `src/web/workspace_ws_handler/decisions/inbound.rs`
- Modify: `src/web/workspace_ws_handler/tests.rs`
- Modify: `src/web/workspace_ws_types/tests.rs`

**Interfaces:**
- Consumes: `WorkspaceEngine::retry_interrupted_run`
- Produces: `WsInMessage::RetryInterruptedRun { failed_node_id }`
- Produces: Review → `ProviderRunKind::ReviewOnly`
- Produces: Draft → `ProviderRunKind::WorkItemPlanDraft { feedback: None }`

- [ ] **Step 1: 写协议解析和阶段校验失败测试**

```rust
let message: WsInMessage = serde_json::from_value(json!({
    "type": "retry_interrupted_run",
    "failed_node_id": "timeline_node_054"
})).unwrap();
assert!(is_message_valid_for_stage(&message, &WorkspaceStage::PrepareContext));
assert!(!is_message_valid_for_stage(&message, &WorkspaceStage::Running));
```

- [ ] **Step 2: 运行协议测试确认失败**

Run: `cargo test --locked --lib workspace_ws_handler`

Expected: FAIL，消息类型或处理分支不存在。

- [ ] **Step 3: 实现入站消息与 handler 分支**

```rust
RetryInterruptedRun { failed_node_id: String },
```

handler 调用 Engine，映射 outcome 后调用 `spawn_provider_run_from_handler`。不可恢复时发送：

```rust
WsOutMessage::ProtocolError {
    code: error.code().to_string(),
    message: error.to_string(),
    context: Some(json!({ "failed_node_id": failed_node_id })),
}
```

- [ ] **Step 4: 写 handler 调度测试**

分别使用 PromptRecordingProvider 验证 Review 和 Draft 恢复会启动 provider，并验证 stale node ID 不启动 provider。

- [ ] **Step 5: 运行 WebSocket 定向测试**

Run: `cargo test --locked --lib workspace_ws_handler`

Expected: PASS。

---

### Task 3: 前端协议、Store 与发送动作

**Files:**
- Modify: `web/src/api/types/workspace.ts`
- Modify: `web/src/state/workspace-ws-store-types.ts`
- Modify: `web/src/state/workspace-ws-store.ts`
- Modify: `web/src/state/workspace-ws-store.rebuild.test.ts`
- Modify: `web/src/hooks/useWorkspaceWs.ts`
- Modify: `web/src/hooks/useWorkspaceWs.actions.test.tsx`

**Interfaces:**
- Produces: TypeScript `RecoverableInterruptedRun`
- Produces: Store 字段 `recoverableInterruptedRun`
- Produces: Hook 方法 `retryInterruptedRun(failedNodeId: string): boolean`

- [ ] **Step 1: 写 Store hydration 失败测试**

```typescript
expect(useWorkspaceStore.getState().recoverableInterruptedRun).toEqual({
  failed_node_id: "timeline_node_054",
  operation: "review",
  label: "重试中断审核",
});
```

- [ ] **Step 2: 写 Hook 发送失败测试**

```typescript
harness.api.retryInterruptedRun("timeline_node_054");
expect(sent()).toContainEqual({
  type: "retry_interrupted_run",
  failed_node_id: "timeline_node_054",
});
```

- [ ] **Step 3: 运行前端定向测试确认失败**

Run: `cd web && pnpm exec vitest run src/state/workspace-ws-store.rebuild.test.ts src/hooks/useWorkspaceWs.actions.test.tsx`

Expected: FAIL，字段或方法不存在。

- [ ] **Step 4: 实现类型、Store hydration 和发送动作**

发送成功时清除 error、清空 execution events、设置 provider status 为 running；新的 session state 或 ProtocolError 到达后由既有消息处理更新 UI。

- [ ] **Step 5: 运行前端协议测试**

Run: `cd web && pnpm exec vitest run src/state/workspace-ws-store.rebuild.test.ts src/hooks/useWorkspaceWs.actions.test.tsx`

Expected: PASS。

---

### Task 4: 手动恢复按钮与错误操作防护

**Files:**
- Modify: `web/src/components/workspace/DisconnectBanner.tsx`
- Modify: `web/src/components/workspace/DisconnectBanner.test.tsx`
- Modify: `web/src/components/chat-workspace/ChatInputBar.tsx`
- Modify: `web/src/components/chat-workspace/ChatInputBar.test.tsx`
- Modify: `web/src/pages/ChatWorkspacePage.tsx`
- Modify: `web/src/pages/ChatWorkspacePage.actions.test.tsx`

**Interfaces:**
- Consumes: `recoverableInterruptedRun`
- Consumes: `retryInterruptedRun`
- Produces: 动态按钮文案和单击锁定
- Produces: 存在恢复动作时隐藏“开始生成”

- [ ] **Step 1: 写 DisconnectBanner 按钮失败测试**

覆盖 review 和 `work_item_draft_generation` 两种 label，断言双击只触发一次回调。

- [ ] **Step 2: 写 ChatInputBar 防误触失败测试**

增加 `hideStartGeneration?: boolean`，在 `prepare_context` 且值为 true 时断言找不到“开始生成”。

- [ ] **Step 3: 运行组件测试确认失败**

Run: `cd web && pnpm exec vitest run src/components/workspace/DisconnectBanner.test.tsx src/components/chat-workspace/ChatInputBar.test.tsx src/pages/ChatWorkspacePage.actions.test.tsx`

Expected: FAIL，新 props 和按钮不存在。

- [ ] **Step 4: 实现按钮与页面接线**

`DisconnectBanner` 新增：

```typescript
recoverableInterruptedRun?: RecoverableInterruptedRun | null;
onRetryInterruptedRun?: (failedNodeId: string) => boolean | void;
```

组件本地维护 `retrying`；点击后立即锁定。`ChatWorkspacePage` 将 Store 描述传给 Banner，并把 `Boolean(recoverableInterruptedRun)` 传给 `ChatInputBar.hideStartGeneration`。

- [ ] **Step 5: 运行组件定向测试**

Run: `cd web && pnpm exec vitest run src/components/workspace/DisconnectBanner.test.tsx src/components/chat-workspace/ChatInputBar.test.tsx src/pages/ChatWorkspacePage.actions.test.tsx`

Expected: PASS。

---

### Task 5: 回归验证与真实会话状态验收

**Files:**
- Verify only: `.aria/projects/project_0001/issues/issue_0001/workspace-sessions/workspace_session_0003.json`
- Verify only: `.aria/projects/project_0001/issues/issue_0001/workspace-timelines/workspace_session_0003/`
- Verify only: `.aria/projects/project_0001/issues/issue_0001/work_item_plan_drafts/issue_work_item_plan_0001/`

- [ ] **Step 1: 运行 Rust 格式与定向测试**

Run:

```bash
cargo fmt --check
cargo test --locked --lib interrupted_run_recovery
cargo test --locked --lib workspace_ws_handler
```

- [ ] **Step 2: 运行前端定向测试与类型检查**

Run:

```bash
cd web
pnpm exec vitest run src/components/workspace/DisconnectBanner.test.tsx src/components/chat-workspace/ChatInputBar.test.tsx src/pages/ChatWorkspacePage.actions.test.tsx src/state/workspace-ws-store.rebuild.test.ts src/hooks/useWorkspaceWs.actions.test.tsx
pnpm tsc -b
```

- [ ] **Step 3: 运行项目标准完整验证**

Run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
cd web && pnpm test
```

- [ ] **Step 4: 验证真实会话恢复描述**

使用只读测试或现有服务加载 `workspace_session_0003`，断言恢复描述指向 `timeline_node_054`，operation 为 `review`，且 active Outline、`draft_012` 与 artifact version 未变化。

- [ ] **Step 5: 检查工作区差异**

Run: `git diff --check && git status --short`

确认本功能修改未覆盖当前工作区既有结构化审核恢复改动。
