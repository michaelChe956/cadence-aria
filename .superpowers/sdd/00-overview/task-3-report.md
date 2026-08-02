# Task 3 Report — 普通 Workspace 权限持久化 + Pi 接入 + fail-fast

**Status:** DONE_WITH_CONCERNS
**Commit:** `f1bb216f` — feat(workspace): persist per-role permission mode (default Auto) and support Pi with fail-fast
**Base:** `26f24377`

> 注：实现由 implementer subagent 完成，但它在 commit/report 前遭遇 504 gateway timeout。
> 本报告由 controller 在核实全部产物后补写并提交，并修掉了 implementer 遗留的两个质量问题（见下）。

## 实现内容（四层 + 归一化）

**1. `ProviderPermissionMode` 加 serde derive** — `src/cross_cutting/streaming_provider/mod.rs`。原本只有 `Debug, Clone, PartialEq, Eq`，持久化必须加 `Serialize, Deserialize`。

**2. `WorkspaceRolePermissionModes`** — `src/product/models/workspace.rs:32`，`{ author, reviewer }`，`Default` 全 `Auto`。**与 Coding 的 `CodingProviderPermissionMode` 保持两套独立类型**（Decision 3），未合并未混用。

**3. 持久化实体 `WorkspaceSessionRecord`** — `workspace.rs:59` 加 `permission_modes`，带 `#[serde(default)]`，旧记录缺字段按 `Auto` 反序列化。

**4. 运行时 `WorkspaceSession`** — `types.rs:86` 加字段，`types.rs:119` `from_record` 映射。

**5. 传输 DTO `ProviderConfigSnapshot`** — `common.rs:117` 加字段（`#[serde(default)]`）。

**6. store 层** — `lifecycle_store/inputs.rs` 的 `CreateWorkspaceSessionInput`、`lifecycle_store/workspace.rs:241` 创建记录（用 `default()`）、`:427` 更新 API 签名带 `permission_modes`。

**7. 服务端 Pi→Auto 归一化** — `lifecycle.rs:624-629`：`start_generation` 锁定快照前，若 `author == Pi` 或 `reviewer == Some(Pi)` 就强制该角色 mode 为 `Auto`。前端过滤挡不住陈旧数据与直连 API/WS，服务端兜底。

## `ProviderConfigSnapshot` 构造点盘点

| 位置 | 权限模式来源 |
|---|---|
| `workspace_engine/lifecycle.rs:121` | `session.permission_modes.clone()` |
| `workspace_engine/lifecycle_recovery.rs:127` | `session.permission_modes.clone()` |
| `workspace_engine/plan_repair_recovery.rs:41` | `session.permission_modes.clone()` |
| `workspace_engine/plan_repair_recovery.rs:428` | `session.permission_modes.clone()` |
| `workspace_engine/plan_repair_transaction.rs:759` | `engine.session.permission_modes.clone()` |
| `workspace_engine/session_state.rs:220` | `session.permission_modes.clone()` |
| `workspace_engine/session_state/timeline.rs:417` | `self.session.permission_modes.clone()` |
| `web/handlers/coding.rs:446` | `session.permission_modes.clone()` |
| `lifecycle_store/workspace.rs:241`（创建记录，无既有 session） | `WorkspaceRolePermissionModes::default()` |

规则落实：**有 session 就从 session 复制；仅创建路径用 default**。未把 `CodingRolePermissionModes` 混入普通 Workspace 字段。

## 硬编码 `Supervised` → 读配置

| 位置 | 语境 | 改为 |
|---|---|---|
| `prompts.rs:154` | Author | `session.permission_modes.author.clone()` |
| `prompts.rs:177` | Author（work item split） | `session.permission_modes.author.clone()` |
| `prompts/review.rs:84,283,409,587` | Reviewer | `session.permission_modes.reviewer.clone()` |
| `prompts/review_repair.rs` | Reviewer | `session.permission_modes.reviewer.clone()` |
| `prompts/revision.rs` | Author（返修由 author 执行） | `session.permission_modes.author.clone()` |

复验：`grep ProviderPermissionMode::Supervised` 在这些文件的非测试代码中**已无残留**。

## 测试结果

`cargo test -p cadence-aria --lib` → **1441 passed / 0 failed**（Task 2 后为 1433，+8）

brief 要求的 7 个测试全部通过：
- `workspace_role_permission_modes_default_is_auto`
- `old_workspace_session_record_without_permission_modes_deserializes_to_auto`
- `new_session_defaults_permission_modes_to_auto`
- `start_generation_locks_selected_modes_into_store`
- `start_generation_normalizes_pi_role_to_auto_and_keeps_disabled_reviewer_mode`
- `author_run_with_pi_uses_pi_provider_in_auto_mode`
- `pi_start_failure_reports_without_switching_or_retrying`

**质量门禁**：`cargo check --all-targets` 无 error 无 warning；`cargo fmt --check` 通过；`cargo clippy --all-targets` 无 error。

## Controller 修掉的 implementer 遗留问题

1. **fmt 违规** — `coding_workspace_engine/tests/gate_rework.rs` 未格式化。已 `cargo fmt`。
2. **未使用 import 引发 lib warning** — `workspace_engine/mod.rs:38` 导入了 `WorkspaceRolePermissionModes` 但 lib 代码未用。移除后发现 `tests/part_31.rs:464,510` 两处依赖它的非限定名。已把那两处改为全限定路径 `crate::product::models::WorkspaceRolePermissionModes`（与仓库其余测试一致），再移除 import。最终 lib 编译零 warning。

## Concerns

1. **implementer 未提交且无报告**：504 timeout 中断在最后一步。产物经 controller 完整核实（四层实现、归一化、构造点盘点、硬编码清除、1441 测试、三道门禁）后提交。
2. **改动面 88 文件**：绝大多数是 `ProviderConfigSnapshot` / `WorkspaceSessionRecord` struct literal 加字段导致的机械修复（含大量测试 fixture）。核心逻辑改动集中在 model / engine / store / DTO 四层与 `lifecycle.rs` 的归一化。
3. **`revision.rs` 归为 Author 语境**：返修运行由 author provider 执行，故读 `permission_modes.author`。若产品语义认为返修应独立配置，需后续澄清 —— 但这超出本 change 契约范围。

---

# Task 3 Round 1 Fix Report — Pi Auto-only bypass closure

**Status:** DONE

## 修改内容与不可绕过边界

- 在 `src/product/workspace_engine/mappings.rs` 新增唯一的 Pi 权限归一化规则：Pi 一律返回 `ProviderPermissionMode::Auto`；Claude Code、Codex 和 Fake 则保留各自原有的配置值。`WorkspaceRolePermissionModes` 与 `CodingProviderPermissionMode` 仍是独立类型，未合并或交叉赋值。
- `controls.rs::set_provider` 在 `ProviderSelect` 改写 author/reviewer provider 时立即通过该规则归一化对应 role，并将 permission modes 同 provider 选择一起持久化。这会修正陈旧持久化数据。
- 所有 `StreamingProviderInput` 构造边界均通过同一规则重新归一化：普通 author、WorkItemPlan author、revision author、所有 reviewer 变体和 review-repair。这个“输入即将交给 adapter”的边界是最终不可绕过的服务端兜底：无论入口是 `StartGeneration`、遗留 `ProviderSelect -> UserMessage` WS 路由、恢复流程或未来内部调用，Pi 都不可能收到 `Supervised`。
- 保留并强化 `lifecycle.rs::start_generation` 的既有锁定前归一化：它现在同样调用该共享规则，仍在锁定快照及持久化之前强制 Pi 为 Auto；没有用构造边界替换该保护。
- 未修改 Claude Code 或 Codex 的 Supervised 行为。

## 覆盖测试

- `web::workspace_ws_handler::tests::provider_select_then_user_message_forces_pi_to_auto_from_stale_supervised_mode`：在 store 中预置 author=`Supervised`，通过真实 `ProviderSelect { author: Pi }` 后发 `UserMessage`，断言 Pi adapter 实收 `Auto`。
- `web::workspace_ws_handler::tests::pi_failures_do_not_start_registered_alternate_provider`：分别覆盖 Pi adapter 的 start error 与运行期 `ProviderEvent::Failed`；registry 同时注入 Claude Code alternate，并断言它从未启动。

## Red evidence（修复前）

先只添加绕过回归测试并执行：

```text
cargo test -p cadence-aria --lib provider_select_then_user_message_forces_pi_to_auto_from_stale_supervised_mode
... FAILED
left: Supervised
right: Auto
ProviderSelect followed by UserMessage must not send stale Supervised mode to Pi
```

失败说明未经过 `start_generation()` 的真实 WS 路径把陈旧 `Supervised` 直接写入了 Pi 的 `StreamingProviderInput`，与报告中的绕过链一致。

## Green evidence（修复后）

- 同一绕过测试通过。
- Pi start/runtime fail-fast alternate 测试通过。
- `cargo test -p cadence-aria --lib`：**1443 passed, 0 failed**。
- `cargo fmt --check`：通过。
- `cargo clippy -p cadence-aria --all-targets`：无 error；仅报告 `src/product/work_item_split_engine/types.rs` 既有、未修改的 `clippy::items_after_test_module` warning，因此本修复未引入 warning。`cargo clippy -p cadence-aria --lib` 无 warning。

## Finding 2 处理

原 `pi_start_failure_reports_without_switching_or_retrying` 的唯一 adapter harness 无法证明“不切换”，因此已更名为 `pi_start_failure_does_not_retry_selected_provider`，使名称严格对应它实际验证的“Pi 启动失败不重试同一 adapter”。另新增 WS handler 层的 `pi_failures_do_not_start_registered_alternate_provider`：它注册 Pi 和 Claude Code 两个真实候选 adapter，分别覆盖 Pi **启动失败**及运行期 `ProviderEvent::Failed`，并断言 Claude Code alternate 的启动计数均为零；这补足了不切换与运行期 failure 证据。
