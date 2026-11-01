# human-gate-termination-reliability Specification

## Purpose

run 中止与人工门终止的语义边界与回执诚实性：每个终止/中止动作都有确定语义、明确回执与指路；degraded 连接不因静默丢帧停在 stale 视图。

## Requirements

### Requirement: abort 语义与回执诚实（REQ-HTR-01）

WS `Abort` 消息 SHALL 仅表示「取消当前进行中的 provider run」：存在 active run 时执行取消并回执 Aborted（既有语义）；**无 active run 时 SHALL 返回稳定错误码的 ProtocolError（如 `abort_no_active_run`），SHALL NOT 静默零回执**。Abort SHALL 受 stage validation 矩阵约束（不再全局豁免）：人工门开态（human_confirm 等待中）SHALL 拒绝并附指路信息（门级操作应使用反馈/确认/终止此门）；仅 running/cross_review/revision 等存在进行中 run 的阶段受理。错误回执 SHALL 携带当前阶段与可用动作提示。

#### Scenario: 门开态中止被拒并指路

- **WHEN** 会话停在人工确认门（无 active run）且连接发送 Abort
- **THEN** 返回 ProtocolError（abort_no_active_run 或 stage 相关稳定码）附「当前为人工确认门，可用操作：提交反馈/确认/终止此门」类指路；会话状态零变化

#### Scenario: 有 run 时中止正常

- **WHEN** provider run 进行中（running/cross_review/revision）且连接发送 Abort
- **THEN** 取消当前 run 并回执 Aborted（既有语义与恢复点不变）

### Requirement: 门态动作呈现分层（REQ-HTR-02）

前端 SHALL 按会话状态渲染中止/终止入口：人工门开态 SHALL NOT 渲染普通 run 中止钮（输入条 X）；门级「终止此门」按既有 abandon 通道呈现并带二次确认；错误面出现时 SHALL 提供指路（连接/租约类错误→重新接管；门态操作→门卡入口）。useStageUI 死动作与 rollback 发送器 SHALL 经全仓动态消费者核查后删除（无行为影响）。

#### Scenario: 门开态无中止钮

- **WHEN** 会话停在人工门
- **THEN** 输入条不渲染 run 中止钮；门卡呈现反馈/确认/终止此门三动作

### Requirement: degraded 连接关键帧投递（REQ-HTR-03）

服务端 SHALL 对关键帧（至少 stage_change、session_state、HumanGateOpened/门开类广播）避免 degraded 连接静默丢帧：投递失败/缓冲满时 SHALL 采用等待式投递或向该连接显式发送 `resync_required` 信号（客户端收到后主动拉取全量状态）。degraded 状态转移 SHALL 落 append-only 诊断打点（连接标识/时刻/原因，lease-diagnostics 同构通道），使「连接停在 stale 视图」可定案。打点失败零影响业务。

#### Scenario: 门开不被静默吞

- **WHEN** 连接处于 degraded 态且引擎广播门开关键帧（stage_change+session_state）
- **THEN** 该连接要么最终收到帧（等待式投递），要么收到 resync_required 并经客户端拉取恢复一致视图；不出现「帧被吞且无任何信号」的 stale 停留

#### Scenario: degraded 转移可观测

- **WHEN** 事后调查某连接的 stale 视图时段
- **THEN** 诊断打点给出该连接 degraded 进入/退出时刻与原因
