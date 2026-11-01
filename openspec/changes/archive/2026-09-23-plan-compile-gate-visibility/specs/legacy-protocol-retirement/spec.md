# legacy-protocol-retirement Delta

## MODIFIED Requirements

### Requirement: 删除面分层与残留归零（REQ-RET-02）

旧协议删除 SHALL 按「SC 门关门决策 typed 重承载 → 前端 legacy 消费面迁移 → 后端删除跨端原子收口」的分层推进，每层独立可验收。必删集 SHALL 至少覆盖：WS 入站协议的 legacy 逐段确认族（generation-mode 决策、逐段确认消息、review_decision 双选项语法、SelectRevisionPath 族及其专属 DTO）与 `HumanConfirmDecision` 旧枚举及其消息族；引擎侧 legacy 决策路由分支与逐段确认引擎面；`flow_kind` 单路径收敛（新会话一律单候选流）。SC 门关门决策 SHALL 以门专属 typed 决策承载：approve 保持 `Confirm` 语义，abandon 以显式 typed 入站命令承载，MUST NOT 复用 legacy `human_confirm` 通道或其枚举。删除完成后全仓残留 SHALL 归零：必删集符号在全仓产码与测试不可检索（历史归档文档与 durable 数据除外），收到已删除消息类型系统 SHALL 返回 protocol error 且零副作用，全量门禁（后端+前端）双绿。

Cockpit 为已保留且有明确 durable 状态的 Final Compile recovery action 提供 typed 消费入口，属于现有协议能力的可达性修复，不构成旧逐段确认协议的恢复；旧 generation-mode、逐段确认、review_decision 双选项和旧枚举消息族仍不得重新暴露。收到不匹配阶段的 recovery action 或已删除 legacy 消息时，系统 SHALL 返回 stage-specific protocol error 且零副作用。

#### Scenario: SC 门关门决策 typed 重承载先行

- **WHEN** 后端以显式 typed 入站命令承载 SC 门 abandon、关门决策不再依赖 `HumanConfirmDecision`
- **THEN** SC 门 approve/abandon/反馈行为与重承载前等价（含幂等、预算、恢复语义），legacy 双轨此时仍保留且全量门禁绿

#### Scenario: 前端 legacy 消费面全量迁移

- **WHEN** 前端 legacy 决策发送与动作面（WS 消息 union、stage 动作配置、cockpit/plan-repair 路由、批量确认发送、legacy 页面决策面）切换为 typed
- **THEN** 前端不再发送任何 legacy 决策消息，前端测试面同步收敛，前端测试全绿

#### Scenario: 后端删除跨端原子收口

- **WHEN** 后端删除 legacy 变体族、DTO、决策路由分支与逐段引擎面
- **THEN** 前端残余 legacy 类型同批归零，中间双轨态不对外存在，收到已删除消息类型返回 protocol error 且零副作用

#### Scenario: 残留归零断言

- **WHEN** 删除完成判定执行
- **THEN** 必删集符号在全仓产码与测试的检索结果为零（历史归档文档与 durable 数据除外），全量门禁双绿，legacy 回归测试族随删除退役且其重测证据留档在案

#### Scenario: Cockpit recovery 入口不恢复 legacy

- **WHEN** Cockpit 用户在合法可恢复 Final Compile 状态选择既有 recovery action
- **THEN** 系统经现有 typed recovery 消息完成操作，前端不发送 generation-mode、逐段确认、review_decision 或 `HumanConfirmDecision`

#### Scenario: 退役消息与非法 recovery 仍 fail-closed

- **WHEN** 客户端发送已删除 legacy 决策消息，或在不匹配阶段发送 recovery action
- **THEN** 系统返回 stage-specific protocol error 且不改变会话、门、compile、预算或 provider 状态
