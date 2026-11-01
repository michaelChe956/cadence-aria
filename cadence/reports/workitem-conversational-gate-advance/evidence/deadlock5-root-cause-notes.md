> 第 5 死路根因侦察(scout,2026-09-03):真实根因=run_sc_manual_revision_turn 后缺 Evaluate policy route;现场 turn 的 result_artifact_ref=artifact_version_004(非空,此前 controller 读错字段);修复切口与测试建议见内。

# Code Context

## 结论摘要

**根因不是 provider 死锁，而是人工修订完成后的持久化/路由断链：`complete_human_gate_revision` 将候选 source/IR/report 和 turn 写成完成，但没有把 session 的 `single_candidate_phase` 从 `Evaluate` 推到 `Approval`，也没有再次调用 Evaluate 路由。随后 Confirm 只能命中 `human_gate_close` 的 phase CAS 冲突。**

## Files Retrieved

1. `.worktrees/feat-b-0808-add-monorepo/src/product/workspace_engine/conversational_gate.rs:428-516` - 人工修订编译、artifact 生成、turn 完成和返回路径。
2. `.worktrees/feat-b-0808-add-monorepo/src/product/lifecycle_store/workspace_single_candidate.rs:323-414` - `complete_human_gate_revision` 的持久化顺序及字段更新。
3. `.worktrees/feat-b-0808-add-monorepo/src/product/lifecycle_store/workspace.rs:590-634` - human-gate close 的前置状态和 CAS 清理。
4. `.worktrees/feat-b-0808-add-monorepo/src/product/workspace_engine/review/routing_scope.rs:413-454,534-565` - Evaluate policy route 与无 reviewer 路由。
5. `.worktrees/feat-b-0808-add-monorepo/src/product/workspace_engine/single_candidate.rs:41-70,145-260` - 初始 author 的生成→Evaluate→机械报告→路由链。
6. `.worktrees/feat-b-0808-add-monorepo/src/web/workspace_ws_handler/run/provider_run.rs:954-973,1059-1094` - provider 启动前 Running 标记和人工 turn 完成事件。
7. `.worktrees/feat-b-0808-add-monorepo/src/product/workspace_engine/conversational_gate_recovery.rs:146-199,205-288` - 重连补 artifact 的严格条件和 attempt 上限。
8. `.worktrees/feat-b-0808-add-monorepo/src/web/workspace_ws_handler/socket.rs:40-59,458-517` - 95s idle close 与 human-gate recovery 调度。
9. `.worktrees/feat-b-0808-add-monorepo/.aria/.../workspace_session_0120.json` - 现场 durable session。
10. `.aria/.../human_gate_turn_bd2a11dc-a2ec-405c-9278-8867cfde58c6.json` - 现场 durable turn。
11. `/tmp/aria-stage35-20260903/matrix/codex/rep1/ws.jsonl:12886-12892` - confirm 冲突和最终断连证据。

## Key Code

### 1. `human_gate_turn_completed` 写入链与 artifact ref

WebSocket provider 分支先调用 `mark_human_gate_turn_running`，该方法读取 session/turn，将 `Reserved→Running`，经 `update_human_gate_turn` 写回（`conversational_gate.rs:189-214`）。provider 输出成功后调用 `run_sc_manual_revision_turn`；成功结果才发送 `WsOutMessage::HumanGateTurnCompleted { artifact_ref }`（`provider_run.rs:1059-1067`）。

`run_sc_manual_revision_turn` 的完整链为：解析/编译 source、写 source revision/IR/mechanical report（`conversational_gate.rs:371-427`），构造新 artifact version，调用 `complete_human_gate_revision`（`428-473`），再更新 timeline artifact 并返回 `Accepted { artifact_ref }`（`474-515`）。

`complete_human_gate_revision` 在锁内先校验 expected session 和现有非终态 turn（`workspace_single_candidate.rs:360-392`），写 artifact versions（`394-397`）、写 session 的 source/IR/report refs（`398-406`），随后将 `turn.result_artifact_ref = Some(...)` 并写 turn（`407-413`）。因此正常成功链会同时得到 `status=completed` 和非空 `result_artifact_ref`。

**现场与用户描述不符（红）：用户称字段 `artifact_ref=None`，但模型字段实际名为 `result_artifact_ref`（`models/human_gate.rs:23-45`）；现场 turn 是 `status=completed` 且 `result_artifact_ref="artifact_version_004"`，并非 None。**

若出现 completed+None，正常函数没有该分支：崩溃窗口会在 session/artifact 写后、turn 写前留下旧 turn（通常仍 Running），而不是可靠地产生 Completed+None；这应按 legacy/corruption 现场处理，现有 recovery 只自动补 `Running`（`conversational_gate_recovery.rs:245-258`），不会修补已 Completed 的空 ref。

### 2. `single_candidate_phase=evaluate` 的推进者、provider 依赖与零流量

初始 author 完成后，`complete_single_candidate_work_item_plan_author` 已写 source/IR，再写 mechanical report 并通过 `compare_and_save_single_candidate_evaluation`；后者要求 expected phase=Evaluate 并只写 report ref（`single_candidate.rs:186-240`; `workspace_single_candidate.rs:418-474`）。随后：无 reviewer（`review_rounds==0 || reviewer_provider.is_none()`）调用 `route_single_candidate_evaluate_without_reviewer`，构造本地 synthetic Pass，再走 `work_item_policy_action`（`single_candidate.rs:251-259`; `routing_scope.rs:534-553`）。这段机械 Evaluate 本身不启动 provider。

有 reviewer 时，初始链在 Evaluate 后 `start_review()`，若进入 CrossReview 才 `request_provider_run(ReviewOnly)`（`single_candidate.rs:251-258`）；follow-up macro 才实际消费 CrossReview provider（`run/followups.rs:465-494`）。所以 Evaluate 不必然依赖 provider；provider 只属于 author 生成或 reviewer 评审。

**本次真正断点：** 人工修订的 `run_sc_manual_revision_turn` 虽然重新编译并写了 mechanical report，却在 `complete_human_gate_revision` 后直接返回 `Accepted`（`conversational_gate.rs:428-515`），没有重新调用 `route_single_candidate_evaluate_without_reviewer` 或 `start_review/request_provider_run`。因此 session 停在 Evaluate，provider run 结束，后续没有 provider traffic；这解释了“零流量”。

### 3. reservation 清理与 CAS 冲突因果链

新反馈在 `compare_and_reserve_human_gate_turn` 锁内一次性扣 `manual_repairs_remaining`、写 `human_gate_reservation`、追加 provider-start ledger，再写 turn（`lifecycle_store/human_gate.rs:97-189`）。reservation 是 command 的幂等锚点，模型注释明确它会保留在 session（`models/human_gate.rs:47-57`）；turn 失败或完成路径不会清掉它。

清理仅在 `compare_and_save_human_gate_close` 的 Confirmed/Terminated 分支，写目标 status 同时清 snapshot 与 reservation（`workspace.rs:623-634`）。close 入口先要求 durable `WaitingForHuman` 且 `single_candidate_phase=Approval`（`workspace.rs:604-611`）。

因此现场因果链是：人工修订完成 → turn completed/ref 已写，但 session 仍 Evaluate、reservation 仍在 → Confirm 进入 close → phase 前置校验直接返回 `product_store_conflict: human_gate_close`（甚至尚未进入锁内 reservation 清理）→ session 继续 waiting_for_human；现场日志正是该序列（`ws.jsonl:12886-12892`）。这不是 reservation 自身死锁，而是 stale phase 使清理不可达。

### 4. 三个现象是否同一事务链

**部分相关但不能合并为一个“artifact_ref=None 事务”：**

- `Evaluate 卡死` 与 `reservation 残留` 是同一业务链的后果：同一个人工修订成功调用写入 turn/session，随后缺失 route；phase 没到 Approval，close CAS 不可达，reservation 不能清。
- 现场的 `completed + result_artifact_ref=None` 不成立；实际 ref 为 `artifact_version_004`。即使真的存在 completed+None，也不是当前正常写入链的结果，且 recovery 的补偿只看 Running（`conversational_gate_recovery.rs:245-258`），需要单独的 fail-closed/repair 设计。
- `complete_human_gate_revision` 并非严格单文件事务：artifact versions、session、turn 是顺序写入并带部分回滚（`workspace_single_candidate.rs:394-411`）。但当前现场 ref 非空，说明该链已成功完成 turn 写；真正遗漏是之后的 Evaluate policy route。

### 5. 最小修复切口与测试/连接策略

最小代码切口应在 `run_sc_manual_revision_turn` 成功持久化后补“与初始 author 完全相同的 Evaluate route”：

1. 以 `saved` 刷新 engine session（现有 `conversational_gate.rs:505`）。
2. 触发 mechanical Evaluate 的 policy 路由：无 reviewer 走本地 synthetic Pass；有 reviewer 则启动 reviewer review（应复用 `start_review` 与 `request_provider_run(ReviewOnly)` 的既有条件，而不是让 close 直接绕过 Approval）。
3. 仅 route 成功进入 interactive Approval 后，Confirm 才调用 close CAS；close 成功才清 reservation。

失败先行回归测试建议放在现有 `src/product/workspace_engine/tests/conversational_gate_revision.rs` 或 `src/web/workspace_ws_handler/tests/campaign_stage3_interactive/cases.rs`：构造 Approval gate、打开并 Running turn，调用有效 `run_sc_manual_revision_turn`，断言 durable turn Completed 且 `result_artifact_ref.is_some()`；随后断言 durable session 已到 `Evaluate` 后又经 route 到 `WaitingForHuman + Approval`，最后 Confirm 得 `Confirmed + Completed` 且 `human_gate_reservation.is_none()`。另加一个“provider 不可用/route persistence conflict”失败用例，断言不误清 reservation、session 保持可重试而非假终态。

95s 空闲断连和重连上限：默认 idle 实际是 90 秒（`socket.rs:504-517`; `test_controls/socket.rs:154-160`），现场约 95 秒关闭是 5s tick 的观测误差，不是“95s 业务超时”。idle 仅在 `!is_active_run()` 时关闭；本次 provider 已结束且 phase 卡住，故会断开。**最小修复不应先改 timeout**，应修 phase route。重连上限已有固定 `HUMAN_GATE_PROVIDER_MAX_ATTEMPTS=2`（`conversational_gate.rs:17-21`），Running 且 provider 死亡最多 resume 一次，超过后 ProviderErr；Reserved 恢复 attempt 1（`conversational_gate_recovery.rs:23-67`）。除非产品要求无限/更多重试，否则无需改；建议保留上限并为“phase Evaluate + completed turn + reservation”增加重连回归，确保恢复不重复启动 provider。

## Architecture

`WS inbound confirm/feedback` → `WorkspaceEngine` → `LifecycleStore` CAS；feedback reservation 扣预算并落 turn → provider task 将 turn Running → SC 编译及 source/IR/report/artifact 持久化 → turn Completed 事件。正常初始 author 之后另有 mechanical Evaluate policy route；**人工 revision 路径遗漏这一步**。因此当前 durable 状态同时呈现 completed turn、Evaluate session、未清 reservation；Confirm 的唯一合法入口要求 Approval，最终冲突并保持 waiting_for_human。

## Start Here

先打开 `src/product/workspace_engine/conversational_gate.rs:428-515`，对照 `src/product/workspace_engine/single_candidate.rs:186-259`。前者是复现断点，后者是应复用的生成后 Evaluate 路由模板。
