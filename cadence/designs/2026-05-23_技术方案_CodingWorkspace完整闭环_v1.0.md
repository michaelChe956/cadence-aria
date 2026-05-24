# Coding Workspace 完整闭环技术方案

## 文档信息

- 文档类型：技术方案
- 分支：`product-workbench-issue-lifecycle`
- 工作区：`.worktrees/product-workbench-issue-lifecycle`
- 制定日期：2026-05-23
- 版本：v1.0
- 适用范围：Product Workbench 中 Work Item confirmed 后的浏览器内 Coding / Testing / Review / Rework / Review Request / Internal PR Review / Final Confirm 闭环
- 依据：
  - `cadence/analysis-docs/2026-05-20_分析报告_ProductWorkbench交互审计与参考项目调研_v1.0.md`
  - `cadence/designs/2026-05-20_技术方案_Workspace产品工作台优化_v1.0.md`
  - `src/web/workspace_ws_types.rs`
  - `src/product/models.rs`
  - `src/product/workspace_engine.rs`
  - `src/web/handlers.rs`
  - `src/task_run/orchestrator.rs`

---

## 一、背景与问题

当前 Product Workbench 已经具备 Issue、Story Spec、Design Spec、Work Item 四列生命周期入口，并且 Story / Design 侧已经接入 Workspace、Provider 执行、Timeline、Artifact、Review 与人工确认。Work Item 侧仍停留在“生成并确认计划”的阶段：用户确认 Work Item 后，后端只把 `WorkItemPlanStatus` 更新为 `confirmed`，没有进入真实代码修改、测试、代码审查、返工、提交分支和最终确认。

现有代码里已经出现了部分执行闭环的基础能力：

| 能力 | 当前状态 | 结论 |
|---|---|---|
| `CodingWorkspaceStage` | 已在 `src/web/workspace_ws_types.rs` 定义，但没有主流程接入 | 可作为概念起点，但阶段不完整 |
| `LifecycleWorkItemRecord.execution_status` | 已存在 | 可承载 Work Item 执行状态 |
| `LifecycleWorkItemRecord.worktree_path` | 已存在 | 可记录 attempt worktree |
| `TaskRunOrchestrator` | 已有非交互式 planning/execution/final chain | 可复用 runtime unit、provider contract、artifact/report 结构 |
| Work Item confirm | 当前只确认 plan | 需要新增 Coding 入口与执行 engine |

本方案解决的问题是：Work Item 的 Plan confirmed 后，用户能在 Product Workbench 中启动真实 Coding Workspace，在隔离 worktree 中完成代码修改、后端真实测试、内部代码审查、自动返工、创建 review branch / review request、内部 PR review 和最终确认。

---

## 二、目标与非目标

### 2.1 目标

1. **补齐 Work Item confirmed 后的入口**：Work Item Plan confirmed 后，在卡片、Drawer 和 Workspace 中提供明确的“开始 Coding”入口。
2. **提供浏览器内完整 Coding Workspace**：用户不需要跳出 Product Workbench，就能看到 coding、testing、code review、rework、review request、internal PR review、final confirm 的状态与证据。
3. **每次 coding attempt 使用独立 git worktree**：不直接修改仓库主工作区，attempt 有独立 branch、worktree path、commit、测试报告和 review 记录。
4. **测试证据以后端真实命令为准**：Provider 声称“测试通过”不作为通过依据；必须由后端在 worktree 中执行命令并记录 stdout/stderr、exit code、耗时和结果。
5. **Review Request 不绑定 GitHub**：优先使用 Git 原生命令完成 branch、commit、push；识别 GitLab 并支持 push option 时尝试自动创建 MR；否则降级为“review branch 已推送 + 内部 review + 手动创建 PR/MR 指引”。
6. **支持自动 rework**：coding / testing / code review / internal PR review 发现问题时，自动返工最多 2 轮；超过后暂停并交给用户决策。
7. **不自动 merge**：第一版只创建或更新 review branch / review request，并在 Aria 内部完成 review；最终是否 merge 由用户或外部平台决定。

### 2.2 非目标

1. 第一版不自动 merge 到目标分支。
2. 第一版不删除 attempt worktree，避免误删调试证据。
3. 第一版不把 Aria 内部 review 评论同步到 GitHub / GitLab 的逐行评论。
4. 第一版不要求外部平台 API token；如果没有 GitHub/GitLab CLI 或平台 API，仍可通过 Git 原生命令完成 review branch 推送。
5. 第一版不把 `aria task run` 直接包装成 Web UI，而是新增独立 Coding Workspace Engine，复用底层能力。

---

## 三、已批准产品决策

| # | 决策点 | 选择 |
|---|---|---|
| 1 | 总体方案 | 方案 3：新增独立 Coding Workspace Engine，复用共享底层能力 |
| 2 | 入口时机 | Work Item Plan confirmed 后出现“开始 Coding” |
| 3 | 执行链路 | `coding -> testing -> code_review -> rework -> review_request -> internal_pr_review -> final_confirm` |
| 4 | 工作区隔离 | 每个 Work Item coding attempt 创建独立 git worktree |
| 5 | Git 集成 | 自动创建/更新 branch、commit、push；不自动 merge |
| 6 | PR/MR 抽象 | 使用 Git 原生命令为基础，按远端能力尝试 GitLab push option，无法自动创建时提供手动 review request 指引 |
| 7 | PR review 范围 | 第一版做 Aria 内部 review，不同步 GitHub/GitLab 评论 |
| 8 | rework 上限 | 自动 rework 最多 2 轮，超过后暂停给用户决策 |
| 9 | 测试可信度 | 后端真实执行命令的结果才是测试证据 |
| 10 | 清理策略 | 第一版不自动删除 worktree |

---

## 四、总体架构

### 4.1 双 Workspace Engine

新增 Coding Workspace Engine，与当前 Document Workspace Engine 并列：

| Engine | 面向实体 | 主产物 | 主流程 |
|---|---|---|---|
| Document Workspace Engine | Story / Design / Work Item Plan | Markdown artifact / Spec version | prepare context、author、review、revision、confirm |
| Coding Workspace Engine | Work Item Execution | code diff、test report、review request、final summary | worktree、coding、testing、code review、rework、review request、internal PR review、final confirm |

两者共享以下能力：

- Provider registry 与 provider config snapshot。
- Timeline / node detail / artifact 持久化模型。
- WebSocket session 管理、snapshot 恢复和事件推送。
- Runtime unit / provider contract 中已经稳定的输入输出约定。
- LifecycleStore 的 project、issue、story、design、work item 上下文。

两者不共享以下业务编排：

- Document Workspace 不负责 git worktree、branch、commit、push。
- Coding Workspace 不产出 Story/Design markdown version。
- Work Item Plan confirmed 不再等同于 Work Item execution completed。

### 4.2 分层设计

| 层级 | 新增/调整 | 职责 |
|---|---|---|
| 前端页面层 | Coding Workspace view | 展示 coding attempt 状态、Timeline、Artifacts、人工 Gate |
| 前端状态层 | coding workspace store | 管理 attempt snapshot、WS events、selected node、tab、用户动作 |
| Web API 层 | coding attempt API | 创建 attempt、启动/暂停/继续、确认、请求返工、获取 snapshot |
| WebSocket 层 | coding workspace WS | 推送阶段变化、Timeline 节点、测试输出、review 结果、permission 请求 |
| Engine 层 | CodingWorkspaceEngine | 编排 worktree、provider coding、后端测试、review、rework、review request、final confirm |
| Git 层 | GitWorkspaceService | 封装 `git worktree`、branch、status、commit、push、remote capability |
| 证据层 | CodingEvidenceStore | 持久化测试报告、review 报告、diff、commit、stdout/stderr 引用 |

### 4.3 为什么不直接包装 `aria task run`

`TaskRunOrchestrator` 已有非交互式真实链路，但它面向 CLI task run，一次性跑完整 planning/execution/final，缺少 Product Workbench 需要的状态与交互能力：

1. 浏览器内需要可恢复的 attempt snapshot、Timeline node detail 和人工 Gate。
2. Work Item 已经有 confirmed plan，不需要重新跑完整 planning。
3. Coding Workspace 需要独立管理 review branch、review request、internal PR review。
4. 测试失败和 review 失败要进入可见 rework loop，而不是只返回最终 blocked report。

因此本方案复用其底层 runtime unit、provider contract、artifact/report 格式，但新增面向 Web 的 Coding Workspace Engine。

---

## 五、核心状态与数据模型

### 5.1 CodingExecutionAttempt

每次点击“开始 Coding”创建一个 attempt。一个 Work Item 可以有多个 attempt，但同一时间只允许一个 active attempt。

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | string | attempt id |
| `project_id` | string | 项目 id |
| `issue_id` | string | issue id |
| `work_item_id` | string | Work Item id |
| `attempt_no` | number | Work Item 内递增序号 |
| `status` | enum | `created`、`running`、`waiting_for_human`、`blocked`、`completed`、`failed`、`aborted` |
| `stage` | enum | 当前 coding stage |
| `base_branch` | string | 从哪个分支创建 worktree |
| `branch_name` | string | attempt 分支名 |
| `worktree_path` | path | 独立 worktree 路径 |
| `provider_config_snapshot` | object | 本次执行锁定的 provider 配置 |
| `rework_count` | number | 已自动返工次数 |
| `max_auto_rework` | number | 默认 2 |
| `head_commit` | string/null | 当前 attempt commit |
| `pushed_remote` | string/null | 已 push 的 remote |
| `review_request_id` | string/null | ReviewRequest 引用 |
| `created_at` / `updated_at` / `completed_at` | string | 审计时间 |

`LifecycleWorkItemRecord.execution_status` 与最新 attempt status 保持同步；`LifecycleWorkItemRecord.worktree_path` 指向最新 active 或 completed attempt 的 worktree。

### 5.2 CodingWorkspaceStage

现有 `CodingWorkspaceStage` 需要扩展，不能原样作为最终状态机：

| 阶段 | 含义 | 自动/人工 |
|---|---|---|
| `prepare_context` | 展示计划、依赖、provider snapshot、目标 repo | 人工启动 |
| `worktree_prepare` | 创建/校验独立 worktree 与 branch | 自动 |
| `coding` | Provider 根据 Work Item plan 修改代码 | 自动，可触发 permission |
| `testing` | 后端执行真实测试命令并生成 TestingReport | 自动 |
| `code_review` | Aria 对 diff、测试证据、需求覆盖做内部代码审查 | 自动 |
| `rework` | 根据测试或 review 问题进行返工 | 自动，最多 2 轮 |
| `review_request` | commit、push、创建或降级生成 review request | 自动 + 可人工补救 |
| `internal_pr_review` | Aria 基于 review branch 做内部 PR review | 自动 |
| `final_confirm` | 用户确认完成或要求继续返工 | 人工 |

> **注意**：`stage` 只表示当前正在执行的阶段。终态（completed、blocked、aborted）完全由 `CodingExecutionAttempt.status` 表达。当 attempt 进入终态时，`stage` 保持最后一个执行阶段的值（如 `final_confirm`）。

> **现有 `CodingWorkspaceStage` 处置**：`src/web/workspace_ws_types.rs` 中现有的 `CodingWorkspaceStage`（包含 PlanGeneration、PlanConfirm 等）是早期草案，将在实现时被新的 stage 定义替换。旧枚举标记为 deprecated 并在迁移完成后删除。

### 5.3 CodingTimelineNode

Timeline 是用户判断真实进展和证据的事实源。每个阶段至少创建一个 node，长阶段可以追加 detail event。

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | string | node id |
| `attempt_id` | string | attempt id |
| `stage` | enum | 所属 coding stage |
| `title` | string | 展示标题 |
| `status` | enum | `pending`、`running`、`completed`、`failed`、`blocked` |
| `agent_role` | enum/null | `author`、`tester`、`reviewer`、`git`、`system` |
| `summary` | string/null | 结果摘要 |
| `started_at` / `completed_at` | string | 时间 |
| `artifact_refs` | string[] | diff、report、commit、review 的证据引用 |

Node detail 按节点持久化，包含：

- provider streaming snapshot。
- execution events。
- permission requests / responses。
- backend command stdout/stderr 引用。
- diff summary。
- testing report。
- review findings。
- branch / commit / push 结果。

### 5.4 TestingReport

TestingReport 必须区分 provider 声称与后端验证。

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | string | report id |
| `attempt_id` | string | attempt id |
| `commands` | array | 后端执行的测试命令列表 |
| `overall_status` | enum | `passed`、`failed`、`skipped_by_user_decision`、`blocked` |
| `provider_claim` | object/null | provider 声称执行过的测试，仅作参考 |
| `backend_verified` | boolean | 是否由后端真实执行 |
| `started_at` / `completed_at` | string | 时间 |

每条命令记录：

| 字段 | 类型 | 说明 |
|---|---|---|
| `command` | string[] | 使用 argv 数组，不用 shell 拼接 |
| `cwd` | path | attempt worktree |
| `exit_code` | number/null | 退出码 |
| `duration_ms` | number | 耗时 |
| `stdout_ref` / `stderr_ref` | string | 输出文件引用 |
| `status` | enum | `passed`、`failed`、`timed_out`、`blocked` |

### 5.4.1 CodeReviewReport

基础 code review 是 coding / testing 后、review branch 创建前的内部审查结果，必须单独持久化，不能只作为临时 WebSocket 消息存在。否则刷新或重连后，前端 Review Tab 无法恢复 Code Review 分组，也无法把 request changes 的证据带入 rework。

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | string | code review report id |
| `attempt_id` | string | attempt id |
| `round` | number | 第几轮 code review |
| `verdict` | enum | `approve`、`request_changes`、`blocked` |
| `findings` | array | 按严重级别排序的问题 |
| `tested_evidence_refs` | string[] | 使用的 TestingReport / command refs |
| `diff_refs` | string[] | 使用的 diff refs |
| `summary` | string | 审查摘要 |
| `created_at` | string | 时间 |

### 5.5 ReviewRequest

ReviewRequest 是平台无关抽象，不等同于 GitHub PR。

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | string | review request id |
| `attempt_id` | string | attempt id |
| `kind` | enum | `git_branch_only`、`gitlab_merge_request`、`github_pull_request`、`manual_external_request` |
| `remote_kind` | enum | `github`、`gitlab`、`generic_git`、`unknown` |
| `remote` | string | remote 名称 |
| `base_branch` | string | 目标分支 |
| `branch_name` | string | review branch |
| `commit_sha` | string | pushed commit |
| `push_status` | enum | `not_pushed`、`pushed`、`failed` |
| `external_url` | string/null | PR/MR URL；没有则为空 |
| `manual_instructions` | string[] | 需要用户手动创建时的指引 |
| `created_at` / `updated_at` | string | 审计时间 |

### 5.6 InternalPrReview

Internal PR Review 不依赖外部平台评论，是 Aria 内部对 review branch 的最终审查。

| 字段 | 类型 | 说明 |
|---|---|---|
| `id` | string | review id |
| `attempt_id` | string | attempt id |
| `review_request_id` | string | review request id |
| `verdict` | enum | `approve`、`request_changes`、`blocked` |
| `findings` | array | 按严重级别排序的问题 |
| `tested_evidence_refs` | string[] | 使用的 TestingReport / command refs |
| `diff_refs` | string[] | 使用的 diff refs |
| `summary` | string | 审查摘要 |
| `created_at` | string | 时间 |

Finding 字段包含 `severity`、`file_path`、`line`、`message`、`required_action`、`source_stage`。若 `verdict=request_changes`，进入 rework；若超过自动 rework 上限，进入 `blocked` 并要求用户选择继续返工、接受风险或中止。

---

## 六、状态机与执行流

### 6.1 正常路径

1. 用户在 Work Item Plan confirmed 后点击“开始 Coding”。
2. 后端创建 `CodingExecutionAttempt`，锁定 provider snapshot。
3. `worktree_prepare`：从 base branch 创建 attempt branch 和独立 worktree。
4. `coding`：Provider 根据 Work Item plan、Story、Design、repo context 修改代码。
5. `testing`：后端执行 repo 推断或用户配置的真实测试命令。
6. `code_review`：Aria 内部审查 diff 与测试证据。
7. 如果测试和 review 均通过，进入 `review_request`。
8. `review_request`：后端检查 git status，生成 commit，push branch，尝试创建 MR 或输出手动指引。
9. `internal_pr_review`：Aria 基于 pushed branch、commit、diff、测试报告做内部 PR review。
10. `final_confirm`：用户看到最终摘要、测试报告、review verdict、branch/commit/URL 后确认完成。
11. `completed`：更新 Work Item execution status，保留 worktree 和证据。

### 6.2 返工路径

测试失败、code review 要求修改、internal PR review 要求修改时：

1. Engine 记录失败原因和证据引用。
2. 如果 `rework_count < max_auto_rework`，进入 `rework`。
3. Provider 根据失败证据修改代码。
4. 回到 `testing`，再次真实执行测试。
5. 超过 2 轮仍未通过，进入 `blocked`，用户可选择：
   - 继续自动返工一轮。
   - 暂停并手动处理 worktree。
   - 放弃 attempt。

### 6.3 错误处理

| 错误 | 阶段 | 行为 |
|---|---|---|
| repo path 不存在或不是 git repo | `worktree_prepare` | attempt `blocked`，提示用户修复 repository 配置 |
| base branch 不存在 | `worktree_prepare` | attempt `blocked`，允许用户选择 base branch |
| worktree 已存在但不干净 | `worktree_prepare` | 不覆盖，生成新的 attempt path 或要求用户处理 |
| provider 不可用 | `coding` / `rework` / `review` | attempt `blocked`，保留已完成证据 |
| permission 请求未响应 | provider 阶段 | 前端显示人工 Gate，超时不自动批准 |
| 测试命令超时 | `testing` | TestingReport 标记 `timed_out`，进入 rework 或 blocked |
| git commit 时无变更 | `review_request` | 如果测试和 review 通过但无 diff，标记 blocked，提示“未产生代码变更” |
| push 失败 | `review_request` | 保留本地 commit，展示可复制的 git 命令和错误输出 |
| GitLab push option 不支持 | `review_request` | 降级为 branch-only review request |

---

## 七、前端交互设计

### 7.1 Work Item 卡片与 Drawer

Work Item 卡片展示五类状态：

| 状态 | 主按钮 | 辅助信息 |
|---|---|---|
| Plan 未确认 | `打开 Plan Workspace` | 提示需先确认 Plan |
| Plan 已确认且无 active attempt | `开始 Coding` | 展示上次 attempt 结果或“尚未执行” |
| attempt 运行中 | `进入 Coding Workspace` | 展示当前 stage、耗时、active node |
| attempt blocked | `处理 Blocker` | 展示 blocker 摘要 |
| attempt completed | `查看结果` / `再次 Coding` | 展示 branch、commit、测试状态、review verdict |

Drawer 中新增 Coding 区块：

- 当前 execution status。
- 最新 attempt 编号、branch、worktree path。
- 测试结果摘要。
- review request 状态。
- 进入 Coding Workspace 的入口。

### 7.2 Coding Workspace 页面布局

页面沿用 Workspace 全屏形态，但内容面向代码执行：

| 区域 | 内容 |
|---|---|
| Header | Work Item 标题、attempt status、stage、provider snapshot、branch、worktree path |
| 左侧 Timeline | worktree、coding、testing、review、rework、review request、internal PR review、final confirm 节点 |
| 右侧 Artifact tabs | `Diff`、`Tests`、`Review`、`Git`、`Logs` |
| 底部/右侧 Gate 面板 | permission response、blocked 决策、final confirm |

Artifact tabs 行为：

- `Diff`：展示当前 attempt 相对 base branch 的 summary 和文件列表。
- `Tests`：展示后端真实执行命令、状态、耗时、stdout/stderr 链接。
- `Review`：展示 code review 与 internal PR review findings。
- `Git`：展示 branch、commit、push、review request URL 或手动创建指引。
- `Logs`：展示 provider streaming、execution events 和系统错误。

### 7.3 人工节点

第一版需要三类人工节点：

1. **Permission Gate**：provider 要执行高风险工具时，用户批准或拒绝。
2. **Blocked Gate**：自动 rework 超限、push 失败、repo 配置缺失等问题需要用户处理。
3. **Final Confirm Gate**：用户确认 attempt 完成，或要求继续返工。

人工节点必须写入 Timeline，刷新后可恢复。

---

## 八、后端 API 与 WebSocket

### 8.1 REST API

新增或扩展 API：

| 方法 | 路径 | 职责 |
|---|---|---|
| `POST` | `/api/projects/:project_id/issues/:issue_id/work-items/:work_item_id/coding-attempts` | 创建 attempt |
| `POST` | `/api/coding-attempts/:attempt_id/start` | 启动执行 |
| `POST` | `/api/coding-attempts/:attempt_id/abort` | 中止 attempt |
| `POST` | `/api/coding-attempts/:attempt_id/rework` | 用户要求继续返工 |
| `POST` | `/api/coding-attempts/:attempt_id/final-confirm` | 最终确认 |
| `GET` | `/api/coding-attempts/:attempt_id` | 获取 snapshot |
| `GET` | `/api/coding-attempts/:attempt_id/artifacts/:artifact_id` | 获取证据文件 |

创建 attempt 前必须校验：

- Work Item 存在。
- Work Item `plan_status=confirmed`。
- Story / Design 依赖仍可解析。
- Repository 配置存在且指向 git repo。
- 当前 Work Item 没有 running attempt。

### 8.2 WebSocket 消息

Coding Workspace 使用独立的 WebSocket endpoint，不与 Document Workspace 共享连接：

**Endpoint**：`/ws/coding-attempts/:attempt_id`

> **设计决策**：由于 Coding Workspace 与 Document Workspace 的状态机和消息协议差异很大，使用独立 endpoint 避免共享连接带来的路由复杂度。attempt_id 本身即为 session 标识，无需额外 session 概念。

Server -> Client：

| 消息 | 用途 |
|---|---|
| `coding_session_state` | 首次连接和重连 snapshot |
| `coding_stage_change` | stage 变化 |
| `coding_timeline_node_created` | 新节点 |
| `coding_timeline_node_updated` | 节点状态变化 |
| `coding_execution_event` | 后端命令、provider event、git event |
| `testing_report_update` | 测试报告更新 |
| `code_review_complete` | code review 完成 |
| `review_request_update` | branch/commit/push/MR 状态 |
| `internal_pr_review_complete` | internal PR review 完成 |
| `coding_gate_required` | permission、blocked、final confirm；blocked gate 的可用操作由后端根据 blocked 原因动态下发 |
| `protocol_error` | 非法阶段动作或字段错误 |

Client -> Server：

| 消息 | 用途 |
|---|---|
| `start_coding` | 启动 attempt |
| `permission_response` | 响应 provider 权限 |
| `continue_rework` | 用户要求超过上限继续返工 |
| `abort_attempt` | 中止 attempt |
| `final_confirm` | 最终确认 |
| `request_manual_pause` | 暂停给用户手动处理 |

### 8.3 Snapshot 恢复

刷新或重连后，snapshot 必须恢复：

- attempt 基础信息。
- 当前 stage 和 active node。
- Timeline nodes 与 node details。
- TestingReport。
- CodeReviewReport。
- ReviewRequest。
- InternalPrReview。
- worktree path、branch、commit、push status。
- 所有等待中的人工 Gate。

---

## 九、Git / Worktree / Review Request 策略

### 9.1 Worktree 创建

默认分支命名：

```text
aria/work-items/<work_item_id>/attempt-<attempt_no>
```

默认 worktree 路径：

```text
<repo_root>/.worktrees/aria-work-items/<work_item_id>/attempt-<attempt_no>
```

如果路径已存在且不是当前 attempt 持有，创建新的后缀路径：

```text
attempt-<attempt_no>-<short_attempt_id>
```

所有 Git 操作通过 argv 调用，不拼 shell 字符串。路径必须限制在 repository root 或 `.worktrees/aria-work-items/` 下。

### 9.2 Commit 策略

进入 `review_request` 前：

1. 执行 `git status --porcelain`。
2. 如果无变更，attempt blocked。
3. 如果有变更，执行 `git add`。
4. commit message 使用结构化模板：

```text
work-item: <title>

Work Item: <work_item_id>
Attempt: <attempt_id>
Issue: <issue_id>

Generated-by: Aria Coding Workspace
```

第一版每个 attempt 默认生成一个 commit。返工发生在同一 attempt 中时，进入 review_request 前 squash 为最终 commit；如果已 push 后再返工，则允许 amend 并 force-with-lease push attempt branch。

### 9.3 Push 与 Review Request

Push 顺序：

1. 识别 remote：优先使用 repository 配置，否则使用 `origin`。
2. 执行普通 `git push -u <remote> <branch>`。
3. 识别 remote URL：
   - GitLab：支持 push option 时尝试 `merge_request.create`。
   - GitHub：第一版不依赖 `gh`，如果环境可用可作为增强能力，但不是必需。
   - 其他 Git：记录 branch-only review request。
4. 如果自动创建 PR/MR 失败，不回滚 commit / push，降级输出手动创建指引。

GitLab push option 尝试属于 best effort。失败时 ReviewRequest `kind=git_branch_only`，`manual_instructions` 包含 base branch、source branch、commit sha、remote URL。

---

## 十、Provider 与 Rework 策略

### 10.1 Provider 输入

Coding 阶段 provider 输入应包含：

- Work Item confirmed plan。
- 关联 Story Spec 和 Design Spec 最新 confirmed version。
- Repository path、base branch、attempt branch、worktree path。
- 项目规则摘要：必须读取 worktree 内 `AGENTS.md`、`CLAUDE.md`、`.claude/rules/` 与 `cadence/project-rules/README.md`。
- 测试要求和可用命令。
- 明确约束：不要修改无关文件，不要自动 merge，不要删除 worktree。

### 10.2 测试命令发现

第一版按优先级选择测试命令：

1. Work Item plan 中明确列出的测试命令。
2. Repository 配置中的测试命令。
3. 项目类型推断：
   - Rust：`cargo fmt --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo check --locked`、`cargo test --locked -j 1`
   - Python：优先 `uv run pytest`，无 pytest 配置时按项目上下文降级
   - Node：优先 `pnpm test`，存在 lint/typecheck 时一并执行
4. 无法推断时进入 blocked，要求用户提供测试命令。

在 cadence-aria 仓库自身开发时，遵循当前已启用项目规则：本地开发、测试与 CLI 验证直接使用宿主机 Rust/Cargo，不使用 Docker 作为默认环境。

### 10.3 自动 Rework 输入

Rework provider 输入必须带上失败证据，而不是只给结论：

- 失败测试命令 argv、exit code、stdout/stderr 摘要与引用。
- code review findings。
- internal PR review findings。
- 当前 diff summary。
- rework_count 与剩余自动返工次数。

Provider 完成 rework 后必须回到后端真实 testing。

---

## 十一、安全边界

1. **不使用 shell 拼接执行 Git/测试命令**：后端使用 argv 数组执行，避免命令注入。
2. **路径约束**：worktree path、artifact path、stdout/stderr path 必须位于允许目录。
3. **权限 Gate**：provider 请求高风险工具时必须经过用户批准。
4. **不自动 merge**：任何 merge/rebase 到 base branch 的行为第一版禁止。
5. **不自动删除 worktree**：避免清理过程误删用户证据。
6. **不信任 provider 的测试结论**：测试状态只看后端真实命令结果。
7. **保留审计证据**：所有人工决策、provider 输出、测试命令、git 操作都写入 Timeline。

---

## 十二、测试策略与验收标准

### 12.1 单元测试

覆盖：

- Work Item 未 confirmed 时不能创建 coding attempt。
- 同一 Work Item 同时只能有一个 active attempt。
- Coding stage 状态转换合法性。
- rework 次数上限。
- ReviewRequest remote kind 识别和降级逻辑。
- TestingReport 不接受 provider claim 作为 backend verified。
- GitWorkspaceService 使用 argv，不使用 shell 字符串。

### 12.2 集成测试

覆盖：

- 创建 attempt 后生成 branch 和 worktree。
- provider fake coding 修改文件后，后端真实测试通过。
- 测试失败触发 rework，rework 后重新测试。
- code review request changes 触发 rework。
- push 失败时 attempt blocked，并保留 commit 和手动指引。
- snapshot 重连后恢复 Timeline、TestingReport、ReviewRequest 和人工 Gate。

### 12.3 前端测试

覆盖：

- Plan 未 confirmed 时没有“开始 Coding”。
- Plan confirmed 后出现“开始 Coding”。
- attempt running 时按钮变为“进入 Coding Workspace”。
- Timeline 阶段和 Artifact tabs 渲染正确。
- Testing tab 展示后端真实命令结果。
- blocked gate 和 final confirm gate 可操作。

### 12.4 真实 E2E 验收用例

使用用户指定仓库：

```text
/home/michael/workspace/github/naruto
```

测试案例：

- Work Item：爬楼梯。
- 需求：写 Python 程序解决爬楼梯，每次能走 1 或 2 步，问走到第 n 步有几种走法。
- 复杂度：O(n)。
- 测试用例：
  - `n=1 -> 1`
  - `n=2 -> 2`
  - `n=3 -> 3`
  - `n=5 -> 8`
  - `n=10 -> 89`

验收标准：

1. Product Workbench 中 Work Item Plan confirmed 后可见“开始 Coding”入口。
2. 点击后创建独立 worktree，不修改 `/home/michael/workspace/github/naruto` 主工作区。
3. Coding Workspace Timeline 展示 worktree、coding、testing、review、review request、internal PR review、final confirm。
4. 后端真实执行 Python 测试命令并记录 TestingReport。
5. Provider claim 与 backend verified 分开展示。
6. 代码审查通过后创建 commit 并 push review branch。
7. 如果无法自动创建 PR/MR，页面展示 review branch、commit sha 和手动创建指引。
8. 用户最终确认后 Work Item execution status 更新为 completed。

---

## 十三、分期建议

### P0：打通最小真实闭环

- 数据模型与 store：`CodingExecutionAttempt`、`TestingReport`、`ReviewRequest`。
- Work Item confirmed 后入口。
- 独立 worktree 创建。
- fake provider 或现有 provider contract 驱动 coding。
- 后端真实测试。
- 基础 code review。
- commit + push branch-only ReviewRequest。
- Final Confirm。
- 使用 `naruto` 爬楼梯完成真实 E2E。

### P1：补齐返工与内部 PR Review

- 自动 rework 最多 2 轮。
- InternalPrReview verdict 和 findings。
- blocked gate。
- Snapshot 恢复完整 node detail。
- GitLab push option best effort MR 创建。

### P2：产品化增强

- 外部 PR/MR URL 更丰富的识别。
- Review branch diff 可视化增强。
- Worktree 清理策略和保留策略。
- 外部平台评论同步，作为明确的后续能力，而不是第一版默认行为。

---

## 十四、风险与应对

| 风险 | 影响 | 应对 |
|---|---|---|
| 不同仓库测试命令差异大 | E2E 容易 blocked | 先支持用户/Work Item 显式测试命令，再做语言推断 |
| GitLab/GitHub/自建 Git 行为不一致 | PR/MR 自动创建不稳定 | ReviewRequest 抽象以 branch-only 为可靠兜底 |
| Provider 修改范围过大 | 引入无关变更 | coding 输入明确 scope；review 检查 diff；final confirm 前展示文件列表 |
| 长时间运行断连 | 用户看不到状态 | attempt 状态持久化，WS 重连恢复 snapshot |
| rework 循环耗时失控 | 成本和时间不可控 | 默认最多 2 轮，超过进入人工 Gate |
| worktree 残留过多 | 占用磁盘 | 第一版保留证据，后续提供显式清理入口 |

---

## 十五、实施触达点

预估触达模块：

| 模块 | 变更 |
|---|---|
| `src/product/models.rs` | 新增 coding attempt、testing report、review request、internal review 模型 |
| `src/product/lifecycle_store.rs` | 新增持久化读写 API |
| `src/product/workspace_engine.rs` | 保持 Document Workspace；Work Item confirm 后不直接进入 execution completed |
| `src/product/coding_workspace_engine.rs` | 新增 Coding Workspace Engine |
| `src/product/git_workspace_service.rs` | 新增 git/worktree/branch/commit/push 封装 |
| `src/web/workspace_ws_types.rs` | 扩展 coding stage 与 WS message |
| `src/web/handlers.rs` | 新增 coding attempt REST API |
| `web/src/pages/WorkspacePage.tsx` | 按 workspace type 分流 Document / Coding 视图 |
| `web/src/state/*` | 新增 coding workspace store |
| `web/src/components/*` | Work Item card/drawer 入口、Coding Timeline、Artifact tabs、Gate 面板 |
| `tests/*` | 单元、集成和 E2E 覆盖 |

---

## 十六、完成定义

本方案进入实现前，必须满足：

1. 本技术方案已提交并经用户确认。
2. 后续实现计划拆分为可验证的小步任务。
3. 每个实现阶段都先写测试或验收脚本，再写实现。
4. 首个端到端验收以 `naruto` 爬楼梯真实仓库为准。
5. 完成后汇报实际执行过的测试命令和结果，不以“应该可以”作为完成依据。
