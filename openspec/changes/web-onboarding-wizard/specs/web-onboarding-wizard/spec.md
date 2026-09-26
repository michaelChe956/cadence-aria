# Spec Delta

## Purpose

为首次进入工作台的用户提供一条可理解、可回看且不会改变业务流程的全流程操作引导，帮助用户从 Project 创建一直走到 Coding 完成确认，并让引导内容可以随工作台能力演进而配置化启用。

## ADDED Requirements

### Requirement: 首次进入展示与设备级已读判定

系统 MUST 在用户首次进入工作台且设备本地没有 `aria-onboarding-seen` 已读记录时展示 onboarding wizard；完成、跳过或关闭引导后 MUST 记录该键，使同一设备后续进入工作台不再自动展示。localStorage 读取或写入不可用时 MUST 静默降级，不阻断工作台渲染。

#### Scenario: 首次进入工作台

- **WHEN** 用户进入工作台且设备 localStorage 没有 `aria-onboarding-seen`
- **THEN** 系统展示启用步骤组成的引导，并从第一个启用步骤开始

#### Scenario: 同一设备再次进入

- **WHEN** 用户再次进入工作台且 `aria-onboarding-seen` 已记录
- **THEN** 系统不自动展示引导，工作台照常可用

#### Scenario: localStorage 不可用

- **WHEN** 浏览器 localStorage 访问抛出异常
- **THEN** 系统不抛出错误、不阻断工作台；自动引导可静默不展示

### Requirement: 13 步手动模式全流程

系统 MUST 为手动模式提供以下 13 个按顺序启用的引导步骤：添加 Project、添加代码库（仅单库）、创建 Issue（含基准分支）、生成 Story、确认 Story、生成 Design、确认 Design、生成 Work Item Plan、确认 Work Item Plan 的第一道门、确认 Work Item Plan 的第二道门、进入 Coding Workspace、启动 Coding、查看进度并确认完成。步骤必须覆盖从前置资源创建到完成确认的端到端路径；单库限制和基准分支选择说明必须出现在对应步骤中。

#### Scenario: 手动模式步骤顺序

- **WHEN** 用户开始手动模式引导
- **THEN** 向导按上述顺序逐步呈现 13 个步骤，当前步骤始终显示在总进度中的位置

#### Scenario: 计划双门说明

- **WHEN** 用户进入 Work Item Plan 确认步骤
- **THEN** 引导分别说明第一道计划确认门和第二道最终确认门，且不把两道门合并为一个模糊步骤

### Requirement: 步骤说明、视觉示意与稳定锚点高亮

每个启用步骤 MUST 从配置读取标题、简短说明、截图或示意资源和目标 `data-testid` 锚点；打开步骤时系统 MUST 在目标锚点周围显示高亮或聚焦效果，并以可访问方式呈现说明。目标元素暂时不存在时 MUST 保留步骤说明并展示锚点未找到的非阻塞提示，不得使工作台崩溃或改变业务状态。

#### Scenario: 锚点存在时高亮

- **WHEN** 当前步骤配置的目标 `data-testid` 元素存在
- **THEN** 引导在该元素周围呈现视觉高亮，并保持用户可操作的工作台内容可见

#### Scenario: 锚点暂时缺失

- **WHEN** 当前步骤配置的目标元素尚未渲染或不存在
- **THEN** 向导仍展示步骤标题、说明和示意资源，并提示当前定位锚点暂不可见，不抛错、不修改业务状态

#### Scenario: 锚点契约稳定

- **WHEN** 工作台对应区域渲染项目、单仓代码库、Issue、Story、Design、Work Item Plan、Coding Workspace 或进度内容
- **THEN** 各区域提供配置引用的稳定 `data-testid`，现有业务行为和既有 testid 契约不改变

### Requirement: 配置驱动步骤与 feature-gate late-bind

步骤定义 MUST 集中配置标题、说明、资源、锚点、顺序和启用条件；渲染器 MUST 根据每个步骤的 feature-gate/late-bind 条件过滤未启用步骤，并按过滤后的列表计算进度。自动化模式选择步骤 MUST 能在配置中声明但默认禁用，待 P1 稳定锚点可用后可仅通过启用条件打开；禁用该步骤时本期手动模式引导 MUST 仍保持 13 步。

#### Scenario: 禁用自动化步骤

- **WHEN** 自动化模式选择步骤的 feature-gate 为关闭或其 late-bind 锚点条件不满足
- **THEN** 该步骤不出现在向导、不占用进度，并且其他手动步骤顺序不变

#### Scenario: P1 锚点可用后启用

- **WHEN** P1 提供稳定自动化模式选择锚点且 feature-gate 配置改为启用
- **THEN** 向导可以纳入该步骤而无需改动渲染器的步骤遍历逻辑

### Requirement: 跳过与帮助入口重开

用户 MUST 能在任一步骤跳过整个引导，跳过后 MUST 立即关闭并记录已读状态；工作台 shell MUST 提供可访问的帮助入口，用户可通过该入口主动重开引导并从第一个启用步骤开始，重开不得清除或改变任何 Project、Issue、计划、门禁或 Coding 状态。

#### Scenario: 用户跳过引导

- **WHEN** 用户点击跳过
- **THEN** 向导关闭、记录 `aria-onboarding-seen`，工作台业务状态保持不变

#### Scenario: 帮助入口重开

- **WHEN** 用户在工作台点击帮助入口
- **THEN** 向导重新打开并从第一个启用步骤开始，用户可再次浏览或跳过

### Requirement: 非侵入边界与驾驶舱兼容

引导 MUST 只作为 Web 前端外围呈现层接入，MUST NOT 修改 API、WebSocket 协议、后端代码或既有驾驶舱通知/收件箱行为；MUST NOT 修改 `useCockpitAutopilot`、`workspace-cockpit-projection`、`CockpitInbox` 或 `ChatCockpitPage` 内部实现。

#### Scenario: 既有业务路径保持不变

- **WHEN** 用户未打开引导或关闭引导
- **THEN** Project、代码库、Issue、Story、Design、Work Item Plan 和 Coding 原有操作仍按既有路径运行，驾驶舱通知与收件箱呈现不改变

#### Scenario: 前端外围改动

- **WHEN** 检查本 change 的改动范围
- **THEN** 改动集中在新的 onboarding 配置/组件/状态与 shell 接入口，以及只补稳定 testid 的既有前端呈现文件；不包含后端或受保护模块内部行为改动
