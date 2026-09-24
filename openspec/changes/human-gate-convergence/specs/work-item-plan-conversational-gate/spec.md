# work-item-plan-conversational-gate Delta

## MODIFIED Requirements

### Requirement: 单飞与预算纪律（REQ-CG-02）

同一人工门同时 SHALL 至多存在一个非终态 turn；已有 in-flight turn 时收到新反馈或 approve/abandon SHALL 均返回 `gate_busy`，不隐式排队、不关门；终止决定仅在 turn 进入终态后处理。`manual_repairs_remaining` SHALL 在 turn durable 预留时扣减；修订失败（provider 错误、校验拒绝、超时）SHALL NOT 退还已扣预算。**预算重置边界**：普通 SC 修订门的快照预算仅在**新 logical gate**（新候选/新会话）建立时取默认值；同一 logical gate 内的修订轮次 SHALL 从 durable 快照 carry-forward（修订成功重建快照时接续剩余值，不得重置回默认）——原「普通 SC 修订门内的人工修订 turn 成功完成并经 Evaluate policy route 重建门快照时，快照预算 SHALL 重置为默认值」修订为本句语义。**amendment 人工门**的接续语义不变（REQ-GCE-03 场景二重开的原门，预算接续原 `human_gate_snapshot`）。amendment 门的接续/守卫判别 SHALL 绑定 durable amendment 事实，且 MUST 复用重开授权（`probe_amendment_gate_context`）所用的同一完整谓词。

**SC 门消息面边界**：SC interactive 与 amendment 门 SHALL 仅接受 `human_gate_feedback`、approve（`Confirm` 语义）与 abandon（显式 typed 入站命令）三类消息；关门决策 SHALL 以门专属 typed 决策承载，MUST NOT 依赖 legacy `HumanConfirmDecision`；收到错误消息类型时系统 SHALL 返回 stage-specific protocol error 且零副作用。

在门状态明确为可恢复 Final Compile 且阶段/flow/durable recovery 事实匹配时，现有 `WorkItemPlanCompileRecoveryAction` MAY 作为独立的 compile recovery 操作经 Cockpit WS 放行；该操作不是人工门反馈或关门决定，不改变三类人工门消息边界。其他阶段、错误 flow、缺少 recovery 事实或已终态时 SHALL 返回 stage-specific protocol error 且零副作用。

**门内修订轮次的 accepted 计数**：每次被接受为修订 turn 的反馈 SHALL 计入 gate-local accepted_feedback_turns（与 `manual_repairs_remaining` 同一 durable 预留原子提交）；该计数用于门卡呈现与「距通过」投影，SHALL NOT 写入 run_history 的 repairs_used/manual_repairs_used（policy 计数仅自动返修）。

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

#### Scenario: 合法 recovery action 与人工门命令并存

- **WHEN** SC 会话处于可恢复 Final Compile 状态，客户端从 Cockpit 发送既有 `WorkItemPlanCompileRecoveryAction`
- **THEN** WS 接受该独立 recovery 操作并按既有 recovery 语义处理；`Confirm`、`HumanGateFeedback`、`AbandonHumanGate` 的语义和放行边界不变

#### Scenario: 非 recovery 状态拒绝 recovery action

- **WHEN** 客户端在 generate、普通 HumanConfirm、AuthorConfirm 或已终态发送 `WorkItemPlanCompileRecoveryAction`
- **THEN** 系统返回 stage-specific protocol error，门、compile、预算与 provider 状态均不变

#### Scenario: recovery action 不替代关门命令

- **WHEN** 用户需要批准、反馈或终止 SC 人工门
- **THEN** Cockpit 继续分别发送 `Confirm`、`HumanGateFeedback` 或 `AbandonHumanGate`，不得把 recovery action 映射为其中任一命令

#### Scenario: 同 logical gate 修订不重置预算

- **WHEN** 同一 logical gate 内人工反馈修订成功且 Evaluate 重建门快照
- **THEN** 重建快照接续 durable 剩余 `manual_repairs_remaining`（如 3→2 后仍为 2），accepted_feedback_turns 递增；不回填默认值

#### Scenario: accepted 计数不污染 policy 计数

- **WHEN** 门内修订轮次发生
- **THEN** run_history 的 repairs_used/manual_repairs_used 不因此增加（仅自动返修计入 repairs_used），门卡从 gate-local 计数呈现轮次
