# Task 4.4 实现报告

## 状态
已完成。Commit hash 以最终交付回执为准。

## 实现摘要

- 新增 `AggregateIndexRebuildRegistry`，按 project 进行 in-process try-register，lease Drop 自动清理。
- 新增聚合索引 active/rebuild HTTP 端点；POST 通过 `spawn_blocking` 同步执行 rebuild，成功后读取 active 投影；并发请求返回 `409 aggregate_index_rebuild_in_progress`。
- GET active 对 Active/Stale/Degraded/Building/Superseded/Failed/无记录执行五态投影；Failed 有可读记录时返回 degraded，无可读记录时返回 missing，并保留 warning。
- 完善单仓 logical-codebase 负向 guard，active/rebuild 均返回 `409 logical_codebase_feature_disabled`。
- 初始化 Completed 尾步的 detached index build 失败仅记录 tracing，失败状态由 aggregate-index operation 持久化为 Failed，不改变初始化 Completed。

## 测试

通过：

- `cargo fmt --check`
- `cargo check --locked`
- `cargo clippy --all-targets --all-features --locked -- -D warnings`
- `cargo test --locked --lib aggregate_index_rebuild_registry`
- `cargo test --locked --test web_logical_codebase_entrypoints aggregate_index_`（6 passed；Fix round 1 后为 7 passed）
- `cargo test --locked --test web_logical_codebase_entrypoints multi_repo_blocks_legacy_mutation_projects_members_and_single_repo_rejects_existing_logical_route`

全量 `cargo test --locked` 编译通过，但现有 `it_web` 中 5 个 logical fixture 测试失败：fixture 的 checkout 目录不是实际 Git 仓库，freshness 读取时报 `aggregate_index_git_missing: git: provider command not found: git`；这些失败与本次端点代码无关，需后续由 fixture/测试环境修复。

## Fix round 1

- 补充 `aggregate_initialization_completion_eventually_exposes_active_index`：通过真实初始化 POST/GET 轮询确认初始化进入 Completed，再通过 active HTTP 端点确认 detached 首建最终投影为 `active`，并断言 revision/indexed_at。
- 补充 `aggregate_index_endpoints_expose_building_and_reject_concurrent_rebuilds`：测试 fixture 注入可控阻塞的 fake CodeGraph CLI，以真实 in-flight rebuild 暴露 `rebuilding`，并验证同 project 并发 POST 返回 `409 aggregate_index_rebuild_in_progress`，释放后首请求返回 active。
- 为集成测试增加仅用于注入 aggregate-index operation 的 state seam；初始化依赖的 coordinator/run registry 保持不变。

### Fix round 1 验证

- `cargo fmt --check`
- `cargo test --locked --test web_logical_codebase_entrypoints`
- `cargo test --locked --lib`
- `cargo clippy --all-targets --all-features --locked -- -D warnings`

## 变更文件

- `src/web/state/aggregate_index_rebuild_registry.rs`
- `src/web/state.rs`
- `src/web/handlers/aggregate_index.rs`
- `src/web/handlers/mod.rs`
- `src/web/app.rs`
- `src/web/error.rs`
- `src/web/handlers/aggregate_initialization/handlers.inc.rs`
- `src/product/logical_codebase/aggregate_index/store.rs`
- `tests/web_logical_codebase_entrypoints/index.rs`
- `tests/web_logical_codebase_entrypoints/guards.rs`
