# Task 4.3 实施报告

## 状态
已完成。

## 实现摘要
- 将 `PlanningContextResolver` 现有测试移至 `planning_context_resolver_tests.inc.rs`，主模块通过 `include!` 引入；两文件行数分别为 571 与 698，均不超过 1200。
- 新增 `build_with_fresh_index` 与 `resume_with_fresh_index` 异步入口；freshness assess/sync 在 `spawn_blocking` 中执行，JoinError 映射为 `ProductStoreError::Io`。
- 仅对 `Stale` 执行同步；`Degraded` 保留 last-known-good 并将 `aggregate index warning: ...` 注入 inventory/context 文本。
- 生命周期 handler、workspace context builder、workspace websocket resume 入口改用异步 fresh resolver；相关测试适配 await。
- 保留同步 `build`/`resume` 供不触发索引的 legacy 单元测试，并新增 stale/degraded 回归测试。

## 验证
- `cargo fmt --check`：通过
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：通过
- `cargo check --locked`：通过
- `cargo test --locked --lib stale_planning_read_syncs`：通过（1）
- `cargo test --locked --lib planning_resume`：通过（8）
- `cargo test --locked --lib planning_context_resolver`：通过（8）
- `cargo test --locked --test it_web fifty_member_planning_budget_injection_truncates_and_keeps_target`：通过（1）
- `cargo test --locked --test web_logical_codebase_entrypoints stale_`：通过（无匹配测试，0）

## Commit
`f152ea2` — `feat(planning): 读时同步 stale 聚合索引`

## Concerns
- `web_logical_codebase_entrypoints stale_` 当前无匹配测试；命令成功但运行 0 个测试。
- 当前工作树中存在任务开始前已有的 `.pi/subagents/`、`cadence/notes/` 未跟踪文件，未纳入本次提交。
