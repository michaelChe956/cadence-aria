# spec-workbench-canvas-experience — Delta Spec

## Purpose

Story/Design workspace 的 AuthorConfirm 阶段以 Canvas 产物审核面板承载"先看产物、再对话"的审核体验，三动作视觉分层，整体视觉迁移到 Claymorphism 蓝紫橙规范。

## ADDED Requirements

### Requirement: Canvas 产物审核面板

Story/Design workspace 进入 AuthorConfirm 阶段时，必须在右侧自动滑出产物审核面板展示当前版本 artifact；左侧执行节点栏全程保留；面板随用户开始反馈或新一轮运行收起。

#### Scenario: 产出完成自动滑出
- **WHEN** author 产出完成（含修订完成、review 报告回来）进入 AuthorConfirm 阶段
- **THEN** 右侧自动滑出产物审核面板展示当前版本 artifact 全文，左侧执行节点栏与收窄的对话流保持可见

#### Scenario: 反馈时收起
- **WHEN** 面板展示中，用户在对话输入区开始输入反馈（或点击「返回对话反馈」）
- **THEN** 面板收起，对话流恢复全宽，用户继续反馈流程

#### Scenario: 吸顶操作条全程可见
- **WHEN** 用户在面板内向下滚动审阅长文档
- **THEN** 操作条（送审/定稿/采纳按钮）吸顶保持可见，不随内容滚出视野

#### Scenario: 本轮改动摘要进面板
- **WHEN** 修订完成进入 AuthorConfirm 且本轮有改动摘要
- **THEN** 面板顶部展示可折叠的「本轮改动」摘要条，默认展开

### Requirement: 三动作视觉分层

终局确认动作与迭代操作必须在位置与样式上明确分层；「发送反馈」不得与终局确认按钮并排呈现。

#### Scenario: 终局确认对在面板操作条
- **WHEN** 面板展示时
- **THEN** 「送审」「定稿」成对出现在面板操作条，默认推荐项为主色样式、另一项为次级样式，两项始终可点

#### Scenario: 发送反馈留在对话输入区
- **WHEN** 任意阶段
- **THEN** 「发送反馈」仅作为对话输入区的提交动作呈现，样式为中性次级，不进入面板操作条

### Requirement: Spec 工作台视觉规范

Spec 工作台视觉迁移到 Claymorphism 蓝紫橙规范；设计规范以 design-system/MASTER.md 持久化为全站唯一事实源。

#### Scenario: token 映射落地
- **WHEN** 实施完成后检查 spec 工作台页面
- **THEN** 卡片/面板呈现 2-3px 深色边框与 rounded-xl/2xl 圆角，页面底色为微暖奶油色，主色为 #4F46E5、CTA 强调为 #F97316，标签为胶囊 chip 形态

#### Scenario: 克制玩具元素
- **WHEN** 检查 spec 工作台页面
- **THEN** 不出现 emoji 装饰图标、hover 放大位移、双层硬阴影、衬线正文；hover/transition 时长在 150-300ms

#### Scenario: 规范持久化
- **WHEN** 查看仓库 design-system/MASTER.md
- **THEN** 文件存在且包含配色/字体/边框/圆角/阴影/按钮/chip 的完整规范与反模式清单，作为后续全站推广的唯一事实源
