# Design

## Context

现有 Web 已有可复用的向导与弹窗形态：`LogicalCodebaseRegistrationWizard` 使用分步状态与对话框交互，`WhatsNewDialog`/`useWhatsNew` 以集中配置和 localStorage 版本键控制首次展示，`IssueLifecycleWorkbenchView` 通过 `WorkbenchSurface` 承载 Project、队列、阶段工作区，`IssueLifecycleWorkbenchHeader` 已提供 shell 头部动作挂点。生命周期工作台的 `ProjectSidebar`、`IssueQueue`、`IssueQueueRow`、`StageStepper`、计划审批组件以及 Coding Workspace 的 `CodingWorkspacePage`/`CodingWorkspaceGroupProgress` 已有可定位的局部 testid；部分目标区域仍需补稳定属性。

本 change 只规划外围引导，不改业务动作和受保护驾驶舱模块。背景计划 §3 已确认本期覆盖手动模式，自动化选择步骤通过 feature-gate/late-bind 等 P1 锚点稳定后再配置启用。

## Goals / Non-Goals

**Goals:**

- 用一个无第三方依赖的 React wizard 展示配置过滤后的步骤、当前位置、说明、示意资源和锚点高亮。
- 固定设备级存储键 `aria-onboarding-seen`，支持首次自动展示、跳过/完成记忆和 shell 帮助入口重开。
- 以集中配置承载 13 个手动模式步骤、锚点、资源、说明和启用条件；让自动化步骤可 late-bind 而不改变渲染器。
- 通过 `data-testid` 契约和缺失锚点降级确保高亮逻辑不依赖脆弱的 CSS 选择器或业务内部状态。
- 为实现 worker 提供明确的锚点映射、受保护文件边界和验收口径。

**Non-Goals:**

- 不实现自动化模式选择、autopilot 行为、通知/收件箱、会话投影或 Coding 业务动作。
- 不修改 API、后端、WebSocket 协议和数据模型；不把引导步骤推进绑定到业务 API 成功回调。
- 不新增 tour/截图第三方库，不重写现有 Project、Issue、计划或 Coding 组件；不修改 `useCockpitAutopilot`、`workspace-cockpit-projection`、`CockpitInbox`、`ChatCockpitPage` 内部。

## Decisions

### 1. 组成结构与接入边界

新增 onboarding 专用目录（建议 `web/src/onboarding/`）承载：

- `steps.ts`：纯配置与类型，导出步骤定义和过滤函数；
- `useOnboarding.ts`：localStorage 读写、首次判定、open/close/skip 状态；
- `OnboardingWizard.tsx`：遮罩/说明卡/高亮层/进度和按钮；
- 相关单测：配置过滤、存储降级、步骤顺序、跳过/重开、锚点缺失。

`AppShell` 在工作台 shell 层调用 hook 并渲染 wizard，提供帮助按钮回调；`WorkbenchSurface` 不承载 onboarding 业务状态，必要时只保留语义挂点或由 `AppShell` 在外层叠加入口。这样引导不会进入生命周期状态机，也不会触碰驾驶舱内部。

高亮采用 DOM `data-testid` 查询（例如 `[data-testid="..."]`）和一个 fixed/absolute highlight rectangle；使用 `getBoundingClientRect`，窗口滚动/resize 时更新。若元素不存在，说明卡仍可见并显示“当前区域尚未出现，完成前置步骤后继续”的非阻塞状态；不自动触发业务操作、不向后端发请求。组件在关闭时清理监听并移除高亮效果。

### 2. 配置 schema 与步骤定义

步骤配置至少包含：

```ts
type OnboardingStep = {
  id: string;
  order: number;
  title: string;
  description: string;
  visual: { kind: "screenshot" | "diagram"; src?: string; alt: string };
  anchorTestId: string;
  gate?: {
    feature: string;
    enabled: boolean;
    lateBind?: () => boolean;
  };
};
```

`getEnabledOnboardingSteps(context)` 先按 `enabled` 与可选 `lateBind` 过滤，再按 `order` 排序；进度、上一步/下一步边界均使用过滤后的数组。配置声明一个 `automation-mode-selection` 步骤但本期 `enabled: false`，并在配置注释/测试中锁定该步骤被跳过；P1 只需翻转配置及其 late-bind 锚点条件，不能要求渲染器增加分支。

本期 13 步定义如下（`id` 为稳定配置标识，锚点名是实现任务必须满足的契约）：

| 顺序 | id | 目标区域 / 推荐 `data-testid` | 说明重点 |
|---:|---|---|---|
| 1 | `create-project` | `onboarding-anchor-project-create`（Project 新建按钮，仅补属性） | 创建 Project 作为流程容器 |
| 2 | `add-single-repository` | `onboarding-anchor-codebase-add`（添加代码库按钮，仅单库路径） | 本期只绑定一个物理代码库 |
| 3 | `create-issue` | `onboarding-anchor-issue-create`（新建 Issue 按钮） | 填写标题、代码库与基准分支 |
| 4 | `generate-story` | `onboarding-anchor-stage-story`（Story 阶段/生成入口） | 生成 Story Spec |
| 5 | `confirm-story` | `onboarding-anchor-story-confirm`（Story 确认动作） | 检查内容后确认 |
| 6 | `generate-design` | `onboarding-anchor-stage-design`（Design 阶段/生成入口） | 基于 Story 生成 Design |
| 7 | `confirm-design` | `onboarding-anchor-design-confirm`（Design 确认动作） | 检查内容后确认 |
| 8 | `generate-work-item-plan` | `onboarding-anchor-stage-work-item-plan`（Work Item/Plan 区域） | 生成 Work Item Plan |
| 9 | `confirm-work-item-plan-review` | `onboarding-anchor-plan-review-gate`（计划审批/第一道门） | 通过第一道计划审阅门 |
| 10 | `confirm-work-item-plan-final` | `onboarding-anchor-plan-final-gate`（计划最终确认/第二道门） | 通过第二道最终确认门 |
| 11 | `enter-coding-workspace` | `onboarding-anchor-coding-workspace-entry`（进入 Coding 入口） | 从已确认 Work Item 进入 Coding Workspace |
| 12 | `start-coding` | `onboarding-anchor-coding-start`（启动 Coding 动作） | 显式启动 Coding |
| 13 | `coding-progress-complete` | `onboarding-anchor-coding-progress`（进度/完成确认区域） | 查看进度并完成最终确认 |

若对应现有组件已经有语义稳定 testid（如 `issue-queue-row`、`stage-stepper`、`cockpit-plan-approval-panel`、`coding-group-progress` 等），实现可以复用；表中 `onboarding-anchor-*` 表示需补上的专用稳定属性，不能修改原有行为或删除既有 testid。计划两道门必须是两个不同锚点，避免单一元素承载两步。

截图/示意以本地静态资源或配置中的示意描述为主，资源缺失时显示可访问的 CSS/文字示意，不阻断步骤；配置不允许把业务 JSX 或运行时截图生成逻辑塞进步骤定义。

### 3. 存储与生命周期

存储常量固定为：

```ts
export const ONBOARDING_SEEN_KEY = "aria-onboarding-seen";
```

读取遵循 `useWhatsNew` 的 try/catch 先例：首次检查失败返回 unavailable，自动 open 为 false；写入失败静默。`skip` 和完成最后一步都调用同一 `markSeen`，帮助入口执行 `reopen`（只更新内存 `open` 和 index，不删除存储记录），因此重开不会影响首次判定。组件卸载/路由离开时不应把未完成状态误记为已读，只有 skip、关闭或显式完成才记忆。

### 4. 锚点契约与跨页面策略

工作台内的 Project、代码库、Issue、Story/Design/Work Item 阶段、计划审批门禁使用当前 `/workbench` DOM；Coding Workspace 路由是独立页面，进入步骤只提示用户使用已有入口，高亮仅在当前 DOM 可见时生效。引导组件不跨路由持有业务状态，也不依赖受保护页面内部实现。若实现需要在路由切换后继续引导，采用 session 内存的步骤 index + 页面重新测量，不新增持久化业务状态；页面无目标锚点时按缺失锚点降级。

帮助入口必须有明确的 `aria-label`（如“重新打开操作引导”）和稳定 testid（如 `onboarding-help-trigger`）；wizard 容器、当前步骤、进度、跳过/上一步/下一步/完成按钮也必须有稳定 testid，便于验证。

### 5. 锚点补齐任务边界

允许补属性的区域包括 `ProjectSidebar`/`IssueLifecycleWorkbenchHeader` 的 Project、代码库、Issue 入口，生命周期阶段/卡片与计划门禁面板，以及 `CodingWorkspacePage` 的进入/启动/进度呈现。禁止为寻找锚点修改 `useCockpitAutopilot`、`workspace-cockpit-projection`、`CockpitInbox`、`ChatCockpitPage` 内部；如果自动化选择锚点位于 P1 受保护模块，则只在本期保留禁用配置，不添加绕行实现。

## Flow / State

1. `AppShell` 挂载 → `useOnboarding` 读取 `aria-onboarding-seen`。
2. 无已读记录且有启用步骤 → `open=true,index=0`；否则保持关闭。
3. Wizard 每次打开/步骤变更 → 读取 anchor 元素并更新高亮矩形；缺失则显示说明与降级提示。
4. 下一步/上一步只改变 index；到最后一步点击完成或点击跳过 → 写入 seen、关闭。
5. 帮助入口 → 内存重开、index=0，不清理任何业务 store、路由或 API 数据。

## Risks / Trade-offs

- **路由跨页锚点不稳定：** Coding 页面与工作台不是同一 DOM。采用缺失锚点降级和局部重测量，避免引导为了高亮而耦合路由或执行业务导航。
- **现有组件缺少目标属性：** 需在实现任务中逐项补齐并以测试锁定；只增加 `data-testid`，避免复用易变的文案/CSS 选择器。
- **配置步骤与实际页面进度可能不同步：** 配置只描述解释与定位，不自动推进实际状态；缺失锚点给出提示，维护者可在 P1 变更 gate/anchor 而不改渲染器。
- **localStorage 隐私/可用性限制：** 存储仅保存已读标记，不保存 Project、Issue 或会话内容；不可用时静默降级，帮助入口仍提供手动尝试。
- **遮罩可能妨碍操作：** 高亮层不捕获业务点击，说明卡提供非阻塞的下一步/跳过控制；焦点管理仅作用于 wizard 控件，关闭时恢复页面可用性。
