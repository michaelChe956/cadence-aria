# work-item-plan-single-candidate Delta

## MODIFIED Requirements

### Requirement: 单候选计划事务（REQ-WSC-01）

系统 SHALL 将 workitem 段对外流程压缩为 prepare → generate → evaluate → approval → completed 五类可见状态，另加吸收态 failed。outline/draft/batch 的逐段确认、逐段 review、生成模式选择 SHALL NOT 暴露为用户可见或可应答的 WS/UI 决策；生成模式（batch/serial）SHALL 由运行时按模型能力与候选规模自行选择。interactive 策略下的人工门 SHALL 升级为对话式多轮协议（typed 反馈回合、SC 专属修订路径、approve→compile→Confirmed 关门链），其回合、预算、幂等与恢复语义以 `work-item-plan-conversational-gate` capability 为唯一来源；`auto_if_valid` 策略语义不变。

在既有单候选计划事务与对话式人工门语义不变的前提下，Work Item Plan 的批次确认与 Final Compile recovery 状态 SHALL 在 Cockpit 具有可观察、可定位的待处理投影。该投影 MAY 使用批次/recovery 专属展示和既有 typed 动作，但 MUST NOT 恢复 outline/draft/batch 的 legacy 逐段确认、逐段 review 或 generation-mode 对外决策协议。Plan 已 durable Confirmed 后仍须由客户端显式调用 `advance`，Cockpit 投影不得将可见门误作 coding 已启动。

#### Scenario: 自动化 campaign 无人工干预到达终态

- **WHEN** 以 `auto_if_valid` 策略运行且候选通过全部机械校验、无未决 human_required finding、无重复指纹
- **THEN** 系统直接完成原子 compile 并发布 Confirmed plan 与 Work Items，全程不需要任何 WS 决策消息

#### Scenario: 生成模式不再询问

- **WHEN** 候选计划生成需要选择 batch 或 serial 执行策略
- **THEN** 运行时依据模型能力与候选规模内部选择，不产生 `select_work_item_generation_mode` 类型的对外决策请求

#### Scenario: interactive 门内多轮修订后批准

- **WHEN** 以 `interactive` 策略运行且 reviewer 判定需要人工处理，人工门打开后人在门内连续给出多轮 typed 反馈
- **THEN** 每轮反馈经 SC 专属修订路径产出通过校验的新候选并回呈；人 approve 后门关闭、确定性 compile 成功后 Plan 进入 durable Confirmed，全程不经过 legacy 逐段确认消息

#### Scenario: Confirmed 后经 advance 进入 coding 就绪

- **WHEN** Plan 已 durable Confirmed 且客户端显式调用 `advance`
- **THEN** 系统建立该 plan 唯一的 WorkItemGroup coding attempt 并返回 group workspace 入口（ready-only，不启动 coding provider）；advance 语义以 `work-item-plan-advance` capability 为唯一来源

#### Scenario: 批次状态可观察但不恢复逐段协议

- **WHEN** 单候选 plan 成功或失败恢复路径进入整组 Work Item Draft 确认状态
- **THEN** Cockpit 显示一个整组确认门及其可用 typed 动作，不显示逐段确认或 generation-mode 决策，不改变单候选状态机

#### Scenario: Compile recovery 可观察且不绕过 Confirmed/advance

- **WHEN** 单候选 plan 处于可恢复 Final Compile 状态
- **THEN** Cockpit 显示 recovery 门并允许既有 recovery action；仅服务端既有 compile 结果可使 plan 进入 Confirmed，Cockpit 不发送隐式 `advance` 或 coding 启动
