# Wave 4 · F-21 修复报告：plan 会话 human_confirm 门终止 UI 接线（第四次，终修）

- 会话基线：worktree `feat-b-0808-add-monorepo`（d2d67b39/17d21491 均在 HEAD 祖先链）
- 登记来源：`cadence/notes/2026-09-19_测试发现登记_阶段4监控.md` F-21 段（0449，v26/v27/v28 三次未修）

## 1. 根因（与前三轮「UI 没接发送器」判断不同）

前三轮修复（3837ba75 typed 迁移 / d2d67b39 story/design 面）后，plan 门在 **approval/evaluate 相位** 的终止接线其实已经通了（本次复现脚本验证：门卡+收件箱两处点击均恰发一次 `abandon_human_gate`）。0449 现场零出站的真正根因在**投影层相位白名单**：

- `gateActionBlockReason` 对 `stage=human_confirm + SC 流` 只放行 `singleCandidatePhase ∈ {approval, evaluate}`，其余一律 `phase_mismatch`；
- 而 plan 会话（SC 流，v26 起新建会话恒为 SC，lifecycle.rs:680）**除终审门外还会停在 human_confirm**：
  - **context blocker 门**（prepare 相位）：`workspace_engine/plan_outline/authoring.rs` `enter_work_item_plan_context_blocker` → `transition_stage(HumanConfirm)`，门卡文案自承诺「…或选择终止」；
  - **author 连续 validate 失败门**（generate 相位）：`decisions.rs` `enter_human_confirm_for_work_item_plan_author_failure`；
  - **旧会话缺相位字段**（null，`models/workspace.rs`「旧 session 缺失该字段时维持 legacy 语义」）。
- 这些形态下门卡/收件箱门条被 `phase_mismatch` 一刀切渲染成纯文案——**决策面整面消失**；用户能找到的「终止」（如 hard_error 行的 `actions.terminate`）再被门面 `terminate()` 的同款判据静默 `return false`——正是 v26/v28 实测的「点击零 WS 出站、无浮层」。
- 服务端事实核对（fix round 1 修正）：矩阵 `workspace_ws_handler/protocol.rs` HumanConfirm **SC 臂**放行 `AbandonHumanGate`（59d59760，不校验相位）、引擎 `close_human_gate` 内存守卫只校验 `WorkItemPlan + SingleCandidate + HumanConfirm`——但 **durable CAS `compare_and_save_human_gate_close` 另有前置**：`WaitingForHuman ∧ phase∈{Approval,Evaluate} ∧ human_gate_snapshot 在场`。context blocker/author 失败两入口只置 stage+status，**不写快照不提相位**——首版只修前端时，这三形态点终止会回 `product_store_conflict` Error、门仍开。fix round 1（controller 裁定=引擎侧补建关门效力）在 CAS 层对 terminate 放行这三形态（confirm 前置一字不动）。

## 2. 修复（scope：plan 门终止专属放行，confirm/feedback 纪律不动）

1. **`state/workspace-cockpit-projection.ts`**：新增 `gateTerminateBlockReason`——`phase_mismatch` 且 `work_item_plan + human_confirm + single_candidate` 时终止放行（null），其余跟随通用判据；`GateProjection` 新增 `terminate_block_reason`（selectGateProjection 四处产出）。
2. **`state/cockpit-action-routing.ts`**：门面 `terminate()` 改用终止专属判据（confirm/feedback/advance 判据不变）；`phase_mismatch` 的 plan 门点终止 → 恰一次 `sendAbandonGate(commandId ?? newCommandId())`。
3. **`state/workspace-chat-rebuild.ts`**：`buildGatePromptEntry` 持久化 `terminate_block_reason`（null=允许）。
4. **`entries/GatePromptEntry.tsx`**：门卡 selector 抽出 `gateCardBlockReason` 共享骨架（活投影匹配 + stage 前缀离场兜底），终止判据单独求值；渲染链新增终止专属分支——终止放行即渲染 ConfirmTwice 终止钮（含 `actionBlockReason` 说明行「门相位与当前阶段不一致，可终止后重新发起」），confirm/反馈编辑器维持相位纪律不渲染。注意 `??` 陷阱：`terminate_block_reason` 显式 null=放行，判「缺省」必须辨 `undefined`。
5. **`cockpit/CockpitInbox.tsx`**：`GateInboxActions` 同款终止专属分支（终止钮露出、确认/反馈/批量勾选维持 `action_block_reason` 纪律）。

### fix round 1（k3 审 1×P1——根因反转）：引擎侧补建关门效力

6. **`lifecycle_store/workspace.rs` `compare_and_save_human_gate_close`**：terminate（`Terminated`）专属放行 `phase ∈ {Prepare, Generate, None}` 的 WaitingForHuman SC plan 记录（快照缺席可接受——这两门的 durable 开态证据即 WaitingForHuman 本身，引擎已在内存校验 stage==HumanConfirm）；confirm（`Running`）前置与反伪造判据（Evaluate+快照在场）、Completed/Failed 拒收**一字不动**。选型：CAS 放宽（最小面）优于门入口补写快照+提相位——后者会把 context blocker 门伪造成 Evaluate 终审门，令前端 confirm 判据连带放行（confirm 一个无 candidate 的计划将流入 compile 链）。

## 3. TDD 证据

- 红（15:26 轮）：5 个定向文件 `6 failed | 80 passed`——plan 门三形态（prepare/generate/null）页面测试、门卡/收件箱组件测试全红（渲染纯文案、零终止钮）；路由/投影新断言红（`gateTerminateBlockReason` 未存在时全文件红）。
- 绿（15:31 轮）：定向 5 文件 86/86；**全量 vitest 175 文件 / 1517 测试全绿**；`tsc --noEmit` 0 错。
- 新增测试 16 个：
  - projection：三形态「terminate 放行 + confirm 维持 phase_mismatch」投影断言 + terminal_stage/closed/非 plan 三形态终止仍拦；
  - routing：三形态门面 terminate 恰一次（新 command id、confirm/feedback 零发）+ 活 turn command id 复用；
  - GatePromptEntry：context blocker 门卡终止专属渲染 + ConfirmTwice 两击恰一次 `actions.terminate`；
  - CockpitInbox：phase_mismatch 门条终止专属行（无确认/无反馈编辑器/无批量勾选）；
  - ChatCockpitPage.plan：三形态页面级——收件箱门条+对话流门卡两处终止点击各恰一次 `sendAbandonGate`（`expect.any(String)` command id）+ evaluate 终审门 confirm/terminate 双通回归。
- fix round 1 引擎 TDD：CAS 单测红 1（`human_gate_close_terminate_relaxes_non_approval_gate_shapes` 在旧前置下 Conflict）→绿 10/10（新增 ⑧terminate 放行三形态 / ⑨confirm 对同形态维持 Conflict / ⑩Failed 相位 terminate 仍拒）；引擎级新增 `plan_gate_terminate.rs` 2 测——0449 形态（context blocker 门，prepare 相位无快照，经真实入口 `enter_work_item_plan_context_blocker` 停门）terminate → `Abandoned` + durable `Terminated`（相位不变）+ 恰一条 `human_gate_closed{terminate,completed}` 事件；author 失败（generate）/缺相位（None）同款。邻域回归：lifecycle_store::tests 76/76、story_author_gate 7/7、workspace_ws_handler 118/118、human_gate_close/conversational_gate_close/workspace_single_candidate 33/33 全绿。
- 存量钉死测试全部不动仍绿（`blocks phase-mismatched gate actions from hotkeys`、`re-reads actionability…`、k3 P2-1/P2-2 离场兜底等）；仅 CockpitInbox stale 投影夹具补 `terminate_block_reason: "terminal_stage"` 字段对齐新投影形状。

## 4. 边界与登记

- **legacy 流 plan 会话**（v24 前存量）at human_confirm：终止仍会被服务端矩阵拒（legacy 臂无 AbandonHumanGate）——前端维持现状（帧可发出、错误可见），非 F-21 范围。
- **plan 会话 author_confirm 门**（Outline/候选/批量确认）：矩阵放行 AbandonHumanGate 但引擎 `close_human_gate` 拒（要求 human_confirm）且前端 `gateActionBlockReason` 对非 story/design 的 author_confirm 判 `terminal_stage` 不露终止——终止面不存在（无静默发送）。若需闭环属引擎侧改动，登记待后续。
- **context blocker 的 provide_context 通路**（typed feedback 编辑器在 phase_mismatch 下仍不渲染）：终止已可脱困；补充上下文继续通道建议另立登记。

## 5. 变更文件

- `web/src/state/workspace-cockpit-projection{.test}.ts` / `cockpit-action-routing{.test}.ts` / `workspace-chat-rebuild.ts`
- `web/src/components/chat-workspace/entries/GatePromptEntry{.test}.tsx` / `cockpit/CockpitInbox{.test}.tsx`
- `web/src/pages/ChatCockpitPage.plan.test.tsx` / `ChatCockpitPage.test-utils.tsx`
- fix round 1：`src/product/lifecycle_store/workspace.rs` / `tests/human_gate_close_promotion.rs`（mod：tests.rs include）+ `src/product/workspace_engine/tests/plan_gate_terminate.rs`（新，mod 注册于 `workspace_engine/tests.rs`）

验收建议：v29 轮在 0449（或任一新建 plan 会话走到 context blocker/author 失败门）点「终止→确认终止」，Network WS 应见恰一帧 `{"type":"abandon_human_gate","command_id":"…"}`，引擎回 `human_gate_closed{decision:"terminate"}` 且 durable session 落 `Terminated` 终态（fix round 1 后 CAS 不再 Conflict）。
