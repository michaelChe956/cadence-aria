# spec-workbench-canvas-experience Specification

## Purpose
Story/Design workspace 的 AuthorConfirm 阶段以 Canvas 产物审核面板承载"先看产物、再对话"的审核体验，三动作视觉分层，整体视觉迁移到 Claymorphism 蓝紫橙规范。

## Requirements

### Requirement: Canvas 产物审核面板

Story/Design workspace 进入 AuthorConfirm 阶段时，必须在右侧自动滑出产物审核面板展示当前版本 artifact；左侧执行节点栏全程保留；面板随用户开始反馈或新一轮运行收起。

#### Scenario: 产出完成自动滑出
- **WHEN** author 产出完成（含修订完成、review 报告回来）进入 AuthorConfirm 阶段
- **THEN** 右侧自动滑出产物审核面板展示当前版本 artifact 全文，左侧执行节点栏与收窄的对话流保持可见

#### Scenario: 反馈时收起
- **WHEN** 面板展示中，用户点击「返回对话反馈」（或屏幕宽度 <1440px 时聚焦对话输入框，overlay 面板遮挡输入框的情形）
- **THEN** 面板收起，对话流恢复全宽，用户继续反馈流程；≥1440px 三栏并存时聚焦输入框不收起面板（用户可对照 artifact 内容撰写修改意见），面板在发送反馈进入运行阶段后随阶段收起

#### Scenario: 吸顶操作条全程可见
- **WHEN** 用户在面板内向下滚动审阅长文档
- **THEN** 操作条（送审/定稿/采纳按钮）吸顶保持可见，不随内容滚出视野

#### Scenario: 本轮改动摘要进面板
- **WHEN** 修订完成进入 AuthorConfirm 且本轮有改动摘要
- **THEN** 面板顶部展示可折叠的「本轮改动」摘要条，默认展开

#### Scenario: 无改动摘要时折叠条隐藏
- **WHEN** 进入 AuthorConfirm 时当前版本非修订产生（如初稿）或无可用摘要
- **THEN** 面板不渲染「本轮改动」折叠条

#### Scenario: 重连恢复后面板状态
- **WHEN** 会话处于 AuthorConfirm 阶段时页面刷新或断线重连
- **THEN** 状态恢复后面板自动滑出展示当前版本产物（阶段驱动，与首次进入一致）

#### Scenario: 送审与运行期间面板收起
- **WHEN** 用户在面板点「确认送审」（进入 CrossReview）或新一轮修订运行开始
- **THEN** 面板收起，reviewer/author 运行期间对话流全宽展示执行过程；运行结束回到 AuthorConfirm 时面板再次滑出

#### Scenario: 采纳 Review 意见预填时自动收起面板
- **WHEN** 面板展示中用户点击「采纳 Review 意见」
- **THEN** 报告文本预填入对话输入框的同时面板自动收起（对话流全宽），用户可直接查看/编辑预填内容

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
- **WHEN** 实施完成后检查页面
- **THEN** token 层全站生效（主色 #4F46E5、CTA #F97316、奶油底色）；spec 工作台组件形态收敛：卡片/面板 2-3px 深色边框与 rounded-xl/2xl 圆角、标签为胶囊 chip 形态；其他工作台仅色系联动、组件形态不变

#### Scenario: 克制玩具元素
- **WHEN** 检查 spec 工作台页面
- **THEN** 不出现 emoji 装饰图标、hover 放大位移、双层硬阴影、衬线正文；hover/transition 时长在 150-300ms

#### Scenario: 规范持久化
- **WHEN** 查看仓库 design-system/MASTER.md
- **THEN** 文件存在且至少包含：配色（主色/CTA/底色/文本/边框）、字体、边框与圆角、阴影、按钮体系（primary/secondary）、chip 组件、反模式清单七个章节，作为后续全站推广的唯一事实源
