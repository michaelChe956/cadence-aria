# WorkItem 拆分与 Coding 计划门禁优化技术方案

## 文档信息

- 文档类型：技术方案
- 创建日期：2026-06-16
- 版本：v1.1
- 目标分支：`feat-b-0616`
- 工作区：`.worktrees/feat-b-0616`
- 适用范围：Product Workbench 中 Design Spec 生成 Work Item、Work Item 依赖编排、Issue 级共享 worktree、Coding Workspace 启动门禁与上下文控制
- 修订依据：`cadence/designs-reviews/2026-06-16_设计评审_WorkItem拆分与Coding计划门禁优化_v1.0.md`

## v1.1 修订摘要

本版保留 v1.0 的核心方向：Issue 级 Work Item Set、DAG、写入范围互斥、共享 worktree、handoff 摘要和上下文预算控制。根据设计评审，本版补齐与现有实现衔接相关的落地约定：

- 明确新字段只挂到活跃模型 `LifecycleWorkItemRecord`；P1 同步清理孤立旧模型 `WorkItemRecord` 与空壳 `WorkItemStore`。
- 明确复用并迁移 `worktree_scheduler.rs` 的依赖与写入范围调度逻辑，而不是重写调度算法。
- 明确 Issue 级共享 worktree 会替代 attempt 独占 worktree；同时参数化现有 `git_workspace_service` 安全前缀，兼容存量 `aria/work-items/*` 并支持新增 `aria/issues/*`。
- 明确第一版用 `IssueSharedWorktree.current_active_work_item_id` 做应用层串行锁，避免共享 worktree 下 `git add -A` 互相污染。
- 将 `WorkItemExecutionPlan` 确认门禁改为可配置：默认生成并展示但不阻塞，高风险 Work Item 可开启逐项确认。
- 明确 `WorkItemHandoff` 由额外 provider run 生成，独立预算，不占用下一个 Work Item 的执行包预算。
- 将 30k-50k 上下文预算从不可测的 token 估算，改为可执行的代理指标：摘要字符数、refs 数、文件路径数、代码片段字符数与 handoff 数量。
- 明确跨端 Issue 判定标准，避免纯后端或纯前端 Issue 被误拆。
- 明确共享同一源码文件的后端计划必须严格串行：P3、P4、P5 都修改 `src/product/lifecycle_store.rs`，必须按 P3 → P4 → P5 顺序执行，不得并行；具体落地编排以拆分总览为准。
- 明确每个实现计划的验证链必须包含项目强制 clippy 检查（`cargo clippy --all-targets --all-features --locked -- -D warnings`），不允许只跑 `fmt + check`。

## 背景

当前 Product Workbench 已具备 `Issue -> Story Spec -> Design Spec -> Work Item -> Coding Workspace` 链路。现有实现中，Design Spec 生成 Work Item 时通常创建一个 Work Item，并通过 `workspace_type=work_item` 的文档 Workspace 生成 Markdown Work Item。用户确认后，后端将 `WorkItemPlanStatus` 置为 `confirmed`，Coding Attempt 即可启动。

该模式对小任务可用，但对跨前后端、跨测试层级的 Issue 风险较高：

- 单个 Work Item 容易同时包含后端、前端、贯通测试和返修约束，Coding 阶段上下文过大。
- 上下文压缩后，Coder 容易遗漏 Story/Design/OpenSpec 约束，产生幻觉或越界改动。
- Work Item 之间缺少显式依赖与交接摘要，后续任务很难稳定复用前序交付结果。
- 现有 attempt 级 worktree 不适合一个 Issue 下多个 Work Item 持续交付同一条变更线。
- 前后端拆分、写入范围互斥、贯通测试是否启用，目前主要依赖 provider 输出质量，缺少系统级校验。

本方案将 Work Item 从“单个大计划文档”升级为“Issue 内部的一组可交付执行单元”，并用 DAG、写入范围、上下文预算和共享 worktree 约束 Coding 流程。

## 目标

1. Design Spec 生成 Work Item 时，强制拆分为多个可在单个 Coding session 内完成的 Work Item。
2. 每个 Work Item 独立交付，但多个 Work Item 合在一起完成同一个 Issue。
3. 跨端 Issue 中，前端与后端实现必须拆分到不同 Work Item；跨端行为由可选贯通测试或 E2E Work Item 覆盖。
4. 用户可选择是否生成贯通测试或端到端测试 Work Item。
5. Work Item 之间使用 DAG 表达依赖；只有必要顺序才排序，非依赖项可并行规划。
6. 并行 Work Item 的写入范围必须互斥；无法证明互斥时必须拆细或建立依赖。
7. 同一个 Issue 下的所有 Work Item 使用同一个共享 branch/worktree。
8. 后序 Work Item 执行时必须注入前序依赖项的交付摘要，而不是完整历史上下文。
9. 每个 Work Item 的执行输入包按可测代理指标控制在 30k-50k tokens 等价规模，避免超过单 session 可控范围。
10. Work Item 拆分、状态、执行计划、交付摘要均为 Aria 内部数据，不向目标项目代码库写入任务产物。

## 非目标

1. 不把 Work Item、拆分计划、执行计划或 handoff 文档写入目标项目仓库。
2. 不要求 `cadence/plans/` 中存在 Work Item 计划文件作为 Coding 启动条件。
3. 不把所有 Work Item 强制串行执行；只对存在依赖或写入冲突的 Work Item 建立顺序。
4. 不在第一版实现多 Work Item 同时修改同一个共享 worktree。即使 DAG 中存在可并行项，执行层仍先保证同一时刻只有一个 active Work Item 修改共享 worktree。
5. 不自动 merge 到目标主分支；共享 branch 的最终集成策略沿用 Coding Workspace 现有 review request/final confirm 方向。
6. 不把旧 daemon `WorktreeLease` 接入 Coding Workspace 第一版；第一版使用应用层串行锁，后续需要并发执行时再评估 lease 化。

## 核心决策

### 1. 采用 Issue 级 Work Item Set

Design Spec 生成 Work Item 时，不再只创建单个 Work Item。系统先生成一个 Issue 级 `IssueWorkItemPlan`，包含多个 Work Item、依赖 DAG、写入范围、上下文预算代理指标、验收策略和贯通测试选项。

用户确认该拆分计划后，Work Item 卡片进入可执行状态。Issue 完成条件由 Work Item Set 决定：所有必选 Work Item 完成后，Issue 才能进入完成态；如果用户启用了贯通测试或 E2E Work Item，该 Work Item 也必须完成。

### 2. Work Item 是 Aria 内部执行单元

Work Item 状态事实源保存在 Aria 产品数据中，例如 lifecycle store 或 `.aria` 数据目录。目标项目仓库只接受 Coding 阶段产生的业务代码改动。

以下内容不得落入目标项目代码库：

- Work Item 拆分计划。
- Work Item 状态。
- Work Item 执行计划。
- Work Item handoff summary。
- Work Item 依赖图与写入范围元数据。

### 3. 活跃模型收敛到 LifecycleWorkItemRecord

现有代码中真正参与 Product Workbench 与 Coding Workspace 链路的是 `LifecycleWorkItemRecord`。旧 `WorkItemRecord` 与 `WorkItemStore` 不参与当前运行链路，`WorkItemStore` 也是空壳。

因此本方案新增字段只扩展 `LifecycleWorkItemRecord`，并在 P1 中清理孤立旧模型，避免未来演化出多套 Work Item 事实源。

`worktree_scheduler.rs` 中已有的依赖与写入范围调度算法应保留并迁移到 `LifecycleWorkItemRecord`，用于判断 Work Item 是 `ready`、`waiting_for_dependency` 还是 `waiting_for_scope`。

### 4. 同一 Issue 共享一个 worktree branch

一个 Issue 创建一个共享 branch/worktree，例如：

- branch：`aria/issues/<issue_id>`
- worktree：`<repo>/.worktrees/aria-issues/<issue_id>`

同一 Issue 下所有 Work Item 的 Coding Attempt 都在该 worktree 上连续执行。每个 Work Item 完成后在共享 worktree 上 commit 留痕，记录 commit/head、diff 摘要、测试结果和 handoff summary，供依赖它的后续 Work Item 使用。

现有安全校验需要从硬编码 `aria/work-items/*` 改为参数化允许列表：

- 保留 `aria/work-items/*` 和 `.worktrees/aria-work-items/*`，兼容存量 attempt。
- 新增 `aria/issues/*` 和 `.worktrees/aria-issues/*`，支持 Issue 级共享 worktree。
- 删除本地 branch 与清理 worktree 时使用同一套安全前缀规则，避免新前缀无法清理。

第一版不支持多个 Work Item 同时修改共享 worktree。`IssueSharedWorktree.current_active_work_item_id` 是应用层串行锁。该约束也避免现有 `git add -A` 在共享 worktree 下污染其他 Work Item 的变更。

### 5. 前后端强制拆分，贯通测试可选

跨端 Issue 必须至少生成后端/API Contract Work Item 与前端/UI Work Item。二者不得合并为同一个 Work Item。

跨端判定使用组合信号，而不是单靠 provider 自报：

- Design Spec 中同时出现后端 API/数据/状态机约束与前端 UI/状态/交互约束。
- 计划写入范围同时覆盖后端路径和前端路径，例如 `src/**`、`src/web/**`、`src/product/**` 与 `web/src/**`。
- 用户在生成选项中明确要求前后端拆分。
- Provider 分类为跨端，但 validator 仍需检查写入范围和任务内容是否支持该分类。

纯后端 Issue 不强行生成 Frontend Work Item；纯前端 Issue 不强行生成 Backend Work Item。若用户显式要求前后端拆分，则必须生成对应 Work Item 或返回可解释的阻塞原因。

贯通测试或 E2E Work Item 由用户选择是否生成：

- 用户启用时，系统必须生成 Integration/E2E Work Item，并让它依赖相关前后端 Work Item。
- 用户跳过时，系统必须在 Aria 内部记录风险和后续手工验证建议，但不阻塞 Work Item Set 确认。

### 6. WorkItemExecutionPlan 确认门禁可配置

现有 `create_coding_attempt` 已有 `plan_status=confirmed` 门禁。本方案在其上引入 `WorkItemExecutionPlan`，但不默认增加第二道强制确认。

规则：

- `WorkItemExecutionPlan` 始终生成并展示，作为 Coder prompt 的结构化来源。
- 默认情况下，拆分计划 confirmed 后即可启动 Coding，`WorkItemExecutionPlan` 不阻塞。
- 当 `require_execution_plan_confirm=true` 时，对应 Work Item 的 `execution_plan_status` 必须为 `confirmed` 才能进入 Coder。
- 用户可在 Issue Work Item Set 级别开启该开关，也可只对高风险 Work Item 开启。

这样保留高风险场景的人审能力，同时避免所有 Work Item 都需要双重确认带来的操作负担。

### 7. Handoff 由额外 provider run 生成

`WorkItemHandoff` 不由简单模板拼接产生，而是在 Work Item 代码、测试、review 阶段完成后，额外运行一次 provider 总结。

handoff provider run 的输入包括：

- 本 Work Item 的目标和范围。
- diff summary。
- files changed。
- 测试结果摘要。
- review findings 摘要。
- commit/head。
- API 或契约变化。

handoff run 输出写入 Aria 内部 `WorkItemHandoff`。它是 Work Item 完成门禁的一部分：缺 handoff 不得标记 Work Item 完成，也不得解锁依赖它的后续 Work Item。

handoff run 消耗 token 和时间，但它独立于下一个 Work Item 的 30k-50k 执行包预算。后续 Work Item 只消费 handoff 摘要，不消费 handoff 生成过程。

### 8. 上下文预算按代理指标控制

上下文预算不是精确 token 计数，而是执行输入包规模约束。每个 Work Item 的 Coding 输入包目标为 30k-50k tokens 等价规模。由于当前系统没有稳定 token 计数器，第一版使用代理指标：

- `max_summary_chars`：Story/Design/OpenSpec 摘要总字符数。
- `max_handoff_chars`：依赖 handoff 摘要总字符数。
- `max_code_context_chars`：代码片段或结构摘要总字符数。
- `max_context_file_refs`：上下文文件路径数量。
- `max_traceability_refs`：REQ/DEC/TASK/AC refs 数量。
- `max_dependency_handoffs`：注入的直接依赖 handoff 数量。

`estimated_context_k` 可以作为展示字段保留，但不能作为唯一门禁。门禁必须基于以上代理指标。超过阈值时，生成期必须继续拆分 Work Item，或将完整上下文降级为摘要和文件路径引用。

## 总体流程

1. 用户在 confirmed Design Spec 上点击生成 Work Item。
2. 前端展示生成选项：是否生成贯通测试/E2E Work Item、是否要求逐项执行计划确认、是否显式强制前后端拆分。
3. 后端启动 Work Item Split Workspace。Provider 基于 confirmed Story Spec、confirmed Design Spec、OpenSpec 约束、仓库结构摘要和用户选项生成 `IssueWorkItemPlan`。
4. `WorkItemSplitValidator` 校验 DAG、写入范围、前后端拆分、上下文预算代理指标、贯通测试选项、traceability 和验证策略。
5. 校验失败时，拆分计划进入返修，不创建可执行 Work Item。
6. 校验通过后，用户在 Aria UI 中确认拆分计划。
7. 确认后，多个 Work Item 卡片进入 Work Item 列。无依赖项可先执行；有依赖项等待前置项完成。
8. 第一个进入 Coding 的 Work Item 创建 Issue 级共享 branch/worktree；后续 Work Item 复用该 worktree。
9. 每个 Work Item Coding 前，Aria 内部生成 `WorkItemExecutionPlan` 并展示。若当前 Work Item 开启确认门禁，则必须确认后才进入 Coder；否则直接作为 prompt 输入进入 Coder。
10. Coder prompt 包含当前 Work Item 的执行计划、允许/禁止写入范围、依赖项 handoff summary、验证目标和 OpenSpec/Superpowers/TDD 要求。
11. Work Item 代码与测试完成后，系统执行 diff scope 校验。校验逻辑复用 `cross_cutting/worktree.rs` 的 `validate_write_path` 与 `is_forbidden_runtime_path`，不要另写一套路径安全规则。
12. diff scope 校验通过后，系统运行额外 provider handoff run，生成 `WorkItemHandoff`。
13. Work Item 完成后在共享 worktree 上 commit 留痕。
14. 依赖它的后续 Work Item 启动时，只注入 handoff summary、commit/head、测试摘要和必要 refs。
15. 所有必选 Work Item 完成后，Issue Work Item Set 完成；如果启用了 Integration/E2E Work Item，它必须通过后才能完成整个 Issue。

## 数据模型

### IssueWorkItemPlan

Issue 级拆分总览，只存 Aria 内部数据。

字段建议：

- `id`
- `project_id`
- `issue_id`
- `source_story_spec_ids`
- `source_design_spec_ids`
- `include_integration_tests`
- `include_e2e_tests`
- `force_frontend_backend_split`
- `require_execution_plan_confirm`
- `status`: `draft | confirmed | change_requested`
- `work_item_ids`
- `dependency_graph`
- `created_from_provider_run`
- `validator_findings`
- `review_summary`
- `created_at`
- `updated_at`

### LifecycleWorkItemRecord 扩展

现有 Work Item 记录继续作为 Work Item 卡片事实源，新增字段：

- `work_item_set_id`
- `kind`: `backend | frontend | integration | e2e | docs | infra | other`
- `sequence_hint`
- `depends_on`
- `exclusive_write_scopes`
- `forbidden_write_scopes`
- `context_budget`
- `required_handoff_from`
- `require_execution_plan_confirm`
- `execution_plan_status`: `not_started | draft | confirmed | change_requested`
- `handoff_summary_ref`
- `completion_commit`
- `completion_diff_summary_ref`

### WorkItemContextBudget

用于替代不可测的纯 token 估算。

字段建议：

- `target_context_k`: 展示用，默认 `30-50`
- `max_summary_chars`
- `max_handoff_chars`
- `max_code_context_chars`
- `max_context_file_refs`
- `max_traceability_refs`
- `max_dependency_handoffs`

### WorkItemExecutionPlan

每个 Work Item Coding 前生成，默认展示但不阻塞；当 `require_execution_plan_confirm=true` 时，用户确认后才能进入 Coder。

字段建议：

- `id`
- `work_item_id`
- `attempt_id`
- `status`: `draft | confirmed | change_requested`
- `goal`
- `allowed_write_scopes`
- `forbidden_write_scopes`
- `dependency_handoffs`
- `story_refs`
- `design_refs`
- `openspec_refs`
- `superpowers_contract`
- `tdd_contract`
- `verification_commands`
- `context_budget`
- `risk_notes`
- `created_at`
- `updated_at`

### WorkItemHandoff

每个 Work Item 完成后由额外 provider run 生成，作为后续 Work Item 的压缩上下文来源。

字段建议：

- `id`
- `work_item_id`
- `attempt_id`
- `provider_run_ref`
- `summary`
- `files_changed`
- `commit_sha`
- `diff_summary`
- `tests_run`
- `test_result_summary`
- `review_summary`
- `api_or_contract_changes`
- `open_risks`
- `next_work_item_notes`
- `created_at`

### IssueSharedWorktree

Issue 级共享 worktree 记录。

字段建议：

- `id`
- `project_id`
- `issue_id`
- `repository_id`
- `branch_name`
- `worktree_path`
- `base_branch`
- `status`: `not_created | ready | running | blocked | completed`
- `current_active_work_item_id`
- `last_completed_work_item_id`
- `created_at`
- `updated_at`

## 校验规则

### 生成期校验

`WorkItemSplitValidator` 必须在拆分计划确认前执行：

- DAG 不允许有环。
- 依赖项必须属于同一 Issue。
- 必选 Work Item 必须能从入口项推进到整体完成。
- 无依赖关系的 Work Item 不允许 `exclusive_write_scopes` 重叠。
- 写入范围无法判断时，必须拆细或建立依赖，不能标记为可并行。
- 跨端 Issue 必须包含后端/API Contract Work Item 和前端/UI Work Item。
- 纯后端或纯前端 Issue 不强制生成另一端 Work Item，除非用户显式要求。
- 用户启用贯通测试或 E2E 时，必须生成对应 Integration/E2E Work Item。
- 用户跳过贯通测试或 E2E 时，必须记录风险说明。
- `WorkItemContextBudget` 的代理指标必须处于可控范围；超过阈值时必须拆分或摘要化。
- Work Item 必须包含 Story/Design/OpenSpec traceability refs。
- Work Item 必须包含验收目标与验证命令策略。
- Work Item 必须声明 Superpowers/TDD/Verification 使用要求。
- 每个 Work Item 必须能关联到自己的 workspace session 与 artifact versions，避免拆分后 Work Item 正文与实体断联。

### 执行期校验

启动 Coding 前必须检查：

- Work Item 所有 `depends_on` 已完成。
- Issue 级共享 worktree 已准备，或可安全创建。
- `IssueSharedWorktree.current_active_work_item_id` 为空，或等于当前 Work Item。
- 当前 Work Item 如果开启执行计划确认门禁，则 `WorkItemExecutionPlan.status` 必须为 `confirmed`。
- Coder prompt 包含允许与禁止写入范围。
- 依赖项的 handoff summary 可读取。

Work Item 完成时必须检查：

- diff 没有越过允许写入范围。
- 禁止写入范围未被修改。
- 必需验证命令已执行或明确进入人工 gate。
- handoff summary 已生成。
- completion commit/head 已记录。

## 错误处理

- Provider 生成的拆分计划不满足校验时，不创建可执行 Work Item，返回拆分计划返修。
- DAG 有环时，要求 provider 重新生成依赖关系。
- 写入范围冲突时，要求 provider 拆细 Work Item 或建立依赖。
- 上下文预算代理指标超限时，要求 provider 缩小 Work Item 范围或改用摘要引用。
- 用户关闭贯通测试/E2E 时不报错，但记录风险。
- 依赖 Work Item 未完成时，后序 Work Item 的 Coding 入口 disabled，并展示等待原因。
- 共享 worktree dirty 且当前 Work Item 非 active 时，进入人工 gate，不自动继续。
- Coder 越界改动时，进入人工 gate 或自动返修，不解锁后续 Work Item。
- Work Item 缺 handoff summary 时，不能标记为完成，也不能解锁依赖它的 Work Item。
- 新增 `aria/issues/*` branch/worktree 前缀后，清理逻辑必须通过参数化安全校验；若安全校验不认识该前缀，禁止绕过校验删除。

## 后端设计

### WorkItemSplitEngine

负责组装拆分上下文并调用 provider 生成 `IssueWorkItemPlan`。

输入：

- Issue 信息。
- confirmed Story Spec。
- confirmed Design Spec。
- OpenSpec 约束摘要。
- repository structure summary。
- 用户选项。

输出：

- draft `IssueWorkItemPlan`。
- draft `LifecycleWorkItemRecord` 列表。
- provider raw output 与校验 findings。

### WorkItemSplitValidator

负责所有生成期结构与语义校验。第一版应作为纯函数模块实现，便于单元测试覆盖。

该模块应复用 `worktree_scheduler.rs` 中已有依赖与 scope 判断思路，并将其入参迁移到 `LifecycleWorkItemRecord`。

### IssueWorktreeService

负责 Issue 级共享 branch/worktree：

- 创建或恢复 `aria/issues/<issue_id>` branch。
- 创建或恢复 `.worktrees/aria-issues/<issue_id>` worktree。
- 维护 `current_active_work_item_id` 应用层串行锁。
- 提供 worktree dirty/status 检查。
- 调用参数化后的 `git_workspace_service` 安全校验，兼容存量 work item attempt 前缀与新增 issue 前缀。

### WorkItemExecutionPlanner

负责 Coding 前生成 `WorkItemExecutionPlan`。默认展示但不阻塞；当 Issue 或 Work Item 启用 `require_execution_plan_confirm` 时，等待用户确认后 Coding Workspace 才能进入 Coder 阶段。

### WorkItemHandoffRunner

负责在 Work Item 代码、测试和 review 完成后运行额外 provider handoff run。

输入：

- Work Item 目标和范围。
- diff summary。
- files changed。
- testing summary。
- review summary。
- commit/head。

输出：

- `WorkItemHandoff`。
- provider raw output ref。

### CodingWorkspaceEngine 调整

Coding Workspace 需要从 Work Item 读取：

- Issue shared worktree。
- `WorkItemExecutionPlan`。
- `exclusive_write_scopes` / `forbidden_write_scopes`。
- dependency handoff summaries。
- context budget 代理指标。

Coder 结束后需要新增 diff scope 校验与 handoff 生成门禁。diff scope 校验复用 `cross_cutting/worktree.rs` 的路径安全函数。

### Lifecycle artifact 关联

拆分为多个 Work Item 后，每个 `LifecycleWorkItemRecord` 仍必须拥有自己的 workspace session 与 artifact versions。P2 实现 `generate_work_items` 多 Work Item 生成时，必须保证：

- 每个 Work Item 的 `entity_id` 与对应 workspace session 一一对应。
- Work Item 正文 artifact version 不被 IssueWorkItemPlan 或其他 Work Item 覆盖。
- Coding Workspace 仍能通过 work item session 找到当前 Work Item markdown。

## 前端设计

### Work Item 生成选项

Design Spec 卡片或 Drawer 触发生成 Work Item 时，前端展示选项：

- 显式强制前后端拆分：默认开启，可在纯后端/纯前端识别后由后端返回无需拆分说明。
- 生成贯通测试 Work Item：默认建议开启，可关闭。
- 生成 E2E Work Item：默认建议开启，可关闭。
- 高风险 Work Item 逐项确认执行计划：默认关闭，可开启。

用户选择会随 `generate_work_items` 请求发送给后端。

### Work Item Set 展示

Work Item 列需要展示 DAG 状态：

- 可执行。
- 等待依赖。
- 等待写入范围。
- 正在执行。
- 已完成。
- blocked。

卡片展示：

- Work Item kind。
- 写入范围。
- 上下文预算代理指标摘要。
- 依赖项。
- handoff 状态。
- 是否属于贯通测试或 E2E。
- 是否要求执行计划确认。

### Coding Workspace Prepare 阶段

Work Item 进入 Coding Workspace 后，Prepare 阶段优先展示 `WorkItemExecutionPlan`：

- 未开启确认门禁时，展示为即将使用的执行计划，可直接开始。
- 开启确认门禁时，用户确认后进入 Coder。
- 用户要求修改时，返回 execution plan 返修。

## 测试策略

### Rust 单元测试

覆盖：

- DAG 无环校验。
- 依赖项必须属于同一 Issue。
- 写入范围冲突校验。
- 跨端判定与前后端拆分校验。
- 纯后端 Issue 不强制前端 Work Item。
- 贯通测试/E2E 选项校验。
- 上下文预算代理指标超限校验。
- traceability refs 缺失校验。
- 旧 `worktree_scheduler` 算法迁移后能基于 `LifecycleWorkItemRecord` 判断 ready/waiting 状态。
- execution plan 未确认只在 `require_execution_plan_confirm=true` 时阻塞 Coding。
- handoff summary 缺失不能完成 Work Item。
- diff 越界进入 gate。

### Rust 集成测试

覆盖：

- `generate_work_items` 一次生成 IssueWorkItemPlan 和多个 Work Item。
- 用户关闭贯通测试时记录风险但不生成 Integration/E2E Work Item。
- 用户启用贯通测试时生成 Integration/E2E Work Item 并依赖前后端项。
- 同一 Issue 下多个 Work Item 复用同一个 shared branch/worktree。
- 新 `aria/issues/*` branch/worktree 前缀通过安全校验，存量 `aria/work-items/*` 仍兼容。
- 同一 Issue 同一时刻只能有一个 active Work Item 修改共享 worktree。
- 后序 Work Item 启动时注入前序 handoff summary。
- 多 Work Item 拆分后，每个 Work Item 的 workspace session 与 artifact version 仍可被 Coding Workspace 读取。

### 前端测试

覆盖：

- 生成 Work Item 选项 UI。
- Work Item DAG 展示。
- 等待依赖或等待写入范围时禁用 Coding 入口。
- Coding Prepare 阶段显示 execution plan confirmation 或非阻塞展示态。
- Work Item 卡片展示写入范围、预算和 handoff 状态。

### 贯通测试

覆盖：

- Backend Work Item 完成后，Frontend Work Item 能读取 backend handoff summary。
- 用户启用 Integration/E2E 时，Integration/E2E Work Item 等待前后端完成后才可执行。
- Integration/E2E Work Item 通过后，Issue Work Item Set 完成。

## 实施拆分建议

本方案范围较大，不建议一个实现计划一次做完。建议后续拆成以下计划：

1. P1：Work Item Set 数据模型与 Split Validator。
   - 扩展 `LifecycleWorkItemRecord`。
   - 删除孤立 `WorkItemRecord` 与空壳 `WorkItemStore`。
   - 迁移 `worktree_scheduler` 到 `LifecycleWorkItemRecord`。
   - 建立 DAG、scope、预算代理指标校验。
2. P2：`generate_work_items` 多 Work Item 生成与用户确认流。
   - 一次生成 IssueWorkItemPlan 和多个 Work Item。
   - 保证每个 Work Item 的 workspace session 与 artifact versions 关联正确。
3. P3：Issue 级共享 worktree 与 Coding 启动门禁。
   - branch/path 改 Issue 级。
   - `git_workspace_service` 安全前缀参数化。
   - 实现应用层串行锁。
4. P4：WorkItemExecutionPlan 与 Prepare 展示。
   - 默认不阻塞。
   - 高风险或用户开启时才要求确认。
5. P5：Handoff summary 与后序上下文注入。
   - handoff 由额外 provider run 生成。
   - diff 越界校验复用现有 worktree 路径安全函数。
6. P6：前端 DAG 展示与生成选项。
7. P7：贯通测试/E2E Work Item 与端到端验收。

每个计划都应控制在单个 Coding session 可完成的范围内，并使用 TDD 先写对应测试。

> **说明：** 上述 P1-P7 为方案级粗粒度建议。实际可执行的细粒度拆分以 `cadence/plans/2026-06-16_计划文档_实施计划_WorkItem拆分与Coding计划门禁优化_拆分总览_v1.0.md`（P1-P9）为准。

### 计划间写入范围与串行约束

多个后端计划共享同一批源码文件，必须按依赖顺序严格串行，禁止并行修改同一文件：

- `src/product/lifecycle_store.rs` 被 Issue 共享 worktree、Coding 启动门禁与 generate_work_items 相关计划共同修改，因此这些计划必须严格串行，不得并行准备。
- `src/web/handlers.rs`、`src/product/models.rs`、`src/product/coding_workspace_engine.rs` 同样存在跨计划共享，凡共享同一文件的计划一律按依赖顺序串行。
- 只有写入范围可证明完全互斥的计划才允许并行。

### 验证链强制要求

每个实现计划的验证链必须包含项目强制检查命令（详见 `cadence/project-rules/build-test-commands.md`）：

- `cargo fmt --check`
- `cargo clippy --all-targets --all-features --locked -- -D warnings`
- `cargo check --locked`
- 该计划的定向测试

不允许只跑 `fmt + check` 而省略 clippy，尤其在删除死代码或迁移模块后必须用 clippy 拦截新引入的 `unused import` 等告警。

## 验收标准

- Design Spec 生成 Work Item 时，跨端 Issue 不再只生成一个大 Work Item。
- 后端与前端 Work Item 在跨端 Issue 中被强制拆开。
- 纯后端或纯前端 Issue 不会被误要求生成另一端 Work Item。
- 用户可选择是否生成贯通测试或 E2E Work Item。
- Work Item DAG 无环，且依赖关系可解释。
- 无依赖 Work Item 的写入范围不重叠。
- 单个 Work Item 执行输入包通过代理指标控制在 30k-50k 等价规模。
- 同一 Issue 下所有 Work Item 使用同一个共享 branch/worktree。
- 同一 Issue 同一时刻只有一个 active Work Item 修改共享 worktree。
- 新 `aria/issues/*` branch/worktree 前缀可安全创建、使用和清理；存量 `aria/work-items/*` 不被破坏。
- 后序 Work Item 能读取前序 Work Item 的 handoff summary。
- WorkItemExecutionPlan 默认展示但不阻塞；开启确认门禁时才阻塞。
- WorkItemHandoff 由额外 provider run 生成，缺失时不能完成 Work Item。
- Work Item 状态和计划不写入目标项目代码库。
- Coding 阶段越界改动不会自动解锁后续 Work Item。
- 多 Work Item 拆分后，Work Item workspace session 与 artifact versions 关联稳定。
- 相关后端、前端和贯通测试覆盖通过。
