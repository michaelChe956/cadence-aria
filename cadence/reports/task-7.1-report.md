# Task 7.1（D）恢复 inbox 动作与 Page 守卫 — 红/绿证据报告

- 日期：2026-10-12
- Worker：ImplAnchorD71（glm53-task）
- Plan：`cadence/plans/2026-10-11_实施计划_规划阶段会话锚点固定与漂移恢复闭环_v1.1.md` Task 7.1
- Change：`openspec/changes/fix-planning-session-anchor-recovery` 工作包 7.1（细则权威）
- 设计：获批设计 §6（C/D：driver、observer、动作与文案）；C6.2 报告移交节第 2 条（`cadence/reports/task-6.2-report.md`）
- cwd：`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/web`
- 前置：C6.1 `a693764c`（投影三态投卡）＋ C6.2 `ef587e8d`（sendLcResumeFresh/typed 对账）；按钮预置 `56a0ded8`（KIND_GLYPH/KIND_CLASS 中性条目，本包沿用）

## Step 1–2 红（先写失败行为测试，过滤命中且断言真实失败）

命令：

```bash
npm test -- src/components/chat-workspace/cockpit/CockpitInbox.waiting-cards.test.tsx src/pages/ChatCockpitPage.generation.test.tsx src/pages/ChatCockpitPage.inbox.test.tsx -t anchor_t21
```

输出（红，新增 8 用例命中；7 个新行为断言 FAIL，1 个守卫锚 PASS）：

```
 ❯ src/components/chat-workspace/cockpit/CockpitInbox.waiting-cards.test.tsx (11 tests | 3 failed | 7 skipped)
   × anchor_t21 lc resume recovery cards > anchor_t21_inbox_explicit_native_fresh_and_existing_guards
     → Unable to find an element with the text: 运行环境已变化，不能继续旧原生会话。当前工作区、产物、反馈与评审意见均保留。
   × anchor_t21 lc resume recovery cards > anchor_t21_recovery_card_disables_when_disconnected_or_provider_unavailable
     → Unable to find an accessible element with the role "button" and name "启动新会话继续修订"
   × anchor_t21 lc resume recovery cards > anchor_t21_recovery_card_missing_map_copy_does_not_guess_drift
     → Unable to find an element with the text: 无法验证旧会话冻结信息
 ❯ src/pages/ChatCockpitPage.generation.test.tsx (22 tests | 2 failed | 20 skipped)
   × ChatCockpitPage > anchor_t21_inbox_explicit_native_fresh_and_existing_guards
     → expected <button …(3)>…(1)</button> to be null        ← 恢复活跃期 generic「开始生成」仍露出
   × ChatCockpitPage > anchor_t21_recovery_fresh_button_keeps_connection_guard
     → Unable to find an accessible element with the role "button" and name "启动新会话继续修订"
 ❯ src/pages/ChatCockpitPage.inbox.test.tsx (31 tests | 2 failed | 29 skipped)
   × ChatCockpitPage > anchor_t21_recovery_card_receipt_metadata_face_and_resolved_unblocks_generic
     → expect(element).toHaveTextContent("prepare_failed") 失败 ← 回执 metadata 未上卡面
   × ChatCockpitPage > anchor_t21_recovery_card_observed_session_shows_facts_without_write_action
     → expect(element).toHaveTextContent("旧原生会话已失效") 失败

 Test Files  3 failed (3)
      Tests  7 failed | 1 passed | 56 skipped (64)
```

红期即绿的 1 个用例为**守卫保持锚**（component `anchor_t21_recovery_card_observer_shows_facts_without_write_action`：实现前卡面本就无任何动作按钮，锚定「observer 无写动作」这一既有事实不被本包破坏；绿后继续 PASS）。非 0 tests、非 default/unsupported 绕过。

## Step 3–4 绿（最小实现 + 同文件旧用例回归）

同一命令输出（绿，8 用例全 PASS）：

```
 ✓ src/components/chat-workspace/cockpit/CockpitInbox.waiting-cards.test.tsx (11 tests | 7 skipped)
 ✓ src/pages/ChatCockpitPage.inbox.test.tsx (31 tests | 29 skipped)
 ✓ src/pages/ChatCockpitPage.generation.test.tsx (22 tests | 20 skipped)
 Test Files  3 passed (3)
      Tests  8 passed | 56 skipped (64)
```

三文件全量（plan 8.1 表 7.1 行，旧用例全保留）：

```bash
npm test -- src/components/chat-workspace/cockpit/CockpitInbox.waiting-cards.test.tsx src/pages/ChatCockpitPage.generation.test.tsx src/pages/ChatCockpitPage.inbox.test.tsx
# Test Files  3 passed (3)   Tests  64 passed (64)
```

邻近面回归（plan 8.1 表 6.1/6.2 行——验证 D 写面零回归，C 波字段原样消费）：

```bash
npm test -- src/state/workspace-ws-store.snapshot.test.ts src/state/workspace-cockpit-projection.test.ts src/state/workspace-observer-store.test.ts src/hooks/workspace-ws-message-handler.test.ts src/hooks/useWorkspaceWs.actions.test.tsx src/api/types.test.ts
# Test Files  6 passed (6)   Tests  191 passed (191)
```

组件/页面目录全量（本包渲染闭包）：

```bash
npm test -- src/components/chat-workspace/ src/pages/
# Test Files  55 passed (55)   Tests  657 passed (657)
```

全量前端单元回归（sanity；基线 C6.2 2087 用例，B 波并行提交叠加后现 2101）：

```bash
npm test
# Test Files  212 passed (212)   Tests  2101 passed (2101)
```

编译门（`npx tsc -b`）：0 错误（C6.2 移交的 CockpitInbox TS7053 由 56a0ded8 已消，本包未引入新错误）。

## 新用例与验收映射（anchor_t21，fixture 复用 C6.1 anchor_t19 值族 rec-1/node-1/resume_fingerprint_mismatch/codex/legacy_draft/op-1）

- `CockpitInbox.waiting-cards.test.tsx`（组件层）：
  - `anchor_t21_inbox_explicit_native_fresh_and_existing_guards`：waiting 卡文案（「运行环境已变化…均保留」「只更换 AI 原生会话，将重新校验权限并继续当前任务」）；「启动新会话继续修订」按钮 enabled；点击恰一次 `onStartLcResumeFresh("rec-1")`；inflight 禁双击（durable updated_at 未推进前第二次点击零出站）；受理后（state=fresh_reserved+updated_at 推进）转「正在启动新原生会话…」无动作面；prepare 失败回 waiting（updated_at 再推进）回执原因 `prepare_failed` 上卡面且按钮复原可再发起。
  - `anchor_t21_recovery_card_observer_shows_facts_without_write_action`：非当前会话卡（id 前缀 session_002）只显示事实零按钮。
  - `anchor_t21_recovery_card_disables_when_disconnected_or_provider_unavailable`：断连禁用＋「连接已断开」原因；author provider 已知不可用 fail-closed 禁用＋「Codex 当前不可用：CLI 未安装」（REQ-PPS-03 同源；快照未加载不假装不可用）。
  - `anchor_t21_recovery_card_missing_map_copy_does_not_guess_drift`：reason_code=`resume_fingerprint_unavailable` 时卡面为「无法验证旧会话冻结信息」，不出现「运行环境已变化」猜测文案。
- `ChatCockpitPage.generation.test.tsx`（页面接线，`newCommandId` mock 固定 "op-1"）：
  - `anchor_t21_inbox_explicit_native_fresh_and_existing_guards`：恢复三活跃态隐藏 generic「开始生成」（start-generation testid 为 null）；点击后 `freshSends` 恰 1 条且 `freshSends[0].operation_id === "op-1"`、recovery_id "rec-1"、provider_config 与普通开始生成同源（codex/codex/rounds 0/auto/auto）、reviewer_enabled false；双击仍 1 条；resolved 后恢复卡消失且「开始生成」复原 enabled（不永久屏蔽）。
  - `anchor_t21_recovery_fresh_button_keeps_connection_guard`：ws disconnected 时按钮 disabled＋原因可见。
- `ChatCockpitPage.inbox.test.tsx`（页面可见性与对账）：
  - `anchor_t21_recovery_card_receipt_metadata_face_and_resolved_unblocks_generic`：driver store `lcResumeRecovery`（waiting+last_operation failed/prepare_failed）经真实 `selectCockpitInbox` 投影出卡；回执 metadata 上卡面；resolved 后卡消失且 generic stopped 卡同屏在场。
  - `anchor_t21_recovery_card_observed_session_shows_facts_without_write_action`：observed 会话恢复卡经 cockpitInbox 直投可见，事实在场、无写动作。

## 实现（Produces）

- `CockpitInbox.tsx`：新 prop `onStartLcResumeFresh?(recoveryId)=>boolean` / `resumeFreshConnectionBlocked?`（经 rowProps→Row 透传）；新 `LcResumeRecoveryCard`（仅 waiting 态动作面；inflight 以点击时 durable `updated_at` 快照对账解除；provider 守卫经 `useProviderAvailabilityStore`+`getProviderOption` 与 ChatInputBar REQ-PPS-03 同源；observer 非 actionable 不出写动作；reserved/running 无动作面——标题已呈「正在启动新原生会话…」）；`LC_RESUME_MAP_MISSING_REASON="resume_fingerprint_unavailable"` 文案分叉。复用 56a0ded8 的 KIND_GLYPH/KIND_CLASS 条目未改。
- `ChatCockpitPage.tsx`：`handleStartLcResumeFresh`（durable 权威守卫：recovery_id 不匹配/非 waiting 不出站；`newCommandId()` 一次用户动作一次；providerConfigFor 同源快照；经 C6.2 `sendLcResumeFresh` 零乐观出站）；`lcResumeRecoveryActive`（三活跃态）——隐藏 ChatInputBar generic「开始生成」（hideStartGeneration）与 DisconnectBanner 中断重试出口（recoverableInterruptedRun 置 null）；CockpitInbox 接线两新 prop（断连＝connectionStatus!=="connected"）。
- `ChatCockpitPage.test-utils.tsx`：`lcResumeRecoveryItem` 夹具（title/summary 随 state/reason 派生，id 前缀决定 actionable 归属）。

## 边界与守卫核对（tasks 7.1 验收）

- lease/connection/provider/终态守卫不放宽：lease/connection 沿 C6.2 sendJson 守卫＋页面断连禁用；provider REQ-PPS-03 同源 fail-closed；终态（F-30/后端 SESSION_TERMINATED 族）不在恢复路径放宽，fresh 资格以 durable recovery waiting 为权威（resolved/新漂移取代本地拒绝）。
- 同一动作不并发重复 spawn：本地 inflight（updated_at 快照对账）＋后端 claim 单受理（A2a Replay/Busy）双层；unknown 不乐观标成败、不自动换新 ID 重发。
- 不进批量确认/C1/advance/retry_interrupted 通道；恢复卡 kind 非 gate 不可勾选（isSelectableGate fail-closed），无 bulkfresh。
- resolved 卡消不永久屏蔽：generic「开始生成」/中断重试出口随 resolved 复原（用例断言）；stopped 卡同屏共存。
- 设计 §6 文案全量落地（标题沿用 C6.1 投影；正文/说明/missing map/reserved-running 状态文字）；未实现「出生生成→继续生成」标签变体（设计为“可显示”非必须，统一「启动新会话继续修订」避免猜 run_kind 归类，不叫同 run 安全重试）。

## 提交

```bash
git add -- web/src/components/chat-workspace/cockpit/CockpitInbox.tsx web/src/pages/ChatCockpitPage.tsx web/src/pages/ChatCockpitPage.test-utils.tsx web/src/components/chat-workspace/cockpit/CockpitInbox.waiting-cards.test.tsx web/src/pages/ChatCockpitPage.generation.test.tsx web/src/pages/ChatCockpitPage.inbox.test.tsx
git commit -m "fix: D 恢复inbox动作与Page守卫"
```

实际提交：`f84adcec`（6 files changed, 604 insertions(+), 5 deletions(-)）。本报告未 git add（按派工）。

## 移交

- Task 8.1 Main 终验可直接按 plan 8.1 表 7.1 行复跑（本包已跑：3 文件 64 用例 PASS）。
- D 写面完成，C fixtures（workspace-cockpit-projection.test-fixtures）仅引用未改；CockpitInboxWaitingCards.tsx 未触碰（新卡组件按写面约束落在 CockpitInbox.tsx）。
- 「出生生成可显示继续生成」标签变体未做（设计许可项非必须）；如 owner 要求，属 D 写面小增量，可随 fixloop 补。

## Fix-loop round 1（审查 2026-10-12：1×P2 + 1×P3）

### P3 红数申报修正（先行更正）

审查复核：初始红实际为 **6/8**（非申报的 7/8）——两个 observer 用例
（waiting-cards `anchor_t21_recovery_card_observer_shows_facts_without_write_action`、
inbox `anchor_t21_recovery_card_observed_session_shows_facts_without_write_action`）
的实现前形态是「无卡体即无按钮」，断言空转绿；inbox observer 例当时红是夹具
title 派生 bug（假红），非行为断言。本轮已给两例补**前补丁必失败判别断言**
（动作区容器 `lc-resume-recovery-actions` 必须在场＋容器内事实文案可见且无
按钮——实现前容器不存在必红），防止空转绿复现。Step 1–2 节红清单应读作
6 行为红 ＋ 2 observer 空转绿（其中 1 例因夹具 bug 误计红）。

### P2 终态守卫（红→绿）

可达性复核（本 worker 独立验证）：`src/web/workspace_ws_handler/decisions/inbound.rs:495-503`
——`StartGeneration` 携带 `resume_recovery` 意图时早退进
`handle_lc_resume_recovery_fresh`（claim→dispatch），**完全绕过**
`engine.start_generation` 的 F-30 终态拒收（lifecycle.rs SESSION_ALREADY_CONFIRMED/
SESSION_TERMINATED）；terminate 也不清 durable recovery。审查成立。

Step 红（新用例 `anchor_t21_recovery_fresh_button_blocks_terminal_sessions_without_sending`，
confirmed/terminated 两态：按钮禁用＋卡内终态提示＋点击零出站）：

```
 ❯ src/pages/ChatCockpitPage.generation.test.tsx
   × ChatCockpitPage > anchor_t21_recovery_fresh_button_blocks_terminal_sessions_without_sending
     → expect(element).toBeDisabled()          ← 实现前终态按钮 enabled
 Tests  1 failed | 8 passed | 56 skipped (65)
```

实现（P2）：

- `ChatCockpitPage.tsx` `handleStartLcResumeFresh`：终态守卫 fail-closed——
  `sessionStatus ∈ {confirmed, terminated}` 直接返回 false 不出站。判据与 F-30
  同源（审查建议），但**不带 stage 条件**：恢复卡不随 stage 显隐（generic 按钮
  仅 prepare_context 在场，故 F-30 带 stage；恢复卡任意 stage 在场），终态任意
  stage 下 fresh 均属未定义路径，取更强守卫方向。
- `ChatCockpitPage.tsx` 接线：`resumeFreshTerminalBlockReason`——终态时复用
  `startGenerationBlockedHint`（F-30 同源文案），非终态 null。
- `CockpitInbox.tsx`：新 prop `resumeFreshTerminalBlockReason?`（经 rowProps→Row→Card）；
  卡片 disabled 并入 `terminalBlockReason !== null`，就地渲染终态提示
  （与 ChatInputBar F-30 提示同文案同屏共存，测试断言收窄到卡内）。

Step 绿（同命令）：

```
 ✓ -t anchor_t21：Test Files 3 passed   Tests 9 passed | 56 skipped (65)
 ✓ 三文件全量：Test Files 3 passed   Tests 65 passed (65)
 ✓ tsc -b：0 错误
 ✓ 邻近面＋目录组（6.1/6.2 三态文件＋hooks/types＋chat-workspace＋pages）：61 文件 849 用例 PASS
 ✓ 全量 npm test：2102 passed（+1 终态守卫用例）
```

### Round 1 提交

```bash
git add -- web/src/components/chat-workspace/cockpit/CockpitInbox.tsx web/src/pages/ChatCockpitPage.tsx web/src/components/chat-workspace/cockpit/CockpitInbox.waiting-cards.test.tsx web/src/pages/ChatCockpitPage.generation.test.tsx web/src/pages/ChatCockpitPage.inbox.test.tsx
git commit -m "fix: D 终态守卫与observer判别断言"
```
