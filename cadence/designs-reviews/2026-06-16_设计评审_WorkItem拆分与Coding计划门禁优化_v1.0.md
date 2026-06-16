# WorkItem 拆分与 Coding 计划门禁优化设计评审

> 版本：v1.0 | 日期：2026-06-16 | 目标分支：`feat-b-0616`
> 被评审方案：`cadence/designs/2026-06-16_技术方案_WorkItem拆分与Coding计划门禁优化_v1.0.md`

## 一、评审结论

方案方向成立，立意准确：把单个大 Work Item 升级为 Issue 级 Work Item Set（DAG + 写入范围互斥 + 共享 worktree + handoff 摘要），直接命中"Coding 上下文过载、约束丢失、缺少显式依赖与交接"的真实痛点。非目标章节划分务实（不写入目标仓库、不强制串行、第一版不并发改同一 worktree、不自动 merge 主分支）。

方案的主要不足不在设计本身，而在"与现存代码的衔接"未充分交代。经对 `feat-b-0616` worktree 内现有实现的只读调研，确认有四处落差，其中两处影响实施顺序，需在动工前定调。结论：**无需独立 P0**，但需修订 P1 范围并补齐两处实现约定。

## 二、与现有实现的关键差距

### 2.1 "两套 Work Item 模型"实为"一套活模型 + 一坨死代码"

调研事实（位置见下）：

- 旧 `WorkItemRecord`（`src/product/models.rs:180`，含 `allowed_write_scope`/`depends_on`/`execution_mode`/`worktree_branch`）在整个代码库**仅被 `worktree_scheduler.rs` 引用 3 处**（定义、use、函数参数）。
- `worktree_scheduler.rs` 的唯一公开函数 `ready_work_items()` **没有任何调用者、无单元测试、完全孤立**。
- 配套 `WorkItemStore`（`src/product/work_item_store.rs`）是空壳，只有 `paths` 字段，无方法、无消费者。
- 实际运行链路 100% 走 `LifecycleWorkItemRecord`（`src/product/models.rs:311`）：`lifecycle_store` 持久化、`handlers.rs` 的 `generate_work_items`/`start_work_item_attempt`/`delete_work_item` 操作它、`coding_workspace_engine.rs` 更新其 `execution_status`、`workspace_engine.rs` 更新其 `plan_status`。
- 两套模型间无任何 `From`/桥接代码，仅共享 `WorkItemStatus` 枚举。

**结论与建议**：方案把新字段（`depends_on`/`exclusive_write_scopes`/`forbidden_write_scopes`/`context_budget_k` 等）全部挂在 `LifecycleWorkItemRecord` 上是**唯一正确路径**，不存在双模型冲突风险。

- P1 应**顺带删除**孤立的 `WorkItemRecord` 与 `WorkItemStore`，避免演化出第三套并存模型。
- `worktree_scheduler.rs` 的调度算法（`WaitingForDependency`/`WaitingForScope`/`Ready` 判定，依赖活代码 `cross_cutting::worktree::scopes_may_overlap`）**正好就是方案「执行期校验」(原方案 L242-244) 需要的逻辑**。建议保留该纯函数，仅把入参类型从 `WorkItemRecord` 迁移到 `LifecycleWorkItemRecord`，复用算法，省去重写。
- 因死代码清理成本极低（无消费者），**不需要独立的 P0 模型收敛阶段**，并入 P1 即可。

### 2.2 共享 worktree 与现有硬编码约定 / 隐藏假设冲突

调研事实：

- worktree 路径在 `coding_workspace_engine.rs:5045 worktree_path_for_attempt` 按 `work_item_id + attempt_no` 计算，**不含 issue_id**；branch 在 `handlers.rs:607` 按 `aria/work-items/{work_item_id}/attempt-{n}` 生成。
- 安全校验**硬编码**：`git_workspace_service.rs:417` 要求路径在 `.worktrees/aria-work-items` 前缀内；`:443` 要求 branch 以 `aria/work-items/` 开头。`delete_local_branch`（`:128-145`）也走 `ensure_safe_attempt_branch_name`，**改成 `aria/issues/` 后会阻止清理**。
- **隐藏假设（最关键）**：`git_add_work_item_changes` 对 worktree 执行 `git add -A`。多个 attempt 共享同一 worktree 会**互相污染暂存区**——现有 commit/push/diff 流程全部默认 worktree 独占。
- `WorktreeLease`/`WorktreeLeaseManager`（`src/cross_cutting/worktree.rs`）**与 coding attempt 流程完全隔离**，仅在 `runtime_units/execution_setup.rs`（另一套 daemon 流程，branch 命名 `aria/{worktask_id}`）使用。方案所说"同一时刻只有一个 active Work Item"**目前没有任何机制保障**。
- rework 复用同 attempt 的 worktree；新 attempt 新建 worktree；清理仅在显式删除（`delete_coding_attempt`/`delete_work_item` → `cleanup_coding_attempt_workspace`，`handlers.rs:2210`）时触发，无自动清理。

**结论与建议**：推荐**方案一（attempt 复用 Issue 级共享 worktree）**，否定"保留 attempt worktree + 末端汇聚"方案——后者需引入跨 worktree cherry-pick/merge，与 `execute_review_request` 直接在 attempt worktree 上 commit+push、以 `base_branch` 为 diff 基线的现有假设正面冲突。配套改造：

1. `worktree_path_for_attempt` 改为按 `issue_id` 计算路径，branch 改 Issue 级；`git_workspace_service.rs:417/443` 两处校验前缀**参数化**（同时放行 `aria-work-items` 与 `aria-issues`，兼容存量数据）。
2. 用 `IssueSharedWorktree.current_active_work_item_id` + 执行期校验（原方案 L243）实现**应用层串行锁**，替代未接入的 `WorktreeLease`，契合非目标 4「第一版同一时刻只一个 active」。
3. **关键收益**：串行执行下，`git add -A` 的暂存区污染问题自然消解（任一时刻只有一个 attempt 动 worktree）。这反向印证方案"第一版不并发"的决策正确——恰好绕开最大技术雷区。
4. 每个 Work Item 完成后在共享 worktree 上 commit 留痕（原方案 L74），作为后续 Work Item 起点与 handoff 的 diff 来源。

### 2.3 `confirmed` 门禁已存在，方案是叠加第二道 gate

`create_coding_attempt`（`handlers.rs:571`）已实现 `plan_status != Confirmed` 拦截。方案在其上再引入每个 Work Item 的 `WorkItemExecutionPlan` 确认（原方案 L108-109、L245），构成"先确认拆分计划，再逐个确认执行计划"两道用户确认。详见第三节决策三。

### 2.4 handoff summary 生成主体未定义

方案多处称"系统生成 `WorkItemHandoff`"（原方案 L110-111、L249-253），未说明产出方式。详见第三节决策四。

## 三、四个流程决策的最终结论

以下决策已与方案负责人确认。

| 决策项 | 结论 | 说明 |
|---|---|---|
| **模型收敛** | 先调研已完成，结论见 2.1 | 实为清理死代码 + 新字段挂 `LifecycleWorkItemRecord`，并入 P1，无需 P0 |
| **worktree 绑定** | 采纳方案一（attempt 复用 Issue worktree），配套见 2.2 | 否定末端汇聚方案；应用层串行锁替代未接入的 WorktreeLease |
| **执行计划确认 gate** | **可配置** | 默认仅确认拆分计划；允许用户对高风险 Work Item 开启逐个 `WorkItemExecutionPlan` 确认。避免默认双重确认带来的操作负担 |
| **handoff 产出** | **额外 provider run 生成** | Coding 结束后再跑一次 provider 总结交付内容；摘要质量优先 |

### 3.1 执行计划确认 gate 可配置的落地要点

- `IssueWorkItemPlan` 或生成选项中增加开关（如 `require_execution_plan_confirm`），默认关闭。
- 关闭时：`WorkItemExecutionPlan` 仍生成并展示（不阻塞），Coding 在拆分计划 confirmed 后即可启动。
- 开启时：对应 Work Item 的 `execution_plan_status` 必须为 `confirmed` 才能进 Coder（复用现有 plan_status gate 模式）。
- 校验规则（原方案 L245「execution plan 已在 Aria 内确认」）需相应改为"在开启该 Work Item 确认时才强制"。

### 3.2 额外 provider run 生成 handoff 的落地要点

- handoff 生成本身消耗 token/时间，需计入整体预算，**不占用下一个 Work Item 的 30k-50k 执行包预算**（二者是不同 run）。
- 输入应为该 Work Item 的 diff、files_changed、测试结果摘要；输出落 `WorkItemHandoff`（原方案 L180-198 字段），仅存 Aria 内部。
- 新增执行阶段或在现有阶段后追加 handoff 生成步骤，并作为"Work Item 完成"门禁（原方案 L267：缺 handoff 不得标记完成、不得解锁依赖项）。
- diff 越界校验应复用 `cross_cutting/worktree.rs` 的 `validate_write_path`/`is_forbidden_runtime_path`（活代码），不要新写。

## 四、其他需要补强的点

- **上下文预算「30k-50k」无测量手段**（原方案 L36/L87/L233）。`estimated_context_k` 来源不明，provider 自报不可信。建议改成可执行的代理指标：refs 数量、文件路径数、摘要字符数上限。否则该校验形同虚设。
- **"跨端 Issue"判定标准缺失**。强制前后端拆分（原方案 L78/L230）依赖"是否跨端"的判断，方案未说明系统如何识别（provider 输出？仓库结构？）。判错会对纯后端 Issue 强行要求前端 Work Item。
- **markdown 间接存储的脆弱性**。当前 Work Item 正文存在 workspace session 的 artifact versions（`coding_workspace_engine.rs:4417`），拆成多个 Work Item 后 session↔work_item 关联不能断，P2 需专门保证。
- **清理逻辑的安全门**。改 branch 前缀为 `aria/issues/` 后，`delete_local_branch` 的 `ensure_safe_attempt_branch_name` 会拒删，须随 2.2 的前缀参数化一并放行。

## 五、实施拆分修订建议

原方案 P1-P7 切分合理、粒度符合"单 session 可完成"。修订：

- **不新增 P0**（模型收敛成本极低，并入 P1）。
- **P1（数据模型 + Split Validator）**追加：删除孤立 `WorkItemRecord`/`WorkItemStore`；迁移 `worktree_scheduler` 调度算法到 `LifecycleWorkItemRecord`。
- **P3（Issue 共享 worktree + Coding 门禁）**追加：worktree 路径/branch 改 Issue 级；`git_workspace_service` 安全校验前缀参数化（兼容存量 + 放行清理）；应用层串行锁。
- **P4（WorkItemExecutionPlan + Prepare 确认）**调整：执行计划确认改为可配置开关，默认不阻塞。
- **P5（Handoff + 上下文注入）**明确：handoff 由额外 provider run 产出，独立预算；diff 越界校验复用现有 `worktree.rs`。

每个计划仍应控制在单 Coding session 范围内，TDD 先写测试。

## 六、参考：关键现有实现位置

| 主题 | 位置 |
|---|---|
| 新模型定义 | `src/product/models.rs:311` `LifecycleWorkItemRecord` |
| 旧模型（待清理） | `src/product/models.rs:180` `WorkItemRecord` |
| 孤立调度器 | `src/product/worktree_scheduler.rs`（`ready_work_items` 无调用者） |
| 计划门禁 | `src/web/handlers.rs:571` `plan_status != Confirmed` |
| attempt 创建 / branch 生成 | `src/web/handlers.rs:557` / `:607` |
| worktree 路径计算 | `src/product/coding_workspace_engine.rs:5045` |
| worktree 创建 | `src/product/coding_workspace_engine.rs:295` `execute_worktree_prepare` |
| 安全校验（硬编码前缀） | `src/product/git_workspace_service.rs:417` / `:443` |
| commit/push（git add -A 假设） | `execute_review_request` `coding_workspace_engine.rs:3446` |
| worktree 清理 | `src/web/handlers.rs:2210` `cleanup_coding_attempt_workspace` |
| 并发 Lease（未接入 coding） | `src/cross_cutting/worktree.rs` / `runtime_units/execution_setup.rs` |
| 写入范围校验（应复用） | `cross_cutting/worktree.rs` `validate_write_path`/`is_forbidden_runtime_path` |
| Work Item 正文存储 | `src/product/coding_workspace_engine.rs:4417`（workspace session artifact versions） |
