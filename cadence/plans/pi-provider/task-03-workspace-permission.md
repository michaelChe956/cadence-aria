# Task 3: 普通 Workspace 权限持久化 + Pi 接入 + fail-fast（tasks 3.1, 3.2）

> 自包含任务文件。执行前请先读 `00-overview.md` 的 Global Constraints 与跨任务接口契约。依赖 Task 1（`ProviderName::Pi`）与 Task 2（`PiProvider` 注册）。

**Goal:** 给普通 Workspace（Story/Design/Work Item）的 Author/Reviewer 持久化独立权限模式（默认 `Auto`），把运行链路从硬编码 `Supervised` 改为读配置，并让 Author/Reviewer/返修运行支持 Pi（Auto，fail-fast）。

**对应 spec requirement:**
- 「Provider 权限模式默认为 Auto，Pi 仅支持 Auto」（普通 Workspace 权限持久化 + 默认 Auto）
- 「Pi 在活跃 Provider 工作流中可发现且可选择」（普通 Workspace 选 Pi）
- 「所选 Provider 的失败直接报告且不切换」（fail-fast）

**背景：** 普通 Workspace 的**真正持久化实体**是 `WorkspaceSessionRecord`（`src/product/models/workspace.rs:31`），运行时实体是 `WorkspaceSession`（`src/product/workspace_engine/types.rs:72`），两者目前只有 `author_provider`/`reviewer_provider`/`review_rounds`，**无权限模式字段**。`ProviderConfigSnapshot`（`src/web/workspace_ws_types/common.rs:112`）只是传输/快照 DTO。运行链路多处把 `StreamingProviderInput.permission_mode` 硬编码为 `Supervised`（`prompts.rs:154/177`、`review.rs` 多处、`review_repair.rs:49`、`revision.rs:55`）。

**Files:**
- Modify: `src/cross_cutting/streaming_provider/mod.rs:34`（`ProviderPermissionMode` 加 `Serialize, Deserialize` derive）
- Modify: `src/product/models/workspace.rs`（定义 `WorkspaceRolePermissionModes` + `WorkspaceSessionRecord` 加 `permission_modes`）
- Modify: `src/product/workspace_engine/types.rs:72`（`WorkspaceSession` 加 `permission_modes`）+ `types.rs:91`（`from_record` 映射）
- Modify: `src/web/workspace_ws_types/common.rs:112`（`ProviderConfigSnapshot` 加 `permission_modes`，`#[serde(default)]`）
- Modify: `src/product/workspace_engine/lifecycle.rs:617-649`（`start_generation()` 锁定权限模式到 session + store）
- Modify: `src/product/lifecycle_store/inputs.rs:174-185`（`CreateWorkspaceSessionInput` 处理初始权限模式）
- Modify: `src/product/lifecycle_store/workspace.rs:231-251`（创建记录时初始化 `permission_modes`）与 `:407-425`（更新接口或新增 `update_workspace_session_permission_modes`）
- Modify: `src/product/workspace_engine/session_state.rs:205-222`、`session_state/timeline.rs:412-419`（timeline snapshot 复制权限模式）
- Modify: `src/product/workspace_engine/lifecycle_recovery.rs:117-122`、`plan_repair_recovery.rs:37-41`、`plan_repair_transaction.rs:755-759`、`web/handlers/coding.rs:442-456`（其余 `ProviderConfigSnapshot` 构造点，见 Step 6 盘点）
- Modify: `src/product/workspace_engine/prompts.rs:154,177`、`review.rs`、`review_repair.rs:49`、`revision.rs:55`（硬编码 `Supervised` 改读配置）
- Test: `src/product/models/workspace.rs`、`src/product/workspace_engine/tests/`

**Interfaces:**
- Consumes: `ProviderConfigSnapshot`、`ProviderPermissionMode`、`StreamingProviderInput`、`ProviderName::Pi`、`PiProvider`。
- Produces: `WorkspaceRolePermissionModes { author: ProviderPermissionMode, reviewer: ProviderPermissionMode }`（`Default` 全 `Auto`）；`WorkspaceSessionRecord.permission_modes`（`#[serde(default)]`）；`WorkspaceSession.permission_modes`。**Task 5 前端依赖 `ProviderConfigSnapshot.permission_modes`。**

**约束（Decision 3）：** `WorkspaceRolePermissionModes` 独立于 Coding 的 `CodingProviderPermissionMode`，不合并；旧持久化会话缺字段按 `Auto`；权限模式变更只影响后续启动的运行。

---

## Step 1: 写失败测试 —— `WorkspaceRolePermissionModes` 默认 Auto

`src/product/models/workspace.rs` 的 `#[cfg(test)]` 加：

```rust
#[test]
fn workspace_role_permission_modes_default_is_auto() {
    let modes = WorkspaceRolePermissionModes::default();
    assert_eq!(modes.author, ProviderPermissionMode::Auto);
    assert_eq!(modes.reviewer, ProviderPermissionMode::Auto);
}
```

- [ ] Run: `cargo test -p cadence-aria workspace`
- Expected: FAIL —— `WorkspaceRolePermissionModes` 未定义

## Step 2: 定义 `WorkspaceRolePermissionModes` + `ProviderPermissionMode` 加 serde derive

`src/cross_cutting/streaming_provider/mod.rs:34` 给 `ProviderPermissionMode` 加 `Serialize, Deserialize`（当前只有 `Debug, Clone, PartialEq, Eq`）：

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderPermissionMode {
    Auto,
    Supervised,
}
```

`src/product/models/workspace.rs` 定义：

```rust
use crate::cross_cutting::streaming_provider::ProviderPermissionMode;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceRolePermissionModes {
    pub author: ProviderPermissionMode,
    pub reviewer: ProviderPermissionMode,
}

impl Default for WorkspaceRolePermissionModes {
    fn default() -> Self {
        Self {
            author: ProviderPermissionMode::Auto,
            reviewer: ProviderPermissionMode::Auto,
        }
    }
}
```

- [ ] Run: `cargo test -p cadence-aria workspace`
- Expected: PASS

## Step 3: 写失败测试 —— 旧 `WorkspaceSessionRecord` 缺字段反序列化为 Auto

`src/product/models/workspace.rs` 的 `#[cfg(test)]` 加（用真实旧记录 JSON，`status` 用合法的 `"open"`，不是 `"active"`）：

```rust
#[test]
fn old_workspace_session_record_without_permission_modes_deserializes_to_auto() {
    let json = serde_json::json!({
        "id": "s1", "project_id": "p1", "issue_id": "i1", "entity_id": "e1",
        "workspace_type": "story", "status": "open",
        "author_provider": "claude_code", "reviewer_provider": "codex",
        "review_rounds": 1, "superpowers_enabled": false, "openspec_enabled": false,
        "messages": [], "created_at": "", "updated_at": ""
    });
    let record: WorkspaceSessionRecord = serde_json::from_value(json).unwrap();
    assert_eq!(record.permission_modes.author, ProviderPermissionMode::Auto);
    assert_eq!(record.permission_modes.reviewer, ProviderPermissionMode::Auto);
}
```

注：`WorkspaceSessionStatus` 的合法 serde 值见 `workspace.rs:17-27`（`open` 等），不要用 `"active"`。

- [ ] Run: `cargo test -p cadence-aria workspace`
- Expected: FAIL —— `WorkspaceSessionRecord` 无 `permission_modes` 字段

## Step 4: `WorkspaceSessionRecord` 加 `permission_modes`（`#[serde(default)]`）

`src/product/models/workspace.rs:31`：

```rust
    #[serde(default)]
    pub permission_modes: WorkspaceRolePermissionModes,
```

- [ ] Run: `cargo test -p cadence-aria workspace`
- Expected: PASS

## Step 5: 运行时 `WorkspaceSession` 加字段 + `from_record` 映射

`src/product/workspace_engine/types.rs:72` 加：

```rust
    pub permission_modes: WorkspaceRolePermissionModes,
```

`types.rs:91` `from_record` 加：

```rust
    permission_modes: record.permission_modes.clone(),
```

- [ ] Run: `cargo test -p cadence-aria workspace_engine`
- Expected: 编译通过（手工构造 `WorkspaceSession` 的测试 fixture 可能需补 `permission_modes` 字段，见 Step 6）

## Step 6: 盘点所有 `ProviderConfigSnapshot` 构造点并补权限模式

`ProviderConfigSnapshot` 加字段后，所有构造点编译错。先盘点：

```bash
rg -n "ProviderConfigSnapshot\s*\{" src/ -g '*.rs' | grep -v test
```

已确认的构造点（按语义分类处理）：

| 位置 | 权限模式来源 |
|---|---|
| `lifecycle.rs:117-121` | 从 session 读 |
| `lifecycle_recovery.rs:117-122` | 从 session 读（恢复路径） |
| `session_state.rs:205-222` | 从 session 读 |
| `session_state/timeline.rs:412-419` | 从 session 读 |
| `plan_repair_recovery.rs:37-41` | 从 session 读 |
| `plan_repair_transaction.rs:755-759` | 从 session 读 |
| `web/handlers/coding.rs:427-446,467-498` | **有已确认 session 的分支**（现已从 session 复制 provider/rounds）用 `session.permission_modes.clone()`，与 provider/rounds 同源；**仅无 session 的 fallback 分支**用 `WorkspaceRolePermissionModes::default()`。**不**用 `CodingRolePermissionModes`（两类型独立）。Coding 自己的每角色模式存在 `CodingRoleProviderConfigSnapshot.permission_modes`，不从本字段复制。补回归测试：已确认 Work Item session 的非默认 mode 出现在输出 snapshot 中 |

`src/web/workspace_ws_types/common.rs:112` `ProviderConfigSnapshot` 加：

```rust
    #[serde(default)]
    pub permission_modes: WorkspaceRolePermissionModes,
```

每个构造点从对应 session 复制 `permission_modes`。

- [ ] Run: `cargo test -p cadence-aria workspace_engine`
- Expected: 编译通过，timeline snapshot 测试断言 `permission_modes` 与 session 一致

## Step 7: `start_generation()` 锁定权限模式到 session + store，并对 Pi 归一化为 Auto（高2 + 服务端归一化）

`src/product/workspace_engine/lifecycle.rs:617-649`：`start_generation()` 从 wire `ProviderConfigSnapshot` 读 `permission_modes`，写入 `WorkspaceSession`。

**服务端归一化（Pi 仅 Auto）：** 锁定前，若 `author_provider == ProviderName::Pi` 则强制 `permission_modes.author = Auto`；`reviewer_provider == Pi` 同理。前端过滤不能防陈旧数据或直接 API/WS 输入，必须服务端兜底。

**store 层（高2）：**
- `src/product/lifecycle_store/inputs.rs:174-184` `CreateWorkspaceSessionInput`：创建时若不携带权限模式，用 `WorkspaceRolePermissionModes::default()`（全 Auto）。
- `src/product/lifecycle_store/workspace.rs:231-251` 创建记录处：初始化 `permission_modes` 字段。
- `src/product/lifecycle_store/workspace.rs:407-421` 现有 `update_workspace_session_providers()` 只写 author/reviewer；新增 `update_workspace_session_permission_modes(...)`（或扩展为 author/reviewer/rounds/modes 原子更新），供 `start_generation()` 调用。

测试（`src/product/workspace_engine/tests/` 或 `lifecycle_store` 测试模块）：

```rust
#[tokio::test]
async fn new_session_defaults_permission_modes_to_auto() {
    let store = test_lifecycle_store();
    let record = store
        .create_workspace_session(create_input_without_permission_modes())
        .await
        .expect("create session");
    assert_eq!(record.permission_modes.author, ProviderPermissionMode::Auto);
    assert_eq!(record.permission_modes.reviewer, ProviderPermissionMode::Auto);
}

#[tokio::test]
async fn start_generation_locks_selected_modes_into_store() {
    let (engine, store) = engine_with_store();
    let wire = ProviderConfigSnapshot {
        author: ProviderName::ClaudeCode,
        reviewer: Some(ProviderName::Codex),
        review_rounds: 1,
        permission_modes: WorkspaceRolePermissionModes {
            author: ProviderPermissionMode::Supervised,
            reviewer: ProviderPermissionMode::Auto,
        },
    };
    engine.start_generation(wire).await.expect("start generation");
    let reread = store.load_workspace_session(session_id).await.expect("reload");
    assert_eq!(reread.permission_modes.author, ProviderPermissionMode::Supervised);
}

#[tokio::test]
async fn start_generation_normalizes_pi_role_to_auto() {
    let (engine, store) = engine_with_store();
    let wire = ProviderConfigSnapshot {
        author: ProviderName::Pi,
        reviewer: None,
        review_rounds: 1,
        permission_modes: WorkspaceRolePermissionModes {
            author: ProviderPermissionMode::Supervised, // 陈旧/非法输入
            reviewer: ProviderPermissionMode::Auto,
        },
    };
    engine.start_generation(wire).await.expect("start generation");
    let reread = store.load_workspace_session(session_id).await.expect("reload");
    // Pi 仅 Auto：服务端归一化
    assert_eq!(reread.permission_modes.author, ProviderPermissionMode::Auto);
}
```

注：`test_lifecycle_store()`/`engine_with_store()`/`create_input_without_permission_modes()` 参照 `src/product/workspace_engine/tests/` 现有 harness 构造（如 `part_01.rs` 的 session/store 搭建方式）。

若 reviewer 关闭（`reviewer: None`），明确 reviewer mode 语义（保留或归零），并测试。

- [ ] Run: `cargo test -p cadence-aria workspace_engine`
- [ ] Run: `cargo test -p cadence-aria lifecycle_store`
- Expected: PASS


## Step 8: 写失败测试 —— Author 运行读 session 权限模式

`src/product/workspace_engine/tests/` 加：构造 `permission_modes.author = Auto` 的 session，调 Author 运行的 `build_streaming_input`（`prompts.rs`），断言返回的 `StreamingProviderInput.permission_mode == Auto`（非硬编码 Supervised）。

- [ ] Run: `cargo test -p cadence-aria workspace_engine`
- Expected: FAIL —— 现有实现硬编码 `Supervised`

## Step 9: 硬编码 `Supervised` 改读配置

`prompts.rs:154,177`（Author）、`review.rs` 多处（Reviewer）、`review_repair.rs:49`、`revision.rs:55`：把 `permission_mode: ProviderPermissionMode::Supervised` 改为读 `session.permission_modes.author`（Author 运行）或 `session.permission_modes.reviewer`（Reviewer 运行）。逐一确认每个构造点是 Author 还是 Reviewer 语境。

- [ ] Run: `cargo test -p cadence-aria workspace_engine`
- Expected: PASS

## Step 10: 写失败测试 -- Author 选 Pi 走 PiProvider + fail-fast

**测试模板依据：** recording adapter 照 `src/product/workspace_engine/tests/part_06.rs:449` 的 `RecordingStreamingProvider`（记录 `input.provider_type`、返回 `ProviderSession { events, commands }`）；registry 注入照 `provider_registry.rs:21-44` 的 `register`。

`src/product/workspace_engine/tests/` 加：

```rust
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// 记录 start 次数与收到的 provider_type / permission_mode
struct CountingProvider {
    starts: Arc<AtomicUsize>,
    seen: Arc<Mutex<Vec<(ProviderType, ProviderPermissionMode)>>>,
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
        self.seen
            .lock()
            .unwrap()
            .push((input.provider_type.clone(), input.permission_mode.clone()));
        if self.fail_on_start {
            return Err(ProviderAdapterError::execution_failed(
                None,
                String::new(),
                "pi start failed",
                0,
            ));
        }
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let output = complete_story_artifact("生成候选草稿。", "候选草稿可进入审核。");
            let _ = event_tx.send(ProviderEvent::TextDelta { content: output.clone() }).await;
            let _ = event_tx
                .send(ProviderEvent::Completed(ProviderCompletion::plain(output, Some("sess-1".to_string()))))
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
async fn author_run_with_pi_uses_pi_provider_in_auto_mode() {
    let pi_starts = Arc::new(AtomicUsize::new(0));
    let pi_seen = Arc::new(Mutex::new(Vec::new()));
    let claude_starts = Arc::new(AtomicUsize::new(0));

    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::Pi, Arc::new(CountingProvider {
        starts: pi_starts.clone(), seen: pi_seen.clone(), fail_on_start: false,
    }));
    registry.register(ProviderName::ClaudeCode, Arc::new(CountingProvider {
        starts: claude_starts.clone(), seen: Arc::new(Mutex::new(Vec::new())), fail_on_start: false,
    }));

    // author 选 Pi、permission_modes.author = Auto 的 session
    let engine = engine_with_registry(Arc::new(registry), session_with_author_pi());
    engine.run_author().await.expect("author run");

    assert_eq!(pi_starts.load(Ordering::SeqCst), 1, "Pi 应被调用一次");
    assert_eq!(claude_starts.load(Ordering::SeqCst), 0, "不应调用其他 provider");
    let seen = pi_seen.lock().unwrap();
    assert_eq!(seen[0].0, ProviderType::Pi);
    assert_eq!(seen[0].1, ProviderPermissionMode::Auto, "Pi 仅 Auto");
}

#[tokio::test]
async fn pi_start_failure_reports_without_switching_or_retrying() {
    let pi_starts = Arc::new(AtomicUsize::new(0));
    let claude_starts = Arc::new(AtomicUsize::new(0));

    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::Pi, Arc::new(CountingProvider {
        starts: pi_starts.clone(), seen: Arc::new(Mutex::new(Vec::new())), fail_on_start: true,
    }));
    registry.register(ProviderName::ClaudeCode, Arc::new(CountingProvider {
        starts: claude_starts.clone(), seen: Arc::new(Mutex::new(Vec::new())), fail_on_start: false,
    }));

    let engine = engine_with_registry(Arc::new(registry), session_with_author_pi());
    let result = engine.run_author().await;

    assert!(result.is_err(), "启动失败应报错");
    assert_eq!(pi_starts.load(Ordering::SeqCst), 1, "Pi 只启动一次，不重试");
    assert_eq!(claude_starts.load(Ordering::SeqCst), 0, "不切换到其他 provider");
}
```

注：`engine_with_registry(registry, session)`、`session_with_author_pi()`、`complete_story_artifact(...)` 参照 `src/product/workspace_engine/tests/part_06.rs` 现有 harness；`engine.run_author()` 用该文件里驱动 Author 运行的实际入口名替换。

- [ ] Run: `cargo test -p cadence-aria workspace_engine`
- Expected: FAIL -- 运行链路未接 Pi / fail-fast 未保证


## Step 11: 运行链路接 Pi + fail-fast 边界

普通 Workspace 运行经 registry 取 provider（Task 2 已注册 Pi）。确认 Author/Reviewer/返修运行选 Pi 时走 `PiProvider`。

**fail-fast 边界：** Pi 失败即终态，不重试。用 `codegraph explore "workspace_engine provider_drive artifact retry fresh restart 触发条件"` 确认现有 retry 分支（`provider_drive.rs:455-491`、`504-570`）：
- 同 provider 内部重试（重跑同一 provider 补 artifact）→ 保留（契约只禁跨 provider）。
- 换 provider 重跑 → Pi 必须跳过（fail-fast）。

测试断言 `pi_start_count == 1`、其他 provider `start_count == 0`。

- [ ] Run: `cargo test -p cadence-aria workspace_engine`
- Expected: PASS

## Step 12: 全量受影响测试 + Commit

- [ ] Run:

```bash
cargo test -p cadence-aria workspace
cargo test -p cadence-aria workspace_engine
git add src/product/models/workspace.rs src/product/workspace_engine/ src/product/lifecycle_store/inputs.rs src/product/lifecycle_store/workspace.rs src/web/workspace_ws_types/common.rs src/web/handlers/coding.rs src/cross_cutting/streaming_provider/mod.rs
git commit -m "feat(workspace): persist per-role permission mode in session record (default Auto) and support Pi with fail-fast"
```

---

## 完成检查（对应 tasks 3.1/3.2）

- [ ] 3.1：普通 Workspace 的 Author/Reviewer 持久化独立权限模式，默认 `Auto`。
- [ ] 3.2：普通 Workspace 的 Author/Reviewer/返修运行支持 Pi（Auto；失败直接报错，不做运行期降级）。
