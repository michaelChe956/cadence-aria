# work-item-plan-conversational-gate Delta

## MODIFIED Requirements

### Requirement: 单飞与预算纪律（REQ-CG-02）

同一人工门同时 SHALL 至多存在一个非终态 turn；已有 in-flight turn 时收到新反馈或 approve/abandon SHALL 均返回 `gate_busy`，不隐式排队、不关门；终止决定仅在 turn 进入终态后处理。`manual_repairs_remaining` SHALL 在 turn durable 预留时扣减；修订失败（provider 错误、校验拒绝、超时）SHALL NOT 退还已扣预算。**普通 SC 修订门**内的人工修订 turn 成功完成并经 Evaluate policy route 重建门快照时，快照预算 SHALL 重置为默认值（与初始 author Evaluate-pass 同构；2026-09-03 专项测量轮实测记录并补句）；门未重建（修订失败/门未重开）时预算 SHALL 保持既有值。**amendment 人工门**（REQ-GCE-03 场景二重开的原门）的修订 turn 成功后，无论无 reviewer 的本地 Evaluate 路由还是有 reviewer 的 review-pass 路由重建 Approval 快照，快照预算 SHALL 接续重开时原 `human_gate_snapshot` 的 `manual_repairs_remaining`（typed turn 只扣快照、不递增 run_history 计数，重建不得凭空恢复已耗预算）。amendment 门的接续/守卫判别 SHALL 绑定 durable amendment 事实，且 MUST 复用重开授权（`probe_amendment_gate_context`）所用的同一完整谓词。

**SC 门消息面边界**：SC interactive 与 amendment 门 SHALL 仅接受 `human_gate_feedback`、approve（`Confirm` 语义）与 abandon（显式 typed 入站命令，`legacy-protocol-retirement` REQ-RET-02 重承载落地）三类消息；关门决策 SHALL 以门专属 typed 决策承载，MUST NOT 依赖 legacy `HumanConfirmDecision`（旧枚举已随退役删除）；收到错误消息类型（含已删除的 legacy 决策消息）时系统 SHALL 返回 stage-specific protocol error 且零副作用。

在门状态明确为可恢复 Final Compile 且阶段/flow/durable recovery 事实匹配时，现有 `WorkItemPlanCompileRecoveryAction` MAY 作为独立的 compile recovery 操作经 Cockpit WS 放行；该操作不是人工门反馈或关门决定，不改变三类人工门消息边界。其他阶段、错误 flow、缺少 recovery 事实或已终态时 SHALL 返回 stage-specific protocol error 且零副作用。

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
