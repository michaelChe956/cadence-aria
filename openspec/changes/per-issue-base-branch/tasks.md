# Tasks: per-issue-base-branch

**全局边界**：创建后锁定；存量 None=默认链（皆无=读取 OK/生成 coding fail-closed）；列表仅本地 refs/heads 不隐式 fetch；prompt 不承担安全边界；不改 worktree 复用语义。

## 1. 模型与创建入口

- [ ] 1.1 issue 记录增 base_branch（Option）+REST 透传+创建校验（分支存在/皆无默认链时未选即拒）；红绿：选分支/默认链两态/皆无强制手选/不存在拒/存量兼容五用例（REQ-PIB-01 场景 1-3、5-6）。
- [ ] 1.2 UI 分支选择器（本地分支列表+默认值预选）+修改拒绝呈现（场景 4）。

## 2. author 上下文基线限定（F-57 根治）

- [ ] 2.1 **全通道审计**：author 文件访问四形态（host-served fs/terminal/MCP/原生读+resume）逐 provider 落点清单+已知漏洞核查（kimi Terminal Auto=Allow/bwrap 宿主可见/author 根含 .worktrees/）+旁路检测方法——审计报告为 2.2 落点依据。
- [ ] 2.2 通道层路由：host-served→git show/ls-tree（树缓存）；provider-native→base 专用检出根（仓库工作区外）+根外不可读；收不住拒启；红绿：F-57 现场形态复现（author 找不到 .worktrees）+所见=基线+收不住拒启三用例（REQ-PIB-02 场景 1-2、4）。
- [ ] 2.3 基线消失 fail-closed 诊断（存量皆无仓生成失败同路径）（场景 3+存量场景）。

## 3. coding fork 与核对同源

- [ ] 3.1 各上游入口隐式基线统一改读 issue.base_branch（coding.rs:157-158/group.rs:105-106 等全清单）+创建时校验；红绿：fork 自所选分支（REQ-PIB-03 场景 1）。
- [ ] 3.2 C1 plan_baseline_tree 改按分支名取树（不依赖共享 worktree）+不可解析 fail-closed（废弃跳过）；红绿：三面同源+核对不可解析不跳过（场景 2-3）。

## 4. 门禁收口

- [ ] 4.1 lib/it_core/it_web/vitest 全量+strict；F-56/F-57 现场回放（基线含目标文件时合法引用过/main 基线时 author 看不到/核对不跳过）。
