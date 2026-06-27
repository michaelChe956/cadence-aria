# Task 1 报告：Coding Attempt Scope 与 Unit Store

## 实现内容

- 在 `src/product/coding_models/execution.rs` 引入 `CodingAttemptScope`，并为 `CodingExecutionAttempt` 增加：
  - `scope`
  - `work_item_group_id`
  - `current_work_item_id`
  - `active_unit_id`
- 为 `CodingExecutionAttempt` 实现 legacy serde 兼容：
  - 旧数据缺少 `scope` 时默认反序列化为 `WorkItem`
  - 旧数据缺少 `current_work_item_id` 时回填为 `work_item_id`
- 确保旧 `create_attempt` 写入：
  - `scope = WorkItem`
  - `current_work_item_id = Some(work_item_id)`
- 新增 `src/product/coding_models/group.rs`：
  - `CodingExecutionUnitStatus`
  - `CodingExecutionUnit`
- 在 `src/product/coding_attempt_store/inputs.rs` 新增：
  - `CreateGroupCodingAttemptInput`
  - `CreateCodingExecutionUnitInput`
- 在 `src/product/coding_attempt_store/paths.rs` 新增 unit 路径：
  - `coding_units_root`
  - `coding_unit_path`
- 新增 `src/product/coding_attempt_store/group.rs` 并实现：
  - `create_group_attempt`
  - `create_coding_unit`
  - `list_coding_units`
  - `get_active_coding_unit`
  - `update_coding_unit_status`
- 在 `src/product/coding_attempt_store/tests.rs` 新增 Task 1 所需测试：
  - legacy attempt 兼容反序列化
  - group attempt 与 unit 存储/查询

## 测试命令和结果

1. RED：

```bash
cargo test --locked --lib coding_attempt_store
```

结果：失败，确认缺少以下接口/字段，符合 TDD 预期：

- `CodingAttemptScope`
- `CodingExecutionUnitStatus`
- `CreateGroupCodingAttemptInput`
- `CreateCodingExecutionUnitInput`
- `create_group_attempt`
- `create_coding_unit`
- `list_coding_units`
- `get_active_coding_unit`
- `CodingExecutionAttempt.scope/current_work_item_id/work_item_group_id`

2. GREEN：

```bash
cargo test --locked --lib coding_attempt_store
```

结果：通过，`4 passed; 0 failed`

3. 格式化后回归：

```bash
cargo fmt
cargo test --locked --lib coding_attempt_store
```

结果：通过，`4 passed; 0 failed`

## TDD RED/GREEN 证据

- RED 证据：第一次执行 `cargo test --locked --lib coding_attempt_store` 时，编译失败并明确报缺少 Task 1 所需新类型、字段与方法。
- GREEN 证据：完成实现后再次执行同一命令，4 个 `coding_attempt_store` 单测全部通过。

## 变更文件

- `src/product/coding_models/execution.rs`
- `src/product/coding_models/group.rs`
- `src/product/coding_models/mod.rs`
- `src/product/coding_attempt_store/attempt.rs`
- `src/product/coding_attempt_store/group.rs`
- `src/product/coding_attempt_store/inputs.rs`
- `src/product/coding_attempt_store/mod.rs`
- `src/product/coding_attempt_store/paths.rs`
- `src/product/coding_attempt_store/tests.rs`
- `src/product/coding_workspace_engine/tests.rs`
- `src/product/tester_agent_loop/tests.rs`
- `src/product/coding_evaluation_context/tests.rs`
- `src/web/coding_ws_handler/tests.rs`
- `src/web/test_controls/fixtures.rs`

说明：后 5 个测试文件仅做编译适配，补齐 `CodingExecutionAttempt` 新字段，未引入 Task 2+ 行为。

## 自检结论

- 已严格限定在 Task 1：仅实现后端模型与 store 基础。
- 未修改 HTTP/API/WS 协议面或前端逻辑。
- legacy attempt serde 兼容已覆盖测试。
- 旧 `create_attempt` 已按要求补齐 `scope` 与 `current_work_item_id`。
- group attempt 与 unit 的最小存储/查询链路已覆盖测试并通过。

## 疑虑

- `create_group_attempt` 的 `attempt_no` 当前沿用“按 `current_work_item_id` 统计”的最小实现；brief 未明确 group attempt 是否应改为按 `plan_id` 维度计数。
- `active_unit_id` 在 `create_coding_unit`/`update_coding_unit_status` 中做了最小同步，但目前没有额外单测覆盖该字段的持久化细节。
