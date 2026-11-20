## MODIFIED Requirements

### Requirement: 紧凑 Issue 队列与阶段 mini-graph

工作台 SHALL 以单行密度展示 Issue 队列，每行 SHALL 包含 Issue 标题与覆盖 Story、Design、Work Item、Coding 四个阶段的 mini-graph 状态链；每行高度 MUST 显著低于当前多行卡片（目标约 44–56px）。Issue 描述摘要、「生成 Story Spec」按钮与删除按钮 MUST NOT 在每行常驻显示，仅在 hover、选中或菜单中提供。

**详情尚未到达的议题**（生命周期详情仍在读取）SHALL 仍以单行展示标题与四阶段 mini-graph，阶段状态为**未知态**（不得呈现为「待开始」，因为未知与待开始语义不同）。此类行的「删除 Issue」入口 SHALL 可用；「生成 Story Spec」入口 MUST NOT 可用，直到该议题的详情到达——该动作的幂等性依赖已知该议题是否已有 Story，未知时放行会造成重复生成。详情到达后，该行 SHALL 按既有规则迁入对应的阶段分组并恢复完整操作入口。

#### Scenario: 单行密度展示

- **WHEN** 用户打开 /workbench 且当前 Project 存在多个 Issue
- **THEN** 每个 Issue 以单行展示，行内可见标题与四阶段 mini-graph 状态链，且无常驻的描述摘要与操作按钮

#### Scenario: 操作按需出现

- **WHEN** 用户将指针悬停或键盘聚焦到某个 Issue 行
- **THEN** 该行的「生成 Story Spec」入口与删除入口变为可用，且行高不发生变化

#### Scenario: 详情未到时的行

- **WHEN** 某议题的生命周期详情仍在读取
- **THEN** 该行仍显示标题与四阶段未知态 mini-graph，「删除 Issue」入口可用，「生成 Story Spec」入口不可用

#### Scenario: 详情到达后迁组并恢复入口

- **WHEN** 某待查看议题的详情读取完成
- **THEN** 该行迁入按阶段判定的分组，四阶段图显示真实状态，「生成 Story Spec」入口恢复可用

### Requirement: 队列滚动边界与渲染上限

工作台外壳 MUST 限制在视口高度内，Issue 队列与工作区 MUST 各自独立滚动，Issue 数量增加 MUST NOT 使整页高度持续增长。每个分组 SHALL 最多渲染前 50 条，超出时提供「显示更多」入口。

**「生命周期待查看」分组**（详情未到达的议题）SHALL 参与同一渲染上限与「显示更多」规则，其分组位置 SHALL 排在既有六个阶段分组之后。

#### Scenario: 长列表不撑高整页

- **WHEN** 当前 Project 的 Issue 数量超过一屏可容纳的行数
- **THEN** 仅队列区域内部出现滚动，整页滚动条长度不随 Issue 数量线性增长

#### Scenario: 分组渲染上限

- **WHEN** 某一分组内的 Issue 超过 50 条
- **THEN** 仅渲染前 50 条并显示「显示更多」入口，点击后追加渲染

#### Scenario: 待查看分组参与同一上限

- **WHEN** 待查看分组的 Issue 数量超过 50 条
- **THEN** 该分组同样只渲染前 50 条并提供「显示更多」入口

### Requirement: 既有契约兼容

改造 MUST 保留现有无障碍区域名（`Issue 卡片列表`、`Issue 生命周期详情`、`Story Spec 内容`、`Design Spec 内容`、`Work Item 内容`）与既有 testid 契约，新增元素 SHALL 使用新 testid；MUST NOT 修改 `web/src/api/**`、Rust 后端代码、API 路径、请求参数、响应类型或 WebSocket 协议。

**议题列表仍在读取时**，队列 SHALL 显示加载态提示，MUST NOT 显示「没有匹配的 Issue」这一空态文案——列表未到与列表为空是两种不同状态。

**深链目标**（`?focus=` 指向的实体）其生命周期详情读取失败时，页面 SHALL 在入口处给出可读的中文提示，MUST NOT 静默无反应。

#### Scenario: 区域名保持不变

- **WHEN** 改造完成后的工作台渲染
- **THEN** 上述五个区域名与既有 testid 仍可被现有测试与辅助技术定位

#### Scenario: 零 API 变更

- **WHEN** 审查本次改动的全部代码差异
- **THEN** 不存在对 API 客户端、后端路由、请求/响应类型或 WebSocket 消息的任何修改

#### Scenario: 列表仍在加载时不报空

- **WHEN** 议题列表请求仍在进行中
- **THEN** 队列显示加载提示，MUST NOT 显示「没有匹配的 Issue」

#### Scenario: 深链目标读取失败有提示

- **WHEN** 深链目标的生命周期详情读取失败
- **THEN** 页面在入口处显示可读的中文提示，MUST NOT 无任何提示

## ADDED Requirements

### Requirement: 详情读取的批次隔离与焦点保持

生命周期详情读取 SHALL 以批次为单位管理并发槽位：切换 Project 或刷新产生新批次时，新批次 SHALL 立即拥有自己的读取槽位，MUST NOT 被上一批次的在途请求阻塞。上一批次的迟到响应 MUST NOT 写入新批次的状态。

议题行在「生命周期待查看」与阶段分组之间迁移时，键盘焦点 SHALL 保持在同一行，MUST NOT 落回页面主体。

#### Scenario: 新批次不被旧请求阻塞

- **WHEN** 上一 Project 的三条详情请求仍在进行中，用户切换到另一 Project
- **THEN** 新 Project 的详情请求立即发出并渲染，MUST NOT 一直停留在加载态

#### Scenario: 旧批次迟到响应不写入新状态

- **WHEN** 上一 Project 的详情响应在切换之后才返回
- **THEN** 该响应 MUST NOT 覆盖当前 Project 的详情

#### Scenario: 迁组后焦点保持

- **WHEN** 用户用键盘聚焦某议题行，该议题的详情随后到达并使其迁入阶段分组
- **THEN** 键盘焦点仍在该行，MUST NOT 落回页面主体
