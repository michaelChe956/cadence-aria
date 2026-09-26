# onboarding 锚点勘察清单 v1.0

> 归属 change：`openspec/changes/web-onboarding-wizard`（Issue 2：13 步全流程引导）
> 勘察范围：`web/src` 下 13 个引导步骤目标元素当前 `data-testid` 现状与补齐决策。
> 纪律：仅新增稳定 `data-testid` 属性，不改业务动作 / 请求 / 既有 testid；受保护模块（`useCockpitAutopilot`、`workspace-cockpit-projection`、`CockpitInbox`、`ChatCockpitPage` 内部、`useCockpitAutopilot`）零改动。

## 1. 结论总览

- 引导 wizard 挂载在 `AppShell`（仅 `/workbench` 路由）；`/workbench/workspace/$sessionId`（会话页）与 `/workbench/projects/.../coding/$attemptId`（Coding 页）是独立路由。
- 因此 13 步锚点以「当前 `/workbench` DOM 可定位」为准；跨路由（Story/Design 确认、计划双门、Coding 启动/进度）目标不可见时按设计降级为纯说明 + 非阻塞缺失提示。
- 计划**第一道门**与**第二道门**是两个**不同**锚点（分别挂在 Work Item 计划区的容器与内容体），不合并为单一元素。
- 会话页的 Story/Design 确认动作与计划双门实现受保护（`ChatCockpitPage` 内部），不在其内加属性；工作台内以对应产物区/计划区作为稳定锚点。

## 2. 13 步锚点逐条清单

| # | step id | 目标元素（文件:行） | 当前 `data-testid` | 判定 | 本期处置 |
|---:|---|---|---|---|---|
| 1 | `create-project` | `ProjectSidebar` 「新建 Project」按钮（`components/lifecycle/ProjectSidebar.tsx:47`） | 无 | 缺失 | 新增 `onboarding-anchor-project-create` |
| 2 | `add-single-repository` | `ProjectSidebar` 「添加代码库」按钮（`ProjectSidebar.tsx:118`） | 无 | 缺失 | 新增 `onboarding-anchor-codebase-add` |
| 3 | `create-issue` | `IssueLifecycleWorkbenchHeader` 「新建 Issue」按钮（`IssueLifecycleWorkbenchHeader.tsx:49`） | 无 | 缺失 | 新增 `onboarding-anchor-issue-create`（基准分支说明在文案内） |
| 4 | `generate-story` | `StageStepper` Story 阶段标签（`StageStepper.tsx`） | `stage-tab-story`（按钮）、`stage-tab-pip-story` | 存在（粒度为按钮） | 新增 `onboarding-anchor-stage-story` 于阶段标签文本 span（稳定、三步常驻） |
| 5 | `confirm-story` | Story 阶段内容区容器 `LifecycleContentSection`（`IssueLifecycleWorkbenchParts.tsx`） | 无 | 缺失 | 新增 `onboarding-anchor-story-confirm`（经 `sectionTestId` 传入） |
| 6 | `generate-design` | `StageStepper` Design 阶段标签 | 同 #4（`stage-tab-design`） | 存在 | 新增 `onboarding-anchor-stage-design` |
| 7 | `confirm-design` | Design 阶段内容区容器 `LifecycleContentSection` | 无 | 缺失 | 新增 `onboarding-anchor-design-confirm` |
| 8 | `generate-work-item-plan` | `StageStepper` Work Item 阶段标签 | 同 #4（`stage-tab-work_item`） | 存在 | 新增 `onboarding-anchor-stage-work-item-plan` |
| 9 | `confirm-work-item-plan-review`（第一道门） | Work Item 计划区容器 `LifecycleContentSection` / `WorkItemRepositoryGroupSection` | 无（`work-item-repository-group-*` 为分组项） | 缺失 | 新增 `onboarding-anchor-plan-review-gate`（容器级） |
| 10 | `confirm-work-item-plan-final`（第二道门） | Work Item 计划区内容体（卡片列表 ul / 分组列表） | 无 | 缺失 | 新增 `onboarding-anchor-plan-final-gate`（内容体，**与 #9 不同元素**） |
| 11 | `enter-coding-workspace` | `LifecycleCardDrawer` 「进入 Coding Workspace」入口按钮（`LifecycleCardDrawer.tsx:423`） | `drawer-open-coding-workspace` | **已存在，复用** | 不新增重复属性；配置直接引用既有稳定 testid |
| 12 | `start-coding` | `CodingWorkspaceControls` 「开始 / 继续 Coding」按钮（`CodingWorkspaceControls.tsx:222,246`） | 无 | 缺失 | 新增 `onboarding-anchor-coding-start`；compact 底栏变体为 `onboarding-anchor-coding-start-compact`，避免双挂载重名 |
| 13 | `coding-progress-complete` | `CodingWorkspaceGroupProgress` 组进度区（`CodingWorkspaceGroupProgress.tsx:38`） | 无 | 缺失 | 新增 `onboarding-anchor-coding-progress` |

## 3. 已有可复用锚点（勘察确认）

- `stage-stepper`：阶段步进器容器（`StageStepper.tsx:44`）。
- `stage-tab-story` / `stage-tab-design` / `stage-tab-work_item`：阶段标签按钮。
- `drawer-open-coding-workspace`：抽屉「进入 Coding Workspace」入口（步骤 11 复用）。
- `cockpit-plan-approval-panel`：计划审批面板（会话页，`PlanApprovalPanel.tsx:67`；受保护边界外但非本页 DOM）。
- `confirm-twice-button`：二次确认按钮（`ConfirmTwiceButton.tsx:53`）。
- `lifecycle-card-story_spec` / `lifecycle-card-design_spec` / `lifecycle-card-work_item`：产物卡片（`LifecycleCard.tsx:86`，动态前缀）。
- `issue-queue-row`、`workbench-shell`、`selected-issue-preview`：队列与工作台容器。

## 4. 关键判定与理由

1. **两道路径独立**：Coding Workspace 页（步骤 12/13）与工作台不同 DOM，引导只提示既有入口、不自动导航；目标缺失时降级。
2. **双计划门必须不同锚点**：#9 挂 Work Item 计划区**容器**（代表计划审阅 / 第一道门视图），#10 挂该区**内容体**（卡片/分组列表，代表计划内容与最终确认）。两者始终为不同 DOM 元素，满足「不把两道门合并为一个模糊步骤」。
3. **Story/Design 确认**：真实确认动作在会话页（`ChatCockpitPage` 内部，受保护）。工作台内以 Story/Design **阶段内容区容器**为稳定锚点，说明文案引导用户经「打开 Workspace」进入确认。
4. **阶段生成入口**：`StageStepper` 三阶段标签常驻，锚点稳定；生成动作（空阶段主按钮 `生成 Story/Design Spec`、Issue 卡 `生成 Story Spec`）为条件渲染，故不作锚点。
5. **零结构改动**：所有补齐均为新增属性；`LifecycleContentSection` / `WorkItemRepositoryGroupSection` 仅新增可选 `sectionTestId` / `bodyTestId` 属性透传，行为不变。
6. **Coding 启动锚点唯一性**：`ActionButtons` 在 Coding 页挂两处（页头非 compact 主控件 + 底部 compact 变体）。非 compact 使用 `onboarding-anchor-coding-start`（引导配置引用此项），compact 使用 `onboarding-anchor-coding-start-compact`，保证同屏 testid 唯一。

## 5. 受保护边界（零改动确认）

`useCockpitAutopilot`、`state/workspace-cockpit-projection.ts`、`components/chat-workspace/cockpit/CockpitInbox.tsx`、`pages/ChatCockpitPage.tsx` 内部均不修改；自动化模式选择步骤仅保留默认关闭配置，不添加绕行 UI。
