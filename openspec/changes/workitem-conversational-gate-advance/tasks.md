# Tasks

> 本 change = 阶段 3（对话流人工门 + advance + SC manual revision + group coding 依赖门）。阶段 1/2 契约（typed outcome、预算、防环、门快照原子写入、compiler/validator、fail-closed）全部复用不重定义。阶段 4 才做多仓扩展与旧协议删除。

## 1. WS 协议面（REQ-CG-01/02/04/06、REQ-ADV-01/02）

- [ ] 1.1 入站类型：`human_gate_feedback { command_id, feedback }` 与 `advance { command_id }`（typed，挂在决策族；serde roundtrip 测试）
- [ ] 1.2 出站事件族：`human_gate_turn_open/turn_completed/turn_failed/gate_busy/closed`、`advance_completed/advance_rejected`（serde roundtrip + stage 准入校验：仅 HumanConfirm/相应 stage 接受）

## 2. HumanGateTurn durable 模型（REQ-CG-01/02/05）

- [ ] 2.1 `HumanGateTurn` 记录结构（turn_id/command_id/feedback_text/status/attempt_no/budget_reserved/result_artifact_ref/failure_class）+ lifecycle store 持久化（CAS，与 session 门区同原子写入约定）
- [ ] 2.2 双键幂等：command_id→turn_id 映射；同 turn_id 重试不重复扣预算/启动 provider；预算在预留时扣减、失败不退
- [ ] 2.3 单飞：in-flight turn 存在时新反馈回 gate_busy 不排队
- [ ] 2.4 恢复：重连重建门与 in-flight turn（provider 在跑→等；已死→同 turn_id attempt_no++）；事件前缀不可变断言

## 3. SC manual revision 专属路径（REQ-CG-03）

- [ ] 3.1 SC revision prompt：注入当前候选全文+typed feedback+grammar 边界+language.md 全文+优先规则句（不注入 code-usage/code-reading 摘要）；独立预算常量 `SC_MANUAL_REVISION_PROMPT_QUALITY_BUDGET_BYTES`（实测上调整百级+margin 注释）；反馈/prompt 受确定性 bounded-field 限制，超限创建 turn 前零副作用拒绝；教学句存在性测试先 RED 后 GREEN
- [ ] 3.2 revision provider run 分支：输出完整修订版 markdown→确定性前言修剪→SC compiler→canonical validator；不过则 turn_failed(validation_reject) 候选不变
- [ ] 3.3 隔离测试：legacy 中文标题约束链不被该路径触碰；legacy/story/design 路径行为零变化；`attempt_no` 固定上限与「每次真实 provider start 写 ledger、逻辑预算每回合只耗一次」的语义测试
- [ ] 3.4 同指纹 findings 重现回同一门（复用阶段 1 防环契约）

## 4. 门关闭与 approve 链（REQ-CG-04、REQ-CG-06、REQ-WSC-01 MODIFIED）

- [ ] 4.1 approve（映射现行 `HumanConfirmDecision::Confirm`）→关门→确定性 compile→成功→durable Confirmed；compile 失败走既有 fail-closed；abandon（映射 `HumanConfirmDecision::Terminate`）→关门终态；in-flight turn 期间 approve/abandon 回 gate_busy 不关门（turn 终态后才处理）
- [ ] 4.2 预算耗尽→拒绝新 feedback（带原因），门保持开启仅 approve/abandon（对齐阶段 1 REQ-TOP-02/06，不改写其语义）；不创建 turn/不扣减/不启动 provider 断言
- [ ] 4.3 门关闭不自动触发 advance（事件/状态断言）
- [ ] 4.4 auto stopped 接管（REQ-CG-06）：扩展现有 `takeover_stopped_needs_human` 端点的接管后恢复语义（不新增端点）；child 继承 parent 快照（不可变引用或校验快照）/预算/候选引用；重复接管返回同一 child；前提重申（仅无 fatal/persistence diagnostic）；原 session 事件不可篡改断言

## 5. advance 幂等编排（REQ-ADV-01/02/03）

- [ ] 5.1 advance handler：前置校验（plan durable Confirmed+子 session 存在+无 active compile/revision）仅适用首次 advance→零副作用拒绝；幂等命中优先于前置校验
- [ ] 5.2 `AdvanceRecord` durable + group 初始化 journal（checkpoint 恢复，中断不重建）
- [ ] 5.3 group attempt 唯一性：command_id 幂等 + plan_id 唯一约束；attempt 创建时写入持久化 `admission_kind = sc_advance`（依赖门/隔离测试的唯一判据，不从 flow_kind/命名推断）；units 按依赖拓扑序生成（order_index 仅 tie-breaker）；plan binding 绑不可变 plan_revision_id + worktree lock；已存在 attempt 的状态矩阵（Initializing/Ready/Running/AwaitingPlanAmendment/Completed 返回 durable 状态；Failed/Aborted 返回原状态不隐式重建）
- [ ] 5.4 ready-only：不启动 coding provider（StartCoding 语义不变）；advance_completed 载荷=attempt_id+workspace 入口
- [ ] 5.5 入口隔离：SC 子 session 直接启动 coding 被拒并提示先 advance；legacy 入口回归零变化；flow_kind 不可变

## 6. group coding 依赖就绪门（REQ-GCE-01，仅限 SC-linked attempt）

- [ ] 6.1 unit 启动前置检查（仅 advance 创建的 SC-linked attempt）：直接依赖全部 Completed+handoff 已发布+handoff revision 与当前 plan binding 匹配；未就绪 Pending 不跳过；全不就绪→等待态
- [ ] 6.2 fail-closed：环/未知依赖/自依赖/handoff 不匹配 durable 记录原因
- [ ] 6.3 共享 worktree 单 active unit 不变式回归测试；**`admission_kind = legacy_group` 的 attempt 保持 order_index 行为的隔离回归锁死**
- [ ] 6.4 Work Item 进展/成果只读投影（REQ-GCE-04）：每 WI 的 unit 状态/stage、当前与最终 commit、code review 结果、handoff 结果、失败/阻塞原因、plan revision binding、group 聚合进度，从 group attempt/unit 确定性派生；验收只要求既有数据面可读

## 7. 失败与 plan amendment 接线（REQ-GCE-02/03）

- [ ] 7.1 失败留在同一 attempt：有界重试同 unit、blocked/人工门呈现、Aborted 保留 durable；不新建第二 attempt 的断言
- [ ] 7.2 plan 缺陷链：BlockedByPlanDefect→AwaitingPlanAmendment（复用现有状态，写入 `PlanAmendmentContext` durable 关联）→回合挂原 plan session 对话门（amendment 上下文，预算接续其 manual_repairs_remaining，attempt 侧不产生新预算账）→新 plan revision→binding 更新（amendment approve 不复用首次 compile 入口）→resume target 回 Coding；断线恢复由原 plan session 负责；不兼容 fail-closed

## 8. 验收（需操作者授权真实运行）

- [ ] 8.1 driver 扩展：ARIA_HUMAN_SCRIPT 多轮语法（多个 request-change + confirm 按序消费）+ advance 调用模拟
- [ ] 8.2 interactive campaign 端到端：人为注入 human_required → 门开 → 多轮反馈 → SC revision 过校验 → approve → Confirmed（durable 断言 turn 记录/预算扣减/事件前缀）
  - 8.2a auto stopped → 现有 takeover 端点 → 新 interactive session 继承快照/预算 → typed feedback 可用
  - 8.2b 预算耗尽 → feedback 被拒（带原因）→ approve/abandon 仍可受理；反馈超长 → 创建 turn 前零副作用拒绝（预算/启动计数不变）
  - 8.2c in-flight turn 期间并发 feedback/approve/abandon → 均回 gate_busy 零副作用；turn 预留在崩溃窗口内恢复后预算恰好扣减一次（同 command_id）
- [ ] 8.3 advance 端到端：Confirmed 后 advance→ready（attempt+units+无 provider 启动）；重复 advance 幂等；依赖门 fixture 实证不跳过；非法依赖图（环/未知/自依赖）与 handoff 不匹配均实证 fail-closed
  - 8.3a `admission_kind` 隔离：SC-linked attempt 启用依赖门、`legacy_group` 完全旁路（行为零变化）；同 plan_id 已有 Failed/Aborted attempt 时 advance 返回原状态不重建；advance 初始化中断后同 attempt 恢复
  - 8.3b 进展/成果投影覆盖：group workspace 数据面能读到每个 Work Item 的状态/成果/失败原因
- [ ] 8.4 全量门禁：fmt/clippy -D warnings/test --locked 全绿（禁 -j；已知 flaky 家族定向复跑定性）
  - 8.4a amendment → 原 plan session → 新 plan revision → 同一 attempt resume（断线恢复同路径）
  - 8.4b 崩溃/重连矩阵：turn reservation、takeover、advance journal、amendment 四处恢复路径；provider ledger 与 turn attempt_no 的重连对账
- [ ] 8.5 验收报告落盘 cadence/reports/ + defer 账登记（SC 子 session 只读投影形态、auto_start_coding 语义等留给后续裁决项）
