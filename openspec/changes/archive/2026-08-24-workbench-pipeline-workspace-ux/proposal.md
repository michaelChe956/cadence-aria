# Proposal: workbench-pipeline-workspace-ux

## Why

`/workbench`（Issue 生命周期工作台）当前的 UI/UX 存在三个已被用户确认的问题：Issues 列表使用高卡片全量平铺且没有任何滚动边界，Issue 增多时整页滚轮无限变长；`focusedIssueId`（决定右侧详情归属）与 `selectedCardKey`（决定卡片高亮）双轨并行，点击 Story/Design 后左侧 Issue 高亮消失，用户无法判断右侧内容属于哪个 Issue；三等宽阶段列 + 常驻逻辑代码库运维面板使整体信息密度与空间分配失衡。竞品调研（Devin / Copilot coding agent / LangSmith / GitLab / Patternfly / NN/g）表明：Agent 交付监督台的合理形态是「紧凑队列（行内阶段 mini-graph）+ 常驻阶段工作区」，而不是整页长列表、双模式切换或行内展开。

## What Changes

- **紧凑 Issue 队列**：Issue 卡片改为单行密度（约 44px），行内提供 Story→Design→Work Item→Coding 四阶段 mini-graph 状态链；摘要、「生成 Story Spec」按钮与删除按钮不再常驻，仅在 hover/选中/菜单中出现。
- **按「下一步动作」分组 + 吸顶筛选条**：队列按待生成 Story / 待生成 Design / 待拆 Work Item / 编码中 / 阻塞 / 已完成分组，组头显示计数、可折叠，默认折叠「已完成」；顶部提供文本过滤，全部基于已加载数据在前端派生，每组渲染上限 50 条 + 「显示更多」。
- **统一选择语义（行为修复）**：队列行高亮唯一由 `focusedIssueId` 驱动并带 `aria-current`；`selectedCardKey` 仅在工作区内标识当前检视的 Story/Design/Work Item 实体；选中任何子实体时必须同步 `focusedIssueId`，从结构上消除「点 Story 后 Issue 高亮消失」。
- **阶段工作区替代三等宽列**：右侧详情改为顶部常驻「当前 Issue」标题栏 + 阶段步进器（含计数与状态色）+ 阶段标签页（Story / Design / Work Item 一次一个占满宽度），默认落在「需要动作的阶段」；Work Item 仓库分组在阶段页内获得完整宽度；「触发下一阶段」动作常驻工作区，peek 抽屉仅保留完整文档通读场景。
- **队列可折叠（监督态 ⇄ 专注态）**：外壳改为 `100dvh`，队列与工作区各自独立滚动；队列可折叠为细轨/隐藏使工作区获得全宽，折叠状态按 Project 记忆；2s 轮询不得改变焦点、滚动位置与折叠状态。
- **逻辑代码库运维面板降级**：主区顶部的 LogicalCodebaseManagementPanel 折叠为一行状态摘要条（索引/发布状态 + 管理入口），异常时强化提示，展开后内容与现有能力一致。
- **视觉打磨**：按 Data-Dense Dashboard 方向收敛视觉——状态一律胶囊 chip、hover 仅变色不位移、过渡 150–300ms、键盘 focus 可见、支持 `prefers-reduced-motion`，图标继续使用 lucide-react。

## Capabilities

### New Capabilities

- `lifecycle-workbench-pipeline-ux`: Issue 生命周期工作台的流水线式信息架构——紧凑队列与阶段 mini-graph、下一步动作分组与过滤、统一选择语义、阶段标签工作区、队列折叠与滚动边界、运维面板降级与视觉规范。

### Modified Capabilities

（无）

## Impact

- 受影响代码：仅 `web/src/components/lifecycle/`（`IssueLifecycleWorkbench.tsx`、`IssueLifecycleWorkbenchParts.tsx`、`LifecycleCard.tsx`、新增队列/mini-graph 组件）及其 Vitest 测试；可能涉及 `web/src/components/lifecycle/LogicalCodebaseManagementPanel.tsx` 的折叠外壳。
- **不改任何 API**：不修改 `web/src/api/**`、Rust 后端、请求路径/参数/响应类型、WebSocket 协议与数据模型；分组、过滤、阶段判定全部基于现有 `groupLifecycleCards` / `lifecycleBlockedReason` / `workItemWaitingReason` 在前端派生。
- 兼容性约束：保留现有 region 名（`Issue 卡片列表` / `Issue 生命周期详情` / `Story Spec 内容` / `Design Spec 内容` / `Work Item 内容`）与既有 testid 契约；新元素使用新 testid。
- 依赖：不新增第三方依赖（不引入虚拟列表库）。

## Non-goals

1. 列表/看板双布局、saved views、命令面板、虚拟列表/服务端分页（数量级实测不足，YAGNI）。
2. 行内展开承载文档内容（设计规范明确反对 task-heavy 内容放展开区）。
3. 路由级独立 Focus 页面（复刻 Copilot 改版前「页面间跳转」痛点）。
4. LogicalCodebaseManagementPanel 的功能改造（仅折叠外壳与默认态，面板内部能力不变）。
5. Chat Workspace / Coding Workspace 页面的布局改动。
