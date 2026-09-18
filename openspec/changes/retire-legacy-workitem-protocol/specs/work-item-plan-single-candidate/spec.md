# work-item-plan-single-candidate Delta

## Purpose

旧协议退役落地本 capability 的终态：REQ-WSC-07（新旧路径并存与可验证退役）的判据已按 `legacy-protocol-retirement` REQ-RET-01 全口径重测全绿并解锁删除，并存要求收敛为单路径契约；多仓 preflight 条款随 legacy fallback 消失同步修订（失败一律收敛新路径 durable 终态）。

## ADDED Requirements

### Requirement: 旧协议退役与单路径收敛（REQ-WSC-08）

REQ-WSC-07 退役门已按 `legacy-protocol-retirement` REQ-RET-01 全口径重测全绿解锁后，旧协议（generation-mode 决策、逐段确认消息、review_decision 双选项语法、`HumanConfirmDecision` 旧枚举及其消息族、SelectRevisionPath 族及其专属 DTO）SHALL 删除。删除后：新会话 SHALL 一律走单候选流，不存在 legacy 入口；收到已删除消息类型系统 SHALL 返回 stage-specific protocol error 且零副作用；多仓 Issue 的确定性 preflight 失败 SHALL 收敛为新路径 durable fatal/recoverable 终态（含失败原因），系统 MUST NOT 存在 legacy fallback 路径，MUST NOT 静默切换 `flow_kind`（本句修订并取代 REQ-WSC-07 原「legacy fallback 只允许在确定性 preflight 失败且新路径尚未产生副作用时发生」条款）；历史 legacy session 的 durable 记录与事件前缀 SHALL 只读保留（处置细则以 `legacy-protocol-retirement` REQ-RET-03 为唯一来源）；SC 门消息面（`human_gate_feedback`/approve/abandon）以 `work-item-plan-conversational-gate` 为唯一来源。

#### Scenario: 已删除消息协议错误拒绝

- **WHEN** 退役完成后客户端发送任一已删除的 legacy 决策消息（generation-mode 决策、逐段确认、review_decision 双选项、human_confirm）
- **THEN** 系统返回 stage-specific protocol error 且零副作用，会话状态与事件流不变

#### Scenario: 多仓 preflight 失败收敛新路径终态

- **WHEN** 多仓 Issue 的确定性 preflight 失败（无论新路径是否已产生副作用）
- **THEN** 失败以新路径 durable fatal/recoverable 终态记录并含原因，系统不存在任何 legacy fallback 或 `flow_kind` 切换路径

#### Scenario: 新会话一律单候选流

- **WHEN** 退役完成后创建任意 workitem workspace 会话
- **THEN** 会话走单候选流（prepare → generate → evaluate → approval → completed），不存在 legacy 逐段路径选项

#### Scenario: 历史 legacy 记录只读保留

- **WHEN** 退役完成后读取历史 legacy session 的 durable 记录
- **THEN** 记录与事件前缀原样保留可读，未被迁移或清洗

## REMOVED Requirements

### Requirement: 新旧路径并存与可验证退役（REQ-WSC-07）

- **Rationale**: 退役门判据已按 `legacy-protocol-retirement` REQ-RET-01 全口径重测全绿（证据矩阵留档），并存与退役门要求完成使命；判据子项已随删除面执行完毕（14 golden 归档、legacy 回归证据留档后测试族退役），单路径终态契约由 ADDED REQ-WSC-08 承接，多仓 preflight 条款修订同步并入 REQ-WSC-08。
