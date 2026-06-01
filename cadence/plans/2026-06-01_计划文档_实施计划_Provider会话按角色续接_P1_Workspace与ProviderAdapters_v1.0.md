# Provider 会话按角色续接 P1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Story/Design Workspace 的 author/reviewer provider 调用按角色续接 Claude Code 和 Codex 原生会话，修复用户选择后重新调用 provider 时新开对话的问题。

**Architecture:** P1 引入通用 `ProviderConversationRef` 模型，拆分 `StreamingProviderInput` 中产品 session 与 provider session 的语义，并在 `WorkspaceEngine` 中按 `role + provider` 保存和读取 provider 会话。Claude Code 用 `--resume <id>` 续接，Codex app-server 用已有 `threadId` 直接启动下一轮 turn。

**Tech Stack:** Rust 1.95、Tokio、serde、Axum WebSocket、Claude Code CLI、Codex app-server JSON-RPC、Cargo 测试套件。

---

## 计划拆分说明

本需求覆盖 Workspace、provider adapter、Coding Workspace 三个执行面。为避免单份计划超过上下文预算，本轮拆成两份计划：

- P1：通用模型、Story/Design Workspace、Claude Code provider、Codex provider。
- P2：Coding Workspace 接入同一 provider conversation 机制。

执行 P2 前必须先完成 P1，因为 P2 依赖 P1 新增的模型和 `StreamingProviderInput` 字段。

## File Structure

- Modify: `src/product/models.rs`
  - 新增 `ProviderConversationRole`、`ProviderConversationRef`。
  - 在 `WorkspaceSessionRecord` 增加 `provider_conversations`，旧 JSON 默认空表。
- Modify: `src/product/lifecycle_store.rs`
  - 新建 workspace session 时初始化空 provider conversations。
  - 增加更新 workspace provider conversations 的持久化方法。
- Modify: `src/product/workspace_engine.rs`
  - 在 `WorkspaceSession` 内存模型增加 provider conversations。
  - author/reviewer provider input 构造时读取对应 role 的 resume id。
  - provider 完成时保存 `provider_session_id`。
  - 文本选择题续跑 author 时自动续接 author provider session。
- Modify: `src/cross_cutting/streaming_provider.rs`
  - 将 `StreamingProviderInput.session_id` 拆成 `workspace_session_id` 和 `resume_provider_session_id`。
  - 更新 fake provider 和本文件内测试构造。
- Modify: `src/cross_cutting/claude_code_provider.rs`
  - `build_args` 支持 resume provider session。
  - `run_streaming` 构造新字段。
- Modify: `src/cross_cutting/codex_provider.rs`
  - `run_codex_session` 有 resume id 时跳过 `thread/start`，直接 `turn/start`。
  - `run_streaming` 构造新字段。
- Create: `tests/fixtures/provider/codex_app_server_resume_fixture.sh`
  - 验证 Codex resume 路径不会发送 `thread/start`。
- Modify: `src/web/test_controls.rs`
  - 更新 `StreamingProviderInput` 字段名。
- Modify: `src/product/compatibility_scan.rs`
  - 更新 `StreamingProviderInput` 字段名。
- Modify: `src/product/coding_workspace_engine.rs`
  - 仅为字段拆分做编译兼容，P2 再接入 Coding provider conversation。
- Modify: `tests/it_product/product_lifecycle_store.rs`
  - 增加 workspace provider conversations 兼容与持久化测试。
- Modify: `tests/it_core/workspace_ws_integration.rs`
  - 增强 author 文本选择题集成测试，断言第二轮 author 续接第一轮 provider session。
- Modify: provider adapter 单元测试所在文件
  - `src/cross_cutting/claude_code_provider.rs`
  - `src/cross_cutting/codex_provider.rs`

## Task 1: Provider Conversation 模型与 Workspace 持久化

**Files:**
- Modify: `src/product/models.rs`
- Modify: `src/product/lifecycle_store.rs`
- Modify: `tests/it_product/product_lifecycle_store.rs`

- [ ] **Step 1: 写 failing 测试，覆盖旧 WorkspaceSession JSON 缺省 provider_conversations**

在 `tests/it_product/product_lifecycle_store.rs` 追加测试：

```rust
#[test]
fn workspace_session_provider_conversations_default_for_legacy_json() {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = LifecycleStore::new(paths.clone());
    let session = store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "story_0001".to_string(),
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: ProviderName::Codex,
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
        })
        .expect("create workspace session");

    let session_path = paths
        .root()
        .join("projects/project_0001/issues/issue_0001/workspace-sessions")
        .join(format!("{}.json", session.id));
    let mut value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&session_path).unwrap()).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .remove("provider_conversations");
    std::fs::write(&session_path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

    let reloaded = store
        .get_workspace_session(&session.id)
        .expect("reload legacy session");
    assert!(reloaded.provider_conversations.is_empty());
}
```

- [ ] **Step 2: 写 failing 测试，覆盖 provider conversation 更新**

在同一测试文件追加：

```rust
#[test]
fn updates_workspace_session_provider_conversations() {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = LifecycleStore::new(paths);
    let session = store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "story_0001".to_string(),
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: ProviderName::Codex,
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
        })
        .expect("create workspace session");

    let conversations = vec![ProviderConversationRef {
        role: ProviderConversationRole::Author,
        provider: ProviderName::ClaudeCode,
        provider_session_id: "claude-author-session".to_string(),
        updated_at: "2026-06-01T00:00:00Z".to_string(),
        last_node_id: Some("node-author-1".to_string()),
    }];

    let updated = store
        .replace_workspace_provider_conversations(&session.id, conversations.clone())
        .expect("persist provider conversations");

    assert_eq!(updated.provider_conversations, conversations);
    let reloaded = store
        .get_workspace_session(&session.id)
        .expect("reload session");
    assert_eq!(reloaded.provider_conversations, conversations);
}
```

- [ ] **Step 3: 运行测试确认失败**

Run:

```bash
cargo test --locked --test product_lifecycle_store workspace_session_provider_conversations_default_for_legacy_json
cargo test --locked --test product_lifecycle_store updates_workspace_session_provider_conversations
```

Expected:

- 第一个测试因 `WorkspaceSessionRecord.provider_conversations` 字段不存在或反序列化失败而失败。
- 第二个测试因 `ProviderConversationRef` / `replace_workspace_provider_conversations` 不存在而编译失败。

- [ ] **Step 4: 新增模型**

在 `src/product/models.rs` 中 `ProviderName` 定义附近或 `WorkspaceSessionRecord` 前加入：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderConversationRole {
    Author,
    Reviewer,
    Coder,
    Tester,
    Analyst,
    CodeReviewer,
    InternalReviewer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProviderConversationRef {
    pub role: ProviderConversationRole,
    pub provider: ProviderName,
    pub provider_session_id: String,
    pub updated_at: String,
    pub last_node_id: Option<String>,
}
```

在 `WorkspaceSessionRecord` 加字段：

```rust
    #[serde(default)]
    pub provider_conversations: Vec<ProviderConversationRef>,
```

- [ ] **Step 5: 初始化新字段并实现持久化方法**

在 `src/product/lifecycle_store.rs` 创建 `WorkspaceSessionRecord` 的位置加入：

```rust
provider_conversations: Vec::new(),
```

在 `impl LifecycleStore` 中追加方法：

```rust
pub fn replace_workspace_provider_conversations(
    &self,
    session_id: &str,
    provider_conversations: Vec<ProviderConversationRef>,
) -> Result<WorkspaceSessionRecord, ProductStoreError> {
    validate_relative_id(session_id)?;
    let session_path = self.find_workspace_session_path(session_id)?;
    let mut session: WorkspaceSessionRecord = read_json(&session_path)?;
    session.provider_conversations = provider_conversations;
    session.updated_at = Utc::now().to_rfc3339();
    write_json(&session_path, &session)?;
    Ok(session)
}
```

确保 `lifecycle_store.rs` 的 imports 包含：

```rust
ProviderConversationRef,
```

- [ ] **Step 6: 运行测试确认通过**

Run:

```bash
cargo test --locked --test product_lifecycle_store workspace_session_provider_conversations_default_for_legacy_json
cargo test --locked --test product_lifecycle_store updates_workspace_session_provider_conversations
```

Expected: 两个测试 PASS。

- [ ] **Step 7: Commit**

```bash
git add src/product/models.rs src/product/lifecycle_store.rs tests/it_product/product_lifecycle_store.rs
git commit -m "feat: persist workspace provider conversations"
```

## Task 2: 拆分 StreamingProviderInput 的 session 语义

**Files:**
- Modify: `src/cross_cutting/streaming_provider.rs`
- Modify: `src/cross_cutting/claude_code_provider.rs`
- Modify: `src/cross_cutting/codex_provider.rs`
- Modify: `src/product/workspace_engine.rs`
- Modify: `src/product/coding_workspace_engine.rs`
- Modify: `src/product/compatibility_scan.rs`
- Modify: `src/web/test_controls.rs`
- Modify: tests that construct `StreamingProviderInput`

- [ ] **Step 1: 写字段拆分编译检查**

Run:

```bash
rg -n "session_id:" src tests
```

记录所有 `StreamingProviderInput { session_id: ... }` 构造点。后续每个构造点必须改成：

```rust
workspace_session_id: <old product/workspace session id or None>,
resume_provider_session_id: None,
```

其中 `WorkspaceEngine` 的 Story/Design provider input 后续 Task 3 会把 `resume_provider_session_id` 改成真实查询结果；本 Task 先保持 `None` 让代码编译。

- [ ] **Step 2: 修改 `StreamingProviderInput` 结构体**

在 `src/cross_cutting/streaming_provider.rs` 中把字段：

```rust
pub session_id: Option<String>,
```

替换为：

```rust
pub workspace_session_id: Option<String>,
pub resume_provider_session_id: Option<String>,
```

- [ ] **Step 3: 更新 adapter `run_streaming` 构造**

在 `src/cross_cutting/claude_code_provider.rs` 和 `src/cross_cutting/codex_provider.rs` 的 `run_streaming` 中，把：

```rust
session_id: None,
```

改成：

```rust
workspace_session_id: None,
resume_provider_session_id: None,
```

- [ ] **Step 4: 更新 WorkspaceEngine 构造点**

在 `src/product/workspace_engine.rs` 的 `build_streaming_input`、`build_review_input`、`build_revision_input` 中，把：

```rust
session_id: Some(self.session.session_id.clone()),
```

改成：

```rust
workspace_session_id: Some(self.session.session_id.clone()),
resume_provider_session_id: None,
```

- [ ] **Step 5: 更新其他构造点**

对 `rg -n "session_id:" src tests` 中属于 `StreamingProviderInput` 的构造点逐一替换。典型替换：

```rust
StreamingProviderInput {
    provider_type,
    role,
    prompt,
    working_dir,
    workspace_session_id: None,
    resume_provider_session_id: None,
    permission_mode,
    env_vars,
    timeout_secs,
}
```

`src/web/test_controls.rs` 中如果原来需要把产品 session id 传给测试控制逻辑，使用：

```rust
workspace_session_id: input.session_id.clone(),
resume_provider_session_id: None,
```

- [ ] **Step 6: 运行编译检查确认字段替换完整**

Run:

```bash
cargo check --locked
```

Expected: PASS。若报 `struct StreamingProviderInput has no field named session_id`，按报错路径继续替换为新字段。

- [ ] **Step 7: Commit**

```bash
git add src tests
git commit -m "refactor: split provider input session fields"
```

## Task 3: WorkspaceEngine 按角色保存和读取 provider conversation

**Files:**
- Modify: `src/product/workspace_engine.rs`
- Modify: `tests/it_core/workspace_ws_integration.rs`
- Test: `src/product/workspace_engine.rs` unit tests

- [ ] **Step 1: 写 failing 单测，author 第二轮续接第一轮 provider session**

在 `src/product/workspace_engine.rs` 的 `#[cfg(test)] mod tests` 内追加一个 recording provider。若已有 `RecordingStreamingProvider` 可复用，扩展它记录 `StreamingProviderInput` 和按调用次数返回 session id。测试代码：

```rust
#[derive(Default)]
struct SessionRecordingProvider {
    inputs: Arc<Mutex<Vec<StreamingProviderInput>>>,
    calls: Arc<Mutex<u32>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for SessionRecordingProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.inputs.lock().unwrap().push(input);
        let mut calls = self.calls.lock().unwrap();
        *calls += 1;
        let call_no = *calls;
        drop(calls);

        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let output = if call_no == 1 {
                "climb_stairs(n) 对 n <= 0 应该如何处理？\nA. 返回 0\nB. 抛出 ValueError\n"
            } else {
                "# Story Spec\n\n## 功能需求\n- 对 n <= 0 返回 0。\n\n## 成功标准\n- n <= 0 时返回 0。\n"
            };
            let _ = event_tx
                .send(ProviderEvent::Completed {
                    full_output: output.to_string(),
                    provider_session_id: Some("provider-author-session-1".to_string()),
                })
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        Err(ProviderAdapterError::not_implemented("streaming test provider"))
    }
}

#[tokio::test]
async fn author_choice_followup_resumes_author_provider_session() {
    let (event_tx, _event_rx) = mpsc::channel(32);
    let mut session = make_session("sess_resume_author");
    session.workspace_type = WorkspaceType::Story;
    session.author_provider = ProviderName::ClaudeCode;
    session.reviewer_provider = None;
    let mut engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(tempdir().unwrap().path().to_path_buf())),
        event_tx,
        session,
    );
    let provider = Arc::new(SessionRecordingProvider::default());

    let (_command_tx, command_rx) = mpsc::channel(8);
    engine
        .handle_user_message("开始生成 Story Spec".to_string(), provider.clone(), command_rx)
        .await;

    let prompt = engine
        .take_pending_author_choice_prompt(
            "author_choice_msg_001",
            vec!["A".to_string()],
            None,
        )
        .await
        .expect("pending author choice prompt");

    let (_command_tx2, command_rx2) = mpsc::channel(8);
    engine
        .handle_user_message(prompt, provider.clone(), command_rx2)
        .await;

    let inputs = provider.inputs.lock().unwrap();
    assert_eq!(inputs.len(), 2);
    assert_eq!(inputs[0].resume_provider_session_id, None);
    assert_eq!(
        inputs[1].resume_provider_session_id.as_deref(),
        Some("provider-author-session-1")
    );
}
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
cargo test --locked --lib author_choice_followup_resumes_author_provider_session
```

Expected: FAIL。失败原因应是 `resume_provider_session_id` 仍为 `None` 或缺少 helper。

- [ ] **Step 3: 在 WorkspaceSession 中增加 provider conversations**

在 `src/product/workspace_engine.rs` 的 `WorkspaceSession` 增加字段：

```rust
pub provider_conversations: Vec<ProviderConversationRef>,
```

在 `WorkspaceSession::from_record` 中填入：

```rust
provider_conversations: record.provider_conversations,
```

在测试 helper `make_session` 中填入：

```rust
provider_conversations: Vec::new(),
```

- [ ] **Step 4: 增加 conversation helper**

在 `impl WorkspaceEngine` 内加入：

```rust
fn provider_resume_session_id(
    &self,
    role: ProviderConversationRole,
    provider: &ProviderName,
) -> Option<String> {
    self.session
        .provider_conversations
        .iter()
        .find(|conversation| conversation.role == role && &conversation.provider == provider)
        .map(|conversation| conversation.provider_session_id.clone())
        .filter(|id| !id.trim().is_empty())
}

async fn record_provider_session(
    &mut self,
    role: ProviderConversationRole,
    provider: ProviderName,
    provider_session_id: Option<String>,
    node_id: Option<String>,
) {
    let Some(provider_session_id) = provider_session_id
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
    else {
        return;
    };
    let now = chrono::Utc::now().to_rfc3339();
    if let Some(existing) = self
        .session
        .provider_conversations
        .iter_mut()
        .find(|conversation| conversation.role == role && conversation.provider == provider)
    {
        existing.provider_session_id = provider_session_id;
        existing.updated_at = now;
        existing.last_node_id = node_id;
    } else {
        self.session.provider_conversations.push(ProviderConversationRef {
            role,
            provider,
            provider_session_id,
            updated_at: now,
            last_node_id: node_id,
        });
    }
    if let Some(store) = &self.lifecycle_store {
        let _ = store.replace_workspace_provider_conversations(
            &self.session.session_id,
            self.session.provider_conversations.clone(),
        );
    }
}
```

确保 imports 包含：

```rust
ProviderConversationRef, ProviderConversationRole,
```

- [ ] **Step 5: 在 input 构造时使用 resume id**

在 `build_streaming_input` 中：

```rust
let provider = self.session.author_provider.clone();
let resume_provider_session_id =
    self.provider_resume_session_id(ProviderConversationRole::Author, &provider);

Ok(StreamingProviderInput {
    provider_type: provider_type_for_name(&provider),
    role: AdapterRole::Orchestrator,
    prompt: self.build_prompt(user_content),
    working_dir,
    workspace_session_id: Some(self.session.session_id.clone()),
    resume_provider_session_id,
    permission_mode: ProviderPermissionMode::Supervised,
    env_vars: BTreeMap::new(),
    timeout_secs: 300,
})
```

在 `build_review_input` 中用 reviewer provider 和 `ProviderConversationRole::Reviewer`。

在 `build_revision_input` 中用 author provider 和 `ProviderConversationRole::Author`。

- [ ] **Step 6: provider 完成时保存 session id**

修改 `drive_provider_session` 的 completed 分支：

```rust
ProviderEvent::Completed {
    full_output,
    provider_session_id,
} => {
    if let Some(node_id) = node_id.as_deref() {
        let _ = self.flush_stream_buffer(node_id).await;
    }
    if let Some(provider) = agent.clone() {
        self.record_provider_session(
            ProviderConversationRole::Author,
            provider,
            provider_session_id,
            node_id.clone(),
        )
        .await;
    }
    self.complete_assistant_message(assistant_msg_id, full_output).await;
    return;
}
```

修改 `drive_reviewer_provider_session` 的 completed 分支为 reviewer role：

```rust
ProviderEvent::Completed {
    full_output: completed_output,
    provider_session_id,
} => {
    self.record_provider_session(
        ProviderConversationRole::Reviewer,
        reviewer.clone(),
        provider_session_id,
        node_id.clone(),
    )
    .await;
    ...
}
```

- [ ] **Step 7: 运行 author 续接测试确认通过**

Run:

```bash
cargo test --locked --lib author_choice_followup_resumes_author_provider_session
```

Expected: PASS。

- [ ] **Step 8: 写 reviewer 隔离单测**

在同一 test module 追加：

```rust
#[test]
fn provider_resume_session_id_is_isolated_by_role_and_provider() {
    let (event_tx, _event_rx) = mpsc::channel(8);
    let mut session = make_session("sess_role_isolation");
    session.author_provider = ProviderName::ClaudeCode;
    session.reviewer_provider = Some(ProviderName::ClaudeCode);
    session.provider_conversations = vec![ProviderConversationRef {
        role: ProviderConversationRole::Author,
        provider: ProviderName::ClaudeCode,
        provider_session_id: "author-session".to_string(),
        updated_at: "2026-06-01T00:00:00Z".to_string(),
        last_node_id: Some("node-author".to_string()),
    }];
    let engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(tempdir().unwrap().path().to_path_buf())),
        event_tx,
        session,
    );

    assert_eq!(
        engine.provider_resume_session_id(
            ProviderConversationRole::Author,
            &ProviderName::ClaudeCode
        ),
        Some("author-session".to_string())
    );
    assert_eq!(
        engine.provider_resume_session_id(
            ProviderConversationRole::Reviewer,
            &ProviderName::ClaudeCode
        ),
        None
    );
    assert_eq!(
        engine.provider_resume_session_id(
            ProviderConversationRole::Author,
            &ProviderName::Codex
        ),
        None
    );
}
```

- [ ] **Step 9: 运行 reviewer 隔离单测**

Run:

```bash
cargo test --locked --lib provider_resume_session_id_is_isolated_by_role_and_provider
```

Expected: PASS。

- [ ] **Step 10: Commit**

```bash
git add src/product/workspace_engine.rs
git commit -m "feat: resume workspace provider sessions by role"
```

## Task 4: Claude Code provider 使用 --resume

**Files:**
- Modify: `src/cross_cutting/claude_code_provider.rs`

- [ ] **Step 1: 写 failing 参数单测**

在 `src/cross_cutting/claude_code_provider.rs` 的 test module 中追加：

```rust
#[test]
fn claude_args_include_resume_when_provider_session_is_available() {
    let provider = ClaudeCodeProvider::new(PathBuf::from("claude"));
    let args = provider.build_args(
        ProviderPermissionMode::Supervised,
        Some("claude-session-123"),
    );

    assert!(args.contains(&"--resume".to_string()));
    assert!(args.contains(&"claude-session-123".to_string()));
    assert!(!args.contains(&"--continue".to_string()));
    assert!(!args.contains(&"--fork-session".to_string()));
}

#[test]
fn claude_args_do_not_include_resume_without_provider_session() {
    let provider = ClaudeCodeProvider::new(PathBuf::from("claude"));
    let args = provider.build_args(ProviderPermissionMode::Supervised, None);

    assert!(!args.contains(&"--resume".to_string()));
    assert!(!args.contains(&"--continue".to_string()));
}
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
cargo test --locked --lib claude_args_include_resume_when_provider_session_is_available
cargo test --locked --lib claude_args_do_not_include_resume_without_provider_session
```

Expected: 第一个测试因 `build_args` 签名或 `--resume` 缺失而失败。

- [ ] **Step 3: 修改 build_args 签名与实现**

把 `build_args` 改为：

```rust
fn build_args(
    &self,
    mode: ProviderPermissionMode,
    resume_provider_session_id: Option<&str>,
) -> Vec<String> {
    let mut args = vec![
        "-p".to_string(),
        "--verbose".to_string(),
        "--output-format=stream-json".to_string(),
        "--input-format=stream-json".to_string(),
        "--include-partial-messages".to_string(),
        "--replay-user-messages".to_string(),
    ];

    if let Some(session_id) = resume_provider_session_id
        .map(str::trim)
        .filter(|session_id| !session_id.is_empty())
    {
        args.push("--resume".to_string());
        args.push(session_id.to_string());
    }

    if mode == ProviderPermissionMode::Supervised {
        args.push("--permission-prompt-tool=stdio".to_string());
    }

    args
}
```

在 `start` 中调用：

```rust
let args = self.build_args(
    input.permission_mode.clone(),
    input.resume_provider_session_id.as_deref(),
);
```

- [ ] **Step 4: 运行 Claude 参数测试**

Run:

```bash
cargo test --locked --lib claude_args_include_resume_when_provider_session_is_available
cargo test --locked --lib claude_args_do_not_include_resume_without_provider_session
```

Expected: PASS。

- [ ] **Step 5: Commit**

```bash
git add src/cross_cutting/claude_code_provider.rs
git commit -m "feat: resume claude provider sessions"
```

## Task 5: Codex provider 复用 threadId

**Files:**
- Modify: `src/cross_cutting/codex_provider.rs`

- [ ] **Step 1: 创建 Codex resume fixture**

新增 `tests/fixtures/provider/codex_app_server_resume_fixture.sh`：

```bash
#!/usr/bin/env bash
set -euo pipefail

while IFS= read -r line; do
  if [[ "$line" == *'"method":"initialize"'* ]]; then
    echo '{"id":1,"result":{"capabilities":{}}}'
  elif [[ "$line" == *'"method":"initialized"'* ]]; then
    :
  elif [[ "$line" == *'"method":"thread/start"'* ]]; then
    echo '{"id":999,"error":{"code":-32000,"message":"thread/start must not be called during resume"}}'
    exit 1
  elif [[ "$line" == *'"method":"turn/start"'* ]]; then
    if [[ "$line" != *'"threadId":"codex-thread-123"'* ]]; then
      echo '{"id":2,"error":{"code":-32001,"message":"unexpected threadId"}}'
      exit 1
    fi
    echo '{"id":2,"result":{"id":"turn-1"}}'
    echo '{"method":"item/completed","params":{"item":{"id":"msg-1","type":"agentMessage","text":"resumed done"}}}'
    echo '{"method":"turn/completed","params":{"turnId":"turn-1"}}'
    exit 0
  fi
done
```

- [ ] **Step 2: 写 failing provider 测试，resume 时不发送 thread/start**

在 `src/cross_cutting/codex_provider.rs` 的 test module 中追加：

```rust
#[tokio::test]
async fn codex_resume_uses_existing_thread_without_starting_new_thread() {
    let fixture =
        executable_fixture("tests/fixtures/provider/codex_app_server_resume_fixture.sh");
    let provider = CodexProvider::new(fixture);
    let mut input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    input.resume_provider_session_id = Some("codex-thread-123".to_string());
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let completed = recv_completed(&mut session.events).await;

    assert_eq!(completed, "resumed done");
}
```

- [ ] **Step 3: 运行测试确认失败**

Run:

```bash
cargo test --locked --lib codex_resume_uses_existing_thread_without_starting_new_thread
```

Expected: FAIL。当前实现仍会发送 `thread/start`。

- [ ] **Step 4: 修改 run_codex_session 的 thread 选择逻辑**

把无条件 `thread/start` 改为：

```rust
let thread_id = if let Some(session_id) = input
    .resume_provider_session_id
    .as_deref()
    .map(str::trim)
    .filter(|session_id| !session_id.is_empty())
{
    Some(session_id.to_string())
} else {
    let thread_response = peer
        .request(json!({
            "jsonrpc": "2.0",
            "method": "thread/start",
            "params": {
                "cwd": input.working_dir,
                "approvalPolicy": match input.permission_mode {
                    ProviderPermissionMode::Auto => "never",
                    ProviderPermissionMode::Supervised => "on-request",
                },
                "ephemeral": true,
            },
        }))
        .await?;
    thread_response
        .pointer("/thread/id")
        .or_else(|| thread_response.pointer("/id"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
};
let turn_thread_id = thread_id.clone().unwrap_or_default();
```

保留完成事件：

```rust
ProviderEvent::Completed {
    full_output,
    provider_session_id: thread_id,
}
```

- [ ] **Step 5: 运行 Codex resume 测试**

Run:

```bash
cargo test --locked --lib codex_resume_uses_existing_thread_without_starting_new_thread
```

Expected: PASS。

- [ ] **Step 6: 运行 Codex 既有 provider 测试**

Run:

```bash
cargo test --locked --lib codex_provider
```

Expected: PASS。若测试过滤名不匹配，运行：

```bash
cargo test --locked --lib codex
```

Expected: PASS。

- [ ] **Step 7: Commit**

```bash
git add src/cross_cutting/codex_provider.rs tests/fixtures/provider/codex_app_server_resume_fixture.sh
git commit -m "feat: resume codex provider threads"
```

## Task 6: WebSocket 集成测试覆盖文本选择题后续接 author provider session

**Files:**
- Modify: `tests/it_core/workspace_ws_integration.rs`
- Modify: `src/product/workspace_engine.rs` if integration exposes a missing persistence edge

- [ ] **Step 1: 扩展 scripted provider 记录 resume id**

在 `tests/it_core/workspace_ws_integration.rs` 中找到 `workspace_ws_author_text_choice_blocks_reviewer_until_user_answers` 使用的 scripted provider。扩展 provider state：

```rust
#[derive(Default)]
struct ChoiceThenArtifactProviderState {
    calls: Mutex<u32>,
    resume_ids: Mutex<Vec<Option<String>>>,
}
```

在 provider `start` 中记录：

```rust
self.state
    .resume_ids
    .lock()
    .unwrap()
    .push(input.resume_provider_session_id.clone());
```

第一次完成事件返回：

```rust
provider_session_id: Some("author-provider-session-1".to_string()),
```

第二次完成事件也返回同一个 id：

```rust
provider_session_id: Some("author-provider-session-1".to_string()),
```

- [ ] **Step 2: 在集成测试中断言第二轮续接**

在用户发送 `choice_response` 并收到最终 `message_complete` 后追加：

```rust
let resume_ids = provider_state.resume_ids.lock().unwrap().clone();
assert_eq!(resume_ids.len(), 2);
assert_eq!(resume_ids[0], None);
assert_eq!(
    resume_ids[1].as_deref(),
    Some("author-provider-session-1")
);
```

- [ ] **Step 3: 运行集成测试**

Run:

```bash
cargo test --locked --test workspace_ws_integration workspace_ws_author_text_choice_blocks_reviewer_until_user_answers
```

Expected: PASS。

- [ ] **Step 4: 增加 reviewer 不复用 author session 的集成断言**

如果该测试启用了 reviewer，记录 reviewer provider input，并断言：

```rust
assert_eq!(reviewer_resume_ids[0], None);
```

如果该测试未启用 reviewer，新增一个小测试 `workspace_ws_reviewer_does_not_resume_author_provider_session`，流程：

1. author provider 返回有效 Story Spec 和 `provider_session_id = Some("author-session")`。
2. reviewer provider 记录 `resume_provider_session_id`。
3. reviewer provider 返回 pass verdict。
4. 断言 reviewer 第一轮 `resume_provider_session_id == None`。

- [ ] **Step 5: 运行 reviewer 隔离集成测试**

Run:

```bash
cargo test --locked --test workspace_ws_integration workspace_ws_reviewer_does_not_resume_author_provider_session
```

Expected: PASS。如果 reviewer 断言合并在原测试中，则运行原测试并确认 PASS。

- [ ] **Step 6: Commit**

```bash
git add tests/it_core/workspace_ws_integration.rs src/product/workspace_engine.rs
git commit -m "test: verify workspace provider session resume flow"
```

## Task 7: P1 全量定向验证

**Files:**
- No source edits unless verification finds a defect

- [ ] **Step 1: 格式检查**

Run:

```bash
cargo fmt --check
```

Expected: PASS。

- [ ] **Step 2: 工作区 provider conversation 定向测试**

Run:

```bash
cargo test --locked --test product_lifecycle_store workspace_session_provider_conversations_default_for_legacy_json
cargo test --locked --test product_lifecycle_store updates_workspace_session_provider_conversations
cargo test --locked --lib author_choice_followup_resumes_author_provider_session
cargo test --locked --lib provider_resume_session_id_is_isolated_by_role_and_provider
```

Expected: 全部 PASS。

- [ ] **Step 3: provider adapter 定向测试**

Run:

```bash
cargo test --locked --lib claude_args_include_resume_when_provider_session_is_available
cargo test --locked --lib claude_args_do_not_include_resume_without_provider_session
cargo test --locked --lib codex_resume_uses_existing_thread_without_starting_new_thread
```

Expected: 全部 PASS。

- [ ] **Step 4: WebSocket 集成测试**

Run:

```bash
cargo test --locked --test workspace_ws_integration workspace_ws_author_text_choice_blocks_reviewer_until_user_answers
```

Expected: PASS。

- [ ] **Step 5: 编译检查**

Run:

```bash
cargo check --locked
```

Expected: PASS。

- [ ] **Step 6: Commit verification fixes**

如果 Step 1-5 中修复了任何问题：

```bash
git add src tests
git commit -m "fix: stabilize workspace provider session resume"
```

如果没有修复任何问题，不创建空提交。

## P1 完成标准

- Story/Design Workspace author 用户选择后续跑时，第二轮 provider input 带上一轮 author provider session id。
- reviewer 第一轮不会读取 author provider session。
- Claude Code 有 resume id 时使用 `--resume <id>`，不使用 `--continue`。
- Codex 有 resume id 时不创建新 thread，直接用旧 thread id 启动 turn。
- `cargo check --locked` 通过。
- P1 完成后再执行 P2。
