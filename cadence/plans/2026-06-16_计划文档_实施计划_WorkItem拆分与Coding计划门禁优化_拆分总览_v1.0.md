# WorkItem 拆分与 Coding 计划门禁优化拆分总览 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement each detailed P plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking in detailed plans.

**Goal:** 将 `cadence/designs/2026-06-16_技术方案_WorkItem拆分与Coding计划门禁优化_v1.1.md` 拆成多个单 session 可完成、前后端分离、可独立验证且最终能合成同一 Issue 交付的实施计划。

**Architecture:** 先收敛 Work Item 活跃模型和调度基础，再实现拆分校验、生成流、共享 worktree、Coding 门禁、handoff 和前端展示。每个 Work Item 只拥有一个清晰上下文边界；存在依赖的后序计划必须消费前序交付摘要，互不依赖的计划只在写入范围可证明不冲突时允许并行。

**Tech Stack:** Rust 1.95.0、Cargo、Serde JSON、Axum、React、TypeScript、Zustand、Vitest、Playwright、OpenSpec、Superpowers。

---

## 计划大小控制规则

- 单个详细 plan 必须能在一个实现 session 内完成；目标是实现者在读取 plan、读代码、写测试、写实现、验证和提交时仍保留充足上下文。
- 单个详细 plan 的实现范围建议控制在 30k-50k tokens 等价上下文内；如果计划需要同时阅读大量旧实现、跨后端和前端、或需要 6 个以上核心源码文件协同改动，必须继续拆分。
- 详细 plan 不允许同时承载后端实现、前端实现和贯通测试；前端、后端、贯通/E2E 必须拆成不同计划。
- 后序详细 plan 必须包含“前置交付摘要”章节，明确依赖哪些已完成计划、需要读取哪些提交摘要、哪些接口已经稳定。
- 非依赖计划只有在写入范围互斥时才可并行；只要会修改同一文件、同一 store、同一 handler 或同一 UI 状态模块，就必须建立顺序依赖。
- 实现过程中如果发现当前 plan 实际超出单 session 范围，执行者必须停止扩大范围，先提交已完成的可验证子集，再产出下一份更小的计划。

## 当前前置状态

- 工作目录：`.worktrees/feat-b-0616`
- 当前分支：`feat-b-0616`
- 设计方案：`cadence/designs/2026-06-16_技术方案_WorkItem拆分与Coding计划门禁优化_v1.1.md`
- 设计评审：`cadence/designs-reviews/2026-06-16_设计评审_WorkItem拆分与Coding计划门禁优化_v1.0.md`
- 关键约束：
  - Work Item 状态、拆分计划、执行计划和 handoff 都是 Aria 内部数据，不写入目标项目代码库。
  - 跨端 Issue 必须强制拆分后端 Work Item 与前端 Work Item。
  - 用户可选择是否生成贯通测试或 E2E Work Item。
  - 同一 Issue 的多个 Work Item 使用同一个共享 worktree branch。
  - Work Item 之间只对真实依赖排序；并行项必须写入范围互斥。
  - Coding 执行前必须具备单 session 可控的输入包；超限时继续拆分或摘要化。

## 拆分原则

- 后端模型、后端生成流程、后端 Coding 门禁、前端 UI、贯通/E2E 分别成计划，不混写。
- 每个计划都使用 TDD：先写失败测试，再写最小实现，再跑定向验证。
- 每个计划都必须说明 OpenSpec、Superpowers、TDD 和验证命令要求。
- 每个计划必须只修改自己声明的写入范围；若实现时发现需要越界修改，先更新拆分总览或新增计划，不在当前计划内临时扩大范围。
- 依赖计划的开头必须提供前置交付摘要，避免后序 session 重新吞入前序完整上下文。
- 每个计划的验证链必须包含项目强制检查命令，至少包含 `cargo fmt --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings` 和 `cargo check --locked`，外加该计划的定向测试；不允许只跑 `fmt + check` 而省略 clippy（详见 `cadence/project-rules/build-test-commands.md`）。

### 写入范围共享与串行约束

多个后端计划共享同一批源码文件，必须按依赖顺序严格串行，禁止并行修改同一文件：

| 共享文件 | 涉及计划 |
|---|---|
| `src/product/models.rs` | P1、P2、P4 |
| `src/product/lifecycle_store.rs` | P3、P4、P5 |
| `src/web/handlers.rs` | P3、P5 |
| `src/product/coding_workspace_engine.rs` | P5、P6 |

因此 P3、P4、P5 三者都修改 `lifecycle_store.rs`，必须严格串行（P3 → P4 → P5），不得并行。只有写入范围可证明完全互斥的计划才允许并行准备。

## P1：后端活跃模型收敛与调度器迁移

**目标：** 把 Work Item 调度基础从孤立旧模型迁移到 `LifecycleWorkItemRecord`，删除死代码 `WorkItemRecord` 与 `WorkItemStore`，为后续 SplitValidator 和 Coding 门禁提供统一事实源。

**依赖：** 无。

**写入范围：**

- `src/product/models.rs`
- `src/product/worktree_scheduler.rs`
- `src/product/mod.rs`
- `src/product/work_item_store.rs`
- `tests/it_core/work_item_scheduler.rs`
- `tests/it_product.rs`
- `tests/it_product/product_work_item_models.rs`

**不做：**

- 不实现 `IssueWorkItemPlan` 持久化。
- 不实现 `WorkItemSplitValidator`。
- 不改 `generate_work_items`。
- 不改 Coding Workspace 启动门禁。
- 不改前端。

**验证：**

- `cargo test --locked --test it_core work_item_scheduler`
- `cargo test --locked --test it_product lifecycle_work_item_deserializes_legacy_json_with_split_defaults`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features --locked -- -D warnings`
- `cargo check --locked`

**详细计划文档：**

- `cadence/plans/2026-06-16_计划文档_实施计划_WorkItem拆分与Coding计划门禁优化_P1_后端模型收敛与调度器迁移_v1.0.md`

## P2：后端 IssueWorkItemPlan 与 SplitValidator

**目标：** 新增 Aria 内部 Issue 级拆分计划模型和纯函数校验器，校验 DAG、写入范围、跨端拆分、贯通测试选项、上下文预算代理指标和 traceability。

**依赖：** P1。

**前置交付摘要要求：** 读取 P1 提交摘要，确认 `LifecycleWorkItemRecord` 已具备 `depends_on`、`exclusive_write_scopes`、`forbidden_write_scopes`、`context_budget`、`kind` 和 execution plan/handoff 引用字段。

**写入范围：**

- `src/product/models.rs`
- `src/product/work_item_split_validator.rs`
- `src/product/mod.rs`
- `tests/it_product.rs`
- `tests/it_product/product_work_item_split_validator.rs`

**不做：**

- 不调用 provider 生成拆分计划。
- 不创建真实 Work Item。
- 不改前端。

**验证：**

- `cargo test --locked --test it_product work_item_split_validator`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features --locked -- -D warnings`
- `cargo check --locked`

## P3：后端 generate_work_items 多 Work Item 与 artifact 关联

**目标：** 将现有 `generate_work_items` 从单 Work Item 生成升级为 Issue Work Item Set 生成，保证每个 Work Item 都有自己的 workspace session 与 artifact versions。

**依赖：** P1、P2。

**前置交付摘要要求：** 总结 P2 的 `IssueWorkItemPlan` 字段、validator findings 结构和校验失败返回方式。

**写入范围：**

- `src/web/handlers.rs`
- `src/product/lifecycle_store.rs`
- `src/product/workspace_engine.rs`
- `src/web/workspace_context.rs`
- `tests/it_web/web_work_item_generation.rs` 或现有同类测试文件
- `tests/it_product/product_lifecycle_store.rs`

**不做：**

- 不实现 Issue 共享 worktree。
- 不实现 Coding 启动门禁。
- 不改前端 UI。

**验证：**

- `cargo test --locked --test it_web generate_work_items`
- `cargo test --locked --test it_product lifecycle_store`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features --locked -- -D warnings`
- `cargo check --locked`

## P4：后端 Issue 共享 worktree 数据与 Git 安全前缀

**目标：** 增加 Issue 级共享 worktree 记录与安全前缀参数化，让 `aria/issues/*` 和 `.worktrees/aria-issues/*` 可创建、使用和清理，同时兼容存量 `aria/work-items/*`。

**依赖：** P1。

**前置交付摘要要求：** 确认 P1 未改变现有 attempt worktree 行为，只提供 Work Item 事实源字段。

**写入范围：**

- `src/product/models.rs`
- `src/product/git_workspace_service.rs`
- `src/product/lifecycle_store.rs` 或新增共享 worktree store 文件
- `tests/it_product/product_git_workspace_service.rs`
- `tests/it_product/product_lifecycle_store.rs` 或新增共享 worktree store 测试

**不做：**

- 不让 Coding attempt 复用 Issue worktree。
- 不实现 active Work Item 串行锁。
- 不改前端。

**验证：**

- `cargo test --locked --test it_product git_workspace_service`
- `cargo test --locked --test it_product issue_shared_worktree`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features --locked -- -D warnings`
- `cargo check --locked`

## P5：后端 Coding 启动门禁与共享 worktree 复用

**目标：** Coding 启动前检查依赖完成、共享 worktree 准备、active Work Item 串行锁、写入范围和 handoff 可读性，并让同一 Issue 下 attempt 复用 Issue 共享 worktree。

**依赖：** P1、P3、P4。

**前置交付摘要要求：** 总结 P3 的 Work Item Set 创建行为和 P4 的 `IssueSharedWorktree` 安全前缀规则。

**写入范围：**

- `src/web/handlers.rs`
- `src/product/coding_workspace_engine.rs`
- `src/product/lifecycle_store.rs`
- `tests/it_web/web_coding_attempt_api.rs`
- `tests/it_product/product_coding_workspace_engine.rs`

**不做：**

- 不实现 `WorkItemExecutionPlan` provider run。
- 不实现 handoff provider run。
- 不改前端。

**验证：**

- `cargo test --locked --test it_web start_work_item_attempt`
- `cargo test --locked --test it_product shared_worktree`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features --locked -- -D warnings`
- `cargo check --locked`

## P6：后端 WorkItemExecutionPlan 与 Handoff Provider Run

**目标：** Coding 前生成内部 `WorkItemExecutionPlan`，默认展示但不阻塞；Work Item 完成后运行额外 provider handoff run，缺 handoff 不允许完成或解锁依赖项。

**依赖：** P1、P5。

**前置交付摘要要求：** 总结 P5 的 Coding 门禁输入包结构、active lock 释放时机和 completion commit 记录方式。

**写入范围：**

- `src/product/coding_models.rs`
- `src/product/coding_attempt_store.rs`
- `src/product/coding_workspace_engine.rs`
- `tests/it_product/product_coding_attempt_store.rs`
- `tests/it_product/product_coding_workspace_engine.rs`

**不做：**

- 不改前端 Prepare 展示。
- 不实现前端 DAG。
- 不做真实浏览器 E2E。

**验证：**

- `cargo test --locked --test it_product work_item_execution_plan`
- `cargo test --locked --test it_product work_item_handoff`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features --locked -- -D warnings`
- `cargo check --locked`

## P7：前端 Work Item 生成选项与 DAG 展示

**目标：** 前端提供生成选项，并在 Work Item 列展示 kind、依赖、写入范围、预算、等待原因、handoff 状态和贯通/E2E 标识。

**依赖：** P2、P3。

**前置交付摘要要求：** 总结 P3 暴露给前端的 Work Item Set、validator findings、用户选项和等待原因字段。

**写入范围：**

- `web/src/api/types.ts`
- `web/src/api/types.test.ts`
- `web/src/state/product-workbench-store.ts`
- `web/src/state/product-workbench-store.test.ts`
- `web/src/pages/ProductWorkbenchPage.tsx` 或现有 Work Item 列组件
- `web/src/pages/ProductWorkbenchPage.test.tsx`

**不做：**

- 不改后端。
- 不改 Coding Workspace Prepare UI。
- 不写 Playwright E2E。

**验证：**

- `pnpm test -- --run ProductWorkbenchPage`
- `pnpm test -- --run product-workbench-store`

## P8：前端 Coding Prepare 执行计划展示

**目标：** Coding Workspace Prepare 阶段展示 `WorkItemExecutionPlan`；默认非阻塞，开启确认门禁时要求用户确认或请求修改。

**依赖：** P6。

**前置交付摘要要求：** 总结 P6 的 execution plan API/WS 字段、确认状态和 change requested 行为。

**写入范围：**

- `web/src/api/types.ts`
- `web/src/state/coding-workspace-store.ts`
- `web/src/hooks/useCodingWorkspaceWs.ts`
- `web/src/pages/CodingWorkspacePage.tsx`
- `web/src/pages/CodingWorkspacePage.test.tsx`

**不做：**

- 不改后端。
- 不改 Product Workbench Work Item 列。
- 不写 Playwright E2E。

**验证：**

- `pnpm test -- --run CodingWorkspacePage`
- `pnpm test -- --run coding-workspace-store`

## P9：贯通测试与可选 E2E Work Item 验收

**目标：** 验证后端 Work Item、前端 Work Item、可选 Integration/E2E Work Item 的端到端关系：后端 handoff 被前端消费，Integration/E2E 等待前后端完成，用户跳过时记录风险但不阻塞。

**依赖：** P1-P8。

**前置交付摘要要求：** 总结 P3/P5/P6/P7/P8 的 API、UI 和状态机行为，只引用摘要与关键测试名，不重新吞入所有实现细节。

**写入范围：**

- `tests/it_web/*` 中专门的贯通测试文件
- `web/tests/e2e/*` 或现有 Playwright 测试目录
- 必要测试夹具文件

**不做：**

- 不改生产后端代码，除非测试暴露真实缺陷；若需要改生产代码，先新增修复计划。
- 不改生产前端代码，除非测试暴露真实缺陷；若需要改生产代码，先新增修复计划。

**验证：**

- `cargo test --locked --test it_web work_item_split_flow`
- `pnpm test -- --run work-item`
- 真实浏览器 E2E 命令按仓库当时已有 Playwright 规范执行。

## 推荐执行顺序

1. 执行 P1，完成活跃模型收敛和调度器迁移。
2. 执行 P2，完成纯后端 SplitValidator。
3. 执行 P3，接入 `generate_work_items` 多 Work Item 创建。
4. 执行 P4；P4 依赖 P1，但其写入范围与 P3、P5 共享 `src/product/lifecycle_store.rs`，因此必须在 P3 之后、P5 之前串行执行，不得与 P3 并行。
5. 执行 P5，让 Coding 启动真正受 Work Item DAG、共享 worktree 和 active lock 约束。
6. 执行 P6，加入 execution plan 与 handoff。
7. 执行 P7 和 P8；二者都改 `web/src/api/types.ts`，因此不能并行修改同一分支，建议先 P7 后 P8。
8. 最后执行 P9；P9 只做贯通/E2E 验收，发现生产缺陷时新建修复计划。

## 验收标准

- Design Spec 生成 Work Item 时不再只能生成单个大 Work Item。
- 跨端 Issue 中后端与前端 Work Item 被强制拆分。
- 纯后端或纯前端 Issue 不会被误要求生成另一端 Work Item。
- 用户可选择是否生成贯通测试或 E2E Work Item。
- Work Item 之间有 DAG；只有真实依赖才排序。
- 无依赖并行项的写入范围必须互斥，无法证明互斥时必须建立依赖或继续拆分。
- 每个 Work Item 的执行上下文受 30k-50k 等价预算代理指标约束。
- 同一 Issue 下多个 Work Item 使用同一个共享 branch/worktree。
- 同一 Issue 同一时刻只有一个 active Work Item 修改共享 worktree。
- 后序 Work Item 可以消费前序 handoff summary，不需要完整历史上下文。
- `WorkItemExecutionPlan` 默认展示但不阻塞；开启确认门禁时才阻塞。
- Work Item 状态、拆分计划、执行计划和 handoff 都只存 Aria 内部数据，不写入目标项目代码库。
- 后端、前端和贯通/E2E 各自有独立计划与验证结果。
