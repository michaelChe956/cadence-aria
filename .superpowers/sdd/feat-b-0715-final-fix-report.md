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
