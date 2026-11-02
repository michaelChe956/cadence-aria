# Spec Delta

> 基线：`openspec/specs/multi-target-group-coding/spec.md` 现有 REQ-MTG-01 至 REQ-MTG-05（REQ-MTG-06 至 REQ-MTG-10 由未归档 change `fix-local-delivery-and-chain-truth` 新增，编号不重叠）。本 delta 只新增 REQ-MTG-11 至 REQ-MTG-13，不修改既有 requirement；REQ-MTG-03 手动推进与启动门、REQ-MTG-04 聚合只读红线保持不变。

## ADDED Requirements

### Requirement: plan 级聚合最终确认（REQ-MTG-11）

系统 SHALL 提供无 body 的 `POST /api/projects/{project_id}/issues/{issue_id}/work-item-plans/{plan_id}/coding-final-confirm`：对当前 plan-run 冻结 targets（≥2 才呈现聚合按钮）按 target 顺序逐个调用既有且不修改的最终确认函数，跳过已 Completed、首失败即停，返回逐仓结果并明确失败仓。`GET …/coding-start` 的 plan 聚合进度 SHALL 新增 `final_confirm` 四态（not_ready/ready/partial/done，含 ready/confirmed attempt 集与 blocking 原因）与复用 A1 类型的 `attention_items`；GET 只读持久事实，git/owner/范围检查仍由既有确认函数执行；逐 attempt 原确认按钮 SHALL 保留。

#### Scenario: 全齐后一次确认全部

- **WHEN** 多仓全部 attempt 待最终确认且 readiness 完整（GET final_confirm.state=ready），用户点「确认本计划全部完成（M 仓/N 项）」
- **THEN** 恰发出一个 POST，后端按序逐仓确认全部成功，返回 outcome=all_confirmed 与每仓 results（status=confirmed）

#### Scenario: 首失败停止并显示失败仓

- **WHEN** 聚合确认中第 2 个仓确认失败
- **THEN** 第 3 个及之后的仓未被调用（status=not_attempted），响应 outcome=partially_confirmed 且第 2 仓的 error_code/message 可见，用户可重试

#### Scenario: 未齐不出现聚合按钮

- **WHEN** 存在未完成的执行、或剩余 attempt 未全部 WaitingForHuman＋FinalConfirm、或 units 未全 Completed、或 readiness snapshot 不满足身份匹配＋Complete＋diagnostics 空
- **THEN** final_confirm.state 为 not_ready（或 partial 且剩余未就绪），聚合确认不可用，blocking 指明未齐的 attempt 与原因

#### Scenario: 响应形状与幂等边界

- **WHEN** POST 执行完成
- **THEN** 响应为 `{plan_id,outcome:all_confirmed|partially_confirmed|not_ready,results:[{attempt_id,target_repository_id,status:confirmed|already_completed|failed|not_attempted,error_code?,message?}]}`；不加 command_id/generation 防重放（B1-e 不做，已知风险）

### Requirement: 只读 plan 总览与待办直达（REQ-MTG-12）

系统 SHALL 提供只读 plan 总览页 `/workbench/projects/$projectId/issues/$issueId/coding-plans/$planId`：按 order_index 列出全部 WI（逻辑 ID、summary 缺省用 ID、仓库、unit 进度、attempt 状态、依赖/等待原因、待办徽标、详情链接），标题显示执行/确认 N/M。`usePlanCodingProgress` SHALL 统一返回 `{data,loading,error,refetch}` 并支持可选轮询（总览可见时 5 秒）。待办徽标 SHALL 复用同源 attention 事实（A1），点击按 `target+focus{gate_id,choice_id}` 进入对应 attempt 真实详情并可返回；总览页除 plan 聚合确认外 MUST NOT 提供业务写操作。

#### Scenario: 一页列全 WI 并进入真实详情

- **WHEN** 用户打开多仓 plan 的总览页
- **THEN** 全部 WI 按序列出并显示各仓状态与待办徽标；点击某 WI 行进入该 attempt 的真实对话/审查/进度详情（复用既有 WS 单详情切换），返回后仍在总览

#### Scenario: 徽标直达可处理卡片

- **WHEN** 用户点击某 WI 的 choice/gate 待办徽标
- **THEN** 页面导航到对应 attempt 并定位/滚动到该 choice 或 gate 卡片（focus 生效），处理入口可用；多 WI 共一 attempt 时注明 attempt 历史范围，不伪造精确筛选

#### Scenario: 轮询见非当前仓进展

- **WHEN** 总览页保持可见且另一仓的 attempt 状态推进
- **THEN** 5 秒轮询刷新后该仓状态更新，无需手动刷新；加载/失败有明确提示

### Requirement: 仪表盘 attempt 全态状态条（REQ-MTG-13）

coding 仪表盘 SHALL 在保留 unit 拓扑判据不变的前提下新增独立 attempt 状态展示（`selectCodingDashboardAttempts`）：多仓取聚合 target_attempts、单仓取当前 session，按 `{attemptId,repositoryName,status,stage,attentionKinds}` 完整覆盖 11 个 attempt 状态；WaitingForHuman＋FinalConfirm SHALL 显示「等待最终确认」，MUST NOT 统一叫分诊。

#### Scenario: 11 态全覆盖且最终确认不叫分诊

- **WHEN** 仪表盘接收 11 种 attempt 状态的参数化夹具（created/running/waiting_for_human/blocked/awaiting_manual_recovery/awaiting_plan_amendment/applying_plan_amendment/amendment_apply_failed/completed/failed/aborted，含 WaitingForHuman＋FinalConfirm）
- **THEN** 每种状态显示既有 ATTEMPT_STATUS_LABELS/STAGE_LABELS 的正确文案，最终确认态显示「等待最终确认」；unit 已完成与 attempt 待确认可同时呈现

#### Scenario: 单仓显示当前 attempt 且拓扑不变

- **WHEN** 单仓 attempt 的 coding 页渲染仪表盘
- **THEN** 状态条显示当前 session 的 attempt；unit 六态拓扑判据与投影和改前一致
