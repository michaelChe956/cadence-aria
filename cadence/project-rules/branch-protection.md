# main 分支保护规则

## 适用范围

本规则适用于本仓库（bare 主仓 + `git worktree` 功能工作树结构）内的一切分支与提交操作，包括所有会话、所有 subagent 派工、所有产物的实施与交付提交。

## 强制要求

### 1. main 分支禁止直接 commit

- main 分支受**双重约束**：共享 git hook `<git-common-dir>/hooks/pre-commit`（拦截 main 上的提交）与本规则文件（约束操作者行为）。
- 共享 hooks 目录被主仓与全部 worktree 复用，因此本约束对任意 worktree 中签出的 main 同样生效。
- 🔴 **禁止**使用 `git commit --no-verify`（或 `-n`）绕过该 hook；需要绕过即意味着操作本身违规。
- 无 HEAD / detached HEAD 场景（例如 bare 主仓本身、未签出提交的 worktree）由 hook 跳过放行，不属于提交行为。

### 2. 实施与交付一律在功能 worktree 分支进行

- 所有实现、修复、文档与产物提交必须在功能 worktree 的功能分支上进行。
- 主仓 `main` 仅作为**合并目标**（merge target），不接受任何直接落地的工作提交。
- 禁止在主仓工作区直接修改文件并提交到 `main`；需要落地时先在功能分支提交，再走合并流程进入 `main`。

### 3. 误在 main 产生 commit 的处置流程

发现 `main` 上存在误提交的工作 commit 时，按以下三步处置，顺序不可颠倒：

1. **备份打 tag**（先保全证据，禁止在打 tag 前做任何 reset/force 操作）：
   `git tag backup/main-pre-reset-<YYYYMMDD> <误提交后的 main HEAD>`
2. **复位到远端**：在承载 `main` 的工作树执行 `git reset --hard origin/main`，使 `main` 回到远端权威状态。
3. **cherry-pick 回功能分支**：切到对应功能 worktree 分支后逐个拣回：
   `git cherry-pick <commit1> <commit2> ...`

禁止在未打备份 tag 的情况下执行 `reset --hard`、强推或删除分支。

## 本次实例（2026-10-15）

- 场景：S7 阶段修复的 3 个 commit 被误提交到 `main`。
- 误提交 commit（`origin/main..backup/main-pre-reset-20261015`）：
  - `74e9cb15` fix: S7 平台确定性交付摘要
  - `d8e962a0` fix: S7 FE接线误删修复
  - `70a654ea` fix: S7 barrel两条既有导出补回
- 复位目标：`origin/main` = `5b978096`（chore: 打开产物自动提交开关）。
- 备份 tag：`backup/main-pre-reset-20261015`（指向 `70a654ea`，保留全部误提交内容）。
- 上述 3 个 commit 已 cherry-pick 回功能 worktree 分支。

## 验证要求

- 修改 hook 或本规则后，必须在 `/tmp` 下建临时 git 仓验证两条路径：`main` 分支 commit 被拒（stderr 含规则路径）、功能分支 commit 放行。
- 交付前用 `git branch --show-current`（或 `git symbolic-ref --short HEAD`）确认当前处于功能分支，再执行提交。
