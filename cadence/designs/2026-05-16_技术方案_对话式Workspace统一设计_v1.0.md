# 对话式 Workspace 统一设计方案

> 版本：v1.0 | 日期：2026-05-16

## 1. 概述

### 1.1 目标

将所有 Workspace（Story Spec / Design Spec / Work Item / Coding）从当前的弹窗模式升级为类似 Claude Code / Codex 的全屏对话式交互体验。用户可以动态输入、实时看到后端流式输出、在任何阶段干预、并支持消息级回退。

### 1.2 范围

| Workspace 类型 | 输入 | 产物 |
|---------------|------|------|
| Story Spec | Issue | Story Spec 文档 |
| Design Spec | Story Spec | Design Spec 文档 |
| Work Item | Story Spec + Design Spec | Work Item 文档 |
| Coding | Work Item（已确认） | Plan + 代码变更 + 测试结果 |

### 1.3 核心设计决策

| 决策项 | 选择 | 理由 |
|--------|------|------|
| 载体 | 全屏面板（路由切换） | 空间充足、沉浸感、对标 Claude Code |
| 通信 | WebSocket 双向 | 消息回退需双向通信、cross review 交互自然 |
| 交互模式 | 全程可对话 | 任何阶段用户都可输入干预 |
| 回退 | 消息级回退到任意历史点 | 参考 vibe-kanban，checkpoint 模式 |
| 产物展示 | 文档类：实时同步 + diff toggle；Coding 类：多 tab | 覆盖不同产物形态 |
| Cross review | 透明展示，角色标签区分 | 建立信任、用户可及时干预 |
| Provider 配置 | 动态可配置，阶段切换时确认 | 灵活性 |
| 会话 | 持久化单会话 | 支持中途离开恢复，回退替代多会话 |

## 2. 页面结构与路由

### 2.1 路由

```
/workbench                          → 看板主页（四列：Issue/Story/Design/WorkItem）
/workbench/workspace/:sessionId     → Workspace 全屏对话页面
```

### 2.2 全屏页面布局（三栏）

```
┌─────────────────────────────────────────────────────────┐
│ TopBar: [← 返回看板] [实体标题] [Provider 配置] [状态]    │
├────────┬──────────────────────────────┬─────────────────┤
│        │                              │                 │
│ Flow   │       Chat Area              │   Artifact      │
│ Rail   │                              │   Pane          │
│        │  ┌─────────────────────┐     │                 │
│ ○ prep │  │ [Author] 正在生成... │     │  (文档类)       │
│ ● run  │  │ streaming content   │     │  Markdown预览   │
│ ○ rev  │  │                     │     │  + Diff toggle  │
│ ○ conf │  ├─────────────────────┤     │                 │
│        │  │ 输入框              │     │  (Coding类)     │
│        │  └─────────────────────┘     │  Plan|Changes|  │
│        │                              │  TestResults    │
├────────┴──────────────────────────────┴─────────────────┤
│ StatusBar: [WebSocket 连接状态] [当前阶段] [耗时]         │
└─────────────────────────────────────────────────────────┘
```

- **Flow Rail（左侧）**：垂直节点列表，当前阶段高亮，已完成打勾，可点击查看该阶段的历史消息
- **Chat Area（中间）**：对话消息流 + 底部输入框，支持流式渲染，消息带角色标签和回退按钮
- **Artifact Pane（右侧）**：可折叠，文档类展示 markdown + diff，Coding 类展示多 tab

## 3. WebSocket 通信协议

### 3.1 连接

```
ws://host/api/workspace-sessions/:sessionId/ws
```

### 3.2 下行消息（Server → Client）

| 类型 | 用途 | Payload |
|------|------|---------|
| `stream_chunk` | Provider 流式输出文本片段 | `{"type":"stream_chunk","role":"author","content":"..."}` |
| `message_complete` | 消息处理完毕 | `{"type":"message_complete","message_id":"...","checkpoint_id":"..."}` |
| `stage_change` | 流程阶段变更 | `{"type":"stage_change","stage":"cross_review"}` |
| `artifact_update` | 产物内容更新 | `{"type":"artifact_update","version":3,"markdown":"...","diff":"..."}` |
| `provider_select_request` | 请求用户选择 provider | `{"type":"provider_select_request","stage":"cross_review","defaults":{"reviewer":"codex"}}` |
| `session_state` | 完整会话状态快照 | `{"type":"session_state","messages":[...],"stage":"...","artifact":"...","checkpoints":[...]}` |
| `error` | 错误信息 | `{"type":"error","message":"..."}` |

### 3.3 上行消息（Client → Server）

| 类型 | 用途 | Payload |
|------|------|---------|
| `user_message` | 用户发送对话消息 | `{"type":"user_message","content":"..."}` |
| `rollback` | 回退到指定 checkpoint | `{"type":"rollback","checkpoint_id":"..."}` |
| `confirm` | 用户确认产物 | `{"type":"confirm"}` |
| `provider_select` | 用户选择 provider | `{"type":"provider_select","role":"reviewer","provider":"codex"}` |
| `abort` | 中断当前 provider 执行 | `{"type":"abort"}` |

### 3.4 重连机制

- 断线后自动重连
- 重连时服务端推送 `session_state` 恢复完整状态
- 每条 `message_complete` 携带 `checkpoint_id`，客户端记录最后收到的 checkpoint
- 重连时发送 last checkpoint_id，服务端只推送之后的增量

## 4. 消息级回退机制

### 4.1 Checkpoint 数据模型

```rust
struct Checkpoint {
    id: String,
    session_id: String,
    message_index: u32,
    artifact_snapshot: String,
    stage: WorkspaceStage,
    created_at: String,
}
```

### 4.2 回退流程

1. 用户点击某条历史消息上的"回退到此处"按钮
2. 前端发送 `{"type":"rollback","checkpoint_id":"xxx"}`
3. 后端：
   - 如果有正在运行的 provider 调用，先 abort
   - 截断该 checkpoint 之后的所有消息记录
   - 恢复产物到该 checkpoint 的 artifact_snapshot
   - 恢复流程阶段到该 checkpoint 的 stage
4. 后端推送 `session_state`（完整状态快照）
5. 前端用新状态替换当前视图

### 4.3 前端交互

- 每条 `message_complete` 类型的消息旁边显示回退图标
- Hover 时高亮该消息及之后的所有消息（表示"这些会被丢弃"）
- 点击后弹出确认："回退到此处？之后的 N 条消息将被丢弃"
- 正在流式输出时，回退按钮不可用（需先 abort 或等待完成）

### 4.4 边界情况

- 回退到 PrepareContext 阶段：产物清空，用户重新开始
- 回退跨越 stage 变更：stage 一起回退
- 回退时 provider 正在运行：先 abort，等停止后再执行回退
- Coding 类 abort：worktree 中可能有部分修改，需 `git checkout .` 恢复到 checkpoint 的 worktree 状态

## 5. 统一流程引擎

### 5.1 文档类阶段定义

```rust
enum WorkspaceStage {
    PrepareContext,   // 准备上下文，用户可补充信息
    Running,          // Author provider 生成产物
    CrossReview,      // Reviewer provider 审查 + Author 修订
    HumanConfirm,     // 用户确认最终产物
    Completed,        // 产物已确认，会话结束
}
```

### 5.2 阶段转换规则

| 当前阶段 | 触发条件 | 下一阶段 |
|---------|---------|---------|
| PrepareContext | 用户显式发送"开始生成"指令 | Running |
| Running | Author provider 输出完成 | CrossReview |
| CrossReview | Reviewer 审查 + Author 修订完成 | HumanConfirm |
| HumanConfirm | 用户发送 confirm | Completed |
| HumanConfirm | 用户发送修改意见 | Running（重新生成） |

### 5.3 用户干预对阶段的影响

- **PrepareContext**：用户消息作为补充上下文，不触发阶段变更
- **Running**：用户消息中断当前生成（abort + 将用户消息作为新指令重新 run）
- **CrossReview**：用户消息作为额外 review 意见注入，reviewer/author 需要响应
- **HumanConfirm**：确认则完成，否则视为修改请求回到 Running

### 5.4 Coding Workspace 扩展阶段

```rust
enum CodingWorkspaceStage {
    PrepareContext,
    PlanGeneration,    // 生成 Plan
    PlanConfirm,       // 用户确认 Plan（硬 gate，拒绝则回到 PlanGeneration）
    Coding,            // 执行编码
    Testing,           // 运行测试
    CodeReview,        // Reviewer 审查代码
    Rework,            // 根据 review 修改
    HumanConfirm,      // 用户最终确认
    Completed,
}
```

Rework 循环：CodeReview 发现问题 → Rework → Testing → 通过则 HumanConfirm，失败则再次 Rework（最多 3 轮，超过后强制进入 HumanConfirm 由用户决策）。

## 6. Provider 配置与多角色调度

### 6.1 默认 Provider 配置

| Workspace 类型 | Author/Coder | Reviewer |
|---------------|-------------|----------|
| Story Spec | Claude Code | Codex |
| Design Spec | Claude Code | Codex |
| Work Item | Claude Code | Codex |
| Coding - Plan | Claude Code | — |
| Coding - Coding | Codex | — |
| Coding - Review | Claude Code | — |
| Coding - Testing | Claude Code | — |

### 6.2 配置时机

1. **Workspace 启动时**：选择 Author provider（有默认值，可跳过）
2. **进入 CrossReview/CodeReview 阶段时**：推送 `provider_select_request`，用户确认或修改 reviewer
3. **Coding 阶段切换时**：provider 与上一阶段不同时，推送选择确认

### 6.3 Provider Adapter 接口

```rust
trait ProviderAdapter {
    async fn generate(&self, input: ProviderInput) -> Result<ProviderOutputStream>;
    async fn abort(&self) -> Result<()>;
}

struct ProviderInput {
    system_prompt: String,
    context: Vec<ContextItem>,
    conversation: Vec<Message>,
    artifact: Option<String>,
}
```

### 6.4 Cross Review 调度流程

1. Running 完成 → artifact v1 生成
2. stage → CrossReview
3. 推送 `provider_select_request`（reviewer）
4. 用户确认 reviewer provider
5. 调用 `reviewer.generate(input: issue + artifact v1)`
6. Reviewer 输出 review 意见（流式展示，标签 `[Reviewer]`）
7. 调用 `author.generate(input: 原始上下文 + review 意见)`
8. Author 输出修订版 artifact v2（流式展示，标签 `[Author]`）
9. stage → HumanConfirm

用户在步骤 6-8 期间可随时发消息干预。

## 7. 前端组件架构

### 7.1 组件树

```
WorkspacePage (路由页面)
├── WorkspaceTopBar
│   ├── BackButton
│   ├── EntityTitle
│   ├── ProviderBadge
│   └── SessionStatus
├── WorkspaceBody (三栏)
│   ├── FlowRail
│   │   └── StageNode[]
│   ├── ChatArea
│   │   ├── MessageList (虚拟化)
│   │   │   └── ChatMessage[] (角色标签 + 内容 + 回退按钮)
│   │   ├── StreamingIndicator
│   │   └── ChatInput
│   └── ArtifactPane (可折叠)
│       ├── DocArtifact (markdown + diff toggle)
│       └── CodingArtifact (Plan/Changes/TestResults tabs)
└── WorkspaceStatusBar
```

### 7.2 状态管理（Zustand）

```typescript
interface WorkspaceStore {
  // 连接
  connectionStatus: 'connecting' | 'connected' | 'disconnected';
  
  // 会话
  sessionId: string;
  stage: WorkspaceStage;
  messages: ChatMessage[];
  checkpoints: Checkpoint[];
  
  // 产物
  artifact: {
    current: string;
    previous: string;
    showDiff: boolean;
  };
  
  // Provider
  providers: { author: string; reviewer: string | null };
  providerSelectPending: boolean;
  
  // 操作
  sendMessage: (content: string) => void;
  rollbackTo: (checkpointId: string) => void;
  confirmArtifact: () => void;
  selectProvider: (role: string, provider: string) => void;
  abort: () => void;
}
```

### 7.3 WebSocket Hook

```typescript
function useWorkspaceWebSocket(sessionId: string) {
  // 建立连接，注册消息处理器
  // stream_chunk → 追加到最后一条消息的 content
  // message_complete → 标记消息完成，记录 checkpoint
  // stage_change → 更新 stage
  // artifact_update → 更新产物状态
  // session_state → 全量替换状态
  // 断线自动重连，携带 last checkpoint_id
}
```

### 7.4 虚拟化策略

- 历史消息使用 TanStack Virtual 虚拟化
- 尾部 8 条消息不虚拟化（保证流式输出区域在真实 DOM）
- 流式输出时扩展到 24 条不虚拟化
- 自动滚动到底部，支持"锁定底部"模式

## 8. 后端架构扩展

### 8.1 新增模块

```
src/
├── web/
│   ├── workspace_ws.rs        // WebSocket handler（新增）
│   └── handlers.rs            // 扩展：workspace session CRUD
├── product/
│   ├── workspace_engine.rs    // 流程引擎（新增）
│   ├── checkpoint_store.rs    // Checkpoint 持久化（新增）
│   └── provider_workspace_runner.rs  // 扩展：abort、多 provider
```

### 8.2 WorkspaceEngine

```rust
struct WorkspaceEngine {
    session_id: String,
    stage: WorkspaceStage,
    checkpoint_store: CheckpointStore,
    provider_registry: ProviderRegistry,
    tx: broadcast::Sender<WsOutMessage>,
}

impl WorkspaceEngine {
    async fn handle_user_message(&mut self, content: String);
    async fn run_provider(&mut self, role: ProviderRole, input: ProviderInput);
    async fn rollback_to(&mut self, checkpoint_id: String);
    async fn abort_current(&mut self);
    fn transition_stage(&mut self, next: WorkspaceStage);
}
```

### 8.3 WebSocket Handler

```rust
async fn workspace_ws_handler(
    ws: WebSocketUpgrade,
    Path(session_id): Path<String>,
    State(app_state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_workspace_socket(socket, session_id, app_state))
}
```

### 8.4 Checkpoint 持久化

存储路径：`.aria/projects/{project}/workspace-sessions/{session_id}/checkpoints/`

```
checkpoints/
├── 001_prepare_ctx.json
├── 002_user_msg.json
├── 003_author_complete.json
├── 004_reviewer_complete.json
└── 005_revision_complete.json
```

## 9. 入口与导航

### 9.1 入口方式

| 操作 | 触发位置 | 行为 |
|------|---------|------|
| 生成 Story Spec | Issue 卡片"生成"按钮 | 创建/恢复 session → 路由跳转 |
| 生成 Design Spec | Story Spec 卡片"生成"按钮 | 同上 |
| 生成 Work Item | Design Spec 卡片"生成"按钮 | 同上 |
| 执行 Coding | Work Item 卡片"执行"按钮 | 同上（需 plan_status=Confirmed） |
| 恢复进行中 | 卡片"继续"标识 | 路由跳转，WebSocket 重连 |

### 9.2 卡片状态联动

- 无 session → 显示"生成"按钮
- Session 进行中 → 显示"继续"标识 + 当前阶段标签
- Session completed → 显示产物摘要 + "查看"按钮

### 9.3 离开与恢复

- 顶栏"← 返回"按钮回到看板
- 离开不中断 session（持久化）
- Provider 正在运行时离开，provider 继续执行
- 回来后通过 `session_state` 恢复完整状态

## 10. Coding Workspace 特殊处理

### 10.1 与文档类的差异

| 维度 | 文档类 | Coding 类 |
|------|--------|-----------|
| 阶段数 | 5 | 9（含 Plan/Coding/Testing/Rework） |
| 产物 | Markdown 文档 | Plan + 代码变更 + 测试结果 |
| 右侧面板 | Markdown + diff | 多 tab |
| Provider 切换 | 1 次 | 多次 |
| 工作目录 | 无 | Worktree |

### 10.2 启动前置条件

1. Work Item 的 `plan_status` 为 Confirmed（硬 gate）
2. 创建或复用 worktree（基于默认分支）
3. 设置 worktree_path 到 session 上下文

### 10.3 多 Tab 产物面板

| Tab | 内容 | 更新时机 |
|-----|------|---------|
| Plan | 步骤列表 + 完成状态 ✓/○ | PlanGeneration 完成；Coding 每步标记 ✓ |
| Changes | 文件树 + diff | Coding/Rework 实时更新 |
| Test Results | 命令输出 + 通过/失败统计 | Testing 完成时 |

### 10.4 Rework 循环

```
CodeReview 发现问题 → Rework → Testing → 通过则 HumanConfirm，失败则再次 Rework（最多 3 轮）
```

### 10.5 Abort 特殊性

- 文档类 abort：丢弃未完成文本，无副作用
- Coding 类 abort：`git checkout .` 恢复 worktree 到上一个 checkpoint 状态
