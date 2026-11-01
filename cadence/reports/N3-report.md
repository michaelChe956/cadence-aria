# N3 报告：plan 一键启动 POST/GET 与单发首启编排（REQ-CS-01/02/06/08）

- Change：`openspec/changes/fix-multi-repo-group-coding-start/`（Task 2.1，REQ-CS-01/02/06/08）
- Plan：`cadence/plans/2026-10-13_实施计划_多仓分组coding启动_v1.0.md` §4
- Worktree：`.worktrees/feat-b-0808-add-monorepo`（分支 feat-b-0808-add-monorepo）
- 状态：DONE（红→绿→定向回归全绿；独立 commit 见本条 `fix: N3 plan 一键启动命令+真跑接线+聚合 DTO`）

## 交付（两新建 + 四窄改）

- **Create `src/web/handlers/coding/plan_start.rs`**：
  - `post_plan_coding_start(State, Path<(project,issue,plan)>, Json<PlanCodingStartRequest>)
    -> ApiResult<Json<PlanCodingStartResultDto>>`：顺序固定——id 校验 →
    plan scope 删除准入（`EntityOperationStore::with_live_write`，争用/墓碑立即
    `entity_deleting`）→ confirmed 检查 → `CodingPlanRunStore::prepare`（revision/graph
    冻结、幂等）→ 现 Manual `advance_plan`（`AdvanceInput.command_id` 用 prepare 返回
    的 `run.command_id`，第二 command 命中时不是新的 request.command_id）→
    `advance_target_bindings` 提取 per-target Ready 集（Rejected/未 Ready/空集合
    fail-closed，无 legacy fallback）→ 全图 ready 稳定选一（`select_first_ready_unit`：
    Kahn 全图拓扑位 → logical WI ID → target UUID，不硬编码 WI-001；已有冻结
    selection 直接复用不重选）→ `bind_ready`/`mark_start_requested` → 现 Manual
    `start_coding_once`（用冻结 `selection.start_command_id`，deterministic 派生唯一
    入口）→ `record_start_outcome`（真实回执冻结）。已有 run
    Started/AlreadyStarted/NeedsHuman 按事实返回不再 start；无 ready 时
    `selection=None` durable 绑定全集保持 ready_bound 等待零首启（WaitingBound）。
    进程内与旧 group 创建面同一 `work_item_group:` 串行键；不持 plan run 锁 await。
  - `get_plan_coding_progress(State, Path) -> ApiResult<Json<PlanCodingProgressDto>>`：
    同一删除准入/confirmed 检查；未创建 run 时只读 confirmed authoritative plan +
    已有 Ready 集（`get_attempt_for_work_item_group` per authoritative target），
    仍返回 bound_plan_revision_id 供 N6 互证，不持久化/不启动。
  - 共用聚合 `build_plan_coding_progress`：run 冻结集优先（无 run 按已有 Ready 集）
    逐 target 载 attempt/units；`execution_state` 值域
    `initialized|running|waiting_dependency|waiting_slot|paused|completed`——provider
    事实（durable start_claim + active 指针 + unit Running）与初始化桶首 unit
    Running（物化事实）严格分开（备忘①）；`waiting_reason` 值域
    `dependency_not_ready|execution_slot_occupied|plan_coding_continuation_not_enabled`
    （依赖优先于暂停文案，依赖未满足不被暂停掩盖）；`lease_state` 沿 repo worktree
    lock 事实（无记录/无 owner=absent、owner=本绑定 attempt=held、他人=contested），
    不造 provider_running 猜测；primary 仅打开入口（selection.attempt_id，否则
    确定性首绑定），`plan_complete` 仅在全部 authoritative units Completed。
- **Create/Test `src/web/handlers/coding/plan_start_tests.rs`**（由 coding.rs 现
  cfg(test) 模式接入）：夹具同 N1/N2 路线（PlanRepair 种子 + 四真实 logical 仓 +
  per-unit target 草稿 + 新发布 wi_service/wi_panel 构成 plan_revision_0002 五
  WI/四 target：busi=wi_core、web=wi_registration（依赖 wi_core 跨桶）、
  gateway=wi_unrelated、frontend={wi_service, wi_panel}，wi_panel 直接依赖
  {wi_unrelated, wi_service}），经 web 层 Manual advance 到 durable Ready。
- **Modify `src/web/handlers/coding.rs`**：挂载 `mod plan_start` + re-export +
  cfg(test) 测试模块（另含 rustfmt 对既有未格式化段落的归一，无语义变化）。
- **Modify `src/web/handlers/mod.rs`**：`pub use coding::{…, get_plan_coding_progress,
  post_plan_coding_start}`（re-export 闭包，本包唯一 owner）。
- **Modify `src/web/app.rs`**：新路由
  `/api/projects/{project_id}/issues/{issue_id}/work-item-plans/{plan_id}/coding-start`
  （GET+POST 同地址）。
- **Modify `src/web/types.rs`**：`PlanCodingStartRequest`、`PlanCodingTargetDto`、
  `PlanCodingUnitProgressDto`、`PlanCodingProgressDto`、`PlanCodingStartResultDto`
  （定义权在此，N6 仅读；attempt_status/unit_status 直用产品枚举，start_outcome
  内嵌 N2 tagged `CodingPlanStartOutcome`）。

## 证据（cargo test --locked --lib，红→绿→回归）

| 命令 | 红（stub 阶段） | 绿（rustfmt 后复验同） |
|---|---|---|
| `plan_coding_n3_` | 5/5 失败且可归因业务缺失：one-click/replay/needs_human 期望 Started/回执而得 `plan_coding_start_not_implemented`；GET 聚合 500≠200；drift 期望 `coding_plan_run_revision` 得 not_implemented | **5 passed** |
| `plan_coding_n1_ plan_coding_n2_ group_split_target coding_provider_snapshot advance_tests` | — | **27 passed, 0 failed** |

### 修复轮（Main 审查 1×P2×2+1×P3，commit `fix: N3 聚合失效投影与clippy门禁`）

| 命令 | 红 | 绿 |
|---|---|---|
| `plan_coding_n3_claimed_barrier` / `plan_coding_n3_deleted_member` | P3：Claimed 相位误投 `running`（期望 initialized）；P2-1：成员删除后 GET 炸 `coding_attempt_not_found`（期望 200+失效投影） | 并入族后 **7 passed** |
| `plan_coding_n3_ plan_coding_n1_ plan_coding_n2_ group_split_target coding_provider_snapshot advance_tests` | — | **34 passed, 0 failed** |
| `cargo clippy --locked --all-targets -- -D warnings` | plan_start.rs:593 多余 `mut` 一处 | **plan_start 零报**（其余报错均在其他波次文件，非 N3 面） |

- P2-1：聚合载入冻结成员时 `NotFound{kind:"coding_attempt"}` 不再上抛——该
  target 投影失效终态（`attempt_status=aborted`、worktree None、冻结 binding
  保留），已删成员桶 units 不再投影（不造 unit 猜测），其余成员事实不受影响；
  身份错配/IO/JSON 仍 fail-closed。run Invalidated 时整体失效文案：未完成
  units `execution_state=invalidated`（值域 additive 扩展，types.rs 已同步注释），
  完成事实仍投影 completed，`waiting_reason=None`。
- P2-2：`map_or(true,…)`→`is_none_or(…)`（clippy 门禁归一）。
- P3：`provider_started` 增判 claim 相位 ≥ ProviderMayHaveStarted
  （ProviderMayHaveStarted|NeedsHuman；Claimed/RunnerRegistered 回落
  initialized），对齐 REQ-CS-04「已接受首启 ≠ provider 实际进度」口径。
  新增测试 `plan_coding_n3_claimed_barrier_phase_is_not_provider_running`、
  `plan_coding_n3_deleted_member_aggregates_invalidated_not_500`（后者含
  N7 接线前窗口 run-active 与 N2 invalidate 后 Invalidated 两段断言）。

新增测试（全部真实 Git/materialize 链，无 mock provider 冒充验收）：

- `plan_coding_n3_one_click_binds_all_and_starts_selected_once`：五 WI/四 target
  expected revision 匹配 → Accepted；run 4 bindings、selection=wi_core/busi、
  `start_command_id`==deterministic 派生；busi claim 恰一次且 phase=
  ProviderMayHaveStarted、runner_count 恰 1；未 selected 三 target 零 claim 零
  runner 保持 Created；frontend 桶两 units；聚合五 units：wi_core=running、
  wi_registration=[wi_core]、wi_service=[wi_unrelated]、wi_panel=[wi_service,
  wi_unrelated] 全直依赖集 waiting_dependency；plan_complete=false。
- `plan_coding_n3_replay_and_restart_keep_same_start_command`：同 command 重放
  Replayed（run created_at/command/selection 不变）；新进程+第二 command 返回
  原 run（result.command_id=原 command）；claim 恒为冻结首启 command 且单发；
  重启进程零新 runner。
- `plan_coding_n3_revision_deleting_and_needs_human_fail_closed`：异 revision
  拒绝（details.kind=coding_plan_run_revision）且不落 run 文件；plan 删除 lease
  持有期间 POST/GET 均 `entity_deleting`；既有 NeedsHuman run 按事实返回
  （零 claim/零 runner/不漂移），新 command 重试不隐式清除。
- `plan_coding_n3_get_aggregation_primary_is_not_completion`：首次 GET 不写 run
  且返 bound revision（phase/mode/command 全 None）；durable 事实驱动
  （advance→bind→claim barrier→Started→wi_core Completed→run Paused）后 GET：
  phase=paused、四 target lease=held+worktree Some；wi_core=completed、
  wi_registration/unrelated=paused+plan_coding_continuation_not_enabled、
  wi_service/wi_panel=waiting_dependency（完整直接依赖集）；plan_complete=false、
  primary=selected attempt。
- `plan_coding_n3_old_post_shape_unchanged`：HTTP 层 build_web_router——旧
  `/coding-attempts` POST 对多 target 仍 `repository_routing_ambiguous`（409 单
  ApiError 对象，原准入零变化）；新 `/coding-start` GET 200 返回新 DTO 与旧路由
  共存。

## 边界与移交

- 未写 advance_plan/start_coding_once/advance_store/worktree 本体（纯调用）；
  N2 store 只消费（prepare/bind_ready/mark_start_requested/record_start_outcome/
  get/mark_paused 只读面未动）。旧 POST/WS shape 零修改。
- **N6 消费**：POST `/work-item-plans/{plan_id}/coding-start`
  `PlanCodingStartRequest{command_id, expected_plan_revision_id}`；GET 同地址
  `PlanCodingProgressDto`；result 的 `operation_state` 沿
  `accepted|replayed|needs_human|rejected`（等待绑定也 Accepted，NeedsHuman 按事实）。
  bound revision 语义：GET 未建 run 时返回当前 authoritative revision。
- 与 N5 无文件交集（其 runner.rs/group_completion.rs 本包零触碰）；与 N7 交集仅
  `web/handlers/mod.rs` 一行 re-export（无段并写）。
- rustfmt 归一了 coding.rs 内两段既有未格式化内容（其他波次未提交段落，非语义），
  已在此登记避免误归属。
- **已知基线红（非 N3 造成）**：跨族批次中 `advance_plan::tests::*`（6）与
  `coding_start::tests::*`（9）共 15 个失败，全部 panic 于 wiga fixture 编译链
  `provenance boundary must leave durable compile reservation`——由确定性结构治理/
  级联删除波次未提交改动（`lifecycle_store/workspace_single_candidate.rs` ~1891 行、
  `workspace_engine/compile/finalizer.rs`）引入；已在无 N3 的临时 worktree
  （HEAD=2b6417c9 + 除 N3 六文件外的全部未提交 .rs）复现同失败，N3 六文件不在该
  调用图。归 Main 登记至对应 owner。
- 发布/真人启动门禁维持：N3 handler 落地不代表可提前开放真人启动（N5/N7 未过
  禁真跑）；真 provider 验收属 Main 统一窗口（plan §9.2）。
