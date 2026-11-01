# Proposal: ui-workbench-autopilot

## Why

aria 工作区引擎的 web 前端今天是「多面板控制台」：`ChatWorkspacePage` 平铺 artifacts / review / actions / work-item-plan 四块，`CodingWorkspacePage` 平铺六块，人必须盯着屏幕、自己翻找当前该做什么，且阶段 3 已锁定的 typed 门禁事件面前端**尚未消费**（`human_gate_turn_open` / `advance_completed` / `advance_rejected` 在 `web/src` 零命中，`human_gate_closed` 已 typed 却在 `workspace-ws-message-handler.ts` 无分支、到达到即静默丢弃），接管 REST（后端闭环已存在）前端**零接入**。3.7 UI/UX 改造把对话侧前端变成**自动执行驾驶舱**：系统自动跑全程，人只在四类卡壳点（门禁 / 分诊 / stopped / 硬错）被叫醒，全部处理动作在界面上就地完成、全程不需要开命令行。现在做，是因为上游五阶段路线 v2.0 的专项测量轮已提供真实跑证据，且阶段 3 的 typed 协议已稳定——界面天然只依赖它，早于阶段 4 的旧协议退役。

## What Changes

> 术语：SC = SingleCandidate（单候选）；「门面」= 引擎两套人工门面（legacy `human_confirm` 与 SC typed 门）；「卡壳点」= 门禁 / 分诊 / stopped / 硬错。

**范式（用户拍板，不得漂移）**：自动执行为主、异常驱动人工（exception-driven autopilot）。三条界面公理：默认无人工输入、人的入口唯一（待处理收件箱）、卡壳必须显眼（四层递进提醒）。**Phase 1–4 全部只动 `web/src/`，引擎零改动**（后端事件面与 takeover REST 均已存在）。

**分阶段交付（各段独立增量验收）**：

- **Phase 1a**——双向 typed 协议补齐 + 单会话三区只读：
  - **出向**补 6 条 typed 事件分支与投影（`human_gate_turn_open` / `_turn_completed` / `_turn_failed` / `_busy` / `advance_completed` / `advance_rejected`），并修复既有 `human_gate_closed` 的静默丢弃；`switch` 补 `default` 宽容分支记诊断（未识别 `type` 不抛错但留痕，防止下一次协议新增再被静默吞掉）。
  - **入向**补前端 `WsInMessage` 的 `human_gate_feedback` / `advance` 两条变体与发送 helper；门禁命令 `command_id` 统一 `crypto.randomUUID()` 生成，每会话每命令只生成一次，重试/重连重放复用同一 `command_id`。
  - **单会话三区只读**：`ChatWorkspacePage` 从四块平铺重组为「① 待处理收件箱 / ② 自动执行流 / ③ 下钻对话流」；② 区复用既有 `TimelineNodeList`、③ 区复用既有 `ChatEntryList`（其虚拟化来自现成依赖 `@tanstack/react-virtual`），不新建时间线/列表组件。**不含**全局 shell 与跨会话聚合。
  - **事实底座**：三投影 + gate 投影（单一事实源、去重键 `turn_id`）；`trigger` 联合类型加宽补 `verification_new_findings`（`state/workspace-ws-store-types.ts` 与 `api/types/workspace.ts` 同批改，否则 TS 编译不过）；gate 投影重建条件由 `stage === "human_confirm"` 扩到「存在 store gate 投影」，使 SC typed 门的 turn 形态也被重建到。
  - **双轨特性开关**：`ChatWorkspacePage` 内部**分支早返回**旧四块形态 + `localStorage`（键如 `aria.chat.cockpit`）+ **默认旧形态（legacy）**；不新增路由、不新增开关组件；parity 达标后只翻默认值。
- **Phase 1b**——全局聚合与就地动作：
  - **全局 shell 骨架**：新建全局 shell 组件（落在既有 `web/src/components/shell/`），挂**路由根 layout**（`router.tsx` 的 `RootRouteComponent`）——覆盖面 = 工作流路由子树 **+ `/image-create`**（其 parent 是 rootRoute，不在 `GuardedWorkflowLayout` 子树内，只挂后者会漏掉它）。
  - **跨会话聚合（方案 a，用户拍板）**：全局 sticky 警示条与 L3 标题/favicon 的待处理计数是**跨会话聚合量**；实现路径固定为**前端多 WS 订阅**——会话清单走既有 REST（`/api/projects` → `/api/issues` → per-issue lifecycle → `workspace_sessions[]`），挑活跃会话 **watch 前 K 个、K = 8 可配**；超 K 只 watch 最近活跃的 K 个（K 外在会话列表仍可见，但实时卡壳不进全局计数=降级语义）；**计数源 = watch 集合的并集，不是全量会话**；已在集合内的会话保持连接不重建，入集合建连、出集合断开；引擎零改动。
  - **就地动作（按门面分流）**：收件箱门禁条目与门卡**改造既有 `GatePromptEntry`**（不新建组件），动作载体按 `flowKind` 分流——SC typed 门：**反馈 = `HumanGateFeedback`**（每次扣一次 manual-repair 预算）、确认/终止 = `HumanConfirm` close；legacy `human_confirm` 门：Confirm / `RequestChange` / Terminate 三取值均走 `HumanConfirm`。**两套门面不得混用**：typed 门发 `HumanConfirm{RequestChange}` 被引擎明确拒绝。危险动作二次确认态 + 按钮旁常显预算余量。
  - **收件箱四类条目**：门禁等待 / 分诊 / stopped 接管 / 硬错；分诊是门禁条目的亚型（同会话同门去重只呈现一条）；硬错**不提供接管**（takeover 只接受 `stopped_needs_human`，其他状态 409），`protocol_error` 先按 `code` 分流——门禁/advance 相关 code 内联回所属条目，无法归属才升为硬错条目。
  - **autopilot**：每会话默认开启，**唯一自动推进锚点是观测到 durable `Confirmed`**（close 完成事件或会话状态投影）；`HumanGateFeedback` 之后只等修复轮结果、**绝不自动 Advance**；compile 失败导致门重开 → 回收件箱呈现、**不重试**；`AdvanceRejected` 回落收件箱并停止该会话自动推进；同会话同 entry 去抖（未观测到闭环前只发一次、同 `command_id` 复用）；收到 `HumanGateBusy` 立即放弃本次自动推进（不清空、不重试、不排队）；编码类组内单元推进由引擎自动、前端不代发。可配停点集合，默认**每次人工门前必停**。
  - **四层提醒（L1/L2/L3/L4）**：L1 收件箱置顶 + 脉冲 + toast（`web/src` 现状 toast 零命中，从零新建）；L2 顶部 sticky 红底白字警示条（跨会话计数、处理完才消失、不可手动关闭）；L3 浏览器标题 `🔴待处理×N`（R1 唯一 emoji 例外、可关闭）+ favicon 角标 + 系统通知（卡壳 30s 未处理才触发，权限可拒并降级不丢提醒）；L4 **双触发**——① 剩余修复轮次 `remaining_budget ≤ 1` 转橙 + 次数梯度重复提醒，② 门开时长前端自计时（默认 10min、阈值可配、引擎零改动）。**旧稿「超时前 5min / 预算 < 20%」口径作废。**
  - **设置面板**：以对话框形式挂在驾驶舱（不新增路由），可配提示音、系统通知、升级间隔、L4 门开时长阈值、重复提醒梯度、标题 emoji 标记、autopilot 停点集合、聚合窗口 K。
  - **停点接管接线（DEF-1 补位）**：只在 stopped 条目呈现「接管」，调既有 `POST /api/workspace-sessions/{session_id}/takeover` 建子会话并接入 ③ 对话流；父会话不被修改；409 `workspace_session_takeover_not_allowed` 在条目内联呈现 `code` + `reason`，按钮置灰并附原因文案，不静默失败、不自动重试。
- **Phase 2**——编码仪表盘：① 单元拓扑图（六态节点 + 依赖连线、区分已满足/阻塞中）、② 依赖链视图、③ 预算门（work item 60min / coding 90min **为 driver 约定口径、前端自计时呈现，引擎不发布该预算事件**）、④ 实时日志抽取（从 `stream_chunk` / `coding_stream_chunk` 按节点归并、可筛选）。以**只读**为主，需要动作时统一走 Phase 4 收口。
- **Phase 3**——计划审批深度：① 轮次间 markdown revision diff + 变更摘要、② 逐条 capability/契约核对表（provided vs required，缺口一键跳转 finding）、③ **DEF-3 吸收**：SC 子 session（amendment / plan-repair）只读呈现面板（不改子会话语义、不提供子会话内写操作）。
- **Phase 4**——advance / 接管操作收口：① 两页一致的键盘快捷键（确认/反馈/接管/advance）、② 批量确认（仅限可幂等、无副作用的动作；危险动作不参与）、③ 历史操作审计视图（谁在何时对哪个门/会话做了什么）。**只归一、不重复实现**；Phase 1–3 分散在两页的入口在 Phase 4 统一为同一套操作语义与同一个审计出口。

**设计系统（增量不换血）**：沿用现有 `--aria-*` token、组件类与 `prefers-reduced-motion` 兜底，**不引入第二套 token 体系或第二个组件库**；新增三组派生 token（对话流 / 拓扑六态 / 门禁三态），一律从现有语义色系派生、不新增色相。硬规则：R1 图标一律 SVG(Lucide)、零 emoji（仅 L3 标题标记例外且可关）；R2 触达目标 ≥ 44px；R3 `focus-visible` 环；R4 过渡 150–300ms；R5 新增动画在 `prefers-reduced-motion` 下全降级为静态；R6 对比度 ≥ 4.5:1。

**验收口径（分两层，不得混用）**：**人工验收**只认真实链——轻语料 + `pi` provider，在界面上走通「建会话 → 看计划 → request-change / confirm / advance → 看编码进度 → 看 push」，全程不开命令行；**自动化替身**（`ARIA_PROVIDER_MODE=fake` + 测试端点 review-fixture 注入 `verdict: needs_human` 驱动门开 + 前端 vitest）只用于单测与回归，**不代替人工验收**。每 Phase 两清单：自动化证据清单（三投影期望态、新增事件分支用例、去重键与重连重放幂等、门禁三态迁移）与人工证据清单（真实链走查步骤记录、卡壳提醒层级逐层可观察、全程无命令行）。Phase 1 拆 **1a / 1b 两段独立验收**（1a 验「看得到真事实」、1b 验完整一遍），不合并。

**非目标（明确不做）**：C1 引擎零等待——Phase 1–4 均不动后端、不新增接口/事件；C2 跑动中接管（运行中插话/改门禁语义）属引擎变更，另立项；C3 `ImageCreatePage` 页面内容不动（仅全局警示条与 toast 在其上可见）；C4 旧页面保双轨到 parity——Phase 1 期间新驾驶舱只覆盖对话侧、编码侧仍走旧 `CodingWorkspacePage`；C5 DEF-4/5/6（`auto_start_coding` 显式语义 / 多仓 coding / 旧协议退役门）归阶段 4；C6 不新增 Provider / CI / 持久化评估语料；C7 编码侧不新增插话入口（coding WS 无插话入口且 single-candidate Running 拒 `UserMessage`，fail-closed）。

## Capabilities

### New Capabilities

- `web-workbench-autopilot`: 对话侧 web 前端的自动执行驾驶舱能力——三区信息架构（待处理收件箱 / 自动执行流 / 下钻对话流）、typed 门禁与推进协议的双向消费与投影、按门面分流的就地动作、Confirmed 锚定的 autopilot、四层卡壳提醒与设置、方案 a 跨会话聚合、stopped 接管接线、分阶段（1a/1b/2/3/4）交付与双轨特性开关。

### Modified Capabilities

（无）——本 change 为纯前端呈现层与操作层改造，**不改变任何既有 spec 的行为契约**：`work-item-plan-conversational-gate` 的后端语义（typed 反馈回合、预算纪律、durable 恢复、接管端点）原样保留，本 change 只是前端首次消费它；`lifecycle-workbench-pipeline-ux` / `spec-workbench-canvas-experience` 的既有 requirement 均不触及。

## Impact

- **受影响代码**：`web/src/` 全域，不触 `src/`（引擎零改动）。协议层 `api/types/workspace.ts`、`hooks/workspace-ws-message-handler.ts`；状态层 `state/workspace-ws-store.ts`、`state/workspace-ws-store-types.ts`、`state/chat-entries.ts`、`state/workspace-chat-rebuild.ts`；钩子层 `hooks/useWorkspaceWs.ts`、`hooks/useStageUI.ts`；页面层 `pages/ChatWorkspacePage.tsx`（+ `ChatWorkspacePageParts.tsx`）、Phase 2 起 `pages/CodingWorkspacePage.tsx`；组件层改造既有 `components/chat-workspace/entries/GatePromptEntry.tsx`（+ 分发点 `ChatEntryRenderer.tsx`），新建全局 shell 组件（`components/shell/`）与 toast；复用既有 `TimelineNodeList` / `ChatEntryList`；样式 `styles.css` 新增三组 token；`router.tsx` 路由根 layout 挂全局 shell。
- **受影响接口/依赖**：消费既有 REST（`/api/projects`、`/api/issues`、`/api/issues/{id}/lifecycle`、`POST /api/workspace-sessions/{session_id}/takeover`）与既有 workspace WS 双向事件面；**不新增后端接口、不新增前端依赖**（虚拟化用现成 `@tanstack/react-virtual`）。
- **BREAKING（测试基线）**——**既有测试迁移**：`ChatWorkspacePage.tsx` 根形态重组牵动 7 个既有测试文件——`ChatWorkspacePage.{test,actions,artifacts,review,work-item-plan}.test.tsx` + `ChatWorkspacePageParts.test.tsx` + `router.test.tsx`；必须随改动同步迁移断言，**不得靠删除用例过关**。`router.test.tsx` 断言路由树与页面挂载，特性开关在页面内部，故路由层断言不变；渲染默认形态时走的仍是 `legacy` 分支，**不得**为了通过路由测试把默认值翻成 `cockpit`。其余受影响测试：`workspace-ws-message-handler.test.ts`、`workspace-ws-store.*.test.ts`、`state/chat-entries.test.ts`、`useWorkspaceWs.*.test.tsx`（8 个）+ `useWorkspaceWsReconnect.test.ts`、`useStageUI.test.ts`、`components/chat-workspace/entries/p1-entries.test.tsx`。
- **运行验证**：每 Phase 关闸做增量验证（该 Phase 的链路片段在真实 provider 上跑通），证据分「人工走查记录」与「自动化替身用例」两清单，不做一次性大验收。
- **非目标影响面**：不动 `ImageCreatePage` 页面内容与其组件；不动 `src/` 任何后端代码；不新增 Provider / CI 语料 / 持久化评估。
