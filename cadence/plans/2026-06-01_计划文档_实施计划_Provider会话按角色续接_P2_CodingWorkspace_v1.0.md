# Provider 会话按角色续接 P2 Coding Workspace Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 P1 的 provider conversation 基础上，让 Coding Workspace 的 coder、tester、analyst、code reviewer、internal reviewer provider run 按角色续接各自 provider 会话。

**Architecture:** Coding attempt 持久化 `provider_conversations`，每个 provider run 用 `CodingProviderRole` 映射到通用 `ProviderConversationRole`。`CodingWorkspaceEngine` 在启动 provider 前读取对应 role/provider 的 resume id，在 provider 完成后保存返回的 `provider_session_id`。

**Tech Stack:** Rust 1.95、Tokio、serde、Coding Workspace engine、CodingAttemptStore、Cargo 测试套件。

---

## 前置条件

执行本计划前必须完成 P1：

- `ProviderConversationRole` 和 `ProviderConversationRef` 已存在于 `src/product/models.rs`。
- `StreamingProviderInput` 已包含 `workspace_session_id` 和 `resume_provider_session_id`。
- Claude Code 和 Codex provider adapter 已支持 resume。

## File Structure

- Modify: `src/product/coding_models.rs`
  - 在 `CodingExecutionAttempt` 增加 `provider_conversations`，旧 JSON 默认空表。
- Modify: `src/product/coding_attempt_store.rs`
  - 创建 attempt 时初始化空 provider conversations。
  - 增加替换 attempt provider conversations 的持久化方法。
- Modify: `src/product/coding_workspace_engine.rs`
  - `CodingProviderStreamRun` 增加 provider role。
  - `execute_coding_with_commands`、`execute_testing_with_provider_commands`、`execute_code_review_with_commands`、`execute_rework_with_commands`、`execute_internal_pr_review_with_commands` 均读取对应 role 的 resume id。
  - provider 完成时保存 provider session。
- Modify: `tests/it_product/product_coding_attempt_store.rs`
  - 增加旧 JSON 兼容和 provider conversation 持久化测试。
- Modify: `tests/it_product/product_coding_workspace_engine.rs`
  - 增加 coder 续接、tester 不复用 coder、code reviewer 不复用 tester 的 engine 测试。
- Modify: `tests/it_product/product_coding_models.rs`
  - 更新 `CodingExecutionAttempt` serde roundtrip 期望。

## Task 1: CodingExecutionAttempt 持久化 provider conversations

**Files:**
- Modify: `src/product/coding_models.rs`
- Modify: `src/product/coding_attempt_store.rs`
- Modify: `tests/it_product/product_coding_attempt_store.rs`
- Modify: `tests/it_product/product_coding_models.rs`

- [ ] **Step 1: 写 failing 测试，覆盖旧 CodingExecutionAttempt JSON 缺省 provider_conversations**

在 `tests/it_product/product_coding_attempt_store.rs` 追加：

```rust
#[test]
fn coding_attempt_provider_conversations_default_for_legacy_json() {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = CodingAttemptStore::new(paths.clone());
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");

    let attempt_path = paths
        .root()
        .join("projects/project_0001/issues/issue_0001/coding-attempts")
        .join(format!("{}.json", attempt.id));
    let mut value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&attempt_path).unwrap()).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .remove("provider_conversations");
    std::fs::write(&attempt_path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

    let reloaded = store
        .get_attempt_by_id(&attempt.id)
        .expect("reload legacy coding attempt");
    assert!(reloaded.provider_conversations.is_empty());
}
```

- [ ] **Step 2: 写 failing 测试，覆盖 Coding attempt provider conversations 更新**

在同一测试文件追加：

```rust
#[test]
fn updates_coding_attempt_provider_conversations() {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = CodingAttemptStore::new(paths);
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");

    let conversations = vec![ProviderConversationRef {
        role: ProviderConversationRole::Coder,
        provider: ProviderName::ClaudeCode,
        provider_session_id: "coder-session-1".to_string(),
        updated_at: "2026-06-01T00:00:00Z".to_string(),
        last_node_id: Some("coding-node-1".to_string()),
    }];

    let updated = store
        .replace_attempt_provider_conversations(&attempt.id, conversations.clone())
        .expect("persist coding provider conversations");

    assert_eq!(updated.provider_conversations, conversations);
    let reloaded = store
        .get_attempt_by_id(&attempt.id)
        .expect("reload attempt");
    assert_eq!(reloaded.provider_conversations, conversations);
}
```

在测试文件 imports 中加入：

```rust
use cadence_aria::product::models::{
    ProviderConversationRef, ProviderConversationRole, ProviderName,
};
```

并移除原有单独的 `use cadence_aria::product::models::ProviderName;`。

- [ ] **Step 3: 运行测试确认失败**

Run:

```bash
cargo test --locked --test product_coding_attempt_store coding_attempt_provider_conversations_default_for_legacy_json
cargo test --locked --test product_coding_attempt_store updates_coding_attempt_provider_conversations
```

Expected: 第一个测试因字段不存在或反序列化失败而失败；第二个测试因方法或字段不存在而编译失败。

- [ ] **Step 4: 修改 CodingExecutionAttempt 模型**

在 `src/product/coding_models.rs` 的 imports 中把：

```rust
use crate::product::models::ProviderName;
```

改为：

```rust
use crate::product::models::{ProviderConversationRef, ProviderName};
```

在 `CodingExecutionAttempt` 中加入：

```rust
#[serde(default)]
pub provider_conversations: Vec<ProviderConversationRef>,
```

- [ ] **Step 5: 创建 attempt 时初始化字段**

在 `src/product/coding_attempt_store.rs` 创建 `CodingExecutionAttempt` 的位置加入：

```rust
provider_conversations: Vec::new(),
```

更新所有测试中手写的 `CodingExecutionAttempt` literal，加入：

```rust
provider_conversations: Vec::new(),
```

- [ ] **Step 6: 增加 store 更新方法**

在 `src/product/coding_attempt_store.rs` imports 中加入：

```rust
ProviderConversationRef,
```

在 `impl CodingAttemptStore` 中新增：

```rust
pub fn replace_attempt_provider_conversations(
    &self,
    attempt_id: &str,
    provider_conversations: Vec<ProviderConversationRef>,
) -> Result<CodingExecutionAttempt, ProductStoreError> {
    validate_relative_id(attempt_id)?;
    let mut attempt = self.find_attempt_by_id(attempt_id)?;
    let path = self.attempt_path(&attempt.project_id, &attempt.issue_id, &attempt.id);
    attempt.provider_conversations = provider_conversations;
    attempt.updated_at = Utc::now().to_rfc3339();
    write_json(&path, &attempt)?;
    Ok(attempt)
}
```

- [ ] **Step 7: 运行持久化测试**

Run:

```bash
cargo test --locked --test product_coding_attempt_store coding_attempt_provider_conversations_default_for_legacy_json
cargo test --locked --test product_coding_attempt_store updates_coding_attempt_provider_conversations
cargo test --locked --test product_coding_models
```

Expected: 全部 PASS。

- [ ] **Step 8: Commit**

```bash
git add src/product/coding_models.rs src/product/coding_attempt_store.rs tests/it_product/product_coding_attempt_store.rs tests/it_product/product_coding_models.rs
git commit -m "feat: persist coding provider conversations"
```

## Task 2: CodingWorkspaceEngine provider conversation helpers

**Files:**
- Modify: `src/product/coding_workspace_engine.rs`

- [ ] **Step 1: 写 role 映射单测**

在 `src/product/coding_workspace_engine.rs` 的 test module 中追加：

```rust
#[test]
fn coding_provider_role_maps_to_provider_conversation_role() {
    assert_eq!(
        provider_conversation_role_for_coding_role(&CodingProviderRole::Coder),
        ProviderConversationRole::Coder
    );
    assert_eq!(
        provider_conversation_role_for_coding_role(&CodingProviderRole::Tester),
        ProviderConversationRole::Tester
    );
    assert_eq!(
        provider_conversation_role_for_coding_role(&CodingProviderRole::Analyst),
        ProviderConversationRole::Analyst
    );
    assert_eq!(
        provider_conversation_role_for_coding_role(&CodingProviderRole::CodeReviewer),
        ProviderConversationRole::CodeReviewer
    );
    assert_eq!(
        provider_conversation_role_for_coding_role(&CodingProviderRole::InternalReviewer),
        ProviderConversationRole::InternalReviewer
    );
}
```

- [ ] **Step 2: 写 provider session 查询隔离单测**

在同一 test module 追加：

```rust
#[test]
fn coding_provider_resume_session_id_is_isolated_by_role_and_provider() {
    let store = CodingAttemptStore::new(ProductAppPaths::new(
        tempdir().expect("tempdir").path().join(".aria"),
    ));
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let mut attempt = test_attempt("coding_attempt_0001");
    attempt.provider_conversations = vec![
        ProviderConversationRef {
            role: ProviderConversationRole::Coder,
            provider: ProviderName::ClaudeCode,
            provider_session_id: "coder-session".to_string(),
            updated_at: "2026-06-01T00:00:00Z".to_string(),
            last_node_id: Some("coder-node".to_string()),
        },
        ProviderConversationRef {
            role: ProviderConversationRole::Tester,
            provider: ProviderName::ClaudeCode,
            provider_session_id: "tester-session".to_string(),
            updated_at: "2026-06-01T00:01:00Z".to_string(),
            last_node_id: Some("tester-node".to_string()),
        },
    ];

    assert_eq!(
        engine.provider_resume_session_id_for_attempt(
            &attempt,
            &CodingProviderRole::Coder,
            &ProviderName::ClaudeCode,
        ),
        Some("coder-session".to_string())
    );
    assert_eq!(
        engine.provider_resume_session_id_for_attempt(
            &attempt,
            &CodingProviderRole::Tester,
            &ProviderName::ClaudeCode,
        ),
        Some("tester-session".to_string())
    );
    assert_eq!(
        engine.provider_resume_session_id_for_attempt(
            &attempt,
            &CodingProviderRole::Coder,
            &ProviderName::Codex,
        ),
        None
    );
}
```

如果 test module 中没有 `test_attempt` helper，新增：

```rust
fn test_attempt(id: &str) -> CodingExecutionAttempt {
    CodingExecutionAttempt {
        id: id.to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        work_item_id: "work_item_0001".to_string(),
        attempt_no: 1,
        status: CodingAttemptStatus::Running,
        stage: CodingExecutionStage::Coding,
        base_branch: "HEAD".to_string(),
        branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
        worktree_path: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::ClaudeCode,
            reviewer: Some(ProviderName::Codex),
            review_rounds: 1,
        },
        provider_conversations: Vec::new(),
        rework_count: 0,
        max_auto_rework: 2,
        head_commit: None,
        pushed_remote: None,
        review_request_id: None,
        created_at: "2026-06-01T00:00:00Z".to_string(),
        updated_at: "2026-06-01T00:00:00Z".to_string(),
        completed_at: None,
    }
}
```

- [ ] **Step 3: 运行测试确认失败**

Run:

```bash
cargo test --locked --lib coding_provider_role_maps_to_provider_conversation_role
cargo test --locked --lib coding_provider_resume_session_id_is_isolated_by_role_and_provider
```

Expected: FAIL，因为 helper 不存在。

- [ ] **Step 4: 实现 helper**

在 `src/product/coding_workspace_engine.rs` 中加入：

```rust
fn provider_conversation_role_for_coding_role(
    role: &CodingProviderRole,
) -> ProviderConversationRole {
    match role {
        CodingProviderRole::Coder => ProviderConversationRole::Coder,
        CodingProviderRole::Tester => ProviderConversationRole::Tester,
        CodingProviderRole::Analyst => ProviderConversationRole::Analyst,
        CodingProviderRole::CodeReviewer => ProviderConversationRole::CodeReviewer,
        CodingProviderRole::InternalReviewer => ProviderConversationRole::InternalReviewer,
    }
}
```

在 `impl CodingWorkspaceEngine` 中加入：

```rust
fn provider_resume_session_id_for_attempt(
    &self,
    attempt: &CodingExecutionAttempt,
    role: &CodingProviderRole,
    provider: &ProviderName,
) -> Option<String> {
    let conversation_role = provider_conversation_role_for_coding_role(role);
    attempt
        .provider_conversations
        .iter()
        .find(|conversation| {
            conversation.role == conversation_role && &conversation.provider == provider
        })
        .map(|conversation| conversation.provider_session_id.clone())
        .filter(|id| !id.trim().is_empty())
}

fn record_attempt_provider_session(
    &self,
    attempt: &CodingExecutionAttempt,
    role: &CodingProviderRole,
    provider: ProviderName,
    provider_session_id: Option<String>,
    node_id: &str,
) -> Result<(), CodingWorkspaceEngineError> {
    let Some(provider_session_id) = provider_session_id
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
    else {
        return Ok(());
    };

    let conversation_role = provider_conversation_role_for_coding_role(role);
    let mut conversations = attempt.provider_conversations.clone();
    let now = chrono::Utc::now().to_rfc3339();
    if let Some(existing) = conversations
        .iter_mut()
        .find(|conversation| {
            conversation.role == conversation_role && conversation.provider == provider
        })
    {
        existing.provider_session_id = provider_session_id;
        existing.updated_at = now;
        existing.last_node_id = Some(node_id.to_string());
    } else {
        conversations.push(ProviderConversationRef {
            role: conversation_role,
            provider,
            provider_session_id,
            updated_at: now,
            last_node_id: Some(node_id.to_string()),
        });
    }

    self.store
        .replace_attempt_provider_conversations(&attempt.id, conversations)
        .map_err(CodingWorkspaceEngineError::from)?;
    Ok(())
}
```

确保 imports 包含：

```rust
ProviderConversationRef, ProviderConversationRole,
```

- [ ] **Step 5: 运行 helper 测试**

Run:

```bash
cargo test --locked --lib coding_provider_role_maps_to_provider_conversation_role
cargo test --locked --lib coding_provider_resume_session_id_is_isolated_by_role_and_provider
```

Expected: PASS。

- [ ] **Step 6: Commit**

```bash
git add src/product/coding_workspace_engine.rs
git commit -m "feat: map coding roles to provider conversations"
```

## Task 3: Coder run 保存并续接 provider session

**Files:**
- Modify: `src/product/coding_workspace_engine.rs`
- Modify: `tests/it_product/product_coding_workspace_engine.rs`

- [ ] **Step 1: 写 failing engine 测试，coder 第二轮续接第一轮 session**

在 `tests/it_product/product_coding_workspace_engine.rs` 追加 provider：

```rust
struct SessionInputCapturingProvider {
    inputs: Arc<Mutex<Vec<StreamingProviderInput>>>,
    outputs: Arc<Mutex<VecDeque<String>>>,
    provider_session_ids: Arc<Mutex<VecDeque<Option<String>>>>,
}

impl Default for SessionInputCapturingProvider {
    fn default() -> Self {
        Self::with_outputs(
            ["coding done"],
            [Some("coder-session-1".to_string())],
        )
    }
}

impl SessionInputCapturingProvider {
    fn with_outputs<const N: usize, const M: usize>(
        outputs: [&str; N],
        provider_session_ids: [Option<String>; M],
    ) -> Self {
        Self {
            inputs: Arc::new(Mutex::new(Vec::new())),
            outputs: Arc::new(Mutex::new(
                outputs.into_iter().map(ToOwned::to_owned).collect(),
            )),
            provider_session_ids: Arc::new(Mutex::new(provider_session_ids.into_iter().collect())),
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for SessionInputCapturingProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.inputs.lock().expect("inputs lock").push(input);
        let output = self
            .outputs
            .lock()
            .expect("outputs lock")
            .pop_front()
            .unwrap_or_else(|| "coding done".to_string());
        let provider_session_id = self
            .provider_session_ids
            .lock()
            .expect("provider session ids lock")
            .pop_front()
            .unwrap_or_else(|| Some("coder-session-1".to_string()));
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        event_tx
            .try_send(ProviderEvent::Completed {
                full_output: output,
                provider_session_id,
            })
            .expect("send completed");
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
```

确保测试文件 imports 包含：

```rust
use std::collections::VecDeque;
```

在同一文件追加测试：

```rust
#[tokio::test]
async fn coding_coder_run_resumes_previous_coder_provider_session() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree.clone()),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::ClaudeCode,
                reviewer: Some(ProviderName::Codex),
                review_rounds: 1,
            },
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let provider = SessionInputCapturingProvider::default();

    let first = engine
        .execute_coding(&attempt, &provider, &CodingExecutionContext::default())
        .await
        .expect("first coding run");
    let second = engine
        .execute_coding(&first, &provider, &CodingExecutionContext::default())
        .await
        .expect("second coding run");

    assert_eq!(second.stage, CodingExecutionStage::Coding);
    let inputs = provider.inputs.lock().expect("inputs lock");
    assert_eq!(inputs.len(), 2);
    assert_eq!(inputs[0].resume_provider_session_id, None);
    assert_eq!(
        inputs[1].resume_provider_session_id.as_deref(),
        Some("coder-session-1")
    );
}
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
cargo test --locked --test product_coding_workspace_engine coding_coder_run_resumes_previous_coder_provider_session
```

Expected: FAIL。第二次 input 的 resume id 当前为 `None`。

- [ ] **Step 3: 让 CodingProviderStreamRun 携带 provider role**

修改 `CodingProviderStreamRun`：

```rust
struct CodingProviderStreamRun<'a> {
    attempt: &'a CodingExecutionAttempt,
    node_id: &'a str,
    provider: &'a dyn StreamingProviderAdapter,
    legacy_input: &'a AdapterInput,
    input: StreamingProviderInput,
    provider_name: &'a ProviderName,
    provider_role: CodingProviderRole,
    command_rx: &'a mut mpsc::Receiver<CodingRunnerCommand>,
}
```

更新 `run_provider_stream_to_completion` destructuring，加入：

```rust
provider_role,
```

更新 coder 构造处：

```rust
provider_role: CodingProviderRole::Coder,
```

- [ ] **Step 4: 构造 coder input 时读取 resume id**

在 `execute_coding_with_commands` 创建 `StreamingProviderInput` 前加入：

```rust
let resume_provider_session_id = self.provider_resume_session_id_for_attempt(
    &attempt,
    &CodingProviderRole::Coder,
    &coder_provider,
);
```

构造 input：

```rust
let input = StreamingProviderInput {
    provider_type: legacy_input.provider_type.clone(),
    role: legacy_input.role.clone(),
    prompt: legacy_input.prompt.clone(),
    working_dir: worktree_path.clone(),
    workspace_session_id: Some(attempt.id.clone()),
    resume_provider_session_id,
    permission_mode: ProviderPermissionMode::Auto,
    env_vars: BTreeMap::new(),
    timeout_secs: legacy_input.timeout,
};
```

- [ ] **Step 5: provider 完成时保存 session id**

在 `run_provider_stream_to_completion` 的 completed 分支中把：

```rust
ProviderEvent::Completed { full_output: completed_output, .. } => {
```

改成：

```rust
ProviderEvent::Completed {
    full_output: completed_output,
    provider_session_id,
} => {
    let _ = self.record_attempt_provider_session(
        attempt,
        &provider_role,
        provider_name.clone(),
        provider_session_id,
        node_id,
    )?;
```

保留后续 `CodingMessageComplete` 和 `return Ok(full_output)`。

- [ ] **Step 6: 运行 coder 续接测试**

Run:

```bash
cargo test --locked --test product_coding_workspace_engine coding_coder_run_resumes_previous_coder_provider_session
```

Expected: PASS。

- [ ] **Step 7: Commit**

```bash
git add src/product/coding_workspace_engine.rs tests/it_product/product_coding_workspace_engine.rs
git commit -m "feat: resume coder provider sessions"
```

## Task 4: Tester provider run 保存并隔离 provider session

**Files:**
- Modify: `src/product/coding_workspace_engine.rs`
- Modify: `tests/it_product/product_coding_workspace_engine.rs`

- [ ] **Step 1: 写 failing 测试，tester 不复用 coder session**

在 `tests/it_product/product_coding_workspace_engine.rs` 追加：

```rust
#[tokio::test]
async fn coding_tester_does_not_resume_coder_provider_session() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let mut attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::ClaudeCode,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            ..create_input()
        })
        .expect("create attempt");
    attempt.provider_conversations = vec![ProviderConversationRef {
        role: ProviderConversationRole::Coder,
        provider: ProviderName::ClaudeCode,
        provider_session_id: "coder-session-1".to_string(),
        updated_at: "2026-06-01T00:00:00Z".to_string(),
        last_node_id: Some("coding-node-1".to_string()),
    }];
    let attempt = store
        .replace_attempt_provider_conversations(&attempt.id, attempt.provider_conversations)
        .expect("persist coder conversation");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let provider = SessionInputCapturingProvider::default();

    let _report = engine
        .execute_testing_with_provider(
            &attempt,
            &provider,
            &CodingExecutionContext::default(),
            &[],
            TesterAgentOptions::default(),
        )
        .await
        .expect("testing provider run");

    let inputs = provider.inputs.lock().expect("inputs lock");
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].resume_provider_session_id, None);
}
```

在 `SessionInputCapturingProvider` impl 中加入：

```rust
fn supports_tool_calls(&self) -> bool {
    true
}
```

并在 test imports 中加入：

```rust
use cadence_aria::product::models::{
    ProviderConversationRef, ProviderConversationRole, ProviderName,
};
use cadence_aria::product::tester_agent_loop::TesterAgentOptions;
```

移除原有单独的 `ProviderName` import。

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
cargo test --locked --test product_coding_workspace_engine coding_tester_does_not_resume_coder_provider_session
```

Expected: FAIL，直到 tester input 接入 role-based resume。

- [ ] **Step 3: 为 tester input 填入 role-based resume**

在 `execute_testing_with_provider_commands` 创建 `StreamingProviderInput` 前加入：

```rust
let resume_provider_session_id = self.provider_resume_session_id_for_attempt(
    &attempt,
    &CodingProviderRole::Tester,
    &tester_provider,
);
```

input 中填入：

```rust
workspace_session_id: Some(attempt.id.clone()),
resume_provider_session_id,
```

- [ ] **Step 4: tester 完成时保存 provider session**

在 tester loop 的 completed 分支中把：

```rust
ProviderEvent::Completed { full_output: completed_output, .. } => {
```

改成：

```rust
ProviderEvent::Completed {
    full_output: completed_output,
    provider_session_id,
} => {
    let _ = self.record_attempt_provider_session(
        &attempt,
        &CodingProviderRole::Tester,
        tester_provider.clone(),
        provider_session_id,
        &node.id,
    )?;
```

保留后续 full output 覆盖、message complete 和 `break`。

- [ ] **Step 5: 运行 tester 隔离测试**

Run:

```bash
cargo test --locked --test product_coding_workspace_engine coding_tester_does_not_resume_coder_provider_session
```

Expected: PASS。

- [ ] **Step 6: Commit**

```bash
git add src/product/coding_workspace_engine.rs tests/it_product/product_coding_workspace_engine.rs
git commit -m "feat: resume tester provider sessions"
```

## Task 5: Review 类 provider run 接入 role-based resume

**Files:**
- Modify: `src/product/coding_workspace_engine.rs`
- Modify: `tests/it_product/product_coding_workspace_engine.rs`

- [ ] **Step 1: 为 code reviewer input 填入 role-based resume**

在 `execute_code_review_with_commands` 创建 `provider_input` 前加入：

```rust
let resume_provider_session_id = self.provider_resume_session_id_for_attempt(
    &attempt,
    &CodingProviderRole::CodeReviewer,
    &reviewer,
);
let mut provider_input = streaming_input_from_adapter(&input, worktree_path.clone());
provider_input.workspace_session_id = Some(attempt.id.clone());
provider_input.resume_provider_session_id = resume_provider_session_id;
```

`CodingProviderStreamRun` 构造中加入：

```rust
provider_role: CodingProviderRole::CodeReviewer,
```

- [ ] **Step 2: 为 analyst/rework input 填入 role-based resume**

在 `execute_rework_with_commands` 构造 `provider_input` 前使用 `CodingProviderRole::Analyst` 和 `analyst_provider`：

```rust
let resume_provider_session_id = self.provider_resume_session_id_for_attempt(
    &attempt,
    &CodingProviderRole::Analyst,
    &analyst_provider,
);
let mut provider_input = streaming_input_from_adapter(&input, worktree_path.clone());
provider_input.workspace_session_id = Some(attempt.id.clone());
provider_input.resume_provider_session_id = resume_provider_session_id;
```

`CodingProviderStreamRun` 构造中加入：

```rust
provider_role: CodingProviderRole::Analyst,
```

不要在 `execute_rework_with_commands` 中使用 `CodingProviderRole::Coder` 或 `coder_provider`；当前 rework provider run 是 Analyst 路径。

- [ ] **Step 3: 为 internal reviewer input 填入 role-based resume**

在 `execute_internal_pr_review_with_commands` 构造 provider input 前使用 `CodingProviderRole::InternalReviewer`：

```rust
let resume_provider_session_id = self.provider_resume_session_id_for_attempt(
    &attempt,
    &CodingProviderRole::InternalReviewer,
    &reviewer,
);
let mut provider_input = streaming_input_from_adapter(&input, worktree_path.clone());
provider_input.workspace_session_id = Some(attempt.id.clone());
provider_input.resume_provider_session_id = resume_provider_session_id;
```

`CodingProviderStreamRun` 构造中加入：

```rust
provider_role: CodingProviderRole::InternalReviewer,
```

- [ ] **Step 4: 写 code reviewer provider run 隔离测试**

在 `tests/it_product/product_coding_workspace_engine.rs` 追加：

```rust
#[tokio::test]
async fn coding_code_reviewer_run_does_not_resume_coder_or_tester_session() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    fs::write(worktree.join("src.txt"), "hello\nreviewed\n").expect("modify file");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            base_branch: "HEAD".to_string(),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::ClaudeCode,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            ..create_input()
        })
        .expect("create attempt");
    let attempt = store
        .replace_attempt_provider_conversations(
            &attempt.id,
            vec![
                ProviderConversationRef {
                    role: ProviderConversationRole::Coder,
                    provider: ProviderName::ClaudeCode,
                    provider_session_id: "coder-session-1".to_string(),
                    updated_at: "2026-06-01T00:00:00Z".to_string(),
                    last_node_id: Some("coding-node-1".to_string()),
                },
                ProviderConversationRef {
                    role: ProviderConversationRole::Tester,
                    provider: ProviderName::ClaudeCode,
                    provider_session_id: "tester-session-1".to_string(),
                    updated_at: "2026-06-01T00:01:00Z".to_string(),
                    last_node_id: Some("testing-node-1".to_string()),
                },
            ],
        )
        .expect("persist conversations");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = SessionInputCapturingProvider::with_outputs(
        [r#"{"verdict":"approve","summary":"code review ok","findings":[]}"#],
        [Some("code-reviewer-session-1".to_string())],
    );

    let report = engine
        .execute_code_review(&attempt, &provider)
        .await
        .expect("code review provider run");

    assert_eq!(report.verdict, ReviewVerdict::Approve);
    let inputs = provider.inputs.lock().expect("inputs lock");
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].resume_provider_session_id, None);
    let updated = store
        .get_attempt("project_0001", "issue_0001", &attempt.id)
        .expect("updated attempt");
    assert!(updated.provider_conversations.iter().any(|conversation| {
        conversation.role == ProviderConversationRole::CodeReviewer
            && conversation.provider == ProviderName::ClaudeCode
            && conversation.provider_session_id == "code-reviewer-session-1"
    }));
}
```

- [ ] **Step 5: 写 analyst provider run 续接测试**

在同一测试文件追加：

```rust
#[tokio::test]
async fn coding_analyst_rework_resumes_analyst_provider_session() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::ClaudeCode,
                reviewer: Some(ProviderName::Codex),
                review_rounds: 1,
            },
            ..create_input()
        })
        .expect("create attempt");
    let attempt = store
        .replace_attempt_provider_conversations(
            &attempt.id,
            vec![ProviderConversationRef {
                role: ProviderConversationRole::Analyst,
                provider: ProviderName::ClaudeCode,
                provider_session_id: "analyst-session-1".to_string(),
                updated_at: "2026-06-01T00:00:00Z".to_string(),
                last_node_id: Some("rework-node-1".to_string()),
            }],
        )
        .expect("persist analyst conversation");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::Testing,
        )
        .expect("testing stage");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let provider = SessionInputCapturingProvider::with_outputs(
        [r#"{"verdict":"no_issue","summary":"testing ok"}"#],
        [Some("analyst-session-2".to_string())],
    );

    engine
        .execute_rework(&attempt, "testing evidence", &provider)
        .await
        .expect("analyst rework provider run");

    let inputs = provider.inputs.lock().expect("inputs lock");
    assert_eq!(inputs.len(), 1);
    assert_eq!(
        inputs[0].resume_provider_session_id.as_deref(),
        Some("analyst-session-1")
    );
}
```

- [ ] **Step 6: 写 internal reviewer provider run 续接测试**

在同一测试文件追加：

```rust
#[tokio::test]
async fn coding_internal_reviewer_resumes_internal_reviewer_provider_session() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    fs::write(worktree.join("src.txt"), "hello\ninternal review\n").expect("modify file");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            base_branch: "HEAD".to_string(),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::ClaudeCode,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            ..create_input()
        })
        .expect("create attempt");
    let attempt = store
        .replace_attempt_provider_conversations(
            &attempt.id,
            vec![ProviderConversationRef {
                role: ProviderConversationRole::InternalReviewer,
                provider: ProviderName::ClaudeCode,
                provider_session_id: "internal-reviewer-session-1".to_string(),
                updated_at: "2026-06-01T00:00:00Z".to_string(),
                last_node_id: Some("internal-review-node-1".to_string()),
            }],
        )
        .expect("persist internal reviewer conversation");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::ReviewRequest,
        )
        .expect("review request stage");
    store
        .save_review_request(&sample_review_request(&attempt.id))
        .expect("save review request");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let provider = SessionInputCapturingProvider::with_outputs(
        [r#"{"verdict":"approve","summary":"internal review ok","findings":[],"impact_scope":["src"],"pr_description":"实现 work item","commit_message_suggestion":"feat: implement work item"}"#],
        [Some("internal-reviewer-session-2".to_string())],
    );

    let review = engine
        .execute_internal_pr_review(&attempt, &provider)
        .await
        .expect("internal reviewer provider run");

    assert_eq!(review.verdict, ReviewVerdict::Approve);
    let inputs = provider.inputs.lock().expect("inputs lock");
    assert_eq!(inputs.len(), 1);
    assert_eq!(
        inputs[0].resume_provider_session_id.as_deref(),
        Some("internal-reviewer-session-1")
    );
}
```

确保测试文件 imports 包含：

```rust
use cadence_aria::product::models::{
    ProviderConversationRef, ProviderConversationRole, ProviderName,
};
```

- [ ] **Step 7: 运行 review 类 provider run 测试**

Run:

```bash
cargo test --locked --test product_coding_workspace_engine coding_code_reviewer_run_does_not_resume_coder_or_tester_session
cargo test --locked --test product_coding_workspace_engine coding_analyst_rework_resumes_analyst_provider_session
cargo test --locked --test product_coding_workspace_engine coding_internal_reviewer_resumes_internal_reviewer_provider_session
```

Expected: 全部 PASS。

- [ ] **Step 8: 运行编译检查**

Run:

```bash
cargo check --locked
```

Expected: PASS。

- [ ] **Step 9: Commit**

```bash
git add src/product/coding_workspace_engine.rs tests/it_product/product_coding_workspace_engine.rs
git commit -m "feat: resume coding review provider sessions"
```

## Task 6: P2 全量定向验证

**Files:**
- No source edits unless verification finds a defect

- [ ] **Step 1: 格式检查**

Run:

```bash
cargo fmt --check
```

Expected: PASS。

- [ ] **Step 2: Coding attempt store 测试**

Run:

```bash
cargo test --locked --test product_coding_attempt_store coding_attempt_provider_conversations_default_for_legacy_json
cargo test --locked --test product_coding_attempt_store updates_coding_attempt_provider_conversations
cargo test --locked --test product_coding_models
```

Expected: 全部 PASS。

- [ ] **Step 3: Coding workspace engine 定向测试**

Run:

```bash
cargo test --locked --lib coding_provider_role_maps_to_provider_conversation_role
cargo test --locked --lib coding_provider_resume_session_id_is_isolated_by_role_and_provider
cargo test --locked --test product_coding_workspace_engine coding_coder_run_resumes_previous_coder_provider_session
cargo test --locked --test product_coding_workspace_engine coding_tester_does_not_resume_coder_provider_session
cargo test --locked --test product_coding_workspace_engine coding_code_reviewer_run_does_not_resume_coder_or_tester_session
cargo test --locked --test product_coding_workspace_engine coding_analyst_rework_resumes_analyst_provider_session
cargo test --locked --test product_coding_workspace_engine coding_internal_reviewer_resumes_internal_reviewer_provider_session
```

Expected: 全部 PASS。

- [ ] **Step 4: 编译检查**

Run:

```bash
cargo check --locked
```

Expected: PASS。

- [ ] **Step 5: P1/P2 联合冒烟**

Run:

```bash
cargo test --locked --test workspace_ws_integration workspace_ws_author_text_choice_blocks_reviewer_until_user_answers
cargo test --locked --test product_coding_workspace_engine coding_coder_run_resumes_previous_coder_provider_session
```

Expected: 两个测试 PASS。

- [ ] **Step 6: Commit verification fixes**

如果 Step 1-5 中修复了任何问题：

```bash
git add src tests
git commit -m "fix: stabilize coding provider session resume"
```

如果没有修复任何问题，不创建空提交。

## P2 完成标准

- Coding attempt 持久化 provider conversations，旧 JSON 默认空表。
- coder 后续 run 续接 coder provider session。
- tester 不复用 coder session。
- code reviewer 不复用 tester 或 coder session。
- analyst 和 internal reviewer 使用各自 role 的 provider session。
- `cargo check --locked` 通过。
