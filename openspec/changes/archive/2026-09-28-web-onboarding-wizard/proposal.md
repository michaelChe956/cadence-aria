# Proposal

## Why

当前用户需要分别理解 Project、代码库、Issue 生命周期、计划门禁与 Coding Workspace，虽然工作台已有这些能力，但缺少一条面向首次使用者的端到端路径，用户容易在正确的入口、阶段顺序和确认门之间迷失。Issue 2 现在独立交付前端引导，正好可以在既有工作台与驾驶舱基础上提供低侵入的全流程说明，而不重复实现驾驶舱通知或业务动作。

## What Changes

- 新增面向首次进入用户的 13 步手动模式 onboarding wizard，覆盖从创建 Project、绑定单一代码库、创建 Issue（含基准分支）到 Story、Design、Work Item Plan、Coding Workspace、Coding 进度和完成确认的完整路径。
- 每个步骤提供简短中文说明、当前区域的截图或示意图，并以稳定的 `data-testid` 锚点定位并高亮对应界面；缺少锚点的既有界面只补属性，不改变业务行为。
- 引导支持上一步、下一步、跳过；在工作台 shell 提供帮助入口，用户可在首次展示后重新打开引导。
- 使用设备级浏览器 `localStorage` 首次判定，存储键固定为 `aria-onboarding-seen`；存储不可用时不阻断工作台，手动帮助入口仍可尝试打开。
- 引导步骤内容、顺序、说明、视觉资源、锚点和启用条件集中配置，不把文案与步骤规则散落在渲染逻辑中。
- 配置支持步骤级 feature-gate/late-bind：自动化模式选择步骤先以禁用配置存在并从本期 13 步中跳过，待 P1 提供稳定锚点后仅通过配置启用；本期不改动自动化执行逻辑。
- 明确不重复设计或替换既有驾驶舱收件箱、通知和对话页面能力；不新增后端接口、协议、第三方 tour 依赖或 API 数据模型。

## Capabilities

### New Capabilities

- `web-onboarding-wizard`: 提供配置驱动、可跳过且可从工作台帮助入口重开的用户全流程操作引导，包括首次判定、步骤视觉定位、稳定锚点高亮和 feature-gate 跳过机制。

### Modified Capabilities

- 无。现有工作台、驾驶舱收件箱和 Coding Workspace 的业务契约保持不变；本 change 只为引导增加外围呈现与必要的稳定定位属性。

## Impact

- 主要影响 Web 前端呈现层：新增 wizard 配置、状态/存储逻辑、引导浮层与测试；工作台 shell 或 `WorkbenchSurface` 增加帮助入口，相关现有呈现组件可能补充 `data-testid` 属性。
- 引导仅消费已存在的工作台、生命周期阶段、计划审批和 Coding Workspace UI，不新增 API 请求或后端代码，不修改 `useCockpitAutopilot`、`workspace-cockpit-projection`、`CockpitInbox`、`ChatCockpitPage` 内部实现。
- 不新增前端依赖；沿用 React、现有 Tailwind/`--aria-*` 视觉 token、lucide 图标和既有测试工具。