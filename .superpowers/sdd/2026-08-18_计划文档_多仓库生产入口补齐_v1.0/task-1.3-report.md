# Task 1.3 报告：生产 RepositoryStore 按项目构造

## 完成范围

将任务 brief 所列全部 18 个生产 `RepositoryStore::new` 构造点替换为按目标项目查询 `ProjectStore` 后调用 `RepositoryStore::for_project`。因此 `ProjectRecord.multi_repo` 决定 logical-codebase feature；Web 请求的项目查询错误使用 `product_store_api_error` 映射为 `project_not_found`。

默认 repository registration 依赖改为 `ProjectAwareRepositoryPersistence`：每次 `find_by_path` 和 `create_repository` 均按传入的 project id 查询项目并构造 store，未在应用启动时固化 feature。

新增 `web_product_api` 集成回归：创建 multi-repo 项目、通过生产 POST repository 路径注册真实 Git 仓库、GET repositories 验证结果及 logical-codebase manifest、POST issue 验证 Issue 路径，并验证不存在项目 GET repositories 返回 `404 project_not_found`。

## 18 个生产构造点逐点核对

| # | Brief 位置 | 处理结果 |
| --- | --- | --- |
| 1 | `src/web/handlers/lifecycle.rs:320` | 已替换；`issue_lifecycle` 在读取 Issue 前查询 project，并以该记录构造 store。 |
| 2 | `src/web/handlers/repository_registration.rs:203` | 已替换；默认依赖改为按每个 `project_id` 查询项目的 `ProjectAwareRepositoryPersistence`。 |
| 3 | `src/web/handlers/coding.rs:395` | 已替换；`resolve_work_item_repository` 查询项目，并以 `for_project` 解析 repository。 |
| 4 | `src/web/handlers/coding/group.rs:408` | 已替换；group logical repository 分支查询项目并使用 `for_project`。 |
| 5 | `src/web/handlers/coding/group.rs:422` | 已替换；legacy group repository 分支查询项目并使用 `for_project`。 |
| 6 | `src/web/handlers/support.rs:188` | 已替换；`find_repository` 查询项目并使用 `for_project`。 |
| 7 | `src/web/handlers/product_resources.rs:93` | 已替换；`list_repositories` 先查询项目，错误经 `product_store_api_error` 映射。 |
| 8 | `src/web/handlers/product_resources.rs:112` | 已替换；`delete_repository` 先查询项目，错误经 `product_store_api_error` 映射。 |
| 9 | `src/product/coding_attempt_repository.rs:40` | 已替换；按 `attempt.project_id` 查询项目后构造 store。 |
| 10 | `src/product/workspace_repository.rs:209` | 已替换；`resolve_selected_logical_repository` 按 `project_id` 查询项目后构造 store。 |
| 11 | `src/product/workspace_repository.rs:219` | 已替换；`resolve_legacy_physical_repository` 按 `project_id` 查询项目后构造 store。 |
| 12 | `src/product/compatibility_scan.rs:178` | 已替换；helper 接收 `ProjectRecord`，以其创建 store，未再传递独立 project id。 |
| 13 | `src/product/coding_evaluation_context/builder.rs:297` | 已替换；按 `project_id` 查询项目后构造 store。 |
| 14 | `src/product/coding_attempt_store/issue_delivery.rs:137` | 已替换；按显式 `project_id` 查询项目后解析 strict logical repository。 |
| 15 | `src/product/coding_attempt_store/target_snapshot.rs:46` | 已替换；按 `project_id` 查询项目，且保持现有 `map_resolve_error` 边界。 |
| 16 | `src/product/logical_codebase/production_policy_resolvers.rs:34` | 已替换；resolver 保存 paths、在每次请求中按 `request.project_id` 查询项目并构造 store。 |
| 17 | `src/product/logical_codebase/snapshot_validator.rs:60` | 已替换；按 attempt 项目查询 store，仍将失败闭合为 `Inconsistent`。 |
| 18 | `src/web/workspace_context/entity.rs:293` | 已替换；`repository_for` 按 `project_id` 查询项目后构造 store。 |

结论：18/18 已替换；没有生产构造点保留 `RepositoryStore::new`。

## `RepositoryStore::new` 命中分类

执行：`rg -n --glob '*.rs' 'RepositoryStore::new\(' src`

结果为 18 个保留命中，全部是 test fixture/test-control seed，均未替换：

- `src/web/workspace_ws_handler/tests.rs:400,529,662`
- `src/web/workspace_ws_handler/tests/gateway_start.rs:284`
- `src/web/workspace_ws_handler/tests/planning_resume.rs:481`
- `src/web/test_controls/fixtures.rs:53,293`
- `src/web/test_controls/plan_repair/seed.rs:86`
- `src/web/workspace_context/tests.rs:356`
- `src/web/workspace_context/tests/work_item_plan_context.rs:9`
- `src/web/workspace_context/tests/linked_context.rs:9,69,229`
- `src/web/coding_ws_handler/tests/runner_cleanup.rs:439`
- `src/web/coding_ws_handler/tests/plan_repair/support.rs:12`
- `src/product/work_item_revision_store/tests/initial_publication.rs:629`
- `src/product/workspace_engine/tests/part_09.rs:392`
- `src/product/logical_codebase/reference_scanner.rs:1015`（brief 所述原 `:1013`，因已有行变动显示为 `:1015`）

其中 `reference_scanner.rs` 的命中处于 `#[cfg(test)] mod tests`（模块起始于 :835）；已确认未替换。其余变更过的 fixture 均只增加对应的 legacy project seed，以满足生产层新增 project lookup，仍保留 `RepositoryStore::new` 的单仓 fixture 语义。

## 前一 worker 遗留改动盘点

接手时有 27 个已修改、未暂存的 task 相关文件（`+291/-42`）：15 个生产源码文件、11 个既有测试 fixture 文件及 1 个新增 `tests/it_web/web_product_api.rs` 回归测试。逐项 diff 审计后，18 个生产点均已完成；fixture 的 project seed 是新增 lookup 后维持既有单仓测试语义所必需；integration test 直接覆盖 multi-repo 和 missing-project 生产路径。未发现超出本 task 的功能性改动。

另有未跟踪 `.pi/subagents/` 及 `cadence/notes/` 评审材料；它们并非本 task 文件，未修改、未暂存、未提交。受影响的 `coding.rs` 与 `support.rs` 中有少量既有空行格式整理，无行为改变，`cargo fmt --check` 通过。

## 验证

| 命令 | 结果摘要 |
| --- | --- |
| `cargo fmt --check` | 通过，无输出。 |
| `cargo check --locked` | 通过，完成开发配置编译。 |
| `cargo test --locked --test it_web production_repository_paths_do_not_silently_disable_missing_or_multi_repo_projects` | 通过：1 passed，392 filtered。 |
| `cargo test --locked --lib repository_store` | 通过：77 passed。 |
| `cargo test --locked --lib lifecycle` | 通过：56 passed。 |
| `cargo test --locked --test it_web web_product_api` | 通过：5 passed。 |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | 通过，无 warning。 |
| `cargo test --locked` | 未全绿：`it_provider::provider_error_routes::parse_error_timeout_and_incompatible_output_have_stable_routes` 失败；该二进制 53/54 通过，断言期望 `ProviderTimeout`、实际 `ProviderExecutionFailed`。失败位于未变更的 `tests/it_provider/provider_error_routes.rs:47`，与本任务的 RepositoryStore/ProjectStore 路径无交集；按协调决定未调查或修改。 |
| `cargo test --locked --test it_core` | 通过：148 passed。 |

所有 cargo 命令均未使用 `-j`。

## 自审

- 18 个 brief 构造点均转为 `for_project`，并在可见 Web 边界将 project lookup 错误映射为 API 错误。
- `repository_registration` 不再把 disabled feature 固化为 default app dependency，而是每个 project 操作动态查项目。
- `rg` 证明 `src` 中没有生产 `RepositoryStore::new`；保留 18 个均为 fixture/test-control，尤其确认 `reference_scanner.rs` 的 `#[cfg(test)]` fixture 未改。
- diff 通过 `git diff --check`，格式、编译、lint、相关单测和 `it_core` 均通过。

## Commit

`refactor(repository): 生产路径按 project 构造 store`

## Concerns

唯一 concern 是上述 `cargo test --locked` 的既有或环境相关 `it_provider` 单测失败；其余本任务门禁均通过。该失败未被纳入本 task commit，需由后续独立核实。
