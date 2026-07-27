## ADDED Requirements

### Requirement: Coding Workspace 完成不依赖 TestingReport

在 Testing 阶段未纳入产品流程期间，Coding Workspace 的完成判定 SHALL 以 internal PR review 通过及全部非 testing 完成门禁满足为准。系统 MUST NOT 仅因 required verification check 缺少 Passed testing report 而阻塞 attempt 完成，也 MUST NOT 伪造 testing report 或 testing 成功状态。

#### Scenario: schema v2 group 缺少 testing report

- **WHEN** schema v2 group attempt 的所有 unit 已完成、internal PR review 已通过、其他非 testing 门禁均满足，且绑定的 verification plan revision 含 required verification check 但 attempt 没有 testing report
- **THEN** 系统 SHALL 允许最终完成流程成功，不得返回 `VerificationGateResultMissing`

#### Scenario: legacy group 缺少 testing report

- **WHEN** legacy group attempt 已通过 internal PR review 和其他非 testing 完成门禁，verification plan 含 required gate，但 attempt 没有 matching testing report
- **THEN** 系统 SHALL 允许最终完成流程继续，不得仅因缺少 testing report 阻塞完成

#### Scenario: single attempt 缺少 testing report

- **WHEN** single-attempt 已通过适用的 review 流程和其他非 testing 完成门禁，work item 引用了含 required gate 的 verification plan，但 attempt 没有 testing report
- **THEN** 系统 SHALL 允许最终完成流程继续，不得仅因缺少 testing report 阻塞完成

#### Scenario: 存量 testing report 状态不参与完成判定

- **WHEN** attempt 存在历史 testing report，其状态不是 Passed 或 PassedWithWarnings，但 internal PR review 和全部非 testing 完成门禁均满足
- **THEN** 系统 SHALL 忽略该 report 对完成判定的影响，且 MUST 保留原 report 数据不变

### Requirement: 非 testing 完成门禁继续生效

解除 testing report 依赖后，Coding Workspace MUST 继续执行并强制满足当前路径适用的文件范围、runtime binding、handoff、unit 完成状态、completion commit、共享 worktree 清洁性及其他非 testing 一致性门禁。

#### Scenario: 文件范围校验失败

- **WHEN** internal PR review 已通过且不存在 testing report，但 changed files 违反 work item 或 runtime 声明的写入范围
- **THEN** 系统 MUST 拒绝完成并返回既有文件范围校验错误

#### Scenario: handoff 或 unit 完成条件缺失

- **WHEN** internal PR review 已通过且不存在 testing report，但可见 handoff 缺失、unit 未完成或 completion commit 缺失
- **THEN** 系统 MUST 拒绝完成并返回对应的既有非 testing 门禁错误

#### Scenario: 共享 worktree 不清洁

- **WHEN** internal PR review 已通过且不存在 testing report，但共享 worktree 未满足既有清洁性要求
- **THEN** 系统 MUST 拒绝完成并保持既有恢复/人工处置语义

### Requirement: Testing 基础设施保持兼容

本行为变化 MUST NOT 删除或修改 Testing stage、tester 配置、TestingReport 持久化格式或现有 testing report 数据。系统 SHALL 保留 `VerificationGateResultMissing` 公共错误变体，以避免本 change 造成不必要的公共类型破坏。

#### Scenario: 读取既有 testing report

- **WHEN** 部署本 change 后读取历史 attempt 的 testing report
- **THEN** 系统 SHALL 按原数据模型返回 report，不得因完成门禁放宽而丢失或迁移该数据

#### Scenario: 不新增 testing gate 配置

- **WHEN** 创建或恢复 Coding Workspace attempt
- **THEN** 系统 MUST NOT 要求新增 testing gate 开关或配置字段，现有 API 和持久化结构 SHALL 保持兼容
