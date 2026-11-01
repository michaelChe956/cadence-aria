# Spec Delta

## ADDED Requirements

### Requirement: 候选快照驱动的 plan 门恢复（REQ-C1-GATE-01）

当 plan 的候选门因 relay 失败、无 WS consumer、慢 observer 或其他观察面故障进入可恢复等待时，系统 SHALL 先将完整 candidate snapshot、source revision、budget、gate 身份与诊断事实持久化，再展示恢复操作。驾驶舱和系统通知 MUST 提供“恢复运行”或“从权威候选重建”按钮；没有完整候选快照时 SHALL 禁止裸 approve。恢复/重建成功后 SHALL 回到现有 typed feedback/approve/abandon 协议，不创建第二候选权威或自动代替用户关门。

#### Scenario: relay 失败后恢复原门

- **WHEN** relay 失败且 candidate snapshot、source revision、budget 和 gate 事实已完整持久化
- **THEN** 驾驶舱显示失败原因与“恢复运行”操作；用户点击后系统恢复原反馈/批准面并由既有编排继续，issue 不被 abandon

#### Scenario: 缺失快照禁止裸批准

- **WHEN** plan 门处于恢复等待但缺少完整 candidate snapshot 或其 source/budget 事实不可读
- **THEN** 系统不显示或拒绝 approve，显示“从权威候选重建/恢复运行”操作和缺失原因；不扣预算、不启动 provider、不伪造门已批准

#### Scenario: 无 WS consumer 仍可从驾驶舱恢复

- **WHEN** observer/WS consumer 不存在或投递失败但 durable 候选和门转换已落盘
- **THEN** 系统通过 inbox/系统通知呈现可恢复卡片，用户经 REST/驾驶舱操作提交反馈或批准后继续原门；观察面失败不被当作候选业务失败

### Requirement: plan 门操作结果通知（REQ-C1-GATE-02）

每个恢复等待项 SHALL 向驾驶舱 inbox/系统通知提供失败原因、已完成步骤、目标身份、plan/session/gate 身份、可能外部副作用、按钮动作及成功后的下一阶段。恢复操作 MUST 携带稳定 `command_id`、gate/绑定身份和 expected version；同键同负载重放 SHALL 返回同一结果，过期对象 SHALL fail-closed。

#### Scenario: 重复恢复命令幂等

- **WHEN** 客户端在首次响应未知时以同一 `command_id` 重试恢复操作
- **THEN** 系统返回首次操作结果，不重复重建候选、不重复扣预算、不启动第二 provider

#### Scenario: 旧 gate 操作被拒

- **WHEN** 用户以过期 gate/version 或不同 plan/session 身份提交恢复、feedback 或 approve
- **THEN** 系统返回需刷新/重新绑定提示，门、候选、预算和历史均不变
