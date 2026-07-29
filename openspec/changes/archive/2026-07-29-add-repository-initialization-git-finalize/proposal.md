## Why

代码库初始化的五个固定步骤会在目标仓库内生成框架配置文件（`.claude/`、`.pi/`、`openspec/`、`cadence/`、`CLAUDE.md`、`AGENTS.md` 等）。当前流程结束后这些文件只停留在工作区，不会被提交，也不会推送到目标仓库远端；用户需要手动进入目标仓库提交，既容易遗漏，也无法在进度面板看到最终状态。

## What Changes

- 在五个固定步骤全部成功、且 Repository 持久化成功之后，新增第六步 `git_finalize`：在目标仓库 git 根目录执行 `git add -A`（遵守目标仓库 `.gitignore`）、按需 `git commit`（消息固定为"初始化cadence-aria 代码库"）、并按 remote/上游情况决定 `git push`。
- 固定步骤数从 5 变为 6：`cadence_skills`、`pre_check`、`rule_config`、`mcp_configuration`、`project_rules_examples`、`git_finalize`。
- 终态语义调整：前五个步骤成功且 Repository 持久化成功后，operation 一定标 `completed`；`git_finalize` 失败只影响其自身步骤状态，不回滚注册、不阻塞完成，前端在成功面板以红色步骤和提示文案引导用户手动提交推送。
- `git_finalize` 跳过 push 或 push 失败的原因通过成功结果中的 `git_finalize_warning` 透出。

## Capabilities

### Modified Capabilities

- `repository-initialization-progress`: 固定步骤从五个扩展为六个，新增 `git_finalize` 步骤与"注册成功不因 git_finalize 失败而回滚"的终态语义。

### New Capabilities

- 无（`non-interrupt-repository-bootstrap` 不受影响；`git_finalize` 不是 Claude Code 命令）。

## Impact

- 后端：`RepositoryInitializationStepKind` 新增 `GitFinalize`、coordinator 在 Repository 持久化后执行 git 操作、成功结果契约新增 `git_finalize_warning`。
- 前端：进度面板从五步扩展为六步，新增 `git_finalize` 失败时的红色步骤与手动提交推送提示文案，成功分支仍在 operation `completed` 时刷新代码库列表。
- 测试：扩展 Rust 单元/集成测试覆盖六步顺序、git add/commit/push 各分支与 git_finalize 失败仍 completed；前端测试覆盖六步渲染与失败提示。
- 不新增第三方依赖；复用既有 git 命令执行与状态机。
