# part_15/part_16 拆分报告

## 变更

- 将 `src/product/workspace_engine/tests/part_03/part_15.rs` 按主题拆分为：
  - `part_15.rs`：原有 legacy compile transaction characterization 基线与 recovery journal 测试/helper。
  - `part_16.rs`：3.2/3.3 输入抽取、publication identity/durable context 与 parity/recovery 测试/helper。
- 保持测试函数、helper、断言和测试内容原样移动；未修改测试行为。
- 在 `part_03.rs` 追加 `include!("part_03/part_16.rs");`。
- 拆分后行数：`part_15.rs` 501 行，`part_16.rs` 789 行，均低于 1200 行红线且留有余量。

## 内容守恒检查

使用 `ast-grep outline` 对拆分前 HEAD 与拆分后两个文件的符号集合进行比较，结果一致：拆分前后均为 28 个 struct/function 符号；13 个 `work_item_plan_initial_compile` 测试保持不变。

## 验证

在干净的临时 detached worktree 中，仅应用本次三个源码文件变更并补齐已存在的 `web/dist` 构建产物后执行：

- `cargo test --locked --lib work_item_plan_initial_compile -- --list`：通过，`13 tests, 0 benchmarks`。
- `cargo test --locked --lib workspace_engine::tests -- --list`：通过，`891 tests, 0 benchmarks`。
- `cargo test --locked --lib workspace_engine::tests`：通过，`890 passed; 0 failed; 1 ignored`。
- `cargo test --locked --test it_core large_file_guard`：通过，`1 passed`。
- `cargo fmt --check`：通过。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：通过。

原 worktree 存在其他并行 worker 的未提交修改；因此源码/编译验证在隔离 checkout 完成，避免把无关改动纳入本次提交。
