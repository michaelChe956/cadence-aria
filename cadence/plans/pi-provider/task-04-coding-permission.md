# Task 4: Coding Workspace 默认 Auto + Pi 接入 + fail-fast（tasks 4.1, 4.2）

> 自包含任务文件。执行前请先读 `00-overview.md` 的 Global Constraints 与跨任务接口契约。依赖 Task 1（`ProviderName::Pi`）与 Task 2（`PiProvider` 注册）。

**Goal:** 把 Coding Workspace 各角色（Coder/Code Reviewer/Internal Reviewer）的新建默认权限模式改为 `Auto`（保留既有独立 `Supervised` 配置），并让三角色支持 Pi（Auto，fail-fast）。

**对应 spec requirement:**
- 「Provider 权限模式默认为 Auto，Pi 仅支持 Auto」（Coding 默认 Auto；Claude/Codex 保留 Supervised）
- 「Pi 在活跃 Provider 工作流中可发现且可选择」（Coding 选 Pi）
- 「所选 Provider 的失败直接报告且不切换」（fail-fast）

**背景：** Coding Workspace 已有 per-role 权限结构 `CodingRolePermissionModes`（`src/product/coding_models/provider_config.rs:17`），默认全 `Supervised`。本任务只改新建默认值为 `Auto`，不改 `CodingProviderPermissionMode` 类型，不动 Claude/Codex 的 Supervised 实现。

**Files:**
- Modify: `src/product/coding_models/provider_config.rs:23-31`（`CodingRolePermissionModes::default()` 全改 `Auto`）
- Modify: `src/product/coding_workspace_engine/`（三角色运行接 Pi；权限硬编码改读配置）
- Test: `src/product/coding_models/provider_config.rs`、`tests/it_product/product_coding_workspace_engine/`

**Interfaces:**
- Consumes: `CodingRolePermissionModes`、`CodingProviderPermissionMode`、`CodingRoleProviderConfigSnapshot`、`PiProvider`。
- Produces: `CodingRolePermissionModes::default()` 全 `Auto`；Coding 三角色可选 Pi。**不改 `CodingProviderPermissionMode` 类型。**

**约束（Decision 3）：** 只改**新建默认值**为 `Auto`；已持久化的显式 `Supervised` 值不受影响；旧记录缺 `permission_modes` 字段按新默认 `Auto` 反序列化（符合 Decision 3 与 Risks）。

---

## Step 1: 写失败测试 —— 新建默认 Auto + 显式值保留 + 旧记录缺字段 Auto

`src/product/coding_models/provider_config.rs` 的 `#[cfg(test)]` 加：

```rust
#[test]
fn coding_role_permission_modes_default_is_auto() {
    let modes = CodingRolePermissionModes::default();
    assert_eq!(modes.coder, CodingProviderPermissionMode::Auto);
    assert_eq!(modes.code_reviewer, CodingProviderPermissionMode::Auto);
    assert_eq!(modes.internal_reviewer, CodingProviderPermissionMode::Auto);
}

#[test]
fn explicit_supervised_value_preserved() {
    let json = serde_json::json!({
        "coder": "supervised", "code_reviewer": "supervised", "internal_reviewer": "supervised"
    });
    let modes: CodingRolePermissionModes = serde_json::from_value(json).unwrap();
    assert_eq!(modes.coder, CodingProviderPermissionMode::Supervised);
    assert_eq!(modes.code_reviewer, CodingProviderPermissionMode::Supervised);
}

#[test]
fn old_coding_snapshot_without_permission_modes_deserializes_to_auto() {
    // CodingRoleProviderConfigSnapshot.permission_modes 用 #[serde(default)]；缺字段按新默认 Auto
    let json = serde_json::json!({
        "coder": "claude_code", "code_reviewer": "codex",
        "internal_reviewer": "claude_code", "review_rounds": 1
    });
    let snapshot: CodingRoleProviderConfigSnapshot = serde_json::from_value(json).unwrap();
    assert_eq!(snapshot.permission_modes.coder, CodingProviderPermissionMode::Auto);
}
```

- [ ] Run: `cargo test -p cadence-aria coding_models`
- Expected: FAIL —— 默认值是 `Supervised`

## Step 2: `CodingRolePermissionModes::default()` 改 Auto

`src/product/coding_models/provider_config.rs:23-31`：

```rust
impl Default for CodingRolePermissionModes {
    fn default() -> Self {
        Self {
            coder: CodingProviderPermissionMode::Auto,
            code_reviewer: CodingProviderPermissionMode::Auto,
            internal_reviewer: CodingProviderPermissionMode::Auto,
        }
    }
}
```

⚠️ `CodingRoleProviderConfigSnapshot.permission_modes` 用 `#[serde(default)]`，改默认后旧记录缺字段按 `Auto` 反序列化——这正是契约要求，已由 Step 1 第三个测试覆盖。

- [ ] Run: `cargo test -p cadence-aria coding_models`
- Expected: PASS

## Step 3: 写失败测试 -- Pi+Supervised 规范化为 Auto + fail-fast（真实未实现行为）

注：Task 1/2 完成后，选 Pi 走 `PiProvider` 可能已通过 registry 路径执行，故"三角色可选 Pi"不作为 red test。本步聚焦 Task 4 真正未实现的行为：(a) Pi 角色的 Supervised mode 被规范化为 Auto；(b) Pi 运行失败不触发 Codex-only fresh retry。

**测试模板依据：** recording adapter 照 `src/product/workspace_engine/tests/part_06.rs:449` 的 `RecordingStreamingProvider`；Coding harness 照 `tests/it_product/product_coding_workspace_engine/part_04.rs:39-45` 的真实写法：`CodingAttemptStore::new(ProductAppPaths::new(...))` + `create_attempt(CreateCodingAttemptInput{..create_input()})` + `CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx)` + `execute_coding(&attempt, &provider, &CodingExecutionContext::default())`（provider 直接传入，不经 registry）。

```rust
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::fs;
use tempfile::tempdir;
use tokio::sync::mpsc;

/// 记录 start 次数与收到的 permission_mode
struct CountingProvider {
    starts: Arc<AtomicUsize>,
    seen_modes: Arc<Mutex<Vec<ProviderPermissionMode>>>,
    fail_on_start: bool,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for CountingProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        self.seen_modes.lock().unwrap().push(input.permission_mode.clone());
        if self.fail_on_start {
            return Err(ProviderAdapterError::execution_failed(None, String::new(), "pi failed", 0));
        }
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::Completed(ProviderCompletion::plain("done", Some("sess-1".into()))))
                .await;
        });
        Ok(ProviderSession { events: event_rx, commands: command_tx })
    }

    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(None, String::new(), "unused", 0))
    }
}

#[tokio::test]
async fn pi_role_with_supervised_mode_normalized_to_auto() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status("project_0001", "issue_0001", &attempt.id, CodingAttemptStatus::Running)
        .expect("running");

    let seen_modes = Arc::new(Mutex::new(Vec::new()));
    let provider = CountingProvider {
        starts: Arc::new(AtomicUsize::new(0)),
        seen_modes: seen_modes.clone(),
        fail_on_start: false,
    };
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    // coder 选 Pi 且 permission_modes.coder = Supervised（陈旧/非法输入）
    let config = CodingRoleProviderConfigSnapshot {
        coder: ProviderName::Pi,
        code_reviewer: ProviderName::ClaudeCode,
        internal_reviewer: ProviderName::ClaudeCode,
        review_rounds: 1,
        permission_modes: CodingRolePermissionModes {
            coder: CodingProviderPermissionMode::Supervised,
            code_reviewer: CodingProviderPermissionMode::Auto,
            internal_reviewer: CodingProviderPermissionMode::Auto,
        },
    };
    engine
        .execute_coding(&attempt, &provider, &CodingExecutionContext::default())
        .await
        .expect("execute coding");

    let modes = seen_modes.lock().unwrap();
    assert_eq!(modes[0], ProviderPermissionMode::Auto, "Pi 角色须归一化为 Auto");
}

#[tokio::test]
async fn pi_failure_does_not_trigger_fresh_retry() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status("project_0001", "issue_0001", &attempt.id, CodingAttemptStatus::Running)
        .expect("running");

    let pi_starts = Arc::new(AtomicUsize::new(0));
    let provider = CountingProvider {
        starts: pi_starts.clone(),
        seen_modes: Arc::new(Mutex::new(Vec::new())),
        fail_on_start: true,
    };
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let result = engine
        .execute_coding(&attempt, &provider, &CodingExecutionContext::default())
        .await;

    assert!(result.is_err(), "Pi 启动失败应报错");
    assert_eq!(pi_starts.load(Ordering::SeqCst), 1, "Pi 只启动一次（无 fresh retry）");
}
```

注：`CodingAttemptStore::new`/`ProductAppPaths::new`/`CreateCodingAttemptInput`/`create_input()`/`CodingAttemptStatus::Running`/`GitWorkspaceService::new()`/`CodingWorkspaceEngine::new(store, git, tx)`/`execute_coding(&attempt, &provider, &CodingExecutionContext::default())` 均为 `tests/it_product/product_coding_workspace_engine/part_04.rs` 现有 harness 与 API。`CodingRoleProviderConfigSnapshot` 需在 Coding 运行链路中传入（确认 `execute_coding` 如何读 config，可能在 `CodingExecutionContext` 或 attempt 上）。

- [ ] Run: `cargo test -p cadence-aria product_coding_workspace_engine`
- Expected: FAIL -- Pi+Supervised 未规范化；Pi 失败可能触发 fresh retry

- [ ] Run: `cargo test -p cadence-aria product_coding_workspace_engine`
- Expected: FAIL -- Pi+Supervised 未规范化；Pi 失败可能触发 fresh retry


## Step 4: Coding 运行链路接 Pi + fail-fast

`src/product/coding_workspace_engine/`：三角色运行经 registry 取 provider（Task 2 已注册 Pi）。确认 `StreamingProviderInput` 把 `permission_modes`（来自 `CodingRoleProviderConfigSnapshot`）传给 provider。

用 `rg -n "CodingProviderPermissionMode::Supervised" src/product/coding_workspace_engine/ -g '*.rs'` 定位权限硬编码，改为读 `CodingRolePermissionModes` 对应角色。

**fail-fast：** 用 `codegraph explore "coding_workspace_engine resume stall fresh retry 触发条件 coding.rs provider_stream.rs"` 确认 Codex resume-stall fresh retry（`coding.rs:184-206`、`provider_stream.rs:542-555`）是同 provider 内部重试还是换 provider：同 provider 内部 retry 保留；Pi 不实现该 retry（Pi 失败即终态）。测试断言 `pi_start_count == 1`。

- [ ] Run: `cargo test -p cadence-aria product_coding_workspace_engine`
- Expected: PASS

## Step 5: 全量受影响测试 + Commit

- [ ] Run:

```bash
cargo test -p cadence-aria coding_models
cargo test -p cadence-aria product_coding_workspace_engine
git add src/product/coding_models/provider_config.rs src/product/coding_workspace_engine/
git commit -m "feat(coding): default role permission modes to Auto and support Pi with fail-fast"
```

---

## 完成检查（对应 tasks 4.1/4.2）

- [ ] 4.1：Coding Workspace 各角色新建默认权限模式改为 `Auto`，保留独立 `Supervised` 配置。
- [ ] 4.2：Coder/Code Reviewer/Internal Reviewer 支持 Pi（Auto；失败直接报错，不做运行期降级）。
