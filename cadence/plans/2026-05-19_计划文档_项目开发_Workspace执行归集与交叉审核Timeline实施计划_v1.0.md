# Workspace 执行归集与交叉审核 Timeline 实施计划

## 文档信息

- 文档类型：计划文档
- 日期：2026-05-19
- 版本：v1.0
- 分支：`product-workbench-issue-lifecycle`
- 关联设计：`cadence/designs/2026-05-19_技术方案_Workspace执行归集与交叉审核Timeline设计_v1.0.md`

## 实施策略

采用**后端先行、前端跟进**的策略，分 5 个阶段递增交付。每个阶段完成后可独立验证。

---

## 阶段 1：后端 — Timeline 数据模型与阶段扩展

### 目标
扩展 WorkspaceStage、新增 Timeline 节点模型、扩展 WebSocket 消息类型。

### 任务清单

#### 1.1 扩展 WorkspaceStage 枚举
- 文件：`src/web/workspace_ws_types.rs`
- 改动：`WorkspaceStage` 新增 `ReviewDecision` 和 `Revision` 两个变体
- 文件：`src/product/workspace_engine.rs`
- 改动：`WorkspaceStage` 枚举同步新增，`as_str()` 和 `from_stage_name()` 补充新变体

#### 1.2 新增 Timeline 节点类型定义
- 文件：`src/web/workspace_ws_types.rs`（新增类型）
- 新增：
  - `TimelineNodeType` 枚举（PrepareContext, Generation, Review, ReviewDecision, Revision, HumanConfirm, Completed）
  - `TimelineNodeStatus` 枚举（Active, Paused, Completed, Failed, Skipped）
  - `TimelineNode` 结构体
  - `ReviewVerdictType` 枚举（Pass, Revise, NeedsHuman）
  - `ReviewVerdict` 结构体
  - `ProviderConfigSnapshot` 结构体
  - `ArtifactVersion` 结构体

#### 1.3 扩展 WebSocket 消息类型
- 文件：`src/web/workspace_ws_types.rs`
- `WsOutMessage` 新增变体：
  - `TimelineNodeCreated { node: TimelineNode }`
  - `TimelineNodeUpdated { node_id, status, summary, completed_at }`
  - `ReviewComplete { node_id, round, verdict, comments, summary }`
  - `ReviewDecisionRequired { node_id, round, options }`
- `WsInMessage` 新增变体：
  - `ReviewDecision { decision, extra_context }`
- `WsOutMessage::SessionState` 新增字段：`timeline_nodes`, `active_node_id`, `artifact_versions`
- `WsOutMessage::StreamChunk` 新增字段：`node_id: Option<String>`
- `WsOutMessage::ExecutionEvent` 内 `WsExecutionEvent` 新增字段：`node_id: Option<String>`, `agent: Option<ProviderName>`

#### 1.4 扩展 Artifact 版本模型
- 文件：`src/product/models.rs`
- 新增 `ArtifactVersionRecord` 结构体（version, markdown, generated_by, reviewed_by, review_verdict, confirmed_by, source_node_id, created_at）

#### 1.5 测试
- 为新增的枚举和结构体编写序列化/反序列化测试
- 验证 snake_case 序列化正确

### 验证方式
- `cargo build` 通过
- `cargo test` 通过（新增测试 + 现有测试不回归）

---

## 阶段 2：后端 — CrossReview 执行引擎

### 目标
实现真正的 cross review 执行逻辑，包括 reviewer 调用、结论解析、返修循环。

### 任务清单

#### 2.1 重构 workspace_engine 核心流程
- 文件：`src/product/workspace_engine.rs`
- 改动：
  - `WorkspaceEngine` 新增字段：`timeline_nodes: Vec<TimelineNode>`, `active_node_id: Option<String>`, `current_round: u32`, `review_rounds: u32`
  - 新增方法：`create_timeline_node()` — 创建节点并发送 `TimelineNodeCreated` 事件
  - 新增方法：`update_timeline_node()` — 更新节点状态并发送 `TimelineNodeUpdated` 事件
  - 重构 `complete_assistant_message()` — 生成完成后不再直接跳转 CrossReview+HumanConfirm，改为进入 review 流程

#### 2.2 实现 review 执行流程
- 文件：`src/product/workspace_engine.rs`
- 新增方法：`drive_review_session()` — 核心 review 执行逻辑
  1. 创建 Review 类型 Timeline 节点
  2. 构建 reviewer 输入（`build_review_input()`）
  3. 流式调用 reviewer provider（复用 `drive_provider_session` 的模式）
  4. 解析 reviewer 输出为 `ReviewVerdict`（`parse_review_verdict()`）
  5. 发送 `ReviewComplete` 事件
  6. 根据 verdict 决定下一步

#### 2.3 实现 reviewer 输入构建
- 文件：`src/product/workspace_engine.rs`
- 新增方法：`build_review_input()` — 构建 reviewer prompt
  - 包含：issue 描述 + 用户上下文消息 + artifact markdown + 审核指令
  - 审核指令要求 reviewer 在输出末尾附加 JSON 结论块

#### 2.4 实现 verdict 解析
- 文件：`src/product/workspace_engine.rs`
- 新增方法：`parse_review_verdict(output: &str) -> ReviewVerdict`
  - 从输出末尾提取 JSON 代码块
  - 解析 verdict 和 summary
  - comments 为 JSON 块之前的全部内容
  - 解析失败时默认为 NeedsHuman

#### 2.5 实现 ReviewDecision 等待与处理
- 文件：`src/product/workspace_engine.rs`
- 改动：
  - verdict 为 `Revise` 时：transition 到 `ReviewDecision`，发送 `ReviewDecisionRequired`，创建 ReviewDecision 节点
  - 新增方法：`handle_review_decision(decision, extra_context)` — 处理用户决策
    - "continue"：进入 Revision 阶段
    - "continue_with_context"：记录 extra_context，进入 Revision 阶段
    - "human_intervene"：进入 HumanConfirm

#### 2.6 实现返修流程
- 文件：`src/product/workspace_engine.rs`
- 新增方法：`drive_revision_session(extra_context: Option<String>)` — 返修执行
  1. 创建 Revision 类型 Timeline 节点
  2. 构建返修输入（`build_revision_input()`）：原始上下文 + 上一版 artifact + reviewer 意见 + 用户补充
  3. 流式调用 author provider
  4. 完成后更新 artifact，回到 `drive_review_session()`（下一轮）
  5. 达到 max rounds 时直接进入 HumanConfirm

#### 2.7 Fake provider 处理
- 文件：`src/product/workspace_engine.rs`
- 改动：当 reviewer_provider 为 Fake 时
  - 创建 Review 节点但状态为 Skipped
  - 不调用 provider，直接进入 HumanConfirm
  - 发送 `TimelineNodeUpdated` 标记为 Skipped

#### 2.8 WebSocket handler 适配
- 文件：`src/web/handlers.rs`（或 workspace ws handler 文件）
- 改动：处理新增的 `WsInMessage::ReviewDecision` 消息，调用 engine 的 `handle_review_decision()`

#### 2.9 测试
- 单元测试：`parse_review_verdict` 各种输入场景
- 单元测试：review 流程状态机（pass/revise/needs_human 三条路径）
- 单元测试：max rounds 限制
- 单元测试：Fake provider 跳过逻辑

### 验证方式
- `cargo build` 通过
- `cargo test` 通过
- 手动测试：使用 Fake provider 验证阶段流转正确

---

## 阶段 3：后端 — Timeline 节点生命周期管理

### 目标
完善 Timeline 节点的创建、更新、持久化，以及 SessionState 重建。

### 任务清单

#### 3.1 PrepareContext 节点管理
- 文件：`src/product/workspace_engine.rs`
- 改动：
  - engine 初始化时创建 PrepareContext 节点
  - 用户发消息时更新节点摘要（"N 条消息"）
  - 点击"开始生成"时完成 PrepareContext 节点

#### 3.2 Generation 节点管理
- 文件：`src/product/workspace_engine.rs`
- 改动：
  - `handle_user_message` / `startGeneration` 时创建 Generation 节点
  - 流式事件带 node_id
  - 生成完成时更新节点状态和摘要

#### 3.3 HumanConfirm 和 Completed 节点
- 文件：`src/product/workspace_engine.rs`
- 改动：
  - 进入 HumanConfirm 时创建节点
  - `handle_confirm` 时完成 HumanConfirm 节点，创建 Completed 节点

#### 3.4 SessionState 重建
- 文件：`src/product/workspace_engine.rs`
- 改动：`build_session_state()` 方法返回 timeline_nodes 和 active_node_id
- 旧 session（无 timeline_nodes）时根据 messages 和 stage 重建简化 Timeline

#### 3.5 Timeline 节点持久化
- 文件：`src/product/lifecycle_store.rs`
- 新增方法：
  - `save_timeline_nodes(session_id, nodes)` — 保存节点列表
  - `load_timeline_nodes(session_id) -> Vec<TimelineNode>` — 加载节点列表
- 存储格式：JSON 文件，路径 `{session_dir}/timeline_nodes.json`

#### 3.6 Artifact 版本追溯持久化
- 文件：`src/product/lifecycle_store.rs`
- 新增方法：
  - `append_artifact_version(session_id, version)` — 追加版本记录
  - `list_artifact_versions(session_id) -> Vec<ArtifactVersion>` — 列出所有版本

#### 3.7 测试
- SessionState 重建测试（新 session + 旧 session 兼容）
- Timeline 节点持久化读写测试
- Artifact 版本追溯测试

### 验证方式
- `cargo build` 通过
- `cargo test` 通过
- WebSocket 连接后收到完整 SessionState（含 timeline_nodes）

---

## 阶段 4：前端 — Timeline 视图重构

### 目标
用 Timeline 视图替代当前的"对话区 + 执行 tab"布局。

### 任务清单

#### 4.1 重构 workspace-ws-store
- 文件：`web/src/state/workspace-ws-store.ts`
- 改动：
  - 新增类型：`TimelineNode`, `TimelineNodeType`, `TimelineNodeStatus`, `TimelineNodeDetail`, `ReviewVerdict`, `ArtifactVersion`, `ProviderConfigSnapshot`
  - 重构 state 结构：新增 `timelineNodes`, `activeNodeId`, `selectedNodeId`, `nodeDetails`, `artifactVersions`, `pendingDecision`
  - 新增 actions：`addTimelineNode`, `updateTimelineNode`, `setSelectedNode`, `appendNodeStreamChunk`, `appendNodeExecutionEvent`, `setNodeVerdict`, `setPendingDecision`
  - 消息处理：按 node_id 归集 StreamChunk、ExecutionEvent 到对应节点详情

#### 4.2 更新 useWorkspaceWs hook
- 文件：`web/src/hooks/useWorkspaceWs.ts`
- 改动：
  - 处理新增消息类型：`timeline_node_created`, `timeline_node_updated`, `review_complete`, `review_decision_required`
  - StreamChunk 和 ExecutionEvent 消息中的 node_id 归集
  - 新增出站方法：`sendReviewDecision(decision, extraContext)`

#### 4.3 新增 TimelinePanel 组件
- 文件：`web/src/components/workspace/TimelinePanel.tsx`（新建）
- 内容：
  - 垂直 Timeline 列表，渲染 `TimelineNodeCard` 组件
  - 自动滚动到 active 节点
  - 点击节点切换 selectedNodeId

#### 4.4 新增 TimelineNodeCard 组件
- 文件：`web/src/components/workspace/TimelineNodeCard.tsx`（新建）
- 内容：
  - Agent badge（颜色区分：Claude Code 蓝、Codex 紫、Human 绿、System 灰）
  - 动作标题 + 轮次
  - 状态标签（颜色 + 文字）
  - 摘要文本（一行截断）
  - 耗时
  - 选中态样式
  - Active 节点脉冲动画

#### 4.5 新增 DetailPanel 组件
- 文件：`web/src/components/workspace/DetailPanel.tsx`（新建）
- 内容：
  - 根据选中节点类型渲染不同内容
  - PrepareContext：对话消息列表
  - Generation/Review/Revision：流式输出 + 执行事件列表
  - ReviewDecision：审核意见 + 决策按钮（移到底部操作栏后此处只展示意见）
  - HumanConfirm：Artifact 预览 + 追溯信息
  - Completed：最终 Artifact + 追溯链
  - 执行事件列表复用 ExecutionEventRow 但增加 agent badge

#### 4.6 新增 WorkspaceFooter 组件
- 文件：`web/src/components/workspace/WorkspaceFooter.tsx`（新建）
- 内容：
  - 左侧：阶段进度指示器（5 个大阶段圆点 + 标签）
  - 右侧：根据当前阶段动态显示操作按钮
    - PrepareContext：输入框 + 开始生成
    - Running/CrossReview/Revision：中止按钮
    - ReviewDecision：直接返修 / 补充信息后返修 / 人工介入 + 可选输入框
    - HumanConfirm：确认通过 / 要求修改

#### 4.7 重构 WorkspacePage
- 文件：`web/src/pages/WorkspacePage.tsx`
- 改动：
  - 移除现有的 Chat Panel + Artifact/Execution tab 布局
  - 替换为：Header + (TimelinePanel | DetailPanel) + WorkspaceFooter
  - Header：返回按钮 + Workspace 类型 + Provider 配置（始终可见）+ 连接状态
  - Provider 配置点击可修改（仅 PrepareContext 阶段）

#### 4.8 测试
- workspace-ws-store 单元测试：消息归集、节点状态更新
- TimelineNodeCard 组件测试：各状态渲染
- WorkspaceFooter 组件测试：各阶段按钮显示
- WorkspacePage 集成测试：整体布局渲染

### 验证方式
- `pnpm build` 通过
- `pnpm test` 通过
- 浏览器手动验证 Timeline 布局正确

---

## 阶段 5：端到端集成与验收

### 目标
全链路打通，手动验证三个问题全部解决。

### 任务清单

#### 5.1 端到端流程验证
- 使用真实 provider（Claude Code + Codex）执行完整流程：
  - Issue → 开始生成 → author 流式输出 → review 流式输出 → verdict 展示 → 用户决策 → 返修（如需要）→ 人工确认 → 完成
- 验证 Timeline 节点正确创建和更新
- 验证右侧面板实时流式更新
- 验证 Agent badge 正确显示

#### 5.2 Fake provider 路径验证
- 使用 Fake provider 验证：
  - Review 节点标记为 Skipped
  - 明确展示"未执行真实 review"
  - 流程正常完成

#### 5.3 返修循环验证
- 触发 revise 结论
- 验证三种用户决策路径
- 验证 max rounds 限制
- 验证多轮返修后 Timeline 节点正确累积

#### 5.4 向后兼容验证
- 打开旧 session（无 timeline_nodes）
- 验证 Timeline 重建正确
- 验证不影响现有功能

#### 5.5 验收标准逐项检查
- 对照设计文档第七节验收标准逐项确认

### 验证方式
- 全部验收标准通过
- 无回归问题

---

## 依赖关系

```
阶段 1 (数据模型) → 阶段 2 (执行引擎) → 阶段 3 (节点管理)
                                                    ↓
                                              阶段 4 (前端)
                                                    ↓
                                              阶段 5 (集成验收)
```

阶段 4 依赖阶段 3 的 WebSocket 消息格式稳定后才能开始。

## 风险与缓解

| 风险 | 影响 | 缓解措施 |
|------|------|---------|
| Reviewer provider 输出格式不稳定 | verdict 解析失败 | 默认 NeedsHuman + 完整输出作为 comments |
| 多轮返修导致 token 消耗过大 | 成本和延迟 | review_rounds 限制（1-5）+ 用户可随时人工介入 |
| 前端重构范围大 | 回归风险 | 保留旧组件代码直到新组件验证通过再删除 |
| 旧 session 兼容 | 打开旧数据报错 | timeline_nodes 为 Optional，缺失时重建 |

## 预估工作量

| 阶段 | 预估 |
|------|------|
| 阶段 1：数据模型 | 中等（类型定义 + 测试） |
| 阶段 2：执行引擎 | 大（核心逻辑，最复杂） |
| 阶段 3：节点管理 | 中等（生命周期 + 持久化） |
| 阶段 4：前端重构 | 大（整页重写 + 新组件） |
| 阶段 5：集成验收 | 小（验证为主） |
