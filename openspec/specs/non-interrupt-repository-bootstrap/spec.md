# non-interrupt-repository-bootstrap Specification

## Purpose

以无中断模式（`--no-interrupt`）执行 Cadence-skills 的四个 Claude Code 初始化命令，禁止初始化流程等待人工输入。
## Requirements
### Requirement: 无中断 Claude Code 初始化命令
系统 SHALL 使用 Claude Code 按固定顺序执行四个独立初始化命令，并为每条命令传递完整的 `--no-interrupt` token：`/pre-check --no-interrupt --upgrade 用大陆镜像`、`/rule-config --no-interrupt`、`/mcp-configuration --no-interrupt`、`/project-rules-examples --no-interrupt`。`pre_check` 命令 SHALL 附带 `--upgrade` 与 `用大陆镜像` 参数，其中 `--upgrade` 位于 `--no-interrupt` 之后、`用大陆镜像` 之前；`--upgrade` 的行为由 Cadence-skills 的 `/pre-check` 命令定义，本仓库不解释其语义。第六步 `git_finalize` 不是 Claude Code 命令，SHALL NOT 通过 Provider 执行。

#### Scenario: 执行预检查命令
- **WHEN** `cadence_skills` 步骤已完成且系统开始第二步
- **THEN** 系统 SHALL 以新的 Claude Code Provider turn、目标 Git 根作为工作目录和提示词 `/pre-check --no-interrupt --upgrade 用大陆镜像` 执行该步骤

#### Scenario: 顺序执行其余命令
- **WHEN** 前一个 Claude Code 初始化命令已完成
- **THEN** 系统 SHALL 依次使用 `/rule-config --no-interrupt`、`/mcp-configuration --no-interrupt` 和 `/project-rules-examples --no-interrupt` 开始后续对应步骤，且这些命令 SHALL NOT 附带 `--upgrade`，SHALL NOT 附带 `用大陆镜像` 参数

#### Scenario: git_finalize 不经 Provider
- **WHEN** 四个 Claude Code 命令全部完成且 Repository 持久化成功
- **THEN** 系统 SHALL 由协调器直接对目标仓库执行 git 操作完成 `git_finalize`，不得创建新的 Provider turn

#### Scenario: 命令请求人工交互
- **WHEN** 任一无中断 Claude Code 命令仍产生权限请求或选择请求
- **THEN** 系统 SHALL 中止该 Provider session、将对应固定步骤标为 `failed`，并返回说明需要人工恢复的结构化错误

#### Scenario: 最终摘要保留无中断命令
- **WHEN** 四个 Claude Code 初始化命令全部完成
- **THEN** 最终初始化摘要 SHALL 按实际执行顺序列出四条命令，其中 `pre_check` 条目为 `/pre-check --no-interrupt --upgrade 用大陆镜像`

