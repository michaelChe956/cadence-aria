# Workspace 执行归集与交叉审核 Timeline 设计

## 文档信息

- 文档类型：技术方案
- 日期：2026-05-19
- 版本：v1.0
- 分支：`product-workbench-issue-lifecycle`
- 关联问题清单：`cadence/analysis-docs/2026-05-19_分析报告_Workspace执行归集与交叉审核可见性问题总结_v1.0.md`

## 设计目标

解决三个核心问题：

1. **交叉审核执行**：后端真正调用 reviewer provider 执行审核，支持 pass/revise/needs_human 三种结论和返修循环
2. **Agent 职责可见性**：每个执行节点标识执行者，Artifact 可追溯，Provider 配置可见
3. **Timeline 归集视图**：用 Timeline 作为 Workspace 主视图，替代当前分散的信息区域

## 一、后端 CrossReview 执行引擎

### 1.1 阶段状态扩展

当前 `WorkspaceStage` 扩展为 7 个状态：

```rust
enum WorkspaceStage {
    PrepareContext,
    Running,
    CrossReview,
    ReviewDecision,   // 新增：等待用户决策
    Revision,         // 新增：author 返修中
    HumanConfirm,
    Completed,
}
```

阶段流转路径：

```
PrepareContext → Running → CrossReview → [verdict]
                                           ├─ pass → HumanConfirm → Completed
                                           ├─ needs_human → HumanConfirm → Completed
                                           └─ revise → ReviewDecision → [用户选择]
                                                          ├─ 直接返修 → Revision → CrossReview (下一轮)
                                                          ├─ 补充后返修 → Revision → CrossReview (下一轮)
                                                          └─ 人工介入 → HumanConfirm → Completed
```

达到 `review_rounds` 最大轮数后，无论结论如何，进入 `HumanConfirm`。

### 1.2 Reviewer 输入构建

reviewer provider 收到的 prompt 包含：

1. Issue 描述（原始需求）
2. 用户在 prepare_context 阶段补充的上下文消息
3. Author 生成的 artifact markdown（当前版本）
4. 审核指令：要求输出结构化结论

### 1.3 Reviewer 输出格式

reviewer provider 需要返回结构化结论：

```rust
struct ReviewVerdict {
    verdict: ReviewVerdictType,
    comments: String,    // 审核意见全文
    summary: String,     // 一句话摘要（用于 Timeline 节点卡片）
}

enum ReviewVerdictType {
    Pass,
    Revise,
    NeedsHuman,
}
```

解析策略：reviewer 的 prompt 要求其在输出末尾附加一个 JSON 块，格式如下：

```json
{"verdict": "pass|revise|needs_human", "summary": "一句话摘要"}
```

后端从流式输出的最后一个 ``` 代码块中提取该 JSON。审核意见全文为 JSON 块之前的所有内容。如果无法解析则默认为 `NeedsHuman`，comments 为完整输出。

### 1.4 返修输入构建

当用户选择返修时，author 收到的 prompt：

1. 原始 issue 描述 + 用户上下文
2. 上一版 artifact markdown
3. Reviewer 审核意见全文
4. 用户补充信息（如果用户选择了"补充信息后返修"）
5. 明确指令："请根据以上审核意见修改产物"

### 1.5 新增 WebSocket 消息类型

后端 → 前端：

```rust
// Timeline 节点生命周期
TimelineNodeCreated { node: TimelineNode }
TimelineNodeUpdated { node_id: String, status: TimelineNodeStatus, summary: Option<String>, completed_at: Option<String> }

// 节点内流式内容（带 node_id 归集）
StreamChunk { node_id: String, content: String }
MessageComplete { node_id: String, message_id: String }
ExecutionEvent { node_id: String, event: ExecutionEvent }

// Review 专用
ReviewComplete { node_id: String, round: u32, verdict: String, comments: String, summary: String }
ReviewDecisionRequired { node_id: String, round: u32, options: Vec<String> }
```

前端 → 后端：

```rust
ReviewDecisionResponse { decision: String, extra_context: Option<String> }
// decision: "continue" | "continue_with_context" | "human_intervene"
```

### 1.6 执行事件扩展

`ExecutionEvent` 新增 `agent` 字段：

```rust
struct ExecutionEvent {
    event_id: String,
    node_id: String,           // 新增：归属的 Timeline 节点
    agent: Option<ProviderName>, // 新增：执行者标识
    kind: ExecutionEventKind,
    status: ExecutionEventStatus,
    title: String,
    detail: Option<String>,
    command: Option<String>,
    cwd: Option<String>,
    output: Option<String>,
}
```

## 二、Timeline 数据模型

### 2.1 Timeline 节点

```rust
struct TimelineNode {
    node_id: String,
    node_type: TimelineNodeType,
    agent: Option<ProviderName>,
    stage: WorkspaceStage,
    round: Option<u32>,
    status: TimelineNodeStatus,
    title: String,
    summary: Option<String>,
    started_at: String,
    completed_at: Option<String>,
    duration_ms: Option<u64>,
    artifact_ref: Option<String>,
    provider_config_snapshot: ProviderConfigSnapshot,
}

enum TimelineNodeType {
    PrepareContext,
    Generation,
    Review,
    ReviewDecision,
    Revision,
    HumanConfirm,
    Completed,
}

enum TimelineNodeStatus {
    Active,
    Paused,
    Completed,
    Failed,
    Skipped,
}

struct ProviderConfigSnapshot {
    author: ProviderName,
    reviewer: Option<ProviderName>,
    review_rounds: u32,
}
```

### 2.2 节点详情内容

每个节点关联的详情数据（前端按 node_id 归集）：

```rust
struct TimelineNodeDetail {
    node_id: String,
    messages: Vec<WsMessageDto>,
    streaming_content: Option<String>,
    execution_events: Vec<ExecutionEvent>,
    verdict: Option<ReviewVerdict>,
    user_decision: Option<UserDecision>,
}

struct UserDecision {
    decision: String,
    extra_context: Option<String>,
    decided_at: String,
}
```

### 2.3 Artifact 版本追溯

```rust
struct ArtifactVersion {
    version: u32,
    markdown: String,
    generated_by: ProviderName,
    reviewed_by: Option<ProviderName>,
    review_verdict: Option<ReviewVerdictType>,
    confirmed_by: Option<String>,
    created_at: String,
    source_node_id: String,
}
```

### 2.4 Session State 扩展

`WsOutMessage::SessionState` 新增 timeline 字段：

```rust
SessionState {
    session_id: String,
    workspace_type: WorkspaceType,
    stage: String,
    timeline_nodes: Vec<TimelineNode>,       // 新增
    active_node_id: Option<String>,          // 新增
    providers: WsProviderConfig,
    artifact: Option<String>,
    artifact_versions: Vec<ArtifactVersion>, // 新增
}
```

## 三、前端 Timeline 视图

### 3.1 整体布局

```
┌─────────────────────────────────────────────────────────────────┐
│ Header                                                          │
│ [← 返回] [Story Spec]        [Author: Claude Code | Reviewer: Codex] │
├────────────────────────────┬────────────────────────────────────┤
│ Timeline (左侧, ~35%)      │ Detail Panel (右侧, ~65%)          │
│                            │                                    │
│ ┌────────────────────┐    │ ┌──────────────────────────────┐  │
│ │ ○ 准备上下文         │    │ │ [流式输出全文]                 │  │
│ │   2条消息 · 完成     │    │ │                              │  │
│ └────────────────────┘    │ │                              │  │
│ ┌────────────────────┐    │ ├──────────────────────────────┤  │
│ │ ● [Claude Code]     │◄──┤ │ [执行事件列表]                 │  │
│ │   Story Spec 生成    │    │ │  ├ turn: 分析需求             │  │
│ │   运行中 · 12s       │    │ │  ├ command: cat issue.md     │  │
│ └────────────────────┘    │ │  └ artifact: 生成完成          │  │
│ ┌────────────────────┐    │ └──────────────────────────────┘  │
│ │ ○ [Codex]           │    │                                    │
│ │   Review Round 1    │    │                                    │
│ │   等待中             │    │                                    │
│ └────────────────────┘    │                                    │
├────────────────────────────┴────────────────────────────────────┤
│ Footer                                                          │
│ [● 准备 ● 运行中 ○ 审查 ○ 确认 ○ 完成]    [中止] [输入...] [发送] │
└─────────────────────────────────────────────────────────────────┘
```

### 3.2 左侧 Timeline 节点卡片

每张卡片内容：

- **Agent badge**：彩色标签
  - `[Claude Code]` — 蓝色
  - `[Codex]` — 紫色
  - `[Human]` — 绿色
  - `[System]` — 灰色（prepare_context、completed）
- **动作标题**：如 "Story Spec 生成"、"Review Round 1"、"返修 Round 1"
- **状态标签**：
  - Active — 蓝色脉冲
  - Paused — 橙色
  - Completed — 绿色
  - Failed — 红色
  - Skipped — 灰色删除线
- **摘要文本**：一行，截断显示
- **耗时**：右上角小字
- **选中态**：左侧边框高亮 + 背景色变化

### 3.3 右侧 Detail Panel

根据选中节点类型展示不同内容：

| 节点类型 | 面板内容 |
|---------|---------|
| PrepareContext | 用户对话消息列表（气泡形式） |
| Generation | 流式输出全文（markdown 渲染）+ 执行事件列表 |
| Review | 流式输出全文 + 执行事件列表 + 结论卡片（verdict badge + comments） |
| ReviewDecision | 审核意见展示 + 三个决策按钮 + 可选输入框 |
| Revision | 流式输出全文 + 执行事件列表 |
| HumanConfirm | Artifact 预览（markdown 渲染）+ 追溯信息 + 确认/要求修改操作 |
| Completed | 最终 Artifact + 完整追溯链 |

执行事件列表中每条事件带 Agent badge，复用现有 `ExecutionEventRow` 组件但增加 agent 标识。

### 3.4 底部操作栏

阶段条 + 操作区合并为一行：

- **左侧**：阶段进度指示器（圆点 + 标签，紧凑排列，显示 5 个大阶段）
  - 大阶段映射：`PrepareContext` → 准备上下文，`Running` → 运行中，`CrossReview`/`ReviewDecision`/`Revision` → 交叉审查，`HumanConfirm` → 人工确认，`Completed` → 已完成
  - 已完成阶段：实心绿点
  - 当前阶段：实心蓝点（脉冲）
  - 未开始阶段：空心灰点
- **右侧**：根据当前阶段动态显示
  - `PrepareContext`：输入框 + "开始生成" 按钮
  - `Running` / `CrossReview` / `Revision`："中止" 按钮
  - `ReviewDecision`："直接返修" / "补充信息后返修" / "人工介入" 按钮；选择"补充信息后返修"时展开输入框
  - `HumanConfirm`："确认通过" / "要求修改" 按钮

### 3.5 Header 区域

- 左侧：返回按钮 + Workspace 类型标题
- 右侧：Provider 配置始终可见（`Author: Claude Code | Reviewer: Codex`），点击可修改（仅在 PrepareContext 阶段允许修改，点击"开始生成"后锁定）
- 连接状态指示器

### 3.6 交互行为

1. **自动聚焦**：新节点创建时自动选中，右侧面板切换到该节点详情
2. **历史回看**：点击任意历史节点查看详情，不影响流程执行
3. **实时更新**：active 节点的右侧面板实时流式更新
4. **回退操作**：有 checkpoint 的节点在卡片上显示回退图标
5. **配置变更**：用户在 PrepareContext 修改 provider 后，Header 实时更新；流程开始后 provider 配置锁定
6. **Skipped 标识**：使用 Fake provider 时，review 节点标记为 Skipped，明确展示"未执行真实 review"

## 四、前端状态管理

### 4.1 Store 重构

`workspace-ws-store.ts` 重构为以 Timeline 为核心的状态结构：

```typescript
interface WorkspaceWsState {
  sessionId: string;
  workspaceType: string;
  stage: string;
  connectionStatus: WsConnectionStatus;
  providers: WsProviderConfig;
  error: string | null;

  // Timeline 核心状态
  timelineNodes: TimelineNode[];
  activeNodeId: string | null;
  selectedNodeId: string | null;  // 用户当前选中查看的节点

  // 节点详情（按 node_id 索引）
  nodeDetails: Record<string, TimelineNodeDetail>;

  // Artifact
  artifact: string | null;
  artifactVersions: ArtifactVersion[];

  // 底部操作栏状态
  pendingDecision: ReviewDecisionRequired | null;
}
```

### 4.2 消息处理

WebSocket 消息按 `node_id` 归集到对应节点的详情中：

- `TimelineNodeCreated` → 追加到 `timelineNodes`，设为 `activeNodeId` 和 `selectedNodeId`
- `TimelineNodeUpdated` → 更新对应节点的 status/summary/completed_at
- `StreamChunk { node_id }` → 追加到 `nodeDetails[node_id].streaming_content`
- `MessageComplete { node_id }` → 将 streaming_content 转为 message，清空 streaming
- `ExecutionEvent { node_id }` → 追加到 `nodeDetails[node_id].execution_events`
- `ReviewComplete` → 设置 `nodeDetails[node_id].verdict`
- `ReviewDecisionRequired` → 设置 `pendingDecision`，阶段切换到 ReviewDecision

## 五、Fake Provider 处理

当 reviewer 配置为 Fake provider 时：

- Review 节点状态标记为 `Skipped`
- 节点摘要显示："未执行真实 review（Fake 快速路径）"
- 不产生 ReviewDecision 节点，直接进入 HumanConfirm
- Artifact 追溯中 `reviewed_by` 标记为 `Fake`，`review_verdict` 为 None

## 六、向后兼容

### 6.1 数据迁移

- 现有 `WorkspaceSessionRecord` 保持不变，新增 `timeline_nodes` 字段（可选）
- 旧 session 打开时，如果没有 timeline_nodes，根据 messages 和 stage 重建一个简化的 Timeline（只有 PrepareContext + Generation + HumanConfirm/Completed）
- 新 session 从创建时就生成 Timeline 节点

### 6.2 WebSocket 协议

- 新增的消息类型不影响旧客户端（旧客户端忽略未知消息类型）
- `SessionState` 中新增字段为 Optional，旧客户端不受影响

## 七、验收标准

### 问题 1：交叉审核执行

- [ ] CrossReview 阶段真正调用 reviewer provider，流式执行
- [ ] Reviewer 输出解析为结构化结论（pass/revise/needs_human）
- [ ] revise 时暂停等用户决策，支持三种选择
- [ ] 返修循环正常工作，受 review_rounds 限制
- [ ] Fake provider 时明确标识为 Skipped

### 问题 2：Agent 职责可见性

- [ ] 每个 Timeline 节点有 Agent badge
- [ ] 执行事件带 agent 标识
- [ ] Artifact 版本记录生成者、审核者、确认者
- [ ] Provider 配置在 Header 始终可见
- [ ] 点击"开始生成"后 Provider 配置锁定，不可修改

### 问题 3：Timeline 归集

- [ ] Workspace 主视图为 Timeline（左侧节点卡片 + 右侧详情面板）
- [ ] 节点按动作粒度划分（生成、review N、返修 N、决策、确认）
- [ ] prepare_context 作为整体节点
- [ ] 当前活跃节点自动选中，右侧实时流式更新
- [ ] 底部操作栏固定，阶段条 + 操作按钮合并
- [ ] 历史节点可回看
