# Tasks: multi-repo-group-coding

**工作包性质**：本文件只登记**高层工作包**与验收口径（映射 specs 的 requirement 与 design.md 的 D 系列决策）。精确文件、命令、测试与提交步骤由 `superpowers:writing-plans` 展开到 `cadence/plans/`；Plan 只能展开工作包，不能重定义契约。

**主契约**：三方决议 `stage4-tripartite-decision.md`（Q2(b)(c) 一期四项/Q5 change ②，决议承接对照表 = design.md 附录 B）+ 本 change 四件套。

**全局边界（每工作包均继承）**：

- **范围铁律**：仅决议一期四项（解禁分流/拆分独立 attempt/手动推进/聚合只读视图含增殖审计），不加不减；跨 attempt 依赖门自动编排=二期凭一期证据另行立项，任何「简化版编排」变体均越权。
- **单值 schema 红线（D1）**：`CodingExecutionAttempt` 单值字段（worktree_path/head_commit/stage/target_snapshot）零改动；REQ-COD-02 单快照不变式零松动；REQ-COD-01/03/05/06 语义不动（WP4 仅适配核查）。
- **守卫零变化红线（D2）**：`StartCoding` 唯一入口与 `SC_CODING_REQUIRES_ADVANCE` fail-closed 语义 per-attempt 适用不改文本；advance record 每 plan 恰一条+additive 绑定扩展（存量零迁移）。
- **同批交付红线（决议+oracle）**：聚合只读视图（含前端呈现）与拆分能力同批验收，不存在「拆分已交付而聚合视图缺席」的中间态。
- **单 target 零变化**：每 WP 验收含单 target 回归等价（既有测试族零改语义通过或仅改锚不改断言语义）。
- **unit 级基座不动**：REQ-GCE-01..05 语义 per-attempt 适用（不因多 attempt 改 unit 级依赖门/失败处理/amendment 链）。

**排序总则**：WP1 解禁 → WP2 拆分创建+advance 绑定 → {WP3 聚合视图 ∥ WP4 底座适配核查} → WP5 恢复矩阵+审计收口 → 关闸（ter 依赖序 A-WP1→A-WP2→{A-WP3,A-WP4}→A-WP5 承接）。

## 1. WP1 mixed-target 解禁（三处校验+两消费面分流化）

- [ ] 1.1 解禁点改造：`group.rs` 两处（`group_target_snapshot`/`resolve_group_repository`）与 `coding_attempt_repository.rs`（`logical_repository_for_group_attempt`）的多 target 拒绝分支改为按 target 分流解析；`workspace_repository.rs`/`coding_evaluation_context/builder.rs` 消费面同步适配；`TargetAmbiguous` 在 group attempt 路由面退役（selection focus 面保留）；无唯一 target 归属保留 TargetMissing fail-closed（REQ-MTG-01、design D2 解禁面清单）——验证：mixed-target 建组不再被拒（分流解析成功）；无归属负向测试仍 fail-closed；既有 TargetAmbiguous 断言测试改锚分流语义（负向改正向+新增无归属负向）
- [ ] 1.2 group_validation 适配：authoritative plan binding 解析对多 target 的 units 分组语义（按 `target_repository_id` 分组、source_draft_error/无归属 fail-closed 沿用）（REQ-MTG-01）——验证：多 target binding 解析单测（分组正确+无归属拒绝）；单 target 解析回归零变化
- [ ] 1.3 WP1 关闸——验证：三处+两消费面清单逐处改造留痕；单 target 全量回归绿（零变化锁死）；REQ-COD-04 delta scenario 逐条对应

## 2. WP2 按 target 拆分创建+per-attempt 快照绑定+advance 绑定扩展

- [ ] 2.1 拆分创建：advance/建组初始化按 target 分流为每 `(plan, target)` 恰一个 target-attempt（units 按 target 分入各自 attempt；各自冻结该 target 的 `AttemptTargetSnapshot`；`admission_kind=sc_advance`/plan binding/worktree lock/units 拓扑序逐 attempt 建立）；初始化 journal 对多 target 的 checkpoint 扩展（恢复不重分配另一套）（REQ-MTG-01/02、design D1/D2）——验证：多 target 建组产出 N attempts（各自快照+units 正确分组）；重复建组/重放幂等命中（per-(plan,target) 唯一）；journal 中断恢复同套 attempts；单 target 路径回归零变化
- [ ] 2.2 唯一性与单 active 细化：`get_attempt_for_work_item_group` 检索升级 per-`(plan, target)`（消解「取最早」歧义）；issue 级单 active attempt 不变式细化为 per-`(issue, target)`（同 target 串行保留、异 target 并行解禁）——全仓消费面清单化改造（writing-plans 展开检索）（REQ-MTG-01/02、design D2）——验证：同 target 第二 active attempt 被拒；异 target 多 attempt 并存/并行执行测试绿；消费面改造清单留痕
- [ ] 2.3 advance 绑定 additive 扩展：`AdvanceRecord` 新增 target-attempt 集绑定（serde default、存量零迁移）；`advance_is_ready_for_attempt` 判定=精确匹配或集合包含（fail-closed 语义零变化）；`advance_completed` 载荷多 target 含引用集（REQ-MTG-03、design D2）——验证：守卫测试族（Ready 前拒绝/绑定缺失拒绝）per-attempt 通过；存量单 target record 判定逐字节等价回归；每 plan 仍恰一条 record（冲突测试不变）
- [ ] 2.4 start_attempt/半启动恢复适配：group 短路与 `prepare_resumed_attempt_for_runner` 按 per-attempt 各自快照/worktree 独立运转（与单 target 等价语义）；SC advance 守卫接线新绑定判定（REQ-MTG-02/03）——验证：多 target 下 start_attempt 各 attempt 独立推进测试；半启动恢复矩阵关键路径（Coding+已物化/未物化×多 target）绿
- [ ] 2.5 WP2 关闸——验证：2.1-2.4 证据齐备；REQ-ADV-01/02 delta scenario 逐条对应（含单 target 零变化 scenario）；无自动跨 attempt 编排断言（任何状态转换不触发他 attempt 启动）

## 3. WP3 group 级聚合只读视图/终态（同批交付）

- [ ] 3.1 聚合投影数据面：每 plan 的 per-target 投影（状态/stage/分支/head/push/ReviewRequest/失败阻塞原因）+聚合终态三值（全部交付/部分/未启），判定口径对齐 `issue_delivery`（每 target 最新 attempt+Completed+Pushed 才算交付）；只读派生无独立持久化状态机（REQ-MTG-04、design D3）——验证：聚合 API 派生正确性测试（含 partial failure 负向：未推送/失败/无 attempt 各态）；「不伪装全局成功」断言；无第二状态机（无新增持久化聚合记录）
- [ ] 3.2 前端 per-attempt 呈现+partial failure 视图：group workspace 的 per-target attempt 呈现与聚合终态视图（决议钉死同批交付；oracle attempt 增殖 UI 面同 WP）；聚合视图不承载操作面（决策/启动/中止仍走各 attempt 既有入口）（REQ-MTG-04、design D3）——验证：多 target 前端呈现冒烟（per-target 状态+终态+partial failure 可见）；聚合面无任何操作入口断言
- [ ] 3.3 WP3 关闸——验证：3.1-3.2 证据齐备；「同批交付」红线自查（WP2/WP3 同批验收，无拆分先行聚合缺席中间态）；REQ-MTG-04 scenario 逐条对应

## 4. WP4 底座适配核查（REQ-COD-05/06 已有实现的 per-target 适配，非重建）

- [ ] 4.1 跨仓证据注入适配核查：`evidence_injection`/`evidence_mediator`/`evidence_index` 在多 target target-attempt 下的 per-unit target 适配核查（证据来源 snapshot 与预算口径不变；检索目标=他 target 仓只读）（REQ-COD-05 语义、ter A-WP4）——验证：多 target 下跨仓证据注入测试绿（snapshot 来源+预算+审计口径与单仓一致）；核查结论留痕（无需改造则登记依据）
- [ ] 4.2 每仓交付聚合适配核查：`issue_delivery`/`git_operation` 每 target-attempt 独立交付链（branch/commit/push/ReviewRequest）多 target 适配核查（REQ-COD-06 语义）——验证：多 target 各仓独立交付测试绿（互不串扰）；WP3 聚合终态与本链判定一致性测试
- [ ] 4.3 WP4 关闸——验证：核查结论逐项留痕（改造项/无需改造依据）；底座语义零变化回归绿

## 5. WP5 恢复矩阵回归+增殖审计收口

- [ ] 5.1 attempt 增殖审计：分流创建 durable 审计记录（plan/target/attempt 身份+分流依据+触发入口）；恢复/重放溯源可读；检索语义 per-(plan,target)（REQ-MTG-05、design D4）——验证：审计记录落盘+恢复后可追溯测试；审计面无调度逻辑断言（只记事实）
- [ ] 5.2 恢复矩阵回归：创建/恢复/replay/半启动/断连重连 × 单 target（回归零变化）/多 target（每 attempt 独立等价单 target 语义）组合覆盖（ter A-WP5）（REQ-MTG-05）——验证：矩阵组合逐项留痕（阶段×target 数关键路径全量+抽样）；单 target 全量回归绿
- [ ] 5.3 二期证据留档：增殖审计+聚合终态/partial failure 分布作为二期（跨 attempt 依赖门）立项证据基础留档登记（defer 登记：触发=一期后真实多仓使用证据，另行立项）（REQ-MTG-03 defer 句、design D5）——验证：defer 登记在案且指向证据来源；本 change 无任何跨 attempt 编排落地
- [ ] 5.4 WP5 关闸——验证：5.1-5.3 证据齐备；决议一期四项逐项对照（附录 B）；范围铁律自查（无越界项、无二期预埋）

## 6. 关闸与销账

- [ ] 6.1（**归档 sync 后必做两项手工核对：①两主 spec（per-repo-coding-execution/work-item-plan-advance）的 ## Purpose 段——openspec 1.13.1 specs-apply 不应用 delta Purpose（源码 :231-246 忽略仅 warn），须手工编辑为各自 delta Purpose 并集文本（advance Purpose 并入 DEF-4 显式化句：Ready 即止+StartCoding 唯一入口+SC_CODING_REQUIRES_ADVANCE 守卫+auto 通道红线 defer）；②RENAMED FROM/TO 实际应用核对（本仓库首例）**） validate strict+主 specs 预检：`openspec validate multi-repo-group-coding --strict` 过；REMOVED/无（本 change 无 REMOVED）；`multi-target-group-coding` 入主 specs 后被引点自洽（REQ-COD-04/REQ-ADV-01/02 的分流契约指向不悬挂）——验证：validate 输出留档；主 specs 引用扫描无悬挂
- [ ] 6.2 全量门禁双绿+证据归档：lib/it_web/it_core+前端+fmt/clippy 全绿；增殖审计/恢复矩阵/核查结论归档 `cadence/`——验证：门禁结果与归档清单留档
- [ ] 6.3 关闸终检——验证：决议 Q2 一期四项+oracle 两风险承接（显式路由/增殖 UI 同批）逐项对照附录 B；单 target 零变化终检；同批交付红线终检
