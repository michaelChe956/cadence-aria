# 提案：work item group 计划生成的判定一致与可执行选项

## Why

owner 2026-10-31 真机跑多仓 work item group 时连续失败，并明确指出：

> 我最大的问题就是这个 work item group 这一块一直过不去：有写 plan 直接出错的，有写 plan 成功了但是 review 之后直接出错，说让人工处理的。我没太懂这块逻辑为什么会搞得如此复杂

**为什么现在做**：根因已定位到**代码不对称**（同一候选被两套图判定、结论相反），且它命中的是 owner 体感最差的那一类失败（review 给「无环」、终编译说「成环」）。这类失败**与模型无关** —— 修它不需要改 prompt、不需要模型更聪明，属「好修复的」部分。

**为什么先只做这一部分**：owner 2026-10-31 裁定「**复杂内容先不做，先把好修复的先修复，并尽可能保证通过率，后续再慢慢调优 prompt、校验规则**」→ 路线 B（平台派生 Plan Intent 四键）、路线 C（平台生成跨仓骨架）与 prompt 全面瘦身留待后续。

## What Changes

| # | 改动 | 落点 |
|---|---|---|
1 | **「物理仓 → 逻辑成员」反查收敛为单一来源**，三态（`Unique`/`Absent`/`Ambiguous`），多命中**绝不猜**；本树原有**三份**实现（候选结构评估 / Final Compile / `compile/ir_adapter.rs`）全部改为调用它 | 新模块 `src/product/workspace_engine/member_attribution.rs` |
2 | **候选结构评估与 Final Compile 同口径**：候选门认得出 `single_repository` 的逻辑成员归属 → 仓序自动边在两处同时生效 → **「review 说无环、终编译说成环」消失** | `plan_projection.rs` + `compile/finalizer.rs` + `compile/ir_adapter.rs` |
3 | **parity 测试**（真同时跑两处）：有效边集合逐条一致 / 成环结论一致 / witness 首跳与来源逐字一致；红态实测（改前两条用例红，报错与真机现场同形） | `tests/single_candidate/structure_parity.rs` |
4 | **依赖环给可执行选项而非归因断言**：owner 面只由 typed witness 事实构成（环上链 + 仓序位次 + 逆序那一跳及来源），给 2~3 个选项各带后果；**不再断言「根因在 design」**、不声称存在唯一合法托管仓；witness 不完整时只陈述稳定码与已知事实 | `compile_failure_surface.rs` |
5 | **结构修复指令携带算出的选项与保护项**（保留全部 canonical Inputs 业务语义、不得改 change_order、不得为消环删真实数据流）；同一结构指纹重复 → 升级 owner 决策、不再烧预算 | `compile_failure_surface.rs` + `review/routing_platform_rescue.rs` |
6 | **自动重写真正执行**：仅当「平台合成的编译失败 finding（单源前缀 `candidate_compile_failure:`）+ Verification 相位 + 返修预算未用尽」才复原为可返修并**真的委托 author 重写**；**模型产出的 finding 一律仍落人工门** | `review/routing_platform_rescue.rs` + `review/routing.rs` |
7 | **Plan Intent 取值来源按会话类型唯一**（单仓会话取 `[issue]` 头 `repo=`；LC 多仓会话取成员仓清单里的物理成员仓 ID）；删除逐字印着报错原文的噪声句；**「模型自己填归属」规则保留**（平台接管前不得删） | `sc_author_teaching.rs` |

## Impact

- **owner 侧**：需要人判断时，拿到的是**一个能懂的问题 + 2~3 个带后果的选项**，而不是「依赖环」三个字或「请去改 design」这种**本案为错**的归因；「review 说没问题、终编译才报错」这一形态消失
- **平台侧**：归属判定只有一处实现 → 将来改动不会只改一侧；自动重写第一次真正跑起来（此前 0.67 秒就落到人工门、一次 author 重写都没发生）
- **可观测**：尝试流水与通过率读数在 `mechanicalize-plan-compile-repair` 的 3.1 / 3.2 落地 → 「保证通过率」从空话变成可测
- **契约**：本 change 只含**已兑现**的 5 条（`REQ-WG-01` / `REQ-WG-02` / `REQ-WG-05` / `REQ-WSC-06` / `REQ-WG-08`）

## Non-Goals

- **Plan Intent 四键由平台派生**（路线 B，5–7 人日）→ 留在 `unify-work-item-group-structure` 延后
- **仓序边平台持有 / Inputs 方向冲突检出**（路线 B）→ 同上
- **平台接管后移除 prompt 结构契约** → 依赖上面两条 → 同上
- **平台生成跨仓骨架**（路线 C，9–12 人日）→ 建议推迟（骨架与 design 耦合，改仓序就要重算）
- **prompt 全面瘦身**（固定教学块 15.5K → ≤8K）→ 待本批上线观测后再定
- **不改机器门禁的判定语义**（`**/x` 类 scope 等于全放行、门禁硬编码大小写敏感）→ 独立票据，不混入本 change
- **不删边消环**（`REQ-SG-02A`「禁止假删边」）：平台不替 owner 做业务取舍
