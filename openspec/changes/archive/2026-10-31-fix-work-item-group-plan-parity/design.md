# 设计：work item group 计划生成的判定一致与可执行选项

> 完整设计见 `cadence/designs/2026-10-31_技术方案_Plan生成不可控性治理_v2.0.md`（两位设计角色讨论收敛，413 行）。本文件只记**本 change 范围内**的落地要点。

## 1. 根因（物证级）

同一候选被两套图判定：

- 候选结构评估 `src/product/work_item_plan_compiler/plan_projection.rs`（旧 `plan_projection.rs`）对 `SingleRepository` 归属落 `(None, None)` → 仓序自动边一条不加 → 判「无环」
- Final Compile `src/product/workspace_engine/compile/finalizer.rs` 物理仓反查成逻辑成员 → 仓序边 5 条生效 → 判「成环」

物证：喂给 reviewer 的 `mechanical_report/report-956ec37e….json` 里 5 条有效边全是 `canonical_input`、`order_indices` 全空、`structurally_valid: true`、`cycle_witness: []`。

## 2. 落地要点

| 面 | 落点 |
|---|---|
归属反查单一来源（三态）| 新模块 `src/product/workspace_engine/member_attribution.rs`：`MemberReverse{Unique,Absent,Ambiguous}`；本树原有**三份**实现（候选门 / 终编译 / `compile/ir_adapter.rs`）全部改为调用它 |
候选门认得出 `single_repository` 归属 | `plan_projection.rs` → `resolve_member_targets`；`Absent`/`Ambiguous` **不造归属、不计仓序边**（REQ-WG-02 明文） |
parity 测试（真同时跑两处）| `src/product/workspace_engine/tests/single_candidate/structure_parity.rs`：有效边集合逐条一致 / 成环结论一致 / witness 首跳与来源逐字一致；**红态实测**（注掉调用 → 两条用例红，报错与现场 D8/D5 同形）|
owner 面：可执行选项而非归因断言 | `compile_failure_surface.rs`：`cycle_witness_facts`（fail-closed 解析 witness）+ `owner_hint`（位次事实 + 两个修法各带后果）+ `rewrite_guidance` / `author_teaching_block`（携带位次 + 保护项）|
自动重写真正执行 | `review/routing_platform_rescue.rs`：`merge_platform_compile_failure_origin` **只**把带 `candidate_compile_failure:` 单源前缀的**平台合成** finding 并入 Verification scope 的 `original_fingerprints`；`rescue_platform_compile_failure_repair` 仅当四条件满足才复原为 `Repairable → TriggerAggregateRepair → request_provider_run(SC author)`。**模型产出的 finding 一律不并入、守卫字节未动**（两条红线用例钉住）|
prompt 取值来源按会话类型唯一 | `sc_author_teaching.rs`：改为一条按会话类型分支的权威句；删掉逐字印着报错原文的噪声句；**「模型自己填归属」规则保留**（平台接管前不得删）|

## 3. 风险与红线

- **不删边消环**（`REQ-SG-02A`「禁止假删边」）：平台不替 owner 做业务取舍
- **已持久或已评审的 source 不直接改写**
- **平台不自动补验证 check、不改 change_order**
- 红线：豁免只认**平台合成** finding 的**单源前缀**；同一轮里**模型产出**的 actionable finding 仍须落人工门
- 反向风险（自修率过高会掩盖模型缺陷）→ 自修率进观测指标
