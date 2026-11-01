# Spec Delta

## ADDED Requirements

### Requirement: 单仓初始化失败的通知与恢复后继续（REQ-INIT-C5-RESUME）

单仓代码库登记 SHALL 继续使用现有 Claude 初始化流程（固定步骤与 `git_finalize` 语义不变）。operation 以 `failed` 终态结束时（含 provider 不可用、命令失败、要求交互与执行中断），系统 SHALL 在驾驶舱以稳定事实 key 呈现一次失败等待项，内容至少包括失败步骤、已完成步骤、结构化诊断（失败原因码、provider、可能的 changed paths、是否可重试与建议动作）以及“网关恢复后继续”操作的效果说明。该操作 SHALL 以失败 operation 冻结的登记输入重新提交一个新的初始化 operation，MUST NOT 复活、改写或删除原失败 operation 及其证据；同一 command_id 重放 SHALL 返回同一个新 operation，同一路径已有登记或正在初始化时 SHALL 按既有互斥规则拒绝。provider/网关仍不可用时新 operation SHALL 再次以失败终态停等并保留诊断，MUST NOT 伪造成功、MUST NOT 直写仓库登记数据或绕过初始化步骤；本需求 MUST NOT 引入 Claude 以外的初始化 recipe。

#### Scenario: 网关不可用时停等并通知

- **WHEN** 单仓登记在 Claude 网关或账号池不可用时失败
- **THEN** operation 为 `failed`，不创建 Repository 记录；驾驶舱出现一次包含失败步骤、诊断与“恢复后继续”按钮的等待项，不自动重试

#### Scenario: 恢复后继续完成登记

- **WHEN** 网关恢复后用户点击“恢复后继续”
- **THEN** 系统以原冻结输入创建新 operation 并执行完整初始化步骤直至 `completed`，原失败 operation 保持只读可查，等待项转为已处理

#### Scenario: 重复点击与路径互斥

- **WHEN** 用户以同一 command_id 重复提交，或该路径已有正在进行的初始化
- **THEN** 同键返回同一新 operation；路径互斥时返回既有冲突错误，不并发执行两个初始化

#### Scenario: 仍不可用不伪造成功

- **WHEN** 用户点击继续但网关仍不可用
- **THEN** 新 operation 再次失败并保留诊断，Repository 记录不被创建，仓库登记数据不被直接写入
