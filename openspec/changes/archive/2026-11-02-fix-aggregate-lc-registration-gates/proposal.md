# 提案：聚合 LC 登记门禁与生命周期修正

## 背景

真机操作暴露一个**阻塞性缺陷**：任何做过 Cadence-skills 初始化的多仓项目，其聚合根目录必然包含 `AGENTS.md` / `CLAUDE.md` / `.claude/rules/*`（这正是初始化自己的产物），而登记预检把「根目录存在这些文件」当作**用户文件冲突**直接拒绝，返回 `aggregate_root_ownership_conflict`（UI 只显示笼统的 `aggregate root preflight rejected`）。结果是**这类项目永远无法添加逻辑代码库**，且因为回执只覆盖 `AGENTS.md`、不覆盖 `CLAUDE.md`，连"第二次重跑"也无路可走。

同期还暴露两处缺口：

1. **删掉的 LC 无法从 UI 恢复**：后端有 `restore` 端点，但前端未接，且删除后 id 不可见；「彻底删除」也不存在 —— 软删保留的政策正文永远不会被清理。
2. **同一目录可被两个 LC 占用**：重叠检查跳过同项目，同一项目内可在同一根上登记出两个 LC，政策产物互相覆盖。

## 目标

| # | 目标 |
|---|---|
1 | **登记不再以用户规则文件为门禁**：`AGENTS.md` / `CLAUDE.md` / `.claude/rules/*` / `.mcp.json` / `.gitignore` 的存在不阻塞登记；其余准入校验（非 git 根、成员边界、symlink 逃逸、嵌套 worktree）不变 |
2 | **同一目录全局只能有一个 LC**（跨项目亦然）；同一 LC 的复验/重跑不得被自身拦掉；软删（墓碑）期间仍占用该根 |
3 | **聚合初始化按需跳过 skills**：已初始化（`AGENTS.md` 且 `.claude/rules/language.md` 存在）→ 跳过 4 条无中断命令，只合成并发布聚合政策；未初始化 → 走完整五步 |
4 | **新增聚合「重新初始化」入口**：无视检测结果，强制重跑完整五步（供 Cadence-skills 更新后重装） |
5 | **LC 生命周期完整**：软删（可恢复）+ 恢复 + 彻底删除（硬删，需先软删；清政策产物与成员指针）；前端暴露「已删除」分区与两个按钮 |

## 非目标

- 不改「四命令」的**命令文本与语义**（`/pre-check`、`/rule-config`、`/mcp-configuration`、`/project-rules-examples`）
- 不做全文备份（只留判定摘要与回执）
- 不改 provider 适配层、工具策略、网关信封、流式输入结构
- 不改成员仓的 Git 操作语义
- 不新增 `pointer-publications` 的自动触发（成员指针仍由独立端点发布）

## 范围

| 能力 | 变更 |
|---|---|
`logical-codebase-registration` | MODIFIED REQ-REG-09（去所有权门禁、加同目录全局唯一）；ADDED「LC 生命周期：软删/恢复/彻底删除/已删除列表」 |
`non-interrupt-repository-bootstrap` | MODIFIED（按已初始化状态跳过 skills 四步）；ADDED「聚合重新初始化入口」 |

## 风险与取舍

| 风险 | 处置 |
|---|---|
移除所有权门禁后，聚合根的 `AGENTS.md` 不再被"保护"以防改写 | 聚合初始化**本身不写**根 `AGENTS.md`（政策发布只写 `<root>/policy/<project>/<lc>/<rev>`）；`/rule-config` 仅在**强制重新初始化**路径执行，且判定窗口不变量保证不会静默改判 |
`Skipped` 步骤可能掩盖"该跑没跑" | 跳过必须留判定依据（两文件 sha256）并与三步在同一次持久化写落盘；**一旦有命令审计事实即关窗**，绝不改判；跳过式回执与完整回执互斥 |
硬删不可恢复 | 必须先软删（`logical_codebase_not_tombstoned` 409）；前端二次确认文案写明不可恢复与清理范围 |
