# 阶段 4 C2 T2（WP2 拆分编排+advance 绑定）执行报告（部分交付）

- **change**: `openspec/changes/multi-repo-group-coding/`（REQ-MTG-01/02/03，REQ-ADV-01/02）
- **计划**: `cadence/plans/2026-09-19_计划文档_阶段4-C2_多仓group编码_v1.0.md`（v1.1）Task 2
- **工作树**: `.worktrees/feat-b-0808-add-monorepo`（基线 `c16e497c`；期间兄弟提交 `542cc5af`（F16Fix，零文件交集））
- **状态**: 分批纪律中途交接——Step 2/4 全量+Step 3 存储面已提交（两批），**advance.rs 分流循环（k3 F3 必改点一）与 engine 级多 target 测试族未实施**（会话预算约束，见 concerns）
- **详细留痕**: `cadence/reports/multi-repo-group-coding/wp2-split-report.md`（交付清单/消费面表/D2.1 对应/剩余工作交接）

## 交付摘要

1. **Batch 1 `4830bf03`（Step 2+Step 4）**：advance additive 集绑定（OQ1 全形态：`AdvanceTargetAttemptBinding`/`AdvanceRecord.target_attempts`/journal `target_attempt_ids`/`AdvanceOutcome::Completed` 载荷/WS `AdvanceCompleted` additive——单 target wire 零变化测试钉死）；`advance_is_ready_for_attempt` 精确匹配 OR 集合包含（fail-closed 零变化）；`map_advance_outcome` 多 target 完备判据（k3 F3 必改点二）；外层 journal 集绑定 `load_or_prepare_advance_initialization_for_attempts`。
2. **Batch 2 `942a8373`（Step 3 存储面）**：per-(plan,target) 唯一性+per-(issue,target) 单 active（D2.1 A1/A4 豁免落地）；`get_attempt_for_work_item_group` 检索升级+`list_attempts_for_work_item_group`；per-target journal 子路径（OQ2 路径规则+定位规则+幂等重放）；消费面 7 处产码+14 处测试调用点全量改造（OQ4 留痕）。

## 验证证据

- 红绿：Batch 1 编译红 11 errors→17/17+16/16 绿；Batch 2 编译红→11/11 绿（1 处测试序断言修正）。
- 单 target 回归零变化：125/125（advance_handler 全族+coding_attempt_store 全量+amendment 链相关）。
- 全部定向测试在主工作树跑绿；隔离 worktree 终验属剩余工作（Batch 3 后执行）。

## Commits

- `4830bf03` feat(coding-ws): advance additive set binding + ready judgment extension + map multi-target completion (WP2 Step2/Step4)
- `942a8373` feat(coding-ws): per-(plan,target) uniqueness + per-(issue,target) single-active + per-target journal subpath (WP2 Step3 storage face)

## Concerns

1. **未完成主体**：advance.rs 分流循环（必改点一）、worktree 嵌套共存检查、审计占位、engine 级多 target 测试族（Step 1 #1/#2/#3/#8/#9/#10 engine 面）、隔离树终验——精确交接清单在 wp2-split-report.md「六」。当前态：mixed-target 建组在存储面已就绪，但 engine 编排仍走单值路径（T1 后预期中间态延续）。
2. 主树并行兄弟（CapFixer/F16Fix）中间态曾挡全量编译；本任务以显式文件清单提交隔离，两批提交均在其编译窗口恢复后落盘。
3. 预算约束下 map/WS 前端 TS 类型未动（OQ1 定案 wire additive 空集不发送，前端零感知；T3 前端面消费时补类型）。
