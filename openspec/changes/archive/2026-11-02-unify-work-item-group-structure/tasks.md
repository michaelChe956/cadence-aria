# 任务：unify-work-item-group-structure

> 只出方案，本 change 未开始实施。任务按「先止血 → 形态一消失 → 防回长」分期，每期可独立提交、独立验收。

## 1. 止血：反查单一来源（REQ-WG-02）

> 实施落点（A1，2026-10-31）：反查函数落在**新模块**
> `src/product/workspace_engine/member_attribution.rs`（controller 授权的
> 「新模块」选项），不放 `compile_support.rs`——后者 2309 行已顶到登记上限
> 2310，零增长余量。模块在 `workspace_engine/mod.rs` 以
> `pub(crate) mod member_attribution;` 注册。

- [x] 1.1 在 `src/product/workspace_engine/member_attribution.rs` 新增 `MemberReverse`（`Unique(LogicalRepositoryId) | Absent | Ambiguous`）与 `reverse_member_logical(physical, &physical_by_logical)`，注释标明「唯一来源」。
- [x] 1.1b 本树里同一个反查原有**三份**实现，第三处 `compile/ir_adapter.rs` 的 `logical_target_for_ir_item` 也改为调用 1.1（经 A3 同意后改，两条错误文案逐字保留，含 Ambiguous 分支不带稳定码前缀的现状）。
- [x] 1.2 `src/product/workspace_engine/compile/finalizer.rs` 的本地 `reverse_logical` 闭包改为调用 1.1（`:929-939`）；`Absent` / `Ambiguous` 复用现有 `work_item_target_missing` 文案（逐字不变，由新增用例逐字断言）。
- [x] 1.3 单测：`src/product/workspace_engine/tests/part_03/park1_work_item_records.rs` 既有三分支用例全绿；新增 `attribution_ambiguous_physical_member_fails_closed_with_existing_wording` 断言 `Ambiguous` 报错文案与现状逐字一致。
- 验收：`finalizer` 行为零变化（既有测试不改一字而全绿）。

## 2. 止血：候选门认出 single_repository 成员归属（REQ-WG-01、REQ-WG-02）

> 实施落点（A1，2026-10-31）：**不改 `planning_structure_items_from_ir` 的签名**
> ——归属解析（需要成员表）抽到 `member_attribution::resolve_member_targets`，
> 由 `assess_single_candidate_structure` 在投影之后调用。因此 2.2 的 4 个调用点
> 零改动（该函数已移到 `member_attribution`，`plan_projection` 保留同名
> re-export，调用点路径不变）。

- [x] 2.1 `single_repository` 声明物理仓 → `ReverseMember::Unique` 时 `logical_target = Some`；`Absent` / `Ambiguous` 保持 `None`（不猜、不新增失败种类）。集合 intent 分支语义原样保留。
- [x] 2.2 调用点同步：**无需改动**（签名未变；`plan_projection.rs` 以同名 re-export 承接）。
- [x] 2.3 保持 `plan_projection.rs` 的 legacy 清空块不动（成员表为空时零变化）。
- [x] 2.4 parity 用例（先红后绿）：`src/product/workspace_engine/tests/single_candidate/structure_parity.rs`（经 `include!` 内联）——同一 fixture 真跑候选门与终编译 `prepare_initial_plan_compile`，断言有效边集合逐条一致、成环结论一致、witness 首跳与链延续逐字一致。
- [x] 2.5 补充：**未声明 Plan Intent** 的 WI 也按同一反查从 IR 已定的回落物理仓解析归属（否则这一类仍在候选门落 `(None, None)`，REQ-WG-01 的「两处结论一致」不成立）。
- 验收：本案形态的候选在 review 之前被拦下；legacy 单仓用例全部保持原结论。

## 3. 止血：owner 面提示改为 witness 事实 + 选项（REQ-WG-05、REMOVED REQ-WSC-20）

- [x] 3.1 `src/product/workspace_engine/compile_failure_surface.rs` `owner_hint`：删除「根因指向 design / 需要 owner 修改 design」断言；witness 完整时渲染两跳来源 + 两个仓序位次 + 两个选项及后果。（A3：`owner_hint` 改收 `&CandidateCompileFailure`，witness 事实由 `cycle_witness_facts` 从 producer 固定模板 fail-closed 解析。）
- [x] 3.2 `rewrite_guidance` 删除「根因可能在 design」条目；`author_teaching_block` 改为携带算出的位次与选项 + 保护项。（A3：两者均改 witness-aware；witness 不完整时退化为无归因通用要点。）
- [x] 3.3 单测：witness 完整 → 不含「修改 design」、含两个位次与两个选项；witness 不完整 → 不含任何位次/归因断言。（A3：`tests/candidate_compile_failure_surface.rs` 与 `compile_failure_surface.rs` 内嵌 tests。）
- 验收：用本案失败记录重放，owner 面文案不含 design 归因。

## 4. 形态一消失：Plan Intent 目标四键平台派生（REQ-WG-03，≥2.5 人日）

> 前置提醒：这不是"把已有能力提上来"。`PlanMemberTargets`（`lower.rs:47-53`）缺成员 checkout 前缀表，必须先接线。

- [ ] 4.1 把成员 checkout 前缀表接入 lowering 上下文（现状只在写范围校验期传入，见 `write_scope_bound.rs:110-148`）；对「跨成员 / 无前缀 / 不唯一」定义 typed 缺口。
- [ ] 4.2 `lower.rs` 按写范围前缀 + 成员表派生 `intent_target_kind/_repo/_codebase/_logical_repo` 四键；**`intent` / `provider_work_item_id` 不派生**，缺席仍按 REQ-C1-PLAN-01 停等。
- [ ] 4.3 **仅在 4.1/4.2 完成后**放开「允许省略 `### Plan Intent` 段」；在此之前省略 MUST 仍以 typed 缺口停等（禁止落入 `fallback_physical_id` 静默归首仓，`lower.rs:381-392`）。
- [ ] 4.4 冲突以派生为准 + 审计事件（原 key/值/派生依据），随 source revision 落盘。
- [ ] 4.5 测试：省略段时可从写范围派生且不归首仓；`intent` 缺席仍 `intent_undeclared` 停等；冲突候选编译成功且有审计；跨成员写范围被 typed 拒绝。
- 验收：`lowering_error:0:Plan Intent 声明在场时该字段必填。` 在该形态下不可再复现；且无"静默归首仓"的新路径。

## 5. prompt 去矛盾 + 防回长（REQ-WG-07）

- [x] 5.0 **A+（0.3 人日，可先单独发）**：删掉 `sc_author_teaching.rs:41` 与 `:253` 互斥取值来源中的**一句**（保留另一句并标注唯一来源），消除形态一在 prompt 侧的唯一成因；不依赖其他改动。
  - 实际落地（2026-10-31）：不是「删一句留一句」，而是把同一字段的取值来源收敛成**一条按会话类型分支的权威句**（grammar 段：单仓会话=本 prompt 无成员仓清单 → 逐字取 `[issue]` 头 `repo=`；LC 多仓会话=有成员仓清单 → 逐字取该清单的物理成员仓 ID）；`:253` LC 成员段删掉「改为取成员表物理成员仓 ID、不取 `[issue]` 头 `repo=`——grammar 段……仅是单仓默认取值」的改口互斥句，只保留「取上列物理成员仓 ID」+ 否定式提醒「（不取 `[issue]` 头 `repo=`）」。同批删除 `:41` 逐字印着 lowering_error 原文「Plan Intent 声明在场时该字段必填。」的噪声句。**「模型自己填归属」的分支必填规则保留**（A1 平台接管归属未落地前不得删）。契约：`src/product/work_item_split_engine/tests/prompt_contract_plan_intent_target.rs`。
- [ ] 5.1 删除 `sc_author_teaching.rs:41` 的「逐字取 [issue] 头 repo=」与 `:253` 的互斥改口（四键派生上线后，二者都不再需要）。
- [ ] 5.2 删除已被平台接管的条款（Plan Intent 分支必填、仓序自动边规则复述）。
- [ ] 5.3 新增计量测试：固定教学块字符数与硬性措辞数上限（基线按实施时实测锁定，只降不升）。
  - 部分覆盖（未勾选）：A+ 只对 Plan Intent 归属取值来源块做了计量锁（规则句字节上限 320B + 硬性措辞数上限 + 「取值来源」权威句计数 == 1），见 `tests/prompt_contract_plan_intent_target.rs`；其余教学块的字符数/措辞数计量未做。
- [x] 5.4 既有 prompt 契约测试同步（`tests/prompt_contract*.rs` 中引用被删条款的断言）。
- 验收：互斥取值来源不存在；体量测试可拦住回长。

## 6. 收口

- [ ] 6.1 清理 `PlanningStructureReport` / `project_effective_planning_graph` 上已失真的「尚未接线」注释与 `#[allow(dead_code)]`。
- [ ] 6.2 把「仓序边生成」抽成纯函数，`project_effective_planning_graph` 与 `apply_change_order_cross_repo_depends_on` 共用。
- [ ] 6.3 与 `openspec/changes/mechanicalize-plan-compile-repair/` 的 `REQ-WSC-23/24/25` 去重（择一保留），避免两套断言漂移。
- [ ] 6.4 观测：记录改动前后「候选首次通过率 / review 通过后编译失败率」，作为二期是否继续的依据。

## 7. 待验证假设（不改契约，取证后再决定）

- [ ] 7.1 `e5f6b290` 形态（review 连报三轮 + `validation_reject` 停原地）：现场已删。需构造回放用例验证两条假设：(a) 相邻两轮非 advisory 指纹集合不相等导致重复闸门失效；(b) owner 排除 key 用 `severity+message` 计算、换措辞即失效。**未取证前不下机理结论。**
- [ ] 7.2 `1cca3d60` 已实现的自动重写通道在多仓 plan 上的实际收敛率（本案 1 次未收敛即停）。
