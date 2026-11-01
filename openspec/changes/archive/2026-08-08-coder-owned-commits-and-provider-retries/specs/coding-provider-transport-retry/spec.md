## Purpose

为 Coding 阶段的 Coder 与 Work Item Code Reviewer 提供有限、可审计的技术失败自动恢复，减少短暂 Provider 故障对人工操作的依赖，同时不把业务结论误当作可重试错误。

## ADDED Requirements

### Requirement: Coder 与 Work Item Code Reviewer 的技术失败自动重试有固定上限

Coder 与 Work Item Code Reviewer 的一次 Provider 执行 MUST 在首次调用之外最多自动重试 2 次，因此同一用户或流程触发最多发起 3 次调用。可自动重试的失败仅限于非用户意图的 Provider 技术失败，包括启动错误、流提前关闭、连接或进程中断、Provider 执行超时与可识别的上游临时 5xx 错误。

用户主动取消、权限或选择请求等待（包括等待该交互而超时）、Provider 正常完成后的结构化输出无效、正常 Reviewer finding、验证失败与 Plan Defect MUST NOT 自动重试。

#### Scenario: Coder 因临时 503 自动恢复

- **WHEN** Coder 的首次调用因可识别的上游 503 失败
- **THEN** 系统 MUST 自动发起下一次 Coder 调用，且该失败 MUST NOT 增加 Coder rework 计数或立即创建人工门禁

#### Scenario: Reviewer 连续技术失败后转人工

- **WHEN** Work Item Code Reviewer 的首次调用及两次自动重试均因技术失败而未完成
- **THEN** 系统 MUST 停止自动重试，并落地既有 Reviewer 中断人工处置入口

#### Scenario: 正常完成但结构化输出无效不自动重试

- **WHEN** Reviewer 已正常完成调用，但其输出不符合结构化审查契约
- **THEN** 系统 MUST 按既有输出无效或人工分诊语义处理，MUST NOT 将该结果计为技术失败自动重试

#### Scenario: 用户取消不自动重试

- **WHEN** 用户在 Coder 或 Work Item Code Reviewer 运行中取消 attempt
- **THEN** 系统 MUST 停止当前调用，MUST NOT 发起自动重试

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

### Requirement: 人工重试在自动预算耗尽后重新取得有限自动恢复机会

当自动重试耗尽后，用户通过既有 Coder 或 Reviewer 人工重试动作发起新的调用周期时，该新的用户授权周期 MUST 再次适用“首次调用之外最多自动重试 2 次”的限制。系统 MUST 保留该人工触发与先前耗尽周期之间的审计关联；Coder 人工重试也必须建立该关联，不得仅以新的 `initial` role run 覆盖其来源。

#### Scenario: 人工重试开启新的有限周期

- **WHEN** Reviewer 的自动重试耗尽并进入人工门禁，用户选择重试代码审查
- **THEN** 系统 MUST 启动一个新的用户授权调用周期；该周期最多包含首次调用和两次自动重试，并 MUST 保留与上一个耗尽周期的关联
