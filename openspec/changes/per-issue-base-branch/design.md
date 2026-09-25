# Design: per-issue-base-branch

> 依据：F-57（f51-53 域外新实证：design author 文件浏览到 .worktrees/aria-issues/issue_0001/ 写入「既有契约」）+ F-56 连锁 + 用户方案裁决。

## Context

issue_0002 的 design（session_0008 v2）source id 含 `repository_0001/status.html`——author 从兄弟 worktree 发现文件；coding fork base 隐式 main；C1 AC 核对用 fork base 树。三面不同源。

## Decisions

### D1 存储：issue 记录增 base_branch 字段（Option<String>，None=main 兼容）

durable issue.json 增字段（serde default None）；REST 创建/读取透传；UI 创建表单增分支选择器（GET 分支列表端点复用既有 git 服务）。

### D2 author 上下文限定：文件读取路由经 base 树

author 工具面（文件浏览/读取）改为经 `git show <base_branch>:<path>` / `git ls-tree <base_branch>` 形态（不 checkout、不触碰工作区路径）；provider 工具策略的路径白名单收窄到该形态。实施时核查 author 文件访问的实际通道（pi/kimi 的文件工具 bridge），在通道层路由而非 prompt 层劝导（prompt 不是安全边界）。

### D3 coding fork：create_branch base 取 issue.base_branch

lifecycle.rs 的 journal.base_branch 从 issue 记录取值（现硬编码/隐式 main 处改为查询 issue）；worktree 复用/幂等语义不动。

### D4 C1 核对统一：plan_baseline_tree 取 issue.base_branch

C1-T2 已实现的 git ls-tree 调用改参数源（fork base→issue.base_branch；两者在 D3 后一致，此为显式统一）。

### D5 锁定与 fail-closed

创建后修改拒绝（REST/UI 无更新通道）；运行时分支消失：author 生成终止报诊断（D2 的 git show 失败映射）；coding 创建时校验存在拒绝 attempt。

## Risks / Trade-offs

- author 文件工具通道因 provider 而异（pi ask/kimi read_file 等）——D2 需逐通道核查，漏一个=限定失效；实施首步做通道清单审计
- git show 形态对大仓库的性能（每次读文件 spawn git）——可缓存树对象清单（ls-tree 一次+逐文件 show），实施时定
- UI 分支列表端点权限（只列分支名无风险）

## Open Questions（实施首步）

- author 文件访问通道全清单（provider bridge 审计）——决定 D2 落点
