# Workspace 执行归集与交叉审核可见性问题总结

## 文档信息

- 文档类型：分析报告
- 日期：2026-05-19
- 版本：v1.0
- 分支：`product-workbench-issue-lifecycle`
- Worktree：`.worktrees/product-workbench-issue-lifecycle`
- 关联测试目标仓库：`/Users/michaelche/Documents/git-folder/github-folder/naruto`
- 关联手工测试 Issue：`爬楼梯问题`

## 背景

在手工验证 `Issue -> Story Spec -> Design Spec -> Work Item` 生命周期时，当前 Workspace 已能展示阶段条、对话消息、Artifact、执行事件和 Provider 配置，但这些信息分散在多个区域。用户在实际检查时无法直观看到交叉审核是否发生、Claude Code 与 Codex 分别做了什么，也无法从一个统一视角理解完整执行过程。

本记录聚焦 2026-05-19 手工测试中暴露出的产品体验与流程可观测性问题，不包含代码实现方案。

## 问题 1：交叉审核效果不可见

### 现象

- 页面底部阶段条包含 `交叉审查`。
- 生成完成后，阶段会经过 `cross_review` 并进入 `human_confirm`。
- 用户只能看到“交叉审查已经过”这类阶段状态，无法看到具体审核过程。
- 页面没有明确展示：
  - 审核由哪个 Agent 执行。
  - 审核输入是什么。
  - 审核意见是什么。
  - 审核结论是 `pass`、`revise` 还是 `needs_human`。
  - 是否发生过返修与二次审核。

### 影响

- 用户无法判断交叉审核是否真的执行。
- 用户无法区分“真实审核通过”和“阶段被快速跳过”。
- `Claude Code 生成 + Codex review` 的核心协作价值没有被产品化表达出来。
- 后续人工确认缺少审核证据，确认动作可信度不足。

### 建议验收标准

- 每一轮 review 都应在 UI 中形成可见记录。
- Review 记录至少包含 reviewer、round、输入摘要、审核意见、审核结论和时间。
- 如果 review 被跳过或使用 fake provider，应明确展示“未执行真实 review / fake 快速路径”，不能暗示交叉审核已完成。

## 问题 2：无法区分 Claude Code 与 Codex 的职责边界

### 现象

- Workspace 右侧顶部只以小字显示 `Author: xxx | Reviewer: xxx`。
- Provider 配置入口隐藏在设置按钮里。
- 执行事件卡片本身没有强标识显示该步骤由 Claude Code 还是 Codex 执行。
- 当前体验中，用户需要推断：
  - 生成类任务是否由 Claude Code 执行。
  - review 类任务是否由 Codex 执行。
  - 某条输出或 Artifact 来自哪个 Agent。

### 影响

- 多 Agent 协作变成了后台配置，用户感知不到实际分工。
- 当输出质量有问题时，用户无法定位问题来自生成者还是审核者。
- 测试人员无法直接验证默认 Agent 策略是否符合 PRD 中的协作模型。

### 建议验收标准

- 每个执行节点都应有明确 Agent badge，例如 `[Claude Code] N05 Story Spec 生成`、`[Codex] N06 Story Spec Review`。
- Artifact 应能追溯生成者、审核者和人工确认者。
- Provider 配置不应只作为隐藏设置存在，至少应在执行前和执行记录中可见。
- 当用户覆盖默认 Agent 时，Timeline 应记录本次实际选择，而不是只显示当前全局配置。

## 问题 3：输出内容缺少统一归集，用户需要在多个区域拼流程

### 现象

当前 Workspace 信息被拆散在多个区域：

- 左侧对话区：用户消息、assistant 流式输出、权限请求。
- 右侧 `Artifact` tab：最终产物正文。
- 右侧 `执行` tab：provider / command 事件。
- 底部阶段条：当前阶段和已经过阶段。
- 看板列：Story Spec、Design Spec、Work Item 卡片。

用户需要在这些区域之间切换和拼接，才能理解“从 Issue 到 Work Item 到底发生了什么”。

### 影响

- 手工测试缺少一个稳定的检查入口。
- 用户无法快速回答以下问题：
  - 当前流程走到哪一步。
  - 上一步是谁做的。
  - 产物是什么。
  - 谁审核过。
  - 人工确认发生在哪一步。
  - 失败或返修发生在哪一步。
- Artifact、对话、执行事件之间缺少明显关联，导致结果显得零散。

### 建议验收标准

- Workspace 应提供一个默认的全流程 Timeline 视图。
- Timeline 应串起 `Issue 创建 -> Story Spec 生成 -> Story Review -> 人工确认 -> Design Spec 生成 -> Design Review -> 人工确认 -> Work Item 生成 -> Work Item Review -> 人工确认`。
- 每个 Timeline 节点应展示阶段、动作、执行者、状态、输入摘要、输出摘要、Artifact 引用和可回退点。
- 对话区和 Artifact 可以保留，但应作为 Timeline 节点的详情，而不是主要流程载体。

## 建议后续设计方向

后续设计应围绕“统一 Timeline”展开：

1. 以 Timeline 作为 Workspace 的默认主视图。
2. 将 Claude Code / Codex / Human 都建模为 Timeline actor。
3. 将生成、审核、返修、人工确认都建模为可审计事件。
4. 将 Artifact、流式输出、执行事件和 checkpoint 挂载到对应 Timeline 节点下。
5. 保留现有对话与执行面板能力，但降低其作为主流程理解入口的权重。

## 当前结论

当前分支可以继续验证生命周期主链路，但以下三点不应视为通过：

- 交叉审核可见性不足。
- Claude Code 与 Codex 的职责分工不可见。
- 输出缺少 Timeline 式归集，用户难以理解端到端过程。

这些问题应进入下一轮产品设计与实现范围。
