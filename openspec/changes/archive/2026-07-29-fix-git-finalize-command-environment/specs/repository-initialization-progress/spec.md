# Spec: repository-initialization-progress (delta)

## ADDED Requirements

### Requirement: git_finalize 命令环境包含用户身份解析所需变量

`git_finalize` 步骤执行的所有 git 命令 SHALL 在进程隔离（`env_clear`）语义下注入 `LC_ALL=C`、已验证的用户主目录 `HOME`，以及进程环境中存在时的 `SSH_AUTH_SOCK`；其余环境变量 MUST 保持隔离。注入的 `HOME` SHALL 来自 Web 层 `resolve_user_home` 验证结果或等效注入点，不得硬编码主机绝对路径。

#### Scenario: commit 使用宿主 git 身份

- **WHEN** 目标仓库无仓库级 `user.name`/`user.email` 配置，而注入 HOME 下的 `.gitconfig` 含有完整身份
- **THEN** `git_finalize` 的 `git commit` SHALL 成功，且提交作者身份来自该 `.gitconfig`

#### Scenario: push 可使用宿主 ssh 凭证

- **WHEN** 进程环境存在 `SSH_AUTH_SOCK` 且目标仓库配置了 upstream
- **THEN** `git_finalize` 的 `git push` 子进程 SHALL 携带该 `SSH_AUTH_SOCK` 与注入的 `HOME`

#### Scenario: 其余变量保持隔离

- **WHEN** `git_finalize` 执行任意 git 命令
- **THEN** 子进程环境 SHALL NOT 包含除 `LC_ALL`、`HOME`、`SSH_AUTH_SOCK` 之外的继承变量
