# Proposal: WorkItem 对话式人工门与 advance 接口（阶段 3）

## Why

阶段 2 单候选 MVP 已收官归档（`openspec/changes/archive/2026-08-31-rearch-workitem-plan-pipeline/`）：auto 模式可零人工到达 Confirmed，但两个断点未解——(1) interactive 人工门只有一次性的 confirm/request-change 决策消息，且 SC 的 manual revision 走 legacy 中文标题约束链被拒（r27-interactive 实证），等于 SC 没有可用的人工返修通道；(2) plan Confirmed 之后没有任何受控接口把计划推进到 coding 执行，用户只能走 legacy 逐段旧协议。阶段 3 补上这两块，并顺手加固 group coding 的依赖顺序门。

## What Changes

- 新增**对话式人工门**：门内多轮 typed 反馈回合（`human_gate_feedback`），每回合服务端分配 `turn_id`、durable 持久化后启动 provider、预算在预留时扣减、单飞（冲突回 `gate_busy`）、幂等重试；`approve`/`abandon`/重连不耗预算；预算耗尽时拒绝新反馈（对齐阶段 1 既有契约），门保持开启仅剩 approve/abandon
- 新增 **auto stopped 接管入口**：`stopped_needs_human` 的 auto session 可由显式操作创建新 interactive session 接管，原 session 事件不可篡改（兑现阶段 1 REQ-TOP-06 延迟项）
- 新增 **SC manual revision 专属路径**：feedback → 独立预算的 SC revision prompt → 完整修订版 markdown → SC compiler → canonical validator；不再触碰 legacy 中文标题约束链
- 新增 **`advance` 接口**：Confirmed plan → WorkItemGroup coding workspace ready 的幂等编排动作（创建/恢复唯一 group attempt + plan binding + worktree lock + coding units），ready-only 不启动 provider；双键幂等（`command_id` + plan 唯一性）
- **加固 group coding 依赖门**（仅限 `admission_kind = sc_advance` 的 attempt）：unit 启动前置检查改为「直接依赖全部 Completed + handoff 已发布 + handoff revision 与当前 plan binding 匹配」，未就绪保持 Pending 不跳过；环/未知依赖/handoff 不匹配 fail-closed；`order_index` 降级为确定性 tie-breaker；legacy（`admission_kind = legacy_group`）保持既有 order_index 行为
- `approve` 语义钉死：approve → 关门 → 确定性 compile → 成功 → Plan durable Confirmed；advance 仅在 durable Confirmed 后可调用，已存在 Failed/Aborted attempt 时返回原状态不隐式重建
- 新增 **Work Item 进展/成果只读投影契约**：group coding workspace 为唯一观察面，每 WI 的状态/成果/失败原因从 group attempt/unit 确定性派生（后端契约，前端 UI 后置）
- 旧 WS 协议与 legacy 入口全部保留（阶段 2 退役门未通过：pi 938.04s > 12min 子项）

非目标：不重构 coding engine 内部、不改 SC author prompt 预算（19,000 红线不动）、不做多仓 coding（阶段 4）、不删除任何旧协议、不让 SC per-WI session 成为第二 coding 执行面、kimi 修复验证与 95% 测量为独立专项不入本 change、**不做 web/ 前端 UI**（对话门界面与 advance 按钮后置；本 change 以后端 + campaign driver 端到端验收，沿用阶段 2 模式）。

## Capabilities

### New Capabilities

- `work-item-plan-conversational-gate`: 对话式人工门——typed 反馈回合协议、HumanGateTurn durable 模型、预算/单飞/幂等语义、SC manual revision 专属路径、门关闭决定（仅 approve→compile→Confirmed 与 abandon；预算耗尽时门保持开启、拒绝新反馈）
- `work-item-plan-advance`: advance 幂等编排接口——前置校验、AdvanceRecord durable 模型、group attempt 唯一性与初始化 journal 恢复、ready-only 语义、SC/legacy 入口隔离
- `work-item-group-coding-execution`: group coding 执行编排——依赖就绪门（权威=依赖图+handoff readiness，order_index 仅 tie-breaker）、共享 worktree 单 active unit、失败留在 attempt 内处理、plan 缺陷走 amendment 链恢复

### Modified Capabilities

- `work-item-plan-single-candidate`: REQ-WSC-01 的 interactive 人工门从一次性决策消息升级为对话式多轮回合（auto 模式「无人工干预到达终态」语义不变）；新增与对话门/advance 的衔接场景

## Impact

- **WS 协议**：`src/web/workspace_ws_types/in_.rs` 新增 `human_gate_feedback`/`advance`；`out.rs` 新增 turn 事件族 + 门关闭决定 + advance 结果事件
- **Engine**：`src/product/workspace_engine/decisions.rs` 旁新增对话门 handler；`HumanGateSnapshot` 旁新增 durable `HumanGateTurn`；新增 SC revision prompt 与专属 provider run 分支
- **Coding**：`src/product/coding_workspace_engine/group.rs` 的 unit 选择加依赖就绪门；advance 与 `src/web/handlers/coding/group.rs` 初始化逻辑共享唯一性/binding/worktree lock/replay 四道约束
- **Driver**：campaign driver `ARIA_HUMAN_SCRIPT` 扩展多轮语法，interactive 端到端验收
- **durable**：lifecycle store 新增 HumanGateTurn/AdvanceRecord 持久化与恢复路径
- **契约**：阶段 1（typed outcome/预算/接管语义）与阶段 2（compiler/validator/fail-closed）全部复用不重定义
