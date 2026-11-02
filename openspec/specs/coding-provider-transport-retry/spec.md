# coding-provider-transport-retry Specification

## Purpose

为 Coding 阶段的 Coder 与 Work Item Code Reviewer 提供有限、可审计的技术失败自动恢复，减少短暂 Provider 故障对人工操作的依赖，同时不把业务结论误当作可重试错误。

## Requirements

### Requirement: Coder 与 Work Item Code Reviewer 的技术失败自动重试有固定上限

Coder 与 Work Item Code Reviewer 的一次 Provider 执行周期 MUST 在首次调用之外最多自动重试2次，因此该cycle最多3次调用

#### Scenario: 上限不重排既有窗口组合

- **WHEN** 设定本自动重试上限与既有教学/repair 窗口组合
- **THEN** 本上限不重排已存在的恰一次coder教学窗口或group-review一次repair窗口最大组合

#### Scenario: 可自动重试的失败范围

- **WHEN** 判定失败类型是否可作 transport 技术失败自动重试
- **THEN** 可自动transport重试的失败仅限非用户意图的Provider技术失败，包括启动错误、流提前关闭、连接或进程中断、Provider执行超时与可识别上游临时5xx错误；L1消费既有剩余机会而不扩池

#### Scenario: 业务结论类失败不自动重试

- **WHEN** 出现用户主动取消、权限或选择请求等待、正常 Reviewer finding、开放验证失败或 Plan Defect
- **THEN** 用户主动取消、权限或选择请求等待（包括等待该交互而超时）、正常 Reviewer finding、开放验证失败与 Plan Defect MUST NOT 作为transport技术失败自动重试

#### Scenario: 结构化输出无效不伪记技术失败

- **WHEN** Provider 正常完成后的结构化输出无效
- **THEN** Provider正常完成后的结构化输出无效 MUST NOT 伪记技术失败

#### Scenario: L1 确定性结构拒收沿原授权修复

- **WHEN** 服务端证实的L1确定性结构拒收出现
- **THEN** 但服务端证实的L1确定性结构拒收 SHALL 按 workspace-structural-governance 沿原角色授权、teaching/repair/cycle剩余机会机器指令自动修复与全量复验，不要求用户写技术prompt

#### Scenario: 机会池不扩张

- **WHEN** coder 教学窗口、每cycle 上限与 group-review repair 窗口共同作用
- **THEN** coder既有一次teaching窗口、每cycle最多3及group-review既有一次repair窗口均不增机会、不叠新池，共用耐久parent identity；权限/业务判断不自动放宽

#### Scenario: Coder 因临时 503 自动恢复

- **WHEN** Coder 的首次调用因可识别的上游 503 失败
- **THEN** 系统 MUST 自动发起下一次 Coder 调用，且该失败 MUST NOT 增加 Coder rework 计数或立即创建人工门禁

#### Scenario: Reviewer 连续技术失败后转人工

- **WHEN** Work Item Code Reviewer 的首次调用及两次自动重试均因技术失败而未完成
- **THEN** 系统 MUST 停止自动重试，并落地既有 Reviewer 中断人工处置入口

#### Scenario: 正常完成但结构化输出无效不自动重试

- **WHEN** Reviewer 已正常完成调用，但其输出不符合结构化审查契约
- **THEN** 系统 MUST 保留真实结构拒收原因，服务端证实L1且有原授权剩余机会时携机器指令自动修复；无授权/机会或未知诊断则工程故障/原协议恢复，MUST NOT 将结果计为transport失败或新增独立池

#### Scenario: 用户取消不自动重试

- **WHEN** 用户在 Coder 或 Work Item Code Reviewer 运行中取消 attempt
- **THEN** 系统 MUST 停止当前调用，MUST NOT 发起自动重试

#### Scenario: coding教学与review修复不叠新池

- **WHEN** coder一次teaching机会或group-review一次repair机会已消费后又出现同类L1
- **THEN** 不另建L1调用池，不自动新run授权，不重置cycle/teaching/repair耐久身份；保既有最大组合，不改为统一三次总限，业务结论与自动rework仍按原合同

### Requirement: 每次自动重试必须保留独立可审计运行记录

每一次首次调用和自动重试 MUST 分别持久化为独立的 role run，并记录角色、调用周期标识、周期内调用序号、触发类型、失败原因、原始输出引用和终态。前一次技术失败的 role run MUST 保留失败状态；后续成功 MUST 不覆盖其记录。Provider 特有的新会话恢复也属于一次独立调用，MUST NOT 作为单一 role run 内部不可见的嵌套重试。

前端 MUST 能区分“正在自动重试”和“自动重试已耗尽、等待人工处理”。

#### Scenario: 第一次失败后第二次成功

- **WHEN** Coder 的首次调用技术失败而第二次调用成功
- **THEN** 系统 MUST 保留两个独立 role run：首次为失败并带技术失败原因，第二次为完成并带其自身的原始输出引用

#### Scenario: 自动重试可见

- **WHEN** 系统正在执行第 1 次或第 2 次自动重试
- **THEN** 前端 MUST 显示当前为自动重试及其序号，且不得显示为用户已经需要操作的 blocked gate

#### Scenario: 权限等待超时不消耗技术重试预算

- **WHEN** Provider 已发出权限或选择请求，之后在等待该人工响应时超时
- **THEN** 系统 MUST 保留该 role run 和人工交互状态，MUST NOT 发起自动重试，也 MUST NOT 消耗两次技术重试预算

### Requirement: 自动重试与 Coder rework、Plan Repair 按职责隔离

Provider 自动重试 MUST 仅恢复未完成的同一角色调用，MUST NOT 递增 `rework_count`、创建 rework instruction、改变 Review verdict 或启动 Plan Repair。只有 Provider 正常完成并产生符合既有契约的业务结论后，系统才 MAY 按既有规则进入 Coder rework 或 Plan Repair。

#### Scenario: 自动重试成功后再进入 Coder rework

- **WHEN** Reviewer 经过一次自动重试后正常完成并给出需要修改的 implementation finding
- **THEN** 系统 MUST 仅在该正常审查结论之后进入既有 Coder rework 流程，自动重试次数 MUST 不计入 rework 次数

#### Scenario: Coder rework 中发现 Plan Defect

- **WHEN** Coder 在一次 rework 调用正常完成后输出符合契约的 Plan Defect
- **THEN** 系统 MUST 按既有 Plan Repair 流程暂停该 attempt，MUST NOT 将 Plan Defect 作为 Provider 技术失败重试

### Requirement: 自动重试使用适合角色的完整执行上下文

Coder 的自动重试 MUST 在同一 worktree 上以新的完整执行上下文启动，使其能够检查先前中断调用可能留下的改动；Work Item Code Reviewer 的自动重试 MUST 使用新的只读审查会话重新读取当前 worktree。任何 Provider 特有的恢复调用也 MUST 计入上述两次自动重试预算。

#### Scenario: Coder 中断后保留部分 worktree 改动

- **WHEN** Coder 在中断前已向共享 worktree 写入部分改动
- **THEN** 下一次自动重试 MUST 在同一 worktree 上以完整上下文启动，要求 Coder 先检查现有改动后再继续

#### Scenario: Provider 特有恢复不突破预算

- **WHEN** 某个 Provider 需要以新会话替代失效恢复会话
- **THEN** 该新会话调用 MUST 占用自动重试预算，且同一触发最多仍为首次调用加两次自动重试

#### Scenario: Reviewer 重试不能重用污染基线
- **WHEN** Reviewer 技术失败符合自动重试条件
- **THEN** 系统 MUST 先收口前轮窗口；无违规且恢复完整才可按原预算建新窗口、新基线和材料，任何违规事实保留并转人工，不沿用旧报告或把污染态作为下一轮基线

### Requirement: 人工重试在自动预算耗尽后重新取得有限自动恢复机会

当自动重试耗尽后，用户通过既有 Coder 或 Reviewer 人工重试动作发起新的调用周期时，该新的用户授权周期 MUST 再次适用“首次调用之外最多自动重试 2 次”的限制。系统 MUST 保留该人工触发与先前耗尽周期之间的审计关联；Coder 人工重试也必须建立该关联，不得仅以新的 `initial` role run 覆盖其来源。

#### Scenario: 人工重试开启新的有限周期

- **WHEN** Reviewer 的自动重试耗尽并进入人工门禁，用户选择重试代码审查
- **THEN** 系统 MUST 启动一个新的用户授权调用周期；该周期最多包含首次调用和两次自动重试，并 MUST 保留与上一个耗尽周期的关联

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
