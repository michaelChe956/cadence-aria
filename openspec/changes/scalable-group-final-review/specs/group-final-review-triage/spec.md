## ADDED Requirements

### Requirement: 新 Group attempt 的最终审查不得调用 Group Reviewer Provider

当 Group attempt 的每个 Work Item 已完成其独立 Coder 与 Code Reviewer 流程后，系统 MUST NOT 启动 Internal Group Reviewer、shard、reduction 或任何替代的自动语义审查 Provider 调用。系统 MUST 直接生成组就绪检查并进入人工最终确认流程。

#### Scenario: 全部 Work Item 独立审查完成

- **WHEN** Group attempt 的全部 Work Item 已完成且各自拥有最新的通过 Code Review 记录
- **THEN** 系统 MUST 生成组就绪检查，MUST NOT 创建 Internal Group Reviewer、shard 或 reduction Provider 运行

#### Scenario: Group Final 不产生 Provider 输出无效门禁

- **WHEN** 新 Group attempt 到达最终审查步骤
- **THEN** 系统 MUST NOT 因不存在的 group Provider 输出创建 `reduction_output_invalid`、shard 输出无效或对应重试门禁

### Requirement: 组就绪检查必须提供客观的人工审查证据

组就绪检查 MUST 从权威执行记录生成每个 Work Item 的 UnitRun `start_commit`、terminal completion commit、完整提交区间及有序提交引用、最新独立 Code Review 的结论/发现/摘要与原始输出证据引用、resolved handoff 和绑定的 Work Item Plan revision。所有文件与 diff 引用 MUST 由 `start_commit..completion_commit` 区间派生，MUST NOT 只使用末尾 commit 的父提交 diff。检查 MUST 验证全部 Unit 已完成、必需 completion commit 与独立审查记录存在、handoff 与依赖解析一致，以及 attempt 的计划 binding 与活跃 revision 一致。

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

#### Scenario: 缺少客观完成证据

- **WHEN** 任一 Work Item 缺少 completion commit、最新独立审查记录、resolved handoff 或与活跃计划不一致的 binding
- **THEN** 系统 MUST 持久化具体的不一致诊断，MUST NOT 允许人工最终确认完成该 attempt

### Requirement: 人工最终确认是新 Group attempt 的唯一最终语义决策

完整的组就绪检查完成后，系统 MUST 向用户展示全部 Work Item 的客观证据并等待明确的人工 Final Confirm。只有该确认才可以使 Group attempt 进入完成终态。人工最终确认 MUST NOT 生成新的 Coder rework、Plan Repair 或自动 Provider 调用。

#### Scenario: 用户确认完整组

- **WHEN** 用户已查看完整组就绪检查并执行明确的 Final Confirm
- **THEN** 系统 MUST 完成该 Group attempt，且 MUST 不启动任何 Group Reviewer Provider

#### Scenario: 人工确认不绕过不完整检查

- **WHEN** 组就绪检查包含未解决的不一致诊断
- **THEN** 系统 MUST 拒绝 Final Confirm 并保持 attempt 未完成

### Requirement: 人工 Final Confirm 保留既有终态完整性检查

完整组就绪检查只决定用户是否具备作出最终语义决定的证据，不得取消既有的 completion binding、基于 UnitRun 完整提交区间的写入范围检查或 shared worktree 清洁性检查。用户执行 Final Confirm 时，系统 MUST 继续执行这些既有终态检查；其失败 MUST 以既有确定性诊断或人工清理动作呈现，MUST NOT 转换为 Group Reviewer finding、Coder rework 或 Plan Repair。

#### Scenario: 范围外残留不被自动处理

- **WHEN** Coder 报告共享 worktree 中存在不属于当前 Work Item `write_policy` 的未跟踪或修改内容，且组就绪检查其余证据完整
- **THEN** 系统 MUST 不自动暂存、提交或删除该残留；若 Final Confirm 命中既有 shared-worktree-clean 检查，系统 MUST 保留其人工清理语义而不是创建新的 Group Review 门禁

### Requirement: 历史 Group Final 产物保持可读且不得触发新 Provider 调用

已持久化的 shard、reduction 或 InternalPrReview 产物 MUST 保持可读，以供历史 attempt 审计。恢复未终态的历史 Group attempt 时，系统 MUST 使用权威记录生成组就绪检查并转入人工最终确认；系统 MUST NOT 为恢复而启动新的 shard、reduction 或 Internal Group Reviewer 调用。权威身份不一致时 MUST 失败关闭并显示诊断。

#### Scenario: 恢复含 reduction 产物的历史 attempt

- **WHEN** 历史 Group attempt 已保存 reduction 原始输出或审查产物但尚未完成
- **THEN** 系统 MUST 保持其可读取，并以当前权威记录生成组就绪检查，MUST NOT 重新调用 reduction Provider

#### Scenario: 历史身份不一致

- **WHEN** 恢复历史 Group attempt 时其 UnitRun、handoff 或 plan binding 无法唯一校验
- **THEN** 系统 MUST 失败关闭并提供身份诊断，MUST NOT 推断或重建 Group Provider 审查结论
