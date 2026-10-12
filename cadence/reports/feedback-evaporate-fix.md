# 反馈蒸发链修复报告：反馈轮终态重放回显化与 command_id 终态不复用

- 日期：2026-10-13
- 诊断输入：`local://diag-manual-revision-evaporate.md`（workspace_session_0003 / turn 71b47478，修法①②③+顺手项冻结）
- 分支/worktree：`feat-b-0808-add-monorepo`
- 提交：`fix: 反馈轮终态重放回显化与 command_id 终态不复用`

## 修复内容

### ① 前端根修：command_id 复用仅限非终态活 turn

- `web/src/state/cockpit-action-routing.ts`
  - 新增导出策略 `replayableGateCommandId(state)`：仅当 `humanGateTurn.status ∈ {"open","busy"}`（↔ 服务端 Reserved/Running）才返回其 `command_id`；`"failed"` / `"awaiting_confirm"`（↔ Failed/Completed 终态）返回 `null`。
  - `feedback()` / `terminate()` 改为 `replayableGateCommandId(state) ?? newCommandId()`——终态 turn 的 command_id 已被 durable 查重消费，复用必被吞（现场第二次提交零 spawn、零回显），必须新 id 开新回合。点击时从 store 现读，不吃 memo 闭包过期。
  - facade 输入参数 `commandId: string | null` 整体退役（与 getState 派生重复的数据流），三处调用方（`ChatCockpitPage.tsx`、`useCockpitHotkeyActions.ts` ×2）同步收口，单一策略点杜绝后续接线回退。
- 语义影响：`terminate`（abandon）与 feedback 同款纪律；abandon 服务端本无 command_id 查重，行为不变，仅统一复用口径。

### ② 服务端兜底：终态重放 typed Rejected 回显

- `src/product/workspace_engine/conversational_gate.rs`
  - 新增稳定码 `HUMAN_GATE_COMMAND_REPLAYED_AFTER_TERMINAL_TURN` 与分派 `replay_outcome_for_command_turn`：按 `(session_id, command_id)` 查重命中**终态**（Failed/Completed）turn → `Rejected{code, reason(含 turn_id)}`；非终态（Reserved/Running）保持 `Replayed`（重连重放幂等，回显 TurnOpen）。
  - 两处接线：入站查重点（handle_human_gate_feedback 开局）与 CAS 竞争和解路径（compare_and_reserve 失败后按 command_id 对账）。
  - 回显通路：WS 侧 `decisions.rs` 既有 `Rejected → ProtocolError` 帧（聊天流错误条目，任何客户端可见）；REST 侧 `workspace_human_action.rs` 既有 `Rejected → 422 ApiError(rejection_code)`。蒸发变回显，零新增 wire 类型。
  - 拒收零副作用（不落 turn、不动预算/ledger），回归断言保持。

### ③ REST 反馈 spawn 的丢弃通道回显化

- `src/web/handlers/workspace_human_action.rs`
  - `let (sink_tx, _sink_rx) = channel(1)`（接收端立即 drop 的静默丢弃通道）替换为 `rest_feedback_outbound_drain(session_id)`：常驻 drain 任务逐帧 `eprintln` 打点（对齐 `[aria-cancellation]` 风格，前缀 `[aria-rest-feedback] ... outbound_dropped`），run 内 `HumanGateTurnFailed` 等错误帧不再蒸发；通道也不再因容量 1 反压 run。观察者侧事实仍由 event-router 的 EngineEvent 广播承载。

### 顺手项

- **attempt_no 单源化**（conversational_gate.rs）：预约侧 `attempt_no: 1` 与幂等键 `human_gate:{turn_id}:attempt:1` 两个散落字面量收敛到 `HUMAN_GATE_TURN_RESERVATION_ATTEMPT_NO`（行为零变化；attempt 语义为 turn 内 provider 启动序数，恢复链可续跑 attempt 2，不与门内反馈轮序数混淆——受 `HUMAN_GATE_PROVIDER_MAX_ATTEMPTS=2` 与 store 校验约束，不能改为门内轮数）。
- **turn 失败后 author 节点收口**（conversational_gate.rs `close_revision_author_node_after_turn_failure`）：`fail_human_gate_turn` 真实迁移到 Failed 时，若活动节点是修订 author 载体（AuthorRun）则同步置 Failed（与成功路径 `revision_turn.inc.rs` 的 `complete_active_node` 同构），消除时间线幻影（现场 node_005 永停 Active）。门节点载体保持 Active（门仍开着，收口归 enter_human_confirm/终态 compile）。
- **select! 取消臂回显化**（`src/web/workspace_ws_handler/run/provider_run.rs`）：
  - 新增 `fail_cancelled_human_gate_revision_turn`（run.rs re-export）：durable `Failed(cancelled)` + `HumanGateTurnFailed` 回显帧 + `[aria-cancellation]` 打点；返回是否真实迁移，刚自然收口的 turn 幂等跳过（不回显误导帧）。
  - 取消臂接线：drive 被 drop 时先打点 `provider_run_cancelled_dropped_drive`，SC 修订 turn 经助手终态化+回显，再走既有 `finish_run` 回收。同时解开一个潜在死锁面：turn 永停 Running 会把后续 feedback 全卡成 gate_busy。
  - `fail_human_gate_turn` 签名 `Result<(),String> → Result<bool,String>`（true=真实迁移）；既有 `let _ =` 调用点零改动。
  - `HumanGateTurnFailureClass` 新增 `Cancelled` 变体（serde `"cancelled"`，前端 failure_class 字符串直收）。

## TDD 过程（红→绿→回归）

- 红：
  - 前端：`cockpit-action-routing.test.ts` 复用类断言 3 failed（open/busy 复用、terminate 复用——facade 旧实现无状态感知）。
  - Rust：8 个编译红（缺 `Cancelled` 变体、缺 `fail_cancelled_human_gate_revision_turn`、缺 `rest_feedback_outbound_drain`、provider_run 模块私有）。
- 绿（新增测试，8 过）：
  - `conversational_gate_revision/part_07.rs`：Failed 终态重放→typed Rejected（码+reason 含 turn_id+零副作用）；Running 重放保持 Replayed（对照锚）；Completed 终态重放→typed Rejected；turn 失败收口 author 节点（Active→Failed+completed_at）。
  - `tests/human_gate_revision_cancel.rs`：取消→durable Failed(cancelled)+`human_gate_turn_failed` 回显帧+author 节点收口；终态幂等 no-op（不回显、不改原 failure_class）。
  - `workspace_human_action.rs` 内嵌：drain 连续消费 3 帧（容量 1，无 drain 必挂起）。
  - `models/tests.rs`：`Cancelled` serde roundtrip + `"cancelled"` 枚举钉死。
- 回归：
  - Rust `cargo test --lib` 全量：**4435 passed, 0 failed**（含 conversational_gate / human_gate / campaign_stage3 / group_amendment / recovery / protocol 全族）。
  - 4 个既有测试按冻结语义更新（终态重放 Replayed→Rejected 断言）：`campaign_stage3_amendment.rs`、`campaign_stage3_interactive/cases.rs`（Failed 口味循环）、`campaign_stage3_recovery_matrix/thin_rows.rs`、`amendment_row.rs`——四处零增量断言（预算/ledger/turn 数）原样保留。
  - 前端：`cockpit-action-routing.test.ts` 44 过（新增 open/busy 复用与 failed/awaiting_confirm 再生成 `it.each`）；`workspace-ws-store`+`gate-convergence` 33 过；`ChatCockpitPage.gate/confirm/inbox` 67 过；`tsc -b` 零错误；全量 vitest 见提交前核验。

## 行为对照（修复前后）

| 场景 | 修复前 | 修复后 |
|---|---|---|
| 前端 failed turn 上再点「提交反馈」 | 复用死 command_id → 服务端 Replayed → 同 turn TurnOpen → 客户端幂等吞帧 = 无声蒸发 | 新 command_id → 新 turn + 修订 run 正常 spawn |
| 任一客户端终态 turn 重放（兜底） | Replayed（零回显） | `HUMAN_GATE_COMMAND_REPLAYED_AFTER_TERMINAL_TURN`（WS ProtocolError / REST 422 可见） |
| Reserved/Running 重放（重连重发） | Replayed（TurnOpen 幂等） | 不变（Replayed） |
| REST 反馈 run 内错误帧 | 丢弃通道静默蒸发 | `[aria-rest-feedback]` 逐帧打点（+durable/engine 事件既有承载） |
| 修订 run 被取消（supersede/断连） | turn 永停 Reserved/Running，零落盘零回显，后续 feedback 全 Busy | durable Failed(cancelled) + HumanGateTurnFailed 回显 + 打点 |
| turn 失败后的 author 节点 | 永停 Active（时间线幻影） | 置 Failed + completed_at（与成功路径同构） |
| 每个新 turn 的 attempt 显示 | 字面量 1 双处散落 | 单源常量（语义不变，防漂移） |

## 风险与边界

- ②的语义变更是有意为之（冻结修法）：Completed turn 的同 command 重发从幂等 Replayed 变为可见拒收——前端 ① 已保证正常路径不复用终态 id，兜底拒收只影响陈旧/异常客户端，且回显可自解释（reason 指明 turn_id 与「换新 id 重发」）。
- 取消臂终态化可能与 drive 自然收口竞态：`fail_human_gate_turn` 幂等守卫（终态 no-op、不回显）覆盖；CAS 冲突错误路径仅打点不重试（服务重启后恢复链 `recover_human_gate_turns` 仍可对非终态 turn 兜底）。
- `stale human_gate_reservation`（指向终态 turn）诊断中已判明不阻塞新 CAS，未动。
