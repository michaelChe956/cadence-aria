# Phase 4 Task 2 报告：操作语义归一

## 实施内容

- 新增 `cockpit-operation-semantics.ts`，锁定跨 Task 共享的操作类型、热键映射、批量确认白名单、危险确认超时、操作者标签、文本编辑目标识别和 `ConfirmTwiceButtonHandle` 契约。
- 新增 `ConfirmTwiceButton`：`forwardRef` + `useImperativeHandle` 暴露 `arm()`；首次 arm 进入确认态，第二次才调用 `onConfirm`，统一在 10 秒后复位。
- 新增 `GateFeedbackEditor`，统一 input/textarea 的受控输入、去空白提交、禁用空反馈和可访问性样式。
- 将 CockpitInbox 的门禁终止、hard-error 终止及接管，以及 GatePromptEntry 的终止都迁移至共享危险确认组件；删除三处自建 10 秒 timer、`pendingTerminate`、`pendingTakeover` 与 `feedbackEditorOpen` 状态机。
- 将 CockpitInbox 和 GatePromptEntry 的 typed 反馈迁移至共享编辑器。
- 为 `CockpitActionFacade` 增加 `advance()`；Cockpit 与 legacy 页都提供同一 `sendAdvance` 适配器。Cockpit header 在 `humanGateClosure.decision === "confirm"` 或 `sessionStatus === "confirmed"` 时显示“手动推进”，由 facade 新建 command id；重试仍传回已有 id。
- 在两页 facade 入口守卫已关闭 gate，保持确认只在 gate 未关闭时发送，并保留 typed / legacy wire 分流。

## 红灯与绿灯证据

- 红灯：`pnpm test src/state/cockpit-operation-semantics.test.ts src/state/cockpit-action-routing.test.ts src/components/chat-workspace/cockpit/ConfirmTwiceButton.test.tsx src/components/chat-workspace/cockpit/GateFeedbackEditor.test.tsx src/pages/ChatCockpitPage.test.tsx` 在实现前失败：新模块/组件不存在，`advance()` 与“手动推进”不存在。
- 定向绿灯：同一目标集加 CockpitInbox 与 P1 entries 后，7 文件、81 测试通过。
- 全量门：`cd web && pnpm tsc -b && pnpm test` 通过：160 文件、1342 测试通过。

## 提交

- `f989d724 refactor: 归一驾驶舱操作语义与危险确认`

## 自我审查

- `web/src/components/chat-workspace` 仅剩 `ConfirmTwiceButton` 的统一 10 秒 timer；未保留自建确认计时器或 pending 状态。
- 两处 facade 构造都已传入 `sendAdvance`；手动推进无确认态不渲染，且重试路径继续复用原 command id。
- `git diff --stat -- src/` 为空；`web/package.json` 无 diff；未改动 image-create 页面。
- 全量测试运行中保留已有 jsdom navigation stderr（相关测试仍通过），未把它归因于本任务。
