# SC 门内修订批准元组随 refs 轮换——修复报告

- 日期：2026-10-12
- 范围：纯后端（`lifecycle_store/workspace_single_candidate.rs` + 测试）
- 诊断来源：max-task（`MaxPlanConfirmRecheck`）冻结诊断；修法按诊断执行

## 1. 缺口

`lifecycle_store/workspace_single_candidate.rs` 的 `complete_human_gate_revision`
CAS（原 L542-546）在门内人工修订完成时轮换 source/IR/report 三 refs，却保留旧的：

- `approval_attempt_id` / `approved_at`（与旧 refs 绑定的批准元组）
- `compile_reservation`（旧编译代次的预约）
- `publication_provenance_ref`（旧编译代次的 provenance 引用）

事故链（与 0009 同类、F-54 B1 的门内修订变体）：

1. 首轮 confirm 进入 compile，在 Approval/reservation/provenance 持久化之后
   中断（如 provenance 边界故障）→ 门按 F-54 B2 重开 `WaitingForHuman`，
   durable 留下与 v1 refs 绑定的旧元组。
2. 人工在门内修订（`run_sc_manual_revision_turn`）→ 新 source 内容寻址出
   v2 refs → 修订 CAS 换 refs 但旧元组未失效。
3. 复评通过路由回 Approval 相位。
4. 再 confirm → `compile/single_candidate.rs`（L181-224）以新 refs 重算
   attempt id ≠ durable 旧元组 → 命中 `single_candidate_approval_conflict`
   冲突臂 → "single-candidate approval compile failed; human gate remains
   open"，confirm 永久失败，门死锁。

普通 generation 修复轮已有对应失效逻辑（F-54 B1，
`compare_and_save_single_candidate_generation` L449-458），门内修订 CAS 缺失同构。

## 2. 修法（按冻结诊断）

在 `complete_human_gate_revision` 现有 CAS 内，按 refs 真正轮换失效旧批准绑定：

- `refs_rotated` = 三 refs 任一与 durable 现值不同；
- 轮换时清空 `approval_attempt_id` / `approved_at` / `compile_reservation` /
  `publication_provenance_ref`（旧 provenance 引用同属旧编译代次，一并失效）；
- refs 未轮换（同 source 重放：IR/report ID 为 source-only 内容寻址，同 source
  即同 refs）时现元组仍与 refs 精确绑定，原样保留——同 refs 无新代次可言；
- 不触碰 source store、不静默覆盖 immutable 记录、不在 confirm 中 re-lower。
  下次 approve 走 `(None, None)` 全新元组分支，新 attempt id 按新 refs 计算，
  新 `compile_id` 派生新 provenance 记录——同 source 约束下自然建立新代次。

## 3. TDD 证据

红（先写测试，修复前运行）：

- store 层 `human_gate_revision_rotation_invalidates_stale_approval_tuple`：
  断言修订轮换 refs 后元组四字段为 None——实际 `Some("8f6c…21d1")`，FAILED。
- 引擎层 `conversational_gate_revision_rotates_stale_approval_tuple_then_confirm_compiles`：
  真实事故链（failpoint 在 `ProvenancePersisted` 打断首轮 confirm → 门重开 →
  门内修订 v2 → 路由回 Approval → confirm）。修复前在"修订轮换 refs 必须失效
  旧 Approval 元组"断言 FAILED；临时屏蔽中间断言后 confirm 直接 FAILED 于
  "single-candidate approval compile failed; human gate remains open"
  （即 compile 冲突臂原貌）。

绿（修复后同一组测试）：

- 上述两测试 + 既有 `repair_generation_rotation_invalidates_stale_approval_tuple`
  全部通过；引擎测试终态 `Confirmed + Completed`，新 attempt id/compile id/
  provenance ref 均与旧代次不同。

## 4. 回归

命令（worktree `.worktrees/feat-b-0808-add-monorepo`，`cargo test --lib -- <filter>`）：

| 过滤族 | 结果 |
|---|---|
| `single_candidate` | 172 passed / 0 failed |
| `conversational_gate` | 129 passed / 0 failed |
| `anchor_t17` + `anchor_t18` | 7 passed / 0 failed |
| `lifecycle_store::` + `human_gate` | 124 passed / 0 failed |

改动文件 rustfmt（style_edition 2024）校验干净；`workspace_single_candidate.rs`
L8-11 的 import 单行差异为工作区既有格式扫尾改动（非本次提交内容），未触碰。

## 5. 改动清单

- `src/product/lifecycle_store/workspace_single_candidate.rs`：
  `complete_human_gate_revision` 增加 `refs_rotated` 条件失效（净 +18 行语义修复）。
- `src/product/lifecycle_store/workspace_single_candidate_tests.rs`：
  store 层红→绿测试（轮换失效 + 复评 approve 不冲突 + 同 refs 重放保留有效元组）。
- `src/product/workspace_engine/tests/conversational_gate_revision/part_03.rs`：
  引擎层端到端红→绿测试（真实事故链 + 新代次断言）。

提交：`fix: SC 门内修订批准元组随 refs 轮换`
