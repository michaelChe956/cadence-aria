# 终止确认面可达性核实

> 当前工作目录（`pwd`）：`/home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02`

## 结论摘要

**F-C 当前不再成立**：原来的 grid 自动行错位/遮挡根因已在 `ChatCockpitPage` 右列改为 flex 流，并且确认面本体已改成经 `DialogShell` portal 到 `document.body` 的 fixed modal；现代码注释明确记载这是 F-C 根修。确认请求仍须先由入口按钮/其他调用触发，面板 `request === null` 时不渲染。

> 此核实依据当前源码结构和明确的修复实现；本次未进行当前浏览器真机点击复测，因此“可点”是由 portal/fixed/z-index/pointer-events 结构作出的代码级判断，不冒称本次实测。

## Q1｜F-C 原始根因现在还在吗？

**已不成立。** 旧的按 pending choice 条件切换 grid 行模板以及“无条件确认面自动占据第二行”的组合已撤除。当前确认面依然作为右列内的无条件 React 子组件存在，但它返回的是 Portal，不再成为宿主 flex/grid 的布局子元素；无 pending choice 也不会被 flex 子项挤压。

- `web/src/pages/ChatCockpitPage.tsx:1123-1128`：
  > `// F-C 同源根修：右列不再用「按 pending choice 二选一的 grid 行模板」排布——`
  > `// 条件子节点（终止确认面/终止横幅/选择提示/对话流/输入条）数量随状态变化，`
  > `// 行模板却在声明处固定，任何一处数量不一致就会把某个子节点塞进`
  > `// [?` 

  原文（连续主体）：
  > `// 条件子节点（终止确认面/终止横幅/选择提示/对话流/输入条）数量随状态变化，`
  > `// 行模板却在声明处固定，任何一处数量不一致就会把某个子节点塞进`
  > `` `minmax(0,1fr)` 行压成 0px 并被后续兄弟覆盖（确认面「DOM 在、点不到」的根因）。``
  > `// 改为 flex 纵向流：滚动区 `flex-1 min-h-0`，其余子节点保持自然高度。`
- `web/src/pages/ChatCockpitPage.tsx:1128-1129`：
  > `className="flex min-h-0 min-w-0 flex-1 flex-col rounded-xl border-2 border-[var(--aria-line-strong)] bg-[var(--aria-panel)]"`
- `web/src/pages/ChatCockpitPage.tsx:1230-1238`：
  > `{cockpitTerminateEligible && cockpitTerminatedAt === null ? (`
  > `<AutomationTerminationEntryButton onOpen={handleOpenAutomationTermination} />`
  > `) : null}`
  > `</div>`
  > `{cockpitTerminatedAt !== null ? (`
  > `<AutomationTerminatedBanner terminatedAt={cockpitTerminatedAt} />`
  > `) : null}`
  > `<AutomationTerminationConfirmSurface />`
  > `{pendingChoices.length > 0 || pendingChoiceRequests.length > 0 ? (`

  **注意**：这里仍按 React 条件渲染入口/横幅/choice notice，但布局不再依赖固定行槽；而 `<AutomationTerminationConfirmSurface />` 返回 portal，所以即使它的组件调用在页面树里无条件出现，也没有“隐形占一个 grid row”的效果。
- `web/src/pages/ChatCockpitPage.tsx:1075-1079` 的 grid 是左侧自动执行流 sidebar，而不是右列确认面父容器：
  > `className="flex min-h-0 flex-1 flex-row gap-2"`
  > `className="grid min-h-0 w-48 shrink-0 grid-rows-[auto_minmax(0,1fr)] rounded-xl border-2 border-[var(--aria-line-strong)] bg-[var(--aria-panel)]"`

**挤压机制变化**：右列由 `grid-rows-[auto_auto_minmax(0,1fr)_auto_auto]` / `grid-rows-[auto_minmax(0,1fr)_auto_auto]` 条件模板改成纵向 flex；更根本的是确认面已由 portal 脱离该布局流，因此不占 grid/flex 行。当前代码中旧模板已不在该确认面链路上。

## Q2｜确认面本体定位、容器链与 z-index

**确认面是 fixed 视口浮层，不是 grid/flex 内的普通流面板。** `AutomationTerminationConfirmSurface` 在 `request === null` 时返回 `null`；有 pending request 时渲染 `DialogShell`。`DialogShell` 调用 `createPortal(..., document.body)`，外层 fixed 覆盖全视口，卡片居中且可接收指针事件。

- `web/src/components/chat-workspace/cockpit/CockpitInboxWaitingCards.tsx:129-133`：
  > `const request = termination.pending;`
  > `if (request === null) {`
  > `  return null;`
  > `}`
- `web/src/components/chat-workspace/cockpit/CockpitInboxWaitingCards.tsx:218-234`：
  > `// F-C 根修（owner 2026-10-25 真机复现「确认面看不见、点不到」）：确认面`
  > `// 不再作为宿主布局（页面 grid 行模板／收件箱滚动块）的内联兄弟参与排布，`
  > `// 改为经 DialogShell 的视口浮层（createPortal → document.body，fixed`
  > `// inset-0 + 蒙层 + 顶层 z）。`
  > `<DialogShell`
  > `  ...`
  > `  layerClass="z-[80]"`
- `web/src/components/shared/DialogShell.tsx:196-210`：
  > `return createPortal(`
  > `<div className={`fixed inset-0 ${layerClass}`}>`
  > `<div className="pointer-events-none absolute inset-0 flex items-center justify-center p-4">`
- `web/src/components/shared/DialogShell.tsx:217-229`：
  > `role="dialog"`
  > `className="pointer-events-auto flex max-h-[85vh] w-full flex-col overflow-hidden ..."`

本体不再有宿主父容器链；Portal 的 DOM 父级是 `document.body`。逻辑上从 ChatCockpitPage 调用时的 React 组件链是 `ChatCockpitPage → AutomationTerminationConfirmSurface → DialogShell`，但它不产生对应的 DOM 内联祖先链。对话框内部：fixed layer（`z-[80]`）→ 全屏 pointer-events-none 居中壳 → `pointer-events-auto` section。`max-h-[85vh] overflow-hidden` 约束卡片高度/内容溢出，不会导致父 grid/flex 挤压；没有 `relative` 定位需求。

**z-index 覆盖风险低**：确认层 `z-[80]`，由 `document.body` portal 绘制；页面内 sibling 的常规层叠不再能覆盖它。`WorkbenchOverlayDrawer` 自身为 `z-50`（`web/src/components/shared/WorkbenchOverlayDrawer.tsx:49-54`），低于确认面 z80。代码核实的三处挂载中未发现高于 z80 的兄弟 overlay；未对应用运行时所有全局浮层/扩展 DOM 作浏览器枚举，故不将“所有运行时绝无冲突”作绝对保证。

## Q3｜三个挂载点分别判断

三处组件调用都可保留在宿主位置：组件真正输出 portal 到 body，故宿主父容器的 `overflow`、`min-h-0`、flex/grid 尺寸及后续兄弟覆盖不再剪裁/覆盖确认卡片。

1. **驾驶舱 `ChatCockpitPage`**
   - 宿主组件父级为右列 `<section data-testid="cockpit-conversation-flow">`，其上 `cockpit-main-columns` flex row → `cockpit-main-region` flex column → 页面根 flex column。右列类 `flex min-h-0 min-w-0 flex-1 flex-col ...`；它无条件调用确认组件。源码：`web/src/pages/ChatCockpitPage.tsx:1120-1129`、`:1234-1244`。
   - 原文：`className="flex min-h-0 min-w-0 flex-1 flex-col ..."`；`<AutomationTerminationConfirmSurface />`。
   - **挤压/兄弟遮挡：否**（portal 脱离此流）。**可点：是，代码结构上**（pending request 下进入 fixed z80 dialog，卡片 pointer-events-auto）。
2. **Coding Workspace `CodingWorkspacePage`**
   - 确认面调用在页面主布局中 header/终止横幅之后，且与宿主布局同级排布；入口在 header 内独立工具行。源码：`web/src/pages/CodingWorkspacePage.tsx:1007-1040`、`:1041-1052`。该页面即使有 `overflow` 或兄弟块，也不会压缩 portal 浮层。
   - 原文：`<AutomationTerminationConfirmSurface />`（1051）；其前有 `AutomationSessionInvalidHint`，后有页面普通内容块。
   - **挤压/兄弟遮挡：否**。**可点：是，代码结构上**。按钮仅当托管自动化且未终止时显示（详 Q4）；门 blocked 不会抑制该终止按钮。
3. **Workbench Inbox Overlay `WorkbenchInboxOverlay`**
   - `WorkbenchInboxContent` 是抽屉内容子项：`flex min-h-0 flex-1 flex-col overflow-y-auto px-3 py-2`，确认组件调用是第一个内容节点。源码 `web/src/components/lifecycle/WorkbenchInboxOverlay.tsx:222-229`。外层 `WorkbenchOverlayDrawer` layer 是 `fixed inset-0 z-50`，drawer section 为 `absolute ... flex h-full min-h-0 flex-col`（`:48-85`）；本确认面 portal 后不受抽屉滚动容器剪裁。
   - 原文：`className="flex min-h-0 flex-1 flex-col overflow-y-auto overscroll-contain px-3 py-2"`；`<AutomationTerminationConfirmSurface />`。
   - **挤压/兄弟遮挡：否**。**可点：是，代码结构上**；当 inbox drawer 关闭时，此挂载点的 drawer 内容会被 `open=false` 的外层状态隐藏/禁交互，但全局 `ChatCockpitPage`、Coding Workspace 挂载点仍可能响应同一 pending request；而单独只挂载 drawer 的场景要在 drawer 打开时才可经此挂载呈现。**注**：WorkbenchInboxOverlay 注释指明分诊卡 abort 428 可打开此确认面，确认组件本身全局 store 驱动。

## Q4｜入口按钮挂载、条件与指路落屏

**入口按钮调用点共三个 JSX 实例、两种页面宿主**（ChatCockpitPage 一个、CodingWorkspacePage 一个；如果计入不同源码分支/重复挂载点则由 grep 确认下列两个文件的全部引用）：

- 驾驶舱：`web/src/pages/ChatCockpitPage.tsx:829-832` 定义 eligibility，`isCurrentSession && sessionManagedByAutomation && state.workspaceType === "work_item_plan"`；`:1230-1232` 只有 `cockpitTerminateEligible && cockpitTerminatedAt === null` 才渲染入口。
  > `const cockpitTerminateEligible = isCurrentSession && sessionManagedByAutomation && state.workspaceType === "work_item_plan";`
  > `{cockpitTerminateEligible && cockpitTerminatedAt === null ? (`
  > `<AutomationTerminationEntryButton onOpen={handleOpenAutomationTermination} />`
  > `) : null}`
- Coding Attempt：`web/src/pages/CodingWorkspacePage.tsx:175` 以 `store.automation?.owner === "server"` 判为托管；`:1034-1038` 仅 `automationManaged && terminationTerminatedAt === null` 渲染。旁边注释明确门阻塞时仍可达。
  > `const automationManaged = store.automation?.owner === "server";`
  > `/* ... 托管且未终止 ...（门阻塞时同样可达） */`
  > `{automationManaged && terminationTerminatedAt === null ? (`
- 按钮实际本身没有 eligibility 条件，只调用 `onOpen`：`web/src/components/chat-workspace/cockpit/CockpitInboxWaitingCards.tsx:64-78`。
  > `onClick={onOpen}`
  > `{PLAN_TERMINATION_ENTRY_LABEL}`
- F-C 原始笔记说挂载三处是确认面，不是入口按钮；当前代码检索显示入口按钮在上述两个页面宿主，WorkbenchInboxOverlay 没有入口按钮。确认面第三挂载点是共享抽屉内。

**被从 Work Item Plan 配置弹窗指路后最可能落屏**：由 `ChatCockpitPage` 的当前托管 `work_item_plan` 会话（总览驾驶舱会话页）承接最自然，因为该页面入口条件明确要求 `workspaceType === "work_item_plan"` 且当前 session 为 server-managed。页面代码不要求 pending choice，因此没有 pending choice 也照样可显示；pending choice 只影响 notice，不控制入口。

**入口不保证无条件可见**：驾驶舱按钮要求“当前 session + server托管 + work_item_plan + 未终止”；其中任一不满足则按钮不出现。特别是用户落在非托管/普通会话，或不是 plan 会话，按钮不出现；若身份地址 `planIssueAddress` 缺失，点击 handler 会尝试 `findWorkspaceSessionIssueAddress(sessionId)`，无法解析则直接 return、不会打开确认面（`:833-840`）。

Coding Attempt 页要求 `automation.owner === "server"` 且未终止，未托管/已终止时按钮不出现；`pendingGate?.kind === "blocked"` 只会隐藏 `ActionButtons`，终止按钮位于该条件分支之后，因此仍显示（`:1021-1038`）。确认面不是入口按钮自动触发；用户须点击入口。

## Q5｜结论与建议

**一句话结论：F-C 当前不再成立，当前代码路径不会再因旧 grid 行模板把“终止整个自动化”确认面压到 17px 并被对话流覆盖；只要确认请求已打开，确认面以固定视口 modal 形态可见且代码结构上可点。**

因此 F-C 可从待处理批次 N 移除/降级为“修复已落地、待真机回归验证”；不应再把旧的“确认面看不见、点不到”作为块1指路目标的阻断理由。仍需注意的是**入口可用性受托管 plan 身份/终止状态约束**：块1 指路应落到当前仍满足 `isCurrentSession && sessionManagedByAutomation && workspaceType === "work_item_plan"` 的驾驶舱会话；若 owner 可能从配置弹窗落到非托管或普通会话，则指路文字需附条件/提供 Coding Workspace 另一入口，而不是声称所有落屏情况入口必然存在。没有证据支持把旧 F-C 缺陷重新并入块1，也无需再建议 fixed 化——fixed Portal 已实施。若后续补验发现实际浏览器仍有可见/交互故障，最小修法才应针对新证据；当前源码无需按旧方案再改。

## 检索范围与命令

- 工作目录确认：`cd /home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02 && pwd`（输出见首行）；`codegraph init`（已存在初始化）。
- 结构/调用链：CodeGraph 查询 `AutomationTerminationConfirmSurface`、`AutomationTerminationEntryButton`、`ChatCockpitPage`、`CodingWorkspacePage`、`WorkbenchInboxOverlay`、`DialogShell`、`WorkbenchOverlayDrawer`、`cockpitTerminateEligible`、`cockpitTerminatedAt`。
- 精确引用搜索：`grep`（范围 `web/src`，模式 `cockpitTerminateEligible|cockpitTerminatedAt|AutomationTerminationConfirmSurface|AutomationTerminationEntryButton`）。
- 定向读取：`ChatCockpitPage.tsx`（825-860、970-1005、1070-1081、1099-1293、1390-1415）；`CodingWorkspacePage.tsx`（1007-1068）；`CockpitInboxWaitingCards.tsx`（57-121）；`DialogShell.tsx`（168-240）；`WorkbenchOverlayDrawer.tsx`（48-110）；原始 F-C 笔记（`cadence/notes/2026-10-25_UI复验发现_待处理噪音与总览入口形态.md`）。
- 本次未运行测试、构建或真机浏览器点击复测；未修改任何代码。
