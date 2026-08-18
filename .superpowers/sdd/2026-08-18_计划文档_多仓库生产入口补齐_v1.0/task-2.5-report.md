# Task 2.5 报告：前端登记向导与 registration API client

## 状态

已完成；实现提交 hash 以本 worktree 的 `git rev-parse HEAD` 为准。

## 完成内容

- 新增 `logical-codebase-registration` 前端 DTO 类型及 API client，覆盖 preflight、submit、GET、resume、cancel 五个后端入口；请求体逐字匹配 Rust DTO，项目/批次 ID 均 URL-encode，并沿用 `ApiRequestError` / `normalizeApiError` / `requestJson` 模式。
- 新增 `LogicalCodebaseRegistrationWizard`：聚合根与候选路径输入、预检、七类分类展示、eligible 自动进入确认集合、needs_attention 必须显式勾选、同步提交 loading/防重复、partial_failed 逐项结果与 resume、错误 message 展示。
- 在逻辑代码库面板增加“登记成员”入口；成功完成登记后刷新 workbench。
- 新增 API client、向导及 workbench 入口 Vitest 覆盖；扩展共享 workbench mock 以支持 registration 五端点。

## 红绿证据

- 红：先运行 `cd web && pnpm test -- LogicalCodebaseRegistrationWizard.test.tsx logical-codebase-registration.test.ts`，两个新增模块尚不存在，Vitest 报 import resolution failure。
- 绿：新增测试通过；向导测试覆盖七类展示、needs_attention 显式确认、partial_failed → resume；API 测试覆盖 DTO 请求体、编码路径和错误归一化。

## 验证

- `cd web && pnpm test -- LogicalCodebaseRegistrationWizard.test.tsx logical-codebase-registration.test.ts IssueLifecycleWorkbench.test.tsx`（通过，840 tests）
- `cd web && pnpm test -- IssueLifecycleWorkbench.test.tsx`（通过，840 tests）
- `cd web && pnpm tsc -b`（通过）
- `git diff --check`（通过）

## 自审

- 提交调用为同步 await，不引入前端 polling；提交和恢复期间按钮、关闭动作均禁用，`aria-busy` 与 spinner 文本可观察。
- `confirmed_paths` 只由 eligible 与明确勾选的 needs_attention 组成；其他五类只展示，不会进入提交集合。
- 共享 `IssueLifecycleWorkbench.test-utils.ts` 仅增加 registration mock 类型与路由分支，未改动既有生命周期 mock 语义。

## Concerns

- 现有 workbench 初次加载仍由既有并行请求负责；向导完成后调用既有 `refresh` 刷新成员与发布面板。前端不主动查询/轮询 registration batch，符合后端同步链路。
