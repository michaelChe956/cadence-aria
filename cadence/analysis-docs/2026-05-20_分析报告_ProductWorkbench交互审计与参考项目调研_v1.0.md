# Product Workbench 交互审计与参考项目调研

## 文档信息

- 文档类型：分析报告
- 分支：`product-workbench-issue-lifecycle`
- 工作区：`.worktrees/product-workbench-issue-lifecycle`
- 调研日期：2026-05-20
- 版本：v1.0

## 结论摘要

当前分支已经从“旧执行工作台”推进到“Issue 生命周期看板 + 全屏 Workspace + WebSocket Provider 执行 + Timeline 归集”的形态。四列主流程、Story/Design/Work Item 下游入口、Provider 配置锁定、交叉 review、权限请求、确认和返修循环都有了工程基础。

主要问题不再是“有没有工作台”，而是“Workspace 内每个动作的语义是否足够明确、刷新后审计证据是否完整、Work Item 是否真的进入代码开发闭环”。如果把它作为产品化入口继续推进，建议先收敛 Workspace 的命令模型和 Timeline 持久化，再补 Work Item 专属执行流，最后优化信息架构和风险操作。

## 当前进度判断

### 已形成的能力

1. 生命周期主入口已经切到四列看板：Issue、Story Spec、Design Spec、Work Item。
2. Story/Design/Work Item 卡片可打开对应 Workspace Session，生成下游卡片后会自动打开 Workspace。
3. Workspace 已从旧聊天面板推进到 Timeline 为主的执行视图，每个节点可以带 agent、状态、summary 和 provider 配置快照。
4. 后端 WebSocket 已支持真实 provider registry，能驱动 author、reviewer、revision、permission response、abort、confirm。
5. Fake reviewer 会明确标记“未执行真实 review”，这比之前的“交叉审查已经过”更可信。
6. Story/Design 生成后会追加版本，卡片上能展示版本和 markdown preview。
7. 现有测试覆盖了卡片选择、开始生成按钮、Timeline 节点展示、权限响应、provider 选择持久化、重连恢复节点和 artifact version。

### 仍未完成或存在偏差的能力

1. 设计文档要求的 `ProviderWorkspaceDialog` 在实现中变成了全屏 `WorkspacePage`。这不一定错，但需要明确产品决策：到底是弹窗内完成，还是路由页完成。
2. Work Item 当前仍复用 Story/Design 的普通 Workspace stage。`CodingWorkspaceStage` 已定义，但主流程没有使用。确认 Work Item 时只是把 `plan_status` 置为 confirmed，还没有真正进入 coding/testing/review/rework/final。
3. Project 级 provider 默认配置、superpowers/OpenSpec 默认与单次覆盖已进入模型字段，但前端没有形成清晰入口。
4. Timeline 只持久化节点和 artifact versions，不持久化每个 Timeline 节点的完整详情。刷新后能看到节点列表，但不能完整恢复每个节点的流式输出、execution event、permission event 和 review detail。

## 交互问题清单

### P0：准备阶段输入语义仍然混淆

当前 UI 已新增“开始生成”按钮，但输入框仍在 `prepare_context` 阶段可用，placeholder 是“输入消息...”。提交任意文本会发送 `user_message`，后端会直接启动 provider run。这意味着用户想补充约束时，仍可能误触发生成。

相关代码：

- `web/src/pages/WorkspacePage.tsx:120`：表单提交直接调用 `sendMessage(content)`。
- `web/src/pages/WorkspacePage.tsx:298`：同时存在显式“开始生成”按钮。
- `web/src/hooks/useWorkspaceWs.ts:131`：`sendMessage` 统一发送 `user_message`。
- `src/web/workspace_ws_handler.rs:233`：后端把所有 `UserMessage` 当成运行入口。

建议：把 Workspace 输入拆成两类协议消息。

1. `context_note`：只追加准备阶段上下文，不启动执行。
2. `start_generation`：显式启动生成，并锁定 provider 配置。

UI 上应把准备阶段输入框改名为“补充上下文”，并把“开始生成”作为唯一启动入口。

### P0：Timeline 详情持久化不完整，审计链刷新后断裂

当前重连恢复测试只验证 `timeline_nodes` 和 `artifact_versions`，没有验证节点详情。前端 `setSessionState` 会用 `detailsForTimelineNodes` 创建空 detail，因此刷新后每个节点下的 streaming 内容、execution events、review verdict 展示会丢失。

相关代码：

- `src/product/lifecycle_store.rs:442`：持久化 `timeline_nodes.json`。
- `src/product/lifecycle_store.rs:478`：持久化 `artifact_versions.json`。
- `src/product/workspace_engine.rs:1242`：SessionState 只返回 messages、checkpoints、artifact、providers、timeline_nodes、artifact_versions。
- `web/src/state/workspace-ws-store.ts:211`：前端应用 snapshot 时重建空 `nodeDetails`。

建议：新增 `timeline_node_details.json` 或按节点分文件持久化，至少包含：

1. `messages`
2. `streaming_content_snapshot`
3. `execution_events`
4. `permission_events`
5. `review_verdict`
6. `artifact_ref`

### P1：刷新/断开时的运行控制策略需要产品化说明

当前 WebSocket 连接关闭时会取消当前 `ActiveRun`。这比“刷新后旧 run 不可控”安全，但用户体验上可能变成“刷新页面导致运行被中止”，而 UI 没有解释。

相关代码：

- `src/web/workspace_ws_handler.rs:465`：socket 退出后取出 active run。
- `src/web/workspace_ws_handler.rs:467`：发送 abort。
- `src/product/workspace_engine.rs:1189`：失败或取消后把 session status 更新回 open，并切回 prepare_context。

建议二选一：

1. 产品采用“断开即中止”：UI 在刷新/离开前提示，Timeline 记录 `aborted_by_disconnect`。
2. 产品采用“后台继续跑”：ActiveRun 应脱离 WebSocket 生命周期，并支持重连后重新绑定 command channel。

如果不先选定，后续权限响应、中止、刷新恢复会持续互相牵扯。

### P1：右侧 Artifact/执行 Tab 在 Timeline 存在时实际不可用

右侧顶部有 Artifact / 执行两个 tab，但只要 `selectedNode` 存在，页面始终渲染 `TimelineDetailPanel`，tab 的选择不会改变主体内容。由于 `selectedNodeId` 默认取 active node 或最后一个 node，Timeline 存在时几乎总有 selected node。

相关代码：

- `web/src/pages/WorkspacePage.tsx:134`：`selectedNode` 默认回退到最后一个 Timeline node。
- `web/src/pages/WorkspacePage.tsx:412`：Artifact / 执行 tab 更新 `activeRightTab`。
- `web/src/pages/WorkspacePage.tsx:477`：`selectedNode` 优先级高于 tab。

建议：把右侧改为“节点详情 / Artifact / 执行日志 / 版本”四个明确 tab，或把当前 tab 放入每个 Timeline 节点详情内部，避免无效控件。

### P1：Provider 配置可见性仍不够强

设计目标要求 Header 区域始终可见 Author/Reviewer，并在开始后锁定。当前实现里顶部只放了设置按钮，真正配置藏在折叠面板；右侧小字显示 Author/Reviewer，但不够像“本次执行快照”。

相关代码：

- `web/src/pages/WorkspacePage.tsx:165`：设置按钮入口。
- `web/src/pages/WorkspacePage.tsx:439`：右侧小字显示 provider。
- `web/src/pages/WorkspacePage.tsx:449`：展开后才能选择 Author/Reviewer。

建议：把 provider snapshot 放到 Workspace header 主信息区，包含 Author、Reviewer、review rounds、superpowers、OpenSpec。开始后展示“已锁定”。

### P1：Work Item 没有真正成为代码执行闭环

当前 Work Item 是生命周期实体，但 Workspace 执行模型仍是生成 artifact、review、人工确认。确认 Work Item 时只是确认 plan，不会显式进入 worktree、coding、testing、review、rework、final。

相关代码：

- `src/product/models.rs:238`：有 `WorkItemPlanStatus`。
- `src/product/models.rs:287`：Work Item 有 `plan_status`、`execution_status`、`worktree_path`。
- `src/product/workspace_engine.rs:1083`：Work Item 确认时仅更新 `WorkItemPlanStatus::Confirmed`。

建议：Work Item Workspace 应有专用 stage：PlanGeneration、PlanConfirm、WorktreePrepare、Coding、Testing、CodeReview、Rework、FinalConfirm。Plan 确认前禁用 coding；Plan 确认后进入 worktree 与真实代码执行链。

### P2：四列看板的聚焦关系不够清晰

点击 Issue 后，Story/Design/Work Item 会按 issue 过滤，但 Issue 列仍展示所有 Issue。这个交互有利于快速切换，但当前缺少明显的“当前焦点 Issue”表达，用户可能误以为所有列仍是全局视图。

相关代码：

- `web/src/state/lifecycle-workbench-store.ts:127`：`visibleLifecycle` 聚焦过滤。
- `web/src/state/lifecycle-workbench-store.ts:140`：聚焦后 Issue 列仍返回全部 issue。

建议：Issue 列保留全部可以，但需要置顶/高亮当前 Issue，并在其他列标题显示“当前 Issue：xxx”。

### P2：删除操作缺少确认

Project、Repository、Issue 删除都是直接调用接口。Issue 卡片上删除按钮可见，误点代价较高。

相关代码：

- `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx:203`：删除 Project。
- `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx:214`：删除 Repository。
- `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx:226`：删除 Issue。

建议：加入确认弹窗，并展示影响范围：Issue、派生 Story/Design/Work Item、Workspace Session、版本和 Timeline。

## 外部参考项目

### GitHub Copilot Workspace / Copilot coding agent

可参考点：

1. 从 Issue 或自然语言任务进入，围绕“计划、构建、测试、运行”组织 AI 工作流。
2. Coding agent 的主交付物是 PR，用户在 GitHub/VS Code 的 review 流里验收。
3. 适合借鉴“任务准备度”和“Issue 上下文随任务传递”的产品边界。

对 Aria 的启发：

- Story/Design 阶段可以保留 Workspace，但 Work Item 阶段最终应收敛到 branch/worktree/PR 或等价代码变更包。
- 确认动作不应只确认“一个 session 完了”，而应确认“这个 artifact/plan/PR 能进入下一生命周期”。

来源：

- https://github.blog/news-insights/product-news/github-copilot-workspace/
- https://docs.github.com/en/copilot/how-tos/use-copilot-agents/coding-agent/assign-copilot-to-an-issue
- https://docs.github.com/en/copilot/using-github-copilot/coding-agent/about-assigning-tasks-to-copilot

### Vibe Kanban

可参考点：

1. 明确把产品定位在“plan/review AI agents”，而不是自己当 coding agent。
2. Workspace 是任务执行环境，创建时自动准备 git worktree 和 setup scripts。
3. 任务执行过程中展示实时 logs、commands、file operations、tool usage，并允许 follow-up。
4. 每个 task attempt 独立隔离，避免污染主工作区。

对 Aria 的启发：

- Aria 的 Work Item 应对齐“attempt”模型：一个 Work Item 可以有多次 attempt，每次 attempt 有独立 worktree、branch、provider、日志、结果和回退点。
- Timeline 节点详情必须能承载实时日志和执行证据，而不是只展示最终 artifact。

来源：

- https://vibekanban.com/docs
- https://www.vibekanban.com/docs/core-features/monitoring-task-execution
- https://vibekanban.com/docs/workspaces/creating-workspaces

### Cursor Background Agents

可参考点：

1. Background agent 是异步远端 agent，用户能查看 status、发 follow-up、随时接管。
2. Agent 在隔离机器里运行，clone repo 后在独立 branch 上工作，并 push 回 repo。
3. 环境配置有显式 install/start/terminal 机制。
4. 安全文档明确说明自动运行命令的风险。

对 Aria 的启发：

- 如果 Aria 支持真实 provider 长时间运行，需要把“重连后继续控制”设计成一等能力，或者明确“断开即中止”。
- Provider 配置和环境配置应进入 Workspace header 和 Timeline snapshot，不能藏在折叠面板里。

来源：

- https://docs.cursor.com/background-agents
- https://docs.cursor.com/en/background-agents
- https://docs.cursor.com/background-agent/api/list-agents

### OpenAI Codex cloud

可参考点：

1. 每个 cloud task 使用独立 sandbox container。
2. 支持后台并行任务，可从不同设备或 GitHub 触发。
3. 官方强调 work log、review 输出、安全域名控制和 sandbox 边界。

对 Aria 的启发：

- Timeline 不只是 UI 组件，也应是安全审计记录。
- 对真实 provider 的权限请求、命令执行、网络访问和中止链路要形成可追踪事件。

来源：

- https://platform.openai.com/docs/codex
- https://openai.com/index/introducing-codex/
- https://openai.com/index/running-codex-safely/

### OpenHands

可参考点：

1. OpenHands 把 agent 能力抽象为能修改代码、运行命令、浏览网页、调用 API 的开发者式工具集。
2. 支持 GUI、CLI、API、headless 多种运行方式。
3. 默认 agent 能对话，也能通过命令执行任务，强调运行环境和 sandbox。

对 Aria 的启发：

- Workspace 的核心不是聊天，而是“人类确认 + agent 行动 + 证据追溯”的统一动作空间。
- UI 应区分“对话补充”和“执行动作”，避免把所有输入都落到一个 `user_message`。

来源：

- https://docs.openhands.dev/overview/introduction
- https://docs.openhands.dev/openhands/usage/agents
- https://docs.openhands.dev/overview/faqs

## 推荐改进优先级

### 第一阶段：修正 Workspace 动作语义

1. 新增 `context_note` 与 `start_generation` 两种 WebSocket 入站消息。
2. 准备阶段输入只追加上下文，不运行 provider。
3. “开始生成”点击后锁定 provider，并在 Timeline 中创建明确节点。
4. 把断开策略产品化：断开即中止，或后台继续跑。

### 第二阶段：补齐 Timeline 审计能力

1. 持久化 Timeline node detail。
2. 所有 stream chunk、execution event、permission response、abort、review verdict 都归属到 node。
3. 刷新后恢复每个 node 的 detail，而不是只恢复节点壳。
4. Human confirm 必须绑定到 artifact version，而不是只更新实体状态。

### 第三阶段：补 Work Item 执行闭环

1. 启用 Work Item 专属 stage。
2. Plan 确认前禁用 coding。
3. Plan 确认后创建 worktree attempt。
4. coding/testing/review/rework/final 全部进入 Timeline。
5. 支持 attempt diff、checkpoint、rollback、PR 或本地合入动作。

### 第四阶段：优化工作台信息架构

1. 看板上明确当前 Issue 焦点和下游关系。
2. Provider 配置、review rounds、superpowers/OpenSpec 始终可见。
3. 修复右侧 tab 与 selected node 的互斥问题。
4. 删除操作加确认。
5. Project 级默认配置补前端入口。

## 需要确认的流程问题

1. 准备阶段是否允许多次补充上下文而不启动 provider？我的建议是允许，并且必须与“开始生成”分离。
2. Workspace 刷新/关闭时，真实 provider run 应该继续后台运行，还是立即中止？当前代码更接近“断开即中止”。
3. Work Item 的最终交付物是本地 worktree diff、branch、PR，还是 Aria 内部 artifact？外部产品普遍以 branch/PR 作为代码任务的最终审查对象。
4. `ProviderWorkspaceDialog` 是否仍是目标形态？如果不是，应更新设计文档，把全屏 Workspace route 作为正式产品决策。

## 建议验收标准

1. 在准备阶段输入“补充约束”不会触发 provider，Timeline 记录为上下文补充。
2. 只有点击“开始生成”才进入 running，并锁定 provider 配置。
3. 运行中刷新后，产品行为明确：要么恢复控制通道，要么展示“已因断开中止”的 Timeline 事件。
4. 刷新后，Timeline 每个节点仍能看到历史输出、执行事件、权限事件和 review verdict。
5. Artifact version 展示生成者、审核者、确认者和确认时间。
6. Work Item Plan 未确认前不能 coding；确认后能进入 worktree attempt。
7. 右侧 Artifact / 执行 / 节点详情切换真实有效。
8. 删除 Project/Repository/Issue 需要二次确认。

## 当前总体判断

这套 workspace 的方向是对的，尤其是把 Story、Design、Work Item 前置为生命周期实体，并用 Timeline 把 author、reviewer、human 串起来。但现在仍有几个会影响真实使用信任度的问题：

1. 输入动作不够明确，用户容易误启动真实 provider。
2. Timeline 还没有成为真正的审计事实源。
3. Work Item 尚未进入代码执行闭环。
4. Provider 与运行控制对用户不够显性。

建议下一轮不要急着扩更多实体，而是先把 Workspace 变成一个“动作明确、可恢复、可审计、能交付代码变更”的最小闭环。
