# 提案：coder 写入范围可见性与工作树本地产物屏蔽

## 背景

真机操作暴露一条**越界链**：coder agent 修改了 `.gitignore`，而 `.gitignore` 不在该 Work Item 的允许范围内 → 收口门禁判 `work_item_diff_scope_violation`。链条有两环：

1. **本地产物污染 `git status`**：平台在工作树里产生的本地产物（`.aria/` 证据令牌、`.codegraph/` 本地索引、`.verify/` 验证产物、`.worktrees/` 平台自建工作树）**没有被任何 ignore 规则覆盖**（实测 `.gitignore` 只覆盖 `.codegraph/`、`.worktrees/`、`.aria/`，`.verify/` 是净新增）→ agent 看到 untracked 噪音 → **自行把这些目录登记进 `.gitignore`** → 改了一个被跟踪的仓库文件 → 越界。
2. **范围语义缺失**：agent 看到「允许修改范围 / 禁止修改范围」只有**裸列表**，没有「范围之外一律禁止」的默认拒绝语义，也没有「本地产物已由平台屏蔽」的告知 → 即使 `git status` 干净，仍可能顺手整理 `.gitignore`。

同时暴露两个**载体层缺陷**：

3. **reviewer 看不到范围**：范围信息原本只随编译后 Work Item 上下文走，而 reviewer Prompt 与评估上下文都只读这一条通路 → 若把范围改由 coder 专用通道携带，reviewer 将失去判定「这个文件是否越界」的能力。
4. **本地产物清单有两份真源**：exclude 写入侧与 Work Item 提交过滤侧各一份，且提交过滤**漏掉了 `.aria/evidence-token`** → exclude 一旦失效，令牌文件会被 `git add -A` 暂存进提交。

## 目标

| # | 目标 |
|---|---|
1 | **本地产物在 `git status` 里不可见**：工作树准备阶段把平台本地产物写入**仓库本地** ignore 文件（`<common gitdir>/info/exclude`），**绝不修改 `.gitignore`**（改它本身即越界） |
2 | **写入范围以门禁同源的规范语义呈现**：判定基准 / 匹配语义（非 glob）/ 空列表语义 / 默认拒绝句，coder 与 reviewer 同源，**同一份 Prompt 内只有一份范围** |
3 | **范围未知时 fail-closed**：取不到已确认 Work Item 记录时，必须明确「不得改动任何仓库文件」，MUST NOT 渲染成「无限制」 |
4 | **本地产物清单单一事实源**：exclude 写入与提交过滤共用一份清单；写入幂等、并发安全、落 common gitdir |

## 非目标

- 不改机器门禁的**判定语义**：`**/x` 前缀为空导致等于全放行、门禁硬编码大小写敏感，这两项是既有缺陷，**另开单**，不混入本 change（改判定语义风险大、会让本批难以回滚）。
- 不把 `cadence/` / `openspec/` / `.claude/` / `.agents/` / `.codex/` / `.pi/` / `.kimi-code/` / `.omp/` 纳入 exclude —— 实测这些目录的文件**都已被 git 跟踪**（1519 / 591 / 52 / 18 / 38 / 12 / 6 / 1 个 tracked 文件），不是 `git status` 噪音源；排除它们会让新建文档从 `git status` / `git add -A` 消失，直接砸掉项目工作流。
- 不做远端 runner 的 exclude 同步（当前 coding 执行均在本地）。
- 不改用户自有的 `.gitignore` 内容（只读，不编辑）。
