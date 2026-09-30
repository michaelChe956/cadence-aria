# Spec Delta

## ADDED Requirements

### Requirement: 返修指令与上下文注记单次消费（REQ-GCE-C2-INSTR）

Coder 首次 spawn 与 rework 两条路径 SHALL 使用同一消费口径：系统 SHALL 先以待消费的 rework instruction 与 context note 渲染完整 prompt 与执行上下文，再在一次可重放的原子写入中认领这些指令、绑定渲染结果并标记消费，最后 spawn provider。实际发送给 provider 的 prompt SHALL 包含被消费的指令内容；以指令为空渲染后再标记消费的行为 MUST NOT 发生。标记消费 MUST NOT 早于完整 prompt 渲染完成。同一指令 SHALL 至多被一次 role run 消费；中断后重放同一次认领 SHALL 命中同一结果而非生成新的上下文 hash。已记录的执行上下文 hash MUST NOT 被覆盖为新含义。

#### Scenario: rework 实际 prompt 含新指令

- **WHEN** Code Review 要求返修并落地新的 rework instruction，系统启动返修 Coder
- **THEN** provider 收到的 prompt 包含该指令，指令被该次 role run 消费一次，执行上下文 hash 与实际 prompt 一致

#### Scenario: spawn 路径与 rework 路径口径一致

- **WHEN** 首次 spawn 时存在未消费的 rework instruction 或 context note
- **THEN** 渲染出的执行上下文与实际 prompt 均包含其完整内容，而不仅是摘要

#### Scenario: 认领后 spawn 前中断

- **WHEN** 指令已认领并绑定上下文，但在 provider spawn 前发生中断
- **THEN** 系统落地可操作等待项并通知用户；用户点击继续后以同一认领与同一上下文 hash 启动，不产生 hash 冲突，指令不被消费第二次

#### Scenario: 渲染失败不消费

- **WHEN** 完整 prompt 渲染失败
- **THEN** 指令保持未消费，attempt 进入可操作等待，用户重试时仍能读取该指令
