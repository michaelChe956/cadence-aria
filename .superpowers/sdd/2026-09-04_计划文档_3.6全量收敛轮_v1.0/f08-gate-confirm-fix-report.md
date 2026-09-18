# F-08：Work Item Plan 人工门确认帧修复报告

## 结论

已修复 Work Item Plan 单候选人工确认门发送错误消息类型的问题。`sendHumanConfirm("confirm")` 现在发送裸帧 `{"type":"confirm"}`；`request-change` 与 `terminate` 仍保持 typed `human_confirm` 帧。

同时，确认按钮不再在 WebSocket 成功写入时乐观标记为「已确认」：只有收到引擎 `human_gate_closed { decision: "confirm" }` 事件后才结算门卡。这样引擎拒绝或编译失败时，页面不会显示错误的成功状态。

## 根因与协议核对

- F-08 现场记录证明引擎在 `human_confirm` 阶段拒绝 typed `human_confirm { decision: "confirm" }`，而裸 `confirm` 一击通过。
- `src/web/workspace_ws_types/in_.rs` 中 `WsInMessage::Confirm` 不带字段，因此实际 wire 形状是 `{"type":"confirm"}`，不是带 `payload: null` 的对象。
- `src/web/workspace_ws_handler/protocol.rs` 的 SingleCandidate `HumanConfirm` 分支只允许 `human_gate_feedback`、裸 `Confirm` 与 typed `HumanConfirm::Terminate`。
- `src/web/workspace_ws_handler/protocol.rs` 的 legacy 分支同时允许裸 `Confirm` 与 typed `HumanConfirm`；`decisions/inbound.rs` 对 legacy 的裸 Confirm 转调既有 `handle_human_confirm(... Confirm, None)`。因此 story/design 共用的确认调用改为裸 confirm 不会回归，typed 的 request-change/terminate 路径保持不变。

## TDD 证据

### 红测

新增/收紧 `useWorkspaceWs.actions.test.tsx` 的确认消息回归断言后执行：

```text
pnpm vitest run src/hooks/useWorkspaceWs.actions.test.tsx

FAIL expected {"type":"confirm"}
Received {"type":"human_confirm","decision":"confirm","payload":null}
```

该失败精确复现 F-08 的错误消息构造。

### 绿测

最小实现改为 confirm 决策发送裸 Confirm；同文件新增确认仅在 `human_gate_closed` 后结算的断言。

```text
pnpm vitest run src/hooks/useWorkspaceWs.actions.test.tsx src/pages/ChatCockpitPage.test.tsx

Test Files  2 passed (2)
Tests       119 passed (119)
```

覆盖项：

1. `request-change` 仍发送 typed `human_confirm`，confirm 发送裸 `confirm`。
2. 发送 confirm 后 gate prompt 不会立即标记 resolved。
3. 收到 `human_gate_closed(confirm)` 后才标记 resolved。
4. 驾驶舱「确认产物」按钮在 single-candidate approval gate 状态下调用同一确认路径。

## 「确认产物」按钮排查

当前源码中的按钮已接到 `GatePromptEntry → CockpitActionFacade.confirm → sendHumanConfirm`。新增页面级回归用例实际点击按钮，验证会调用 `sendHumanConfirm("confirm")`，因此当前工作树内不存在独立的静默 no-WS 接线断点；它将随修复后的裸 confirm 走同一已验证的 WS 发送路径。未扩展额外组件改动。

## 定向回归

```text
pnpm vitest run src/state/ src/hooks/ src/components/chat-workspace/

Test Files  74 passed (74)
Tests       722 passed (722)
```

## 提交文件

- `web/src/hooks/useWorkspaceWs.ts`
- `web/src/hooks/useWorkspaceWs.actions.test.tsx`
- `web/src/pages/ChatCockpitPage.test.tsx`
- `.superpowers/sdd/2026-09-04_计划文档_3.6全量收敛轮_v1.0/f08-gate-confirm-fix-report.md`

## Concerns

无代码阻塞项。当前验证为前端定向测试；引擎协议矩阵未改动，真实浏览器端到端复验仍可在下一次部署后按 F-08 的干净人工门路径执行。
