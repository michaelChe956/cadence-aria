# Task 4.x logical fixture repair 2

## 根因调查

`PlanningContextResolver::new` 使用真实的 `AggregateIndexFreshnessService`。`assess` 在 active aggregate index 的 membership revision 匹配后，会调用 `AggregateIndexSnapshotCollector::capture_included`，并对每个 manifest member 的唯一可用 main checkout 执行 `git rev-parse HEAD` 与 `git status --porcelain=v1`。因此仅播种 `revision: "abc123"`、但 checkout 目录不是 Git 仓的 fixture 会在 freshness assess 阶段失败为 `aggregate_index_git_missing`，而不是被 seeded index evidence 绕过。

## 修复

仅修改测试 fixture，不改变生产代码语义：

- `tests/it_web/web_lifecycle_api/part_02.rs` 的 `seed_logical_codebase` 创建真实成员 Git 仓、提交 fixture 源文件，并使用真实 `HEAD` 同时播种 checkout revision 与 aggregate index member snapshot；membership revision 使用 manifest 的实际 revision。
- `tests/it_web/pointer_publication/part_05.rs` 的 logical prompt fixture 同样创建带提交的真实成员 Git 仓，并使用真实 `HEAD` 播种 evidence。

这样 freshness assess 能验证 fixture 的 Git evidence 与 active index 一致，规划/逻辑读路径继续保持真实生产 freshness 行为。

## 验证

- `cargo fmt --check`：通过。
- `cargo test --locked --test it_web`：381 passed，12 ignored，0 failed。
- `cargo test --locked --test web_logical_codebase_entrypoints`：通过。
- `cargo test --locked --lib`：通过。
- 五个原回归测试均通过：4 个 lifecycle logical fixture 测试，以及 `pointer_publication_scenario_f_logical_context_injects_authority_reference`。

## 变更范围

无生产代码变更；无新增测试，仅修复既有测试 fixture 的真实 Git/evidence 预置。
