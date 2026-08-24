# Design: workbench-pipeline-workspace-ux

## Context

见 proposal.md - Why。当前实现关键事实（已勘察确认）：

- `IssueLifecycleWorkbench.tsx` 外层为 `lg:grid-cols-[17rem_minmax(0,1fr)]`（ProjectSidebar）+ 主区 `space-y-3` 先放常驻 `LogicalCodebaseManagementPanel`，再放 `lg:grid-cols-[minmax(18rem,24rem)_minmax(0,1fr)]`（IssueCardList + IssueLifecycleDetail）。
- `IssueCardList` 内为全量 `<ul className="space-y-2">`，无高度约束与 `overflow`；`LifecycleCard` 单卡约 120px（类型 chip + 两行标题 + 两行摘要 + 徽章 + 常驻操作按钮）。
- `focusedIssueId` 决定右侧归属；`selectedCardKey` 决定卡片高亮。`handleSelectCard` 在非 issue 分支只 `setSelectedCardKey` + `openDrawer`，不同步 `focusedIssueId` 的视觉表达，导致点 Story/Design 后 Issue 行高亮消失。
- `IssueLifecycleDetail` 使用 `xl:grid-cols-3` 三等宽列展示 Story/Design/Work Item，空阶段照样占 1/3 宽。
- 数据已全量在内存（`refresh()` 一次性加载 issues + lifecycles），`POLL_INTERVAL_MS = 2_000` 轮询。
- 已有可复用资产：`IssueLifecycleWorkbenchDrawer`（peek 抽屉）、`lifecycleBlockedReason` / `workItemWaitingReason` / `groupLifecycleCards`（阶段与阻塞派生）、设计令牌 `--aria-*`。

## Goals / Non-Goals

Goals：紧凑队列（mini-graph）+ 阶段标签工作区 + 队列可折叠 + 统一选择语义 + 运维面板摘要化，全部纯前端、零 API 变更、保留既有 region/testid 契约。

Non-Goals（设计层）：不引入路由变化（无独立 Focus 页、URL 结构不变）；不引入状态管理库变更（继续 React state + 既有 Zustand drawer store）；不引入第三方虚拟列表/表格库；不改动 drawer 的内部内容结构。

## Decisions

### D1：单屏双密度（可折叠队列），而非双模式路由或永久双栏

- **选择**：外壳 `100dvh` + flex；队列（左，`w-72` 级）与工作区（右，弹性）各自 `min-h-0 overflow-y-auto`；队列可折叠为细轨（仅图标/计数）或隐藏，工作区随之占满。折叠状态存于内存 Map（按 projectId）+ `localStorage` 持久化。
- **替代方案 A（Inbox↔Focus 双模式路由/视图切换）**：被否——复刻 Copilot coding agent 2025-10 改版前「页面间跳转」痛点，巡检多个并发 Issue 时失去全局视野；且深链/刷新恢复语义复杂。
- **替代方案 B（永久固定双栏 + 独立滚动）**：被否——只治滚动症状，不解决密度与文档宽度；Aria 已有 17rem Project Sidebar，再加永久 Issue 栏形成三层导航。
- **依据**：Jira 可折叠 filter panel、VS Code 侧栏 ⌘B、Copilot agents panel 的共识——折叠是同一屏内的密度切换，不是模式切换。

### D2：行内 4 阶段 mini-graph，而非大卡片或行内展开

- **选择**：新增 `IssueQueueRow` 组件（约 44–52px）：阶段 pip 链（Issue→Story→Design→Work Item，pip 四态：完成/进行/未开始/阻塞，Coding 状态并入 Work Item pip 的进行态表达）+ 单行截断标题 + 状态 chip + 计数徽章；「生成 Story Spec」与删除收进 hover/kebab。阶段判定复用 `groupLifecycleCards` 输出 + `lifecycleBlockedReason`。
- **替代方案（泳道矩阵 + 行内展开）**：被否——CI 产品无一行内展开承载产物；Patternfly/NN/g 明确 task-heavy/文档内容须升级为 panel/全页；展开态有可访问性陷阱。
- **依据**：GitLab Pajamas mini pipeline graph——「行内紧凑 stage 状态、失败/阻塞优先」与我们 4 阶段顺序链同构。

### D3：阶段标签工作区，而非三等宽列

- **选择**：`IssueLifecycleDetail` 重构为：吸顶 Issue 头（标题/ID/状态 chip）+ `StageStepper`（四段，计数+状态色，可点击切换）+ 单阶段内容区（Story/Design/Work Item tab，内容占满宽度）。默认阶段 = 「需要动作的最早阶段」（无 Story→Story；有 Story 无 Design→Design；其余→Work Item）。Work Item 仓库分组 `WorkItemRepositoryGroupSection` 在阶段页内全宽展示。「生成下一阶段」主按钮常驻阶段页头部；完整文档通读仍走现有 `IssueLifecycleWorkbenchDrawer`。
- **替代方案（保留三等宽列仅加滚动）**：被否——文档型产物需要阅读宽度；空阶段白占 1/3；顺序关系被并列布局稀释。
- **依据**：Devin session 的可点击 plan 步骤 + 标签页（Changes/Worklog/…）结构。

### D4：统一选择语义（归属唯一源）

- **选择**：队列行 `focused` 视觉 = `card.issueId === focusedIssueId`，加 `aria-current="true"` 与 3px 左侧主色条 + `--aria-primary-soft` 底；`selectedCardKey` 仅传给工作区内部实体卡片（主色 ring）。`handleSelectCard` 非 issue 分支补 `setFocusedIssueId(card.issueId)`（不变式修复）。
- **依据**：master-detail 选择语义——master 侧必须始终标识「当前详情归属于谁」（LangSmith side panel「keeps the surrounding context」）。

### D5：分组与过滤全部前端派生，渲染上限 50

- **选择**：`deriveIssueQueue(lifecycles)` 纯函数：输入现有 `IssueLifecycleResponse[]`，输出分组（待生成 Story / 待生成 Design / 待拆 Work Item / 编码中 / 阻塞 / 已完成）+ 每组行数据（含 mini-graph 状态）。文本过滤在同一函数前置谓词。每组渲染上限 50 + 「显示更多」本地计数。
- **替代方案（虚拟列表）**：暂缓——10/100 量级纯渲染足够；>300 条实测卡顿再窗口化（YAGNI）。
- **依据**：数据已全量在内存，瓶颈是 DOM 与视觉扫描而非数据获取。

### D6：LogicalCodebaseManagementPanel 摘要外壳

- **选择**：新增 `LogicalCodebaseSummaryBar`：一行（LC 选择 chip + 索引状态 + 发布状态 + 「管理」展开按钮）；异常（索引 missing/stale、发布 failed/partial）时 amber/rose 警示样式。展开后渲染现有 `LogicalCodebaseManagementPanel` 全量内容，面板内部零改动；展开状态按 Project 记忆。
- **依据**：低频运维能力不应常驻首屏（竞品一致：Devin/Copilot 把运维收敛进二级入口）。

### D7：轮询刷新上下文冻结

- **选择**：`refresh()` 已用 `refreshRequestId` 防抖；本设计补充约束——轮询路径不重置 `focusedIssueId`/`selectedCardKey`/队列折叠 Map/滚动位置（现状基本满足，仅 `focusedIssueId` 在 issue 仍存在时保留，需回归测试锁定）。列表渲染以 `lifecycleCardKey` 为 React key，避免整体重挂载。仅 `focusEntityKey` 深链进入时 `scrollIntoView({block:"nearest"})`。

### D8：视觉令牌沿用既有 `--aria-*` 体系

- **选择**：沿用现有 CSS 变量（`--aria-bg/panel/panel-muted/line/ink/ink-muted/primary/danger`），阶段 pip 颜色复用现有类型色（sky/emerald/violet/amber），状态一律胶囊 chip；hover 仅 `transition-colors` 150–300ms；`@media (prefers-reduced-motion: reduce)` 下禁用位移动画。不引入新字体/新色板。
- **依据**：ui-ux-pro-max Data-Dense Dashboard 建议与仓库既有视觉系统一致；避免引入第二套设计语言。

## Risks / Trade-offs

- [既有测试大量依赖当前 DOM 结构（三列区域、`selected-issue-preview`、卡片 testid）] → 保留全部 region 名与 testid；阶段 tab 内仍渲染 `Story Spec 内容`/`Design Spec 内容`/`Work Item 内容` 区域；`selected-issue-preview` 保留在 Issue 头。测试按需补充而非大面积重写。
- [阶段 tab 一次只看一个阶段，跨阶段对比需要点 tab] → 步进器常驻各阶段计数与状态，切换成本一次点击；验收场景锁定默认阶段规则。
- [队列折叠记忆用 localStorage，多标签页可能不一致] → 接受（与现有 UI 偏好粒度一致，不影响数据正确性）。
- [mini-graph 阶段判定规则与用户心智不符] → 规则集中在 `deriveIssueQueue` 纯函数，表驱动单测覆盖全部分支。
- [50 条上限在单组超量时藏住条目] → 「显示更多」入口 + 计数明示；文本过滤可定位任意条目。

## Migration Plan

纯前端 UI 改造，无数据迁移。实施顺序（详见 tasks.md 与 cadence/plans/ Plan）：派生层（D5/D4）→ 队列组件（D2/D1）→ 工作区重构（D3）→ 运维摘要条（D6）→ 视觉打磨（D8）。回滚 = git revert，无状态残留（localStorage 键为新增，旧版本忽略之）。

## Open Questions

- 队列细轨折叠态的具体宽度与图标集在实现时按视觉走查微调（不影响验收）。
- 「编码中」分组与「可编码」的边界以 `latest_attempt.status` 现状枚举为准，实现时在 `deriveIssueQueue` 单测中冻结。
