# work-item-plan-conversational-gate Delta

## Purpose

SC 门关门决策 typed 重承载：approve/abandon 不再映射 legacy `HumanConfirmDecision`（旧枚举随 `legacy-protocol-retirement` 退役删除）；SC 门消息面边界句与门关闭决定句同步修订。REQ-CG-01/03/05/06/07 与 REQ-CG-02/04 其余语义（单飞、预算、幂等、恢复、amendment 接续）零变化。

## MODIFIED Requirements

### Requirement: 单飞与预算纪律（REQ-CG-02）

同一人工门同时 SHALL 至多存在一个非终态 turn；已有 in-flight turn 时收到新反馈或 approve/abandon SHALL 均返回 `gate_busy`，不隐式排队、不关门；终止决定仅在 turn 进入终态后处理。`manual_repairs_remaining` SHALL 在 turn durable 预留时扣减；修订失败（provider 错误、校验拒绝、超时）SHALL NOT 退还已扣预算。**普通 SC 修订门**内的人工修订 turn 成功完成并经 Evaluate policy route 重建门快照时，快照预算 SHALL 重置为默认值（与初始 author Evaluate-pass 同构；2026-09-03 专项测量轮实测记录并补句）；门未重建（修订失败/门未重开）时预算 SHALL 保持既有值。**amendment 人工门**（REQ-GCE-03 场景二重开的原门）的修订 turn 成功后，无论无 reviewer 的本地 Evaluate 路由还是有 reviewer 的 review-pass 路由重建 Approval 快照，快照预算 SHALL 接续重开时原 `human_gate_snapshot` 的 `manual_repairs_remaining`（typed turn 只扣快照、不递增 run_history 计数，重建不得凭空恢复已耗预算）。amendment 门的接续/守卫判别 SHALL 绑定 durable amendment 事实，且 MUST 复用重开授权（`probe_amendment_gate_context`）所用的同一完整谓词——指向本 plan session 的 Open/Applying `PlanAmendmentContext`，**且**其 group attempt 处于 AwaitingPlanAmendment 并绑定本 plan session 的 entity；仅凭会话状态三元组（SingleCandidate + phase Completed + WaitingForHuman）SHALL NOT 判定为 amendment 门——该三元组可由通用状态写入伪造；group attempt 已离开 AwaitingPlanAmendment 的应用窗口（context 先行 Open→Applying 而 attempt 状态已同步/未同步的窗口内）SHALL NOT 判定为 amendment 门——迟到 verdict 照常被终态评审守卫丢弃，重建走普通门重置语义。判别为三态：「明确无 amendment 事实」（记录可读且签名不符 / 无 context / attempt 不在 AwaitingPlanAmendment）走普通门语义；判别命中而原快照缺席时 SHALL fail-closed 终止（AbortFatal{PersistenceFailure}）；判别所需 durable 事实无法读取/校验（session record 或 context 文件损坏、目录读失败、瞬态 I/O 错误）时 SHALL 同样以持久化失败 fail-closed 终止（AbortFatal{PersistenceFailure}），SHALL NOT 被当作「无 amendment 事实」进入普通门重置公式——否则真 amendment 链在存储瞬断时会被凭空恢复预算。fail-closed 终止的 policy diagnostics SHALL 写回 durable session record（与 policy route 落盘机制一致），仅内存诊断不足。判别命中时 SHALL NOT 回退普通重置公式重建。amendment 接续语义与普通门重建重置语义存在家族分叉，已登记 defer 待统一裁决。provider 传输层瞬断 SHALL 在同一逻辑 `turn_id` 下以 `attempt_no` 递增内部重试，复用原 provider-start ledger 语义，SHALL NOT 创建新 turn。

**SC 门消息面边界**：SC interactive 与 amendment 门 SHALL 仅接受 `human_gate_feedback`、approve（`Confirm` 语义）与 abandon（显式 typed 入站命令，`legacy-protocol-retirement` REQ-RET-02 重承载落地）三类消息；关门决策 SHALL 以门专属 typed 决策承载，MUST NOT 依赖 legacy `HumanConfirmDecision`（旧枚举已随退役删除）；收到错误消息类型（含已删除的 legacy 决策消息）时系统 SHALL 返回 stage-specific protocol error 且零副作用。

#### Scenario: 门内并发反馈被拒绝

- **WHEN** 存在 in-flight turn 时客户端发送新的 `human_gate_feedback`
- **THEN** 服务端返回 `gate_busy`（含当前 `turn_id`），预算与 provider 启动计数不变；门状态不变

#### Scenario: 回合进行中收到终止决定被拒绝

- **WHEN** turn 处于非终态时人发送 approve 或 abandon
- **THEN** 系统返回 `gate_busy`（含当前 `turn_id`），门状态不变、不关门；终止决定仅在 turn 进入终态后处理

#### Scenario: provider 瞬断恢复

- **WHEN** provider 在 turn 执行中 ws 断线或流式中断
- **THEN** 系统以同一 `turn_id` 递增 `attempt_no` 重试，预算扣减记录不变；超过重试上限后 turn 置 `Failed(provider_err)`，候选保持不变

#### Scenario: 已删除 legacy 决策消息零副作用拒绝

- **WHEN** SC 门开启期间客户端发送已删除的 legacy `human_confirm` 类决策消息
- **THEN** 系统返回 stage-specific protocol error 且零副作用（门状态、预算、turn 均不变），关门决策不依赖 `HumanConfirmDecision`

### Requirement: 门关闭决定（REQ-CG-04）

人工门 SHALL 仅由两类决定关闭：`approve`（以 `Confirm` 语义承载，关门并启动既有确定性 compile，compile 成功后 Plan 进入 durable Confirmed）与 `abandon`（以显式 typed 入站命令承载，关门进入终态）——两类关门决定均以门专属 typed 决策承载，MUST NOT 映射或依赖 legacy `HumanConfirmDecision` 枚举。人工修订预算耗尽时系统 SHALL 拒绝新的 `human_gate_feedback` 并返回预算耗尽原因，门保持开启且仅接受 approve/abandon——与阶段 1 `work-item-typed-outcome-policy` 既有语义一致，本 capability 不改写该语义。门关闭 SHALL NOT 自动触发 `advance`；advance 是 Plan durable Confirmed 之后由客户端显式调用的独立动作。

#### Scenario: approve 不绕过 compile

- **WHEN** 人在门内发出 approve
- **THEN** 系统关门并启动确定性 compile；仅当 compile 成功时 Plan 才进入 durable Confirmed；compile 失败按既有 fail-closed 路径处理

#### Scenario: abandon 经 typed 命令关门

- **WHEN** 人在门内发出显式 typed abandon 命令
- **THEN** 系统关门进入终态，全程不经过 legacy `HumanConfirmDecision` 通道，事件前缀不可变语义保持

#### Scenario: 伪造重开三元组不命中 amendment 接续

- **WHEN** 无任何 Open/Applying `PlanAmendmentContext` 的普通 SC 会话被通用状态写入伪造成 Completed+WaitingForHuman 三元组后，Evaluate 重建或迟到 review verdict 到达
- **THEN** 重建走普通门重置公式（默认预算 − run_history 计数），不继承遗留快照；终态评审守卫照常丢弃该 verdict，不重路由

#### Scenario: amendment 判别命中而快照缺席 fail-closed

- **WHEN** durable 重开签名与 Open/Applying `PlanAmendmentContext` 均在场，但原 `human_gate_snapshot` 缺席（损坏/外部删改）时触发 Approval 重建
- **THEN** 系统以 AbortFatal{PersistenceFailure} 报错终止（含 `human_gate_amendment_snapshot_missing` 诊断），不落任何重建快照，不回退普通重置公式

#### Scenario: 判别 durable 读失败 fail-closed 不进重置公式

- **WHEN** 真 amendment 门重开在场的会话，其判别所需 durable 事实（session record 或 `PlanAmendmentContext` 文件）损坏/不可读（权限、目录读失败、瞬态 I/O）时触发 Evaluate 重建或迟到 review verdict 到达
- **THEN** 系统以 AbortFatal{PersistenceFailure} 显式 fail-closed 终止（含 `persistence_failure` 诊断），不把「无法读取」当作「无 amendment 事实」走普通门重置公式凭空恢复预算，也不静默丢弃 verdict 保持门开启；诊断写回 durable session record

#### Scenario: 应用窗口不判 amendment 门

- **WHEN** `PlanAmendmentContext` 已先行 Open→Applying 而 group attempt 已离开 AwaitingPlanAmendment（应用窗口内）时，迟到 review verdict 或 Evaluate 重建到达该 Completed+WaitingForHuman 会话
- **THEN** 终态评审守卫照常丢弃该 verdict（不重路由）；Evaluate 重建走普通门重置公式，不接续快照预算——判别与 probe 放行重开的完整谓词同源

#### Scenario: 预算耗尽拒绝新反馈

- **WHEN** `manual_repairs_remaining` 为零且人再次发送 `human_gate_feedback`
- **THEN** 系统拒绝该反馈并返回预算耗尽原因，不创建 turn、不扣减、不启动 provider；门保持开启，仅接受 approve/abandon
