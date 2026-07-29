# Design: 初始化完成后自动提交推送（git_finalize）

## Context

代码库注册流程在 `execute_initialization` 中按固定顺序执行五步（cadence_skills 与四个 Claude Code 命令），随后采集 Git 状态、计算 changed_paths、持久化 Repository，最后将 operation 标为 `completed`。本设计在这之后追加第六步 `git_finalize`，对目标仓库执行提交与推送，并调整终态语义使其失败不回滚已成功的注册。

## Decisions

### 1. git_finalize 是第六个固定步骤，但不是 Claude Code 命令

`RepositoryInitializationStepKind` 新增 `GitFinalize`，置于 `ALL` 数组末尾，固定步骤顺序为 `cadence_skills`、`pre_check`、`rule_config`、`mcp_configuration`、`project_rules_examples`、`git_finalize`。

- `GitFinalize` 的 `command()` 返回 `None`：它不是 Claude Code Provider turn，而是由 coordinator 直接执行的 git 操作序列。
- `from_command_index` 保持不变：它只把 `1..=4` 映射到四个 Claude 命令，`git_finalize` 没有 command_index。
- 前端 `STEP_LABELS` 增加 `git_finalize: "提交并推送"`，进度面板自然渲染第六步。

**替代方案**：把 git_finalize 做成第五个 Claude 命令之后的内部动作、不进入步骤模型。拒绝原因：用户明确要求在页面上看到该步骤的成功/失败状态并据此手动处理，只有进入步骤状态机才能复用既有的红色步骤展示与状态播报。

### 2. git_finalize 在 Repository 持久化之后执行，失败不回滚注册

执行顺序：五个步骤成功 → 采集 Git 状态与 changed_paths → 持久化 Repository → 执行 `git_finalize` → operation 标 `completed`。

- 前五个步骤任一失败：operation `failed`，不创建 Repository（维持现状）。
- Repository 持久化失败：operation `failed`（维持现状）。
- 前五个步骤成功且 Repository 持久化成功：operation **一定** `completed`，无论 `git_finalize` 成功或失败。

**替代方案**：git_finalize 失败则整个 operation 失败。拒绝原因：此时 Repository 已注册可用，回滚会让用户误以为"添加代码库失败"，而实际只是提交推送未完成；按用户确认的 Y 方案，注册成功优先，git_finalize 是收尾增强。

### 3. git_finalize 的 git 操作序列

在目标仓库 git 根目录，按序：

1. `git add -A`：暂存全部改动，自动遵守目标仓库 `.gitignore`（被忽略的 `.mcp.json`、`.codex/` 等不进入暂存）。
2. 检查暂存改动：若无任何暂存内容，跳过 commit，第 6 步 `completed`（不创建空 commit）。
3. `git commit -m "初始化cadence-aria 代码库"`。
4. push 决策：
   - 无 `git remote`：跳过 push，记 warning，`completed`。
   - 有 remote 但当前分支无上游（`@{u}` 解析失败）：跳过 push，记 warning，`completed`。
   - 有 remote 且有上游：`git push`；失败（网络/权限/非快进）→ 第 6 步 `failed`，记 push 错误。

commit 失败（如 hooks 拒绝、user.name/email 未配置）→ 第 6 步 `failed`。

### 4. 结果契约透出 git_finalize_warning

`RepositoryRegistrationSuccess` 增加 `git_finalize_warning: Option<String>`：

- push 跳过：`git_finalize_warning = Some("git_finalize: 无 remote/无上游，已跳过 push，请手动推送")`。
- push 失败：第 6 步 `failed`，`git_finalize_warning = Some("git_finalize: push 失败（摘要），请手动提交推送")`。
- 完全成功：`git_finalize_warning = None`。

operation 成功结果与 HTTP DTO 透出该字段；前端在 `git_finalize` 红色步骤旁展示提示文案"自动提交推送未完成，请在目标仓库手动执行 git commit / git push"。

### 5. 步骤状态机校验

operation 创建时生成六个 `pending` 步骤；`git_finalize` 遵循与其他步骤相同的 `pending → running → completed/failed` 转换与严格顺序推进。六个步骤的状态仍是服务端唯一事实来源。

## Risks

- 目标仓库无 remote/上游时 push 被跳过，需在 warning 中清晰说明，避免用户误以为已推送。
- `git add -A` 会暂存初始化命令之外的其他未提交改动（若目标仓库工作区本就有未提交内容）。接受：用户确认范围 A（`git add -A`），且这些改动本就属于该仓库工作区。
- git hooks 或凭证缺失导致 commit/push 失败：如实标 `failed` 并提示手动处理。
