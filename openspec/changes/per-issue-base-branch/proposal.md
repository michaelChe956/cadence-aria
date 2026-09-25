# Proposal

## Why

F-57 实证：design/plan author 的文件系统浏览泄漏到兄弟 issue worktree（`.worktrees/aria-issues/issue_0001/`），把其他分支的交付物当「既有契约」写入 spec（F-56 连锁：plan AC 引用基线外文件迫使 coder 越界）。根因：issue 没有显式基准分支概念——author 上下文浏览无基线限定、coding worktree fork base 隐式取 main、AC 核对树与 author 所见树可能不同源。用户裁决方案：**issue 创建时设置基准分支，全链（story/design/plan 生成 + coding worktree）统一锚定**。

## What Changes

1. **Issue 模型增 `base_branch`**：创建时显式设置（UI 选择器列出仓库分支 + REST 字段；默认 main）；创建时校验分支存在（fail-closed）；创建后锁定（改基线=新 issue——防已生成产物与基线错位）；存量 issue 缺省视为 main（零迁移）。
2. **author 上下文基线限定（F-57 根治）**：story/design/plan 生成的仓库文件浏览/读取以 issue.base_branch 的树为基线语义——host-served 通道（kimi）通道层硬路由（git show/ls-tree+terminal 拒绝，SHALL NOT 访问工作区/兄弟 worktree）；provider 原生通道（claude/codex/pi）prompt 软限制教学块（用户 2026-09-25 裁决，残余风险接受，AC 路径形态由核对兜底）；沙箱（专用检出根）遗留后续。
3. **coding worktree fork base 接线**：`create_branch` 的 base 从 issue.base_branch 取（现隐式 main）。
4. **C1 AC×基线核对统一取值**：plan_baseline_tree 从 issue.base_branch（与 author 所见、coding fork 同源）。

## Capabilities

### New Capabilities

- `per-issue-base-branch`: issue 基准分支的设置、锁定与全链锚定契约（author 上下文/coding fork/AC 核对三面同源）

### Modified Capabilities

- `work-item-plan-single-candidate`: REQ-WSC-02 的 AC 基线核对树来源改为 issue.base_branch（措辞从「worktree fork base」统一）

## Non-Goals

- 不做基线分支的创建后修改/迁移（锁定语义）
- 不做多仓 issue 的 per-repo 差异基线（单仓默认仓库的分支；多仓后续演进）
- 不做分支删除后的自动切换（运行时分支消失→明确报错人工处置）
- 不改 coding 段已有的 worktree 复用/幂等语义（只换 fork base 来源）
