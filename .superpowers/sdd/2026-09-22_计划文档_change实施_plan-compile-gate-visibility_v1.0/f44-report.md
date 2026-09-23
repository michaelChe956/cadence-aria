# F-44 复验报告：coding 页中止/失败终态「重新开始」入口

- 日期：2026-09-23
- 实施：F44Impl（worker 子代理）
- worktree：`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo`（基线 HEAD 1534be36；期间并入 SplitGuard3 的 ac15b2d0，文件零交集）
- 结论：**DONE_WITH_CONCERNS**——入口已落地并实测可用；**group 作用域终态 attempt 的「重开可用面」受引擎既有结构门限制，本批 fail-closed 拒绝而非半途崩溃**，需要决策后续（见 §5）。

## 1. 实测根因（三处，非一处）

用户在 attempt `coding_attempt_e4a4aa9d6cb245e9844774392ab820c5` 上看到的「代码编写·用户已中止、页面无任何重试入口」，实测由三层叠加：

1. **前端无入口**：`CodingWorkspaceControls.tsx::ActionButtons` 对终态只走到末尾 `return null`（stage 分支均不匹配），页面仅剩「补充上下文」。
2. **后端守卫拒绝隐式启动**：`is_coding_ws_message_allowed`（socket.rs:988）对 `Completed|Failed|Aborted` 一律 `false` → `coding_message_admission` 返回 `Rejected` → 回帧 `coding_message_not_allowed`。**注意：主代理 brief 中的「无终态拒绝守卫」不成立**——socket.rs:297-373 的 StartCoding 分支之前还有 admission 白名单这一道（实测帧见 §4 红面）。
3. **重开后引擎结构门**（本轮新发现）：即使放行并 CAS 回 Running，runner 会在既有阶段链上立刻死于
   `coding_group_attempt_incomplete: coding_attempt_0001: attempt has no legal active or resume target`
   （`group_validation.rs:438` ← `validate_group_attempt_pointers`：无 active unit 时必须「指针为空 + 全 unit 完成 + stage ≥ ReviewRequest」才算有合法目标）。中止会把非终态 unit 归一为 `Skipped`（`group_terminal.rs:71`），于是 group attempt 中止后**结构上不存在恢复目标**。

现场取证（worktree `.aria`，attempt e4a4…）：`scope=work_item_group`、`status=aborted`、`stage=coding`、`admission_kind=legacy_group`、unit1/2 `completed`、unit3 `skipped`（其 unit run 仍 `running`）、`head_commit` 已落盘。

## 2. 实施（按最小后端守卫 + 显式通道）

**关键设计判断：不复用 `StartCoding` 作为终态重开动词，而是新增显式 `RestartCoding`。** 理由（实测驱动）：

- `tests/it_web/web_coding_ws_handler/part_12.rs` 钉死「终态 group attempt 的 `StartCoding` 必须被拒且零 provider 运行」（F-43 面，commit bd8799eb）。放行 `StartCoding` 必须改写该契约 = 回退 F-43 面；brief 的「F-35~43 面不得回退」与该改法冲突。
- 仓库既有先例正是「终态显式通道」：F-16 为 `awaiting_manual_recovery` 新增 `RecoverCoding` 而非放行 `StartCoding`（socket.rs:1029 注释明确「其余收紧面不动」）。F-44 与之同形。
- 显式动词使「终态不得被隐式唤醒」这一 fail-closed 语义保持，同时审计可辨。

改动点：

| 层 | 文件 | 内容 |
|---|---|---|
| 协议 | `src/web/coding_ws_handler/protocol.rs` | 新增 `CodingWsInMessage::RestartCoding`（F-44 注释） |
| 白名单 | `src/web/coding_ws_handler/socket.rs` | 终态分支改为「仅 `Aborted`/`Failed` 放行 `RestartCoding`；`Completed` 全拒；`StartCoding` 等仍 fail-closed」 |
| SC 门复用 | 同上 | 把 StartCoding 的 sc_advance durable-ready 门提为共用块（`StartCoding`/`RestartCoding` 同款语义；行为零变化，仅去重） |
| 分支 | 同上 | 新增 `RestartCoding` 分支：CAS 重开 → 复用 `spawn_coding_runner` 同款路径 → 回发快照；失败 fail-visible `coding_restart_failed` |
| durable | `src/product/coding_attempt_store/admission.rs` | 新增 `restart_terminal_attempt_for_execution`（仅 `Aborted`/`Failed`；同一 attempt 锁内重走完整 routing/快照/policy 校验 + ticket CAS 回 Running；清除 `completed_at`）；`allow_manual_recovery: bool` → `AdmissionReopen{None,ManualRecovery,TerminalRestart}` 枚举；新增 `ensure_restart_has_resume_target` 前置门（见 §5） |
| 前端 api | `web/src/api/types/coding.ts`、`web/src/hooks/useCodingWorkspaceWs.ts` | `{type:"restart_coding"}` + `restartCoding` 动作（沿用 `start_coding` 审计生命周期，`startupAuditRecordIdsRef` 同一套） |
| 前端 UI | `web/src/pages/CodingWorkspaceControls.tsx`、`web/src/components/chat-workspace/cockpit/ConfirmTwiceButton.tsx` | `ActionButtons` 终态分支（**优先于阶段分支**，覆盖「中止于 prepare_context」旧死按钮形态）：`ConfirmTwiceButton` 两步确认（对齐既有防误触先例），运行中/已完成不渲染；`ConfirmTwiceButton` 增可选 `ariaLabel`/`confirmAriaLabel`（页面操作区 + 底部工具栏双实例可访问名） |

未改：`ChatCockpitPage.tsx`、`workspace-ws-store.rebuild.test.ts`（拆分 peer 独占）；未新增 wire 之外的 REST。

## 3. 测试证据（全绿，含红绿）

前端（`cd web`）：
- `npx tsc -b` → 0。
- `npm test` → **183 文件 / 1710 用例全绿**（基线 1705，+5）。
- 新增 `src/pages/CodingWorkspacePage.restart-terminal.test.tsx`（4 例）：中止态渲染「重新开始」、**只点一次不发**、点第二次发 `restartCoding`；失败态渲染；running / completed 均不渲染。
- `useCodingWorkspaceWs.actions.test.tsx` 增 1 例：`restartCoding()` 实发 `{"type":"restart_coding"}` 且落 `start_coding` 审计记录。
- **红面实测**：临时还原 `CodingWorkspaceControls.tsx` 到 HEAD 后重跑 → 中止/失败两例 FAILED（running/completed 两例本就无按钮，绿）；还原即绿。

后端：
- `cargo clippy --all-targets --all-features --locked -- -D warnings` → 0。
- `cargo test --locked --lib` → 全绿（含新增 4 例 store 级：`terminal_attempt_restarts_through_explicit_restart_channel`、`restart_channel_rejects_non_terminal_sources`、`general_admission_still_rejects_terminal_attempts`、`recovery_channel_rejects_terminal_sources`）。
- `cargo test --locked --test it_web web_coding_ws_handler` → **70 passed / 0 failed**（含原 part_12 终态 StartCoding 拒绝面零回归）。
- 新增 `tests/it_web/web_coding_ws_handler/part_19.rs`（2 例）：
  - `aborted_work_item_attempt_restart_coding_reopens_and_resumes_runner_pipeline`：work_item 作用域中止 → `restart_coding` → 无拒绝帧、runner 推进到 Coding 阶段门、durable 回 Running 且 `completed_at=None`、确认阶段门后 coder 角色运行重新落地。
  - `aborted_group_attempt_restart_coding_is_refused_without_state_write`：group 中止且无 resume target → `coding_restart_failed: attempt_restart_no_resume_target`，**状态保持 Aborted、version 不变、零角色运行**（不产生「先重开再崩」的中间态污染）。
- **红面实测**：临时还原 socket.rs 白名单后重跑 → 两例均 FAILED，帧为 `coding_message_not_allowed: message is not allowed in current coding stage`（即 F-44 现场根因）。
- `cargo test --locked --test it_core large_file_guard` → ok（拆分后 `src/**` 全部 ≤1200 行）。

拆分（SplitGuard3 提示后自查）：新增测试顶过 1200 硬线的两个 rust 文件已拆回——`src/product/coding_attempt_store/admission_tests.rs` 1169（新增 F-44 用例移入子模块 `admission_restart_tests.rs`，110 行）、`src/web/coding_ws_handler/tests.rs` 1166（新增白名单用例移入 `tests/restart.rs`）。

## 4. 交付面（用户可见行为）

- 终态（`aborted`/`failed`）页面**页头操作区 + 底部工具栏**出现「重新开始」按钮（两步确认）；`running`/`waiting_for_human`/`completed` 不出现。
- 点击 → `restart_coding` → 后端重验 routing/快照/policy 后 CAS 回 Running → runner 从当前 stage 续跑（work_item 作用域已端到端实测）。失败一律回 `coding_protocol_error`，页面 statusText 可见（不静默）。
- `StartCoding` 对终态**仍拒绝**（part_12 契约零回归）；`RecoverCoding`/`awaiting_manual_recovery` 面零改动。

## 5. Concerns（需决策）

1. **group 作用域中止终态的「重开可用面」结构性受限（主要遗留）**：中止会把 unit 归一为 `Skipped`，重开后 `validate_group_attempt_pointers` 无合法目标 → 本批**fail-closed 拒绝**（实测：状态不被改写，不会退化成人工恢复态）。要真正放行现场 attempt e4a4… 这类 group 中止，需要「恢复 group resume target」的真实特性：把首个非 `Completed` unit 复位为 active（并清 `completed_at`）、对齐 `active_unit_id`/`current_work_item_id`、保证该 unit 有 active unit run（现场 unit3 的 run 仍是 `running`，可复用；无 run 时需 `create_retry_coding_unit_run`/启动 run）——涉及 `update_coding_unit_status` 语义扩展与 unit run/handoff 不变量，超出「最小守卫」，需独立立项 + 你的授权（本次未自授权实施）。
2. **UI 与可用面的一致性**：按钮在所有终态都渲染，group 且无 resume target 时点击会得到明确错误（fail-visible）。若你希望「不可重开就不显示入口」，前端可从快照的 `attempt_scope`+`units` 自行判定（本批未做，避免入口静默消失掩盖 F-44 本意）。
3. **审计动词**：`restart_coding` 复用既有 `start_coding` 审计 operation（同一用户意图 + 同一拒绝/完成生命周期，零新增审计词汇）；如需审计区分重开与首启，需扩 `CockpitOperation` 并同步拒绝 lifecycle 过滤面。
4. **未改的相邻缺陷**：`CodingComposer` 的 `inputDisabled` 只含 `completed`/`aborted`，`failed` 态仍可输入「补充上下文」，而后端对终态 `ContextNote` 拒绝——同一「终态提供不可能动作」家族，属 F-43 相邻面，本批未动（避免越界与既有用例churn）。
5. **未验证项**：真浏览器视觉复验未做（本次以组件测试 + 真实 WS 集成测试为准）；现场 attempt e4a4… 的实际重开结果**不可用**（落在 concern 1），因此**报告不声称现场 attempt 已能重启**。
