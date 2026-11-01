# Spec Delta

> 基线：`openspec/specs/coding-provider-transport-retry/spec.md` 现有 transport 技术失败自动重试（3 次上限）等 requirement。本 delta 只新增一项 idle 挂起 requirement，不修改既有 requirement；既有纯 transport 3 次预算语义保持不变。

## ADDED Requirements

### Requirement: provider idle 挂起检测与一次自动重试（REQ-PTR-IDLE）

共享 provider stream SHALL 增加可重置的 idle watchdog：CodeReviewer/InternalReviewer 默认 300 秒、Coder 默认 600 秒、工具未返回期间取 `max(角色阈值, 1800 秒)`，阈值经环境变量配置且每次调用只读取一次；真实活动（含 Pi adapter 的 `pi_empty_output_retry`）重置计时，等待用户 choice/permission 时暂停。首次挂起 SHALL 同 provider fresh prompt 自动重试恰一次；再次挂起 SHALL 转 Blocked 并落 stalled 门（coder 门三动作、review 门 `[retry_review, abort]`）。MUST NOT 切换 provider、新增 Stalled 状态/last_event_at 字段，MUST NOT 挤占既有纯 transport 3 次预算。

#### Scenario: 首次挂起自动重试后继续

- **WHEN** coder 流静默达到 idle 阈值（默认 600 秒，`ARIA_CODING_CODER_IDLE_SECS` 可覆写）
- **THEN** 系统记录 reason=`provider_idle_stall` 的既有 Timeout 事件与空闲诊断，取消当前 provider，role-run 记 Failed，同 provider 以 fresh prompt 自动重试恰一次；重试成功则原流程继续

#### Scenario: 再次挂起转 Blocked 并通知

- **WHEN** 重试后的流再次静默达到阈值
- **THEN** attempt 转 Blocked，落对应 stalled 门（`coder_provider_stalled`/`code_review_provider_stalled`，review 门按 stage 恢复对应审查）并进入平台通知（kind=stalled），不存在第三次自动重试，provider 未被切换

#### Scenario: 活动重置与人工等待暂停

- **WHEN** 流处于工具执行中（阈值取 max(角色, `ARIA_CODING_TOOL_INFLIGHT_IDLE_SECS=1800`)）或持续产生真实事件（TextDelta/Execution/ToolCall/Result/StatusChanged），或 attempt 正等待用户 choice/permission
- **THEN** idle 计时分别按更长阈值判定、被活动重置、或暂停不计时；应用心跳/倒计时/snapshot/PersistenceWarning 不算活动，不产生误报挂起

#### Scenario: 不挤占 transport 预算

- **WHEN** 同一 run 内既发生纯 transport 失败又发生 idle 挂起
- **THEN** 两类预算独立计数（transport 仍最多 3 次、纯 idle 最多 2 次），互不折算
