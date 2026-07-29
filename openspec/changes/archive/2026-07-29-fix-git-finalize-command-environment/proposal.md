# Proposal: fix-git-finalize-command-environment

## Why

代码库初始化的 `git_finalize` 步骤通过 `BoundedCommandRunner::run`（`spawn_isolated`，`env_clear()`）执行 git 命令，且 `run_git` 仅注入 `LC_ALL=C`，导致子进程没有 `HOME`：git 读不到 `~/.gitconfig` 的 `user.name`/`user.email`，`git commit` 以 exit 128 失败（`Author identity unknown`）；即使 commit 成功，`git push` 也会因缺少 `HOME`（`~/.ssh`）与 `SSH_AUTH_SOCK` 失败。真实环境已复现（`git_finalize_commit: git command exited Some(128)`），单测因使用录制假 Runner 从未覆盖真实 git 链路。

## What Changes

- `RepositoryRegistrationCoordinator` 新增 `git_environment` 字段：在 `run_git` 中替代内联的 `{LC_ALL: C}`，内容为 `LC_ALL=C` + `HOME`（Web 层已验证的用户主目录）+ 进程环境中存在时的 `SSH_AUTH_SOCK`。
- 构造函数默认从进程环境尽力填充（`HOME`/`SSH_AUTH_SOCK` 存在才注入），并提供 `with_git_environment` 注入点供 Web 层（传入验证过的 home）与测试使用。
- 新增真实 git 链路的回归测试：临时 HOME 内放置 `.gitconfig`，真实执行 `git_finalize` 的 add/commit，断言提交成功且作者身份来自该 `.gitconfig`。
- 其余环境变量保持隔离（`env_clear()` 语义不变）。

## Capabilities

### New Capabilities

无。

### Modified Capabilities

- `repository-initialization-progress`: `git_finalize` 步骤的 git 命令执行环境要求发生变化（新增 HOME/SSH_AUTH_SOCK 透传要求与身份解析场景）。

## Impact

- 后端：`src/product/repository_store/registration.rs`（coordinator 字段与 `run_git`）、`src/web/handlers/repository_registration.rs`（builder 注入 git 环境）、`src/product/repository_store/registration/tests/`（新回归测试与构造点适配）。
- 行为变化：`git_finalize` 的 commit 将使用宿主用户 `~/.gitconfig` 身份；push 可使用宿主 ssh-agent/密钥。其余 git 调用方不受影响（`run_git` 为 coordinator 私有）。
- 存量恢复：本 change 不自动重试历史失败 operation；受影响仓库由用户手动 commit/push 或重新初始化。
