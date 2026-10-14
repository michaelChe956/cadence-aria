# FE-1 报告：前端类型与标签

- 状态：**DONE_WITH_CONCERNS**（切片本体完成并验证；concerns 为跨切片交接项）
- 切片：FE-1（前端类型与标签），设计权威 history://DesignPushLocal2「切片 FE-1」节
- worktree：`.worktrees/feat-b-0808-add-monorepo`
- 提交：见文末 commit（`fix: FE1 类型与标签`）

## 1. 交付内容（按锚点）

| 文件 | 变更 |
|---|---|
| `web/src/api/types/coding.ts` | `PushStatus` + `skipped_not_configured`；`PlanCodingExecutionState` + `awaiting_manual_recovery/waiting_human/failed/aborted`；`PlanCodingUnitProgressDto.waiting_reason` 放宽为 `string \| null`；`PlanCodingTargetDto` 补九字段（snake_case，`string \| null`） |
| `web/src/state/coding-execution-labels.ts`（新增） | 三组标签单一定义点：`ATTEMPT_STATUS_LABELS` / `STAGE_LABELS` / `PUSH_STATUS_LABELS` + `LOCAL_DELIVERY_LABEL = "本地交付(无远端)"` |
| `web/src/components/lifecycle/DeliveryStatusPanel.tsx` | 新增 `skipped` 交付态（`本地交付(无远端)`）；仓卡取最严重者 `failed > pending > skipped > pushed`；收起摘要与副行列出本地交付计数 |
| `web/src/components/lifecycle/PlanGroupProjectionPanel.tsx` | 私有标签表改从共享模块导入；`delivered` 谓词对齐后端 `is_delivered`（completed + 已推送/本地交付） |
| `web/src/pages/CodingWorkspaceReports.tsx` | GitPanel `push` 行改用 `PUSH_STATUS_LABELS`（不再裸露 enum 原文） |

**复审追加（P3，Main 指令）**：`coding.ts:110` `CodingAttempt.push_status` 原为内联的
`"not_pushed" \| "pushed" \| "failed" \| null`（第三份值域副本，且漏 skipped）。改为
`PushStatus \| null` —— 复用单一值域别名，S2a 后端可产出的 `skipped_not_configured`
因此可承载。红：`CodingAttempt["push_status"] = "skipped_not_configured"` 触发
`TS2322: not assignable to '"failed" | "not_pushed" | "pushed" | null'`；修后 `tsc -b` exit 0。

九字段与 S3 后端契约经 ImplS3 消息确认（2026-10-14）：全部 `Option<String>`、无枚举；`execution_state` 四个新增字面量一致；`waiting_reason` 原样透传 `attempt.manual_recovery_reason`（开放值集）；`SkippedNotConfigured` 序列化为 `skipped_not_configured`。

## 2. TDD 红绿

**红**（先写测试）：`web/src/state/coding-execution-labels.test.ts`（新）+ 三处组件测试的 skipped 用例。
运行 `npx vitest --run src/state/coding-execution-labels.test.ts src/components/lifecycle/DeliveryStatusPanel.test.tsx src/components/lifecycle/PlanGroupProjectionPanel.test.tsx src/pages/CodingWorkspacePage.reports.test.tsx` → **5 failed / 24 passed**（模块缺失、三处 skipped 标签缺失）。

**绿**（实现后同命令）→ **33 passed**。

**双绿**：
- `npx tsc -b --force` → exit 0
- `npx vitest --run`（全量）→ **215 files / 2202 tests passed**
- `npm run build`（`tsc -b && vite build`）→ exit 0（chunk >500kB 警告为既有）

## 3. 因类型变更而必须同步的夹具

`PlanCodingTargetDto` 九字段按设计冻结为**必填**（非可选），因此以下既有夹具补字段（该类型变更导致的编译失败，非重跑确认）：

- `web/src/components/coding-workspace/dashboard/CodingDashboard.test.tsx`（`planTarget()` 工厂）
- `web/src/state/coding-dashboard-projection.test.ts`（`planTarget()` 工厂）
- `web/src/pages/CodingWorkspaceGroupProgress.test.tsx`（4 个 target 字面量，抽 `targetFacts` 常量展开）
- `web/src/pages/CodingWorkspacePage.test.tsx`（1 个 target 字面量）

均为"本夹具不覆盖逐仓交付事实 → 九字段取 null"，不改变既有断言。

## 4. 行为变化（需要浏览器复验的点）

1. GitPanel 的 `push` 行由裸露 enum 文本（`pushed`/`failed`）改为中文标签（`已推送`/`推送失败`/`本地交付(无远端)`）。相应两处既有断言由 `"pushed"`/`"failed"` 改为 `"已推送"`/`"推送失败"`。
2. 仓卡 `data-status` 新增取值 `skipped`；本地交付用成功色 + 「本地交付(无远端)」文案，与「已推送」区分。
3. 计划级 group 面板：`completed + skipped_not_configured` 计为「已交付」（与后端 `is_delivered` 同口径），但仍显式标出「本地交付(无远端)」。

## 5. Concerns / 交接项

1. **FE-2 需补 execution_state 新值文案**：`web/src/pages/plan-coding-progress.ts` 的 `EXECUTION_STATE_LABELS` 仍只有旧七值，`awaiting_manual_recovery/waiting_human/failed/aborted` 会回落成英文原文（该文件属 FE-2 切片，本次未动）。
2. **GitPanel 的 `remote` 行**：skipped 现场 `store.pushedRemote` 为 null、`request.remote` 为配置名（如 `origin`），可能出现「remote origin」与「本地交付(无远端)」并列。是否改由 FE-2 的逐仓 PR 面板统一口径，请 controller 裁定。
3. **`PlanCodingWaitingReason` 保留**：字段放宽后该联合类型暂无使用点，但按"additive-only"保留为编排等待（REQ-CS-06 三值）的公开记录类型，未删除。
4. **FE-1 未动的相邻契约**：`PlanTargetEntryDto.push_status`（work-item-plan.ts）仍是 `string | null`，未收窄为 `PushStatus`——收窄会与 S2a 的推进时序耦合，留给 FE-2 决定。

## 6. Controller 浏览器复验步骤

前置：无需 S3 后端落地即可验标签层；skipped 数据用 mock/构造响应注入。

1. **Issue 交付面板**：打开含 `push_status="skipped_not_configured"` 条目的 issue 详情 → 期望该仓卡 `data-status="skipped"`、文案「本地交付(无远端)」、成功色；不含「推送失败」与错误行；「已推送」仅出现在真正 pushed 的仓卡上。折叠态摘要含「本地交付(无远端) N」。
2. **运行结果 → git 面板**：attempt 的 `reviewRequest.push_status="skipped_not_configured"` → push 行显示「本地交付(无远端)」，无「重新推送」按钮；`failed` 时仍显示「推送失败」且有按钮；`pushed` 时显示「已推送」且无按钮。
3. **计划级 group 面板**：单 target `completed + skipped_not_configured` → 行 `data-status="delivered"`、「已交付」+「本地交付(无远端)」，无「未满足」行。
4. **单仓/非 group 回归**：单 target 场景顶部 header DOM 与改前一致（本次未改 header 区）。

上述均为代码侧结论；UI 确认以 controller 浏览器复核为准。
