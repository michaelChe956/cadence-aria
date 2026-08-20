# Proposal: spec-workbench-canvas-experience

## Why

Story/Design workspace 的 AuthorConfirm 体验存在三个问题：① 三动作（发送反馈/送审/定稿）平级摆放且主色随默认推荐切换，迭代操作与终局确认视觉不分层；② 审核对象是 spec 文档本身，却以对话流为主视图展示，产物被放在次要位置，用户期望"生成完成→切到产物审核视图、有问题再切回对话"的模式切换；③ 整体视觉为冷灰细边框专业风，用户期望参考 ui-ux educational-platform demo 的 Claymorphism 视觉（粗边框实体卡片、奶油底、蓝紫橙配色）提升质感。

## What Changes

- **Canvas 产物审核面板**：author_confirm 阶段右侧自动滑出产物审核面板（展示当前版本 artifact 全文），左侧执行节点栏全程保留；开始反馈输入/新一轮运行开始前面板收起；面板内嵌吸顶操作条与「本轮改动摘要」折叠条；「采纳 Review 意见」预填时自动收起面板（避免输入区被面板遮挡的交互死点）。
- **三动作分层**：终局确认对（「确认送审」「确认定稿」，保留现有文案，主/次样式按默认推荐）与「采纳 Review 意见」移入面板操作条；「发送反馈」随输入区留在对话流，不再与终局确认并排。
- **Spec 工作台视觉规范**：ui-ux-pro-max 生成全站设计规范（design-system/MASTER.md）。落地分两层：**token 层全站生效**（--aria-* 变量改值：主色 #4F46E5、新增 CTA #F97316、奶油底色——全站共享变量，其他工作台顺带获得新色系，属规范全站的自然结果）；**组件形态收敛仅落 spec 工作台**（2-3px 深色边框卡片、rounded-xl/2xl、胶囊 chip、按钮体系），克制玩具元素（无 emoji 装饰、无 hover 放大、无双层阴影、无衬线字体）。

## Capabilities

### New Capabilities

- `spec-workbench-canvas-experience`: Story/Design workspace 的 Canvas 产物审核面板（阶段驱动滑出/收起）、三动作视觉分层、以及 Claymorphism 蓝紫橙视觉规范在 spec 工作台的落地。

### Modified Capabilities

（无——`spec-design-dialog-revision` 的行为契约（决策语义/状态机）不受影响，本 change 仅改变呈现层。）

## Impact

- 受影响代码：`web/src/pages/ChatWorkspacePage.tsx`（面板布局与切换）、`web/src/components/chat-workspace/`（ArtifactPane 放大为面板、操作条组件、ChatInputBar 按钮迁移）、`web/src/styles.css`（token 变量）、`web/tailwind.config.ts`（如需扩展）、新增 `design-system/MASTER.md`。
- 非目标：story→design 级联联动（用户明确无诉求，story 变更→design 作废语义暂不需要）；**组件形态的全站推广**（coding/image-create 等其他工作台的粗边框卡片/按钮体系改造，后续按 MASTER.md 逐个推广——本 change 仅其 token 色系随变量改值联动）；逐行 diff 视图；移动端深度适配（桌面工具，仅保证基本可用）。
- 行为契约不变：spec-design-dialog-revision 的全部决策语义（Revise/AcceptWithReview/AcceptFinalize、provisional 恢复、review 回对话流）仅换呈现，不改行为。
