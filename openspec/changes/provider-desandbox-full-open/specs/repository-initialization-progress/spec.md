# 规范增量：仓库初始化 git_finalize 宿主继承环境

<!-- 来源：openspec/specs/repository-initialization-progress/spec.md:109-127。去沙箱实现删除 ProcessManager 的 env_clear（宿主继承 + 显式覆盖层）后，原「进程隔离（env_clear）…其余环境变量 MUST 保持隔离」与实现及本 change 的宿主继承基线直接矛盾；本 delta 只修正该唯一冲突要求，其余初始化步骤/终态合同不动。要求名与场景名按 openspec validate --strict 的场景守恒规则保留为匹配锚（strict 禁止在 MODIFIED 中改名/丢场景），语义以正文为准：「其余变量保持隔离」场景内的旧隔离结论已退役。 -->

## MODIFIED Requirements

### Requirement: git_finalize 命令环境包含用户身份解析所需变量

`git_finalize` 步骤执行的所有 git 命令 SHALL 继承宿主进程环境，并由平台以显式覆盖层叠加 `LC_ALL=C`、已验证的用户主目录 `HOME`，以及进程环境中存在时的 `SSH_AUTH_SOCK`；其余宿主变量随继承到达子进程，平台不构造、不清空、不额外覆盖隔离环境。覆盖层 `HOME` SHALL 来自 Web 层 `resolve_user_home` 验证结果或等效注入点，不得硬编码主机绝对路径。

#### Scenario: commit 使用宿主 git 身份

- **WHEN** 目标仓库无仓库级 `user.name`/`user.email` 配置，而覆盖层 `HOME` 下的 `.gitconfig` 含有完整身份
- **THEN** `git_finalize` 的 `git commit` SHALL 成功，且提交作者身份来自该 `.gitconfig`

#### Scenario: push 可使用宿主 ssh 凭证

- **WHEN** 进程环境存在 `SSH_AUTH_SOCK` 且目标仓库配置了 upstream
- **THEN** `git_finalize` 的 `git push` 子进程 SHALL 携带该 `SSH_AUTH_SOCK` 与覆盖层 `HOME`

#### Scenario: 其余变量保持隔离

- **WHEN** `git_finalize` 执行任意 git 命令
- **THEN** 子进程环境 SHALL 继承全部宿主变量并叠加 `LC_ALL`、`HOME`、`SSH_AUTH_SOCK` 显式覆盖层；平台 SHALL NOT 构造隔离环境、清空或剔除其余宿主变量（原 `env_clear`「其余变量保持隔离」语义已退役，其余变量随继承到达子进程）
