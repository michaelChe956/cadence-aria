# Coding Workspace 角色重跑与可读进度气泡技术方案

## 背景

当前 Coding Workspace 已支持 Coder、Tester、Analyst、Code Reviewer、Internal Reviewer 的分阶段执行，并已引入 Provider 驱动测试、Analyst 路由决策和 blocked gate 恢复动作。但真实 E2E 暴露了三个问题：

- Tester 在 `plan_tests` 阶段可能停留在 provider 的 task/tool update，用户只能看到零散执行事件，看不到测试计划和执行进度。
- 为保证后端可解析，Tester 和 Analyst prompt 被强化为 JSON-only 后，Provider 主气泡更难承载用户可读过程。
- Tester、Reviewer、Analyst 失败后虽然已有局部 retry action，但重跑语义不完整，旧报告、旧决策、旧消息和新一轮输出容易混在一起。

本方案目标是把“机器可解析契约”和“用户可读过程”拆开，同时提供统一的角色阶段重跑模型。

## 目标

1. Tester 失败、卡住、超时或 blocked 后，用户可以回退到 Testing 执行前，重新执行 Tester。
2. Code Reviewer 失败或 blocked 后，用户可以回退到 CodeReview 执行前，重新执行 Code Reviewer。
3. Analyst 输出无效 JSON、决策 blocked、人工放行后仍需复核时，用户可以回退到 Rework 执行前，重新执行 Analyst。
4. 每一次重跑都形成独立 run，不覆盖旧证据；默认 UI 展示当前 run，旧 run 可追溯。
5. Tester、Analyst、Code Reviewer、Internal Reviewer 的消息气泡显示计划、进度、结论和证据，而不是直接展示大段 raw JSON。
6. `plan_tests` 阶段具备后端超时和可诊断 blocked 结果，避免无限停留在 task update。

## 非目标

- 不改变 Coder 对目标仓库的实现逻辑。
- 不删除历史报告、历史 raw provider output 或旧 timeline 节点。
- 不把 Provider prompt 改回自由文本输出；结构化 JSON 仍是后端解析契约。
- 不在本方案内设计新的多 attempt 分支策略；本方案限定在同一个 coding attempt 内做阶段 run 隔离。

## 现状判断

当前 Tester 的 `plan_tests` 使用通用 provider stream 等待 `Completed`，而不是像 execute 阶段一样有独立的 `TesterAgentOptions.timeout` 保护。若 provider 长时间只发 task/tool/execution event，不发最终结果，后端会一直等待，用户侧最后看到的只是最近一条 task update。

当前前端按 node 分组消息。若某个 node 没有 `provider_stream` 主消息，只会显示折叠的 `execution_event` 行；这解释了“Tester 气泡没有输出”的体验。Code Reviewer 和 Internal Reviewer 已在解析后 emit summary 气泡，Analyst 已有 verdict 气泡，但三者仍缺少统一的“运行中进度”和“当前 run/历史 run”边界。

## 设计原则

- 结构化输出给系统解析，可读摘要给用户阅读，两者分离。
- 重跑不销毁历史，只标记新旧关系。
- 阶段重跑必须从角色阶段入口重新执行，而不是在失败点继续拼接。
- UI 默认聚焦当前 run，历史 run 保留可展开入口。
- blocked 原因必须可诊断：区分测试失败、计划解析失败、provider 超时、用户 abort、权限等待和人工质量豁免。

## 方案选型

### 方案 A：仅优化 prompt 和前端渲染

要求 provider 在 JSON 外再输出说明，前端解析 markdown 或 JSON 展示。

优点是改动小。缺点是会破坏 JSON-only 契约，容易再次触发解析失败；也无法解决重跑时历史结果混杂的问题。

### 方案 B：新增阶段 run 快照，后端生成可读气泡

每次 Tester、Analyst、Code Reviewer、Internal Reviewer 执行时创建 role run。Provider 继续输出结构化 JSON，后端解析后生成用户可读 chat entry。重跑时新建 role run，旧 run 标记为 superseded。

优点是契约清晰、历史可追溯、UI 可稳定展示，能同时解决卡住诊断和重跑隔离。缺点是需要修改后端模型、store、WS 快照和前端渲染。

### 方案 C：每次重跑都新建 coding attempt

失败后关闭当前 attempt，创建一个全新 attempt 重新走流程。

优点是隔离彻底。缺点是用户要在多个 attempt 间跳转，历史上下文、worktree、review request 和当前 UI 都会变复杂；对于只想重跑 Tester 或 Reviewer 的场景过重。

推荐采用方案 B。

## 核心数据模型

新增或扩展一个角色 run 概念，建议命名为 `CodingRoleRun`：

- `id`：如 `coding_role_run_0001`。
- `attempt_id`：所属 coding attempt。
- `stage`：`testing`、`rework`、`code_review`、`internal_pr_review`。
- `role`：`tester`、`analyst`、`code_reviewer`、`internal_reviewer`。
- `run_no`：同一 role/stage 下从 1 递增。
- `status`：`running`、`completed`、`failed`、`blocked`、`superseded`、`aborted`。
- `started_at`、`completed_at`。
- `supersedes_run_id`：当前 run 替代的旧 run。
- `superseded_by_run_id`：旧 run 被哪个新 run 替代。
- `trigger`：`initial`、`retry_test_plan`、`rerun_missing_steps`、`retry_review`、`retry_analyst`、`manual_rerun`。
- `reason_code`：如 `plan_tests_timeout`、`provider_start_failed`、`analyst_parse_error`。
- `raw_provider_output_refs`：本 run 关联 raw 输出。
- `artifact_refs`：本 run 关联 testing report、analyst decision、code review report、internal review。

现有 `TestingReport`、`AnalystDecisionRecord`、`CodeReviewReport`、`InternalPrReview` 增加可选 `role_run_id` 和 `run_no`。前端可以据此筛选当前 run，同时保留历史入口。

## 阶段重跑语义

### Tester 重跑

触发动作：

- `retry_test_plan`：重新执行 `plan_tests` 和 `execute_test_plan`。
- `rerun_missing_steps`：重新执行 Tester；实现上仍从 Tester 阶段入口开始，但 prompt 可带上缺失 step 作为上下文。

行为：

- 关闭当前 blocked gate。
- 将当前 Testing run 标记为 `superseded`。
- attempt status 置为 `running`，stage 置为 `testing`。
- 创建新的 Testing timeline node 和 Tester role run。
- 旧 testing report 保留，新 report 绑定新 `role_run_id`。

### Analyst 重跑

新增动作 `retry_analyst`。

触发场景：

- Analyst 输出不是有效 JSON。
- Analyst 决策进入 human gate，用户认为需要让 Analyst 重新判断。
- Analyst blocked 或 provider 超时。

行为：

- 关闭当前 Analyst gate 或 blocked gate。
- 将当前 Rework/Analyst run 标记为 `superseded`。
- attempt status 置为 `running`，stage 置为 `rework`。
- 使用同一 evidence 重新执行 Analyst。若原 evidence 来自 Testing/CodeReview/InternalReview，应通过持久化 refs 重建，不依赖内存变量。
- 新 Analyst decision 绑定新的 `role_run_id`。

### Code Reviewer 重跑

触发动作沿用或扩展 `retry_review`。

行为：

- 将当前 CodeReview run 标记为 `superseded`。
- attempt status 置为 `running`，stage 置为 `code_review`。
- 重新执行 Code Reviewer。
- 旧 code review report 保留，新 report 绑定新 `role_run_id`。

### Internal Reviewer 重跑

新增动作 `retry_internal_review`，或扩展 `retry_review` 通过 gate stage 区分。

行为：

- 将当前 InternalPrReview run 标记为 `superseded`。
- attempt status 置为 `running`，stage 置为 `internal_pr_review`。
- 复用最新 review request，重新执行 Internal Reviewer。
- 旧 internal review 保留，新 review 绑定新 `role_run_id`。

## Tester plan 阶段超时

`plan_tests` 和 `plan_tests_repair` 必须像 `execute_test_plan` 一样受 `TesterAgentOptions.timeout` 管控。超时后：

- cancel provider session。
- 保存当前已收到的 partial output 或 execution event refs。
- 生成 blocked testing report，`overall_status = blocked`。
- `context_warnings` 包含 `plan_tests_timeout`。
- gate 提供 `retry_test_plan`、`send_raw_output_to_analyst`、`abort`。
- UI 显示“Tester 计划阶段超时”，而不是泛化成测试失败。

如果用户主动 Abort，结果应标记为 `aborted`，不应伪装为 tester blocked。当前 attempt 可根据产品策略保持 `aborted` 或提供重新开始入口；本方案只要求诊断文案准确。

## 用户可读气泡设计

### Tester

Tester 气泡分三类：

- Plan：解析 TestPlan 后由后端生成，显示 summary、required steps、工具/命令、风险等级、证据预期。
- Progress：每个 step 的 tool call/result 转成状态行，显示执行中、通过、失败、blocked、证据 refs。
- Result：TestingReport 生成后显示整体状态、失败 bugs、missing/skipped required steps、raw refs。

raw JSON 默认折叠，仅作为诊断材料，不作为主阅读内容。

### Analyst

Analyst verdict 气泡显示：

- structured verdict。
- next stage。
- reason。
- rework instructions 或 human gate 建议。
- evidence refs 和 raw refs。
- parse error 诊断。

如果 Analyst 当前 run 尚未完成，显示“等待 Analyst 决策”并附最近 execution event；完成后替换为结构化决策卡片。

### Code Reviewer

Code Reviewer 气泡显示：

- verdict。
- summary。
- findings count 和关键 finding。
- tested evidence refs、diff refs、raw ref。
- blocked 时显示可执行恢复动作。

### Internal Reviewer

Internal Reviewer 气泡显示：

- verdict。
- impact scope。
- PR description 摘要。
- commit message suggestion。
- findings 和 raw ref。

### 历史 run 展示

当前 run 默认展开。被 superseded 的旧 run 折叠在“历史执行”区域，标题包含 role、run no、状态、触发动作和完成时间。旧 run 不参与当前状态判断，但保留完整证据链。

## Gate 与交互

Blocked gate 的 action 应按 stage 和 role 精确生成：

- Testing：`重新执行 Tester`、`发送给 Analyst 决策`、`终止`。
- Analyst：`重新执行 Analyst`、`人工放行`、`补充上下文`、`终止`。
- CodeReview：`重新执行 Code Reviewer`、`发送给 Analyst 决策`、`终止`。
- InternalPrReview：`重新执行 Internal Reviewer`、`发送给 Analyst 决策`、`终止`。

前端仍通过 `gate_response` 发送动作，后端根据 gate stage/role 决定回退目标。需要新增 action type：

- `retry_analyst`
- `retry_internal_review`

也可以保留 `retry_review`，但后端必须按 gate stage 区分 CodeReview 与 InternalPrReview。

## 后端改造范围

- `coding_models.rs`：新增 `CodingRoleRun`，扩展 gate action type，给报告/决策增加 `role_run_id`。
- `coding_attempt_store.rs`：保存、读取、supersede、查询 latest role run；session snapshot 带出当前 run 和历史 run 摘要。
- `coding_workspace_engine.rs`：在四类角色执行入口创建 role run；完成、blocked、failed、aborted 时更新 run；gate response 重跑时 supersede 旧 run 并回退到目标 stage。
- `tester_agent_loop.rs`：保留 JSON-only prompt，新增后端可读摘要生成函数，或由 engine 基于 TestPlan/TestingReport 构造 chat entry。
- `coding_ws_handler.rs`：gate response 后恢复 runner 时按新 action 支持 Analyst/Internal Reviewer 重跑。
- provider raw output：路径可继续按 stage 保存，但 metadata 必须带 `role_run_id`，避免历史 raw 混淆。

## 前端改造范围

- API types：同步 `CodingRoleRun`、`role_run_id`、新 gate action type。
- store：保存 role runs，计算每个 role/stage 当前 run。
- chat grouping：按 `node_id + role_run_id` 分组，避免重跑后旧消息和新消息合并。
- entries：新增或扩展 TesterPlan、TesterResult、RoleRunSummary 结构化渲染。
- CodingWorkspacePage：在测试、审查、Analyst 状态区域优先显示当前 run，历史 run 折叠。
- gate 面板：按 action type 显示更明确文案，Analyst 人工放行继续要求 reason。

## 兼容与迁移

旧数据没有 `role_run_id` 时：

- 后端读取时视为 legacy run no 1。
- UI 将旧报告归入“历史执行”或“当前执行”，取决于是否存在新 role run。
- 不需要批量迁移 `.aria` 文件；写入新数据后自然带上 role run。

已有 gate action：

- `retry_test_plan`、`rerun_missing_steps`、`retry_review` 继续兼容。
- 对旧 gate 缺少 role 信息的情况，后端按 gate stage 推断目标 role。

## 测试策略

### 后端单元与集成测试

- Tester plan 阶段 provider 只发 execution event、不发 completed，超时后生成 `plan_tests_timeout` blocked report 和 gate。
- `retry_test_plan` 后旧 Testing run 被 superseded，新 Testing run 生成独立 testing report。
- `retry_analyst` 后旧 Analyst decision 被 superseded，新 decision 成为 latest。
- `retry_review` 在 CodeReview gate 下重跑 Code Reviewer。
- `retry_review` 或 `retry_internal_review` 在 InternalPrReview gate 下重跑 Internal Reviewer。
- JSON-only Tester plan 成功解析后，后端保存可读 plan chat entry。
- Tester tool step result 生成可读 progress/result chat entry。
- Analyst parse error 保存 raw output，并显示可重跑 Analyst 的 gate。

### 前端测试

- Tester plan 气泡显示 summary、steps、risk、evidence expectation。
- Tester result 气泡显示 missing/skipped required steps 和 raw refs。
- Analyst verdict 气泡显示 next stage、reason、parse error、raw refs。
- Code Reviewer/Internal Reviewer 显示 verdict、summary、finding/impact 信息。
- 重跑后当前 run 默认展开，旧 run 折叠为历史执行。
- Gate action 点击发送正确 action id，manual continue 仍强制填写 reason。

### 真实 E2E 验收

- 触发 Tester plan 卡住或超时，页面显示明确 blocked 原因，用户可点击重新执行 Tester。
- 重新执行 Tester 后，旧 Tester 输出进入历史，新 Tester 计划和进度可见。
- 让 Analyst 输出非法 JSON，页面显示 parse error 和 raw ref，用户可重新执行 Analyst。
- 让 Code Reviewer blocked，用户可重新执行 Code Reviewer。
- Internal Reviewer blocked 后可重新执行 Internal Reviewer，并保留 review request 关联。

## 风险与处理

- 数据模型扩展会影响 session snapshot 和前端类型。处理方式：先保持字段可选，兼容旧数据。
- run 分组如果只依赖 node_id，重跑仍可能混合。处理方式：chat entry metadata 必须带 `role_run_id`，前端分组键使用 `node_id + role_run_id`。
- Analyst 重跑需要重建 evidence。处理方式：Analyst gate 创建时保存 source stage、evidence refs 和 raw refs，重跑时从持久化记录恢复。
- 保留历史 run 会增加 UI 信息量。处理方式：默认只展示当前 run，历史折叠。

## 实施顺序建议

1. 建立 `CodingRoleRun` 模型和 store 能力，兼容旧数据。
2. 将 Tester plan/execute 绑定 role run，并补 plan 超时。
3. 生成 Tester 可读 plan/progress/result 气泡。
4. 将 Analyst/CodeReview/InternalReview 绑定 role run，并补重跑 action。
5. 前端按 role run 分组展示当前与历史。
6. 补齐真实 E2E 测试脚本和手工验收步骤。

## 验收标准

- Tester 卡在 task update 时不会无限等待；超时后 blocked 原因准确。
- 用户能从 gate 重新执行 Tester、Analyst、Code Reviewer、Internal Reviewer。
- 重跑后旧 run 不污染当前 run 的报告、决策和消息气泡。
- Tester 的测试计划和执行进度在消息气泡中可读。
- Analyst、Code Reviewer、Internal Reviewer 的结构化结论在消息气泡中可读。
- 自动化测试覆盖后端重跑语义、前端渲染和旧数据兼容。
