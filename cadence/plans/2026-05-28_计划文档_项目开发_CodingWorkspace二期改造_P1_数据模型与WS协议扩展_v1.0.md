# CodingWorkspace 二期 P1：数据模型与 WS 协议扩展

## 文档信息

- 文档类型：计划文档
- 分支：`product-workbench-issue-lifecycle`
- 制定日期：2026-05-28
- 版本：v1.0
- 前置：P0（当前场景预备收口）
- 产出：兼容式 5 角色 Provider 模型、CodingChatEntry、ContextNote 回显闭环
- 设计文档：`cadence/designs/2026-05-28_技术方案_CodingWorkspace二期改造_v1.0.md`
- 设计评审：`cadence/designs-reviews/2026-05-28_设计评审_CodingWorkspace二期改造_v1.0.md`

---

## 一、目标

在现有 CodingWorkspace 基础上完成数据层和协议层扩展。Provider 角色模型采用兼容式迁移，不直接替换现有 author/reviewer 对外协议：

1. 新增 `CodingProviderRole` 枚举（5 角色）
2. 新增 Coding 内部 5 角色配置，并从现有 `ProviderConfigSnapshot` 派生默认值
3. 新增 `CodingChatEntry` + `CodingEntryType` 统一聊天条目模型
4. 改进 `CodingContextNote` 模型（增加 `consumed_by_rework_round` 字段）
5. WS 协议新增 `CodingChatEntry`、`ProviderSelect`、`StageGateConfirm` 消息
6. 实现 ContextNote 后端处理闭环（存储 + 回显）
7. 前端 CodingComposer 增加 optimistic echo

---

## 二、现有代码基线

| 文件 | 现状 |
|------|------|
| `src/product/coding_models.rs` | 已有 `CodingExecutionStage`(9值)、`CodingAttemptStatus`(7值)、`CodingAgentRole`(Author/Tester/Reviewer/Git/System)、`TestingReport`、`CodeReviewReport` 等 |
| `src/web/coding_ws_handler.rs` | 已有 `CodingWsOutMessage`(含 CodingStreamChunk/CodingGateRequired 等)、`CodingWsInMessage`(含 ContextNote/GateResponse 等)，但 ContextNote handler 为空 |
| `src/web/workspace_ws_types.rs` | 已有 `ProviderConfigSnapshot`（单一配置） |
| `web/src/pages/CodingWorkspacePage.tsx` | CodingComposer 无 optimistic echo |
| `web/src/hooks/useCodingWorkspaceWs.ts` | 无 provider_select 消息发送 |

---

## 三、任务清单

### 3.1 CodingProviderRole 枚举（src/product/coding_models.rs）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 1.1 | 新增 `CodingProviderRole` 枚举：Coder / Tester / Analyst / CodeReviewer / InternalReviewer | 序列化测试 | 5 个值正确序列化为 snake_case |
| 1.2 | 为 `CodingProviderRole` 实现 `Display` trait | 单元测试 | 输出人类可读名称 |

````rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingProviderRole {
    Coder,
    Tester,
    Analyst,
    CodeReviewer,
    InternalReviewer,
}
````

### 3.2 CodingProviderConfigSnapshot（src/web/workspace_ws_types.rs 或新文件）

> 修订：本节不替换现有 `ProviderConfigSnapshot`。现有 `author/reviewer/review_rounds` 继续作为 HTTP/WS 对外兼容字段；新增 Coding 内部角色配置，例如 `CodingRoleProviderConfigSnapshot`。在前端和旧 API 完成迁移前，`CodingSessionState.provider_config_snapshot` 仍返回旧结构。

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 2.1 | 新增 `CodingRoleProviderConfigSnapshot` 结构体（5 角色独立 ProviderConfig） | 序列化/反序列化测试 | 各角色配置独立可序列化 |
| 2.2 | 实现 `From<ProviderConfigSnapshot>` 转换（从旧 author/reviewer 派生 5 角色默认值） | 单元测试 | 旧配置可无损转为新格式 |
| 2.3 | `CodingSessionState` 保留旧 `provider_config_snapshot`，新增可选角色配置字段 | 编译通过 | 前后端协议兼容 |

````rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingProviderConfigSnapshot {
    pub coder: ProviderConfig,
    pub tester: ProviderConfig,
    pub analyst: ProviderConfig,
    pub code_reviewer: ProviderConfig,
    pub internal_reviewer: ProviderConfig,
}
````

### 3.3 CodingChatEntry 模型（src/product/coding_models.rs）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 3.1 | 新增 `CodingEntryType` 枚举（UserMessage / AssistantMessage / ToolCall / ToolResult / StageGate / AnalystVerdict / StageSummary / SystemEvent） | 序列化测试 | 各变体正确序列化 |
| 3.2 | 新增 `CodingChatEntry` 结构体 | 序列化测试 | 所有字段正确映射 |
| 3.3 | 新增 `AnalystVerdict` 枚举（NeedsFix / NeedsHumanInput / NoIssue） | 序列化测试 | 结构化判定可序列化 |

````rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CodingEntryType {
    UserMessage,
    AssistantMessage,
    ToolCall { tool_name: String, input: serde_json::Value },
    ToolResult { tool_use_id: String, output: String, is_error: bool },
    StageGate { stage: CodingExecutionStage, countdown_seconds: u8 },
    AnalystVerdict(AnalystVerdict),
    StageSummary { stage: CodingExecutionStage, summary: String },
    SystemEvent { event_type: String, message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingChatEntry {
    pub id: String,
    pub attempt_id: String,
    pub node_id: String,
    pub role: CodingAgentRole,
    pub entry_type: CodingEntryType,
    pub content: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: String,
}
````

### 3.4 CodingContextNote 改进（src/product/coding_models.rs）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 4.1 | 新增 `CodingContextNote` 结构体（含 consumed_by_rework_round） | 序列化测试 | 字段可选、默认 None |

````rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingContextNote {
    pub id: String,
    pub attempt_id: String,
    pub content: String,
    pub created_at: String,
    pub consumed_by_rework_round: Option<u32>,
}
````

### 3.5 WS 协议扩展（src/web/coding_ws_handler.rs）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 5.1 | `CodingWsOutMessage` 新增 `CodingChatEntryCreated { entry: CodingChatEntry }` 变体 | 序列化测试 | 正确序列化为 JSON |
| 5.2 | `CodingWsOutMessage` 新增 `CodingProviderConfigUpdated { role: CodingProviderRole, config: ProviderConfig }` 变体 | 序列化测试 | 正确序列化 |
| 5.3 | `CodingWsInMessage` 新增 `ProviderSelect { role: CodingProviderRole, config: ProviderConfig }` 变体 | 反序列化测试 | 正确解析客户端消息 |
| 5.4 | `CodingWsInMessage` 新增 `StageGateConfirm { stage: CodingExecutionStage }` 变体 | 反序列化测试 | 正确解析 |

### 3.6 ContextNote 后端处理（src/web/coding_ws_handler.rs）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 6.1 | 实现 ContextNote handler：接收 → 创建 CodingContextNote → 存入 attempt 状态 | 单元测试 | ContextNote 被正确存储 |
| 6.2 | ContextNote handler：生成 CodingChatEntry(UserMessage) → 通过 WS 回显 | 集成测试 | 客户端收到 CodingChatEntryCreated |
| 6.3 | `is_coding_ws_message_allowed` 函数允许 ContextNote 在所有活跃状态下发送 | 单元测试 | Running/WaitingForHuman/Blocked 状态均允许 |

### 3.7 前端 Optimistic Echo（web/src/pages/CodingWorkspacePage.tsx）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 7.1 | CodingComposer 发送 ContextNote 后立即 append optimistic entry 到 chatEntries | — | 输入后立即在聊天区看到自己的消息 |
| 7.2 | 收到后端 CodingChatEntryCreated 时去重（替换 optimistic entry） | — | 不出现重复消息 |
| 7.3 | coding-workspace-store 新增 `appendChatEntry` / `replacePendingEntry` action | — | store 操作正确 |

---

## 四、验收标准

1. `cargo test` 全部通过（新增模型序列化测试）
2. `cargo build` 无 warning
3. 前端 `pnpm build` 通过
4. 手动测试：在 CodingWorkspace 输入 ContextNote → 立即看到回显气泡 → 后端存储确认
5. ProviderSelect 消息可正确解析（后续 P2 实现实际处理逻辑）

---

## 五、不做的事

- Stage Gate 的实际倒计时逻辑（P2）
- Provider 切换的实际执行逻辑（P2）
- Test Agent Loop（P3）
- Analyst 判定逻辑（P4）
- 前端 ChatEntryList 复用（P5）
