# Design: ui-workbench-autopilot

## Context

**主契约（权威输入，不得漂移）**：`cadence/designs/2026-09-13_设计文档_3.7UIUX自动执行驾驶舱_v1.0.md`（716 行；v1.0 双审 + 二审修订，经 oracle + k3 + ds-task 三审与用户终审）。本文是它的 OpenSpec 架构摘要与落点登记，**不重新论证、不引入未拍板项**；两文冲突时以设计文档为准。

**本文只做规划产物**（OpenSpec 契约层职责，见 `openspec/config.yaml`）：精确文件、命令、测试与提交步骤由 `superpowers:writing-plans` 写入 `cadence/plans/`。

**现状技术事实基线**（设计文档 §0.6，勘察已证；实施时以代码为准）：

| # | 事实 | 证据 |
|---|---|---|
| 1 | 现有两页：`ChatWorkspacePage`（artifacts / review / actions / work-item-plan 四块）与 `CodingWorkspacePage`（六块） | `web/src/pages/` |
| 2 | 状态层与 WS 钩子齐全（workspace WS store、coding WS store、重连与重建） | `web/src/state/workspace-ws-store.ts`、`web/src/hooks/useWorkspaceWs.ts`、`web/src/state/workspace-chat-rebuild.ts` |
| 3 | 路由已就位：`/workbench` 三工作区 + `/image-create`；`RootRouteComponent` 与 `GuardedWorkflowLayout` 已存在 | `web/src/router.tsx:21`、`:25`、`:156` |
| 4 | workspace WS 后端入向 **29** 个顶层变体；前端 `WsInMessage` **27** / `WsOutMessage` **22**。`RequestChange` 是 `HumanConfirmDecision` 取值、`HumanTriage` 是 `WorkItemPlanCompileRecoveryActionDto` 取值，**都不是顶层变体** | `src/web/workspace_ws_types/in_.rs:12,149-153,191-195`；`web/src/api/types/workspace.ts` |
| 5 | coding WS 无插话入口（入向 **15** 变体，含 `CodingPing`）；single-candidate Running 拒 `UserMessage`（fail-closed） | `src/web/coding_ws_handler/protocol.rs:152` |
| 6 | **takeover 后端闭环已存在**，**前端零接入（DEF-1 空位）** | `src/web/app.rs:332`、`src/product/lifecycle_store/workspace.rs:289`；`web/src` 内 `takeover` 零命中（已复核） |
| 7 | 服务端约 116 条路由；C1 据此得出「引擎零等待」 | `src/web/app.rs` |
| 8 | 前端**尚未消费**阶段 3 typed 门禁协议；`remaining_budget` 只在后端 | `src/web/workspace_ws_types/out.rs:129,132` |
| 9 | 门禁三动作枚举 `HumanConfirmDecision` = `Confirm` / `RequestChange` / `Terminate`；**引擎两套门面分流**：legacy `human_confirm` 支持 `RequestChange`，SC typed 门拒绝之（须走 `HumanGateFeedback`） | `in_.rs:149-153`、`src/product/workspace_engine/conversational_gate.rs:911-914` |
| 10 | 门禁卡数据面 = `WorkItemPlanHumanGateSnapshot`（findings / attempts_used / manual_repairs_remaining / trigger / resumable） | `web/src/api/types/workspace.ts:75` |
| 11 | 停点状态 `stopped_needs_human` 已存在 | `src/product/models/workspace.rs:317` |
| 12 | 设计系统基线已存在（`--aria-*` token、`prefers-reduced-motion` 块、组件类） | `web/src/styles.css:16-38`、`:89-100`、`:103-129` |

**本次复核补证**（规划期只读核对，供实施对齐）：`trigger` 联合类型在 `web/src/state/workspace-ws-store-types.ts:315` 与 `web/src/api/types/workspace.ts:89` 两处均缺 `verification_new_findings`（设计文档 M6 的「同批改」为 TS 编译前提）；6+1 条出向事件的字段与设计文档 §2.7.1 完全一致（`out.rs:129-160`）；`human_gate_feedback` / `advance` 两条入向变体后端已存在（`in_.rs:113,117`）而前端缺失；`web/src/components/shell/` 目录已存在（新增全局 shell 组件落此）；`web/src/components/chat-workspace/` 下 `TimelineNodeList` / `ChatEntryList` / `entries/GatePromptEntry` / `ChatEntryRenderer` 均在位；`web/src` 内 toast 与 takeover 均零命中。

**一处路径勘误（供实施对齐）**：设计文档 §2.8 的 M6 行把待改的 store 类型文件写作 `web/src/hooks/workspace-ws-store-types.ts`，仓库实际路径为 `web/src/state/workspace-ws-store-types.ts`（`hooks/` 下不存在该文件）；同表另有一行已正确写作 `state/...`。实施以仓库实际路径为准，`trigger` 联合类型的加宽仍是**两处同批改**（`state/workspace-ws-store-types.ts:315` + `api/types/workspace.ts:89`）。此为路径登记勘误，不涉及任何架构或验收边界变更。

**硬前置（本文最重要的工程结论，即 Phase 1a 的地基）**：Phase 1 不是纯样式活——必须**先补齐前端 typed 协议层（双向）**，出向接 6+1 条事件、入向补 2 条命令与发送 helper，然后才做界面重组。**不改后端**（事件面与 REST 均已存在）。

## Goals / Non-Goals

**Goals:**

- 把对话侧前端重组织为三区驾驶舱，使「自动执行为主、异常驱动人工」的范式在界面上可观察（对应 `REQ-UI37-01`、`REQ-UI37-18`）。
- 双向补齐前端 typed 协议消费与命令面，使阶段 3 已锁定的事件面第一次真正落地到投影（对应 `REQ-UI37-02`、`REQ-UI37-03`、`REQ-UI37-04`）。
- 让四类卡壳点各就位、可提醒、可就地处置（`REQ-UI37-05`、`REQ-UI37-06`、`REQ-UI37-08`、`REQ-UI37-09`）。
- 让全局计数在跨会话范围内可用且**降级语义诚实**（`REQ-UI37-07`）。
- 提供可回滚的双轨灰度与分段独立验收路径（`REQ-UI37-11`、`REQ-UI37-12`）。

**Non-Goals（设计级边界，与 proposal 的非目标互补、不重复其动机叙述）:**

- **不改后端任何一个字节**——不新增路由、WS 事件、持久化结构；安全边界与权限模型不在本设计范围（前端只消费既有能力）。
- **不新建时间线 / 列表 / 门卡组件**，不引入第二个组件库或第二套 token 体系（增量不换血）。
- **不做运行中接管**（运行中插话 / 改门禁语义），不做 `auto_start_coding` 显式语义 / 多仓 coding / 旧协议退役门（C2、C5）。
- **不实现真只读安全边界**：本设计收窄的是呈现与操作面，不是引擎授权面。
- **不动 `ImageCreatePage` 页面内容**（C3），**不在编码侧新增插话入口**（C7）。

## Decisions

### D1: 三区信息架构 + 单一事实源 gate 投影

**决策**：`ChatWorkspacePage` 重组为①待处理收件箱 / ②自动执行流 / ③下钻对话流三区，职责互斥；② 区纯只读、无输入控件；人只从①进。

**门禁单一事实源**：收件箱门禁条目与 ③ 区门卡**都从 store 的 gate 投影派生**，不各自维护状态；**去重键 = `turn_id`**（同一 `turn_id` 只投影一条条目 + 一张门卡，与 `REQ-UI37-04` 同源）。条目契约 = 「存在 gate 投影即存在、门闭环即收敛」，turn 三终态只改子状态不移除条目（唯一移除点 = `human_gate_closed` 或重建口径下会话状态投影已 Confirmed）。

**替代方案与否决理由**：① 收件箱与门卡各自持状态 → 双源必然漂移（同一门出现两处不同预算/子状态），且去重要写两遍；② 以 `stage === "human_confirm"` 作为条目存在判据（现状 `workspace-chat-rebuild.ts:523,527`）→ SC typed 门的 turn 形态不经该 stage，重建会漏条目（设计文档 H5 已证）——故必须把生成条件扩到「存在 store gate 投影」，两形态共用同一投影入口。

**分诊的定位**：`awaiting_triage` 是**UI 投影态、不是引擎状态**，由 `needs_human` verdict（`ReviewGate::UserTriageRequired`，路由终点 `human_confirm`）投影而来，不持久化；分诊是门禁条目的**亚型**，同会话同门只呈现一条，不另设动作面（`REQ-UI37-01`）。

### D2: 组件复用（改造既有、不新建）与门卡动作载体按 `flowKind` 门面分流

**组件复用（L1 结论，零新增依赖）**：② 区复用既有 `TimelineNodeList`（`ChatWorkspacePage` 已 import），③ 区复用既有 `ChatEntryList`（其 `useVirtualizer` 来自现成依赖 `@tanstack/react-virtual`），门卡改造既有 `GatePromptEntry`——**不新建时间线 / 列表 / 门卡组件，不新增依赖**；因此长会话虚拟化零新增成本。

**替代方案与否决理由**：① 新建「驾驶舱专用」组件族 → 与既有分发 / 虚拟化 / 重建路径并存出第二套渲染，违反增量不换血，且重建路径需写两遍；② 为驾驶舱重写虚拟化 → 引入新依赖与新性能风险，而现成虚拟化已在既有会话列表上验证过。

**门卡动作载体**：改造既有 `GatePromptEntry`（分发点 `ChatEntryRenderer`），不新建门卡组件；动作载体按 `flowKind` 分流——`single_candidate`（typed 门）→ 反馈 `HumanGateFeedback` / 确认 / 终止；legacy `human_confirm` → 确认 / `RequestChange` / 终止（三者均走 `HumanConfirm`）。

**替代方案与否决理由（动作载体）**：单一「反馈」按钮抽象两门面 → typed 门会发出 `HumanConfirm{RequestChange}`，被引擎明确拒绝（`conversational_gate.rs:911-914`），用户在界面上看到的是引擎拒绝而非动作成功。

**取舍**：门面差异对用户可见（模式切换须有明确标签、不允许同一输入框隐式推断意图），代价是 UI 上出现两种动作集——接受，因为这是引擎语义的真实形状，隐式统一会造成误发。

### D3: 全局 shell 挂**路由根 layout**

**决策**：新建全局 shell 组件（落 `web/src/components/shell/`，含 L2 警示条骨架与跨会话计数），挂在 `router.tsx` 的 `RootRouteComponent`（`<Outlet/>` 外层）；toast（L1）与 L3 的标题 / favicon 逻辑挂**同址**。

**替代方案与否决理由**：① 挂 `GuardedWorkflowLayout` → 覆盖面只有 workspace / coding 子树，**会漏掉 `/image-create`**（其 parent 是 `rootRoute`，`router.tsx:156-171`），而 C3 明确要求警示条在 `/image-create` 上同样可见——这是设计文档 H2 的挂点改正；② 挂 `app-shell.tsx`（设计文档旧稿含糊写法）→ 该文件在 `web/src` 不存在，且「app shell」没有精确挂点语义；③ 挂在单个页面内 → 覆盖不了其他页面，违反 L2「任何页面可见」。

### D4: 跨会话聚合 = 方案 a（前端多 WS 订阅 watch 集合）

**决策**（用户拍板）：会话清单走既有 REST（`/api/projects` → `/api/issues` → per-issue lifecycle → `workspace_sessions[]`），挑活跃会话（非终态）按最近活动排序 **watch 前 K 个、K = 8 可配**；每会话一条既有 `useWorkspaceWs` 连接；**全局 sticky / L3 计数源 = watch 集合的并集**（不是全量会话）；集合内保持连接不重建、入集合建连、出集合断开；改 K 立即重算。

**降级语义（必须诚实呈现）**：K 外会话仍在会话列表（REST 态）可见，但**其实时卡壳不进全局计数**；界面 MUST NOT 声称计数为全量。

**替代方案与否决理由**：① REST 轮询全部会话 → 卡壳是事件驱动的瞬时事实，轮询延迟与请求量都不可接受，且轮询无法得到「实时卡壳」；② 新增后端聚合接口 → 违反 C1 引擎零等待；③ 全量会话都建 WS → 活跃会话数无上界，连接数不可控（K 是显式上界）；方案 a 是唯一同时满足「实时 / 引擎零改动 / 连接上界」的路径。

### D5: 双向协议补齐的落点与命令 id 纪律

**决策**：出向按三层落——`api/types/workspace.ts`（typed 变体与投影类型）→ `hooks/workspace-ws-message-handler.ts`（补 7 条分支 + `default` 宽容诊断分支）→ `state/workspace-ws-store.ts`（三投影 + gate 投影）；入向补 `WsInMessage` 的 `human_gate_feedback` / `advance` 与发送 helper（落点 `hooks/useWorkspaceWs.ts`）。

**`command_id` 纪律**：统一在发送 helper 内用 `crypto.randomUUID()` 生成；**每会话每命令只生成一次**——动作发起时生成，重试 / 重连重放复用同一 `command_id`，不重新生成；引擎对同 `turn_id` / `command_id` 的重发幂等（`decisions.rs:265-278` 的 `Replayed { turn }` 只重发同 `HumanGateTurnOpen`，不新开门、不重复扣预算）。

**替代方案与否决理由**：① 在调用点各自生成 `command_id` → 重试路径会生成新 id，破坏引擎幂等、可能重复扣预算（M2 的原始 finding）；② 把 `default` 分支写成静默忽略 → 现状 `human_gate_closed` 已 typed 却无分支、到达到即被丢弃（H4），静默是这类缺陷的根源，故 `default` 必须**留痕不抛错**。

### D6: 特性开关 = 页内分支早返回 + `localStorage` + 默认旧形态

**决策**：不新增路由、不新增独立开关组件；`ChatWorkspacePage` 函数体**顶部早返回**既有四块形态，命中新形态取值才渲染三区；取值持久化于 `localStorage`（键如 `aria.chat.cockpit`，值 `legacy | cockpit`），**默认 `legacy`**；灰度 = 默认旧形态 → parity 达标后只翻默认值。

**替代方案与否决理由**：① 新增 `/cockpit` 路由 → 引入第二个 URL 形态，与 Phase 4 的操作收口冲突，且 `router.test.tsx` 的路由树断言会失真；② 环境变量 / 构建期开关 → 无法在真实链上做同机 parity 对比与快速回滚；③ 默认新形态 → 旧形态未保双轨即切换，违反 C4「不留半成品切换」。

**兼容约束**：`router.test.tsx` 断言的是路由树与页面挂载，开关在页面内部，故路由层断言不变；渲染默认形态时走的仍是 `legacy` 分支，**不得**为了通过路由测试把默认值翻成 `cockpit`（M5）。

### D7: autopilot 锚点、去抖与 busy 处置

**决策**：每会话默认开启；**唯一自动推进锚点 = 观测到 durable `Confirmed`**（close 完成事件或会话状态投影）。三条硬约束：`HumanGateFeedback` 之后只等修复轮结果、**绝不自动 Advance**；compile 失败导致门重开 → 回收件箱呈现、**不重试**；`advance_rejected` → 回落收件箱并**停止该会话自动推进**。去抖：同会话同 entry 在未观测到 `advance_completed` / `advance_rejected` 前只发一次（同 `command_id` 复用）；收到 `human_gate_busy` **立即放弃本次自动推进**（不清空、不重试、不排队），等该 turn 下一次门开 / 终态事件再判定。引擎已自动的组内单元推进，前端**不代发**。停点默认**每次人工门前必停**、可配，且停点只影响「是否自动发 `Advance`」，不改变引擎门禁语义。

**替代方案与否决理由**：① 以「人已提交动作」为锚点提前推进 → 反馈尚未产生 Confirmed，提前推进等于跳过引擎判定（H2 原始 finding）；② 在 `human_gate_busy` 时排队重试 → 与引擎在途 turn 抢门；③ 默认「无停点全自动」→ 与用户拍板的保守默认冲突，且把引擎判定权交给前端；④ 前端代发 `StartCoding` → 属 DEF-4 前置子项，留阶段 4（C5）。

### D8: 重连重建保留去重集

**决策**：重连后 `turn_id` / `command_id` 去重集**从持久化会话状态重放恢复、不丢弃**（`session_state` 携带 gate snapshot 与 run history 等 durable 事实）；重建期间显示「重建中」态而非空态；已处理的门与已终结的推进不得在重连后被当作新事件重复提醒。

**替代方案与否决理由**：① 重连清空去重集、以事件面重放重建 → 历史 `advance_rejected` 重放（`ADVANCE_REPLAY_NOT_READY` / `ADVANCE_REPLAY_INCOMPLETE`）会被当作新拒绝，反复打断用户；② 只信内存态 → 刷新即丢。

### D9: 四层提醒，L4 双触发

**决策**：L1 收件箱置顶 + 脉冲 + toast（toast 从零新建，`web/src` 现状零命中）；L2 顶部 sticky 红底白字警示条（不可手动关闭、处理完才消失、跨会话计数）；L3 标题 `🔴待处理×N`（R1 唯一例外、可关）+ favicon 角标 + 系统通知（卡壳 30s 未处理才触发，权限被拒降级为 L1+L2 且不丢提醒）；**L4 双触发**——① 剩余修复轮次 `remaining_budget ≤ 1` 转橙 + 次数梯度重复提醒；② 门开时长前端自计时（默认 10min、阈值可配、引擎零改动）。

**替代方案与否决理由**：旧稿口径「超时前 5min / 预算 < 20%」**已作废**（用户新拍板 H3）——引擎事件面唯一真实可用的预算量是 `manual_repairs_remaining`（`out.rs:132`），「剩余时间」不是引擎概念，前端无法从事件面推出一个可信的「超时前 5min」；改为「剩余轮次 ≤ 1」是唯一有真实数据源的阈值，门开时长则改为纯前端自计时（明确标注非引擎数据）。

**取舍**：L4 升级只是提醒升级，不改变任何引擎语义（不自动终止、不自动代做决策）——提醒强度与引擎行为解耦，代价是极端情况下用户仍可能不理，但这正是「异常驱动人工」的边界，不由前端越权。

### D10: 设计系统增量不换血

**决策**：沿用既有 `--aria-*` token、组件类与 `prefers-reduced-motion` 兜底；**只新增三组派生 token**（对话流 / 拓扑六态 / 门禁三态），一律从现有语义色系派生、不新增色相（靛主色 / 橙 CTA / 绿黄红语义色）；等宽统一 `ui-monospace`、数字 `tabular-nums`；硬规则 R1–R6 全量遵守。

**替代方案与否决理由**：引入新 token 体系或组件库 → 与既有暖白底 + 墨色基调并存会产生两套视觉语言，且 `prefers-reduced-motion` / 焦点环 / 对比度等既有兜底会出现第二套实现。

### D11: 测试与证据策略

**决策**：**人工验收只认真实链**（轻语料 + `pi` provider，界面走通「建会话 → 看计划 → request-change / confirm / advance → 看编码进度 → 看 push」，全程不开命令行）；**自动化替身只用于单测/回归**（`ARIA_PROVIDER_MODE=fake` + 既有测试端点注入 `verdict: needs_human` 驱动门开 + 前端 vitest 覆盖三投影与门禁三态），**不代替人工验收**。测试形态对齐既有格局：组件级 vitest（如 `components/chat-workspace/entries/*.test.tsx`）+ fixture 驱动的门开替身。

**既有测试迁移（7 文件，随改动同步迁移断言，不得靠删除用例过关）**：`ChatWorkspacePage.{test,actions,artifacts,review,work-item-plan}.test.tsx`、`ChatWorkspacePageParts.test.tsx`、`router.test.tsx`。其余受影响：`workspace-ws-message-handler.test.ts`、`workspace-ws-store.*.test.ts`、`state/chat-entries.test.ts`、`useWorkspaceWs.*.test.tsx`（8 个）+ `useWorkspaceWsReconnect.test.ts`、`useStageUI.test.ts`、`p1-entries.test.tsx`。

**替代方案与否决理由**：只用 fake 替身关闸 → 无法证明「真实全链无需命令行」这一产品目标；只跑真实链 → 门禁三态 / 去重 / 重放等分支无法稳定复现。

## Risks / Trade-offs

| # | 风险 / 取舍 | 缓解 |
|---|---|---|
| R1 | **WS 重连后三区投影空窗**：驾驶舱依赖完整投影，断线可能出现收件箱/执行流空 | 复用既有 `workspace-chat-rebuild`；重连按事件面重建投影，重建期显示「重建中」态而非空态；去重集从 `session_state` 重放恢复（D8） |
| R2 | **流式性能**：长会话 + 高频 `stream_chunk` 导致渲染卡顿 | 长会话虚拟化（只渲染视口内轮次，复用既有虚拟化依赖）；流式更新按帧合并；日志抽取与对话流渲染解耦 |
| R3 | **系统通知权限被拒** | 明确降级路径：L3 通知失效 → 退化为 L1 + L2，**不因权限被拒而丢提醒**；设置页给出恢复指引 |
| R4 | **autopilot 误推进**（最危险的失败模式） | 只在观测到 durable `Confirmed` 后自动发 `Advance`；反馈后绝不自动 Advance；compile 失败不重试；停点默认保守（每次人工门前必停）；`AdvanceRejected` 停止该会话自动推进；重连重放按 `command_id` 去重（D5、D7）——**不提供「无停点全自动」默认** |
| R5 | **提醒过载**（多门同时打开） | 收件箱按严重度排序；标题/角标只显示计数；L4 重复通知按设置节流间隔限流 |
| R6 | **K 外降级带来「计数不精确」** | 这是方案 a 的显式取舍而非缺陷：界面不声称全量为精确计数，K 可在设置中提高；相比无上界连接数或轮询延迟，降级语义可控且可解释 |
| R7 | **门开时长自计时与引擎无超时概念不一致** | 计时纯前端、阈值不进入引擎（D9）；界面把它呈现为**提醒阈值**而非引擎状态，不产生「引擎会超时」的错误预期 |
| R8 | **双轨并存成本**：旧代码路径保留使页面文件同时承载两形态 | 开关为页内早返回（最小侵入），不新增路由/组件；parity 达标后翻默认值，旧路径留作回滚通道；7 个既有测试同步迁移以锁定两形态均可用 |
| R9 | **门面分流误用**（typed 门发 `RequestChange`） | 载体按 `flowKind` 分流，typed 门**不提供**该可达入口（D2）；模式切换有明确标签，不用同一输入框隐式推断 |

## Migration Plan

**交付与灰度（无部署迁移、无数据迁移，纯前端）**：

1. **分段交付**：Phase 1a（双向协议补齐 + 单会话三区只读 + 事实底座）→ 1b（全局 shell + 跨会话聚合 + 就地动作 + autopilot + 四层提醒 + 设置 + takeover 接线）→ 2 → 3 → 4；**每段独立增量验收**，不合并为一次性大验收（`REQ-UI37-12`）。
2. **双轨灰度**：1a 落地特性开关，**默认 `legacy`（旧形态）**；新形态仅对显式切换 `localStorage` 的用户生效，用于同机 parity 对比；parity 达标后**只翻默认值**（D6）。
3. **双轨边界**：Phase 1 期间新驾驶舱只覆盖对话侧，**编码侧仍走旧 `CodingWorkspacePage`**（C4）；Phase 2 才深化编码侧。
4. **回滚策略**：任一 Phase 出问题 → 把 `localStorage` 开关翻回 `legacy` 即回到旧形态（旧代码路径原样保留）；无后端变更与数据变更，故无数据回滚/部署回滚需求。1b 之后的全局 shell / toast 挂在路由根，回滚时随之不渲染。
5. **关闸证据**：每 Phase 两清单（自动化证据清单 + 人工证据清单），人工清单含真实链走查步骤与截图、卡壳提醒层级逐层可观察、全程无命令行（D11）。

## Open Questions

均为**设计文档已明示的、实施时各自细化**的留白（不改本文架构、specs 或工作包划分）：

1. **Phase 2 拓扑图 / 依赖图布局算法**（节点自动布局 vs 手工分层）——设计文档只定义六态语义与「依赖连线区分已满足/阻塞中」，布局算法属实施细化。
2. **Phase 2 流式日志抽取的归并键与筛选维度**——设计文档定义「按节点归并、可筛选」，具体归并键（节点 id / 阶段 / attempt）留实施细化。
3. **Phase 4 历史操作审计视图的数据来源**——受 C1（引擎零改动）约束，必须由既有 durable 事件面推导；具体从哪条既有记录回查（会话事件前缀 / gate snapshot / run history）留 Phase 4 规划时确认。
4. **会话清单（REST）的刷新节奏**——方案 a 定了清单来源与 watch 集合重算规则，未定「多久拉一次清单」；该参数不影响 watch 集合语义与计数口径，属实施/设置细化。
5. **`localStorage` 开关键名与取值字面量**——设计文档给出示例（`aria.chat.cockpit`，`legacy | cockpit`），最终命名属实施细化。
