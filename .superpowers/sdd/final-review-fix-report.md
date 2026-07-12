# 最终整分支审查修复报告

## 状态

- 结果：DONE_WITH_CONCERNS
- 实现提交：
  - `58b03e5 fix: serialize interrupted code review retries`
  - `9026fcf fix: safely degrade structured review repair`
  - `b30680c fix: close recovery admission races`
- 工作目录：`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0709`
- 未触碰真实 `coding_attempt_0001`，未访问或修改共享 issue worktree。
- dirty worktree 中原有 Workspace 中断恢复、结构化审核及前端改动均保留，三个实现提交只包含本任务文件。

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
- admission、reservation 与 journal 推进现在由同一 Attempt 级异步临界区串行化；active recovery reservation 在 journal 创建前即拒绝其他 mutating message。
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
- 可控双 barrier 测试证明 AbortAttempt、错误 Gate、`manual_continue`、`send_to_coder`、`ContextNote` 在 reservation 已建立但 journal 未 Prepared 时全部阻塞；journal 推进后全部 Reject，且 reservation 不丢失。

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
  - 结果：21 passed，包含旧 Runner、双 socket、mixed-message barrier、reserved spawn、journal 完成顺序与新旧多前缀幂等收敛。

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
- 行数：所有本任务修改的 Rust 文件均小于 800 行；本轮 `socket.rs` 拆分后 724 行，`socket/admission.rs` 90 行，最大本轮测试文件 626 行。

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

## Concerns / 未运行门禁

- 未运行包含所有 integration test targets 的 `cargo test --locked`；已运行完整 `cargo test --locked --lib`（553/553）及所有新增/受影响定向测试。
- worktree 仍有任务开始前已存在的未提交改动；这些改动未被 stage、commit、reset、stash、clean 或覆盖。
