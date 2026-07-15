# 最终整分支审查修复报告

## 状态

- 结果：DONE_WITH_CONCERNS
- 实现提交：
  - `58b03e5 fix: serialize interrupted code review retries`
  - `9026fcf fix: safely degrade structured review repair`
  - `b30680c fix: close recovery admission races`
  - `380cad6 fix: gate recovery test exports`
  - `f6cefa2 fix: narrow recovery attempt lock scope`
  - 第四轮 ordinary-first mutation lease 修复与本报告更新位于同一原子提交。
- 工作目录：`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0709`
- 未触碰真实 `coding_attempt_0001`，未访问或修改共享 issue worktree。
- dirty worktree 中原有 Workspace 中断恢复、结构化审核及前端改动均保留，本任务实现提交只包含各轮审查修复文件。

## 根因复核与修复

### Important 1：新 Code Review retry 绕过 reservation/journal

根因复核：

- `recoverable_failed_code_review()` 只识别历史 `Failed + CodeReview`，不识别新 `Blocked + code_review_provider_interrupted`。
- 新 Gate 对应 Reviewer Role Run 已正确持久化为 `Failed`，但 journal 校验只接受历史遗留的 `Running/Superseded` stale run。
- socket 因未识别该恢复请求而进入普通 `handle_blocked_gate_response()`，先执行多文件 Gate/Role Run/Attempt 写入，再走普通 `spawn_coding_runner()`。
- 产品层普通 Gate API 也可直接执行 interrupted review 的 `retry_review`，形成 reservation 旁路。
- 第一轮修复后仍存在 reservation 已取得、journal 尚未 Prepared 的窗口；另一 socket 可在该窗口执行 Abort/GateResponse，且旧 `abort_attempt()` 会删除 reservation。

修复：

- 新 `Blocked + CodeReview` Gate 通过内部 blocked-gate record 精确绑定 Gate ID、失败 Timeline Node ID 与失败 Reviewer Role Run ID。
- 新旧两种入口统一复用 `FailedCodeReviewRecoveryJournal` 状态机。
- socket 在任何恢复写入前取得 Attempt 级 reservation，并只使用 `spawn_coding_runner_reserved()` 激活该 reservation。
- `prepare_coding_message()` 统一生产 admission 生命周期：Hello/Ping 在 Attempt lock 前返回；其他消息在锁内 reload/admission，普通消息返回前释放锁。
- recovery 在同一 Attempt 临界区内完成 reload、admission、reservation 与 journal 幂等推进至 GateResolved；helper 返回前显式释放 guard，此时 reservation 仍 active、journal 仍 unfinished。
- `spawn_coding_runner_reserved()` 在 guard 释放后激活 reservation 并完成 journal；snapshot 构建与所有 socket I/O 也不再持有 Attempt lock。
- `abort_attempt()` 不再删除 recovery reservation；reservation 只由其持有者激活、显式释放或 Drop 释放。
- 普通 `handle_blocked_gate_response()` 对 `code_review_provider_interrupted + retry_review` 返回 `coding_failed_review_recovery_requires_reservation`，彻底关闭普通 Role Run/Runner 旁路。
- 新失败 Role Run 保持 `Failed` 状态，仅记录 `superseded_by_run_id`；历史遗留 `Running` stale run 仍按旧兼容语义转为 `Superseded`。
- 覆盖旧 Runner 未退出、两个 socket 并发、Prepared/AttemptReopened/RetryRunCreated/AttemptRunning/GateResolved 崩溃前缀和重复恢复；最终只有一个 RetryReview Role Run、一个 Runner。

### Important 2：未完成 journal 消息白名单缺失

根因复核：

- socket 在通用 stage guard 前没有检查未完成 recovery journal。
- `Blocked` 通用规则允许任意 `GateResponse` 与 `AbortAttempt`，因此可在 journal 中间态写入无关状态。
- 第一轮 journal 白名单只覆盖 journal 已存在后的阶段，未覆盖 active reservation 到 Prepared 之间的窗口。

修复：

- 在通用 stage guard 前读取未完成 journal。
- 仅允许 `expected_gate_id + retry_review`，且该消息还必须通过完整 recovery identity 校验。
- 错误 Gate ID、`manual_continue`、`send_to_coder`、`abort` Gate action、普通 `ContextNote` 与 `AbortAttempt` 均稳定进入 `coding_message_not_allowed` 分支。
- Prepared、AttemptReopened、RetryRunCreated、AttemptRunning、GateResolved 前缀均验证精确 retry 可继续、Abort 不可绕过。
- 普通 terminal/stage 消息规则未放宽。
- 生产共用 preparation helper 的可控测试证明：winner 在 reservation 建立且 journal 尚未创建时持锁，AbortAttempt/ContextNote 等待；helper 推进至 GateResolved 并释放 guard 后，两条竞争消息均因 active reservation/unfinished journal 被 Reject，随后 winner 正常激活 reserved runner 并将 journal 完成至 Completed。

### Important 3：repair Provider 失败未安全降级

根因复核：

- `ReviewProviderRunResult::Terminal` 同时表达明确 Abort 与 Provider 执行失败。
- `drive_reviewer_provider_session_once()` 在启动失败、空 completion、`ProviderEvent::Failed`、权限超时或空 stream close 时直接调用业务级失败收尾，repair 编排无法再使用首次 completion 生成 fallback verdict。

修复：

- 将单次 reviewer 结果拆为 `Completed`、`Aborted` 与 `Failed(ReviewProviderRunFailure)`。
- 明确 Abort/cancellation 继续终止业务 Review。
- 首次 Review Provider 失败继续走原有失败语义，保持既有回归行为。
- repair Provider 执行失败不再提前修改业务 Review Node/Session；关闭 repair execution event 后，保留首次 completion，调用 `fallback_review_verdict(..., repair_attempted=true)` 并完成原 Review。
- 启动失败、空 completion、Failed event、权限超时、空 stream close 最终均进入 `HumanConfirm`，产生 `needs_human` 与 `structured_output_diagnostic`，其中 `repair_attempted=true`、`repair_succeeded=false`。
- Story、Design、Work Item 现在通过持久化表驱动覆盖 3 × 5 共 15 个 repair failure 场景，断言 Session=`WaitingForHuman`、Review Node=`Completed`、diagnostic 持久化及 reload 一致。
- Work Item Plan 保留 Outline terminal fallback 代表用例；Draft/Batch 复用同一 `drive_review_session()`，其独立 scope/routing 由 item/batch parser 与 review action 测试覆盖。
- repair PermissionTimeout 会在返回非业务终止的 failure 前持久化 timeout response；request→timeout→fallback→reload 后 NodeDetail 不再存在 response=None 的 pending permission。
- 实施计划已更正为“Abort 终止；repair 执行失败安全降级”，删除冲突的通用 `Terminal => return` 描述。

### Minor 1：repair 不应要求非空原生 Provider session ID

- 删除非空 session ID 前置条件。
- `None` 或空白 session ID 统一归一化为 `None`，同一 Provider 启动 fresh session 执行一次 repair。
- 测试确认 Provider 总启动次数为 2，第二次 `resume_provider_session_id=None`，且 repair 最多一次。

### Minor 2：结束边界不确定时不得截断可读文本

- 完全缺少结束标签或结束标签未闭合时，`readable_output` 保留 `full_output`。
- 仅在结束边界明确时剥离 structured block。
- 新测试包含 structured JSON 后的尾随 reviewer 说明，确认全文不被截断，同时仍保留可验证的 `recoverable_value`。

## TDD RED / GREEN 证据

### Important 1

- RED：`cargo test --locked --lib blocked_provider_interrupted_review_retry_enters_the_same_recovery_journal`
  - 失败原因：`blocked review must be recoverable`，证明新 Blocked Gate 未进入 journal。
- RED：`cargo test --locked --lib blocked_provider_interrupted_retry_cannot_use_the_ordinary_gate_path`
  - 失败原因：普通 Gate API 返回 Running Attempt，而非 reservation-required 错误，证明旁路存在。
- GREEN：以上两个命令均通过。
- GREEN 汇总：`cargo test --locked --lib failed_review_recovery`
  - 结果：21 passed，包含旧 Runner、双 socket、生产 preparation 生命周期、reserved spawn、journal 完成顺序与新旧多前缀幂等收敛。

### Important 2

- RED：`cargo test --locked --lib unfinished_blocked_review_journal_allows_only_its_exact_retry_message`
  - 失败原因：缺少 `unfinished_failed_code_review_recovery_message_allowed`，证明专用白名单不存在。
- GREEN：同命令通过。
- GREEN 汇总：`cargo test --locked --lib failed_review_recovery`，21 passed。

### Important 3

- RED：`cargo test --locked --lib repair_terminal_paths_close_started_event_as_failed`
  - 失败原因：StartError 后 stage 为 `PrepareContext`，期望 `HumanConfirm`，证明 repair 失败提前终止业务 Review。
- GREEN：同命令通过，覆盖 StartError、EmptyCompletion、Failed、PermissionTimeout、StreamClosed。
- GREEN：`cargo test --locked --lib repair`
  - 结果：15 passed，包含 Story/Design/Work Item 持久化 failure 矩阵、Work Item Plan Outline、PermissionTimeout reload、payload 等价保护与 Abort 路径。
- GREEN 回归：`cargo test --locked --lib reviewer_empty_provider_output_marks_review_node_failed_without_human_confirm`
  - 结果：1 passed，证明首次 Reviewer 空输出仍保持原失败语义。

### Minor 1

- RED：`cargo test --locked --lib missing_or_blank_first_provider_session_id_starts_one_fresh_session_repair`
  - 失败原因：Provider starts 为 1，期望 2。
- GREEN：同命令通过。

### Minor 2

- RED：`cargo test --locked --lib missing_or_unclosed_end_boundary_keeps_full_readable_output`
  - 失败原因：`readable_output` 仅剩“审核说明”，期望完整原文。
- GREEN：同命令通过。
- GREEN 汇总：`cargo test --locked --lib structured_output`
  - 结果：20 passed。

## 最终验证

- `cargo check --locked`：PASS。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：PASS。
- `cargo test --locked --lib`：553 passed，0 failed。
- `cargo fmt --check`：PASS。
- `git diff --check`：PASS。
- coding provider failure 回归：`code_review_provider_failure_blocks_attempt_without_cleaning_shared_worktree` PASS。
- 前端：本任务未修改前端文件，未运行前端测试。
- 行数：所有本任务修改文件均小于 800 行；第三轮 `socket.rs` 723 行、`socket/admission.rs` 90 行、`socket/preparation.rs` 137 行、recovery runner 测试 674 行、WebSocket 集成测试分片 41 行。

## 文件清单

### Commit `58b03e5`

- `src/product/coding_attempt_store/gate.rs`
- `src/product/coding_attempt_store/recovery.rs`
- `src/product/coding_workspace_engine/failed_review_recovery.rs`
- `src/product/coding_workspace_engine/gates.rs`
- `src/web/coding_ws_handler/socket.rs`
- `src/web/coding_ws_handler/tests/failed_review_recovery.rs`
- `src/web/coding_ws_handler/tests/failed_review_recovery/blocked.rs`
- `src/web/coding_ws_handler/tests/failed_review_recovery/runner.rs`
- `src/web/coding_ws_handler/tests/failed_review_recovery/support.rs`

### Commit `9026fcf`

- `cadence/plans/2026-07-10_计划文档_实施计划_共享结构化输出协议与WorkItemPlan返修路由修复_v1.0.md`
- `src/cross_cutting/structured_output.rs`
- `src/product/workspace_engine/prompts/review_repair.rs`
- `src/product/workspace_engine/review/drive.rs`
- `src/product/workspace_engine/types.rs`
- `src/product/workspace_engine/tests/part_03/part_06.rs`
- `src/product/workspace_engine/tests/part_14.rs`

### Commit `b30680c`

- `src/web/state.rs`
- `src/web/coding_ws_handler/socket.rs`
- `src/web/coding_ws_handler/socket/admission.rs`
- `src/web/coding_ws_handler/tests/failed_review_recovery/runner.rs`
- `src/product/workspace_engine/review/drive.rs`
- `src/product/workspace_engine/tests/part_03/part_06.rs`

### Commit `f6cefa2`

- `src/web/coding_ws_handler/socket.rs`
- `src/web/coding_ws_handler/socket/preparation.rs`
- `src/web/coding_ws_handler/tests/failed_review_recovery/runner.rs`
- `tests/it_web/web_coding_ws_handler.rs`
- `tests/it_web/web_coding_ws_handler/part_10.rs`
- `.superpowers/sdd/final-review-fix-report.md`

### 第四轮原子提交（本提交）

- `src/web/state.rs`
- `src/web/coding_ws_handler/socket.rs`
- `src/web/coding_ws_handler/socket/preparation.rs`
- `src/web/coding_ws_handler/tests/failed_review_recovery/runner.rs`
- `src/web/coding_ws_handler/tests/failed_review_recovery/runner/ordinary_mutation.rs`
- `.superpowers/sdd/final-review-fix-report.md`

## 第二轮复审 RED / GREEN

### Reservation 到 journal Prepared 临界窗口

- RED：`cargo test --locked --lib active_recovery_reservation_serializes_all_competing_attempt_messages`
  - 初始失败：缺少 `lock_attempt`、active recovery reservation 查询与统一 admission；证明窗口没有 Attempt 级序列化能力。
- GREEN：同命令通过。
- barrier 证据：获胜 retry 持有 Attempt lock、取得 reservation 后在 journal 未 Prepared 处暂停；五类竞争消息均未完成 admission，Attempt 仍 Blocked、Role Run 仍为 1、journal 不存在、reservation 保持。恢复继续后五条消息均为 `Rejected`，只产生一个 RetryReview Role Run，ContextNote 为空，journal 收敛至 GateResolved。

### Story / Design / Work Item repair failure 强制矩阵

- RED：`cargo test --locked --lib repair_terminal_paths_close_started_event_as_failed`
  - 覆盖计数失败：实际 5，期望 15，证明第一轮只有 Story × 5 terminal path。
- GREEN：同命令通过，现覆盖 Story、Design、Work Item × StartError/EmptyCompletion/Failed/PermissionTimeout/StreamClosed。
- 每个 case 均检查内存与持久化 Session、Review Node、diagnostic，并用 `WorkspaceEngine::new_persistent` reload 再验证。
- Work Item Plan Outline 由 `work_item_plan_outline_repair_provider_failure_safely_degrades` 覆盖；Draft/Batch routing 依据：`work_item_plan_item_review_pass_with_strong_finding_requires_current_item_revision` 与 `work_item_plan_review_revise_batch_maps_to_needs_human_generic_verdict_with_extension`。

### PermissionRequest → PermissionTimeout 持久化

- RED：`cargo test --locked --lib repair_permission_timeout_resolves_persisted_request_before_fallback_reload`
  - 失败原因：NodeDetail 中 permission event 的 response 仍为 `None`。
- GREEN：同命令通过；response 持久化为 `{ "status": "timeout" }`，fallback Review 保持 Completed/HumanConfirm，reload 后无 pending permission。

## 第三轮复审 RED / GREEN

### 生产 recovery guard 释放时点

- RED：`cargo test --locked --lib production_recovery_lifecycle_rejects_competing_abort_and_context_note`
  - 测试先按旧生产顺序执行 lock→reservation→recover→reserved spawn/Completed→释放 lock；竞争 AbortAttempt/ContextNote 取得锁后实际返回 `Allowed`，期望 `Rejected`。
- GREEN：winner 与竞争者均改用生产共用 `prepare_coding_message()` 内核；测试 probe 仅控制 reservation 后的暂停点，不替代 admission/recovery 生命周期。
- guard 释放证据：winner helper 已返回、reserved spawn 尚未执行时，reservation 仍 active，journal phase=`GateResolved`；竞争 AbortAttempt/ContextNote 均为 `Rejected`。
- 完成证据：放行 winner 后真实 `spawn_coding_runner_reserved_with_probe()` 依次记录 `task_created`、`journal_completed`、`provider_entry`，journal 最终为 `Completed`，runner 正常清理。

### Hello/Ping 非变更快路径

- RED：`cargo test --locked --test it_web coding_ws_hello_and_ping_do_not_wait_for_attempt_recovery_lock`
  - 真实 WebSocket 收到初始 snapshot 后，测试持有对应 Attempt lock，依次发送 Hello 与 CodingPing；旧实现因 Hello 等待 lock，100ms 内收不到 Pong。
- GREEN：同命令通过；Hello/Ping 在 preparation helper 获取 lock 前返回，同一 socket 的 Pong 在 guard 被外部持有时仍可立即到达。

### 第三轮回归与门禁

- `cargo test --locked --lib failed_review_recovery`：21 passed。
- `cargo fmt --check`：PASS。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：PASS。
- `cargo check --locked`：PASS。
- `git diff --check`：PASS。

## 第四轮复审 RED / GREEN

### ordinary-first admission → mutation TOCTOU

根因：

- 普通消息在 `prepare_coding_message()` 的 Attempt guard 内完成 reload/admission 后，以 `Allowed` 返回 Attempt 快照并释放 guard。
- socket 随后才执行 Abort、GateResponse、ContextNote 等持久化 mutation；反向 retry 可在这个窗口取得 Attempt guard、建立 reservation并推进 recovery journal，导致两个请求都基于各自旧判断继续。

RED：

- `cargo test --locked --lib ordinary_allowed_mutation_finishes_before_retry_reloads_state`
- 首次编译先发现测试夹具将 `ordinary_event_tx` move 两次；仅补 `clone()` 后重跑，测试稳定失败于 `AbortAttempt: retry crossed Allowed→mutation window`。
- 测试中的 A 使用生产 `prepare_coding_message()` 得到 `Allowed` 后暂停，B 发起真实 retry preparation；旧实现中 B 在 A mutation 前完成，直接证明 ordinary-first 窗口存在。

GREEN：

- `CodingRunRegistry` 新增独立的 Attempt 级 mutation AsyncMutex，`CodingAttemptMutationLease` 持有 `OwnedMutexGuard` 并通过 Drop 自动释放。
- 非 Hello/Ping 消息按 Attempt guard → mutation lease 的固定顺序进入 preparation；在取得 mutation lease 后才 reload/admission，避免 recovery 等待普通 mutation 后继续使用等待前的旧快照。
- 普通 `Allowed` 返回 Attempt 与 RAII lease，并在返回前释放 Attempt guard；socket 仅把 lease 保持到实际 mutation/runner command 完成或失败，所有 snapshot 和 `send_coding_json()` 前均显式释放。
- recovery 在同一顺序下先等待 mutation lease，再 reload/admission、取得 reservation并推进 journal；普通 mutation完成后，retry 必须基于最终状态重新判断。
- Hello/Ping 仍在 Attempt guard 与 mutation lease 之前返回，非变更快路径未退化。

对称线性结果：

- AbortAttempt 先完成：retry 重新读取 `Aborted + CodeReview` 后返回 `Rejected`；无 recovery journal、reservation 或 runner，原 Role Run 数量不变。
- ContextNote 先完成：note 与 chat entry 先持久化，retry 随后恢复；Attempt=`Running + CodeReview`、journal=`Completed`、唯一 RetryReview Role Run、runner 从 1 清理到 0。
- recovery-first 既有生产生命周期测试仍验证 AbortAttempt/ContextNote 均在 reservation/journal 后被拒绝。

### 锁序与 RAII 释放审计

- 固定顺序为 Attempt AsyncMutex → mutation AsyncMutex → registry StdMutex 短锁。
- `lock_attempt()` 与 `lock_attempt_mutation()` 只在 registry StdMutex 内 clone 对应 Arc，释放 StdMutex 后才 await AsyncMutex；不存在持 registry 锁等待 Attempt/mutation 的反向边。
- mutation lease Drop 只释放 Tokio `OwnedMutexGuard`，不获取 Attempt guard或 registry StdMutex；普通 mutation结束不会被正在等待的 recovery 反向阻塞。
- recovery reservation 的创建仍在 Attempt guard 与 mutation lease 内完成；reserved spawn、provider entry、snapshot 与 socket I/O 均不持 Attempt guard或 mutation lease。
- `StartCoding`、`FinalConfirm`、`AbortAttempt`、普通 `GateResponse`、`ProviderSelect`、`PermissionModeSelect`、`MaxAutoReworkSelect`、`StageGateConfirm`、`PermissionResponse`、`ChoiceResponse`、`ContextNote` 全部分支均审计；错误、`continue`、task cancellation 与 unwind 由显式 drop 或 RAII Drop 释放。

### 第四轮测试与门禁

- `cargo test --locked --lib ordinary_allowed_mutation_finishes_before_retry_reloads_state`：1 passed。
- `cargo test --locked --lib failed_review_recovery`：22 passed，包含 ordinary-first、recovery-first、双 socket、journal/reservation 与真实 reserved spawn。
- `cargo test --locked --test it_web coding_ws_hello_and_ping_do_not_wait_for_attempt_recovery_lock`：sandbox 内因监听端口被拒绝；按权限规则在 sandbox 外用同一命令重跑，1 passed。
- `cargo fmt --check`：PASS。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：PASS。
- `cargo check --locked`：PASS。
- `git diff --check`：PASS。
- 行数：`socket.rs` 754、`socket/preparation.rs` 145、`state.rs` 543、`runner.rs` 676、`runner/ordinary_mutation.rs` 274；均低于 800 行。

## Concerns / 未运行门禁

- 第四轮未运行包含所有 integration test targets 的 `cargo test --locked`，由主 Agent 负责完整 integration；本轮运行了 22 个 failed review recovery 单测与 Hello/Ping 定向 integration。
- worktree 仍有任务开始前已存在的未提交改动；这些改动未被 stage、commit、reset、stash、clean 或覆盖。
