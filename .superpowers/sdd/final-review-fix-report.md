# 整分支最终评审 Important 修复报告

日期：2026-07-16

工作树：`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0715`

分支：`feat-b-0715`

## 结论

两项 Important 已完成严格 TDD 修复：

1. legacy 唯一 finder 不再信任路径内 JSON 的内嵌身份；扫描阶段仅记录路径 scope，唯一命中后调用 `get_attempt(project_id, issue_id, requested_attempt_id)` 复用 `IdentityMismatch` 校验。
2. execution-plan confirm/change 与 delete 的异步结果均使用完整 address key、request generation、mounted guard 隔离；A 请求在切换到 B 后不会更新 B store/error/loading，也不会触发 B 导航。

未修改真实 `.aria`；所有后端损坏数据回归均使用 `tempdir`。未调用业务 API、浏览器或停止开发服务。未 push。

## 修改文件

### 生产代码

- `src/product/coding_attempt_store/mod.rs`
  - finder 扫描只记录 `(project_id, issue_id)`。
  - 唯一命中后通过 scoped `get_attempt` 读取并校验 requested ID 与内嵌 project/issue/attempt ID。
- `web/src/pages/CodingWorkspaceReports.tsx`
  - confirm/change 捕获完整 route address。
  - 使用完整 address key、request generation、mounted guard。
  - resolve 写全局 store 前额外验证 store full-match。
  - stale reject/finally 不写当前地址 error/busy；地址切换重置旧地址输入/loading。
- `web/src/pages/CodingWorkspacePage.tsx`
  - delete 保持 route address + 起始 full-match gate。
  - 使用完整 address key、delete generation、mounted guard。
  - stale resolve 不调用 `onBack()`；stale reject/finally 不写 B error/busy。
  - 地址切换清除旧地址 delete/plan error 与 loading。

### 回归测试

- `tests/it_product/product_coding_attempt_store.rs`
- `tests/it_product/product_coding_attempt_store/part_04.rs`
- `tests/it_web/web_coding_attempt_api/part_06.rs`
- `tests/it_web/web_coding_ws_handler/part_11.rs`
- `web/src/pages/CodingWorkspacePage.execution-plan.test.tsx`
- `web/src/pages/CodingWorkspacePage.delete-race.test.tsx`
- `web/src/pages/CodingWorkspacePage.test-utils.ts`

## TDD：RED

### 后端 RED

无效命令记录：

- `cargo test --locked --test it_product global_lookup_rejects_unique_legacy_path_identity_mismatch -- --exact`
  - 结果：exit 0，但运行 0 个测试；集成测试完整名称包含模块前缀，因此此结果不计作 RED 证据。

有效 RED：

- `cargo test --locked --test it_product global_lookup_rejects_unique_legacy_path_identity_mismatch`
  - 结果：exit 101，1 failed。
  - 预期失败：唯一 alias 路径内嵌 B/Y 记录时，`get_attempt_by_id` 没有返回 `IdentityMismatch`。
- `cargo test --locked --test it_web legacy_coding_attempt_api_reports_scope_mismatch_for_unique_corrupt_alias`
  - 结果：exit 101，实际 HTTP 200，期望 409。
  - 预期失败：legacy GET 信任内嵌 B/Y 并返回成功。
- `cargo test --locked --test it_web legacy_delete_rejects_corrupt_alias_and_preserves_real_attempt`
  - 结果：exit 101，实际 HTTP 204，期望 409。
  - 预期失败：legacy DELETE 确实进入真实 B/Y 删除链，而非拒绝损坏 alias。
- `cargo test --locked --test it_web legacy_coding_ws_reports_scope_mismatch_for_unique_corrupt_alias`
  - 结果：exit 101。
  - 预期失败：实际收到 B/Y `CodingSessionState`，而非 `coding_attempt_scope_mismatch`。

合法历史 ID 基线：

- `cargo test --locked --test it_web legacy_coding_attempt_api_loads_unique_valid_attempt`
  - 结果：exit 0，1 passed。
  - 说明：修复前已确认合法唯一 legacy GET 语义存在，修复后继续保留。

### 前端 RED

- `cd web && pnpm test CodingWorkspacePage.execution-plan.test.tsx`
  - 结果：exit 1，13 tests 中 5 failed。
  - stale confirm resolve 覆盖 B plan。
  - stale confirm reject 在 B 显示 A error。
  - stale change resolve 覆盖 B plan。
  - stale change reject 在 B 显示 A error。
  - A busy 泄漏到 B，B 当前 confirm 按钮无法启用。
- `cd web && pnpm test CodingWorkspacePage.test.tsx`
  - 结果：exit 1，16 tests 中 3 failed。
  - stale delete resolve 调用了 A `onBack()`。
  - stale delete reject 在 B 显示 A error。
  - A delete busy 泄漏到 B，B 当前 delete 无法开始。

前端地址切换测试刻意保持相同 `attemptId`，只改变 `projectId`/`issueId`，用于证明实现没有退化为裸 attempt ID 比较。

## TDD：GREEN 与定向验证

### 后端最小 GREEN

- `cargo test --locked --test it_product global_lookup_rejects_unique_legacy_path_identity_mismatch`
  - 1 passed。
- `cargo test --locked --test it_web legacy_coding_attempt_api_`
  - 2 passed：合法 legacy GET 与损坏 alias 409。
- `cargo test --locked --test it_web legacy_delete_rejects_corrupt_alias_and_preserves_real_attempt`
  - 1 passed；真实 attempt 保持完整，alias 文件仍存在。
- `cargo test --locked --test it_web legacy_coding_ws_reports_scope_mismatch_for_unique_corrupt_alias`
  - 1 passed。

### 后端受影响套件

- `cargo test --locked --test it_product product_coding_attempt_store::`
  - 30 passed，0 failed。
- `cargo test --locked --test it_web web_coding_attempt_api::`
  - 32 passed，0 failed。
- `cargo test --locked --test it_web web_coding_ws_handler::`
  - 47 passed，0 failed。

### 前端最小 GREEN

- `cd web && pnpm test CodingWorkspacePage.execution-plan.test.tsx`
  - 13 passed，0 failed。
- `cd web && pnpm test CodingWorkspacePage.test.tsx`
  - 拆分前 16 passed，0 failed。
- 测试拆分后：
  - `cd web && pnpm test src/pages/CodingWorkspacePage.test.tsx src/pages/CodingWorkspacePage.delete-race.test.tsx src/pages/CodingWorkspacePage.execution-plan.test.tsx`
  - 3 files、29 passed，0 failed。

### Task 4/5 与 Page/Reports 相关定向回归

命令：

`cd web && pnpm test src/pages/CodingWorkspacePage.execution-plan.test.tsx src/pages/CodingWorkspacePage.gates.test.tsx src/pages/CodingWorkspacePage.reports.test.tsx src/pages/CodingWorkspacePage.test.tsx src/pages/LegacyCodingWorkspaceRedirect.test.tsx src/hooks/useCodingWorkspaceWs.test.tsx src/hooks/useCodingWorkspaceWs.actions.test.tsx src/state/coding-workspace-store.test.ts src/api/coding-attempts.test.ts src/api/client.test.ts src/router.test.tsx src/components/lifecycle/IssueLifecycleWorkbench.single-coding.test.tsx src/components/lifecycle/IssueLifecycleWorkbench.drawer.test.tsx`

结果：13 files、133 passed，0 failed。

## 全量门禁

### Rust

- `cargo fmt --check`
  - 最终 fresh 运行 exit 0。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`
  - 最终 fresh 运行 exit 0，无 warning。
- `cargo check --locked`
  - 最终 fresh 运行 exit 0。
- 首次 `cargo test --locked`
  - 行为测试通过，但 `large_file_guard` 失败：
    - `tests/it_product/product_coding_attempt_store/part_01.rs` 819 行。
    - `web/src/pages/CodingWorkspacePage.test.tsx` 825 行。
  - 根因：新增回归测试超过仓库单文件 800 行门禁。
- 测试文件拆分：
  - Rust：`part_01.rs` 780 行，新 `part_04.rs` 38 行。
  - Web：`CodingWorkspacePage.test.tsx` 694 行，新 `CodingWorkspacePage.delete-race.test.tsx` 152 行。
- `cargo test --locked --test it_core large_file_guard::product_source_and_test_files_stay_under_line_limit -- --exact`
  - 1 passed。
- 最终 `cargo test --locked`
  - exit 0，所有 target 0 failed。
  - 其中 lib 683 passed；it_core 143 passed；web 大型集成套件 246 passed、12 ignored；doc tests 0 failed。

### 前端

- `cd web && pnpm test`
  - 73 files、617 passed，0 failed。
- `cd web && pnpm tsc -b`
  - exit 0。
- `cd web && pnpm build`
  - exit 0，1773 modules transformed，构建完成。
  - 存在既有 Vite 提示：一个 minified chunk 大于 500 kB；不影响构建，本任务未扩大依赖或处理 bundle 拆分。

### 工作树与差异

- `git diff --check`
  - exit 0。
- `wc -l` 行数门禁复核
  - Rust：780 / 38。
  - Web：694 / 152。
- 提交前将执行 staged diff 与 `git status` 复核；提交后要求 tracked worktree clean。

## 自审

- finder 扫描不反序列化任何候选记录，因此多路径存在时优先保持 `Ambiguous`，不会因首个损坏 JSON 提前返回其他错误。
- 零匹配仍返回 `NotFound`。
- 唯一合法 legacy 路径仍通过 `get_attempt` 返回成功。
- 唯一损坏路径通过同一 scoped identity 校验返回 `IdentityMismatch`，REST/WS 映射保持现有 `coding_attempt_scope_mismatch`。
- DELETE 回归使用真实可删除 attempt 与损坏 alias，RED 实际返回 204，证明测试覆盖了真正破坏性 handler；GREEN 断言真实记录仍完整。
- confirm/change/delete 都捕获 route address 的完整三元组；测试使用相同 attempt ID 跨 project/issue 切换。
- generation 防止同地址较旧请求的 resolve/reject/finally 覆盖新请求；mounted guard 防止卸载后写状态。
- execution-plan resolve 写 Zustand 前额外检查 store full-match。
- delete 在发请求前继续使用既有 store full-match gate，API 目标只取捕获的 route address。
- stale finally 不解除 B 当前 loading；B 当前请求有正向完成回归。
- 未引入依赖、未修改公共 API、未改真实 `.aria`。

## 顾虑

- `pnpm build` 仍报告既有大 chunk warning；与本修复无直接关系。
- 报告文件已被仓库跟踪，将随本次 fix commit 一并保留。
- 无其他已知顾虑。
