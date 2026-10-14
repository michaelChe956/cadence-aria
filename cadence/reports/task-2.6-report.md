# Task 2.6 (A2b: 真实等待收口与后端旁路守卫) 实施报告

- worktree: `/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo`
- 基线: 7462f22b（2.4 已落 router/attach/socket 全 caller Result 闭包）+ 4af2113c（2.5 已落 durable 等待 CAS/claim）
- 红绿命令: `cargo test --locked --lib anchor_t07`（cwd=worktree 根；隔离 target `/tmp/anchor_t26_target`，不占 4317 release 编译锁）
- 说明: 本文件为证据记录，按 brief 不 git add。

## 写面（brief 显式列表，全部 7 文件均归属本包）

1. `src/product/workspace_engine/lifecycle.rs` —
   - `lc_resume_recovery_active()`：Waiting/FreshReserved/FreshRunning 视为 active，Resolved 只留证据不阻断；
   - `persist_lc_superseded_wait(context, reason) -> Result<LcResumeRecoveryRecord, String>`（Plan 接口字典）：**安全 record 先落盘**——读取最新 record 作 expected，经 A2a `compare_and_save_lc_resume_state` 整记录 CAS single-flight 落 `Waiting` recovery（recovery_id=`lc_resume_recovery_<uuid>`；superseded_native_session_id 取旧 Author resume 引用、previous_fingerprint_digest 取 engine 指纹索引、failed_provider_node_id 取 active 节点兜底 run_context 原节点）；CAS 成功后同步 engine 内存镜像；
   - `reconcile_lc_resume_waiting()`：真实收口写入口——幂等终态化旧 provider 节点（仅 Active→Failed，已终态不覆写）、清 active run/active node、durable status 仅 Running→Open、run 三阶段（Running/CrossReview/Revision）回 PrepareContext；SC gate/phase/turn/预算与人审 CAS 不触碰；
   - `start_generation` 顶部守卫：active recovery 期间普通参数一律 `Err("LC_RESUME_RECOVERY_ACTIVE: …")`，先于终态检查与 SingleCandidate 重臂（SC 不 rearm）；
   - `new_persistent_for_projection` 纯派生块：waiting active 时**只在内存**派生节点终态/无 active/run 阶段 PrepareContext，零磁盘写（fallback 不成为另一写方）；`lc_waiting_node_summary` 与真实收口同源文案；`lc_recovery_context_original_node_id` 自由函数（12 tag 全臂取 business.original_node_id）。
2. `src/product/workspace_engine/session_state/timeline.rs` — `finalize_lc_waiting_provider_node`（幂等：仅命中且 Active 才 `update_timeline_node(Failed)`，durable timeline+node detail+TimelineNodeUpdated 事件同步）。
3. `src/product/workspace_engine/review/drive.rs` —
   - `drive_revision_session` 顶部等待守卫：active recovery 期间同 driver 重驱 → 幂等 reconcile + typed 等待错误（`blocked by lc resume recovery`），零 provider 启动（替代旧「resume id 清空后可自动 fresh」路径）；
   - mismatch（StartNew/supersede）臂真实等待收口：`persist_lc_superseded_wait` 先 → `clear_superseded_provider_session` 清旧 native 引用 → `reconcile_lc_resume_waiting` → 稳定等待错误上浮（保留既有 `superseded`/`start a new generation explicitly` 文案契约，附 recovery_id）；**不再走 `finish_failed_run` generic 失败**；persist 写失败 typed 传播不吞（错误事件 + 原失败收口，仍零启动）；
   - `lc_recovery_business_context()`：engine 已知身份全量（entity/type/author/effective reviewer/原节点/repository_path 字符串化），未知身份按 A1a 纪律留 None 不倒算。
4. `src/product/workspace_engine/interrupted_run_recovery.rs` — `InterruptedRunRecoveryError::LcResumeWaiting`（code `LC_RESUME_RECOVERY_ACTIVE`）；`retry_interrupted_run` 在 stage/active 检查后、recoverable 判定前 typed 拒收；`recoverable_interrupted_run` 对被等待占用节点 `filter` 不投影（SessionState 不投可恢复假态）。
5. `src/web/workspace_session/manager/durable_projection.rs` — `recover_on_creation` **先** reconcile（任何自动 recover_outline/human_turn 之前）；waiting active 时早退：阻断旧 outline resume、human gate turn 重驱、委托返修孤儿回落、僵尸 run 终态化，并 `broadcast_current_session_state`。
6. `src/product/workspace_engine/tests/author_revision_loop_parts/root_cwd_revision.inc.rs` — 新用例 `anchor_t07_superseded_waiting_finishes_provider_node`；既有 `revision_resume_cwd_drift_supersedes_session` 第三轮按 A2b 契约更新（普通重驱被拒、零新增启动、durable waiting record 断言）。
7. `src/web/workspace_ws_handler/tests/interrupted_run_recovery.rs` — 新用例 `anchor_t07_waiting_blocks_plain_generation_and_interrupted_retry`。

### 关键实现决策

- **run_context tag**: revision mismatch 捕获为 `ReviewerRevision{original_feedback=pending_revision_context}`——A2b 唯一 live mismatch 路径是 author revision resume supersede；Story/Design fresh 由 5.2 `build_revision_input_without_resume` 消费，tag 选择不阻塞 B 波（B1b/B2 covered-run handler 按 run kind 补全重建事实）。
- **tombstone Vec 不越权**：`superseded_native_sessions` 多代 tombstone 的写入/消费归 B1a（4.1 资格门）；本包只落 recovery record（内含 superseded_native_session_id）+ 清 conversation 引用（既有 `clear_superseded_provider_session`），不让 A2b 越权写 store。
- **等待不落 generic SC failure**：reconcile 的 status 仅 Running→Open、stage 仅 run 三阶段回 PrepareContext——SC 已收敛 Failed 终态、人审门（HumanConfirm/WaitingForHuman）与非 run 阶段原样保留（§4.3「等待不等价 Failed 业务 turn」）。
- **驱动侧双防线**：顶部守卫（等待期间任何重驱零启动）+ mismatch 收口臂（持久等待）；RequestRevision 入站拒收按设计归 B1b（inbound.rs 不在本包写面）。
- **commit 隔离**：主 worktree 同期有并行 trust 波 30+ 文件在写（均非本包 7 文件，已逐 hunk 核对本包 diff 无并行方改动混入）；仅 stage brief 显式列表。

## TDD 证据

### 红（Step 2）

命令: `cargo test --locked --lib anchor_t07`（2026-10-11，实现前，基线 7462f22b，隔离 target）

新接口编译红（计划 §0.2「新接口初次可编译红但绿须全契约」；task-2.3 报告 E0432 先例同型）：

```
error[E0599]: no variant, associated function, or constant named `LcResumeWaiting` found for enum `InterruptedRunRecoveryError` in the current scope
   --> src/web/workspace_ws_handler/tests/interrupted_run_recovery.rs:435:80
    |
435 |             Err(crate::product::workspace_engine::InterruptedRunRecoveryError::LcResumeWaiting)
    |                                                                                ^^^^^^^^^^^^^^^ variant, ... not found
```

错误位于 anchor_t07 新测试代码内（过滤命中新用例）。完整原始输出留存 `/tmp/anchor_t07_red_raw.txt`、错误详情 `/tmp/anchor_t07_red_errors.txt`。

### 绿（Step 4）

命令（cwd=worktree 根，CARGO_TARGET_DIR=/tmp/anchor_t26_target）：

```
$ cargo test --locked --lib anchor_t07
running 2 tests
test web::workspace_ws_handler::tests::interrupted_run_recovery::anchor_t07_waiting_blocks_plain_generation_and_interrupted_retry ... ok
test product::workspace_engine::tests::author_revision_loop::anchor_t07_superseded_waiting_finishes_provider_node ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 4326 filtered out
```

红→绿间的真实迭代（编译层，不改行为语义）：导入行误替换(super::*/WorkItemPlanFlowKind/ReviewProviderRunFailure)恢复、`role`/`now` clone、`TimelineNodeStatus` 枚举误用更正、business_repository_path String 化、测试 expected.status clone。

### 范围化回归组（Plan 8.1 表 2.5/2.6 行 + 安全覆盖，同隔离 target）

```
$ cargo test --locked --lib interrupted_run        → ok. 7 passed; 0 failed   （2.5/2.6 行：waiting 拒普通旁路）
$ cargo test --locked --lib workspace_session      → ok. 71 passed; 0 failed  （manager/recover_on_creation 面）
$ cargo test --locked --lib author_revision        → ok. 26 passed; 0 failed  （revision/drift fixture 族，含既有 supersede 用例更新后行为）
$ cargo test --locked --lib workspace_engine       → ok. 1402 passed; 0 failed（engine 全量安全网）
$ cargo test --locked --lib anchor_t0              → ok. 14 passed; 0 failed  （相邻 anchor 包 t01–t07）
```

`cargo fmt --check` 对 7 文件 clean（fmt 后复跑 anchor_t07 仍 2 passed）。

## 完成判据对照（tasks.md 2.6 / plan Step 1）

- mismatch 0 start ✓（`provider_starts` 漂移前后相等 + 零捕获 input；等待期间重驱零启动）
- 旧 provider node Failed、无 active ✓（`old_node.status==Failed`、`active_timeline_node_id/active_run_id` 双 None）
- 真实 recover 入口 reconcile、pure fallback 不写 ✓（`recover_on_creation` 先 reconcile 且 waiting 时阻断全部自动接力；纯投影用例：磁盘节点保持 Active、内存派生 Failed/PrepareContext，`load_timeline_nodes_for_issue_session` 复核零写）
- 普通 generation / interrupted 旁路拒 ✓（`LC_RESUME_RECOVERY_ACTIVE` String 稳定码 + `LcResumeWaiting` typed 变体；`recoverable_interrupted_run`/SessionState 不投影被占用节点；durable record 拒收后不变）
- SC 原业务态不 Failed/rearm ✓（reconcile 条件化 status/stage；守卫先于 SingleCandidate rearm 分支——结构性保证，未另立 SC 专属用例，B2c/B2d 波补 SC 场景断言）
- waiting allowed_actions 为空直到 B 齐 ✓（两用例均断言 DTO `allowed_actions.is_empty()`；2.2 `from_record` 恒空实现 + 本包不放宽）

## 提交（Step 5）

```
git add -- src/product/workspace_engine/review/drive.rs src/product/workspace_engine/session_state/timeline.rs src/product/workspace_engine/lifecycle.rs src/product/workspace_engine/interrupted_run_recovery.rs src/web/workspace_session/manager/durable_projection.rs src/product/workspace_engine/tests/author_revision_loop_parts/root_cwd_revision.inc.rs src/web/workspace_ws_handler/tests/interrupted_run_recovery.rs
git commit -m "fix: A2b 真实等待收口与后端旁路守卫"
```

commit hash: `cf92ad32`（7 files changed, 861 insertions(+), 34 deletions(-)）

## 移交

- RequestRevision 入站拒收（inbound.rs 任何业务修订写入前）→ Task 4.2（B1b，socket/inbound 串行）
- `resume_recovery{recovery_id,intent,operation_id}` 显式 fresh 入参与 socket Replay 接线 → Task 4.2（B1b）
- Plan 全 run-kind 的 tag capture/自动接力守卫（stale outline/turn resume）→ Task 5.1+（B2a 起；本包 `recover_on_creation` 早退已阻断 manager 侧自动接力，provider_run 分流归 B2）
- SC author/manual 的 `NativeSupersededWaiting` 独立 typed 分类与同 reservation 重入 → Task 5.5/5.6（B2c/B2d）
- tombstone Vec 写入/消费 + `provider_resume_session_id`→typed candidate clean-cutover → Task 4.1（B1a）
- `persist_lc_superseded_wait` 的 run_context 目前覆盖 Author revision resume 面；Plan 族 tag 捕获由 B2 各包在其 builder take 前固化
