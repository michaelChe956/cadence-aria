# Task 5.1 Wire 兼容缺陷修复报告

## 状态

- 已修复 `session_state` 出站 WebSocket 消息在空 `provider_start_ledger` 时省略字段的 wire 契约缺陷。
- 已完成定向与全量 Rust 门禁；本任务将以精确路径创建一个原子提交。

## 根因与修复

`WsOutMessage::SessionState.provider_start_ledger` 原先同时标注 `#[serde(default, skip_serializing_if = "Vec::is_empty")]`。这使空账本在序列化时被省略，而阶段 1 wire 契约要求该字段始终存在且为数组，首条 session_state 因而不兼容。

最小修复保留 `#[serde(default)]`，移除仅对空数组生效的 `skip_serializing_if`：

- 空账本现在序列化为 `"provider_start_ledger": []`；
- 非空账本仍按既有元素结构完整序列化；
- `default` 保留反序列化旧 payload 缺字段时的兼容能力；
- 该规则施加于共享 `SessionState` 出站 schema，Story、Design、Work Item 与 WorkItemPlan 均一致，不需按 workspace 类型分支。

## 变更文件

- `src/web/workspace_ws_types/out.rs`
  - 调整 `provider_start_ledger` 的 serde 注解，保证空数组也出站。
  - 增加空账本字段存在断言与非空账本序列化保持断言。
- `src/web/workspace_ws_types/tests.rs`
  - 在既有 session_state wire schema 测试中断言字段存在且值严格为 `[]`。
- `.superpowers/sdd/2026-08-27_计划文档_WorkItem阶段2单候选C′MVP_v1.0/task-5.1-wirefix-report.md`
  - 本报告。

## TDD 证据

1. 先在既有 `review_messages_and_session_state_serialize_as_contract` 中加入空账本必须为 `[]` 的断言。
2. 修复前运行 `cargo test --locked --lib review_messages_and_session_state_serialize_as_contract`：失败，实际值为 `None`，期望为 `Some(Array [])`。
3. 移除空 Vec 的跳过序列化注解后，同一测试通过。
4. 新增/强化 phase-5.1 出站 schema 覆盖：空账本字段存在、非空账本元素不变。

## 验证记录

| 命令 | 结果 | 摘要 |
| --- | --- | --- |
| `cargo test --locked --lib session_state -- --list` | 通过 | 列出 18 个 session_state 相关测试，包含本任务 3 个目标测试。 |
| `cargo test --locked --lib session_state_preserves_nonempty_provider_start_ledger` | 通过 | 1 passed；非空 ledger 序列化保持不变。 |
| `cargo test --locked --lib session_state_serializes_work_item_plan_durable_fields` | 通过 | 1 passed；WorkItemPlan 空 ledger 明确为 `[]`。 |
| `cargo test --locked --lib review_messages_and_session_state_serialize_as_contract` | 通过 | 1 passed；共享 session_state wire schema 明确字段存在且为 `[]`。 |
| `cargo fmt && cargo fmt --check` | 通过 | 格式化已应用且最终检查通过。 |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | 通过 | 无 warning。 |
| `cargo test --locked` | 通过 | 400 passed、0 failed、12 ignored；另有 44 integration tests 与 2 doc-tests 全部通过。 |

## 范围与残余风险

- 仅修改 WebSocket 出站 schema 及其测试；未改 session_state 投影、持久化模型、Provider 逻辑或前端状态实现。
- 已确认前端 `WorkspaceWsState.providerStartLedger` 是数组字段，字段稳定存在符合消费者契约。
- 无已知残余风险。

## 提交与暂存

- 仅精确暂存上述两个 Rust 文件与本报告；未使用 `git add -A` 或 `git add .`。
- 提交后将检查工作树与暂存区均为空。
