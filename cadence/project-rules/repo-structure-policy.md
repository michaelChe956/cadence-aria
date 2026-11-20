# 仓库结构与合并方式规则（强制）

## 适用范围

本规则适用于本仓库（**bare 主仓 + `git worktree` 功能工作树结构**）内的一切 git 操作，包括所有会话、所有 subagent 派工、harness 的工作树创建流程、以及所有产物与代码的落地提交。

本规则与 `cadence/project-rules/branch-protection.md` 互补：那份管「谁可以提交到 main」，本份管「仓库结构本身怎么保持」与「改动怎么进 main」。

## 强制要求

### 1. `core.bare` 恒为 `true`，禁止任何形式修改

- 根目录 `/home/michaelche/workspace/github/cadence-aria` 是**共享 gitdir 容器**，不是工作区；`.worktrees/` 下的各工作树才是实际检出位置。
- `core.bare=true` 是这条结构的**基石**：它让根目录被 git 视作 bare，从而在根目录执行 `git status` 直接失败（「this operation must be run in a work tree」），杜绝「在根目录顺手改文件」。
- 🔴 **禁止**执行 `git config core.bare false`、`git config --unset core.bare`，或以任何等效方式（含间接写入 `.git/config`）改动该标志。
- 🔴 **禁止**把「bare 改成 false」当作任何操作的前置步骤；确需临时工作区时，应新建工作树，而不是翻转主仓标志。

**已知触发路径（2026-11-20 实测）**：harness 的 `git worktree add` 包装（`omp worktree add`，做 copy-on-write clone）在根目录执行时会因 bare 布局无法遍历目录而回退到 plain checkout；该路径曾把 `core.bare` 翻成 `false` 且未复原。当时 `git worktree list` 把根目录显示成 `main` 的工作区，根目录 `git status` 一次报出 5709 行「已删除」，而这些文件其实都在工作树里完好。**纯 `git worktree add` 不会翻转该标志**（已用对照实验确认），问题出在包装层。

**发现即复原**：`git config core.bare true`，并核验 `git worktree list` 首行仍显示 `(bare)`。

### 2. 所有内容一律经 `.worktrees/` 下的工作树实现

- 任何代码、文档、规则、配置、脚本的**读取之外的改动**，必须在 `.worktrees/<name>/` 的独立工作树中进行。
- 🔴 **禁止**在仓库根目录（`/home/michaelche/workspace/github/cadence-aria`）直接编辑、创建或删除任何项目文件——包括 `src/`、`cadence/`、`web/`、`tests/`。
- 根目录只允许存在：`.git/`（共享 gitdir）、`.worktrees/`（工作树集合），以及历史遗留的空壳目录（清理与否另议，不得据此写入）。
- 工作树创建**不得**改变主仓的 bare 语义（见第 1 条）；创建后必须核验 `git worktree list` 仍把根目录显示为 `(bare)`。
- 测试产物（构建 target、夹具仓、日志、快照）的落盘位置另见 `cadence/project-rules/test-content-tmp-only.md`（一律 `/tmp`）。

### 3. 所有代码合并一律经 PR 落地

- 功能分支 → `main` 的**唯一**通道是 GitHub Pull Request：`gh pr create` → 评审 → `gh pr merge`。
- 🔴 **禁止**在本地把功能分支 merge 进 `main` 后直接推 `origin/main`；🔴 **禁止**绕过 PR 直接 `git push origin main`；🔴 **禁止**对 `main` 强推（force push）。
- 合并后主仓 `main` 工作树只做**只读同步**（`git pull --ff-only origin main`），不得在 `main` 上产生任何新提交。
- PR 合并方式默认 squash；合并提交的作者信息以 PR 平台记录为准。
- 版本发布严格遵循 `cadence/project-rules/release-policy.md`：**先合并进 main 再打 tag 发版**，禁止从未合并的功能分支发版。

## 强制闸门（可执行，不依赖自觉）

### 3.1 共享 `pre-commit` hook 增加 bare 闸门

共享 git hook `<git-common-dir>/hooks/pre-commit` 在原有「拒绝 main 上提交」之外，**先**校验 `core.bare`：

- `core.bare` 不为 `true` → 拒绝提交，并提示复原命令。因为 bare 被翻成 false 时，根目录会变成 `main` 的工作区，main 的提交保护随之失效——这是同一条防线上的两个开关，必须一起锁。
- 仓内维护 hook 的**真源**：`scripts/hooks/pre-commit`。安装：

```bash
install -m755 scripts/hooks/pre-commit "$(git rev-parse --git-common-dir)/hooks/pre-commit"
```

### 3.2 结构核验脚本

`scripts/verify-repo-structure.sh` 做四项只读检查，任一不过即非零退出：

1. `core.bare` 必须为 `true`；
2. 当前工作区不得是仓库根目录；
3. 本地 `main` 不得领先 `origin/main`（领先即说明有绕过 PR 的落地）；
4. 当前分支不得是 `main`。

误改 `core.bare` 时可一键复原：

```bash
scripts/verify-repo-structure.sh --repair-bare
```

## 验证要求

- 修改本规则、`scripts/hooks/pre-commit` 或 `scripts/verify-repo-structure.sh` 后，必须在 `/tmp` 下建临时 git 仓验证四条路径：
  1. `core.bare=true` + 功能分支 → 提交**放行**；
  2. `core.bare=false` + 功能分支 → 提交**被拒**（stderr 含 `core.bare` 与复原命令）；
  3. `core.bare=true` + `main` 分支 → 提交**被拒**（stderr 含规则路径）；
  4. `scripts/verify-repo-structure.sh` 在上述三种状态下退出码与提示正确。
- 每次创建工作树后，必须核验 `git worktree list` 首行仍是 `(bare)`、`git config --get core.bare` 仍是 `true`。
- 交付前用 `git branch --show-current`（或 `git symbolic-ref --short HEAD`）确认当前处于功能分支，再执行提交。
