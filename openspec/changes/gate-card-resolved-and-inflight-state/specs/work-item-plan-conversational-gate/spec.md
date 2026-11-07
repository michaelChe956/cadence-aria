## ADDED Requirements

### Requirement: 因取消或终止导致的人工修订失败必须记为中止并明说选择未生效（REQ-CG-10）

人工门修订 turn 的 provider run 因取消或终止而失败时，SHALL 记为中止而非 `failed` / `provider_err`，并 MUST 在对话流中明确告诉 owner「修订被中断，你的选择未生效」。本 requirement 复用既有中止语义（`TimelineNodeStatus::Aborted`、`HumanGateTurnFailureClass::Cancelled`、`RunAbortOutcome` 三态、REQ-WIGA-16/17），MUST NOT 引入第二套「中止」状态。

#### Scenario: 终止期间的修订不再记为 provider 故障

- **WHEN** owner 在修订 turn 的 provider run 进行中点击终止面，run 被取消，驱动因取消返回中止哨兵 `provider run aborted`
- **THEN** 该 turn SHALL 为 `Failed` 且 `failure_class=Cancelled`，MUST NOT 为 `provider_err`
- **AND** 修订 author 载体节点 `status` SHALL 为 `Aborted`，`summary` SHALL 逐字为 `运行已中止：修订被中断，你的选择未生效`，MUST NOT 为 `Failed` 或 `人工修订失败，本轮反馈未采纳`
- **AND** SHALL 发出 `human_gate_turn_failed`，`failure_class` 为 `cancelled`，`message` 包含 `你的选择未生效`

#### Scenario: 真实 provider 失败仍为 provider_err

- **WHEN** 修订驱动因 provider 真实失败、idle watchdog 或选择等待超时返回错误（消息不是中止哨兵）
- **THEN** turn SHALL 为 `Failed` 且 `failure_class=ProviderErr`，帧 `failure_class` 为 `provider_err`
- **AND** 节点 SHALL 为 `Failed`，`summary` 为 `人工修订失败，本轮反馈未采纳`，行为与本 change 前逐字相同

#### Scenario: 已中止的节点不被改写回失败

- **WHEN** 修订 author 载体节点已被 `finish_aborted_run` 置为 `Aborted`，随后 turn 因 `Cancelled` 终态化
- **THEN** 节点 SHALL 保持 `Aborted`（summary 补全为含明说句的文案），MUST NOT 回到 `Failed`

#### Scenario: 取消兜底路径文案一致

- **WHEN** drive 在取消收口窗口内未完成，由 `fail_cancelled_human_gate_revision_turn` 兜底终态化 turn
- **THEN** 回显帧 `failure_class` SHALL 为 `cancelled`，`message` SHALL 包含 `你的选择未生效`
- **AND** 修订 author 载体节点 SHALL 为 `Aborted`

#### Scenario: 对话流出现明说句且刷新后仍在

- **WHEN** 时间线存在 `status=aborted` 且 `summary="运行已中止：修订被中断，你的选择未生效"` 的 author 节点
- **THEN** 重建的对话流 SHALL 含一条内容逐字为 `修订被中断，你的选择未生效` 的系统条目，位于该节点已有条目之后
- **AND** 该条目来自 durable 节点 `summary`，`session_state` 全量重放后 SHALL 仍在

#### Scenario: 普通中止不误加修订句

- **WHEN** 时间线存在 `status=aborted`、`summary="运行已中止"` 的节点（非人工修订）
- **THEN** 对话流 MUST NOT 为其追加 `修订被中断，你的选择未生效` 条目
