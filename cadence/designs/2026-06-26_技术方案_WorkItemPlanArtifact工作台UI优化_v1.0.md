# WorkItemPlan Artifact 工作台 UI 优化技术方案

## 文档信息

- 日期：2026-06-26
- 版本：v1.0
- 状态：待用户 Review
- 适用分支：feat-b-0616
- 关联范围：Work Item Plan Workspace 的 Artifact 展示与确认体验

## 背景

Work Item Plan 已进入两阶段生成与逐项确认流程：先生成 Work Item Plan Outline，再按串行或批量方式生成 Work Item Draft，最后 compile 成实际 Work Item、Verification Plan 和 child sessions。当前前端 Artifact 面板能够展示 typed artifact 的原始字段，但界面仍像“当前 artifact 文本预览”，不能承担 Work Item Plan 验收所需的工作台职责。

用户在端到端测试中反馈：

- 页面无法清楚回答“整个 work item 是否全部写完”。
- Artifact 区域没有把 Work Item Plan Outline 作为核心内容展示。
- Work Item Draft 展示不清晰，难以判断当前只生成了单个 draft，还是整组 draft 已完成。
- 版本控制只能切换历史版本，无法对比变更。
- 排版偏散，信息密度和层级不符合操作型开发工具。

## 现状分析

当前 Work Item Plan Artifact 主要由以下文件承担：

- `web/src/components/workspace/WorkItemPlanArtifactPanel.tsx`
- `web/src/components/workspace/WorkItemPlanStagedPanel.tsx`
- `web/src/pages/ChatWorkspacePage.tsx`
- `web/src/state/work-item-plan-artifact-summary.ts`
- `web/src/api/types.ts`

现有 `WorkItemPlanArtifactPayload` 已经包含足够的数据：

- `outline_candidate`：Outline、依赖图、write scopes、forbidden scopes、风险、validator findings。
- `draft_candidate`：单个 draft record、implementation context、verification plan、handoff summary、validator findings、can_accept。
- `batch_state`：batch queue、draft records、failure summary、batch status。
- `compile_report`：compile status、plan commit state、work item ids、verification plan ids、child session ids。
- `context_blocker`：上下文阻塞项、需要补充的上下文和允许动作。

主要问题不在后端 contract，而在前端信息架构：

- Artifact 类型被平铺渲染，缺少阶段语义。
- 版本 rail 是横向按钮列表，不表达 Outline、Draft、Batch、Compile 的流程关系。
- Draft card 是长文本堆叠，不适合检查多个 Work Item Draft。
- 没有同类型版本对比能力。
- `Current`、`vN`、`只读历史` 等状态存在，但没有转化为用户可决策的完成度提示。

## 设计目标

本次 UI 优化的目标是把 Work Item Plan Artifact 改造成“计划编排工作台”。

设计必须满足以下要求：

- 清楚表达当前 Work Item Plan 是否完成，以及完成到哪个阶段。
- Outline 必须作为一等页面展示，便于检查拆分结果、依赖关系和写入范围。
- Draft 必须能区分单个 draft、多个 draft、batch draft 和最终 compile 完成态。
- 版本控制必须支持同类型结构化对比。
- 当前流程动作必须和当前阶段对齐，历史版本只读。
- 布局应偏数据密集型开发者工具，不使用营销式卡片和过度装饰。
- 保持 Story Spec、Design Spec Artifact 现有路径不受影响。

## UI 设计系统约束

基于 `ui-ux-pro-max` 检索，本界面采用数据密集型开发工具风格。

视觉原则：

- 使用扁平化、低阴影、清晰边框的 dashboard 风格。
- 信息密度高，但分区明确，默认字体尺寸以 12px 到 14px 为主。
- 卡片圆角不超过 8px，不做卡片嵌套。
- 使用表格、分段控件、状态标签、展开行、版本 rail 和 diff 面板。
- 使用 lucide icons 表示动作和状态，不使用 emoji。
- 主要颜色以中性底色为主，状态色用于语义：
  - 绿色：完成、committed、accepted。
  - 蓝色：当前选择、active。
  - Amber：optional findings、review pending。
  - 红色：blocking、failed、validation failed。
- 所有交互控件必须有 visible focus state。
- 动态状态变更使用 `aria-live="polite"` 或等价机制。
- 表格在小屏幕下必须水平滚动或切换 compact card，不能造成页面横向溢出。

## 完成状态语义

界面必须明确区分以下状态：

| Artifact 类型 | 用户语义 | 是否代表整个 Work Item 写完 |
| --- | --- | --- |
| `outline_candidate` | Plan Outline 已生成，等待确认或返修 | 否 |
| `draft_candidate` | 单个 outline 的 draft 已生成，等待确认或返修 | 否 |
| `batch_state` / `generating` | 正在批量生成 draft | 否 |
| `batch_state` / `review_pending` | 一批 draft 已生成，等待用户确认 | 否 |
| `batch_state` / `review_done` | draft 层面已确认 | 仍不等于最终完成 |
| `compile_report` / `committed` | Work Item、Verification Plan、child sessions 已写入 | 是 |
| `compile_report` / `recovery_required` | compile 遇到可恢复问题 | 否 |
| `context_blocker` | 缺上下文，无法继续 | 否 |

顶部状态条必须用自然语言回答当前状态，例如：

- `Outline 已生成，等待确认。Work Item 尚未生成。`
- `已生成 2 / 4 个 Draft，当前正在确认 outline_frontend。`
- `Batch Draft 已完成，等待接受全部或返修。`
- `Compile 已提交，生成 4 个 Work Item、4 个 Verification Plan、4 个 child session。`
- `正在查看历史版本 v2，不影响当前流程。`

## 信息架构

Artifact 页改为三层布局：

1. 顶部状态条
2. 左侧阶段/版本 rail
3. 主内容区 tabs

### 顶部状态条

顶部状态条固定在 Artifact 面板内，负责汇总当前流程状态。

展示字段：

- Plan 标识：`plan_id`
- Generation round：`generation_round_id`
- 当前阶段：`Outline`、`Drafting`、`Batch`、`Compile`、`Committed`、`Blocked`
- Draft 进度：`generated / accepted / failed / total`
- Review findings 摘要：blocking、optional、minor、suggestion 数量
- 当前版本：`Current` 或 `vN`
- 历史只读提示

状态条下方显示当前阶段 action bar。action bar 不在历史版本下展示。

### 阶段/版本 Rail

左侧 rail 替代现有横向版本按钮。

分组规则：

- Outline
- Drafts
- Batch
- Compile
- Blockers

每个版本项展示：

- 类型标签：Outline、Draft、Batch、Compile、Blocker
- 版本号：`v1`
- 对象标识：
  - Outline：`Plan Outline · 4 items`
  - Draft：`outline_frontend / draft_frontend_002`
  - Batch：`batch_001 / review_pending`
  - Compile：`compile_001 / committed`
- 状态标签：current、active、readonly、accepted、failed
- 生成时间或相对时间

交互规则：

- 点击版本项只切换查看内容，不改变后端流程。
- 当前 active node 对应版本高亮。
- `is_current` 版本展示 current 标签。
- 缺少 artifact 内容的版本禁用，但仍展示摘要，避免历史断裂。

### 主内容 Tabs

主内容区使用分段控件：

- `Overview`
- `Outline`
- `Drafts`
- `Diff`
- `Review`
- `JSON`

默认 tab 规则：

- 当前 artifact 是 `outline_candidate`：默认 `Outline`。
- 当前 artifact 是 `draft_candidate` 或 `batch_state`：默认 `Drafts`。
- 当前 artifact 是 `compile_report`：默认 `Overview`。
- 当前 artifact 是 `context_blocker`：默认 `Review` 或 `Overview`。
- 用户手动切换 tab 后，在当前 session 内保留选择。

## Overview Tab

Overview 回答“现在整体进度怎样”。

内容模块：

- 阶段 stepper：
  - Outline
  - Drafts
  - Batch Review
  - Compile
  - Committed
- 进度摘要：
  - Outline items 数量
  - Draft records 数量
  - Accepted draft 数量
  - Failed draft 数量
  - Work item ids 数量
  - Verification plan ids 数量
  - Child session ids 数量
- 当前待办：
  - `等待确认 Outline`
  - `等待选择生成模式`
  - `等待确认 Draft outline_frontend`
  - `等待接受 Batch`
  - `需要 Compile recovery`
- 最新 findings 摘要。

Overview 不展示大段 JSON，不展示完整 implementation context。

## Outline Tab

Outline 是核心验收页，必须完整展示 Work Item Plan Outline。

顶部 summary：

- `strategy_summary`
- `status`
- `current_generation_round_id`
- items 数量
- dependency edges 数量
- risks 数量

Outline 主表字段：

- 序号
- `outline_id`
- `title`
- `kind`
- `depends_on` / `depends_on_outline_ids`
- `exclusive_write_scopes`
- `forbidden_write_scopes`
- `verification_intent`
- `risk_notes`

每行可展开展示：

- `goal`
- `scope`
- `non_goals`
- `required_handoff_from_outline_ids`
- `handoff_notes`
- `context_budget`
- source story/design spec ids

依赖关系模块：

- 使用紧凑列表展示 `from -> to`。
- 若存在依赖但缺少 handoff，展示 amber warning。
- 若出现 exclusive write scope overlap，展示红色 warning。

写入范围模块：

- 按 outline 展示 exclusive 和 forbidden scopes。
- 对跨 outline scope overlap 做前端可视检查。
- 对 integration test outline 写入 `web/src/**` 这类高风险范围给出 warning。

## Drafts Tab

Drafts 用于检查 Work Item Draft 是否可执行。

如果当前 artifact 是 `draft_candidate`：

- 展示单个 draft 的详情。
- 顶部明确提示：`当前仅展示单个 Draft，不代表整组 Work Item 完成。`

如果当前 artifact 是 `batch_state`：

- 展示 draft 列表和选中 draft 详情。
- 列表支持按 status、kind、outline_id 浏览。

Draft 列表字段：

- `outline_id`
- `draft_id`
- `title`
- `kind`
- `status`
- `attempt_index`
- `generation_mode`
- `can_accept`
- findings 数量

Draft 详情字段：

- title、kind、status、active/superseded
- goal
- implementation context
- depends_on / required handoff
- exclusive write scopes / forbidden write scopes
- verification plan
  - commands
  - required gates
  - manual checks
  - risk notes
- handoff summary
- validator findings

长文本字段使用 readable block，固定行高，支持折叠展开，避免单个字段撑爆页面。

## Diff Tab

Diff 是本次优化的关键能力。

选择器：

- Base version
- Compare version
- Artifact type filter

默认比较：

- 当前版本与最近一个同类型历史版本比较。
- 如果没有同类型历史版本，展示空态：`暂无可比较的 Outline/Draft 版本`。

跨类型处理：

- Outline 只能和 Outline 比较。
- Draft 只能和同一 `outline_id` 的 Draft 比较。
- Batch 只能和 Batch 比较。
- Compile 只能和 Compile 比较。
- 跨类型选择时禁用 compare，并说明原因。

Outline 结构化 diff：

- 新增 outline
- 删除 outline
- title 变化
- goal 变化
- depends_on 变化
- exclusive write scopes 变化
- forbidden write scopes 变化
- verification intent 变化
- handoff notes 变化
- risk notes 变化

Draft 结构化 diff：

- title、kind、goal 变化
- implementation context 变化
- depends_on / required handoff 变化
- write scopes 变化
- verification commands 变化
- required gates 变化
- manual checks 变化
- handoff summary 变化
- validator findings 变化
- status / can_accept 变化

展示方式：

- 字段级 diff 列表作为默认视图。
- 对长文本字段提供 before / after split view。
- JSON diff 作为折叠辅助，不作为默认。

## Review Tab

Review 汇总 validator findings、reviewer findings 和 context blockers。

分组：

- Blocking
- Optional / Suggestion / Minor
- Context blockers
- Validator findings

每条 finding 展示：

- severity
- code / finding_id
- message
- affected outline/draft/work item ids
- 建议动作

当 review verdict 是 pass 但存在 optional findings 时，Review tab 与 action bar 必须同时展示：

- `修复这些建议`
- `不修复，继续生成`

这两个动作只在当前版本和当前 `review_decision` 阶段展示。

## JSON Tab

JSON tab 保留现有 Monaco viewer。

规则：

- 默认不打开 JSON。
- 用于调试和复制原始 payload。
- 只读。
- 高度随面板稳定，不因内容长度撑开布局。

## Action Bar

Action bar 由当前 active node 和当前 artifact 类型共同决定。

规则：

- 历史版本不展示 action bar，只展示只读说明。
- 当前版本才展示动作。
- 动作文案必须对应流程阶段。
- 所有按钮使用 lucide icon，图标 `aria-hidden`，按钮有明确 accessible name。

阶段动作：

| Active node | 动作 |
| --- | --- |
| `work_item_plan_outline_confirm` | 接受 Outline、重写 Outline |
| `work_item_generation_mode` | 逐个生成、自动生成、返回 Outline 返修 |
| `review_decision` + optional findings | 修复这些建议、不修复继续生成 |
| `work_item_draft_confirm` | 接受、重写、暂停 |
| `work_item_batch_confirm` | 接受全部、整组重写、降级串行、暂停 |
| `work_item_plan_compile_recovery` | 继续、放弃并回滚、转人工 |

## 组件边界

建议拆分为以下前端组件：

- `WorkItemPlanArtifactWorkspace`
  - 负责三层布局、tab 状态、当前选择。
- `WorkItemPlanStatusBar`
  - 负责完成状态和进度摘要。
- `WorkItemPlanArtifactVersionRail`
  - 替代当前横向 rail，负责阶段分组和版本选择。
- `WorkItemPlanOverviewTab`
  - 展示整体完成度。
- `WorkItemPlanOutlineTab`
  - 展示 outline 表格、展开详情、依赖和 scope warnings。
- `WorkItemPlanDraftsTab`
  - 展示 draft 列表和详情。
- `WorkItemPlanDiffTab`
  - 展示结构化版本对比。
- `WorkItemPlanReviewTab`
  - 展示 findings 和 context blockers。
- `WorkItemPlanJsonTab`
  - 复用 Monaco viewer。
- `WorkItemPlanActionBar`
  - 从现有 `WorkItemPlanStagedPanel` 演进，集中处理当前阶段动作。

建议新增 selector/helper：

- `deriveWorkItemPlanProgress`
- `groupWorkItemPlanArtifactVersions`
- `summarizeWorkItemPlanArtifact`
- `diffWorkItemPlanArtifacts`
- `findOutlineScopeWarnings`

## 数据与后端边界

第一阶段不修改后端 contract。

前端从以下现有数据推导 UI：

- `workItemPlanArtifact`
- `workItemPlanArtifactVersions`
- `artifactVersions`
- `activeNode`
- `selectedNodeId`
- `stage`
- typed artifact payload

若后续需要更准确的全量进度，可以再让后端返回 plan-level aggregate，例如 outline 总数、accepted draft 总数、compile result summary。本设计第一阶段不依赖该新增字段。

## Story / Design Spec 影响评估

本次设计只改 `workspaceType === "work_item_plan"` 的 typed artifact UI。

Story Spec 与 Design Spec 仍走现有 markdown artifact pane：

- 不修改 `ArtifactPane` 的行为。
- 不修改 markdown artifact version load/cache。
- 不修改 Story/Design 的 review action bar。

如果后续抽象共享 version rail 或 diff 组件，必须补充 story、design、work_item_plan 三类 workspace 的表驱动回归测试。

## 可访问性要求

- 所有 tab 使用 `button` 或语义化 tab 控件。
- 当前 tab、当前版本、当前阶段不能只依赖颜色表达。
- action button 必须有明确文本或 `aria-label`。
- icon 设置 `aria-hidden="true"`。
- findings 和 protocol error 使用 `role="alert"` 或 `aria-live="polite"`。
- 版本切换和 diff 结果变化使用 polite live region。
- 表格列标题清晰，长内容可折叠。
- 键盘可访问：tab、rail、selector、action bar 均可通过键盘操作。

## 响应式布局

桌面：

- 外层保持 Timeline + 主面板布局。
- Artifact 内部使用左 rail + 主内容双栏。
- rail 宽度约 240px。

平板：

- rail 可收缩为顶部分组选择器。
- 主内容 tabs 保持横向滚动。

移动端：

- rail 变成顶部版本 selector。
- Outline 表格转为 compact card 或水平滚动表格。
- Draft 列表和详情上下排列。
- action bar 换行但按钮保持稳定高度。

## 测试策略

前端单元测试：

- `WorkItemPlanArtifactPanel` 或新 workspace 组件能渲染 Overview、Outline、Drafts、Diff、Review、JSON tabs。
- Outline tab 展示 outline 表格、scope、dependency、handoff、risk。
- Drafts tab 对单个 draft 显示“不是全量完成”的提示。
- Batch artifact 显示多个 draft 的进度与列表。
- Compile committed 显示最终完成文案和 work item ids。
- Version rail 按 Outline、Drafts、Batch、Compile 分组。
- 历史版本只读，不展示 action bar。
- Diff tab 能比较两个 Outline 版本的 scope/dependency 变化。
- Diff tab 能比较两个同 outline Draft 版本的 verification/implementation 变化。
- 跨类型 diff 显示禁用说明。
- Optional findings 阶段展示 `修复这些建议` 和 `不修复，继续生成`。

页面级测试：

- `ChatWorkspacePage` 在 `workspaceType=work_item_plan` 时使用新工作台。
- Story/Design workspace 仍使用原 markdown artifact pane。
- timeline selection 仍能定位到历史 typed artifact。
- 手动选择版本不会触发后端流程动作。

视觉与可访问性验证：

- 375px、768px、1024px、1440px 下无横向溢出。
- 所有交互控件有 focus-visible。
- 动态状态更新有可感知反馈。
- 长 implementation context 不撑开整体布局。

## 分阶段实现建议

阶段 1：结构与状态

- 新建工作台组件。
- 顶部状态条和阶段/版本 rail。
- Overview、Outline、Drafts 三个 tab。

阶段 2：Diff 与 Review

- 新增结构化 diff helper。
- 新增 Diff tab。
- 新增 Review tab 的 findings 分组。

阶段 3：Action Bar 归位与响应式 polish

- 将 staged panel 行为整合到工作台 action bar。
- 完成历史只读、focus、mobile layout。
- 补齐页面级回归测试。

## 风险与约束

- 当前 artifact version 可能只有 summary，没有 typed artifact 内容。rail 必须支持“可见但不可点击”的缺内容版本。
- Draft 的全量完成度在 `draft_candidate` 场景无法从单个 artifact 完整推导，必须以保守文案表达。
- 结构化 diff 只能比较前端已持有的版本内容；缺内容时不能自动请求除当前接口外的数据，除非实现中复用现有 artifact version load 能力。
- 不应把 `WorkItemPlanArtifactPanel.tsx` 继续扩成超大文件，应在实现时拆分组件和 helper。

## 验收标准

- 用户进入 Artifact 页后，一眼能看出 Work Item Plan 是否已完成。
- 用户能在 Artifact 页直接查看完整 Work Item Plan Outline。
- 用户能检查每个 Draft 的 scope、implementation context、verification plan 和 findings。
- 用户能对比两个 Outline 或两个 Draft 版本的结构化差异。
- 历史版本与当前版本的交互边界清楚。
- 排版符合开发工具工作台风格，信息密度高但不拥挤。
- Story Spec 与 Design Spec Artifact 行为不回归。

## 自检

- 无未决空项。
- 设计范围限定在 Work Item Plan typed artifact UI，不改变后端 contract。
- 完成状态语义已明确区分 Outline、Draft、Batch、Compile。
- Version rail、Diff、Review、Action Bar 的职责边界清楚。
- 已覆盖 Story/Design Spec 不受影响的原因。
