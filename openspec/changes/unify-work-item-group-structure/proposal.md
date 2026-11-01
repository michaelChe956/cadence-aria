# Proposal: unify-work-item-group-structure

## Why

owner 2026-10-31 第二轮反馈：「**我最大的问题就是这个 work item group 这一块一直过不去**：有写 plan 直接出错的，有写 plan 成功了但是 review 之后直接出错、说让人工处理的。**我没太懂这块逻辑为什么会搞得如此复杂**。」

本轮用真实现场（issue `issue_0001` / session `auto_2adc5e41`，`prompt_chars: 31229`）取证到一条**纯机械**的根因，不是模型能力问题：

**同一份候选计划，被两套图模型判定，且两套对「这个 Work Item 属于哪个仓」的口径相反。**

1. 候选结构门（review 之前）：`planning_structure_items_from_ir` 只对 `intent_target_kind=logical_codebase` 置 `logical_target`，`single_repository` 分支落 `(None, None)`（`src/product/workspace_engine/plan_projection.rs:981-1014`）；而 `project_effective_planning_graph` 的 change_order 自动边只对 `logical_target` 非空的项生效（`draft_batch/compile_support.rs:534-540`）。
2. 物证：喂给 reviewer 的确定性结构报告里 **5 条有效边全部是 `canonical_input`、`order_indices` 全空**、`structurally_valid: true`、`cycle_witness: []`（`work-item-plan-sources/…/mechanical_report/report-956ec37e5e6777e7.json`）。所以 Review Round 1 敢写「4 个 WorkItem 构成无环拓扑」。
3. Final Compile 侧相反：`SingleRepository { repository_id }` 会被 `reverse_logical` 反查成逻辑成员（`workspace_engine/compile/finalizer.rs:924-937`、`:961-963`），change_order 自动边因此生效，成环 → `final_compile_prepare_failed`。

结果就是 owner 看到的「review 说没问题，点确认才报错」。而失败后唯一的出口是**让 author 重写整份 plan**（`compile_failure_surface.rs:449-495`），平台那份图是平台算的，重写不会改变它 —— 最终停在人工门，且提示断言「根因在 design、需要你改 design」（`compile_failure_surface.rs:363-401`），本案这句话是**事实错误**（design 仓序正常，错在 plan 把集成验收项托管在仓序第 2 的仓）。

第二类失败（写 plan 直接出错）同样是机械的：`Plan Intent` 在场却缺本分支 key 即 `lowering_error`（`work_item_plan_compiler/lower.rs:421-435`、`:443-480`），而 prompt 自身给出两条互斥取值来源（`prompts/sc_author_teaching.rs:41` vs `:253`），并在 `:41` 逐字印着这句报错原文 —— 「把规则再写一遍」这条路已经用尽。

## What Changes

- **同一事实单一来源**：新增「物理成员仓 → 逻辑成员」反查的单一实现（三态 `Unique/Absent/Ambiguous`），候选结构门与 Final Compile 共用；候选门对 `single_repository` 也解析成员归属，使仓序自动边在 review 之前就生效。
- **图奇偶性**：同一 source 经两处判定，有效边集合 / 成环结论 / witness 必须一致；不一致以结构失败停等并留证。
- **`Plan Intent` 目标四键平台派生**：`intent_target_kind/_repo/_codebase/_logical_repo` 由平台按成员表 + 写范围前缀派生；`intent` / `provider_work_item_id` 仍由 author 声明（REQ-C1-PLAN-01 禁止代填）。派生能力需先给 lowering 上下文接入成员 checkout 前缀表；未接线前 MUST NOT 允许省略该段（现状省略会静默归首仓）。
- **仓序边由平台持有**：平台派生并持有跨仓边；author 的 Inputs 方向与之冲突时由平台**检出并给选项**，平台不代写 Inputs、也不要求 author 理解 `change_order`。
- **环提示改为 witness 事实 + 2~3 个可执行选项**：删除「根因在 design / 唯一合法托管仓」这类断言（REMOVED `REQ-WSC-20`，由 `REQ-WG-05`/`REQ-WG-06` 取代）。
- **prompt 去矛盾 + 防回长**：删除互斥取值来源与已被平台接管的条款；固定教学块体量与硬措辞数由计量测试锁定。

## Impact

- 受影响 spec（delta）：
  - 新增能力 `work-item-group-structure`：`REQ-WG-01`..`REQ-WG-04`、`REQ-WG-06`、`REQ-WG-07`
  - `work-item-plan-single-candidate`：**REMOVED** `REQ-WSC-20`（该条强制「必须把根因指向 design」；替代条款为新能力的 `REQ-WG-05` / `REQ-WG-06`，「候选失败必须完整可读呈现」仍由既有 `REQ-WSC-18` 承担）
- 代码面（本 change 只做前两项，见 tasks.md 分期）：
  - `src/product/workspace_engine/plan_projection.rs`（`planning_structure_items_from_ir` 签名 + `SingleRepository` 分支）
  - `src/product/workspace_engine/compile/finalizer.rs`（`reverse_logical` 改为共用实现）
  - `src/product/workspace_engine/draft_batch/compile_support.rs`（新增反查函数，与 `plan_compile_member_targets` 同处）
  - `src/product/workspace_engine/compile_failure_surface.rs`（`owner_hint` / `rewrite_guidance`）
  - 后续阶段：`src/product/work_item_plan_compiler/lower.rs`、`src/product/work_item_split_engine/prompts/sc_author_teaching.rs`
- 兼容：
  - legacy 单仓（成员表为空）路径**零变化**：`plan_projection.rs:1044-1052` 的 legacy 清空块保持不动。
  - `finalizer` 的输出逐字不变（只是代码来源统一）。
  - 不新增 provider attempt、不改状态机、不动 `CandidateCompileStatus` 线格式。

## Non-Goals

- **不实施**：本 change 只出契约与任务分解，不改实现（owner 要求只出方案）。
- 不自动删任何 `canonical Inputs` 边消环（那会删掉真实数据流）。
- 不自动改 design 的 `change_order`。
- 不给「集成类 Work Item 该托管哪个仓」编唯一解（写范围约束使多解并存，只能给选项）。
- 不在本 change 内处理 review 多轮未收敛（`e5f6b290` 类）—— 现场数据已删，只留回放用例。
- 不处理 `outbound_queue_full` 等会话出站可靠性问题（另开工单）。
