# Design: per-issue-base-branch

> 依据：F-57/F-56 实证 + 用户方案与默认值规则裁决 + oracle/max-task 双裁决（pib-oracle.md / pib-maxtask.md，无实质分歧，统一定案回填）。

## Context

issue_0002 的 design author 文件浏览泄漏到 `.worktrees/aria-issues/issue_0001/`（source id `repository_0001/status.html` 实证）；coding fork base 由各上游入口以当前检出/HEAD 隐式确定（web/handlers/coding.rs:157-158、group.rs:105-106——非 lifecycle 硬编码）；C1 核对树依赖共享 coding worktree、不可用时 fail-safe 跳过。

## Decisions

### D1 存储：issue 记录增 base_branch（Option<String>）

None=按默认链解析（main→master→无：读取 OK、生成/coding fail-closed 诊断——兼容 main/master 仓、皆无仓明确失败优于现状）。REST 创建/读取透传；UI 创建表单分支选择器（预选默认值）。创建后锁定：无修改通道。

### D2 分支列表与默认值

列表=全部本地 refs/heads（不隐式 fetch；远端跟踪引用随 fetch 漂移、破坏锁定反漂移意图，且系统无 remote 模型——需远端基线=用户先建本地跟踪分支）。默认链 main→master→无默认强制手选。max-task 技术事实登记：已 fetch 的 remote-tracking ref 可直接 fork（git 实验证实）——技术可行不改变产品口径（锁定语义优先）。

### D3 author 上下文限定：通道层路由（分级）

- **host-served 通道**（kimi fs 桥等）：路由 `git show <base>:<path>`/`git ls-tree`，可配树清单缓存（ls-tree 一次+逐文件 show）
- **provider-native 通道**（自带文件能力的 provider）：base 专用检出作会话根（**检出位于仓库工作区外**——现 author 根=repository.path 结构性含 .worktrees/，必须改）+根外不可读
- **收不住的通道**：fail-closed 拒启（不降级 prompt/白名单/事后日志）
- 工具策略层仅既有写拒绝+spawn 守卫；prompt 仅教学
- 已知漏洞（task 2.1 审计必查）：kimi Orchestrator Terminal Auto=Allow + bwrap 宿主根只读可见；terminal/MCP/原生读/resume 四形态全覆盖

### D4 coding fork 与核对同源

各上游入口的隐式基线（当前检出/HEAD）统一改读 issue.base_branch；C1 plan_baseline_tree 改为按分支名直接取树（不依赖共享 coding worktree）+不可解析 fail-closed（废弃 fail-safe 跳过——drift③采纳）。

## Risks / Trade-offs

- author 文件通道因 provider 而异（terminal/MCP/resume）——task 2.1 全通道审计，漏一个=限定失效；审计报告含旁路检测方法（路径访问审计）
- 皆无 main/master 的存量 Issue 从「静默走当前检出」变「明确失败」——行为收紧（优于现状的隐式漂移），发布说明登记
- 专用检出根位置（工作区外）的生命周期管理（创建/清理）随实施定

## Open Questions（实施首步）

- author 文件访问通道全清单（terminal/MCP/原生/resume 四形态落点）——task 2.1
