# 实施计划：对话式 Workspace 统一改造

> 基于设计文档：`cadence/designs/2026-05-16_技术方案_对话式Workspace统一设计_v1.0.md`
> 分支：`product-workbench-issue-lifecycle`
> 日期：2026-05-16

## 前置条件

- 工作目录：`.worktrees/product-workbench-issue-lifecycle/`
- 后端技术栈：Rust + Axum 0.8 + Tokio
- 前端技术栈：React 19 + TypeScript + Vite + Tailwind + Zustand + TanStack Router/Virtual
- 当前状态：四列看板已实现，ProviderWorkspaceDialog 弹窗已实现，WebSocket 基础设施不存在

---

## Phase 1：后端 WebSocket 基础设施

### Task 1.1：添加 WebSocket 依赖

**文件**：`Cargo.toml`

**变更**：
- axum 添加 `ws` feature：`axum = { version = "0.8", features = ["ws"] }`
- 确认 `tokio-stream` 和 `futures-util` 已存在（已有）

**验证**：`cargo check` 通过

---

### Task 1.2：定义 WebSocket 消息协议类型

**新建文件**：`src/web/workspace_ws_types.rs`

**内容**：
```rust
// 下行消息（Server → Client）
pub enum WsOutMessage {
    StreamChunk { role: String, content: String },
    MessageComplete { message_id: String, checkpoint_id: String },
    StageChange { stage: String },
    ArtifactUpdate { version: u32, markdown: String, diff: Option<String> },
    ProviderSelectRequest { stage: String, defaults: ProviderDefaults },
    SessionState { messages: Vec<...>, stage: String, artifact: Option<String>, checkpoints: Vec<...> },
    Error { message: String },
}

// 上行消息（Client → Server）
pub enum WsInMessage {
    UserMessage { content: String },
    Rollback { checkpoint_id: String },
    Confirm,
    ProviderSelect { role: String, provider: String },
    Abort,
}
```

**验证**：`cargo check` 通过

---

### Task 1.3：实现 CheckpointStore

**新建文件**：`src/product/checkpoint_store.rs`

**职责**：
- 持久化 checkpoint 到 `.aria/projects/{project}/workspace-sessions/{session_id}/checkpoints/`
- 每个 checkpoint 一个 JSON 文件（序号递增）
- 方法：`create_checkpoint(session_id, message_index, artifact_snapshot, stage)`
- 方法：`rollback_to(session_id, checkpoint_id)` — 删除目标之后的所有 checkpoint 文件
- 方法：`list_checkpoints(session_id)` — 列出所有 checkpoint
- 方法：`get_checkpoint(session_id, checkpoint_id)` — 获取单个 checkpoint

**数据结构**：
```rust
pub struct Checkpoint {
    pub id: String,
    pub session_id: String,
    pub message_index: u32,
    pub artifact_snapshot: String,
    pub stage: String,
    pub created_at: String,
}
```

**验证**：单元测试覆盖 CRUD + rollback 截断逻辑

---

### Task 1.4：实现 WorkspaceEngine 流程引擎

**新建文件**：`src/product/workspace_engine.rs`

**职责**：
- 管理单个 workspace session 的完整生命周期
- 处理上行消息（user_message / rollback / confirm / provider_select / abort）
- 驱动阶段转换（PrepareContext → Running → CrossReview → HumanConfirm → Completed）
- 调用 ProviderAdapter 并通过 broadcast channel 推送流式输出
- 创建 checkpoint

**核心结构**：
```rust
pub struct WorkspaceEngine {
    session_id: String,
    project_id: String,
    issue_id: String,
    stage: WorkspaceStage,
    lifecycle_store: Arc<LifecycleStore>,
    checkpoint_store: Arc<CheckpointStore>,
    provider_registry: Arc<ProviderRegistry>,
    tx: broadcast::Sender<WsOutMessage>,
    abort_handle: Option<tokio::task::JoinHandle<()>>,
}
```

**关键方法**：
- `async fn handle_message(&mut self, msg: WsInMessage)` — 分发处理
- `async fn run_provider(&mut self, role: ProviderRole)` — 启动 provider 生成（spawn task）
- `async fn rollback_to(&mut self, checkpoint_id: String)` — 回退
- `async fn abort_current(&mut self)` — 中断当前 provider
- `fn build_session_state(&self) -> WsOutMessage::SessionState` — 构建完整状态快照

**依赖**：需要将 `ProviderAdapter` trait 改为 async（见 Task 1.5）

**验证**：单元测试覆盖阶段转换、回退、abort

---

### Task 1.5：ProviderAdapter 异步化

**文件**：`src/cross_cutting/provider_adapter.rs`（或当前 trait 所在位置）

**变更**：
- 将 `fn run(...)` 改为 `async fn run(...) -> Result<ProviderOutputStream>`
- `ProviderOutputStream` 为 `tokio::sync::mpsc::Receiver<String>`（逐 chunk 推送）
- `FakeProviderAdapter` 适配为 async（模拟逐字符输出）
- 新增 `fn abort(&self)` 方法（通过 CancellationToken 实现）

**影响**：`ProviderWorkspaceRunner::run_next()` 需要适配（可保留同步版本作为兼容，新增 async 版本供 WorkspaceEngine 使用）

**验证**：现有测试仍通过 + 新增 async adapter 测试

---

### Task 1.6：实现 WebSocket Handler

**新建文件**：`src/web/workspace_ws.rs`

**职责**：
- 注册路由：`GET /api/workspace-sessions/:session_id/ws` → WebSocket upgrade
- 连接建立后：
  1. 加载或创建 WorkspaceEngine
  2. 推送 `SessionState`（初始状态）
  3. 循环读取上行消息 → 分发给 engine
  4. engine 通过 broadcast channel 推送下行消息 → 写入 WebSocket

**路由注册**：在 `src/web/app.rs` 的 `build_web_router()` 中添加

**验证**：集成测试 — WebSocket 连接、发送 user_message、接收 stream_chunk + message_complete

---

### Task 1.7：ProviderRegistry 实现

**新建文件**：`src/product/provider_registry.rs`

**职责**：
- 管理可用的 provider adapter 实例
- 根据 provider name（ClaudeCode / Codex / Fake）返回对应 adapter
- 支持动态选择

**验证**：单元测试

---

## Phase 2：前端路由与页面框架

### Task 2.1：添加前端依赖

**文件**：`web/package.json`

**新增依赖**：
- `zustand` — 状态管理（当前项目没有，用的是自定义 mutable store）
- `@tanstack/react-virtual` — 虚拟化列表
- `diff` — 文本 diff 计算（用于 artifact diff toggle）

**命令**：`pnpm add zustand @tanstack/react-virtual diff && pnpm add -D @types/diff`

**验证**：`pnpm build` 通过

---

### Task 2.2：配置路由

**文件**：`web/src/router.tsx`

**变更**：从单一 rootRoute 改为支持子路由：
```typescript
const rootRoute = createRootRoute({ component: RootLayout });
const workbenchRoute = createRoute({ getParentRoute: () => rootRoute, path: '/workbench', component: AppShell });
const workspaceRoute = createRoute({ getParentRoute: () => rootRoute, path: '/workbench/workspace/$sessionId', component: WorkspacePage });
const indexRoute = createRoute({ getParentRoute: () => rootRoute, path: '/', component: () => <Navigate to="/workbench" /> });
```

**注意**：需要保持现有 AppShell 作为看板页面的入口不变

**验证**：`pnpm build` 通过，访问 `/workbench` 显示看板

---

### Task 2.3：创建 WorkspacePage 骨架

**新建文件**：`web/src/pages/WorkspacePage.tsx`

**内容**：三栏布局骨架
```typescript
export function WorkspacePage() {
  const { sessionId } = useParams({ from: '/workbench/workspace/$sessionId' });
  return (
    <div className="h-screen flex flex-col">
      <WorkspaceTopBar />
      <div className="flex-1 flex overflow-hidden">
        <FlowRail />
        <ChatArea />
        <ArtifactPane />
      </div>
      <WorkspaceStatusBar />
    </div>
  );
}
```

**验证**：路由跳转到 `/workbench/workspace/test` 显示骨架布局

---

## Phase 3：前端 WebSocket 与状态管理

### Task 3.1：创建 Workspace Zustand Store

**新建文件**：`web/src/state/workspace-store.ts`

**内容**：完整的 workspace 状态定义（参考设计文档 Section 7.2）
- connectionStatus, sessionId, stage, messages, checkpoints
- artifact (current, previous, showDiff)
- providers, providerSelectPending
- actions: sendMessage, rollbackTo, confirmArtifact, selectProvider, abort

**验证**：单元测试覆盖 store actions

---

### Task 3.2：实现 WebSocket Hook

**新建文件**：`web/src/hooks/useWorkspaceWebSocket.ts`

**职责**：
- 建立 WebSocket 连接到 `/api/workspace-sessions/:sessionId/ws`
- 注册消息处理器，更新 Zustand store
- 断线自动重连（指数退避）
- 组件卸载时关闭连接

**消息处理映射**：
- `stream_chunk` → 追加到 messages 最后一条的 content
- `message_complete` → 标记消息完成，记录 checkpoint
- `stage_change` → 更新 stage
- `artifact_update` → 更新 artifact.current/previous
- `session_state` → 全量替换 store 状态
- `provider_select_request` → 设置 providerSelectPending
- `error` → 显示错误

**验证**：集成测试（mock WebSocket server）

---

### Task 3.3：实现 ChatArea 组件

**新建文件**：`web/src/components/workspace-chat/ChatArea.tsx`

**子组件**：
- `MessageList.tsx` — 虚拟化消息列表（TanStack Virtual）
- `ChatMessage.tsx` — 单条消息（角色标签 + 内容 + 回退按钮 + 时间戳）
- `StreamingIndicator.tsx` — 流式输出指示器
- `ChatInput.tsx` — 输入框 + 发送/abort 按钮

**关键行为**：
- 消息按时间顺序展示，角色标签区分 `[User]` `[Author]` `[Reviewer]` `[System]`
- 流式输出时最后一条消息实时更新
- 每条已完成消息旁有回退图标，hover 高亮后续消息
- 底部输入框：Running 阶段显示 abort 按钮，其他阶段显示发送按钮
- 自动滚动到底部

**验证**：组件渲染测试 + 交互测试

---

### Task 3.4：实现 FlowRail 组件（新版）

**新建文件**：`web/src/components/workspace-chat/FlowRail.tsx`

**行为**：
- 根据 workspace 类型显示不同阶段列表
- 文档类：PrepareContext / Running / CrossReview / HumanConfirm / Completed
- Coding 类：PrepareContext / PlanGeneration / PlanConfirm / Coding / Testing / CodeReview / Rework / HumanConfirm / Completed
- 当前阶段高亮（primary 色），已完成打勾（success 色），未到达灰色
- 点击已完成阶段：滚动到该阶段的第一条消息

**验证**：组件渲染测试

---

### Task 3.5：实现 ArtifactPane 组件

**新建文件**：`web/src/components/workspace-chat/ArtifactPane.tsx`

**子组件**：
- `DocArtifact.tsx` — Markdown 渲染 + diff toggle（使用 react-markdown + diff 库）
- `CodingArtifact.tsx` — 多 tab（Plan / Changes / TestResults）

**行为**：
- 可折叠（右侧面板宽度可调或折叠为图标）
- 文档类：实时展示 artifact.current 的 markdown 渲染，toggle 时展示与 previous 的 diff
- Coding 类：根据当前阶段自动切换 active tab

**验证**：组件渲染测试

---

### Task 3.6：实现 WorkspaceTopBar 和 StatusBar

**新建文件**：
- `web/src/components/workspace-chat/WorkspaceTopBar.tsx`
- `web/src/components/workspace-chat/WorkspaceStatusBar.tsx`

**TopBar 内容**：返回按钮 + 实体标题 + Provider badge + 阶段状态
**StatusBar 内容**：WebSocket 连接状态指示灯 + 当前阶段文本 + 会话耗时

**验证**：组件渲染测试

---

## Phase 4：看板入口改造

### Task 4.1：修改看板卡片入口

**文件**：`web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`

**变更**：
- "生成 Story Spec" / "生成 Design Spec" / "生成 Work Item" 按钮改为：
  - 如果无 session → 调用 API 创建 session → `navigate('/workbench/workspace/' + sessionId)`
  - 如果有进行中 session → 直接 `navigate('/workbench/workspace/' + sessionId)`
- 移除 `ProviderWorkspaceDialog` 的引用和渲染
- 卡片上显示 session 状态标签（进行中 / 已完成）

**验证**：点击"生成"按钮跳转到 workspace 页面

---

### Task 4.2：Provider 选择面板

**新建文件**：`web/src/components/workspace-chat/ProviderSelectPanel.tsx`

**行为**：
- 当 store 中 `providerSelectPending = true` 时，在 ChatArea 中弹出选择面板
- 显示可用 provider 列表 + 默认选中项
- 用户选择后发送 `provider_select` 消息
- 面板消失，流程继续

**验证**：交互测试

---

## Phase 5：回退机制前端实现

### Task 5.1：消息回退交互

**文件**：`web/src/components/workspace-chat/ChatMessage.tsx`

**变更**：
- 每条已完成消息（有 checkpoint_id）右上角显示回退图标（默认半透明）
- Hover 消息时：图标变为实色 + 该消息之后的所有消息添加半透明遮罩
- 点击回退图标：弹出确认 popover（"回退到此处？之后的 N 条消息将被丢弃"）
- 确认后：调用 store.rollbackTo(checkpointId)
- 流式输出中：回退按钮 disabled

**验证**：交互测试 — hover 高亮 + 点击回退 + 状态更新

---

## Phase 6：Coding Workspace 特殊处理

### Task 6.1：Coding 阶段扩展

**文件**：`src/product/workspace_engine.rs`

**变更**：
- WorkspaceEngine 支持 `CodingWorkspaceStage` 枚举
- PlanGeneration → PlanConfirm（硬 gate）→ Coding → Testing → CodeReview → Rework 循环
- Rework 最多 3 轮后强制进入 HumanConfirm
- Coding abort 时执行 `git checkout .` 恢复 worktree

**验证**：单元测试覆盖 Coding 阶段转换 + Rework 循环上限

---

### Task 6.2：CodingArtifact 多 Tab 实现

**文件**：`web/src/components/workspace-chat/CodingArtifact.tsx`

**内容**：
- Plan tab：步骤列表 + 完成状态标记
- Changes tab：文件树 + diff 展示（需要后端提供 git diff 数据）
- Test Results tab：测试输出 + 通过/失败统计

**后端支持**：`artifact_update` 消息需要扩展，Coding 类型时携带 `plan_steps`、`git_diff`、`test_results` 字段

**验证**：组件渲染测试 + tab 切换

---

## Phase 7：集成测试与清理

### Task 7.1：端到端集成测试

**新建文件**：`tests/workspace_ws_integration.rs`

**覆盖场景**：
1. 创建 session → WebSocket 连接 → 收到 session_state
2. 发送 user_message → 收到 stream_chunk + message_complete + stage_change
3. 回退到 checkpoint → 收到新的 session_state
4. 完整文档类流程：prepare → run → cross review → confirm
5. 断线重连 → 收到正确的 session_state

**验证**：`cargo test` 全部通过

---

### Task 7.2：前端 E2E 测试

**新建文件**：`web/e2e/workspace-chat.spec.ts`（Playwright）

**覆盖场景**：
1. 从看板点击"生成" → 跳转到 workspace 页面
2. 输入消息 → 看到流式输出
3. 点击回退 → 消息被截断
4. 完成 confirm → 返回看板，卡片状态更新

**验证**：`pnpm exec playwright test` 通过

---

### Task 7.3：清理旧代码

**文件**：
- 删除 `web/src/components/workspace/ProviderWorkspaceDialog.tsx`
- 删除 `web/src/components/workspace/ProviderWorkspaceLaunchDialog.tsx`
- 删除 `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx` 中的 dialog 相关状态和逻辑
- 保留 `WorkspaceManager.tsx`（CRUD 管理，与对话式 workspace 无关）

**验证**：`pnpm build` + `cargo build` 通过，无 dead code 警告

---

## 实施顺序与依赖关系

```
Phase 1 (后端基础设施)
  1.1 → 1.2 → 1.3 → 1.5 → 1.4 → 1.7 → 1.6
  
Phase 2 (前端路由) — 可与 Phase 1 并行
  2.1 → 2.2 → 2.3

Phase 3 (前端核心组件) — 依赖 Phase 2
  3.1 → 3.2 → 3.3, 3.4, 3.5, 3.6 (并行)

Phase 4 (看板入口) — 依赖 Phase 3
  4.1, 4.2 (并行)

Phase 5 (回退机制) — 依赖 Phase 3 + Phase 1
  5.1

Phase 6 (Coding 特殊处理) — 依赖 Phase 1 + Phase 3
  6.1, 6.2 (并行)

Phase 7 (集成测试) — 依赖所有前置 Phase
  7.1, 7.2, 7.3
```

## 风险与注意事项

1. **ProviderAdapter 异步化（Task 1.5）是最大风险点**——影响现有同步调用路径，需要保持向后兼容
2. **路由改造（Task 2.2）**——当前是单页面应用无子路由，改为多路由需要确保不破坏现有看板功能
3. **WebSocket 连接管理**——需要处理好断线重连、多 tab 打开同一 session 等边界情况
4. **Coding Workspace 的 git 操作**——abort 时的 `git checkout .` 需要确保不影响用户未提交的工作
