## MODIFIED Requirements

### Requirement: 固定五步真实状态机
每个代码库初始化 operation SHALL 按以下顺序创建且仅创建六个步骤：`cadence_skills`、`pre_check`、`rule_config`、`mcp_configuration`、`project_rules_examples`、`git_finalize`。每一步 SHALL 具有 `pending`、`running`、`completed` 或 `failed` 状态；服务端 SHALL 是这些状态的唯一事实来源。

#### Scenario: 初始化开始时的步骤状态
- **WHEN** 系统创建新的初始化 operation
- **THEN** 系统 SHALL 将六个固定步骤按规定顺序记录为 `pending`

#### Scenario: 真实步骤开始与完成
- **WHEN** 后端即将实际执行某个固定步骤
- **THEN** 系统 SHALL 在开始该调用前将该步骤写为 `running`，并且仅在该调用成功返回后将其写为 `completed`

#### Scenario: 严格顺序推进
- **WHEN** 前一固定步骤尚未处于 `completed`
- **THEN** 系统 SHALL 不得将后续步骤置为 `running`

#### Scenario: 步骤失败
- **WHEN** Cadence-skills 准备或任一 Claude Code 初始化命令失败、超时、取消或要求交互
- **THEN** 系统 SHALL 将当前步骤写为 `failed`、将 operation 写为 `failed`、保留后续步骤为 `pending`，且不得创建 Repository 记录

#### Scenario: Repository 持久化失败
- **WHEN** 五个固定步骤均已完成但 Repository 记录持久化失败
- **THEN** 系统 SHALL 将 operation 写为 `failed`，提供持久化失败的结构化诊断和 Git changed paths，且不得报告 operation 成功

### Requirement: 初始化操作终态一致性
系统 SHALL 仅在五个初始化步骤完成、最终 Git 状态已收集且 Repository 记录持久化成功后将 operation 标为 `completed`。系统 SHALL 保持同一 Git 代码库路径的初始化互斥，并将执行中断转换为可诊断终态，避免无限期 `running`。`git_finalize` 步骤的结果 SHALL NOT 改变已 `completed` 的 operation 终态。

#### Scenario: 全部步骤成功后完成
- **WHEN** 五个初始化步骤、最终 Git 状态采集和 Repository 持久化均成功
- **THEN** 系统 SHALL 将 operation 标为 `completed`，保存最终成功结果，并使后续查询返回相同终态结果

#### Scenario: git_finalize 成功
- **WHEN** operation 已 `completed` 且 `git_finalize` 提交/推送成功或无改动可提交
- **THEN** 系统 SHALL 将 `git_finalize` 标为 `completed`，operation 保持 `completed`，`git_finalize_warning` 为 `None`

#### Scenario: git_finalize 失败不回滚注册
- **WHEN** operation 已 `completed` 且 `git_finalize` 提交或推送失败
- **THEN** 系统 SHALL 将 `git_finalize` 标为 `failed`、保持 operation 为 `completed`、保留已创建的 Repository 记录，并通过 `git_finalize_warning` 提供手动提交推送的提示

#### Scenario: 重复提交同一路径
- **WHEN** 同一 Project 中某 Git 根目录已注册或正在初始化
- **THEN** 系统 SHALL 拒绝新的重复初始化请求，且不得为该路径并发执行两个初始化流程

#### Scenario: 执行进程中断
- **WHEN** 服务在 operation 处于非终态时重启或无法继续该 operation
- **THEN** 系统 SHALL 在 operation 恢复可见时将其呈现为带恢复建议的失败终态，而不得无限期报告其为 `running`

## ADDED Requirements

### Requirement: 初始化完成后自动提交推送
系统 SHALL 在五个初始化步骤成功且 Repository 持久化成功后，在目标仓库 git 根目录执行第六步 `git_finalize`：暂存全部改动、按需提交、并按 remote 与上游情况决定推送。暂存 SHALL 遵守目标仓库 `.gitignore`。

#### Scenario: 暂存并提交初始化改动
- **WHEN** 目标仓库存在初始化产生的未提交改动
- **THEN** 系统 SHALL 执行 `git add -A` 并以消息"初始化cadence-aria 代码库"提交

#### Scenario: 无改动时跳过提交
- **WHEN** `git add -A` 后没有任何暂存改动
- **THEN** 系统 SHALL 跳过提交并将 `git_finalize` 标为 `completed`，不得创建空提交

#### Scenario: 无 remote 时跳过推送
- **WHEN** 目标仓库未配置任何 git remote
- **THEN** 系统 SHALL 跳过推送、将 `git_finalize` 标为 `completed`，并在 `git_finalize_warning` 中说明已跳过推送

#### Scenario: 有 remote 无上游时跳过推送
- **WHEN** 目标仓库配置了 remote 但当前分支没有上游分支
- **THEN** 系统 SHALL 跳过推送、将 `git_finalize` 标为 `completed`，并在 `git_finalize_warning` 中说明已跳过推送

#### Scenario: 有 remote 有上游时推送
- **WHEN** 目标仓库配置了 remote 且当前分支有上游分支
- **THEN** 系统 SHALL 执行 `git push`；推送成功时将 `git_finalize` 标为 `completed`

#### Scenario: 推送失败
- **WHEN** 有 remote 且有上游但 `git push` 失败
- **THEN** 系统 SHALL 将 `git_finalize` 标为 `failed`、保持 operation 为 `completed`，并在 `git_finalize_warning` 中提供手动提交推送提示
