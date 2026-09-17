# P38B1 Task 2 报告：终态/相位门投影守卫的唯一出口

## 做了什么

- 在 `gateActionBlockReason` 集中前端 gate actionability：已关闭门优先锁定；single-candidate 仅在 `human_confirm + approval|evaluate` 放行；`completed + humanGateSnapshot` 放行以保留 amendment reopen 入口；六个其余非门阶段的陈旧快照锁定。
- `selectGateProjection` 将守卫理由投影至 cockpit inbox 与 chat gate entry；聊天重建元数据保留同一理由。
- `createCockpitActionFacade` 在每个写动作发送前重新读取 store；锁定时返回 `false` 且不调用 websocket helper。manual advance 仅保留已关闭门的既有推进路径。
- Inbox 与 GatePromptEntry 的锁定门均只显示用户可读理由，移除选择框、feedback、confirm、request-change、terminate 等操作控件，避免键盘与 callback 绕过。
- Cockpit 共享 facade 覆盖确认快捷键、推进快捷键、批量确认和手动推进；Legacy 页的 gate 与 `ChatInputBar` 人工返修均通过共享 facade，锁定 human-confirm 输入随之禁用。
- 补充投影、facade、Inbox、GatePromptEntry 与 Cockpit 页面反例/正例：running 陈旧快照、generate 相位失配零发送；approval、evaluate、completed+快照保持发送能力。

## 验证证据

红灯阶段已执行新增门控测试，生产实现前的预期失败包括：缺少 `gateActionBlockReason`、stale projection 仍显示 gate actions、facade 未返回阻断结果。

绿灯阶段：

```text
cd web && pnpm test src/state/workspace-cockpit-projection.test.ts src/state/cockpit-action-routing.test.ts src/components/chat-workspace/cockpit/CockpitInbox.test.tsx src/components/chat-workspace/entries/GatePromptEntry.test.tsx src/pages/ChatCockpitPage.test.tsx src/pages/ChatWorkspacePage.actions.test.tsx
```

结果：6 个测试文件、114 个测试通过。

```text
cd web && pnpm tsc -b
```

结果：通过。

```text
cd web && pnpm test
```

结果：167 个测试文件、1417 个测试通过。运行期间 lifecycle 队列测试输出既有 jsdom `navigation (except hash changes)` stderr，但对应测试通过；本 Task 未修改 lifecycle 代码。

## 边界与自审

- 放行集合未窄于引擎授权面：Evaluate 与 completed+快照均不被前端提前拒绝；0017 completed 形态仍由引擎返回既有 inline protocol error。
- 未修改 Rust、wire、引擎拒绝语义或 0429/P3b 回门问题。
- `GateProjection.action_block_reason` 对既有测试 fixture 保持可选，以避免无关测试数据被新的投影字段破坏；`selectGateProjection` 的生产返回始终提供该字段。

## 提交

已提交为 `fix: 阻断终态门投影操作`；未 push。

## Round 1 审查 findings 修复

- P1：`gateActionBlockReason` 保留 `completed + humanGateSnapshot` 放行和六个既有终态锁定，并将其余无 `humanGateSnapshot`、无 `humanGateTurn` 的阶段统一锁为 `terminal_stage`。Cockpit 确认热键回归覆盖 `running`、`compile_plan` 与无快照 `completed`，三者均不发送；因此恢复 BASE 的无门静默语义。
- P2：选择渲染时从 live store 派生理由，避免在事件路径重建并丢失聊天历史。`GatePromptEntry` 以 `gate_identity` 重新匹配当前 `selectGateProjection`；legacy 门在 live stage 已迁离 `human_confirm` 时直接显示“已离开人工确认门”并移除全部操作控件。新增回归覆盖“重建期门卡 → stage_change 迁出 → 不可操作”。
- ⚠3 核实：Rust `close_human_gate(Confirm)` 先以 `Running` 关闭门并触发 compile，随后强制读取 durable 状态，要求 `WorkspaceSessionStatus::Confirmed + SingleCandidatePhase::Completed` 才发 `HumanGateClosed`（`src/product/workspace_engine/conversational_gate.rs:831-871`）。因此已确认门的 advance 必须兼容 completed phase；守卫在 `sessionStatus === "confirmed"` 时将 `human_confirm` 视作可推进，保留 BASE 的已确认推进能力。

### Round 1 定向验证

```text
cd web && pnpm test src/state/workspace-cockpit-projection.test.ts src/state/cockpit-action-routing.test.ts src/components/chat-workspace/cockpit/CockpitInbox.test.tsx src/components/chat-workspace/entries/GatePromptEntry.test.tsx src/pages/ChatCockpitPage.test.tsx src/pages/ChatWorkspacePage.actions.test.tsx
```

结果：6 个测试文件、118 个测试通过。

### Round 1 全量验证

```text
cd web && pnpm tsc -b && pnpm test
```

结果：TypeScript 构建通过；167 个测试文件、1421 个测试通过。全量测试仍输出既有 jsdom `navigation (except hash changes)` stderr，对应 lifecycle 测试通过，本次未修改该路径。

