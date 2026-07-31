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
| `web/handlers/coding.rs:442-456,494-508` | 按来源（Coding 域用 `CodingRolePermissionModes`，注意这是普通 Workspace 的 DTO，确认此处是普通还是 Coding） |

`src/web/workspace_ws_types/common.rs:112` `ProviderConfigSnapshot` 加：

```rust
    #[serde(default)]
    pub permission_modes: WorkspaceRolePermissionModes,
```

每个构造点从对应 session 复制 `permission_modes`。

- [ ] Run: `cargo test -p cadence-aria workspace_engine`
- Expected: 编译通过，timeline snapshot 测试断言 `permission_modes` 与 session 一致

## Step 7: `start_generation()` 把 wire 权限模式锁定到 session + store

`src/product/workspace_engine/lifecycle.rs:617-649`：`start_generation()` 从 wire `ProviderConfigSnapshot` 读 `permission_modes`，写入 `WorkspaceSession` 与 store（author/reviewer/rounds/modes 一起持久化）。若 reviewer 关闭（`reviewer: None`），明确 reviewer mode 语义（保留或归零），并测试。

- [ ] Run: `cargo test -p cadence-aria workspace_engine`
- Expected: PASS

## Step 8: 写失败测试 —— Author 运行读 session 权限模式

`src/product/workspace_engine/tests/` 加：构造 `permission_modes.author = Auto` 的 session，调 Author 运行的 `build_streaming_input`（`prompts.rs`），断言返回的 `StreamingProviderInput.permission_mode == Auto`（非硬编码 Supervised）。

- [ ] Run: `cargo test -p cadence-aria workspace_engine`
- Expected: FAIL —— 现有实现硬编码 `Supervised`

## Step 9: 硬编码 `Supervised` 改读配置

`prompts.rs:154,177`（Author）、`review.rs` 多处（Reviewer）、`review_repair.rs:49`、`revision.rs:55`：把 `permission_mode: ProviderPermissionMode::Supervised` 改为读 `session.permission_modes.author`（Author 运行）或 `session.permission_modes.reviewer`（Reviewer 运行）。逐一确认每个构造点是 Author 还是 Reviewer 语境。

- [ ] Run: `cargo test -p cadence-aria workspace_engine`
- Expected: PASS

## Step 10: 写失败测试 —— Author 选 Pi 走 PiProvider + fail-fast

`src/product/workspace_engine/tests/` 加（用 recording `StreamingProviderAdapter`，参照注入点 `provider_registry.rs:21-44`）：

```rust
#[tokio::test]
async fn author_run_with_pi_uses_pi_provider() {
    // 注册 recording Pi provider；构造 author 选 Pi 的 session
    // 跑 Author 运行，断言 PiProvider.start 被调一次
}

#[tokio::test]
async fn pi_start_failure_reports_failure_without_switching_or_retrying() {
    // recording Pi provider 的 start() 返回 Err（启动失败）
    // 断言：运行呈失败状态、pi start_count == 1、其他 provider start_count == 0
}
```

- [ ] Run: `cargo test -p cadence-aria workspace_engine`
- Expected: FAIL —— 运行链路未接 Pi / fail-fast 未保证

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
git add src/product/models/workspace.rs src/product/workspace_engine/ src/web/workspace_ws_types/common.rs src/cross_cutting/streaming_provider/mod.rs
git commit -m "feat(workspace): persist per-role permission mode in session record (default Auto) and support Pi with fail-fast"
```

---

## 完成检查（对应 tasks 3.1/3.2）

- [ ] 3.1：普通 Workspace 的 Author/Reviewer 持久化独立权限模式，默认 `Auto`。
- [ ] 3.2：普通 Workspace 的 Author/Reviewer/返修运行支持 Pi（Auto；失败直接报错，不做运行期降级）。
