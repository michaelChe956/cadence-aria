# Tasks: per-issue-base-branch

**全局边界**：创建后锁定；存量零迁移（None=main）；不改 worktree 复用语义；prompt 不承担安全边界。

## 1. 模型与创建入口

- [ ] 1.1 issue 记录增 base_branch（serde default None）+ REST 创建/读取透传 + 创建时分支存在校验（fail-closed 拒绝）；红绿：选分支/不存在拒/存量兼容三用例（REQ-PIB-01 场景 1-2、4）。
- [ ] 1.2 UI 创建表单分支选择器（列仓库分支）+ 修改拒绝呈现（场景 3）。

## 2. author 上下文基线限定（F-57 根治）

- [ ] 2.1 通道审计：author 文件访问全通道清单（provider bridge 逐个）落报告——决定路由落点。
- [ ] 2.2 文件读取/浏览路由经 base 树（git show/ls-tree 形态）；兄弟 worktree/工作区路径不可达；红绿：F-57 现场形态复现用例（author 找不到 .worktrees 下文件）+ 所见=基线（REQ-PIB-02 场景 1-2）。
- [ ] 2.3 基线消失 fail-closed 诊断（场景 3）。

## 3. coding fork 与核对同源

- [ ] 3.1 create_branch base 取 issue.base_branch；创建时基线存在校验；红绿：fork 自所选分支（REQ-PIB-03 场景 1）。
- [ ] 3.2 C1 plan_baseline_tree 参数源统一；三面同源断言用例（场景 2）。

## 4. 门禁收口

- [ ] 4.1 lib/it_core/it_web/vitest 全量+strict；F-56/F-57 现场回放（issue 基线含 status.html 时 AC 合法引用通过 / main 基线时 author 看不到）。
