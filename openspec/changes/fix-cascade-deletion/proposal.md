# Proposal

## Why

现有实体删除不是完整的拥有闭包：Plan 两路均漏清 durable source/IR/report/provenance，schema-v2 可能留下 issue-wide Work Item，会话关联 review/run 档案亦会成为孤儿；Story/Design 缺少下游引用保护。owner 于 2026-10-13 已明确选定“物理删干净，只留一条无内容删除事实”，本 change 将该裁定固化为可验收契约，不以清历史代替报告身份修复。

## What Changes

- **BREAKING**：永久删除 Plan（legacy/schema-v2）物理清除全部所属会话、source/IR/机械与结构报告、provenance、compile/draft/outline、revision/publication、真实归属 Work Item、review/provider-run、advance record/初始化journal/retry及其专属checkpoint；Confirmed/曾发布内容不保留。已删除身份终止advance幂等重放与Ready放行。
- Story/Design 同样清各自拥有的主记录、版本、会话与关联 review/run；存在存活下游引用时拒删，提示先删后置对象，不自动跨级删除下游。
- 每个已删除实体只保留一条不含业务内容的删除记录；同请求重试收敛到同一条记录，不保审计内容、报告摘录或恢复副本。
- 分级确认：可证从未确认的 Draft 无确认提示；当前或曾经 Confirmed 弹一次明确告知永久清内容的确认框；未知历史的旧 Draft 按 Main 追加兼容裁定保守弹一次，新路径单调保留 ever_confirmed 且不改确认 CAS；coding attempt 存在时先拒删并提示先删 coding workspace，不自动中止 coding。
- 删除与生成/修订/compile 的启动及落盘互斥；所有门禁在破坏性动作前检查，子树先删、根最后删，部分 IO 失败可重入且不得报告成功后留残余。
- R（FixReportId）是先行独立修复/写面移交门，不在本 change 重写报告身份算法；D1 为拥有者 purge/删除记录，D2 为协调、运行隔离、引用门禁及分级确认。

## Capabilities

### New Capabilities

- `lifecycle-entity-deletion`：Story/Design 自有闭包清理，以及三类实体共用的无内容删除记录、引用/运行保护、分级确认和失败收敛。

### Modified Capabilities

- `work-item-group-deletion`：扩展现 REQ-GRP-01 到 Plan 两路完整物理闭包，保留既有 coding 拒删、scope 隔离与上游数据保护。

## Impact

- 存储：LifecycleStore、WorkItemPlanSourceStore、WorkItemPlanStore、WorkItemRevisionStore、AdvanceStore，以及 issue-scoped review/run/session附属文件和稳定repo共享资源准入；沿现scope校验和NotFound purge，不新增依赖。
- 服务端：现有 Story/Design/Plan DELETE 路径、WorkspaceSessionRegistry/manager 与在途 writer 准入；错误 details 给出阻挡对象，前端不得作为归属或删除资格权威。
- 前端：既有生命周期删除入口消费服务端确认事实，一次确认覆盖该实体完整闭包；不重做工作台布局，不更改确认/发布 CAS 或 trust 安全授权。
- 排程：与FixReportId、F1.2、SG3a.0/SG3a.1/SG3b/SG4a.0/SG4b/SG6/SG7a、trust/anchor共享写面串行移交；双向登记于SG Plan §0.2，SG1/SG2无写面交集纯核可继续。
- 权威依据：`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/cadence/designs-reviews/2026-10-13_设计评审_实体级联删除与报告身份修复_v1.0.md`。其中推荐保审计/Confirmed 拒删被上述 owner 最终裁定覆盖。
- 非目标：软删、内容恢复、删除 issue/仓库/LC 注册或 trust、自动删除 coding attempt、无引用证据的跨级级联、一般删会话/native fresh 的历史清零、放宽 immutable/hash/join 校验、IR 身份重设计。
