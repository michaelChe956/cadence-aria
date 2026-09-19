# Design: multi-repo-group-coding

## Context

**决议权威（不得漂移）**：`.superpowers/sdd/2026-09-04_计划文档_3.6全量收敛轮_v1.0/stage4-tripartite-decision.md`（Q2 选 (b) 每 target 一 attempt+(c) 两期交付，一期四项+聚合视图必须同 WP 交付；Q5 切 change ②，落地序 ①→③→②）；oracle 裁决 `stage4-oracle-verdict.md`（Q2：保全 REQ-COD-02 单 target 快照 load-bearing 不变式、100% 复用单仓 group 机制、一期最小化解洞；风险承接：「两形态并存→spec 按显式路由写」+「attempt 增殖 UI 面必须同 WP 交付」）；scout 详报未落盘（决议/输入包转述其要点，本 change 起草已逐项实读复核）；ter 可行性 `ter 可行性=yield 摘要（决议转述，未落盘）`（面 A 工时带 36-62h；单 attempt 多 target 定价 60-100h+ 否决；WP 分解 A-WP1..A-WP5）。本 change 是决议 change ② 一期的契约化落地，不重新论证、不引入未拍板项。

**现状技术事实基线**（本 change 起草实读，行号为 worktree `feat-b-0808-add-monorepo` HEAD 实读，2026-09-19）：

| # | 事实 | 证据 |
|---|---|---|
| 1 | REQ-COD-04 现文=「mixed-target group 一律拒绝+稳定错误码（创建/恢复/replay 三处一致）；同 target 的 group 允许；不做自动按仓拆分」；spec Purpose 句含「mixed-target group 一律拒绝」 | `openspec/specs/per-repo-coding-execution/spec.md:4,34-39` 实读 |
| 2 | 拒绝点三处+消费面两处：创建面 `group_target_snapshot`（target_ids.len()>1 → TargetAmbiguous）与 `resolve_group_repository`（同逻辑）；恢复/replay 面 `logical_repository_for_group_attempt`（:264-284：units 收敛 target_ids，>1 → TargetAmbiguous；=0 时要求 selection focus 唯一否则 TargetMissing）；workspace 路由面（:266-270）；评估上下文面（:329-333，selection 多 focus 拒绝） | `src/web/handlers/coding/group.rs:324-346,424-446`、`src/product/coding_attempt_repository.rs:235-285`、`src/product/workspace_repository.rs:266-270`、`src/product/coding_evaluation_context/builder.rs:329-333` 实读 |
| 3 | 稳定错误码框架在场：`RepositoryRoutingErrorCode::TargetAmbiguous → "repository_routing_ambiguous"`（另有 TargetUnknown/TargetMissing/Inconsistent/MemberRemoved/SelectionInvalidated 等），WS 侧格式化 `{stable_code}: {reason}` | `src/product/logical_codebase/repository_routing.rs:18-41`、`src/web/workspace_ws_handler/socket.rs:258-264` 实读 |
| 4 | **单 worktree 模型（Q2 选型决定性约束）**：`CodingExecutionAttempt` 的 `worktree_path`/`head_commit`/`stage`/`target_snapshot` 均 attempt 级单值；`AttemptTargetSnapshot` 含三层身份映射（logical_repository_id/checkout_id/physical_repository_id+canonical_path+git_dir_identity+revision+policy_digest+membership_revision） | `src/product/coding_models/execution.rs:100-159` 实读 |
| 5 | start_attempt 守卫与 group 短路：SC advance attempt 必须 advance Ready 才可 StartCoding（「SC advance attempt must be Ready before StartCoding」）；group attempt worktree 已物化 → 短路直进 Coding+补 head；未物化回落 WorktreePrepare；半启动恢复 `prepare_resumed_attempt_for_runner` 同语义（Coding+未物化→demote 重物化；group 半启动缺 head 补 git head） | `src/product/coding_workspace_engine/lifecycle.rs:191-323` 实读 |
| 6 | advance 契约：REQ-ADV-01「创建或恢复**唯一的** WorkItemGroup coding attempt…发出 advance_completed（含 attempt_id）」+REQ-ADV-02「同 plan SHALL NOT 创建第二个 attempt」；`AdvanceRecord{plan_id, attempt_id: Option<String> 单值, status}` 每 plan 恰一条（put_record 对同 plan_id 冲突拒绝）；`advance_is_ready_for_attempt`=record.status==Ready && record.attempt_id==Some(attempt_id)（plan+attempt 精确绑定，缺记录/不匹配 fail-closed） | `openspec/specs/work-item-plan-advance/spec.md:9-45`、`src/product/advance_store.rs:23-36,200-283` 实读 |
| 7 | group attempt 创建：`create_group_attempt` 前置「issue 级单 active attempt」不变式（`active_coding_attempt_exists`——`status.is_active()` 覆盖 Created..AmendmentApplyFailed，任一在世即拒绝新 attempt）；`get_attempt_for_work_item_group` per-plan 取最早 attempt（attempt 唯一性 per-plan） | `src/product/coding_attempt_store/group.rs:70-92,140-228`、`src/product/coding_models/execution.rs:51-64` 实读 |
| 8 | **底座已全（决议 scope 接正）**：跨仓只读证据 `logical_codebase/{evidence_injection,evidence_mediator,evidence_index/,evidence_audit,evidence_budget,evidence_token}.rs`；每仓交付聚合 `coding_attempt_store/issue_delivery.rs`（`IssueDeliveryOverall{AllPushed,Partial,None}`+`DeliveryEntry{repository_name,attempt_status,branch_name,commit_sha,push_status,push_error}`+`compute_issue_delivery_summary`：每 WI 取最新 attempt，Completed+Pushed 才算交付）+`git_operation.rs`；worktree 三元键+锁迁移协议（REQ-COD-03）；`RuntimeBindingStore` 按 `repo_id` 分记录（`CreateRuntimeBindingInput.repo_id`） | 目录+文件实读 |
| 9 | unit 级基座 per-attempt 适用：REQ-GCE-01 依赖门（admission_kind=sc_advance 判据、共享 worktree 单 active unit）、REQ-GCE-02..05（失败同 attempt/amendment 链/per-WI 只读投影/出版恢复）语义均在「一个 attempt」内闭环——拆分后各 target-attempt 内原样适用 | `openspec/specs/work-item-group-coding-execution/spec.md` 实读 |
| 10 | ter 面 A 工时带：A-WP1 解禁 4-8h / A-WP2 拆分创建+快照绑定 12-20h / A-WP3 聚合判定+partial failure+UI 8-14h / A-WP4 证据注入适配 4-8h / A-WP5 恢复矩阵 8-12h（合计 36-62h）；实施风险：高风险=advance 门语义细化（Ready 语义按 attempt 还是 plan 聚合，直接影响 SC_CODING_REQUIRES_ADVANCE fail-closed 不变式，有测试钉住）；中风险=UI 聚合视图；低风险=解禁本身 | `ter 可行性=yield 摘要（决议转述，未落盘）` §1 面 A |

**本文只做规划产物**：精确文件、命令、测试与提交步骤由 `superpowers:writing-plans` 写入 `cadence/plans/`。

## Goals / Non-Goals

**Goals:**

- mixed-target group 按 target 分流解禁：三处校验点+两处路由消费面一致改分流，无唯一归属仍 fail-closed（REQ-MTG-01；REQ-COD-04 MODIFIED）。
- per-`(plan, target)` 独立 target-attempt：各持冻结快照、单值 schema 零改动、单 target 场景零变化（REQ-MTG-01/02；REQ-ADV-01/02 MODIFIED）。
- 一期手动推进：StartCoding 唯一入口+守卫 per-attempt 适用，无自动跨 attempt 编排（REQ-MTG-03）。
- group 级聚合只读视图/终态同批交付：派生只读、partial failure 显式（REQ-MTG-04）。
- attempt 增殖审计面+恢复矩阵回归，为二期依赖门供证据（REQ-MTG-05）。

**Non-Goals（与 proposal 非目标互补）:**

- 不重开 Q2/Q5 已决事项（选型 b、两期边界、三 change 切分与 ①→③→② 序）。
- 不动 `CodingExecutionAttempt` 持久化 schema 的单值字段（D1 红线）；advance record 仅 additive 扩展（D2）。
- 不实现跨 attempt 依赖门/自动编排/编排策略（二期，凭一期证据再立项；本 change 增殖审计只记事实）。
- 不动 unit 级基座（REQ-GCE-01..05 语义 per-attempt 适用不改文本）、不动 REQ-COD-01/03/05/06 语义（仅适配核查）、不动 change ①③ 范围。
- 聚合视图不做操作面（决策/启动/中止走各 attempt 既有入口）。

## Decisions

### D1: 每仓独立 attempt 模型——REQ-COD-02 单快照不变式保全

**决策**：拆分的原子单位是 **target-attempt**（scope 仍为 `WorkItemGroup`，绑定同一 `work_item_group_id`/plan，各持一个 target）。`CodingExecutionAttempt` 的单值字段（`worktree_path`/`head_commit`/`stage`/`target_snapshot`/`branch_name`/`base_branch`）**零改动**——每个 target-attempt 各冻结**各自 target** 的 `AttemptTargetSnapshot`（三层身份映射+policy_digest 原语义），REQ-COD-02「创建、恢复、重放一律使用冻结快照」per-attempt 原样适用。**路由权威随之转移**：`logical_repository_for_group_attempt` 现行的「从 units 收敛单一 target」逻辑在 target-attempt 上不再使用——每 attempt 的路由直接以其冻结快照的 `logical_repository_id` 为权威（这正是解禁的实现形态：多 target 不再需要收敛为一个）。恢复/重放/半启动恢复（`prepare_resumed_attempt_for_runner`）按各自快照与 worktree 独立运转，语义与单 target 等价。

**不变式清单（拆分后逐条保全）**：①attempt 单 worktree（每 attempt 恰一个 worktree_path/head_commit）；②快照不可变+恢复不猜测（REQ-COD-02 原文）；③worktree 三元键+同仓串行（REQ-COD-03——worktree lock 已按 `(project, issue, repository)` 天然支持异 target 并行）；④unit 级依赖门/单 active unit（REQ-GCE-01，attempt 内）；⑤SC admission 唯一入口+守卫（per-attempt，见 D2）；⑥每 plan 恰一条 advance record（幂等，见 D2）。

**替代方案与否决理由**：单 attempt 多 target（Q2 选项 a）——ter 定价 60-100h+：持久化 schema 破坏性改造（单值→per-target 集合）、start_attempt group 短路/半启动恢复/runner 管道全重写、WS 事件 per-target 维度=前端协议大改、旧 record 迁移；动摇 REQ-COD-02 load-bearing 不变式（oracle 裁决否决理由）。

### D2: group 拆分编排——按 target 分流+per-(plan,target) 唯一性+advance 绑定 additive 扩展

**决策**（ter 高风险点=advance 门语义细化的闭合方案）：

- **分流规则**：建组/advance 初始化时解析 authoritative plan binding 的 units，按 `target_repository_id` 分组——单 target（含 0 target 且 focus 唯一）走**既有路径零变化**（单 attempt、收敛路由原样）；多 target 为每 target 各建一个 target-attempt（units 按 target 分入各自 attempt 的 coding units，无 target 归属的 unit fail-closed——沿用 source_draft_error/Inconsistent 语义）。**spec 按显式路由写**（oracle 风险承接：两形态并存必须显式，不写「透明拆分」）。
- **唯一性细化**：attempt 唯一性从 per-plan 细化为 per-`(plan, target)`（`get_attempt_for_work_item_group` 语义升级为按 (plan, target) 检索）；issue 级单 active attempt 不变式（事实基线 #7）细化为 **per-`(issue, target)` 单 active**——同 target 串行（REQ-COD-03 同仓串行承接），异 target 并行解禁（多 target-attempt 可同时 Created/Ready/Running，各自 worktree lock 隔离）。
- **advance 绑定扩展（additive）**：每 plan 仍恰一条 `AdvanceRecord`（幂等/冲突语义不变）；`attempt_id` 单值字段对单 target 场景保持原承载（向后兼容，既有记录零迁移）；additive 新增 target-attempt 集绑定（如按 target 的 attempt 引用映射），多 target 场景由集合承载。`advance_is_ready_for_attempt` 判定扩展为「`attempt_id` 精确匹配 **或** 属于集合绑定」——**fail-closed 不变式语义零变化**（未匹配/缺记录/未 Ready 一律拒绝），`SC_CODING_REQUIRES_ADVANCE` 守卫 per-attempt 适用，Ready 判定按 attempt 粒度闭合（不引入 plan 级聚合 Ready——那会弱化守卫）。`advance_completed` 载荷对多 target 含 target-attempt 引用集（单 target 仍单 `attempt_id`）。
- **解禁面清单**：三处校验点+两处消费面（事实基线 #2）逐处改分流；`TargetAmbiguous` 在 group attempt 路由面退役（多 target=正常分流场景），selection focus 面等其他消费点（评估上下文多 focus 拒绝）保留原语义；无唯一 target 归属保留 TargetMissing fail-closed。

**替代方案与否决理由**：①Ready 语义按 plan 聚合（一条 record Ready 即全 attempts 可启动）——弱化 per-attempt fail-closed 精确绑定，守卫测试连锁，且语义上允许「target B 未就绪即可启动」；②每 target 各一条 advance record——违反每 plan 一条幂等约束（put_record 冲突），REQ-ADV-02 重写面更大；③attempt_id 改集合（非 additive）——既有记录迁移，破坏性。

### D2.1 存量无快照 attempt 退化处置

存量 legacy group attempt 的 target_snapshot 为 Option——无快照 attempt 求不出 target 键、不落任何 (plan,target) 桶；「同 plan 存量无快照 attempt+新建 target-attempt」场景下按 display-only/fail-closed 处置（不参与唯一性判定、不静默归属最早桶）——WP2 writing-plans 检索展开时落为可测判据。

## D3: group 级聚合只读视图/终态——同批交付+派生不造第二状态机

**决策**：聚合视图是**新增只读派生投影**（对齐 REQ-GCE-04 风格：从 durable 事实确定性派生，SHALL NOT 形成第二套状态机）。**数据面**：每 plan 的 per-target 投影（target 身份+该 target-attempt 的状态/stage/branch/head_commit/push_status/review_request+失败/阻塞原因）+聚合终态三值——全部交付（全部 target-attempt 达 REQ-COD-06 完成级别：Completed+已推送+ReviewRequest 在案）/部分（任一 target 未满足，显式呈现 partial failure，不伪装全局成功——REQ-COD-06 原语义的 plan 级投影）/未启（无 target-attempt 或全部未启动）。**实现复用**：判定语义直接对齐 `issue_delivery.rs` 的 `IssueDeliveryOverall{AllPushed,Partial,None}`（per-WI 最新 attempt+push_status 口径），聚合粒度从 issue 级加一层 plan/target 级（`DeliveryEntry.repository_name` 字段已在——底座复用的具体形态）。**观察面边界**：各 target-attempt 的 group workspace 仍是各自执行观察面（REQ-GCE-04 per-attempt 适用）；聚合视图只读、不承载决策/启动/中止操作面。**同批交付**（决议钉死：聚合视图必须同 WP 交付——拆分 WP 与聚合 WP 同批验收关闸，不允许「先拆分后补视图」的中间交付态）；前端呈现（per-attempt 视图+partial failure 呈现）同批（oracle：attempt 增殖 UI 面必须同 WP 交付）。

**替代方案与否决理由**：①聚合视图后置二期——决议明文否决（同 WP 交付）；②聚合视图做成可操作面（聚合启动/中止）——引入跨 attempt 编排能力，越权二期（依赖门/自动编排）；③复用 issue 级 `compute_issue_delivery_summary` 不加 plan/target 层——多 plan Issue 下无法按 plan 聚合，增殖审计面（D4）无承载。

### D4: attempt 增殖审计面——拆分溯源 durable 记录+恢复矩阵

**决策**：拆分是 attempt 增殖动作（一 plan 一 attempt → 一 plan N attempts），SHALL 落 **durable 审计记录**：每次分流创建记录 plan、target、target-attempt 身份、分流依据（authoritative binding 解析快照/revision）、创建时间与触发入口（advance 命令键）——恢复/重放可追溯每 attempt 的分流来源，`get_attempt_for_work_item_group` 的「取最早」歧义在多 attempt 下由审计消解。**恢复矩阵回归**（ter A-WP5）：创建/恢复/replay/半启动/断连 × 单 target（回归零变化锁死）/多 target（等价语义验证）全组合覆盖，作为关闸验收口径。**二期证据义务**：审计面是一期证据的落点——跨 attempt 依赖门立项（二期）凭此证据（决议 Q2：二期凭一期证据再立项）；本 change 只记事实不做调度。

**替代方案与否决理由**：以既有 attempt 记录隐式溯源（不立审计面）——「取最早」检索在 N attempts 下语义退化（无法区分分流批次与重建意图），且二期立项无证据锚。

### D5: 二期承接——跨 attempt 依赖门显式 defer 登记

**决策**：跨 target-attempt 依赖就绪门自动编排（REQ-GCE-01 的 attempt 间扩展）在本 change 显式 defer：登记触发条件=一期交付后真实多仓使用证据（增殖审计面+聚合终态/partial failure 分布），凭证据另行立项（决议 Q2(c) 原文）。本 change 的 specs 在 `multi-target-group-coding` REQ-MTG-03 写死一期红线（MUST NOT 自动拉起跨 target attempts），defer 登记与 change ③ DEF-4 的红线条件 defer 同款纪律（触发条件+另行立项+不预埋）。

**替代方案与否决理由**：一期顺带实现简化版依赖门（如按 plan 内 units 依赖跨 attempt 映射）——决议两期边界钉死（一期最小化解洞、二期凭证据），越权且 ter 未定价。

### D6: 附录 B 决议承接对照表

见本文附录 B（Q2/Q5(②) 逐句 → WP/REQ 映射，含 oracle 风险与 ter 工时带承接）。与 C1/C3 惯例一致，作为双审（k3+oracle）对照决议的锚。

## Risks / Trade-offs

| # | 风险 / 取舍 | 缓解 |
|---|---|---|
| R1 | advance 门语义细化连锁（ter 高风险：SC_CODING_REQUIRES_ADVANCE 有测试钉住，attempt_id 绑定扩展动判定路径） | D2 additive 集合绑定+「精确匹配或集合包含」判定：fail-closed 语义零变化；单 target 场景逐字节等价回归锁死；守卫测试族先改锚后扩 |
| R2 | issue 级单 active 不变式细化连锁（`create_group_attempt` 前置+消费面假设「一 issue 一在世 attempt」） | D2 细化为 per-`(issue,target)`：检索/前置逐消费面清单化改造（writing-plans 展开）；同 target 串行不变式保留；恢复矩阵（D4）覆盖并发创建/恢复 |
| R3 | 两形态并存漂移（单 target 既有路径 vs 多 target 分流路径） | oracle 风险承接：spec 按显式路由写（REQ-MTG-01 显式两分支+同 target 零变化 scenario）；单 target 回归零变化作为每 WP 验收口径 |
| R4 | 拆分后半启动/断连恢复矩阵组合爆炸（N target × 各阶段） | D4 恢复矩阵按「每 attempt 独立等价单 target 语义」验证（不做跨 attempt 耦合恢复——一期无跨 attempt 状态）；矩阵组合以阶段×target 数抽样+全量关键路径 |
| R5 | 聚合终态误判（partial failure 伪装成功/终态过早） | D3 判定口径对齐 issue_delivery 既有语义（Completed+Pushed 才算）；负向测试（未推送/无 attempt/失败态）钉死；REQ-COD-06「不伪装全局成功」原文承接 |
| R6 | UI 聚合视图新面工作量大（ter 中风险） | D3 数据面先行+派生投影（无状态机）；呈现按 per-target 卡片/终态汇总最小面，writing-plans 定细节；决议钉死同批交付不可裁 |
| R7 | 增殖审计遗漏重建/恢复场景（attempt 再生不可追溯） | D4 审计记录覆盖创建/恢复/重放入口；`get_attempt_for_work_item_group` 升级为 per-(plan,target) 检索消解「取最早」歧义 |
| R8 | advance record additive 字段与 change ③ REQ-ADV-05/06 的 spec 联动（两 change 同 capability） | 落地序 ①→③→②：本 change 实施时 ③ 已归档（REQ-ADV-05/06 在主 specs）；本 change delta 只 MODIFIED REQ-ADV-01/02 与其不重叠；见「change ③ 归档时序说明」 |
| R9 | ter 工时带下限乐观（36h） | WP 排序解禁→拆分→聚合→适配→矩阵（ter 依赖序 A-WP1→A-WP2→{A-WP3,A-WP4}→A-WP5）；长杆=WP2，超带时以工时带承接记录呈报不静默缩范围 |

## Migration Plan

1. **交付序**：WP1 解禁 → WP2 拆分创建+advance 绑定 → {WP3 聚合视图, WP4 底座适配核查} → WP5 恢复矩阵+审计收口 → 关闸（ter 依赖序承接；WP3/WP4 可并行）。
2. **durable 数据**：additive only——advance record 新增绑定字段（serde default）、增殖审计新记录类型；既有单 target attempt/advance 记录零迁移、读取行为零变化。
3. **部署/回滚**：普通提交回滚；无 wire 破坏性变更（既有 WS/API 语义保留，新增只读聚合投影 additive）；回滚后多 target 记录只读留存（按 REQ-COD-02 旧 attempt 处置先例：display-only/人工恢复，不猜测重路由）。
4. **二期触发**：一期交付+真实使用证据（审计面+终态分布）→ 二期依赖门另行立项（D5）。

## Open Questions

均为实施细化留白（不改架构、specs 或工作包划分）：

1. advance record 集合绑定的具体字段形态（按 target 的映射 vs 引用列表）——D2 锁契约（additive+精确匹配或包含判定），形态属 writing-plans。
2. target-attempt 的 branch_name/base_branch 生成规则（per-target 命名防冲突）——既有 attempt 级字段，命名细则属 writing-plans。
3. 聚合投影 API 形态（既有 group attempt API 扩展 vs 新只读端点）与前端呈现形态——D3 锁「只读派生+同批交付+不承载操作面」。
4. per-`(issue,target)` 单 active 细化的消费面完整清单——D2 锁语义，清单由 writing-plans 全仓检索展开。
5. 无 target 归属 unit 的处置细节（Inconsistent 错误文案/是否允许显式排除）——沿用既有 fail-closed 语义，文案属实施。

## change ③ 归档时序说明

本 change（②）与 change ③（retire-legacy-workitem-protocol）均 delta 了 `work-item-plan-advance`：③ REMOVED REQ-ADV-03+ADDED REQ-ADV-05/06，本 change MODIFIED REQ-ADV-01/02——不重叠。落地序 ①→③→②：本 change 实施时 ③ 已归档（REQ-ADV-05/06 已入主 specs），本 change 的 per-attempt 守卫适配（D2）以 REQ-ADV-05 的「StartCoding 唯一入口+守卫」契约为既成基座，不重复定义；归档 sync 顺序若颠倒（② 先归档），③ 的 delta 不受影响（其 REQ-ADV-05/06 全文自足）。

## 附录 B 决议承接对照表

（决议=`stage4-tripartite-decision.md`；oracle=`stage4-oracle-verdict.md`；scout 详报未落盘（决议/输入包转述要点，本文事实基线已实读复核）；ter=`ter 可行性=yield 摘要（决议转述，未落盘）`（面 A）。逐句承接，双审对照锚。）

| 决议原文 | 承接位置 | 备注 |
|---|---|---|
| Q2：多仓 group 模型 → (b) 每 target 一 attempt | D1/D2 全部；REQ-MTG-01/02；WP1/WP2 | 单快照不变式保全=D1 不变式清单 |
| Q2：(c) 两期交付——一期=解除 REQ-COD-04 拒绝 | proposal What Changes 1；REQ-MTG-01（+REQ-COD-04 MODIFIED）；WP1 | 解禁面清单=事实基线 #2（三处+两消费面） |
| Q2：一期=按 target 拆分独立 attempt | proposal What Changes 2；REQ-MTG-01/02；REQ-ADV-01/02 MODIFIED；WP2 | 唯一性细化 per-(plan,target)+单 active 细化=D2 |
| Q2：一期=手动推进 | proposal What Changes 3；REQ-MTG-03；WP2 守卫适配 | StartCoding 唯一入口+守卫 per-attempt 零变化=D2 |
| Q2：一期=group 级聚合只读视图/终态 | proposal What Changes 4；REQ-MTG-04；WP3 | 派生只读+partial failure 显式=D3 |
| Q2：（聚合视图必须同 WP 交付） | D3 同批交付句；tasks WP3 与 WP2 同批评收口径 | 决议括号原文 |
| Q2：REQ-COD-02 单 target 快照不变式保全（ter 估单 attempt 多 target 需 60-100h+ 动持久化不变式，否） | D1 红线（单值字段零改动）；Non-Goals；R 替代方案否决记录 | ter §2 Q2 表 a 行定价承接 |
| Q2：二期=跨 attempt 依赖就绪门自动编排，凭一期证据再立项 | D5；REQ-MTG-03 红线句+defer 登记；Non-Goals 首条 | 二期证据义务=REQ-MTG-05 审计面 |
| oracle Q2：保全 REQ-COD-02 load-bearing 不变式；100% 复用单仓 group 机制；一期最小化解洞 | D1（不变式清单+路由权威转移）；D3（issue_delivery 复用） | oracle Q2 理由三句逐条承接 |
| oracle Q2 风险：两形态并存→spec 按显式路由写 | D2 显式路由句；REQ-MTG-01 单 target 零变化 scenario；R3 | oracle Q2 风险承接 |
| oracle Q2 风险：attempt 增殖 UI 面必须同 WP 交付 | D3 前端同批句；R6；WP3 | 与决议「聚合视图同 WP 交付」同源双锚 |
| Q5 change ②：多仓 coding（Q2 一期多 WP；底座复用：REQ-COD-05/06 已有实现——ter 实读接正，缺口聚焦执行模型） | 事实基线 #8；D3 复用形态；WP4 底座适配核查（非重建） | 决议 scope 接正原文承接 |
| ter 面 A 工时带 36-62h（A-WP1..A-WP5）与依赖序 A-WP1→A-WP2→{A-WP3,A-WP4}→A-WP5 | tasks WP1..WP5 划分与排序总则；R9 | A-WP2 的「attempt 间依赖门」子项按决议移二期，工时带对应核减留痕 |
| 决议顺序：①→③→② 落地（②设计/proposal 与①③全并行先行） | 本四件套即并行先行交付物；change ③ 归档时序说明；R8 | 落地序约束=实施排程非设计门 |
| 决议风险登记：Q2 二期工时待一期证据 | D5 触发条件；R9 | 二期不预定价 |
| 决议下一步：四件套→validate strict→双审（k3+oracle）→计划→实施 | 本四件套即交付物；tasks 关闸项 validate strict | 双审输入=本 change+附录 B |
