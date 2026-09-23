# plan-compile-gate-visibility Specification

## Purpose

为 Work Item Plan 的整组 Draft 确认与 Final Compile recovery 建立统一 Cockpit 可见性和可操作性契约，使成功、失败可恢复、重连和终态路径都能被用户观察并通过既有 typed 动作安全推进，而不恢复已退役的逐段确认协议。

## Requirements

### Requirement: 批次确认门投影（REQ-PCG-01）

当 Work Item Plan 进入 `work_item_batch_confirm`（包括成功编译后等待整组确认，以及 Final Compile 失败转入批次确认的路径）时，系统 SHALL 在 Cockpit inbox 产生一个与会话稳定关联的待处理门投影。投影 SHALL 标明整组 Draft 确认语义、当前阶段、来源会话和可用动作；同一 durable 门在重连、刷新或重复状态事件后 SHALL 保持同一身份且不得重复生成 inbox 条目。该投影不得重新暴露逐段 draft/generation-mode/review_decision legacy 决策。

#### Scenario: 成功编译后的整组确认可见

- **WHEN** Final Compile 成功产出整组 Work Item Draft 且会话等待用户确认
- **THEN** Cockpit inbox 显示一个「确认整组 Work Item Draft」门，包含可用的确认与终止动作，且门身份稳定

#### Scenario: Compile 失败转批次确认可见

- **WHEN** Final Compile 失败并按既有状态机进入 `work_item_batch_confirm`
- **THEN** Cockpit inbox 显示同一会话的批次确认/可恢复上下文，不要求用户切换 legacy 页面才能发现该门

#### Scenario: 批次门重连不重复

- **WHEN** 客户端断线后重连或重复接收相同批次门状态
- **THEN** Cockpit 以同一门身份更新原条目，不产生重复待处理门，不改变 durable 阶段或确认语义

### Requirement: Compile recovery 门投影与动作面（REQ-PCG-02）

当会话处于可恢复的 `work_item_plan_compile_recovery` 状态时，系统 SHALL 在 Cockpit 投影 recovery 门并展示既有 `WorkItemPlanCompileRecoveryAction` 可用动作及其状态反馈。动作提交 SHALL 经现有 Cockpit action facade/WS 通路发送，成功、拒绝、恢复中和终态结果 SHALL 可诊断地反映在门条目中；recovery 门不复制一套 compile 状态机或改变 action 枚举含义。

#### Scenario: Cockpit 发起继续恢复

- **WHEN** recovery 门处于可操作状态且用户在 Cockpit 选择既有 `continue` 动作
- **THEN** 系统经现有 WS action 通路提交该动作，门显示提交/处理中状态，服务端按既有 recovery 语义继续同一 compile，不创建第二个 compile

#### Scenario: Cockpit 发起回滚或人工分诊

- **WHEN** 用户选择既有 `abort_and_rollback` 或 `human_triage` 动作
- **THEN** 系统提交对应既有动作并展示服务端结果；不得将其错误映射为确认、普通反馈或新的 legacy 决策消息

#### Scenario: recovery 已不可操作

- **WHEN** recovery 状态已结束、门已关闭、会话已进入终态或当前连接不具备 driver 写权限
- **THEN** Cockpit 隐藏或禁用 recovery 写动作并展示可诊断原因，迟到动作零副作用

### Requirement: 投影与业务状态机解耦（REQ-PCG-03）

Cockpit 的批次/recovery 投影 SHALL 由 durable session/timeline/artifact 状态派生，SHALL NOT 通过本地 UI 状态创建虚假门、推进 compile 或改变 Plan 状态。投影缺少必要身份、阶段或状态一致性凭据时 SHALL fail-closed 为不可操作诊断项，而不是猜测默认动作。

#### Scenario: 状态事件顺序变化

- **WHEN** 重连期间状态事件乱序、重复或只收到部分 artifact 更新
- **THEN** Cockpit 按稳定门身份合并可验证状态；无法证明可操作性时保持只读/不可操作，不发送猜测命令

#### Scenario: 终态事件晚到

- **WHEN** `confirmed` 或 `terminated` 终态已由服务端持久化后又收到旧门事件
- **THEN** Cockpit 收口门投影并拒绝旧动作，不重新展示待处理门
