# wave2 F-18 UI/F-20 修复报告：story/design AuthorConfirm 门 UI 决策面补齐

- 工作树：`.worktrees/feat-b-0808-add-monorepo`
- 输入证据：
  - 监控登记 `2026-09-19_测试发现登记_阶段4监控.md` F-18/F-20 段——story/design
    AuthorConfirm 三层死锁（UI 无控件 / WS `author_decision` 被 `LEGACY_MESSAGE_RETIRED`
    拒 / 矩阵只留 Abort）；F-18 v26+v27 终止按钮两连挂（零 WS 出站）。
  - `wave2-f18-report.md` §5 遗留观察——「`selectGateProjection` 对 author_confirm 仍返回
    null，story 页面级 terminate 按钮属 UI 面，留待 UI 波次」（本修复即该波次）。
  - 59d59760（矩阵 AuthorConfirm 臂放行 `AbandonHumanGate`；WS confirm 帧继续拒收，
    approve 维持 HTTP confirm 端点设计通路）+ W2c 实测（HTTP confirm 0472→200 confirmed）。

## 1. 修法

对照 cockpit 门卡/收件箱先例，把 story/design 的 author_confirm 阶段纳入门投影单一事实源，
决策面三处同源派生；确认/终止按矩阵语义分流：

| 面 | 改动 |
|---|---|
| 投影（workspace-cockpit-projection.ts） | 新增 `isStoryDesignAuthorConfirm` 判据；`selectGateProjection` 对 story/design author_confirm 投影 `stage:author_confirm` 前缀门（无 typed turn/snapshot；confirmed 即关门）；`gateActionBlockReason` 对该门开态放行（不再落 `terminal_stage`），confirmed/closure → `closed` |
| live 路径（workspace-ws-message-handler.ts） | `stage_change`→author_confirm 同样落 gate_prompt 门卡（原先仅 human_confirm；否则门开瞬间无决策面，只在 session_state 全量重建时可见） |
| 确认接线（api/client.ts + ChatCockpitPage.tsx） | 新增 `confirmWorkspaceSession`（`POST /api/workspace-sessions/{id}/confirm`，`{confirmed_by:"user"}`）；页面 `routeGateConfirm`：story/design author 门走 HTTP（200 后乐观 `setSessionStatus("confirmed")` 收敛决策面+审计 completed，失败 markRejected 不收敛），其余门走既有 WS confirm 帧；三个门面（actions/confirm 热键/advance 热键）统一接此路由 |
| 终止接线 | 复用 C3-T4 `sendAbandonGate`（WS `abandon_human_gate`，59d59760 矩阵已放行）经门面 terminate；ConfirmTwiceButton 二次确认惯例 |
| 决策面渲染 | ① 收件箱门条（`GateInboxActions`：确认+终止）② 对话流门卡（`GatePromptEntry`）——两处由投影自动派生；③ 产物审核页签（author_confirm 默认视图）动作位恢复「确认产物+终止」按钮（T5 退役留档处，可见性对齐 `gateActionBlockReason`） |
| 批量确认边界（CockpitInbox.tsx） | `isSelectableGate` 排除 author_confirm 门——批量 runner 只发 WS confirm 帧（该阶段被矩阵拒收），author 门确认通路是 HTTP，不提供批量勾选 |

边界：work_item_plan（SC）与 work_item 的 author_confirm 维持无门投影+terminal_stage
拦截（红测钉死）；legacy human_confirm 与 SC typed 门路径零改动（全量回归覆盖）。

## 2. TDD 红绿

红（修复前实测 12 失败，与 F-20 现场证据同因）：

- `selectGateProjection` 对 story author_confirm 返回 null；`gateActionBlockReason` 落
  `terminal_stage`；收件箱零门条；live stage_change→author_confirm 无门卡；页面无任何
  确认/终止按钮。

绿（新增 18 测全过）：

- `workspace-cockpit-projection.test.ts`（+7）：story/design 投影形状；work_item_plan
  边界；开态放行/confirmed 关门/terminate closure 关门；收件箱门条（`gate:stage:author_confirm`）；
  confirmed 后等待项消失。
- `workspace-ws-message-handler.test.ts`（+3）：story/design live stage_change→author_confirm
  落门卡（gate_identity/action_facade 元数据）；work_item_plan 不落。
- `ChatCockpitPage.confirm.test.tsx`（+8）：渲染（收件箱+门卡+产物面板三面两按钮可点，
  ConfirmTwice 惯例）；确认接线（fetch 断言 `POST /api/workspace-sessions/session_001/confirm`
  +`{"confirmed_by":"user"}`，零 WS confirm 帧，乐观收敛 sessionStatus=confirmed+等待面消失）；
  HTTP 失败不收敛（门保持可操作）；终止接线（一次点击仅 arm、二次确认后恰一次
  `sendAbandonGate`）；author 门不提供批量勾选。

回归：定向 11 文件 183/183；全量 vitest **1496/1496**（175 文件）；`tsc --noEmit` 0 错。

## 3. 与 B 线（F-19）边界

- 本修复仅前端面（web/src 8 文件）；引擎/矩阵零触碰（59d59760 已在 HEAD）。
- 与 P0Watchdog 的 ChatInputBar Abort 入口（同文件渲染条件区）无重叠，已 IRC 确认。

## 4. 遗留观察

- HTTP confirm 是记录级写（durable Confirmed + spec 确认态），不产生 WS 关门事件——UI 靠
  乐观 confirmed 收敛，权威 stage 推进等引擎侧后续广播；引擎是否在确认后驱动 story 流程
  后续腿（送审/完成）不属 UI 面（F-19/B 线与全流程复验覆盖）。
- 批量确认对 author 门的 HTTP 通路未接（单会话卡/收件箱/产物面板已可用）；如需跨会话批量
  需扩 runner 走 HTTP，未纳入本次最小面。

## 5. Commit 文件清单

- `web/src/state/workspace-cockpit-projection.ts`（投影+判据+放行）
- `web/src/state/workspace-cockpit-projection.test.ts`
- `web/src/hooks/workspace-ws-message-handler.ts`（live 门卡）
- `web/src/hooks/workspace-ws-message-handler.test.ts`
- `web/src/api/client.ts`（confirmWorkspaceSession）
- `web/src/pages/ChatCockpitPage.tsx`（路由 confirm+产物面板动作位）
- `web/src/pages/ChatCockpitPage.confirm.test.tsx`
- `web/src/components/chat-workspace/cockpit/CockpitInbox.tsx`（批量勾选边界）
- `cadence/reports/provider-validation-round/wave2-f18ui-f20-report.md`（本报告）
