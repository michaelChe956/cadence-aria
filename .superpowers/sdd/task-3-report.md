# Task 3 Report

## Implemented changes

- 后端新增 `CodingExecutionUnitDto`，并将 `attempt_scope`、`work_item_group_id`、`current_work_item_id`、`active_unit_id`、`units` 暴露到 HTTP snapshot 与 WS `coding_session_state`。
- WS snapshot 构造在 group attempt 下通过 `CodingAttemptStore::list_coding_units` 组装 unit 列表；单 Work Item attempt 返回空数组，保持兼容。
- 前端补齐 `CodingAttemptScope` / `CodingExecutionUnit` 类型，并在 `coding-workspace-store` 中恢复 `attemptScope`、`workItemGroupId`、`currentWorkItemId`、`activeUnitId`、`units`。
- 补充后端 WS snapshot 回归测试、前端 store 回归测试，并更新前端类型测试与现有 fixture 以匹配新合同。

## RED evidence

- `cargo test --locked --test it_web coding_ws_session_state_includes_group_units`
  - 失败：`assertion left == right failed`
  - 现象：`state["attempt_scope"]` 为 `Null`，预期 `"work_item_group"`
- `cd web && ./node_modules/.bin/vitest --run src/state/coding-workspace-store.test.ts`
  - 失败：`expected undefined to be 'work_item_group'`
  - 现象：store 未恢复 `attemptScope`

## GREEN commands/results

- `cargo test --locked --test it_web coding_ws_session_state_includes_group_units`
  - 结果：PASS
- `cd web && ./node_modules/.bin/vitest --run src/state/coding-workspace-store.test.ts src/api/types.test.ts`
  - 结果：PASS
- `cargo fmt --check`
  - 结果：PASS
- `cd web && ./node_modules/.bin/tsc --noEmit`
  - 结果：PASS

## Files changed

- `src/web/types.rs`
- `src/web/handlers/dto.rs`
- `src/web/handlers/mod.rs`
- `src/web/handlers/coding.rs`
- `src/web/coding_ws_handler/protocol.rs`
- `src/web/coding_ws_handler/state.rs`
- `tests/it_web/web_coding_ws_handler/part_01.rs`
- `web/src/api/types/coding.ts`
- `web/src/api/types.test.ts`
- `web/src/state/coding-workspace-store.ts`
- `web/src/state/coding-workspace-store.test.ts`
- `web/src/components/lifecycle/IssueLifecycleWorkbench.test-data.ts`

## Commit SHA

- `83d7f10`

## Concerns

- 为完成 Task 3 的实际 snapshot 构造，除 brief 点名文件外，额外修改了 `src/web/coding_ws_handler/state.rs`、`src/web/handlers/coding.rs`、`src/web/handlers/dto.rs`、`src/web/handlers/mod.rs`；这是现有 HTTP/WS snapshot 装配层的必要最小改动。
- 前端因 `CodingAttempt` 合同变更，需要同步调整一个现有 fixture 文件 `web/src/components/lifecycle/IssueLifecycleWorkbench.test-data.ts`，否则 `tsc` 不通过。

## Fix 1: HTTP snapshot coverage

### Implemented changes

- 在 `tests/it_web/web_coding_attempt_api/part_01.rs` 新增 `returns_group_coding_attempt_snapshot_with_units`，通过真实 HTTP `GET /api/coding-attempts/{attempt_id}` 回归验证 group snapshot。
- 断言覆盖 `attempt_scope`、`work_item_group_id`、`current_work_item_id`、`active_unit_id` 以及 `units` 长度和 `running` / `pending` 状态。
- 本次未修改生产代码；当前 HEAD 的 HTTP snapshot 行为已满足需求，此修复为测试覆盖补齐。

### Commands/results

- `cargo test --locked --test it_web returns_group_coding_attempt_snapshot_with_units`
  - 结果：PASS（新测试首次运行即通过，说明实现已存在）
- `cargo test --locked --test it_web coding_ws_session_state_includes_group_units`
  - 结果：PASS
- `cargo fmt --check`
  - 结果：PASS

### Files changed

- `tests/it_web/web_coding_attempt_api/part_01.rs`

### Commit SHA

- `158fda9`

### Concerns

- 无新增产品逻辑风险；本次是 test-only coverage fix。
