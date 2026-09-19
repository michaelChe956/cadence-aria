# WP3 聚合报告：plan 级 group 聚合只读视图/三值终态（T3 / REQ-MTG-04，同批交付）

- **change**: `openspec/changes/multi-repo-group-coding/`
- **计划**: `cadence/plans/2026-09-19_计划文档_阶段4-C2_多仓group编码_v1.0.md`（v1.1）Task 3
- **状态**: **全量交付**——数据面（`plan_group_projection.rs`）+API 组装（issue_lifecycle additive）+前端聚合区（panel+抽屉接线）+三值红绿+partial failure+禁动作断言。

## 一、已交付清单

### 数据面（`src/product/coding_attempt_store/plan_group_projection.rs`，新增）

| 项 | 内容 |
|---|---|
| `PlanGroupOverall{AllDelivered, Partial, NotStarted}` | 三值终态（DTO 序列化 `"all_delivered"/"partial"/"not_started"`——**不复用 issue 级 `"none"`**，k3 §2.3 口径差异） |
| `PlanTargetEntry` | `{target_repository_id, repository_name, attempt_id, attempt_status, stage, branch_name, head_commit, push_status, review_request_id, blocked_reason}`（OQ3 定案形态；attempt 派生字段全 Option） |
| `compute_plan_group_projection(project_id, issue_id, plan_id)` | 只读派生：`list_attempts_for_work_item_group` → 按 `attempt_target_bucket` 分桶（**D2.1 A4：无快照 attempt 不入任何桶**）→ 每 target 取最新 attempt（`(attempt_no, id)` 升序末元素，与 `compute_issue_delivery_summary` 口径一致）→ 最新 ReviewRequest push_status |
| 三值判据（约束 9 定案） | **未启**（优先于部分）= 无 target-attempt，或全部 target-attempt 均满足 `status==Created && stage==PrepareContext`（「已达 provider 启动」可测判据=离开初始二元组，`StartCoding` 唯一入口）；**全部交付**=每 target 最新 attempt `Completed` 且最新 ReviewRequest `Pushed`（issue_delivery.rs:110-113 对齐口径）；**部分**=其余一切 |
| `blocked_reason` 派生（只呈现不判定，OQ3） | `manual_recovery_reason` 优先 → 最新 ReviewRequest `push_error` → 失败态 status 文本（`Failed`/`Aborted`/`AmendmentApplyFailed`，snake_case 与 DTO 文本一致） |
| `resolve_plan_target_repository_name` | 展示名解析同款 issue_delivery `resolve_repository_name`（:132-152）：logical id 经 `resolve_logical_repository_strict` 取 checkout 路径末段目录名，无末段回落 id 字符串 |
| 只读保证 | 无任何写入路径；先读 issue 确认存在（与 `compute_issue_delivery_summary` 同款防空聚合） |
| mod.rs 挂载 | `mod plan_group_projection;` + `pub use {PlanGroupOverall, PlanGroupProjection, PlanTargetEntry}` |

### API 组装（additive）

| 项 | 内容 |
|---|---|
| `IssueWorkItemPlanDetailDto.group_projection`（web/types.rs） | additive `Option<PlanGroupProjectionDto>`（`#[serde(default, skip_serializing_if)]`——旧响应缺省反序列化 None 兼容；新响应总是携带） |
| `PlanGroupProjectionDto` / `PlanTargetEntryDto`（web/types.rs） | DTO 形态（snake_case；`overall` 三值字符串） |
| `plan_group_projection_dto` / `plan_target_entry_dto`（web/handlers/dto.rs） | 映射：status/stage/push 文本复用既有 `coding_attempt_status_text`/`coding_execution_stage_text`/`push_status_text`；`target_repository_id` 取 UUID 字符串 |
| `issue_work_item_plan_detail_dto(plan, group_projection)`（签名扩展） | 两调用点随改：`issue_lifecycle`（:79 起 per plan 计算）+`prepare_work_item_plan`（响应同款投影——新建 plan 恒 NotStarted 空条目，与 issue_lifecycle 同形） |
| `issue_lifecycle` 组装 | plan DTO 映射处 per plan 调用 `compute_plan_group_projection`（additive 进 `IssueLifecycleResponse.work_item_plans[i].group_projection`——响应顶层零新键） |

### 前端（web/src）

| 项 | 内容 |
|---|---|
| `api/types/work-item-plan.ts` | `PlanTargetEntryDto`/`PlanGroupOverallDto`/`PlanGroupProjectionDto` + `IssueWorkItemPlanDetailDto.group_projection?`（additive 可选——旧响应 undefined 兼容） |
| `components/lifecycle/PlanGroupProjectionPanel.tsx`（新增） | 三值终态徽标（`data-status=all_delivered/partial/not_started`）+per-target 行（仓展示名/attempt 状态·stage/推送/分支+head 前 7 位）+partial failure 未满足明细（`plan-target-unmet-reason`：blocked_reason 优先，缺席按推送/完成事实派生）+blocked_reason 独立行；**聚合区零动作元素**（无按钮/无 role=button/无链接——决策/启动/中止走各 attempt 既有入口） |
| `LifecycleCardDrawer.tsx` | `DrawerEntity.groupProjection?`（additive）+`work_item_group` 分支渲染 panel（投影在场才渲染——缺省不渲染，additive 兼容） |
| `IssueLifecycleWorkbenchParts.tsx` | `toDrawerEntity` work_item_group 分支透传 `card.raw.group_projection ?? null`；`normalizeLifecycleResponse` 的 `isIssueWorkItemPlanDetail` 增可选 `group_projection` 形状校验（在场校验 plan_id/overall 三值枚举/entries，缺席不拒绝——旧响应兼容） |

## 二、TDD 红绿证据

### 数据面（`plan_group_projection.rs` tests，12 项）

| 计划 Step1 项 | 测试 |
|---|---|
| #1 全部交付 | `all_delivered_when_every_target_latest_attempt_completed_and_pushed`（两 target Completed+Pushed+ReviewRequest 在案→AllDelivered；条目字段逐项断言含 review_request_id/blocked_reason=None） |
| #2 partial failure 三负向 | `partial_failure_unpushed_completed_attempt_is_partial_with_explicit_state`（Completed+NotPushed→Partial，push_status 显式）；`partial_failure_push_error_surfaces_in_blocked_reason`（push_error→blocked_reason）；`partial_failure_failed_and_manual_recovery_statuses_surface_blocked_reason`（Failed→"failed" 文本回落+AwaitingManualRecovery→manual_recovery_reason 优先）；`partial_failure_missing_review_request_is_partial`（无 ReviewRequest→push_status/review_request_id=None 显式）——全部断言 overall==Partial（不伪装全局成功） |
| #3 未启三态 | `not_started_when_no_target_attempts`（空→NotStarted 空条目）；`not_started_when_all_attempts_stay_in_initial_binary_group`（全部 (Created, PrepareContext)→NotStarted **即使零交付也不落 Partial**，条目仍显式呈现）；`partial_when_some_targets_started_and_others_not`（部分启动+部分未启→Partial） |
| #4 最新 attempt 口径 | `latest_attempt_per_target_wins_the_verdict`（前失败(attempt_no=1)后完成(2)→AllDelivered 取最新；反向 前完成后失败(3)→Partial 取最新+blocked_reason="failed"） |
| #5 只读派生 | `projection_is_read_only_derivation_no_durable_writes`（调用前后 `.aria` 全树 inventory（路径+mtime 纳秒+长度）零 delta——无第二状态机） |
| #6 D2.1-A4 | `snapshotless_attempt_excluded_from_per_target_projection`（无快照存量 attempt 不进 per-target 投影——不猜测归属） |
| #7 单 target 旁路 | `single_target_projection_matches_issue_delivery_verdict`（单条目；AllDelivered ⟺ issue 级 AllPushed 正向一致+Failed 双 Partial 负向一致） |

**红态证据（mutation 反证）**：实现先行的情形下以变异测试反证测试防御力——移除「未启优先于部分」分支（`else if !any_target_started`→`else if false`）后 `not_started_when_all_attempts_stay_in_initial_binary_group` 立即 FAILED（已还原复绿）。

**只读断言口径说明**：首跑捕获到 strict 解析路径触发的 store 级一次性 legacy logical-codebase 懒迁移（`.legacy-logical-codebase-migration.lock`+manifest/成员复制——`LogicalCodebaseStore::migrate_legacy`，issue 级 delivery 面同款既有幂等行为，非聚合派生写入）；测试在夹具中预热迁移后在稳态上断言零 delta——断言对象是「聚合派生不写任何文件」。

### API 面（`web/handlers/lifecycle_tests.inc.rs`）

- `issue_lifecycle_returns_plan_group_projection_with_three_valued_overall`：双 target plan（alpha Completed+Pushed、beta (Created, PrepareContext)）→ `work_item_plans[0].group_projection` 形状逐字段断言（plan_id/overall="partial"/entries=2/attempt_status/stage/branch/head_commit/review_request_id/blocked_reason）。

### 前端（Vitest）

- `PlanGroupProjectionPanel.test.tsx` 6 项：all_delivered 徽标+双已交付行；partial（部分交付徽标+失败行 data-status=failed+未满足原因 blocked_reason 显式+已交付行无未满足行——不伪装全局成功断言 `queryByText("已全部交付")` 为空）；per-target 事实（分支/head 前 7 位/stage 中文标签/推送状态）；not_started（徽标+created/prepare_context 行+空条目 hint）；**聚合面零动作元素**（`button`/`[role="button"]`/`a[href]` 全零）。
- `LifecycleCardDrawer.test.tsx` 增 1 项：work_item_group 投影在场渲染 panel、缺省（null）不渲染——additive 兼容双向。

## 三、REQ-MTG-04 四 scenario 逐条对应

| Scenario | 对应 |
|---|---|
| 聚合视图呈现 per-target 与终态 | `compute_plan_group_projection`（数据面）+`issue_lifecycle_returns_plan_group_projection_...`（API）+`PlanGroupProjectionPanel`（前端 per-target 行/徽标）——数据均派生自 target-attempt durable 事实 |
| partial failure 显式呈现 | 三负向数据面测试+`plan-target-unmet-reason`/blocked_reason 前端显式+「不伪装全局成功」断言（数据面 overall==Partial、前端徽标非全部交付） |
| 聚合视图只读派生 | `projection_is_read_only_derivation_no_durable_writes`（零 durable 写入——无第二状态机）+前端零动作元素断言（经该视图不能触发任何决策或执行动作） |
| 同批交付 | 见 §四 |

## 四、同批交付红线自查

- 决议+oracle 双锚：聚合只读视图（含前端呈现）与拆分能力**同批验收**——本 Task（WP3）提交与 T2（WP2，`4830bf03`…`b154839f`+fix round `5c50ae64`）构成一体交付单元，**此前各批均未单独宣布拆分能力可用**（wp2-split-report §十明确「聚合视图（WP3）落地前不单独宣布拆分能力可用」）。
- 本 commit 落地后 WP2+WP3 同批齐备；T6 关闸终检「无拆分先行聚合缺席中间态」时点为两 WP 均已入库——不存在中间态。
- 无第二状态机：零新增持久化聚合记录（§二 #5 测试钉死）；每次读取从 attempt/ReviewRequest durable 事实现算。
- 聚合面无操作入口：前端面板零按钮/零命令发送（Vitest 断言）；决策/启动/中止走各 attempt 既有入口（drawer footer 既有按钮非聚合区）。

## 五、单 target 零变化旁路

- 单 target plan 投影=单条目，判定口径与 issue 级 delivery 一致（正向 AllDelivered⟺AllPushed、负向双 Partial——`single_target_projection_matches_issue_delivery_verdict`）。
- API additive：`group_projection` 缺省反序列化 None（旧响应兼容）；响应其他字段/wire 零变化（`issue_lifecycle`/`prepare_work_item_plan` 既有测试族全绿）。
- 前端 additive：投影缺省不渲染 panel（drawer 双向测试）；既有 247 项 lifecycle 组件测试零回归。

## 六、验证记录（主树定向+定向回归）

- `cargo test --locked plan_group_projection`：**13 passed / 0 failed**（12 数据面+1 API 面）。
- `cargo test --locked web::handlers::lifecycle`：9/9；`cargo test --locked issue_delivery`：13/13（含 WP4 兄弟对 `compute_issue_delivery_summary` 的覆盖关系改造——两 WP 并存绿）；`cargo test --locked group_split`：11/11；`cargo test --locked issue_lifecycle`：2/2。
- clippy `--lib --tests`：本 Task 文件 0 warning 0 error（仅 pre-existing `contract_autorepair.rs` 2 条，WP2 §九已登记）。
- `cargo fmt --check`：本 Task 全部 Rust 文件通过（主树现存 diff 均为 WP4 兄弟在途文件）。
- 前端：`npx tsc -b` exit 0；`npx vitest run src/components/lifecycle`：**27 files / 247 tests 全绿**（含新增 7 项）。
- 浏览器级走查按 C2 计划归 T6 关闸统一冒烟（本 Task 组件面断言已覆盖 per-target/终态/partial failure/禁动作四要素——与计划 Step 3 验收口径一致）。

## 七、与其他 Task 的面边界（并行无碰撞确认）

- 与 T4（核查型）：本 Task 未动 `issue_delivery.rs`/`git_operation.rs`/`evidence_*`/`advance.rs`/`group_initialization.rs`（T5 审计接线独占面）；T4 对 `issue_delivery.rs` 的覆盖关系改造与本 Task 的 `plan_group_projection.rs` 并存（判定一致性由 T4 Step 2 以本函数为锚复验）。
- 共享文件 `src/web/handlers/lifecycle.rs`：本 Task 改 plan DTO 组装区（:73 起+prepare 响应），与 C3 计划的 legacy fallback 删除区（:647-708 前版本）按落地序 ③→② 串行无冲突（锚点均在删除区前）。

## 八、Fix round 1（k3 审 P2：pre-start abort 误判「已达 provider 启动」，2026-09-19）

| 项 | 内容 |
|---|---|
| 问题 | `target_started` 判据 `!(status==Created && stage==PrepareContext)` 把 **pre-start abort** 形态 `(Aborted, PrepareContext)` 误判为「已达 provider 启动」——`Created→Aborted` 是合法状态转换（WS 白名单在 PrepareContext 放行 AbortAttempt，attempt.rs:783），且 abort 路径不改 stage，落盘即 `(Aborted, PrepareContext)`。后果：两 target 其一 pre-start abort、另一未动时 overall 误报 `partial`，而 spec（约束 9+REQ-MTG-04）要求该形态=未启。 |
| 修法 | `target_started` 排除 pre-start abort：`never_reached_provider = stage==PrepareContext && status∈{Created, Aborted}`——`stage==PrepareContext` 且状态仍属「未进入执行」族（Created 未触碰/Aborted 启动前中止）均不算离开初始二元组；启动后 abort（stage 已离开 PrepareContext）仍算已达（该 attempt 确实经 StartCoding 进入过执行）。模块文档同步改写。 |
| blocked_reason 语义 | `(Aborted, PrepareContext)` 条目 `blocked_reason="aborted"`（失败态文本回落，既有行为）——abort 事实仍显式呈现，仅不参与启动判定。 |

**TDD（红→绿）**：

- 新增 `not_started_when_pre_start_abort_never_reached_provider`——修前红（`left: Partial, right: NotStarted`，与 k3 诊断逐字吻合）+条目断言（Aborted/PrepareContext/blocked_reason="aborted" 显式）；修后绿。
- 新增对偶 `partial_when_pre_start_abort_alongside_real_start`——pre-start abort+另一 target Running/Coding → Partial 语义不变（修前修后均绿，防修过窄/过宽双向钉死）。

**定向复跑**：`plan_group_projection` 全族 **15/15**（13 原有+2 新增）；`web::handlers::lifecycle` 9/9；`split_advance` 8/8（无自动编排测试直读 attempt 状态，与投影判据正交）；前端 Panel+Drawer 13/13（前端零改动——not_started+aborted 行经 `plan-target-blocked-reason` 既有分支显式呈现）；`rustfmt` 幂等+clippy `--lib --tests` 本文件 0 warning。
