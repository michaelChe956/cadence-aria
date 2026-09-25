# Tasks: per-issue-base-branch

**全局边界**：创建后锁定；存量 None=默认链（皆无=读取 OK/生成 coding fail-closed）；列表仅本地 refs/heads 不隐式 fetch；prompt 不承担安全边界（含本期新增的 native 基线教学块——其为软约束非边界）；provider 原生 author 会话无通道层基线边界运行（用户裁决 2026-09-25）：残余风险经 C1 AC×基线硬核对（plan 期，路径形态）部分兜底，其余形态为用户明示接受；不改 worktree 复用语义。

## 1. 模型与创建入口

- [x] 1.1 issue 记录增 base_branch（Option）+REST 透传+创建校验（分支存在/皆无默认链时未选即拒）；红绿：选分支/默认链两态/皆无强制手选/不存在拒/存量兼容五用例（REQ-PIB-01 场景 1-3、5-6）。
- [x] 1.2 UI 分支选择器（本地分支列表+默认值预选）+修改拒绝呈现（场景 4）。

## 2. author 上下文基线限定（F-57 根治）

- [x] 2.1 全通道审计报告（四形态×逐 provider 落点+已知漏洞核查+旁路检测+分级路由定案）——pib-audit-report.md。
- [x] 2.2 通道分级实施：host-served（kimi fs）→git show/ls-tree 树路由+terminal 基线拒绝（硬边界）；provider 原生（claude/codex/pi）→prompts.rs 两族 builder 注入基线教学块（provider 无关，kimi 同注入）+BaselineTreeRef 两级语义注释；红绿：kimi F-57 现场复现（找不到 .worktrees/兄弟件）+所见=基线+native 会话启动成功且 prompt 含教学块断言三用例（REQ-PIB-02 场景 1-2、4）。
- [x] 2.3 基线消失 fail-closed 诊断+存量皆无仓失败诊断（场景 3+存量场景）。

## 3. coding fork 与核对同源

- [x] 3.1 各上游入口隐式基线统一改读 issue.base_branch（coding.rs:157-158/group.rs:105-106 等全清单；advance_split 按 Non-Goal 保持现状）+创建时校验；红绿：fork 自所选分支（REQ-PIB-03 场景 1）。
- [x] 3.2 C1 plan_baseline_tree 改按分支名取树（不依赖共享 worktree）+不可解析 fail-closed（废弃跳过——**软限制唯一硬兜底，优先级最高**）；红绿：三面同源+核对不可解析不跳过+软限制漏网 AC 路径被拦回放（场景 2-3+REQ-PIB-02 场景 5）。

## 4. 门禁收口

- [x] 4.1 全量四门禁+strict+vitest+pnpm build；F-56/F-57 现场回放（基线含目标文件时合法引用过/main 基线时 kimi author 看不到/软限制漏网被 C1 拦）。
