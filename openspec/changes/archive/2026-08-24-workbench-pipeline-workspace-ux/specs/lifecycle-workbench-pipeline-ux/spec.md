## Purpose

为 Issue 生命周期工作台提供流水线式信息架构：以紧凑队列承载大量 Issue 的监督，以阶段标签工作区承载文档型产物与阶段推进操作，使用户始终清楚「当前内容属于哪个 Issue、处于哪个阶段、下一步该做什么」。

## ADDED Requirements

### Requirement: 紧凑 Issue 队列与阶段 mini-graph

工作台 SHALL 以单行密度展示 Issue 队列，每行 SHALL 包含 Issue 标题与覆盖 Story、Design、Work Item、Coding 四个阶段的 mini-graph 状态链；每行高度 MUST 显著低于当前多行卡片（目标约 44–56px）。Issue 描述摘要、「生成 Story Spec」按钮与删除按钮 MUST NOT 在每行常驻显示，仅在 hover、选中或菜单中提供。

#### Scenario: 单行密度展示

- **WHEN** 用户打开 /workbench 且当前 Project 存在多个 Issue
- **THEN** 每个 Issue 以单行展示，行内可见标题与四阶段 mini-graph 状态链，且无常驻的描述摘要与操作按钮

#### Scenario: 操作按需出现

- **WHEN** 用户将指针悬停或键盘聚焦到某个 Issue 行
- **THEN** 该行的「生成 Story Spec」入口与删除入口变为可用，且行高不发生变化

### Requirement: 队列滚动边界与渲染上限

工作台外壳 MUST 限制在视口高度内，Issue 队列与工作区 MUST 各自独立滚动，Issue 数量增加 MUST NOT 使整页高度持续增长。每个分组 SHALL 最多渲染前 50 条，超出时提供「显示更多」入口。

#### Scenario: 长列表不撑高整页

- **WHEN** 当前 Project 的 Issue 数量超过一屏可容纳的行数
- **THEN** 仅队列区域内部出现滚动，整页滚动条长度不随 Issue 数量线性增长

#### Scenario: 分组渲染上限

- **WHEN** 某一分组内的 Issue 超过 50 条
- **THEN** 仅渲染前 50 条并显示「显示更多」入口，点击后追加渲染

### Requirement: 下一步动作分组与过滤

工作台 SHALL 按「下一步动作」对 Issue 分组（至少包含待生成 Story、待生成 Design、待拆 Work Item、编码中、阻塞、已完成），组头 SHALL 显示计数且可折叠，「已完成」分组 MUST 默认折叠；分组折叠状态 SHALL 按 Project 记忆。队列顶部 SHALL 提供文本过滤，过滤与分组 MUST 完全基于已加载数据在前端派生，MUST NOT 新增或修改任何 API。

#### Scenario: 按下一步动作分组

- **WHEN** 当前 Project 的 Issue 处于不同生命周期阶段
- **THEN** 队列按下一步动作分组展示，组头显示组内计数，且「已完成」分组默认折叠

#### Scenario: 文本过滤

- **WHEN** 用户在过滤输入框中输入关键字
- **THEN** 队列仅展示标题或 ID 匹配的 Issue，且不发起任何新的网络请求

### Requirement: 统一选择语义与持续归属高亮

队列行的高亮 MUST 唯一由当前聚焦 Issue（focusedIssueId）驱动，并 MUST 暴露 `aria-current`；当前检视的 Story/Design/Work Item 实体的选中态 MUST 仅作用于工作区内部。选中任何子实体时 MUST 同步聚焦其所属 Issue，使 Issue 行高亮在任何子实体操作后保持。Issue 聚焦高亮与子实体选中高亮 MUST 在视觉上可区分，且颜色 MUST NOT 是唯一的选中指示手段。

#### Scenario: 子实体操作后 Issue 高亮保持

- **WHEN** 用户选中某 Issue 后点击其 Story Spec 或 Design Spec 卡片
- **THEN** 队列中该 Issue 行保持聚焦高亮与 `aria-current`，工作区内对应子实体呈现与 Issue 不同形态的选中样式

#### Scenario: 非颜色选中指示

- **WHEN** 任一 Issue 行处于聚焦状态
- **THEN** 除颜色外还存在非颜色指示（如左侧指示条与 `aria-current` 语义），键盘 focus 状态清晰可见

### Requirement: 阶段标签工作区

选中 Issue 后，工作区 MUST 在顶部常驻展示「当前 Issue」标题、ID 与状态，并提供覆盖 Story、Design、Work Item 的阶段步进器（含各阶段计数与状态）。阶段内容 SHALL 通过标签页一次展示一个阶段并占满工作区宽度，MUST NOT 再以三等宽列并列展示；默认选中的阶段 SHALL 是该 Issue「需要动作的最早阶段」。Work Item 存在仓库分组时，分组内容 SHALL 在 Work Item 阶段页内以完整宽度展示。「触发下一阶段」动作 MUST 在工作区内常驻可达。

#### Scenario: 阶段标签展示

- **WHEN** 用户选中一个同时拥有 Story Spec 与 Design Spec 的 Issue
- **THEN** 工作区顶部展示当前 Issue 标题与阶段步进器，主体区域一次仅展示一个阶段的内容且占满宽度

#### Scenario: 默认落在需要动作的阶段

- **WHEN** 用户选中的 Issue 已有 Story Spec 但尚无 Design Spec
- **THEN** 工作区默认选中 Design 阶段，且「生成 Design Spec」动作在工作区内直接可达

#### Scenario: 空阶段不占空间

- **WHEN** 当前 Issue 尚无 Work Item
- **THEN** Work Item 阶段不以空白列占用工作区宽度，而是以阶段标签形式提供入口与状态

### Requirement: 队列折叠与监督/专注双密度

工作台 SHALL 提供队列折叠能力：折叠后工作区获得完整宽度，展开后恢复队列与工作区并存；折叠状态 SHALL 按 Project 记忆。折叠与展开 MUST NOT 改变当前聚焦 Issue、当前检视实体或触发任何网络请求。

#### Scenario: 折叠进入专注密度

- **WHEN** 用户触发队列折叠
- **THEN** 队列隐藏或收缩为细轨，工作区占满可用宽度，当前 Issue 上下文与阶段步进器保持可见

#### Scenario: 折叠状态记忆

- **WHEN** 用户折叠队列后切换到其他 Project 再切回
- **THEN** 各 Project 恢复其各自的队列折叠状态

### Requirement: 轮询刷新保持上下文

后台轮询刷新 MUST NOT 改变当前聚焦 Issue、当前检视实体、队列滚动位置、分组折叠状态或文本过滤内容；仅当用户通过深链或显式导航进入时，工作台才 SHALL 将焦点行滚动到可视区域。

#### Scenario: 轮询不打断阅读

- **WHEN** 用户正在阅读工作区内某 Design Spec 内容且后台轮询完成一次刷新
- **THEN** 当前聚焦 Issue、选中实体、滚动位置与折叠状态全部保持不变

### Requirement: 逻辑代码库运维面板降级

存在逻辑代码库时，主工作区顶部 MUST 默认仅展示一行状态摘要条（含索引与发布状态及管理入口），完整管理面板 SHALL 在摘要条展开后呈现；存在异常状态（如发布失败或索引缺失）时摘要条 MUST 以警示样式突出。面板内部既有管理能力 MUST 保持可用且行为不变。

#### Scenario: 默认摘要态

- **WHEN** 用户打开存在逻辑代码库的 Project
- **THEN** 主区顶部仅显示一行运维状态摘要条，完整管理面板不占用首屏空间

#### Scenario: 异常突出

- **WHEN** 最近一次指针发布存在失败成员或聚合索引缺失
- **THEN** 摘要条以警示样式展示并提供直达管理面板的入口

### Requirement: 视觉与交互规范

工作台视觉 SHALL 遵循数据密集型仪表盘规范：状态标识一律使用胶囊 chip；hover 反馈仅改变颜色/边框/阴影且 MUST NOT 引起布局位移；过渡时长 150–300ms 并尊重 `prefers-reduced-motion`；所有可点击元素 SHALL 具有指针光标与可见键盘 focus；图标 SHALL 使用 SVG 图标库（lucide-react），MUST NOT 使用 emoji 作为图标；浅色模式下正文对比度 MUST 满足 WCAG AA（4.5:1）。

#### Scenario: 稳定 hover 反馈

- **WHEN** 用户将指针在队列行与工作区卡片之间移动
- **THEN** hover 反馈仅表现为颜色、边框或阴影变化，无任何元素位置或尺寸抖动

#### Scenario: 减弱动效

- **WHEN** 操作系统开启「减弱动态效果」
- **THEN** 折叠/展开与状态过渡不播放位移动画

### Requirement: 既有契约兼容

改造 MUST 保留现有无障碍区域名（`Issue 卡片列表`、`Issue 生命周期详情`、`Story Spec 内容`、`Design Spec 内容`、`Work Item 内容`）与既有 testid 契约，新增元素 SHALL 使用新 testid；MUST NOT 修改 `web/src/api/**`、Rust 后端代码、API 路径、请求参数、响应类型或 WebSocket 协议。

#### Scenario: 区域名保持不变

- **WHEN** 改造完成后的工作台渲染
- **THEN** 上述五个区域名与既有 testid 仍可被现有测试与辅助技术定位

#### Scenario: 零 API 变更

- **WHEN** 审查本次改动的全部代码差异
- **THEN** 不存在对 API 客户端、后端路由、请求/响应类型或 WebSocket 消息的任何修改
