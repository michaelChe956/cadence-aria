# it_core large_file_guard 既有超长文件门禁失败记录

## 背景

在修复 Coding Workspace 加载 Final Compile Work Item 上下文期间，执行 `cargo test --locked` 时发现 `it_core` 中的 `large_file_guard::product_source_and_test_files_stay_under_line_limit` 稳定失败。

该失败不是本次 Coding Workspace prompt/context 修改引入的。失败列表中没有本次新增或修改的 `src/web/coding_ws_handler/*`、`npm/cli/*`、`tests/it_core/workspace_ws_integration/*` 文件。

## 失败信息

门禁要求产品源码与测试文件保持在 800 行以内，当前超限文件为：

- `src/product/work_item_split_engine/tests/part_01.rs`: 838 行
- `src/product/workspace_engine/provider_drive.rs`: 1045 行
- `src/product/workspace_engine/tests/part_01.rs`: 918 行
- `src/web/workspace_context/tests.rs`: 824 行
- `src/web/workspace_ws_types/tests.rs`: 814 行
- `web/src/state/workspace-ws-store.ts`: 842 行

## 当前验证结论

- 与本次变更直接相关的 Coding Workspace 测试已通过。
- 由 lifecycle summary 瘦身导致的 3 个 workspace websocket integration 旧断言已改为读取持久化完整 session，并通过 `cargo test --locked --test it_core workspace_ws_integration` 验证。
- `cli_adapter_baseline` 过滤集独立运行通过。
- 全量 `cargo test --locked` 仍会被上述 `large_file_guard` 稳定门禁阻断。

## 建议处理

将该问题作为单独重构任务处理，不混入当前 Coding Workspace 修复提交。

建议后续按文件拆分：

- 将超长测试文件拆成更多 `part_*.rs` 或子模块。
- 将 `src/product/workspace_engine/provider_drive.rs` 按 provider drive 阶段或职责拆分到子模块。
- 将 `web/src/state/workspace-ws-store.ts` 拆分出 reducer/action helpers 或 websocket message handlers。

完成后重新运行：

- `cargo test --locked --test it_core large_file_guard::product_source_and_test_files_stay_under_line_limit`
- `cargo test --locked`
