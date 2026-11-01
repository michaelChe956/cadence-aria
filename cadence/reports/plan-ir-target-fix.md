# 多仓 plan IR 聚合视图目标修复报告（plan-ir-target-fix）

- 日期：2026-10-12
- 分支：feat-b-0808-add-monorepo
- 主题：多仓 Plan Final Compile 报 `work_item_target_missing`（IR target=`lc_aggregate_root_view_<uuid>`）根修

## 1. 定界结论：存量缺陷，非 3372bdbe 回归

`git diff --stat 3372bdbe^..HEAD -- src/product/workspace_engine/conversational_gate_parts/revision_turn.inc.rs src/product/workspace_engine/compile/ src/product/workspace_engine/draft_batch/compile_support.rs` → **零改动**。

3372bdbe 只改 launch anchor（恒聚合根），未触碰 business/IR 面。缺陷链自 S6 聚合兜底（`workspace_repository.rs:508` `plan_session_repository_target` 多 target+focus 非唯一 → `AggregateRoot`）起即存在：多仓 plan 会话首次走到 Final Compile 才暴露。

## 2. 根因链

1. 写入点（fresh author `web/.../run/single_candidate.rs` 与门内人工修订 `revision_turn.inc.rs:118-146`）把 `workspace_repository_for_session` 解析的会话仓 ID 直接作 `WorkItemPlanSourceContext.target_repository_id`；聚合会话该值 = `lc_aggregate_root_view_<uuid>`。
2. `lower_work_item_plan` 把该会话级 target 复制到**每个** IR item 的 `target_repository_id`。
3. Final Compile（`compile/ir_adapter.rs:223-245` `logical_target_for_ir_item`）只认成员**物理仓 ID**（logical_targets 的 values）；聚合视图 ID 不匹配 → `work_item_target_missing: IR target ... is not effective` → owner 验收 9 次确认重试全同错（session_0003 现场）。

## 3. 修法（按 B1 铁律：聚合视图 ID 不得流入 IR target；保留现业务 target/材料来源，launch anchor 不动）

### 3.1 per-WI 成员归属解析（lowering 层，单一收口）

- `WorkItemPlanSourceContext` 新增 `member_targets: Option<PlanMemberTargets>`：
  - `physical_by_logical`：逻辑成员 → 物理成员仓 ID（与 Final Compile `logical_targets` 同源构建）；
  - `fallback_physical_id`：无归属 WI 的确定性回落。
- `resolve_item_target_repository_id`（lower.rs）：
  - `member_targets=None`（legacy 单仓）→ 沿用会话 target，语义零变化；**B1 守卫**：会话 target 为聚合视图 ID 时 fail-closed（`work_item_target_missing`），漏传成员表的调用方在写入点即报语义错，不再流入 IR 后于 Final Compile 卡死；
  - `member_targets=Some`（LC）→ 每 WI 按 Plan Intent 成员归属落成员物理 ID：
    - `logical_codebase` 归属 → 查成员表（不在有效成员集 → 语义错）；
    - `single_repository` 归属 → 必须命中成员物理仓（非成员仓 → 语义错）；
    - 无 Plan Intent → 落确定性回落（成员锚定会话=会话成员物理仓＝既有 focus/唯一成员规则；聚合会话=confirmed design `change_order` 首成员物理仓，r62 design 首仓兜底先例）；回落缺失 → 语义错 fail-closed（task 裁定：无归属 WI 报语义错）。

### 3.2 写入点接线（三个生产写点全覆盖）

- `compile_support.rs` 新增 `plan_compile_member_targets(lifecycle, plan, session_repository)`：复用 `resolve_logical_work_item_plan_repository_targets`（Legacy→None 零变化）+ `is_aggregate_root_view_repository` 判定回落规则。
- 门内人工修订：`revision_turn.inc.rs`（解析成功臂计算成员表；legacy 回退臂 None）。
- fresh author：web `run/single_candidate.rs` 在 repository 解析后计算一次，同源用于 compile 预检循环、`complete_single_candidate_work_item_plan_author`（新增第三参）。
- `workspace_repository.rs` 抽出 `AGGREGATE_ROOT_VIEW_ID_PREFIX` 常量 + `is_aggregate_root_view_repository` 谓词（写点与 lowering 守卫同源）。

## 4. TDD 证据

红→绿：

- 单元（`work_item_plan_compiler/tests/member_targets.rs`，7 例）：per-WI 归属落物理仓 / 无归属回落 / 无归属无回落语义错 / 未知逻辑成员语义错 / 非成员物理仓语义错 / legacy 零回归基线 / 聚合视图 ID 守卫。红：6 失败（实现前）；绿：7/7。
- E2E（`web/.../single_candidate_provider_run/aggregate_ir_target.rs`）：NonUnique focus 双成员聚合会话真链——IR item target=各自成员物理仓 id（非聚合视图）→ 确认 Final Compile → Confirmed/Completed、双 work item。红：卡 AuthorRun（B1 守卫在旧写入点拒收）；绿：全链通过。
- 回归批（单命令）：`cargo test --lib -- work_item_plan_compiler ir_adapter anchor_t17 single_candidate conversational_gate workspace_repository planning` → **448 passed / 0 failed**（含 anchor_t17 SC 真链、single_candidate 族、planning、路由 S6 族）。

## 5. 影响面与零回归红线

- legacy 单仓 plan：member_targets=None 路径逐行零变化（回归批含全量 compiler/SC 用例）。
- 成员锚定 LC 会话（唯一 target/唯一 focus）：行为增强——有归属 WI 从「恒会话 focus 仓」修正为「各自归属仓」；无归属 WI 仍落会话成员仓（原值）。
- 聚合会话：由「IR 携聚合视图 ID → Final Compile 恒死」修正为「per-WI 成员物理仓 / 语义错前置到写入点」。
- 现业务 target/材料来源/launch anchor（3372bdbe 语义）不动。

## 6. 遗留与建议

- E2E 终态断言按 anchor_t17 同口径（plan_store 收口 `plan.work_item_ids`）；SC Final Compile 不在 lifecycle work-items 目录留档，如需逐 WI 仓落点直读可在后续报告口径统一。
- 真实 session_0003 现场数据未在本地复现（运行时数据不在仓内），修复以代码链路 + fixture 真链 E2E 为准；建议 owner 用原会话重试确认验收解卡。
