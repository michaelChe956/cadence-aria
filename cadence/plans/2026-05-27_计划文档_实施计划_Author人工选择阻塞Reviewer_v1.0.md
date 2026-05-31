# Author 人工选择阻塞 Reviewer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 author 提出人工选择问题后错误进入 reviewer 的流程问题。

**Architecture:** 复用现有 `choice_request/choice_response` 协议和前端 `ChoiceRequestEntry`。后端新增文本型人工问题兜底识别，并让无 active provider run 的 `choice_response` 可以重新启动 author，而不是返回 missing active run。

**Tech Stack:** Rust、Axum WebSocket、Tokio、现有 Workspace Engine、React/Vite 前端。

---

## File Structure

- Modify: `src/product/workspace_engine.rs`
  - 新增 `PendingAuthorChoice` 状态、文本问题识别、choice response 续跑输入构造。
  - 修改 `complete_assistant_message()`，在人工问题场景阻塞 reviewer。
- Modify: `src/web/workspace_ws_handler.rs`
  - `choice_response` 无 active run 时转给 engine pending choice 分支，并重新启动 author。
- Modify: `src/web/workspace_context.rs`
  - 强化 prompt：优先使用结构化 ask-user 机制，不把问题文本当最终候选产物。
- Modify: `tests/workspace_ws_integration.rs`
  - 新增 scripted provider 集成测试。

## Task 1: 后端测试覆盖文本型 author 人工问题

**Files:**
- Modify: `tests/workspace_ws_integration.rs`

- [ ] **Step 1: Write the failing test**

在 `tests/workspace_ws_integration.rs` 增加一个 provider：

```rust
struct ChoiceThenArtifactProvider {
    prompts: Arc<Mutex<Vec<String>>>,
    calls: Mutex<u32>,
}
```

`start()` 第一次输出人工选择问题，第二次输出有效 Story Spec。

新增测试名：

```rust
#[tokio::test]
async fn workspace_ws_author_text_choice_blocks_reviewer_until_user_answers()
```

测试流程：

```rust
send_json(
    &mut ws,
    &WsInMessage::UserMessage {
        content: "开始生成".to_string(),
    },
)
.await;

let choice = recv_until_choice_request(&mut ws).await;
assert!(choice.prompt.contains("n <= 0"));
assert_eq!(choice.options_len, 3);

send_json(
    &mut ws,
    &WsInMessage::ChoiceResponse {
        id: choice.id,
        selected_option_ids: vec!["A".to_string()],
        free_text: None,
    },
)
.await;

let checkpoint = recv_until_message_complete(&mut ws).await;
assert!(checkpoint.starts_with("cp_"));
let stage = recv_until_stage(&mut ws, "human_confirm").await;
assert_eq!(stage, "human_confirm");
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
cargo test --locked --test workspace_ws_integration workspace_ws_author_text_choice_blocks_reviewer_until_user_answers
```

Expected: FAIL。失败原因应是没有收到 `choice_request`，而是收到 `message_complete` 或 `cross_review`。

## Task 2: Engine 阻塞文本型 author 人工问题

**Files:**
- Modify: `src/product/workspace_engine.rs`

- [ ] **Step 1: Add pending state**

在 `WorkspaceEngine` 增加字段：

```rust
pending_author_choice: Option<PendingAuthorChoice>,
```

新增类型：

```rust
#[derive(Debug, Clone)]
struct PendingAuthorChoice {
    id: String,
    prompt: String,
    options: Vec<ChoiceOptionData>,
    source_node_id: Option<String>,
}
```

- [ ] **Step 2: Add parser**

新增函数：

```rust
fn detect_author_choice_request(content: &str, workspace_type: &WorkspaceType) -> Option<PendingAuthorChoice>
```

规则：

- Story 有效产物包含 `## 功能需求` 和 `## 成功标准` 时返回 `None`。
- 否则解析 `A.` / `A、` / `A)` / `1.` 形式选项。
- 选项少于 2 个返回 `None`。

- [ ] **Step 3: Use parser in `complete_assistant_message()`**

在写 message/artifact 前增加拦截：命中人工问题时标记当前 author node 为 `paused`，发送 `EngineEvent::ChoiceRequest`，然后 `return`。

- [ ] **Step 4: Add response handler**

新增方法：

```rust
pub fn take_pending_author_choice_prompt(
    &mut self,
    id: &str,
    selected_option_ids: Vec<String>,
    free_text: Option<String>,
) -> Result<String, String>
```

行为：id 匹配时构造下一轮 author prompt，id 不匹配时返回错误。

## Task 3: WebSocket 无 active run 的 choice response 续跑 author

**Files:**
- Modify: `src/web/workspace_ws_handler.rs`

- [ ] **Step 1: Modify `WsInMessage::ChoiceResponse` branch**

保留现有 active run 转发逻辑。无 active run 时调用 `take_pending_author_choice_prompt()`，并用返回内容重新 `spawn_provider_run_from_handler(... ProviderRunKind::Author { content })`。

- [ ] **Step 2: Run failing test again**

Run:

```bash
cargo test --locked --test workspace_ws_integration workspace_ws_author_text_choice_blocks_reviewer_until_user_answers
```

Expected: PASS。

## Task 4: Prompt 强化

**Files:**
- Modify: `src/web/workspace_context.rs`
- Modify: `tests/workspace_ws_integration.rs`

- [ ] **Step 1: Update prompt text**

在 `workflow_discipline_for()` 的 Story/Design 分支追加：

```text
如果需要向用户提问，必须使用结构化 AskUserQuestion / requestUserInput 交互能力；不要把 A/B/C 选择题作为最终候选产物正文输出。若当前 provider 环境无法发起结构化交互，才允许输出清晰的人工选择题，daemon 会暂停 reviewer 并转换为用户选择卡片。
```

- [ ] **Step 2: Update context hydration test**

在 `workspace_ws_hydrates_context_for_existing_empty_session` 中增加断言：

```rust
assert!(messages[0].content.contains("结构化 AskUserQuestion"));
assert!(messages[0].content.contains("不要把 A/B/C 选择题作为最终候选产物正文输出"));
```

## Task 5: Verification

- [ ] **Step 1: Run focused integration tests**

Run:

```bash
cargo test --locked --test workspace_ws_integration workspace_ws_author_text_choice_blocks_reviewer_until_user_answers
cargo test --locked --test workspace_ws_integration workspace_ws_hydrates_context_for_existing_empty_session
```

Expected: both PASS。

- [ ] **Step 2: Run approval bridge regression**

Run:

```bash
cargo test-approval-bridge
```

Expected: 9 passed。

- [ ] **Step 3: Run cargo check**

Run:

```bash
cargo check --locked
```

Expected: PASS。
