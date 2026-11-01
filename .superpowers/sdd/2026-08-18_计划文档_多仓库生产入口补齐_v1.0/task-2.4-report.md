# Task 2.4 报告：登记查询、resume、cancel 全链

## 状态

已完成；实现提交 hash 以本 worktree 的 `git rev-parse HEAD` 为准。

## 完成内容

- 新增 GET `/api/projects/{project_id}/logical-codebase/registrations/{batch_id}`，先执行多仓项目 guard 与批次 ID 校验，再同步读取批次，未知批次稳定映射为 `registration_batch_not_found` 404；响应投影逐项状态和 `failure_reason`。
- 新增 POST `.../{batch_id}/resume`，先执行多仓 guard 与 ID 校验，再直接同步调用 `resume_batch` 并投影最终 items，不创建异步 job。
- 新增 POST `.../{batch_id}/cancel`，先读取当前批次并在 handler 显式严格限制 `Queued|PartialFailed`；Completed/Cancelled/Running 均返回 `registration_batch_not_cancelable` 409，不依赖 store 静默 no-op。
- 扩展批次 DTO 状态投影，完整覆盖 queued/running/partial_failed/completed/cancelled；注册稳定错误码补齐 HTTP 404/409 映射。
- HTTP 测试覆盖真实 routes 的 preflight → synchronous submit → GET → resume → terminal cancel rejection，并覆盖未知批次 404 与 failure_reason 空值投影。

## 红绿证据

- 红：先运行 `cargo test --locked --test web_logical_codebase_entrypoints registration_http_chain_queries_and_rejects_terminal_cancel`；当时新增链路因 route 缺失返回空响应，测试在 JSON 解析处失败（EOF）。
- 绿：最终定向链路测试和 registration 测试组均通过（见验收报告）。

## 验证

- `cargo fmt --check`
- `cargo test --locked --test web_logical_codebase_entrypoints registration_http_chain_queries_resumes_and_rejects_terminal_cancel`
- `cargo test --locked --test web_logical_codebase_entrypoints registration_`（11 passed）
- `cargo check --locked`
- `git diff --check`

## 自审

- 三个 handler 均在 coordinator/store 操作前执行 multi-repo guard 与 ID validation。
- cancel handler 显式加载当前 batch 并匹配允许状态，Completed/Cancelled 不会调用 cancel store。
- 所有 `.inc.rs` 文件均小于 1200 行，未使用 `-j`。

## Concerns

- 由于 HTTP submit handler 当前同步完成，外部生产路径通常只会观察到 Completed 或 PartialFailed；Queued/Running 仍保留 DTO 投影并由 coordinator/store 生命周期支持。cancel 的预读取与实际取消之间存在极窄并发窗口；handler 对 store 返回值再次校验，避免把非 cancelled 状态报告为成功，但未在本 Task 范围内重构 coordinator/store 的跨调用原子 API。
