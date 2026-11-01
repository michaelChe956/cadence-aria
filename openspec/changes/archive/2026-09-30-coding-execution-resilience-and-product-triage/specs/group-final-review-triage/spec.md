# Spec Delta

## MODIFIED Requirements

### Requirement: 组就绪检查必须提供客观的人工审查证据

组就绪检查 MUST 从权威执行记录生成每个 Work Item 的 UnitRun `start_commit`、terminal completion commit、完整提交区间及有序提交引用、最新独立 Code Review 的结论/发现/摘要与原始输出证据引用、resolved handoff 和绑定的 Work Item Plan revision。所有文件与 diff 引用 MUST 由各 execution 真实的 `start_commit..completion_commit` 区间聚合派生，MUST NOT 只使用末尾 commit 的父提交 diff，MUST NOT 把两个 execution 区间之间由人工产生的提交归属给当前 Work Item。检查 MUST 验证全部 Unit 已完成、必需 completion commit 与独立审查记录存在、handoff 与依赖解析一致，以及 attempt 的计划 binding 与活跃 revision 一致。若检查存在经用户批准的验证处理记录，检查 MUST 将其与对应 check 一并展示。

该检查 MUST NOT 生成代码语义 finding、重新解释 Code Review 结论、执行新的文件写入范围判断或自动启动 Coder rework 与 Plan Repair。`start_commit` 与 completion commit 相同 MUST 表示空观察区间，而非把该起始提交相对父提交的改动归属给当前 Work Item。

#### Scenario: 组就绪检查完整

- **WHEN** 全部 Work Item 的权威 UnitRun、completion commit、独立审查、handoff 和计划 binding 均一致
- **THEN** 系统 MUST 持久化包含这些证据的完整组就绪检查，并允许进入人工最终确认

#### Scenario: Coder rework 的全部提交在人工审查中可见

- **WHEN** 一个 Work Item 在同一 UnitRun 内先由 Coder 创建提交、后经 Code Review rework 创建新的 terminal commit
- **THEN** 就绪检查和人工 Final 面板 MUST 展示从该 UnitRun `start_commit` 至 terminal commit 的完整提交区间、相关 diff/evidence 与独立审查结论，不得只展示返修提交

#### Scenario: Coder 未产生新的可观察提交

- **WHEN** 一个 Work Item 的 terminal completion commit 等于其 UnitRun `start_commit`
- **THEN** 就绪检查 MUST 将其展示为无可观察 Git 增量，并保留该 Coder 的原始输出引用；系统 MUST NOT 将起始提交的父提交 diff 显示为当前 Work Item 的证据

#### Scenario: 带人工 WIP 的零提交重跑可确认

- **WHEN** 某 Work Item 先有一次失败 execution，用户随后在工作树中留下人工提交，重试 execution 零提交完成且其余证据完整
- **THEN** 就绪检查 MUST 把该重试 execution 显示为空区间，人工提交不计入该 Work Item，人工 Final Confirm 可执行

#### Scenario: 越界提交仍被拒绝

- **WHEN** 某 execution 真实区间内包含超出当前 Work Item `write_policy` 的提交
- **THEN** 既有终态写入范围检查 MUST 仍在 Final Confirm 时拒绝，不因区间派生方式改变而放行

#### Scenario: 缺少客观完成证据

- **WHEN** 任一 Work Item 缺少 completion commit、最新独立审查记录、resolved handoff 或与活跃计划不一致的 binding
- **THEN** 系统 MUST 持久化具体的不一致诊断，MUST NOT 允许人工最终确认完成该 attempt
