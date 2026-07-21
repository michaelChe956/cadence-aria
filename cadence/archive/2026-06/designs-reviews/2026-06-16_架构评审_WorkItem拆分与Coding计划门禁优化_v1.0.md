# 架构 Review 报告：WorkItem 拆分与 Coding 计划门禁优化

**审查对象：** `.worktrees/feat-b-0616` 分支上的拆分总览 + P1-P9 实施计划 + 设计评审 v1.0  
**审查人：** Kimi Code CLI  
**审查日期：** 2026-06-16  
**分支 HEAD：** `ae8c2d0 review plan`  

---

## 1. 审查范围与方法

- **计划文档**：拆分总览、`2026-06-16_实施计划_WorkItem拆分与Coding计划门禁优化_P1-P9_v1.1`、设计评审 v1.0。
- **后端源码核对**：
  - `src/product/models.rs`：Work Item 活/死模型。
  - `src/product/lifecycle_store.rs`：`CreateWorkItemInput`、work item CRUD。
  - `src/product/worktree_scheduler.rs`：调度器现状。
  - `src/product/git_workspace_service.rs`：安全前缀、branch/worktree 创建。
  - `src/product/coding_workspace_engine.rs`：`handle_final_confirm`、`handle_abort`、`execute_worktree_prepare`、`complete_attempt_after_final_rework`。
  - `src/product/coding_attempt_store.rs`：attempt 数据结构。
  - `src/cross_cutting/provider_adapter.rs`：`ProviderAdapter` trait 及实现。
  - `src/cross_cutting/worktree.rs`：`validate_write_path`、`scopes_may_overlap`。
  - `src/web/handlers.rs`、`src/web/state.rs`、`src/web/types.rs`、`src/web/coding_ws_handler.rs`。
- **前端源码核对**：
  - `web/src/api/types.ts`、`web/src/api/client.ts`。
  - `web/src/state/lifecycle-workbench-store.ts`、`web/src/state/coding-workspace-store.ts`。
  - `web/src/components/lifecycle/LifecycleCard.tsx`、`LifecycleCardDrawer.tsx`、`IssueLifecycleWorkbench.tsx`。
  - `web/src/pages/CodingWorkspacePage.tsx`、`web/src/hooks/useCodingWorkspaceWs.ts`。
  - 前端现有 helper 与测试模式（本次评审不涉及浏览器 E2E）。
- **基线验证**：
  - `cargo check --locked` ✅
  - `cargo test --locked --lib --quiet`：242 passed ✅
  - `cd web && pnpm exec tsc -b` ✅

---

## 2. 当前分支状态

- 分支目前**只有计划/设计文档提交**，未改动任何生产代码。
- 代码基线干净且可编译，单元测试全绿。
- 设计方案总体方向正确：先收敛 `LifecycleWorkItemRecord`、再引入 `IssueWorkItemPlan`/`RepositoryProfile`/`VerificationPlan`、再做多 Work Item 生成、最后切换到 Issue 共享 worktree 并加 execution-plan/handoff/diff-scope/clean gate。

---

## 3. 方案总体评价

| 维度 | 评价 |
|------|------|
| 拆分粒度 | 合理。P1-P9 依次依赖，串行路径清晰，避免了大规模并发修改同一文件。 |
| TDD 节奏 | 每个 P 都有「先写失败测试 → 实现 → 通过」的循环，符合仓库规范。 |
| 数据模型 | 把 `WorkItemExecutionPlan`、`WorkItemHandoff`、`IssueSharedWorktree`、`IssueWorkItemPlan` 都限定为 Aria 内部数据，符合设计约束。 |
| 风险 | **存在若干真实阻塞/实现缺口**，需要在进入编码前补计划补丁或澄清，否则 P5/P6 会出现锁泄漏、API 绕过、非幂等 worktree 等回归。 |

---

## 4. 关键源码现状核对摘要

- `WorkItemRecord` / `WorkItemStore` 仍是死代码，`worktree_scheduler.rs` 与 `tests/it_core/work_item_scheduler.rs` 仍基于旧模型；P1 删除并迁移合理。
- `LifecycleWorkItemRecord` 是活模型，`CreateWorkItemInput` 目前只有 6 个字段；P1 将扩展 14+ 字段。
- `git_workspace_service.rs` 中 `create_branch`（约 74 行）**没有调用任何分支安全校验**，`create_worktree`（约 86 行）只校验 worktree path；两者都**不是幂等**的。
- `ProviderAdapter` trait 是**同步** `fn run(&self, ...)`，真实实现有 `CliProviderAdapter`、`RoutingProviderAdapter`、`ProviderOverrideAdapter` 等，但 `WebAppState` **没有** `provider_adapter` 字段；当前 web runtime 的真实 provider adapter 藏在 `WebRuntime.real_provider: Option<Box<dyn ProviderAdapter + Send + Sync>>` 中。
- `handlers::abort_coding_attempt`（约 754 行）直接调用 `coding_store.update_attempt_status(Aborted)`，**不经过** `CodingWorkspaceEngine::handle_abort`。
- `coding_workspace_engine.rs::complete_attempt_after_final_rework`（约 4762 行）直接置 `Completed` 并调用 `mark_work_item_completed_if_present`，**绕过** `handle_final_confirm` 的所有门禁。
- 前端 `LifecycleWorkItem`、`GenerateWorkItemsRequest/Response`、`CodingWsOutMessage::CodingSessionState` 都还没有 split/execution-plan/handoff 字段，P7/P8 需要扩展。

---

## 5. 🔴 阻塞问题（必须在实施前或对应 P 中解决）

### 5.1 `ProviderAdapter` 生产化与 `WebAppState` 初始化缺口

- P3/P6 计划都要求 `WebAppState` 新增 `provider_adapter: Arc<dyn ProviderAdapter>`，但当前 `ProviderAdapter` trait **没有 `Send + Sync` bound**；若不加 bound，`WebAppState` 无法安全跨 tokio task 传递。
- 真实运行时的 provider adapter 目前只在 `WebRuntime::new_real()` 中通过 `real_routing_provider()` 构造。P3 需要明确：
  - `WebAppState` 的 `provider_adapter` 在 fake/test 模式下如何初始化？
  - 真实模式下是从 `WebRuntime` 复用，还是在 `WebAppState` 构造时独立创建？
  - `ProviderAdapter::run` 是同步调用，P3 的 `split_engine.generate` 与 P6 的 `generate_work_item_handoff` 都是 `async`；在异步函数中直接调用同步 `run` 会阻塞 tokio worker，建议用 `tokio::task::spawn_blocking` 包裹或把 trait 改为 async。

### 5.2 `create_branch` / `create_worktree` 非幂等 → Issue 共享 worktree 复用会失败

- P5 要求同一 Issue 下的多次 Coding attempt **复用** `aria/issues/{issue_id}` branch 与 `.worktrees/aria-issues/{issue_id}` worktree。
- 当前 `create_branch` 执行 `git branch <branch> <base>`，branch 已存在时会失败；`create_worktree` 执行 `git worktree add <path> <branch>`，path 已注册时也会失败。
- P5 的 `execute_worktree_prepare` 只是简单把 `worktree_path_for_attempt` 改成 Issue 路径，没有处理「branch/worktree 已存在则复用」的逻辑。
- **后果**：同一 Issue 下第二个 attempt 启动时必然失败。

### 5.3 `abort_coding_attempt` / `delete_coding_attempt` 绕过 engine，导致 active lock 泄漏

- P5 计划在 `handle_abort` 中释放 `IssueSharedWorktree` active lock。
- 但 `handlers::abort_coding_attempt` 直接改 store，**不调用** `handle_abort`；用户通过 UI/API 中止 attempt 时，attempt 变成 Aborted，但锁不会释放。
- `delete_coding_attempt` 也可能在 attempt active 时删除记录，锁同样不会释放。
- **后果**：Issue 共享 worktree 死锁，后续 Work Item 无法启动。

### 5.4 `complete_attempt_after_final_rework` 绕过最终门禁

- 该函数在 Analyst decision 选择进入 FinalConfirm 时被调用（约 4970 行），内部直接置 `Completed` 并标记 Work Item 完成。
- P6 的 diff-scope / verification / handoff / clean-gate 如果只加在 `handle_final_confirm` 中，**此路径会完全绕过**。
- P5 计划已意识到这一点（P5 任务 3 步骤 5 第 6 条），但 P6 计划的任务 4-5 主要围绕 `handle_final_confirm` 写测试，没有明确覆盖 `complete_attempt_after_final_rework`。
- **后果**：存在两条完成路径，逻辑分叉，越界改动、缺失 handoff 都可能从 auto-complete 路径漏过去。

### 5.5 P3 中 `WorkItemSplitValidator::validate` 调用签名不一致

- P2 定义签名：`validate(plan, candidates, Option<&RepositoryProfile>, &[VerificationPlan])`。
- P3 任务 4 代码片段写成 `WorkItemSplitValidator::validate(plan, candidates)`，缺少后两个参数。
- **后果**：P3 实现到该步骤时直接编译失败。

---

## 6. ⚠️ 高风险 / 需要澄清的问题

| # | 问题 | 说明 |
|---|------|------|
| 6.1 | `max_auto_rework_exceeded` 改为 Failed 与可恢复 gate 冲突 | P5 步骤 5 第 2 条要求把该路径从 `Blocked` 改为 `Failed`，但该路径原有 `continue_rework`/`provide_context` 选项，属于可恢复人工 gate。改为 Failed 会移除用户继续能力，需确认是否接受此行为变更。 |
| 6.2 | `head_commit` 可能为 `None` | `CodingExecutionAttempt.head_commit` 只在 review request / push 后设置；`complete_attempt_after_final_rework` 若在未 review 时被触发，diff gate / completion_commit 取不到值。 |
| 6.3 | `CreateWorkItemInput` 是否加 `Default` | P3 v1.1 建议加 `Default` 以减少 20+ 处 legacy 调用改动。但加 Default 会让新字段默认空/零值，测试可能遗漏字段；需明确「legacy 调用只是占位，真实生成必须显式填充」。 |
| 6.4 | `IssueSharedWorktree` active lock 非分布式 | lock 基于 Aria 内部文件读写，多实例同时操作同一 Issue 时无法互斥。当前设计假设单实例。 |
| 6.5 | `AdapterRole` 新增变体的编译影响 | P3 可能新增 `WorkItemSplitter`，P6 可能新增 `Handoff`。需检查是否有 exhaustive `match` 会因此编译失败。 |
| 6.7 | dirty worktree 恢复 UX | P5/P6 规定 dirty 时保持锁并返回人工 gate，但 UI 如何展示、如何让用户「处理干净后继续」没有在前端计划中细化。 |

---

## 7. 详细分 P 审查

### P1 后端模型收敛与调度器迁移

- **可行**。删除 `WorkItemRecord`/`WorkItemStore`、扩展 `LifecycleWorkItemRecord`、迁移 `worktree_scheduler` 都是局部改动。
- **注意**：`CreateWorkItemInput` Default 策略需与 P3 对齐；`execution_mode` 字段默认值需保证调度器行为不变。

### P2 IssueWorkItemPlan 与 SplitValidator

- **可行**。纯模型/纯函数校验，不调用 provider，不碰 HTTP。
- **注意**：`RepositoryProfile` / `VerificationPlan` 必须只校验结构、关联、安全边界，不得按 Rust/pnpm/Playwright 等当前仓库技术栈生成兜底命令（设计已明确，实现时需坚守）。

### P3 后端多 Work Item 生成与 Artifact 关联

- **方向正确**，但 **5.1 / 5.5** 两个阻塞点在此 P 爆发。
- 另外，`GenerateWorkItemsResponse` 的兼容方案（保留单数 `workspace_session` + 新增 `workspace_sessions`）合理，但前端 P7 需明确消费哪个字段。

### P4 Issue 共享 Worktree 与 Git 安全前缀

- **可行**，且 v1.1 已正确识别 `create_branch` 没有分支安全校验的问题。
- **建议**：在参数化前缀的同时，顺手把「branch/worktree 已存在则复用」的幂等语义补上，否则 P5 会踩雷。

### P5 后端 Coding 启动门禁与共享 Worktree 复用

- **核心 P，风险最高**。
- 除 **5.2**（幂等）外，必须同步修复 **5.3**（abort/delete 绕过 engine）。
- 建议把 `handle_attempt_failed` 做成所有 Failed/Superseded/Blocked 终态的统一入口，避免每个失败路径单独写锁释放。

### P6 后端 ExecutionPlan 与 Handoff ProviderRun

- **方向正确**，但 **5.4**（`complete_attempt_after_final_rework` 绕过门禁）必须解决。
- 建议把最终门禁抽成一个独立 helper：`run_completion_gates(attempt) -> Result<CompletionGateReport, Error>`，然后 `handle_final_confirm` 和 `complete_attempt_after_final_rework` 都调用它。

### P7/P8 前端

- **依赖后端字段落地**。P7 明确声明「硬前置：P3 必须先交付 DTO 字段」，这一点必须严格执行，避免前端伪造状态。
- P7/P8 共享 `web/src/api/types.ts`，必须串行，不能并行。

### P9 贯通测试

- **测试为主，符合边界**。
- 不做浏览器 E2E；仅通过后端 `it_web` 贯通测试与前端 Vitest 集成测试验证状态机、API 与 UI 联动。

---

## 8. 建议的下一步行动

1. **先补计划补丁**（在进入 P1 编码前）：
   - 明确 `ProviderAdapter` 在 `WebAppState` 中的初始化方案，并给 trait 加 `Send + Sync` bound。
   - 在 P4/P5 计划中补充 branch/worktree 复用的幂等逻辑。
   - 在 P5 计划中明确 `abort_coding_attempt` / `delete_coding_attempt` 必须调用 engine，避免锁泄漏。
   - 在 P6 计划中明确 `complete_attempt_after_final_rework` 必须复用最终门禁 helper。
   - 修正 P3 中 `WorkItemSplitValidator::validate` 调用签名。
2. **按顺序实施**：P1 → P2 → P3 → P4 → P5 → P6 → P7 → P8 → P9，严格执行串行依赖。
3. **每 P 完成后运行标准验证**：
   - `cargo fmt --check`
   - `cargo clippy --all-targets --all-features --locked -- -D warnings`
   - `cargo test --locked`
   - `cd web && pnpm exec tsc -b && pnpm test`
4. **P9 只做贯通测试**：后端 `it_web` 覆盖完整状态机流程，前端 Vitest 覆盖 Workbench 与 Coding Prepare 联动；浏览器 E2E 不在本计划内实现。

---

## 9. 结论

**方案整体可行，设计方向正确，但不是一个可以「直接按顺序开干」的计划。**

Backend 是主要战场，P5/P6 存在锁泄漏、API 绕过、完成路径分叉等真实风险。建议在开始编码前，先用一个小补丁轮（或设计评审 v1.1→v1.2）把 **5.1-5.5** 这 5 个阻塞问题消灭或给出明确实现方案，再进入 P1 的 TDD 循环。这样可以避免后期大规模返工。
