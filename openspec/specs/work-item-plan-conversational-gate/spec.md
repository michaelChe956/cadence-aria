# work-item-plan-conversational-gate Specification

## Purpose

单候选 workitem plan 的 interactive 人工门从一次性决策消息升级为对话式多轮协议：人在门内以 typed 反馈回合驱动 SC 专属修订，每回合 durable、可恢复、受预算约束，直到 approve/abandon 关门；预算耗尽时门保持开启、仅剩 approve/abandon（与阶段 1 一致）。

## Requirements

### Requirement: 对话式反馈回合（REQ-CG-01）

系统 SHALL 仅将服务端接受的 typed `human_gate_feedback` 入站命令（含客户端生成的 `command_id` 与自由文本反馈）计为一个修订回合；任意其他 WS 消息（普通 `UserMessage`、重连、ack、`approve`、`abandon`）SHALL NOT 触发 provider、SHALL NOT 消耗人工修订预算。服务端接受回合后 SHALL 分配 `turn_id` 并在 durable 持久化（CAS）成功后才启动 provider；同一 `command_id` 的重复提交 SHALL 返回同一 `turn_id`，同一 `turn_id` 的重复请求 SHALL NOT 重复扣减预算或重复启动 provider。

#### Scenario: 反馈回合触发一次修订

- **WHEN** 人工门处于等待态且无 in-flight turn，客户端发送 `human_gate_feedback { command_id, feedback }`
- **THEN** 服务端分配 `turn_id`、持久化 turn 记录、扣减预算、启动一次 SC revision provider，并发出 `human_gate_turn_open` 事件（含 `turn_id` 与扣减后的剩余预算）

#### Scenario: 客户端未收到响应重发同一命令

- **WHEN** 客户端在收到 `turn_open` 前以同一 `command_id` 重发 `human_gate_feedback`
- **THEN** 服务端返回首次分配的 `turn_id`，不新建回合、不重复扣预算、不重复启动 provider

### Requirement: 单飞与预算纪律（REQ-CG-02）

同一人工门同时 SHALL 至多存在一个非终态 turn；已有 in-flight turn 时收到新反馈或 approve/abandon SHALL 均返回 `gate_busy`，不隐式排队、不关门；终止决定仅在 turn 进入终态后处理。`manual_repairs_remaining` SHALL 在 turn durable 预留时扣减；修订失败（provider 错误、校验拒绝、超时）SHALL NOT 退还已扣预算。**普通 SC 修订门**内的人工修订 turn 成功完成并经 Evaluate policy route 重建门快照时，快照预算 SHALL 重置为默认值（与初始 author Evaluate-pass 同构；2026-09-03 专项测量轮实测记录并补句）；门未重建（修订失败/门未重开）时预算 SHALL 保持既有值。**amendment 人工门**（REQ-GCE-03 场景二重开的原门）的修订 turn 成功后，无论无 reviewer 的本地 Evaluate 路由还是有 reviewer 的 review-pass 路由重建 Approval 快照，快照预算 SHALL 接续重开时原 `human_gate_snapshot` 的 `manual_repairs_remaining`（typed turn 只扣快照、不递增 run_history 计数，重建不得凭空恢复已耗预算）。amendment 门的接续/守卫判别 SHALL 绑定 durable amendment 事实（指向本 plan session 的 Open/Applying `PlanAmendmentContext`，即重开授权所用的同一谓词），仅凭会话状态三元组（SingleCandidate + phase Completed + WaitingForHuman）SHALL NOT 判定为 amendment 门——该三元组可由通用状态写入伪造；判别命中而原快照缺席时 SHALL fail-closed 终止（AbortFatal{PersistenceFailure}），SHALL NOT 回退普通重置公式重建。amendment 接续语义与普通门重建重置语义存在家族分叉，已登记 defer 待统一裁决。provider 传输层瞬断 SHALL 在同一逻辑 `turn_id` 下以 `attempt_no` 递增内部重试，复用原 provider-start ledger 语义，SHALL NOT 创建新 turn。

**SC 门消息面边界**：SC interactive 与 amendment 门 SHALL 仅接受 `human_gate_feedback`、`Confirm`（approve）与 `Terminate`（abandon）；legacy session 保持现有 `HumanConfirmDecision::RequestChange` 行为不变；收到错误消息类型时系统 SHALL 返回 stage-specific protocol error 且零副作用；旧枚举在 REQ-WSC-07 退役门满足前 SHALL NOT 删除。

#### Scenario: 门内并发反馈被拒绝

- **WHEN** 存在 in-flight turn 时客户端发送新的 `human_gate_feedback`
- **THEN** 服务端返回 `gate_busy`（含当前 `turn_id`），预算与 provider 启动计数不变；门状态不变

#### Scenario: 回合进行中收到终止决定被拒绝

- **WHEN** turn 处于非终态时人发送 `approve` 或 `abandon`
- **THEN** 系统返回 `gate_busy`（含当前 `turn_id`），门状态不变、不关门；终止决定仅在 turn 进入终态后处理

#### Scenario: provider 瞬断恢复

- **WHEN** provider 在 turn 执行中 ws 断线或流式中断
- **THEN** 系统以同一 `turn_id` 递增 `attempt_no` 重试，预算扣减记录不变；超过重试上限后 turn 置 `Failed(provider_err)`，候选保持不变

### Requirement: SC manual revision 专属路径（REQ-CG-03）

人工反馈触发的单候选修订 SHALL 走 SC 专属路径：以反馈文本、当前候选 markdown 全文与必要 grammar 边界构造独立预算的 revision prompt；provider SHALL 输出完整修订版 markdown（非 diff/patch），输出经确定性前言修剪后进入 SC compiler 与 canonical validator。该路径 SHALL NOT 经过 legacy 中文标题 artifact 约束链；SHALL NOT 复用或扩张 SC author prompt 的 19,000 字节预算；修订 prompt  SHALL 注入 `.claude/rules/language.md` 全文与结构字面量优先规则句以维持中文 plan 与 grammar 字面量纪律，SHALL NOT 注入 code-usage/code-reading 摘要。该路径的反馈文本与构造的 revision prompt SHALL 受确定性 bounded-field 长度限制；超限 SHALL 在创建 turn 前拒绝，零预算消耗、零 provider 启动。每个 `turn_id` 的 `attempt_no` SHALL 有固定上限；每次真实 provider start SHALL 写入 provider-start ledger，但逻辑预算每个回合只消耗一次，SHALL NOT 以 WebSocket 事件推断启动次数。修订教学 SHALL 写死「只改反馈点名的内容，其余逐字保留」及反面清单（禁止删字段、清空 Outputs、省略 Handoff Schema 三字段）。修订后 findings 与既有指纹重复时 SHALL 按阶段 1 契约回到同一人工门。

#### Scenario: 修订成功回呈

- **WHEN** 回合的 provider 输出通过 SC compiler 与 canonical validator
- **THEN** 系统以 `human_gate_turn_completed` 回呈新候选 artifact 引用，人工门回到等待态，人可继续反馈或 approve

#### Scenario: 修订不过校验

- **WHEN** 修订输出被 canonical validator 拒绝
- **THEN** turn 置 `Failed(validation_reject)`，当前候选保持不变，已扣预算不退还，人可在剩余预算内再开回合

#### Scenario: 反馈超长零副作用拒绝

- **WHEN** 反馈文本或构造的 revision prompt 超过 bounded-field 上限
- **THEN** 系统拒绝创建 turn，预算与 provider 启动计数不变；人收到超限原因后可改短重发（新 `command_id`）

#### Scenario: 同指纹重现回同一门

- **WHEN** 修订后的新候选经复评产出的 findings 与既有指纹重复
- **THEN** 按阶段 1 防环契约回到同一人工门，不新建门实例，不重置预算

#### Scenario: 不触碰 legacy 约束链

- **WHEN** SC manual revision 产出英文 grammar 标题的 markdown
- **THEN** 产物只经 SC compiler/canonical validator 判定，不被 legacy 中文标题 artifact 约束拦截

### Requirement: 门关闭决定（REQ-CG-04）

人工门 SHALL 仅由两类决定关闭：`approve`（映射现行 `HumanConfirmDecision::Confirm`）关门并启动既有确定性 compile，compile 成功后 Plan 进入 durable Confirmed；`abandon`（映射现行 `HumanConfirmDecision::Terminate`）关门进入终态。人工修订预算耗尽时系统 SHALL 拒绝新的 `human_gate_feedback` 并返回预算耗尽原因，门保持开启且仅接受 approve/abandon——与阶段 1 `work-item-typed-outcome-policy` 既有语义一致，本 capability 不改写该语义。门关闭 SHALL NOT 自动触发 `advance`；advance 是 Plan durable Confirmed 之后由客户端显式调用的独立动作。

#### Scenario: approve 不绕过 compile

- **WHEN** 人在门内发出 `approve`
- **THEN** 系统关门并启动确定性 compile；仅当 compile 成功时 Plan 才进入 durable Confirmed；compile 失败按既有 fail-closed 路径处理

#### Scenario: 预算耗尽拒绝新反馈

- **WHEN** `manual_repairs_remaining` 为零且人再次发送 `human_gate_feedback`
- **THEN** 系统拒绝该反馈并返回预算耗尽原因，不创建 turn、不扣减、不启动 provider；门保持开启，仅接受 approve/abandon

#### Scenario: 伪造重开三元组不命中 amendment 接续

- **WHEN** 无任何 Open/Applying `PlanAmendmentContext` 的普通 SC 会话被通用状态写入伪造成 Completed+WaitingForHuman 三元组后，Evaluate 重建或迟到 review verdict 到达
- **THEN** 重建走普通门重置公式（默认预算 − run_history 计数），不继承遗留快照；终态评审守卫照常丢弃该 verdict，不重路由

#### Scenario: amendment 判别命中而快照缺席 fail-closed

- **WHEN** durable 重开签名与 Open/Applying `PlanAmendmentContext` 均在场，但原 `human_gate_snapshot` 缺席（损坏/外部删改）时触发 Approval 重建
- **THEN** 系统以 AbortFatal{PersistenceFailure} 报错终止（含 `human_gate_amendment_snapshot_missing` 诊断），不落任何重建快照，不回退普通重置公式

### Requirement: 门与回合的 durable 恢复（REQ-CG-05）

人工门快照与人工门预算扣减、`HumanGateTurn` 预留、provider 幂等键 SHALL 作为同一 durable reservation 原子提交（在 session record 上以 CAS 写入，复用阶段 1 快照原子写入契约）；恢复时按 reservation 状态决定释放、等待或继续，保证「预算只扣一次」与「同 `command_id` 不重复启动 provider」可被严格证明。`HumanGateTurn` SHALL 为独立 durable 记录（`turn_id`、`command_id`、`feedback_text`、状态、`attempt_no`、预算预留记账、结果引用、失败分类），其 `Reserved` 状态只在与上述原子事务同提交时生效。断线重连后系统 SHALL 从 durable 状态重建门与 in-flight turn：provider 仍在运行则等待其完成；provider 已终止则以同 `turn_id` 恢复。门相关事件 SHALL 保持事件前缀不可变，恢复不得删除或改写已持久化事件。

#### Scenario: turn 预留原子性

- **WHEN** turn 预留在「预算已扣、turn 未写入」或「turn 已写、预算未扣」的窗口内崩溃，恢复后以同 `command_id` 重发
- **THEN** 系统按 reservation 状态释放/等待/继续，最终预算恰好扣减一次、`turn_id` 恰好一个，无双重启动或双重扣减的中间态可观察

#### Scenario: 回合中断线重连

- **WHEN** 客户端在 turn 处于 provider 运行中断线后重连
- **THEN** 门状态、in-flight turn、剩余预算与既有事件原样恢复呈现；provider 已完成的结果不丢失，已死 provider 以同 `turn_id` 恢复

### Requirement: auto stopped 接管入口（REQ-CG-06）

系统 SHALL 扩展现有 `takeover_stopped_needs_human` HTTP 端点（`src/web/handlers/workspace_session.rs`）的接管后恢复语义，SHALL NOT 新增重复入口（阶段 1 REQ-TOP-06 将本操作入口延迟至阶段 3，本 requirement 兑现之）。接管前置：仅无 fatal/persistence diagnostic 的 `stopped_needs_human` auto session 可被接管（阶段 1 既有前提）。接管时 child session SHALL 继承 parent 的人工门快照（不可变引用或复制后的校验快照）、候选/诊断引用与 `manual_repairs_remaining`（不重置），以 interactive + 对话式人工门启动；parent 保持 `stopped_needs_human`，其状态、历史与事件 SHALL NOT 被篡改。重复接管 SHALL 返回同一 child，不新建。

#### Scenario: 接管 auto stopped session

- **WHEN** 用户对 `stopped_needs_human` 的 auto session 经现有接管端点发起接管操作
- **THEN** 系统创建新 interactive session，继承原门快照与剩余预算进入对话式人工门，可发送 `human_gate_feedback`；原 session 事件前缀不变，接管会话的人工预算等于快照剩余值；重复接管返回同一 child

### Requirement: 修订门重开快照保留（REQ-CG-07）

SC 计划批准的 durable Confirmed 终态 SHALL 保留 human gate snapshot 与最近一次 Markdown 候选文本作为修订门重开与预算接续的唯一来源；系统 SHALL NOT 在批准落库时清除该快照。

#### Scenario: 批准后快照在场供修订门重开

- **WHEN** 单候选计划经 approve→compile 落入 Confirmed+Completed 终态
- **THEN** durable session 记录仍保留 human gate snapshot，后续 amendment 反馈可经 CAS 重开同一门并从同一快照接续预算

#### Scenario: 修订基线回落到最近 Markdown 制品

- **WHEN** 修订回合构建 prompt 时当前 artifact 版本无 markdown（如已被投影版本覆盖）
- **THEN** 系统 SHALL 回落取最近一个 Markdown artifact version（批准时的计划文本）作为修订基线；版本列表完全无 Markdown 时按候选缺失拒绝
