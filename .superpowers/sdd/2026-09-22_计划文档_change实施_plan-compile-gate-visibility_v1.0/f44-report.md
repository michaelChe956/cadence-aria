# F-44 复验报告：coding 页中止/失败终态「重新开始」入口

- 日期：2026-09-23
- 实施：F44Impl（worker 子代理）
- worktree：`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo`（基线 HEAD 1534be36；期间并入 SplitGuard3 的 ac15b2d0，文件零交集）
- 结论：**DONE**——入口已落地并实测可用；**group 作用域终态的 resume target 恢复已按用户裁决实施（§6）**，含现场形态（unit 部分已完成 + 中止归一的 Skipped unit）的端到端实测。

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

## 5. group 作用域 resume target 恢复（用户裁决后实施）

授权内容：①`restart_terminal_attempt_for_execution` group 分支复位首个非 Completed unit 为 active、清其 `completed_at`、对齐指针、保证该 unit 有 active unit run；②`ensure_restart_has_resume_target` 由 fail-closed 前置门改为执行恢复；③`update_coding_unit_status` 语义扩展与 unit run/handoff 不变量保持；④TDD；⑤全门禁；⑥独立 commit；⑦报告追加。

实施与设计取舍：

- **执行顺序 = 先复位 unit、后 CAS 重开**。原因：`update_coding_unit_status` 自身要取 attempt 文件锁，而 `with_exclusive_lock` 是 flock（同进程同文件不可重入），不能嵌在 CAS 锁内；且「复位成功但 CAS 失败」时 attempt 仍在终态，下一次重开可自愈（反向顺序会留下 Running + 无 active unit 的不可恢复态）。socket 分支里 `restart_terminal_attempt_for_execution` 与 runner spawn 仍共用 `prepare_coding_message` 的 attempt 锁与 mutation lease，消息级并发不变。
- **复位范围刻意收窄到 `Skipped`/`Failed`**（不是字面「首个非 Completed」）：中止会把所有 active unit 归一为 `Skipped`（`group_terminal.rs:71`），终态 attempt 上可复位的只可能是这两者；`Superseded`（被计划修订取代）、`BlockedByPlanDefect`、`AwaitingAmendment` 一律不动，避免重开绕过计划修订/重验语义；`Completed` 不动（TDD 断言）。
- **unit run 不新建不改写**：`active_unit_run_projection`（plan_defect.rs:549）在 `get_active_unit_run` 返回 NotFound 时，会按既有链路（plan lineage/revision/projection bundle/handoff 解析）物化一个 `Running` run；现场 unit3 的 stale `running` run 直接被复用。因此恢复只做「unit 状态 + attempt 指针」，不碰 handoff/plan binding/advance 记录。
- **`update_coding_unit_status` 语义扩展**：进入终态仍写 `completed_at`；**离开终态（Running 等）现清除 `completed_at`**——否则会产生「Running + completed_at」矛盾记录。既有调用方只把 unit 推向终态或 `Blocked`（active、本就无 `completed_at`），行为零变化。
- **恢复前的纯内存预演**：先在内存里把 unit 集与 attempt（status=Running + 指针对齐）改好，跑 `ensure_restart_has_resume_target` 复核；不通过则不做任何写盘（保持终态）。写盘只发生一次 `update_coding_unit_status` + 一次 CAS。

新增/变更测试证据：

- `tests/it_web/web_coding_ws_handler/part_19.rs::aborted_group_attempt_restart_coding_restores_resume_target`（红→绿）：fixture 先 `update_coding_unit_status(unit1, Completed)` 再 abort（对齐现场 unit1 已完成 + 末 unit 被归一为 Skipped 的形态），随后 `restart_coding` → 断言 ①回发 Running 快照 ②unit1 仍 `Completed`（不被复位）③unit2 复位为 `Running` 且 `completed_at=None` ④`active_unit_id`/`current_work_item_id` 对齐 unit2 ⑤attempt `Running` 且 `completed_at=None` ⑥阶段链推进到 Coding 阶段门 ⑦确认阶段门后 coder 角色运行重新落地。
- 回归：`part_19` work_item 路径用例（单会话重开端到端）、`part_12`（终态 StartCoding 仍拒绝）、`it_web web_coding_ws_handler` 全量、`--lib` 全量、`it_core` large_file_guard、clippy/fmt、npm/tsc 全绿（见交付摘要）。
- 观察到的 runner 死亡路径（前一版 fail-closed 前）：group 无目标时 runner 死于 `coding_group_attempt_incomplete` 并被推进为人工恢复态——这正是本次恢复要消除的形态，现已由阶段门/角色运行推进取代。

## 6. Concerns（需决策）

1. **group 作用域 resume target 已实施（用户裁决后，见 §5）**：中止会把非终态 unit 归一为 `Skipped`，重开后 `validate_group_attempt_pointers` 无合法目标 → 现由 `restore_group_resume_target` 在 CAS 前把首个 `Skipped`/`Failed` unit 复位为 active（`Running`）、对齐 `active_unit_id`/`current_work_item_id`、清 `completed_at`。实测（含现场形态 fixture）重开后阶段链推进到 Coding 阶段门且 coder 角色运行重新落地。
   - **仍未直接验证的点**：现场 attempt e4a4 本身未在真实 workspace 上执行重开（本批验证在 fixture 层，含同形态 unit 布局；现场数据另有 unit3 的 stale `running` unit run 可复用、WI-003 handoff 归属等真实细节）。建议主代理在 v 部署后对 e4a4 做一次真实点击复验。
2. **UI 与可用面的一致性**：按钮在所有终态都渲染，group 且无 resume target 时点击会得到明确错误（fail-visible）。若你希望「不可重开就不显示入口」，前端可从快照的 `attempt_scope`+`units` 自行判定（本批未做，避免入口静默消失掩盖 F-44 本意）。
3. **审计动词**：`restart_coding` 复用既有 `start_coding` 审计 operation（同一用户意图 + 同一拒绝/完成生命周期，零新增审计词汇）；如需审计区分重开与首启，需扩 `CockpitOperation` 并同步拒绝 lifecycle 过滤面。
4. **未改的相邻缺陷**：`CodingComposer` 的 `inputDisabled` 只含 `completed`/`aborted`，`failed` 态仍可输入「补充上下文」，而后端对终态 `ContextNote` 拒绝——同一「终态提供不可能动作」家族，属 F-43 相邻面，本批未动（避免越界与既有用例churn）。
5. **未验证项**：真浏览器视觉复验未做（本次以组件测试 + 真实 WS 集成测试为准）；现场 attempt e4a4… 未在真实 workspace 上实际点击复验（fixture 层已覆盖同形态 unit 布局，见 §5 与 concern 1）——建议部署后由主代理做一次真实点击复验。

## 7. fix1（F-44 审查回执：1×P1 必修 + 2×P3）

P1（必修，多 remainder 死胡同）：原 `restore_group_resume_target` 只复位**首个** Skipped/Failed unit，而 `group_terminal` 归一化会把**全部**非终态 unit 变 Skipped/Failed——中止在第 k<n 个 unit（主流形态）时，继任选择器只认 `Pending`，unit_{k+1..n} 被永久放弃、final confirm 永不满足、非终态又拒 `RestartCoding` = 死胡同。

- 修法（k3 ②完整版）：新增 `CodingExecutionUnitStatus::is_group_remainder_candidate()`（`Pending | Skipped | Failed`，`Superseded`/`Completed` 永不为候选），并让**两个**继任选择器统一走该谓词：`advance_to_next_group_unit`（legacy 分支）与 `select_next_sc_group_unit`（SC 分支，依赖就绪判定与 Pending 候选同构，未改语义）。复活 unit_k 完成后，选择器按同一谓词接续复活 unit_{k+1}，链路仍是既有 `start_pending_coding_unit_run`（run 为 Running 直接复用 / Pending 翻 Running / 无 run 时由 `active_unit_run_projection` 既有物化），全程不新建/不改写 run。
- 红→绿实测：新增 `tests/it_web/web_coding_ws_handler/part_19.rs::group_remainder_selector_resumes_skipped_successor_unit`——fixture 置 unit1 `Completed` + unit2 `Skipped`（中止归一形态），直接驱动 `advance_to_next_group_unit`，断言 unit2 被接续复活为 `Running` 且 `active_unit_id` 对齐。**红**：把谓词临时退回 `Pending` 后该用例 FAILED（`Skipped remainder 必须被继任选择器接续复活`）；**绿**：谓词在位即 PASS。

P3a（防御纵深）：`restart_terminal_attempt_for_execution` 开头补**只读终态预检**（复用函数首行已取的快照，零额外读盘）——非终态调用在任何写盘之前即拒，不再出现「先落 unit 复位、再在 CAS 阶段失败」。CAS 锁内的终态校验保留为并发兜底。

P3b（注释如实）：`update_coding_unit_status` 的 `completed_at` 清除分支注释改写为如实列举三类触发方——①本批终态重开复位 Skipped/Failed unit；②`resume_attempt_after_amendment` 的 `AmendmentResumeMode::Reexecute` 把 `Superseded` 置回 Running（既有行为，此前遗留 stale `completed_at`，现一并清除，属良性修正）；③ProviderFailure 把 `Blocked` 置回 Running（等价空操作）；并说明「进终态写戳、离开终态清戳」与 attempt record 的 `completed_at` 语义对齐。

Fix1 门禁：`cargo fmt --check` 0、`cargo clippy --all-targets --all-features -- -D warnings` 0、`--lib` / `--test it_web` / `--test it_core large_file_guard` 全绿（含新增用例）。
