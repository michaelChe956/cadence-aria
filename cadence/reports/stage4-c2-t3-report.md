# Stage 4 C2 Task 3 交付报告：WP3 group 级聚合只读视图/终态（REQ-MTG-04）

- **Task**: C2 计划 Task 3（WP3.1-3.3，同批交付红线）
- **执行**: C2T3
- **commit**: `2b184d7d` feat(coding-ws): plan-level group aggregate read-only projection + three-valued terminal state + per-target frontend view (WP3, REQ-MTG-04, multi-repo-group-coding)
- **工作树**: `.worktrees/feat-b-0808-add-monorepo`（基于 `d30f98f0`）
- **详细留痕**: `cadence/reports/multi-repo-group-coding/wp3-aggregate-report.md`

## 交付摘要

| 面 | 内容 |
|---|---|
| 数据面 | `plan_group_projection.rs` 新增：`compute_plan_group_projection`（只读派生，无写入）+三值 `PlanGroupOverall{AllDelivered,Partial,NotStarted}`+`PlanTargetEntry`（OQ3 形态，blocked_reason=manual_recovery_reason→push_error→失败态文本） |
| 三值判据 | 约束 9 定案：未启（优先于部分）=无 target-attempt 或全部未离开 `(Created, PrepareContext)`；全部交付=每 target 最新 attempt Completed+最新 ReviewRequest Pushed（issue_delivery.rs:110-113 口径）；部分=其余 |
| API | `issue_lifecycle` plan DTO additive `group_projection`（serde default 旧响应兼容；`overall` 不复用 issue 级 `"none"`）+`prepare_work_item_plan` 响应同款；DTO 映射复用既有 status/stage/push 文本函数 |
| 前端 | `PlanGroupProjectionPanel`（三值徽标+per-target 行+partial failure 未满足明细+blocked_reason）挂 `LifecycleCardDrawer` work_item_group 分支；类型面+`toDrawerEntity`+`normalizeLifecycleResponse` 校验 additive |
| 禁动作 | 聚合区零按钮/零 role=button/零链接（Vitest 断言）；决策/启动/中止走各 attempt 既有入口 |

## 验收对照

- **三值红绿**：12 数据面测试全绿（三态/partial failure 三负向/最新 attempt 口径/A4 无快照排除/单 target 与 issue 口径一致）；红态以 mutation 反证（移除未启优先分支→not_started 测试立即 FAILED，已还原复绿）。
- **只读派生**：`.aria` 全树 inventory（路径+mtime 纳秒+长度）调用前后零 delta（store 级一次性 legacy 懒迁移在夹具预热后稳态断言——非聚合写入）。
- **前端断言**：Panel 6 项+Drawer 接线 1 项；`npx vitest run src/components/lifecycle` 27 files/247 tests 全绿；`npx tsc -b` exit 0。
- **API 面**：`issue_lifecycle_returns_plan_group_projection_with_three_valued_overall`（逐字段断言）。
- **定向回归**：plan_group_projection 13/13、web::handlers::lifecycle 9/9、issue_delivery 13/13（与 T4 兄弟的覆盖关系改造并存）、group_split 11/11、issue_lifecycle 2/2；clippy 本 Task 文件 0 warning；fmt-check 本 Task 文件通过。
- **同批交付红线自查**：WP2+WP3 一体交付单元成立——此前无任何单独宣布拆分可用（wp2-split-report §十原文）；T6 终检时点两 WP 均已入库，无中间态。

## 过程事件登记

1. 与 C2T4 并行碰撞两次：(a) 其在途编辑（issue_delivery.rs 缺 import+multi_target_delivery.rs 缺 Uuid）短暂打断主树编译——IRC 通知后其修复；(b) wp2-split-report §十一已登记本 Task 在途 mod.rs 暂态破坏（已修复，最终态 13/13 绿）。
2. 本 Task 编辑事故（均已修复并验证）：mod.rs 两次相邻行误删（plan_amendment_context/inputs::*）→编译红定位后恢复；lifecycle_tests.inc.rs 误跑 rustfmt 全文件反缩进→git checkout 还原后按 include 缩进重加测试；TS import 误覆盖 ProductIssue→tsc -b 捕获后恢复。
3. `prepare_work_item_plan` 响应含投影（新建 plan 恒 NotStarted 空条目）——DTO 形态与 issue_lifecycle 统一。

## 遗留与边界

- 浏览器级走查归 T6 关闸统一冒烟（组件面断言已覆盖计划 Step 3 四要素：per-target 状态/终态/partial failure 可见性/无操作入口）。
- 与 T4 面不相交确认：未动 issue_delivery.rs/evidence_*/git_operation/advance.rs/group_initialization.rs（T5 独占面）。

## Fix round 1（k3 审 1×P2，Main 转交）

- **缺陷**：`target_started=!(Created&&PrepareContext)` 把 pre-start abort 落盘形态 `(Aborted, PrepareContext)`（Created→Aborted 为 attempt.rs:783 白名单转换、abort 不改 stage）误判「已达 provider 启动」→整体误报 `partial`（spec 要求=未启）。
- **修法**：排除 pre-start abort——`never_reached_provider = stage==PrepareContext && status∈{Created,Aborted}`；启动后 abort（stage 已离开 PrepareContext）仍算已达。模块文档同步改写。
- **TDD**：`not_started_when_pre_start_abort_never_reached_provider` 修前红（`left: Partial, right: NotStarted`——与 k3 诊断吻合）+对偶 `partial_when_pre_start_abort_alongside_real_start`（双向钉死）；修后全绿。
- **复跑**：plan_group_projection 15/15、web::handlers::lifecycle 9/9、split_advance 8/8、前端 Panel+Drawer 13/13（前端零改动）；rustfmt 幂等+clippy 0 warning。
- **commit**：本 fix 提交（`fix(coding-ws): WP3 fix round 1 (k3 P2) — pre-start abort excluded from provider-started judgment (multi-repo-group-coding)`，紧随 `093fa312`）

## Fix round 2（k3 复审 1×P1，Main 转交）

- **缺陷**：`advance_to_next_group_unit`（coding_workspace_engine/group.rs:315）在多 unit group attempt 间推进时把 stage 回退 PrepareContext，其后 abort 落盘 (Aborted, PrepareContext) 与 pre-start abort 同形态但已执行过 provider（role_runs 在案）——fix round 1 形态判据误判未启→overall 误翻 NotStarted。
- **修法**：判据改取 durable provider 执行证据（新私有 `attempt_reached_provider`）：stage 离开 PrepareContext→已达；(Created, PrepareContext)→未达；其余 PrepareContext 停留态（经 admission）→已达；(Aborted, PrepareContext) 以 `list_role_runs` 非空或 `head_commit` 在场消歧（执行铁证在场=已达）。只读派生面保全。
- **TDD**：`partial_when_stage_reset_abort_has_provider_execution_evidence`（role_runs 通道）+`partial_when_stage_reset_abort_has_head_commit_evidence`（head_commit 通道）双红（`left: NotStarted, right: Partial`——与 k3 诊断吻合）→绿；三用例对照①无证据 pre-start abort→NotStarted（round1 测试保持）②有证据→Partial（新增）③未动→NotStarted（既有保持）。
- **复跑**：plan_group_projection 17/17、split_advance 8/8、web::handlers::lifecycle 9/9、role_run 族 13/13（新消费 list_role_runs 零回归）；前端零改动；rustfmt 幂等+clippy 0 warning（本文件）。
