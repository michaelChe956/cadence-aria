# feat-b-0715 最终修复报告

## 交付摘要

- 分支：`feat-b-0715`。
- 提交策略：单个原子 fix commit，不 amend，不 push。
- 范围：Coding Attempt 创建 TOCTOU、前端 Plan Repair timeline 乱序、Child Workspace 确认 Amendment 后未进入真实 Coding WS 发布/应用/runner 恢复链路，以及相关失败重试。
- 明确未运行：E2E、Playwright、browser、真实网络 Codex/Claude CLI。

## 修复内容

### 1. Coding Attempt 创建 TOCTOU

- 单 Work Item Attempt 创建改为在跨 Store、跨进程共享的文件锁内完成 active-attempt 检查、attempt number 分配、Attempt 写入和 provider config 写入。
- 并发失败方返回结构化 `ProductStoreError::Conflict`，Web API 保持稳定的 `active_coding_attempt_exists` 错误码。
- provider config 写入失败时回滚已写入的 Attempt，避免留下半成品记录。
- 新增两个独立 `CodingAttemptStore` 实例并发创建的确定性回归测试，证明最多只有一个成功记录与一份 provider config。

### 2. Plan Repair timeline 乱序保护

- 前端聚合状态新增 authoritative snapshot `timelineWatermark`。
- 拒绝水位之前或等于水位的未知迟到 create。
- 已存在节点拒绝更旧覆盖；terminal 节点拒绝回退到 non-terminal 或其他状态。
- snapshot 合并保留水位之后的合法 live 节点，同时避免旧 snapshot 重新引入已淘汰节点。
- Coding Plan Repair aggregator 明确只接受 `plan_repair` relation；Story/Design amendment child timeline 保持隔离。

### 3. Child Confirm 到真实 Coding WS 的生产链路

- `WorkspaceEngine::confirm_and_publish_plan_amendment` 统一完成权威 Child snapshot 刷新、确认、candidate package 重建、review attestation 校验、Amendment 发布与 Child Published snapshot 持久化。
- `PlanRepairEngine::load_prepared_amendment` 从 durable candidate package 和权威 revision artifacts 重建 `PreparedPlanAmendment`，不依赖进程内临时对象。
- 新增 per-Attempt Coding socket registry：支持同 Attempt 多 socket token，选择最新未关闭 sender，并在 socket 退出时按 token 注销。
- Child handler 发布成功后取得 Attempt 级锁与 runner reservation，通过 Amendment 专用 reserved spawn 启动既有 Coding runner。
- runner 首步继续复用既有 `recover_plan_amendment`、跨进程 arbitration、application journal、stable event ID、socket-write ACK、ApplyFailed 与 retry 逻辑；Child handler 不复制 apply 实现。
- 重复 Confirm 通过 Attempt guard、reservation 与 runner registry 幂等收敛，不产生双 runner。

### 4. 部分提交与失败重试

- 首次 runner activation 因 Coding WS 尚未连接而失败时，Amendment 发布保持 durable；连接 Coding WS 后重复 Confirm 可恢复为单 runner、单 application journal。
- publication journal 已存在时，重复 Confirm 不再重新执行 awaiting-package base 校验；恢复会复用 journal 中首次持久化的 confirmation、`confirmed_at` 与 review attestation，继续完成同一 publication。
- 修复了 `JournalPlanPublished` 之后 active revision 已推进、request 尚未标记 Published 时重复 Confirm 报 `AmendmentConflict` 的真实部分提交缺陷。
- socket 已写出事件但 delivery mark 失败时，重复 Confirm 使用同一稳定 `event_id` 重发并收敛到 `Delivered`；application journal 与 runner 数量保持唯一。

## 关键 RED → GREEN

| 场景 | RED | GREEN |
| --- | --- | --- |
| Attempt TOCTOU | 两个 Store 可同时通过 active 检查 | 共享文件锁串行化，1 成功、1 typed conflict |
| Timeline 乱序 | 迟到 active/paused 可覆盖 completed/failed | watermark、terminal 单调与时间比较拒绝回退 |
| 真实 Child Confirm | Child WS 确认成功，但 Coding WS 3 秒内收不到 `plan_amendment_updated` | 真实双 WS 收到事件，binding/delivery/Child Completed/runner 均收敛 |
| Publication 部分提交 | `JournalPlanPublished` 失败后重复 Confirm 报 base `AmendmentConflict` | 识别既有 journal，复用首次 confirmation/attestation 后完成 Published |
| Runner activation 重试 | 未连接 Coding WS 时 activation 失败 | 后续连接并重复 Confirm，单 journal、单 runner |
| Delivery mark 重试 | socket write 成功后 mark 失败，Attempt 为 ApplyFailed、marker 为 Pending | 重复 Confirm 使用同一 event ID，恢复 Running/Delivered |

## 验证结果

### 定向验证

- `cargo test --locked --lib single_work_item_attempt_creation_is_serialized_across_store_instances -- --nocapture`：1 passed。
- `pnpm exec vitest --run src/state/plan-repair-session.test.ts src/hooks/useWorkspaceWs.plan-repair.test.tsx`：2 files、34 tests passed。
- `cargo test --locked --test it_web child_confirmation_publishes_applies_and_restarts_through_real_websockets -- --nocapture`：1 passed。
- `cargo test --locked --test it_web child_confirmation_retries_activation_after_coding_socket_connects -- --nocapture`：1 passed。
- `cargo test --locked --lib plan_repair_confirm_and_publish_recovers_partial_publication_with_same_attestation -- --nocapture`：1 passed。
- `cargo test --locked --lib repeated_confirmation_recovers_delivery_mark_failure_without_duplicate_application -- --nocapture`：1 passed。

### 最终门禁

- `cargo fmt --check`：PASS。
- `cargo check --locked`：PASS。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：PASS，0 warnings。
- `cargo test --locked`：PASS，exit 0；lib、integration 与 doc tests 均无失败。
- `cd web && pnpm tsc -b`：PASS。
- `cd web && pnpm test`：89 files、721 tests passed。
- `cd web && pnpm build`：PASS；仅保留 Vite 的既有大 chunk 提示。
- `git diff --check`：PASS。
- 35 个代码改动文件均不超过 800 行；最大文件为 `src/web/workspace_ws_handler/decisions/inbound.rs`，795 行。

## Minor：`src/web/test_controls` 生产边界决策

- 本轮未对 `PlanRepairFixtureRuntime` 做 feature-gate 或迁移。
- 原因：现有大量 integration tests 通过 `cadence_aria::web::test_controls` 使用该 fixture；integration test 编译库时不带 `cfg(test)`，当前 Cargo 也没有能由标准 `cargo test --locked` 自动启用的 self-feature。直接 gate 会破坏标准测试命令，迁移到 `tests/support` 则会形成大范围、高风险的测试基础设施重构。
- 新增能力只是 fixture 方法，不是 HTTP route。现有 `/api/test/*` 仍只在 `ARIA_E2E_TEST_CONTROLS=1` 时注册，默认生产 router 不暴露这些入口。
- 该边界作为已知 Minor 保留，后续如要彻底隔离，应单独设计 test-support crate/feature 与统一的 integration test 启动方式。

## 自审结论与边界

- Attempt 创建、Amendment publication/application/delivery 的权威身份与并发边界均 fail-closed。
- 重复 Confirm 可以恢复 publication、activation、apply/delivery 失败，不宣称客户端 exactly-once；delivery 语义仍是 stable event ID 的 socket-write-confirmed at-least-once。
- Story/Design amendment 未接入 Coding Plan Repair aggregator。
- 未新增生产测试路由，未运行被禁止的 E2E/浏览器/真实 provider 网络流程。
- worktree 保留；完成后仅创建单个原子提交，不 push。

## Round 2 Review 修复（2026-07-20）

### 1. Subgraph Replan durable recovery

- Candidate package 显式持久化 `subgraph_replan`，并将完整 readiness 纳入 package fingerprint、canonical reload、publication 与 review attestation 校验。
- `load_prepared_amendment` 从 durable candidate 恢复真实 `SubgraphReplanResult`，并与权威 base/next dependency graph revision 交叉校验。
- `subgraph_replan` 使用 required-Option 反序列化：字段必须存在，显式 `null` 仅允许非 Subgraph candidate；不接受旧 Schema 的缺字段默认兼容。
- topology fixture 拆分到 `src/web/test_controls/plan_repair/topology.rs`，避免原 fixture 文件超过 800 行。
- 覆盖 topology partial publication recovery 与 Child/Coding 双真实 WebSocket 确认、发布、应用、runner 恢复。

### 2. Issue worktree lease、并发 Attempt 与终态顺序

- `IssueSharedWorktree.current_lock_owner_id` 为 required-Option 严格字段；所有 Issue worktree RMW 均在跨 Store 文件锁内执行。
- acquire 返回 `IssueWorktreeLockLease`；成功 Attempt 将临时 lease owner 绑定为 `attempt.id`；transfer/release/complete 校验 owner。
- stale Work Item release 即使 owner 相同也返回 typed conflict；其他 Work Item 正持锁时，complete 必须匹配当前 owner 才能写完成历史。
- group Unit 推进在任何 Unit/run/Attempt 写入前校验 lease owner；owner conflict 下 Unit 与 Attempt 均保持原状态。
- Final Confirm、Abort、Failed、Delete 与自动完成路径在写 Attempt/Work Item 终态前统一预检 owner；wrong-owner focused 测试确认无终态副作用。
- 同 Work Item 双 HTTP 并发由默认关闭、无 HTTP route 的 `Notify` seam 确定性协调：winner 保留 lease，loser 返回稳定 `coding_attempt_active` 409，其他 Work Item 在 winner abort 前仍被 `issue_worktree_active` 阻止。
- 并发 HTTP 测试等待窗口由 1 秒提升到项目常用的 3 秒，协调本身仍由 Notify 事件驱动，不依赖 sleep 竞争。

### 3. Equal-version timeline snapshot

- `reconcileChildTimelineNodes` 对 snapshot 与 live 同 ID 节点复用 `shouldKeepTimelineNode`，terminal live 节点不会被 equal-version active/paused snapshot 回退。
- snapshot watermark 后的合法新 live node 继续保留。
- Story/Design amendment relation 的表驱动覆盖继续确认其不进入 Coding Plan Repair aggregator；本轮前端改动仅位于共享 Plan Repair 聚合层，未改变 Story/Design Workspace 自身链路。

### 4. 追加 RED → GREEN

| 场景 | RED | GREEN |
| --- | --- | --- |
| Candidate package 严格 Schema | 缺少 `subgraph_replan` 仍被 Serde 解为 `None` | required-Option 拒绝缺字段 |
| Issue worktree 严格 Schema | 缺少 `current_lock_owner_id` 仍被 Serde 解为 `None` | required-Option 拒绝缺字段 |
| stale Work Item release | lock 已转移后旧 Work Item release 静默成功 | 返回 `issue_worktree_lock_owner` conflict，lease 保持在新 Work Item |
| wrong-owner complete | winner 持有其他 Work Item lease 时 loser 可改写 `last_completed` | typed conflict，历史与 owner 均不变 |
| group Unit owner conflict | Unit、Attempt 已推进后 transfer 才失败 | 写前预检，Unit/Attempt/lease 零状态变化 |
| terminal owner conflict | Final Confirm/Abort/Failed 先写终态再报 owner conflict | 写前预检，Attempt 保持原状态 |

对应 focused GREEN：

- `plan_repair_candidate_package_rejects_missing_subgraph_readiness_field`
- `issue_shared_worktree_rejects_missing_lock_owner_field`
- `stale_work_item_release_is_rejected_even_for_current_owner`
- `wrong_owner_completion_does_not_mutate_worktree_history`
- `group_unit_owner_conflict_does_not_advance_unit_or_attempt`
- `final_confirm_owner_conflict_does_not_complete_attempt`
- `abort_owner_conflict_does_not_abort_attempt`
- `failure_owner_conflict_does_not_fail_attempt`
- `concurrent_same_work_item_loser_preserves_winner_issue_worktree_lease`
- `topology_plan_repair_recovers_partial_publication_from_durable_candidate`
- `topology_child_confirmation_applies_and_resumes_through_real_websockets`
- equal-version completed/failed timeline snapshot 回退用例。

### 5. Round 2 最终门禁与审计

- `cargo fmt --check`：PASS。
- `cargo check --locked`：PASS。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：PASS。
- `cargo test --locked`：PASS，exit 0；lib、integration、Web integration 与 doc-test 无失败；Web integration 270 passed、12 ignored，doc-test 1 passed。
- `cd web && pnpm tsc -b`：PASS。
- `cd web && pnpm test`：89 files、724 tests passed。
- `cd web && pnpm build`：PASS；仅既有 Vite 大 chunk 提示。
- `git diff --check`：PASS。
- 40 个改动/新增 Rust、TS 文件全部不超过 800 行；最大为 `tests/it_product/product_coding_workspace_engine/part_13.rs`，792 行。
- `src/web/app.rs` 无 diff；现有 `/api/test/*` 仍仅在 `test_controls_enabled()` 为真时注册，未新增默认生产可达路由。
- 明确未运行：E2E、Playwright、browser、真实 Provider/网络 CLI。

### 6. Round 2 提交

- Commit：`050e728`（`fix(plan-repair): close round two recovery races`）。
- 策略：新原子 fix commit，不 amend `1fba4dc`，不 push；worktree 保留。

## Final Review Important 修复（2026-07-21）

### 1. Group 高层完成入口 owner 预检

- `complete_group_unit_after_code_review` 在读取最新 Attempt 后、结构预检与任何 git/store 写入前校验 Issue Shared Worktree owner。
- Running 模式不再先创建 completion commit、保存 legacy/canonical handoff、完成 Unit Run 或应用 runtime handoff transition 后才由低层入口发现 owner conflict。
- CompletedRetry 模式不再绕过低层预检直接启动下一 Unit；owner conflict 下 Attempt、Unit、Unit Run、handoff、git HEAD/status 与 lease 快照全部保持不变。

### 2. Provider failure 高层入口 owner 预检

- `fail_provider_stream` 在所有 stage 分支前统一校验 owner。
- 非 Coding/CodeReview 的 terminal failure 不再先写 Attempt=Failed 与 timeline=Failed，再由 `handle_attempt_failed` 报 owner conflict。
- 高层回归测试确认 owner conflict 下 Attempt、timeline、Work Item、lease 与 WebSocket event channel 均无副作用；既有 CodeReview/Coding blocked recovery 行为保持不变。

### 3. Single Attempt persist→bind 崩溃恢复

- 新增默认关闭、一次性消费且无 HTTP route 的 `TestControls` seam，确定性中断 Attempt 持久化之后、Issue Worktree lease bind 之前的窗口。
- 新进程/新 router 遇到同 Work Item 的唯一 active Attempt 时，在返回 `coding_attempt_active` 409 前，通过 Issue Worktree 文件锁幂等绑定 owner：
  - owner 已是该 Attempt ID 时直接成功；
  - owner 仅在具有 `issue_worktree_lease_` 系统 pending lease 前缀时允许绑定；
  - Work Item 不匹配、其他 owner 或多个 active Attempt 均 fail-closed。
- 显式枚举 active Attempt；多个 active 时返回 `coding_attempt_ambiguous`，不会任选一条绑定临时 lease。
- 重启回归确认其他 Work Item 不能抢占 orphan lease、恢复后不存在 active Attempt + temporary owner 组合，并可通过真实 Abort 正常释放 lease。
- 选择恢复绑定而非 delete+release 回滚：Attempt 唯一性已由跨 Store 文件锁保证，而 lease bind 是单个 Issue Worktree 文件内的原子 RMW；删除 Attempt 与释放 lease 横跨两个 Store，无法提供等价原子性。

### 4. 追加 RED → GREEN

| 场景 | RED | GREEN |
| --- | --- | --- |
| Group Running 高层入口 | owner conflict 前已改变 git HEAD、Attempt head、Unit Run、handoff 与 canonical transition 状态 | 高层入口首写前预检，完整状态与 lease 快照零变化 |
| Group CompletedRetry 高层入口 | 已完成 Unit 的 retry 直接启动下一 Unit，随后 transfer 才报 owner conflict | retry 在 advance 前被高层预检拒绝，下一 Unit/Run/Attempt 不推进 |
| Provider fail 高层入口 | Attempt 与 timeline 已写 Failed 后才返回 owner conflict | owner 预检先于所有 stage 分支写入，Attempt/timeline/Work Item/lease/event 零变化 |
| 非 pending owner bind | 任意非 `coding_attempt_` owner 都可被覆盖为 Attempt ID | 只接受同 Attempt owner 或 `issue_worktree_lease_` pending owner |
| persist→bind 重启 | 同 Work Item 重试返回 409，但 lease owner 仍是 temporary ID，Abort 无法释放 | 返回 409 前绑定唯一 active Attempt，Abort 后 active owner 全部清空 |
| 多 active 损坏数据 | `get_active_attempt` 任取一条并绑定 lease | 显式检测 ambiguity，lease 不变且 fail-closed |

新增/关键 focused GREEN：

- `group_completion_running_owner_conflict_is_zero_write_at_production_entry`
- `group_completion_retry_owner_conflict_is_zero_write_at_production_entry`
- `provider_failure_owner_conflict_is_zero_write_at_production_entry`
- `attempt_bind_rejects_non_pending_issue_worktree_owner`
- `retry_reconciles_attempt_persisted_before_lease_bind_after_restart`
- `retry_does_not_reconcile_ambiguous_active_attempts`
- Group authority/recovery 模块 20 passed。
- Provider failure recovery 模块 4 passed。
- 既有 owner conflict focused 4 passed。
- LifecycleStore 模块 28 passed。
- 既有 duplicate create 与 concurrent loser HTTP focused 均通过。

### 5. 最终门禁与边界

- `cargo fmt --check`：PASS。
- `cargo check --locked`：PASS。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：PASS，0 warnings。
- `cargo test --locked`：PASS，exit 0；lib 1206 passed，Web integration 272 passed、12 ignored，doc-test 1 passed，其余 integration targets 无失败。
- `cd web && pnpm tsc -b`：PASS。
- `git diff --check`：PASS。
- 本轮 10 个 Rust/测试文件均不超过 800 行；最大为 `src/web/handlers/coding.rs`，724 行。
- 未修改 `src/web/app.rs`，未新增 `/api/test/*` 路由；failpoint 仅能通过直接持有 `WebAppState.test_controls` 配置，默认关闭并一次性消费。
- Story Spec 与 Design Spec Workspace 不受影响：本轮只修改 Coding Attempt 创建、Coding Workspace Group/Provider failure 与 Issue Shared Worktree owner 链路；未改共享产物 workspace timeline/chat/artifact 链路。
- Work Item 影响仅限 Coding Attempt 对 Issue Shared Worktree lease 的校验、绑定、转移和释放，不改变 Work Item 规划、Story/Design 生成或审核流程。
- 明确未运行：E2E、Playwright、browser、真实 Provider、网络 CLI。
- 提交策略：代码、测试与本报告使用同一个新原子提交；不 amend `050e728`/`6daff5b`，不 push，worktree 保留。

## Round 3 最终修复（2026-07-21）

### 1. Group Attempt journal-first 初始化与重启恢复

- 新增 issue 级跨进程文件仲裁锁，并在任何 Attempt、provider config、plan binding 或 Unit 写入之前持久化初始化 journal。
- journal 固定 Attempt、provider config、plan revision、plan binding 与有序 Unit 身份，按 `Prepared`、`AttemptPersisted`、`WorktreeBound`、`PlanBindingSaved`、`UnitsMaterialized`、`Completed` 单调推进。
- 每个阶段使用严格相等校验和幂等 ensure；已有记录身份不一致时 fail-closed，不覆盖或删除权威状态。
- 明确不兼容历史无 journal Group Attempt：Attempt 已存在但 journal 缺失时拒绝恢复，避免从不完整身份推断并写入。
- 三个默认关闭、一次性 checkpoint 覆盖 Attempt persist 后、worktree bind 后和 partial units 后的新 Store/router 重启；半初始化失败不再执行跨 Store rollback。
- 显式删除 Group Attempt 时同步删除匹配 journal，避免合法重新创建被旧初始化身份阻塞。
- 权威 plan revision/provider 身份漂移重启返回 `coding_group_attempt_incomplete`，并验证整个 `.aria` 树、journal、Attempt、lease、binding 与 Unit 零写不变。

### 2. Runner retirement、Abort/Delete 串行与 ownerless 严格校验

- registry runner entry 保存 command sender 与 `watch` completion signal；`remove` 才标记完成并唤醒 Abort waiter。
- Attempt Abort 时先写入 retired tombstone、撤销 reservation，再发送 Abort 命令并等待真实 runner `remove`；同步 registry 锁不跨 `.await`。
- retired Attempt 拒绝 late reservation activation、runner insert 与新 reserve，关闭 Abort 之后旧启动路径重新激活 runner 的窗口。
- HTTP Abort/Delete 在 runner completion 后获取 Attempt mutation lease并重载 Attempt；WS Abort 复用已持有 lease并等待 runner completion，从而与 durable terminal mutation 串行。
- `handle_abort` 对已 Aborted Attempt 零写幂等，不再次改变 `updated_at`、`completed_at` 或 timeline。
- Issue Shared Worktree owner 校验仅接受精确 Work Item + Attempt owner；ownerless 记录不再被视为任意 runner 的合法 owner。
- dummy runner 测试在收到 Abort 后显式调用 registry `remove`，使测试与生产 completion 语义一致。

### 3. Abort 与旧 durable transition 竞态封闭

- 新增按 `.aria` root 与 transition 类型隔离、默认关闭且一次性消费的 test-only pause seam，触发点位于 owner/status preflight 之后、第一个 durable/Git 写之前。
- Group completion 在 pause 恢复后重新加载 Attempt，再次要求持久化 `Running + ReviewRequest` 并重验 Issue Shared Worktree owner。
- Provider failure 在首写前重新加载 Attempt，要求仍处于 active status、stage 未漂移，并重验 owner。
- 确定性覆盖 Abort 与 Running group completion、CompletedRetry、provider failure 三类竞态；Abort 落盘并返回后，旧 transition 恢复执行均 fail-closed，Git、Attempt、Unit、Run、handoff、timeline、event 与 lease 快照零写。

### 4. RED → GREEN 与最终门禁发现

| 场景 | RED | GREEN |
| --- | --- | --- |
| ownerless owner 校验 | ownerless Issue Shared Worktree 记录错误通过 runner owner 校验 | 仅精确 Work Item + Attempt owner 通过 |
| registry Abort wait | Abort 在 runner `remove` 前提前返回 | completion signal 等待真实 `remove` 后返回 |
| retired reservation | Abort 后 reservation 未撤销，late activation/insert/reserve 仍成功 | retirement 原子撤销 reservation 并拒绝所有 late 注册路径 |
| Abort 幂等 | 第二次 Abort 改变 `updated_at`/`completed_at` | 已 Aborted Attempt 零写返回 |
| Running completion race | Abort 后旧 completion 继续越过终态并报 `work_item_handoff_missing` | pause 恢复后重载状态，旧 transition 在首写前拒绝 |
| pause seam | 回归测试因缺少 seam 类型/注册函数无法编译 | 三类 durable race 均可确定性暂停并验证零写 |
| 全量门禁夹具 | 两个旧 Group completion integration fixture 只设置 `ReviewRequest`、Attempt 仍为 `Created`，触发 `group_completion_status_not_ready` | 共享 `create_active_coding_unit_run` 同步持久化 `Running + ReviewRequest`，两个用例分别 1 passed |

### 5. 验证结果

Focused 验证：

- Coding Attempt API Group/Abort 模块累计 43 passed。
- Group completion authority/recovery 模块累计 22 passed。
- Provider failure recovery 模块累计 5 passed。
- Coding run registry 4 passed；LifecycleStore 29 passed；Coding WS handler 53 passed。
- Abort vs Running completion、Abort vs CompletedRetry、Abort vs provider failure 三条 durable race 均通过。
- 移动后的 `abort_coding_attempt_releases_issue_shared_worktree_lock` 与 `handle_abort_marks_attempt_aborted_and_closes_active_timeline_node` 分别 1 passed。
- 全量门禁发现的两个旧 Group completion fixture 用例在最小夹具修正后分别 1 passed。

最终门禁：

- `cargo fmt --check`：PASS。
- `cargo check --locked`：PASS。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：PASS，0 warnings。
- `cargo test --locked`：PASS，exit 0；lib、integration 与 doc-test 无失败，Web integration 274 passed、12 ignored，doc-test 1 passed。
- `cd web && pnpm tsc -b`：PASS。
- 本轮未修改前端 Rust/TypeScript 之外的 Web UI 代码，因此未重跑 Vitest 与前端 build；仍运行 TypeScript 编译门禁。
- `git diff --check`：PASS。
- 26 个改动/新增 Rust、TS 文件全部不超过 800 行；最大为 `tests/it_web/web_coding_ws_handler/part_11.rs`，780 行。
- 明确未运行：E2E、Playwright、browser、真实 Provider、网络 CLI。

### 6. 影响范围与边界

- Story Spec 与 Design Spec Workspace 不受影响：本轮只修改 Coding Attempt Group 初始化、Coding runner registry、Abort/Delete、Issue Shared Worktree owner 校验及 Coding Workspace Engine 的 Group completion/provider failure 链路。
- 未修改 Story/Design/Work Item 产物 Workspace 共用的 timeline/chat/artifact version 重建与前端聚合链路。
- Work Item 侧影响仅限 Coding Attempt 对 Issue Shared Worktree lease 的校验、绑定、转移和释放，不改变 Work Item 规划、Story/Design 生成或审核流程。
- 未新增 HTTP test route；checkpoint 与 mutation pause seam 默认关闭，仅能通过测试代码直接配置。
- 提交策略：Round 3 代码、测试、设计、计划与本报告使用一个新原子提交；不 amend，不 push，worktree 保留。

## Round 4 最终修复（2026-07-21）

### 1. Finding 核验与 Single/Group 创建仲裁

- 核验确认 Single 与 Group 创建此前只在各自路径内收敛，Group initialization arbitration 不能阻止 Single 对同一 Work Item 交错创建；两个独立 `WebAppState`/`CodingAttemptStore` 可越过彼此的 active 检查和 Lifecycle lease 窗口。
- 新增带 project/issue/work-item identity 的 `WorkItemAttemptCreationGuard`，底层复用跨进程 `work-item-attempt-locks/{work_item_id}` 文件锁。
- Single handler 在任何 Issue Shared Worktree lease 操作前取得 guard，并持有到 Attempt、provider config 与 lease bind 完成；`create_attempt` 保留自锁兼容入口，handler 使用 guard-aware 入口并再次校验 active Attempt。
- Group handler 固定使用 `initialization arbitration → Work Item creation guard → Lifecycle lease` 顺序，并将 creation guard 持有到 Attempt、provider config、plan binding、Unit 与 journal 全部完成。
- Group ensure 只接受 journal Attempt 精确存在且它是唯一 active Attempt，或在没有任何 active Attempt 时首次写入；其他 active Attempt 均返回 `coding_group_attempt_incomplete`，不覆盖、不清理、不猜测。

### 2. Required `worktree_lease_id` 与 replay fail-closed

- `CodingGroupInitializationJournal.worktree_lease_id` 为 required `String` 字段，没有 `serde(default)` 或 Optional 兼容层；缺字段的旧 journal 直接反序列化失败，明确不兼容旧 Schema。
- 首次 prepare 生成并持久化固定 lease ID，后续重启 replay 必须复用同一 ID，identity equality 也包含该字段。
- `try_acquire_issue_worktree_lock` 返回 `acquired=false` 时，仅当 journal phase 已达到 `WorktreeBound` 且当前 owner 精确等于 journal Attempt ID，才视为已绑定 replay。
- 其他 temporary lease、其他 Attempt owner、ownerless 或 phase 尚未到 `WorktreeBound` 的 owner 状态全部 fail-closed，不写 Attempt、provider config、binding 或 Unit。

### 3. Runner 必达清理与 WebSocket backpressure

- spawned runner future 首行创建 `CodingRunnerRegistrationGuard`；guard Drop 同步执行 registry `remove`，覆盖正常返回、所有早退、panic unwind 与 task cancellation。
- registry Abort 快照新增 run ID；command receiver 已关闭时立即按 run ID remove，再等待其余 completion receiver，避免永远等待一个已不存在的 runner。
- 新增 `abort_attempt_while_draining_events`：等待 Abort completion 时使用 `tokio::select!` 持续 drain outbound event queue，按接收顺序缓存事件，解除 runner send 与 Abort waiter 之间的满队列死锁。
- Abort 完成后复用 `send_coding_event` 顺序写出缓存事件并结算 ACK；socket 写失败时显式标记剩余缓存事件 delivery failure，再退出 socket。
- panic seam 与 runner start probe 拆分到 `runner/start.rs`，RAII 类型位于 `runner/registration.rs`，使 `runner.rs` 保持在 800 行上限内。

### 4. Group replay/Delete 串行与锁序无环证明

- Group Delete 顺序固定为：等待 runner completion → Attempt mutation lease → 重载 Attempt → Group initialization arbitration → Lifecycle/Git cleanup → journal 与 Attempt 目录删除。
- Delete 持有 initialization arbitration 到 `delete_attempt` 返回；Group replay 从 prepare 到 `Completed` 也持有同一 arbitration，因此 replay 与 Delete 在线性化点串行，不再由 Delete 越过半初始化 replay。
- 创建锁序为 `Group initialization arbitration → Work Item creation guard → Lifecycle Issue Shared Worktree 文件锁`；Single 从 Work Item creation guard 开始，不获取 Group arbitration。
- 终止锁序为 `runner completion → Attempt mutation lease → Group initialization arbitration（仅 Group Delete）→ Lifecycle/Git/store cleanup`。
- 创建/replay 不获取 Attempt mutation lease；Abort/Delete 不获取 Work Item creation guard；没有路径在持有 Lifecycle 文件锁后反向获取 creation guard；registry mutex 只做同步快照/更新且不跨 `.await`。因此锁图不存在反向边或环。

### 5. RED → GREEN 与全量门禁发现

| 场景 | RED | GREEN |
| --- | --- | --- |
| Group → Single 创建 | Group 在 lease 后暂停时 Single 可独立创建，两个路径均可能返回 200 | Single 等待同一 Work Item guard；Group 200，Single `coding_attempt_active` 409 |
| Single → Group 创建 | Single 在 lease 后暂停时 Group 可写 journal/进入错误 owner 路径 | Group 等待同一 guard；Single 200，Group typed conflict |
| spawned runner panic | runner panic 后 registry count 不归零，Abort completion 永久缺失 | registration guard Drop 必达 remove，timeout 内 count=0 |
| closed command receiver | Abort send 失败后仍等待 completion watch | send 失败按 run ID 立即 remove，Abort 有界返回 |
| 满 outbound queue | runner 第二次 send 与 Abort 等待 completion 互相阻塞 | Abort helper 持续 drain，按序返回两个事件并完成 remove |
| Group replay/Delete | Delete 越过暂停 replay，立即产生 owner conflict 500 | Delete 保持 pending；replay 200 后 Delete 204，所有初始化状态清除 |
| 既有并发 HTTP 测试 | 新 creation guard 使旧单线程 Tokio 测试在同步文件锁处形成测试等待环 | 改为 2-thread runtime，竞争请求异步保持 pending，先恢复首请求后验证 200/409 与 lease owner |

关键 focused GREEN：

- Single/Group 双向创建交错：2 passed。
- 跨 Store Single 创建串行：1 passed。
- Group initialization restart：2 passed；Single persist→bind restart：1 passed。
- runner panic cleanup 与容量 1 backpressure：2 passed；closed receiver Abort：1 passed。
- Coding run registry：5 passed；Coding WS Abort：3 passed。
- Group retry/Delete arbitration：1 passed。
- 修正后的 `concurrent_same_work_item_loser_preserves_winner_issue_worktree_lease`：1 passed。

### 6. Round 4 最终门禁与审计

- `cargo fmt --check`：PASS。
- `cargo check --locked`：PASS。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：PASS，0 warnings。
- `cargo test --locked`：PASS，exit 0；lib target 运行 1213 tests，Web integration 277 passed、12 ignored，doc-test 1 passed，所有 target 均无失败。
- `cd web && pnpm tsc -b`：PASS。
- 本轮无前端 TypeScript/React 行为改动，因此未运行 Vitest 与前端 build。
- `git diff --check`：PASS。
- 20 个修改/新增 Rust、TS 文件全部不超过 800 行；最大为 `src/web/coding_ws_handler/runner.rs`，恰好 800 行；`socket.rs` 为 779 行。
- `src/web/app.rs` 无 diff；未新增 `/api/test/*` 或其他默认生产 route。新增 pause/panic seam 只能通过直接持有 `WebAppState.test_controls` 或 `cfg(test)` 调用，默认关闭、一次性消费。
- 未发现 TODO、FIXME、TBD 或 placeholder；unstaged 与新增文件均已逐项审阅。
- 明确未运行：E2E、Playwright、browser、真实 Provider、网络 CLI。

### 7. 影响范围与提交边界

- Story Spec 与 Design Spec Workspace 不受影响：本轮只修改 Coding Attempt 创建、Group initialization、Coding runner registry、Coding WebSocket Abort 与 Group Delete 路径。
- 未修改 Story/Design/Work Item 产物 Workspace 共用的 timeline、chat entry、artifact version 或前端聚合链路；Work Item 影响限于 Coding Attempt 的创建唯一性与 Issue Shared Worktree owner 生命周期。
- Round 4 代码、测试、设计、计划与本报告使用一个新原子提交；不 amend，不 push，提交后保留 worktree 且工作区必须 clean。

## Round 5 最终修复（2026-07-21）

### 1. Group replay、Single scope 与 guard identity

- 新增 `BoundBeforePhaseAdvance` checkpoint，确定性覆盖 Issue Worktree 已绑定为 journal Attempt、journal 仍停留在 `AttemptPersisted` 的崩溃窗口。
- Group replay 仅在 phase 至少为 `AttemptPersisted`、owner 精确等于 journal Attempt、磁盘 Attempt/provider config 与 journal 完全一致且该 Attempt 是唯一 active Attempt 时继续；验证过程只读，不补写缺失记录。
- Single 创建遇到 active `WorkItemGroup` Attempt 时只返回 `coding_attempt_active`，不再替 Group journal 修复或绑定 lease；后续 Group retry 仍使用 journal 固定的 lease/Attempt identity 完成恢复。
- `WorkItemAttemptCreationGuard` 现在同时绑定 project/issue/work-item、canonical Store root 与 canonical lock path；来自另一 Store root 的同业务 ID guard fail-closed，目标 Store 零写。

### 2. Async-safe flock 与无 Tokio worker 饥饿

- `ExclusiveFileLock::acquire_async` 使用 `tokio::task::spawn_blocking` 等待同步 `flock`，锁取得后仍由 RAII guard 跨 `.await` 持有，保留原有线性化语义。
- Single create、Group create/replay 与 Group Delete 全部迁移到 async Store acquire；同步 Store API 与单元测试入口继续保留。
- current-thread heartbeat 测试证明竞争 flock 时 Tokio worker 可在锁释放前继续调度；2-worker/8-contender 测试证明多个竞争者不会耗尽 runtime worker。
- `concurrent_same_work_item_loser_preserves_winner_issue_worktree_lease` 恢复为默认 current-thread `#[tokio::test]`，不再依赖 multi-thread runtime 掩盖阻塞锁。

### 3. Registry-owned CancellationToken 与 attach 证明

- `CodingRunEntry` production registration 在 registry mutex 内原子创建并写入 `CancellationToken`，返回 `CodingRunRegistration { run_id, cancellation }`；普通 runner、failed-review reserved runner 与 Plan Amendment reserved runner 均在 spawn 前取得 registration，不存在 spawn 后 attach 窗口。
- runner task 首行建立 `CodingRunnerRegistrationGuard`，外层 `tokio::select!` 竞争完整业务 future 与 `cancellation.cancelled()`；正常返回、早退、panic、event send/provider wait/gate wait 被取消时均由 guard Drop 必达 `remove`。
- `abort_attempt` 先同步写 retired/revoke reservation 并快照 entries，再 cancel 所有 production token；legacy test entry 仅使用无等待 `try_send`，closed receiver 立即 remove；最后等待 completion watch，全程没有 awaited command send。
- `CodingWorkspaceEngine` 保存 parent token；modern stream、legacy stream 与 provider-driven testing execution 均使用 `child_token()`，runner cancel 会传播到 provider adapter/process，而不是仅 drop 上层 future。
- `runner.rs` 从恰好 800 行降至 699 行，runner task 主体迁入 150 行的 `runner/task.rs`；全部修改/新增 Rust、TS 文件继续满足 800 行上限。

### 4. HTTP/WS durable Abort/Delete 语义

- HTTP Abort/Delete 保持 `cancel tokens → wait RAII completion → acquire Attempt mutation lease → reload → durable mutation/cleanup`；runner cancellation 分支本身不写 Attempt 终态或删除 journal。
- WS Abort 先释放消息准备阶段 mutation lease，再 cancel/wait、drain 已排队事件并结算 delivery ACK，最后重新取得 mutation lease执行 `handle_abort`。
- 移除旧的“存在 open stage gate 且已通知 runner 时直接 continue”短路；该逻辑依赖 runner command 写 durable Abort，与 token-only cancellation 不兼容。
- `handle_abort` 统一取消所有 open stage gate 后再写 Attempt=`Aborted`、收口 timeline 并释放 Issue Worktree lease，HTTP 与 WS 使用同一 durable 终态路径。
- 真实 router 回归覆盖：容量 1 outbound queue 满时 HTTP Abort 250ms 内 200、HTTP Delete 250ms 内 204；registry 归零，Abort 持久化 Aborted 并释放 lease，Delete 删除 Attempt 目录并释放 lease。
- pending socket flush 不阻塞 HTTP Abort；writer task 取消后 delivery ACK 失败结算，不能伪造 Delivered。持久化 Plan Amendment marker 的既有回归同时确认 writer abort 后保持 Pending，并可用同 event ID 恢复。
- runner panic HTTP Abort 与 closed command receiver HTTP Delete 均在 250ms 内完成 durable cleanup；Group retry/Delete 既有回归继续确认 journal、Attempt 与 delivery 子目录随 Delete 清除。

### 5. RED → GREEN

| 场景 | RED | GREEN |
| --- | --- | --- |
| bind-before-phase replay | retry 返回 `coding_group_attempt_incomplete` | 严格只读 identity 校验后恢复同 Attempt 并推进 Completed |
| Single 遇未完成 Group | Single 把 temporary lease 改绑为 Group Attempt | 返回 `coding_attempt_active`，lease 保持 journal temporary ID，Group retry 后绑定 |
| 跨 Store guard | root A guard 可用于 root B 创建 | canonical root/lock path mismatch，root B 零写 |
| current-thread flock | async handler 直接阻塞 Tokio worker | `spawn_blocking` 等锁，heartbeat 在释放前运行 |
| 多 flock contenders | 竞争者占满两个 worker | 8 个竞争者不饿死 heartbeat，释放后全部完成 |
| Registry 满 command channel | awaited `send` 无界等待 | token 先 cancel，满 channel 不影响 RAII remove/completion |
| 真实 runner 满 event queue | Abort 250ms timeout | outer select drop blocked send，registry=0 |
| provider cancellation | provider 使用独立 token，runner cancel 不传播 | engine child token观察 parent cancel |
| WS stage gate Abort | token drop 后旧短路不写终态，先无 snapshot、后 gate 仍 open | handler durable Abort，snapshot Aborted且 pending gate为空 |
| HTTP Abort/Delete backpressure | runner completion 依赖 event/command channel 可写 | 满队列/pending writer/panic/closed receiver均有界完成 |

### 6. 验证结果与边界

Focused 验证：

- Group bind replay 与 Single scope：新增 2 条及 journal boundary 既有回归均通过。
- canonical guard：1 passed；async flock heartbeat/contenders：2 passed。
- Coding run registry：6 passed；runner cleanup/真实 HTTP：8 passed；provider child token：1 passed。
- Coding WS Abort：3 passed；pending socket ACK 与持久化 Pending marker recovery 各 1 passed。
- 既有 HTTP Abort lease 与 dirty worktree Delete 各 1 passed。

最终门禁：

- `cargo fmt --check`：PASS。
- `cargo check --locked`：PASS。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：PASS，0 warnings。
- `cargo test --locked`：PASS，exit 0；lib target 运行 1224 tests，Web integration 279 passed、12 ignored，doc-test 1 passed，所有 target 无失败。
- `cd web && pnpm tsc -b`：PASS。
- `git diff --check`：PASS。
- `src/web/app.rs` 无 diff；未新增 HTTP/test route。`TestControls` 仅新增默认关闭、一次性消费的 checkpoint 枚举值。
- 24 个修改/新增 Rust、TS 文件全部不超过 800 行；最大为 `src/product/coding_workspace_engine/testing_provider/execution.rs` 796 行，其次为 `src/web/coding_ws_handler/socket.rs` 774 行。
- 本轮无前端 TypeScript/React 行为改动，因此未运行 Vitest 与前端 build。
- 明确未运行：E2E、Playwright、browser、真实 Provider、网络 CLI。
- Story Spec 与 Design Spec Workspace 不受影响：本轮只修改 Coding Attempt 创建/Group initialization、runner cancellation、Coding HTTP/WS Abort/Delete 与 Issue Shared Worktree lease 链路；未改产物 Workspace 共用的 timeline/chat/artifact version 前端恢复路径。
- Round 5 代码、测试、设计、计划与本报告使用一个新原子提交；不 amend，不 push，提交后 worktree 必须 clean。

## Round 6 最终修复（2026-07-21）

### 1. Runner 协作式取消与 provider 终态

- runner 固定 pin 同一个业务 future；CancellationToken 触发后不再 drop 业务 future，而是继续等待它完成自身 cleanup，再由 registration guard 移除 registry entry。
- outbound event、stage gate、provider start/stream 与 provider command send 均增加 token-aware cancellation 边界，取消统一返回 `CodingWorkspaceEngineError::Aborted`。
- modern provider、legacy provider 与 provider-driven testing 均使用 Engine parent token 的 child token；取消时 adapter/process 可观察 token，而不依赖上层 future 被动销毁。
- provider start/stream 取消会追加 `Aborted` role-run event，并把仍为 `Running` 的 role run 持久化为 `Aborted`。
- 为满足单文件 800 行约束，provider event/cancellation 持久化辅助方法移入 `provider_stream/persistence.rs`；testing execution 数据结构移入 `execution_types.rs`，状态机行为不变。

### 2. 真正可取消的 async flock

- `ExclusiveFileLock::acquire_async` 改为 `LOCK_NB` 尝试加锁与 Tokio async backoff，不再使用不可取消的 `spawn_blocking` flock waiter。
- 每轮失败尝试后只等待可取消的 Tokio sleep；contender future 被 drop/abort 时立即 drop lock file 并停止重试，不会在 holder 后续释放时迟到取得锁并继续业务。
- current-thread heartbeat、永久 holder 取消 contender 与多 contender 回归覆盖 runtime 可调度、无 busy-loop 和取消后无迟到业务。

### 3. Git command 生命周期与 durable journal

- Git CLI 统一使用 `kill_on_drop(true)` 与独立 process group；timeout/cancel 显式 kill 后 wait/reap，stdout/stderr 使用独立 drain task，避免子进程或 pipe task 遗留。
- Attempt 级 Git journal 使用严格 identity、canonical path 与相邻 phase 单调推进，不提供历史 serde 兼容：
  - Worktree：`Before → BranchCreated → WorktreeCreated → Completed | Compensated`。
  - Review：`Before → CommitStarted → CommitCreated → PushStarted → Completed | Compensated`。
- `Completed` 保存 commit SHA、push status、remote kind 与 review request ID，使远端已成功更新时以权威事实收口，而不是伪回滚。
- branch/worktree-add 命令完成边界取消后探测并删除新 worktree/branch；commit 完成边界取消后校验 parent/message，再 `git reset --mixed before_head`，恢复 HEAD 且保留文件内容。
- push 完成边界取消后探测 remote ref：远端已更新则持久化 `Completed`、ReviewRequest 与 Attempt；远端未更新或被拒绝则 mixed reset 并记录 `Compensated`。正常 push 失败仍记录 `Completed + PushStatus::Failed`。
- `handle_abort` 在写 Attempt=`Aborted` 前 reconcile；HTTP Delete 在 repository/workspace cleanup 前再次 reconcile，覆盖运行时取消后进程已完成、但内存控制流尚未收口的窗口。

### 4. Registry legacy API 与测试控制点收口

- 删除 legacy `CodingRunRegistry::insert` 与 `CodingRunReservation::activate`；entry cancellation token 改为 required，所有 production/reserved runner 都通过同一 registration 路径取得 token。
- Abort 固定先 cancel token，再使用无等待 `try_send` 发送兼容提示；full command channel 不阻塞，closed receiver 按 run ID 移除，最终等待 registration guard completion。
- Git 命令完成暂停器支持并行 registration，并以 `cwd + command_prefix` 精确匹配。全组回归曾复现不同临时仓库的 branch/worktree 命令抢占彼此 pause；加入 cwd scope 后同组连续两次 7/7 通过。
- 暂停器作为 production-compiled test-control 直接调用 seam 存在于 Git service 子模块，并通过 `src/web/test_controls` re-export；默认没有任何 registration，且没有新增 route。

### 5. RED → GREEN

| 场景 | RED | GREEN |
| --- | --- | --- |
| runner token cancellation | outer select drop 业务 future，cleanup probe 不执行或 registry 过早归零 | 继续 await 同一业务 future，cleanup 完成后 registration guard 才 remove |
| provider cancellation | parent token 不稳定传播，role run 可残留 `Running` | child token 传播到 adapter/process，event 与 role run 权威持久化 `Aborted` |
| async flock contender | `spawn_blocking` waiter 无法取消，holder 释放后迟到进入业务 | `LOCK_NB + async backoff` 随 future drop 停止，无迟到业务 |
| branch/worktree-add cancellation | Git 命令完成后取消留下 branch/worktree 副作用 | 探测真实 Git 状态并补偿，journal=`Compensated` |
| commit cancellation | 新 commit 留在 HEAD，或 hard reset 丢失文件内容 | 校验 commit 身份后 mixed reset，HEAD 恢复且工作区内容保留 |
| successful push cancellation | 已更新 remote 仍被当作本地失败回滚 | remote ref 证明成功后 journal/ReviewRequest/Attempt=`Completed` |
| rejected push cancellation | 本地 commit 与未完成 journal 遗留 | remote 未更新，mixed reset 并 journal=`Compensated` |
| Abort/Delete 二次 reconcile | durable terminal cleanup 可越过已完成 Git 副作用 | terminal mutation 前重放 journal 并收敛 Git/Store 权威状态 |
| full registry channel | awaited command send 阻塞 Abort | token-first + `try_send`，full/closed channel 均有界完成 |
| 并行 Git pause | 仅按参数前缀匹配，不同临时仓库互相抢占 pause | 同时匹配 cwd 与参数前缀，连续两轮 7 项并行回归全绿 |
| all-target registry fixture | integration fixture 仍调用已删除的 legacy `insert`，clippy 编译失败 | 8 处调用迁移到 `insert_cancellable` 与 registration accessor，相关 4 条 HTTP/WS integration 各 1 passed |

### 6. Focused 验证

- provider start persistence 与 parent cancellation：3 passed。
- Git journal/store 与高层 cancellation/reconcile：`git_operation` 9 passed；`git_operation_reconcile` 7 passed，修复 pause scope 后连续运行两次均通过。
- Review request integration：4 passed；worktree prepare integration：3 passed。
- async file lock：3 passed；runner cleanup：9 passed。
- failed-review recovery：42 passed；coding run registry：6 passed。
- legacy registry integration fixture 迁移：4 条受影响 HTTP/WS 用例各 1 passed。

### 7. 最终门禁与边界

- `cargo fmt --check`：PASS。
- `cargo check --locked`：PASS。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：PASS，0 warnings。首轮 all-target 编译发现 8 处 legacy registry fixture，迁移后 fresh clippy 通过。
- `cargo test --locked`：PASS，exit 0；lib 1237 passed，`it_core` 143 passed，`it_interactive` 43 passed，`it_product` 221 passed，`it_web` 280 passed、12 ignored，doc-test 1 passed，其余 target 无失败。
- `cd web && pnpm tsc -b`：PASS。
- `git diff --check`：PASS。
- 42 个修改/新增 Rust、TS 文件全部不超过 800 行；最大为 `src/product/coding_workspace_engine/testing_provider/execution.rs` 799 行，其次为 `src/product/git_workspace_service.rs` 789 行。
- `src/web/app.rs` 与 lockfiles 无 diff；源码新增行没有 route 注册或 `/api/`、`/ws/` 路径；Git pause seam 默认无 registration，只能通过直接持有公开 test-control API 配置。
- Story Spec、Design Spec 与 Work Item 产物 Workspace 共用的 timeline/chat/artifact version 恢复链路未修改。本轮 Work Item 影响仅限 Coding Attempt runner、Git workspace/review 与 Issue Shared Worktree 终止收口。
- 明确未运行：E2E、Playwright、browser、真实 Provider、网络 CLI。
- 提交策略：Round 6 代码、测试、设计、计划与本报告使用一个新原子提交；不 amend，不 push，提交后 worktree 必须 clean。

## Round 7 最终修复（2026-07-21）

### 1. Tester ToolCall cancellation 贯穿

- `CancellationToken` 贯穿 `testing provider → tester_agent_loop → test_executor`；Provider ToolCall 执行不再使用不可取消的 `Command::output()`。
- `TestExecutor` 复用 `TokioBoundedCommandRunner` 的继承宿主环境入口，继续保留既有命令 PATH/环境语义，同时获得 process group、timeout/cancel select、kill/wait/reap 与 stdout/stderr drain。
- `TestExecutorError::Cancelled` 为明确取消结果；取消期间不创建 stdout/stderr artifact，artifact 写入边界再次与 token 仲裁，并清理可能的部分文件。
- Tester ToolResult 发送改为 cancellation-aware reserve/send；取消先胜时不发送 Provider ToolResult、不持久化 ToolResult event/chat，并通过既有 `persist_provider_cancellation` 将 RoleRun 与事件收敛为 `Aborted`。
- 为保持单文件上限，ToolCall 执行与发送仲裁迁入 `testing_provider/execution_tool.rs`；`execution.rs` 保持 799 行。

### 2. 统一 process-group 异常 Drop 边界

- 新增共享 `ManagedProcessChild`，由 ProcessManager、Bounded runner、Claude、Codex 与 Git 共用，不再由 Git/Provider 各自维护重复的异常终止实现。
- 所有命令同时启用 Tokio child `kill_on_drop(true)`；Windows 使用 `command.group().kill_on_drop(true).spawn()`，保证 Job Object handle Drop 时终止整个 job。
- Unix 保存 spawn 时 PGID：正常 `wait()` 成功后 disarm；future abort、panic unwind 或 runtime drop 时同步 `killpg(SIGKILL)`，并同步轮询 `try_wait()` 回收直接 child，避免 leader zombie 与孙进程迟到副作用。
- timeout/cancellation 仍显式调用共享 `terminate()`，完成 group kill、wait/reap 与输出 pipe drain；正常退出继续使用 command-group 的 group wait。
- Git 单文件中的测试迁移到 `git_workspace_service/tests.rs`，生产文件由 789 行降至 685 行。

### 3. Push 非零后的远端权威三态

- 新增纯决策：remote ref 等于 commit → `Pushed`；remote ref 明确不同或不存在 → verified `Failed`；remote query error → `Indeterminate`。
- `git push` 非零后不再立即写 `Completed + Failed`，而是先执行 `ls-remote` 权威回查：
  - remote 已更新到当前 commit：完成 journal，ReviewRequest=`Pushed`。
  - remote 明确未更新：完成 journal，ReviewRequest=`Failed`，维持既有 Attempt Blocked 策略。
  - remote 无法查询：返回错误且 journal 保持 `PushStarted`，不创建 ReviewRequest，不写 Attempt review identity；重复调用继续 fail-closed，可在外部状态恢复后重试。
- 本地 receive-pack wrapper 真实证明“远端已接受、客户端进程非零”的场景，避免仅用 mock 推断远端权威结果。

### 4. RED → GREEN

| 场景 | RED | GREEN |
| --- | --- | --- |
| provider tester cancellation | Engine 等长命令完成后才返回，且写入 `tester-late` | 取消有界返回 `Aborted`，RoleRun=`Aborted`，无 late file/ToolResult/artifact |
| HTTP Abort/Delete tester | Abort 超过 250ms，Delete 可被真实 tester 命令拖住 | 两条 HTTP 路径均有界完成，registry=0，无 Store artifact/recreation |
| ProcessManager task abort | future drop 后 leader/孙进程存活或成为 zombie | Unix Drop killpg 并回收 leader，父孙 PID 消失且无 late marker |
| ProcessManager panic/runtime drop | unwind 或 current-thread runtime drop 遗留进程树 | panic 与 runtime drop 均收敛，无 zombie/迟到副作用 |
| Git hook future abort | 只杀直接 git child，pre-commit hook/孙进程继续 | 共享 PGID Drop guard 杀整组，HEAD 不变且无 late marker |
| push nonzero + remote updated | 直接 `Completed + Failed`，durable 状态错误 | remote ref 等于 commit，ReviewRequest/journal=`Pushed` |
| push nonzero + remote absent | 未区分是否完成远端验证 | remote 明确缺失后 ReviewRequest/journal=`Failed` |
| push nonzero + remote query error | 错误写终态 Failed，后续不再查询 | journal 保持 `PushStarted`、无 ReviewRequest，重复调用 fail-closed |
| strict clippy | ToolCall helper 9 参数触发 `too_many_arguments` | 使用 `TesterToolExecutionInput` 上下文对象，未放宽 lint |

### 5. Focused 验证

- Tester cancellation：Engine 1 passed；HTTP Abort/Delete 2 passed。
- Tester/TestExecutor 兼容：tester loop lib 11 passed、integration 4 passed；TestExecutor integration 12 passed。
- Process Drop：task abort、task panic、current-thread runtime drop 3 passed；Bounded runner 9 passed。
- Git 异常终止：显式 cancellation 与 future abort hook 2 passed。
- Push 三态：纯决策 3 passed；missing remote、verified rejection、nonzero-but-remote-updated 各 1 passed；既有 Git journal cancellation/reconcile 7 passed。

### 6. 最终门禁与边界

- `cargo fmt --check`：PASS。
- `cargo check --locked`：PASS。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：PASS，0 warnings；首轮发现 helper 参数过多，改为上下文对象后 fresh clippy 通过。
- `cargo test --locked`：PASS，exit 0；lib 1247 passed，`it_product` 223 passed，`it_web` 280 passed、12 ignored，doc-test 1 passed，其他 target 无失败。
  - 提交前 fresh 首次运行曾在 `claude_provider_classifies_missing_end_nonce` 启动临时 fixture 时遇到一次 `Text file busy (os error 26)`；未修改代码即定向复跑通过，随后完整 `cargo test --locked` 复跑 exit 0，判定为不可复现的执行环境瞬态。
- `cd web && pnpm tsc -b`：PASS。
- `git diff --check`：PASS。
- 21 个修改/新增 Rust 与 integration source 文件全部不超过 800 行；最大为 `src/product/coding_workspace_engine/testing_provider/execution.rs`，799 行。
- `src/web/app.rs` 与 lockfiles 无 diff；新增源码没有 route 注册或 `/api/`、`/ws/` 路径；未新增依赖。
- Story Spec、Design Spec 与 Work Item 产物 Workspace 共用的 timeline/chat/artifact version 恢复链路未修改；本轮 Work Item 影响限于 Coding Tester 命令、共享进程生命周期与 ReviewRequest push 收口。
- 已知 Minor `test-controls` 生产编译边界仍保留：默认无 route，本轮未扩大到 test-support crate 重构，也未发现适合顺手加入的低风险生产误配 fail-startup 边界。
- 明确未运行：E2E、Playwright、browser、真实 Provider、网络 CLI。
- 提交策略：Round 7 代码、测试与本报告使用一个新原子提交；不 amend，不 push，提交后 worktree 必须 clean。
