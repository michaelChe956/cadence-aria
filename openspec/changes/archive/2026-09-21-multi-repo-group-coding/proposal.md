# Proposal: multi-repo-group-coding

## Why

阶段 4 立项三方决议（`.superpowers/sdd/2026-09-04_计划文档_3.6全量收敛轮_v1.0/stage4-tripartite-decision.md`，Q2/Q5，五项零分歧共识）将多仓 coding 切为本 change（change ②，落地序 ①→③→② 的末环——长杆独立，可与 ①③ 全程并行设计先行，实施排 change ① close-provider-validation 与 ③ retire-legacy-workitem-protocol 之后）。决议 Q2 选型 **(b) 每 target 一 attempt + (c) 两期交付**，本 change 只做一期。四项事实依据：

1. **mixed-target group 被一律拒绝的功能洞（Q2 裁决对象）**：现行 `per-repo-coding-execution` REQ-COD-04 明文「多成员 Issue 下创建 mixed-target WorkItemGroup 被一律拒绝并返回稳定错误码（在创建、恢复、replay 三处一致）；不做自动按仓拆分」。HEAD 实读拒绝点收敛：创建面 `src/web/handlers/coding/group.rs:324-346`（`group_target_snapshot`）与 `:424-446`（`resolve_group_repository`）、恢复/replay 面 `src/product/coding_attempt_repository.rs:264-284`（`logical_repository_for_group_attempt`），另有两处路由消费面同语义（`workspace_repository.rs:266-270`、`coding_evaluation_context/builder.rs:329-333`），稳定错误码 `repository_routing_ambiguous`。多目标仓 Issue 的 group coding 因此整面不可用。
2. **单 attempt 多 target 被否（Q2 选型 b 的决定性约束）**：`CodingExecutionAttempt` 是单 worktree 模型——`worktree_path`/`head_commit`/`stage`/`target_snapshot` 均为 attempt 级单值（`src/product/coding_models/execution.rs:115-159`），`start_attempt` 的 group 短路（`coding_workspace_engine/lifecycle.rs:227-258`）、半启动恢复（`:289-323`）、runner 物化管道全部假设一个 worktree。ter 实施侧定价：单 attempt 多 target 需持久化 schema 破坏性改造（60-100h+、动摇全仓最深不变式，否）；每 target 一 attempt 保全 REQ-COD-02 单 target 快照不变式、100% 复用单仓 group 机制（36-62h，中风险，推荐）。
3. **底座已全，缺口聚焦执行模型（决议 scope 接正）**：REQ-COD-05/06 非纯 spec——跨仓只读证据（`logical_codebase/{evidence_injection,evidence_mediator,evidence_index/}`）与每仓交付+聚合判定（`coding_attempt_store/{issue_delivery.rs,git_operation.rs}`，`IssueDeliveryOverall{AllPushed,Partial,None}`）均已实现；worktree 三元键 `(project, issue, repository)`（REQ-COD-03）与 `RuntimeBindingStore` 按 `repo_id` 分记录（`src/product/runtime_binding_store.rs`）在场。真正的工作量在执行模型（拆分创建/推进/聚合视图），不在底座。
4. **两期交付已决（Q2(c)）**：一期=解除 REQ-COD-04 拒绝+按 target 拆分独立 attempt+手动推进+group 级聚合只读视图/终态，**聚合视图必须同 WP 交付**（决议原文；oracle 风险承接「attempt 增殖 UI 面必须同 WP 交付」）；二期=跨 attempt 依赖就绪门自动编排，凭一期证据再立项，本 change 不预做。

## What Changes

（一期四项，决议钉死，不加不减）

1. **解除 REQ-COD-04 拒绝（按 target 分流）**：mixed-target WorkItemGroup 创建不再一律拒绝——三处校验点（创建/恢复/replay）与两处路由消费面一致改为按 target 分流：为每个目标仓库建立独立 target-attempt（每 `(plan, target)` 至多一个）；「group 内多 target」不再构成拒绝理由；无唯一 target 归属（units 无 `target_repository_id` 且 selection focus 不唯一）保持 fail-closed 拒绝（TargetMissing 语义）；`TargetAmbiguous` 稳定错误码在 group attempt 路由面退役（selection focus 面等其他消费点保留）。
2. **按 target 拆分独立 attempt**：advance/建组编排按 target 分流——多 target 场景为每 target 各建一个 target-attempt，各持**各自 target 的冻结快照**（REQ-COD-02 单快照不变式 per-attempt 适用，attempt 单值 schema 零改动）；attempt 唯一性从 per-plan 细化为 per-`(plan, target)`（单 target 场景行为零变化，spec 按显式路由写——oracle 风险承接）；issue 级单 active attempt 不变式相应细化为 per-`(issue, target)`（同 target 串行承接 REQ-COD-03 同仓串行，异 target 并行解禁）；advance durable record 以 additive 方式承载 target-attempt 集绑定（每 plan 仍恰一条，幂等不变）。
3. **手动推进（一期无自动编排）**：每个 target-attempt 的启动保持显式 `StartCoding` 唯一入口与 `SC_CODING_REQUIRES_ADVANCE` fail-closed 守卫（per-attempt 适用，语义零变化）；推进顺序由人决定（可串行可并行 StartCoding，受 per-target worktree lock 约束）；系统 MUST NOT 自动顺序拉起或依赖驱动拉起跨 target attempts（跨 attempt 依赖门=二期）。
4. **group 级聚合只读视图/终态（与拆分同批交付）**：每 plan 提供聚合只读投影——per-target attempt 状态/stage/分支/提交/推送/评审+聚合终态（全部交付/部分/未启，复用 REQ-COD-06 完成级别与 `issue_delivery` 聚合语义）；只读派生不形成第二套状态机；partial failure 显式呈现不伪装全局成功；各 target-attempt 的 group workspace 仍是各自执行观察面（REQ-GCE-04 语义 per-attempt 适用），聚合视图不承载决策/操作面。attempt 增殖审计面（拆分溯源 durable 记录+恢复矩阵回归）一并交付，为二期依赖门立项供证据。

## 非目标（明确不做）

- **不做二期跨 attempt 依赖门自动编排**（Q2(c) 钉死：二期凭一期证据再立项）——不实现跨 target-attempt 的依赖就绪门、自动顺序拉起、编排策略；本 change 不预研不预埋编排逻辑（增殖审计只记事实不做调度）。
- **不做单 attempt 多 target**（Q2 选型 a 已否）——`CodingExecutionAttempt` 单值字段（worktree_path/head_commit/stage/target_snapshot）MUST NOT 数组化或破坏性改造；REQ-COD-02 单快照不变式零松动。
- 不动 coding 三角色执行面、provider 注册面、REQ-COD-01/03/05/06 既有语义（底座复用，仅做适配核查）；不动 REQ-GCE-01..05（unit 级依赖门/失败处理/amendment 链/per-WI 投影语义 per-attempt 原样适用）。
- 不动 `StartCoding` 唯一入口与 advance「到 Ready 即止」契约（change ③ REQ-ADV-05 显式化承接，本 change per-attempt 适用不改文本）。
- 不做自动 PR（REQ-COD-06 既有后置项）、不做 provider 验证/旧协议退役（change ①③ 范围）。
- 不做聚合视图的操作面（决策/启动/中止仍走各 target-attempt 的既有入口）。

## Capabilities

### New Capabilities

- `multi-target-group-coding`：mixed-target group 按 target 分流执行的一期契约——分流创建与 per-`(plan, target)` 唯一性、per-target 冻结快照与单 worktree 不变式、手动推进与启动门、group 级聚合只读视图/终态、attempt 增殖审计与恢复一致性；跨 attempt 依赖门显式 defer（二期凭证据再立项）。

### Modified Capabilities

- `per-repo-coding-execution`：REQ-COD-04 由「mixed-target group 一律拒绝」改为「按 target 分流」（每 target 一 attempt、无唯一归属仍 fail-closed、同 target 场景零变化）；Purpose 同步（「mixed-target group 一律拒绝」句失效）。
- `work-item-plan-advance`：REQ-ADV-01/02 的「唯一 WorkItemGroup coding attempt」语义升级为「单 target 场景唯一 attempt（零变化）／多 target 场景 per-`(plan, target)` target-attempt 集」（幂等、journal 恢复、不启动 provider 语义原样保留）。

## Impact

- **代码（后端）**：`src/web/handlers/coding/group.rs`（两处 TargetAmbiguous 分流化）；`src/product/coding_attempt_repository.rs`（`logical_repository_for_group_attempt` 恢复/replay 面按各自快照路由）；`src/product/coding_attempt_store/{group.rs,group_validation.rs,group_initialization.rs}`（拆分创建+per-`(plan,target)` 唯一性+per-target 单 active 细化）；`src/product/advance_store.rs`（AdvanceRecord additive 集绑定）；`src/product/coding_workspace_engine/lifecycle.rs`（start_attempt/半启动恢复 per-attempt 适配）；`src/product/{workspace_repository.rs,coding_evaluation_context/builder.rs}`（路由消费面解除）；聚合只读投影 API（新，复用 `issue_delivery` 语义）。
- **代码（前端）**：group workspace 的 per-target attempt 呈现与聚合只读视图（决议钉死同批交付）；partial failure 呈现。
- **durable 数据**：additive schema（advance record 绑定扩展、拆分审计记录），无破坏性迁移；既有单 target 记录读取行为零变化。
- **测试**：三处拒绝点既有负向测试改锚分流语义；新增多 target 拆分/聚合/恢复矩阵测试族；单 target 回归零变化锁死。
- **openspec**：主 specs 三 capability 同步（per-repo-coding-execution/work-item-plan-advance 修订+multi-target-group-coding 新增）。
