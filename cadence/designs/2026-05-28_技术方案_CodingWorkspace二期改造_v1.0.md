# CodingWorkspace 二期改造技术方案

> 版本：v1.0 | 日期：2026-05-28

## 1. 概述

### 1.0 评审修订说明

本方案经 `cadence/designs-reviews/2026-05-28_设计评审_CodingWorkspace二期改造_v1.0.md` 评审后，实施顺序做如下约束：

- 先执行 P0 当前场景预备收口，保证 Work Item 上下文、验证命令、prompt 可见和 PrepareContext provider_select 在现有架构下可用。
- 5 角色 Provider 模型采用兼容式迁移，保留现有 author/reviewer 对外协议，新增内部角色配置并从旧快照派生默认值。
- Stage Gate 必须先引入后台 `AttemptRunner`、runner command channel 和 Gate 持久化；不能在当前同步 WebSocket 执行流中直接实现。
- Test Agent Loop 先补齐 `ToolCall` / `ToolResult` 结构化工具协议，再实现 Tester 白名单。
- InternalPrReview 保持在 ReviewRequest(push) 之后执行，以现有稳定代码链路为准。

### 1.1 背景

CodingWorkspace 一期实现了基本的 Coding → Test → CodeReview → Rework → ReviewRequest 流水线，但存在两个核心问题：

1. **Prompt 不可见**：用户输入的 ContextNote 没有回显到聊天区域，无法确认 prompt 是否正确
2. **Provider 不可切换**：provider 在 attempt 创建时锁定，运行中无法按阶段切换

### 1.2 改造目标

- 对齐 ChatWorkspace / SpecWorkspace 的 UX 体验（消息气泡、streaming、tool_call 卡片嵌套）
- Test 阶段 LLM 化（Agent Loop 模式）
- Provider 按阶段独立配置，支持运行时切换
- Rework 改为"分析官"角色（只读分析 + 路由决策，不修改代码）
- 每个 LLM 阶段前加 Stage Gate（5s 倒计时确认闸）

### 1.3 技术路线

**方案 B：独立后端协议 + 前端组件复用**

- 后端：CodingWorkspace 保持独立的 WS 协议和 Engine，不与 ChatWorkspace 合并
- 前端：复用 ChatWorkspace 的 `ChatEntryList`、`MessageGroupView`、`InlineEventRow` 等展示组件

---

## 2. 状态机设计

### 2.1 阶段流转

````
┌─────────────────────────────────────────────────────────────────┐
│                     CodingAttempt 生命周期                        │
├─────────────────────────────────────────────────────────────────┤
│                                                                   │
│  [Gate] → Coding → [Gate] → Testing → [Gate] → Rework(Analyst)  │
│                                                          │        │
│                              ┌────────────────────────────┘        │
│                              ▼                                     │
│                    ┌─── 判定结果 ───┐                              │
│                    │                │                              │
│              有问题可修复      无问题/需人工                        │
│                    │                │                              │
│                    ▼                ▼                              │
│              [Gate] → Coding   [Gate] → CodeReview                │
│                                         │                         │
│                                    ┌────┘                         │
│                                    ▼                              │
│                              Rework(Analyst)                      │
│                                    │                              │
│                         ┌──────────┼──────────┐                   │
│                         ▼          ▼          ▼                   │
│                    有问题     需人工补充    无问题                  │
│                    → Coding   → 暂停等待   → InternalPrReview     │
│                                                    │              │
│                                              通过 → 完成          │
│                                              不通过 → Rework      │
└─────────────────────────────────────────────────────────────────┘
````

### 2.2 Stage Gate 机制

每个 LLM 阶段开始前，后端发送 `coding_stage_gate` 事件：

- 前端展示 5 秒倒计时
- 用户可在倒计时内：
  - 确认立即开始（`stage_gate_confirm`）
  - 切换该阶段的 Provider（`provider_select`）
  - 中止 attempt（`abort_attempt`）
- 倒计时结束自动进入下一阶段

### 2.3 Rework 分析官

Rework 阶段的 Provider（Analyst）只做分析和路由决策：

- **输入**：上一阶段的 summary + 本轮新增的 ContextNote
- **输出**：`AnalystVerdict` 结构化判定
- **不修改代码**，不调用 tool_use

````rust
pub enum AnalystVerdict {
    /// 有问题可自动修复 → 回到 Coding
    NeedsFix { summary: String, fix_hints: Vec<String> },
    /// 需要人工补充信息 → 暂停等待用户输入
    NeedsHumanInput { questions: Vec<String> },
    /// 无问题 → 进入下一阶段
    NoIssue { summary: String },
}
````

### 2.4 Rework 计数与限制

- 仅限制 Coding 重写次数（默认 max_rewrite = 3）
- Rework 分析官不计入重写次数
- 达到上限后自动进入 CodeReview（带 warning 标记）

---

## 3. 数据模型

### 3.1 Provider 角色定义

````rust
/// 5 个独立 Provider 角色
pub enum CodingProviderRole {
    /// 编码阶段
    Coder,
    /// 测试阶段（Agent Loop）
    Tester,
    /// 分析官（Rework 路由决策）
    Analyst,
    /// 代码审查（只分析变更 diff）
    CodeReviewer,
    /// 内部 PR 审查（分析功能影响、输出影响范围）
    InternalReviewer,
}
````

### 3.2 ProviderConfigSnapshot 扩展

````rust
pub struct CodingProviderConfigSnapshot {
    pub coder: ProviderConfig,
    pub tester: ProviderConfig,
    pub analyst: ProviderConfig,
    pub code_reviewer: ProviderConfig,
    pub internal_reviewer: ProviderConfig,
}
````

- 创建 attempt 时从 session/repository 默认配置派生
- 运行时可通过 `provider_select` 消息修改非当前阶段的 provider

### 3.3 CodingChatEntry

````rust
/// 统一的聊天条目（对齐 ChatWorkspace）
pub struct CodingChatEntry {
    pub id: Uuid,
    pub attempt_id: Uuid,
    pub node_id: String,          // 用于分组（stage_name + sequence）
    pub role: CodingAgentRole,    // Author/Tester/Analyst/CodeReviewer/InternalReviewer/System/User
    pub entry_type: CodingEntryType,
    pub content: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}

pub enum CodingEntryType {
    /// 用户输入的 ContextNote（回显）
    UserMessage,
    /// LLM 文本输出
    AssistantMessage,
    /// tool_use 调用（嵌套在 assistant 气泡内）
    ToolCall { tool_name: String, input: serde_json::Value },
    /// tool_result（嵌套在 assistant 气泡内）
    ToolResult { tool_use_id: String, output: String, is_error: bool },
    /// Stage Gate 事件
    StageGate { stage: CodingExecutionStage, countdown_seconds: u8 },
    /// Analyst 判定结果
    AnalystVerdict(AnalystVerdict),
    /// 阶段完成 summary
    StageSummary { stage: CodingExecutionStage, summary: String },
    /// 系统事件（错误、警告等）
    SystemEvent { event_type: String, message: String },
}
````

### 3.4 CodingContextNote 改进

````rust
pub struct CodingContextNote {
    pub id: Uuid,
    pub attempt_id: Uuid,
    pub content: String,
    pub created_at: DateTime<Utc>,
    /// 标记是否已被 Rework 消费
    pub consumed_by_rework_round: Option<u32>,
}
````

- ContextNote 仅注入到下一次 Rework 的 prompt 中
- 注入范围：仅包含上次 Rework 之后新增的 ContextNote（`consumed_by_rework_round IS NULL`）
- 注入后标记 `consumed_by_rework_round = current_round`

### 3.5 TestingReport 扩展

````rust
pub struct TestingReport {
    pub test_commands_executed: Vec<TestCommandExecution>,
    pub bugs_found: Vec<BugReport>,
    pub summary: String,
    /// Tester Agent Loop 的完整对话记录
    pub agent_conversation: Vec<CodingChatEntry>,
}

pub struct TestCommandExecution {
    pub command: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

pub struct BugReport {
    pub description: String,
    pub severity: BugSeverity,
    pub related_files: Vec<String>,
    pub reproduction_steps: Option<String>,
}
````

---

## 4. WebSocket 协议

### 4.1 Server → Client 消息

````rust
pub enum CodingWsOutMessage {
    // === 已有（保留） ===
    ExecutionEvent(WsExecutionEvent),
    AttemptStatusChanged { status: AttemptStatus },
    Error { code: String, message: String },

    // === 新增 ===
    /// 聊天条目（统一格式，streaming 逐 token 推送）
    CodingChatEntry(CodingChatEntry),

    /// Stage Gate 倒计时开始
    CodingStageGate {
        stage: CodingExecutionStage,
        countdown_seconds: u8,
        current_provider: ProviderConfig,
    },

    /// Provider 配置已更新
    CodingProviderConfigUpdated {
        role: CodingProviderRole,
        new_config: ProviderConfig,
    },

    /// Analyst 判定结果
    AnalystVerdictResult(AnalystVerdict),

    /// Streaming token（增量文本）
    StreamingToken {
        node_id: String,
        delta: String,
    },

    /// Streaming 结束
    StreamingEnd {
        node_id: String,
    },
}
````

### 4.2 Client → Server 消息

````rust
pub enum CodingWsInMessage {
    // === 已有（保留） ===
    StartCoding,
    FinalConfirm { approved: bool },
    AbortAttempt,

    // === 新增 ===
    /// 用户输入 ContextNote（后端回显 + 存储）
    ContextNote { content: String },

    /// Stage Gate 确认（立即开始）
    StageGateConfirm { stage: CodingExecutionStage },

    /// 切换 Provider（仅非当前阶段）
    ProviderSelect {
        role: CodingProviderRole,
        config: ProviderConfig,
    },
}
````

### 4.3 ContextNote 处理流程

```
Client                          Server
  │                               │
  │── ContextNote{content} ──────▶│
  │                               │── 存储到 coding_context_notes 表
  │                               │── 生成 CodingChatEntry(UserMessage)
  │◀── CodingChatEntry ──────────│   (回显给前端)
  │                               │── 标记为待注入 Rework
  │                               │
```

### 4.4 Stage Gate 交互流程

```
Client                          Server
  │                               │
  │                               │── 阶段完成，准备进入下一阶段
  │◀── CodingStageGate ─────────│   (stage, countdown=5, provider)
  │                               │
  │   [用户可选操作]               │
  │── StageGateConfirm ─────────▶│── 立即开始下一阶段
  │── ProviderSelect ───────────▶│── 更新 provider，继续等待
  │── AbortAttempt ─────────────▶│── 中止
  │                               │
  │   [5s 超时]                   │── 自动开始下一阶段
  │                               │
```

---

## 5. 前端组件设计

### 5.1 组件复用策略

| ChatWorkspace 组件 | CodingWorkspace 复用方式 |
|-------------------|------------------------|
| `ChatEntryList` | 直接复用，传入 `CodingChatEntry[]` |
| `MessageGroupView` | 直接复用，按 `node_id` 分组 |
| `InlineEventRow` | 直接复用，tool_call 卡片嵌套在气泡内 |
| `ExecutionEventEntry` | 直接复用，展示命令执行结果 |
| `ChatInputBar` | 改造为 `CodingComposer`（加入 optimistic echo） |

### 5.2 新增组件

#### StageGateEntry

- 展示 Stage Gate 倒计时卡片
- 包含：阶段名称、当前 Provider、倒计时进度条、确认/中止按钮
- 倒计时结束后变为"已自动确认"静态卡片

#### CodingProviderConfigPanel

- 展示 5 个 Provider 角色的当前配置
- 非当前阶段的 Provider 可点击切换
- 当前阶段的 Provider 显示为锁定状态（灰色）

#### AnalystVerdictEntry

- 展示 Rework 分析官的判定结果
- 根据 verdict 类型显示不同样式：
  - `NeedsFix`：橙色，展示修复建议列表
  - `NeedsHumanInput`：蓝色，展示问题列表 + 输入提示
  - `NoIssue`：绿色，展示通过 summary

### 5.3 角色颜色映射

| 角色 | 颜色 | 说明 |
|------|------|------|
| Coder | 蓝色 (blue-600) | 编码阶段气泡 |
| Tester | 紫色 (purple-600) | 测试阶段气泡 |
| Analyst | 琥珀色 (amber-600) | 分析官判定气泡 |
| CodeReviewer | 绿色 (green-600) | 代码审查气泡 |
| InternalReviewer | 靛蓝色 (indigo-600) | PR 审查气泡 |
| User | 灰色 (gray-600) | 用户输入气泡 |
| System | 红色 (red-500) | 系统事件 |

### 5.4 Timeline 左侧栏

对齐 SpecWorkspace 的 Timeline 设计：

- 左侧展示阶段节点列表（Coding → Testing → Rework → CodeReview → ...）
- 当前阶段高亮
- 已完成阶段显示 checkmark
- 点击节点滚动到对应消息区域

### 5.5 消息分组规则

扩展 `message-grouping.ts` 的 `groupEntries` 函数：

- `role` 扩展为：`coder | tester | analyst | code_reviewer | internal_reviewer | user | system`
- 按 `node_id` 分组（`node_id` = `{stage}_{round}_{sequence}`）
- tool_call / tool_result 嵌套在父 assistant 气泡内（`InlineEventRow`）
- StageGate / AnalystVerdict 作为独立条目，不参与分组

---

## 6. Test 阶段 Agent Loop

### 6.1 设计原则

- Tester Provider 以 Agent Loop 模式运行
- 可调用 tool_use 执行测试命令
- **不修改源码**（tool_use 白名单限制）
- 发现 bug 继续测试，最终输出 TestingReport

### 6.2 Tester 可用 Tool

| Tool | 说明 |
|------|------|
| `run_command` | 执行测试命令（pytest、cargo test、npm test 等） |
| `read_file` | 读取文件内容（分析测试结果） |
| `list_files` | 列出目录结构 |
| `search_code` | 搜索代码（定位相关测试文件） |

**禁止的 Tool**：`write_file`、`edit_file`、`delete_file`（Tester 不修改代码）

### 6.3 测试命令推断

Tester 根据项目配置推断可用的测试命令：

| 检测文件 | 推断命令 |
|---------|---------|
| `Cargo.toml` | `cargo test` |
| `package.json` (scripts.test) | `npm test` / `pnpm test` |
| `pytest.ini` / `pyproject.toml` | `pytest` |
| `Makefile` (test target) | `make test` |
| `.github/workflows/` | 从 CI 配置提取测试步骤 |

### 6.4 Agent Loop 流程

```
Tester Provider 启动
    │
    ├── 分析变更文件，确定测试范围
    │
    ├── Loop:
    │   ├── 选择测试命令
    │   ├── tool_use: run_command
    │   ├── 分析结果
    │   ├── 如果有失败：记录 BugReport，继续测试其他方面
    │   └── 如果全部通过：结束 loop
    │
    └── 输出 TestingReport（summary + bugs + commands）
```

### 6.5 Tester 终止条件

- 所有测试通过 → 输出 NoIssue summary
- 发现 bug → 输出 bug 列表 + summary
- 超时（默认 5 分钟）→ 输出已完成的测试结果 + timeout warning
- 连续 3 次 tool_use 失败 → 输出错误 summary

---

## 7. 错误处理

### 7.1 Provider 错误

| 场景 | 处理 |
|------|------|
| Provider 连接失败 | 重试 2 次，失败后暂停在 Stage Gate，提示用户切换 Provider |
| Provider 超时 | 同上 |
| Provider 返回格式错误 | 记录错误日志，重试 1 次，失败后进入 Rework 分析 |
| Streaming 中断 | 保留已接收内容，标记为 incomplete，进入 Rework |

### 7.2 Stage Gate 超时

- 默认 5 秒倒计时
- 超时后自动确认进入下一阶段
- 如果用户在倒计时内发送了 ProviderSelect，重置倒计时

### 7.3 Coding 重写次数限制

- 默认 `max_rewrite = 3`
- 达到上限：
  - 发送 SystemEvent 警告
  - 跳过 Rework，直接进入 CodeReview（带 `exceeded_rewrite_limit` 标记）
  - CodeReview 的 prompt 中注明已达重写上限

### 7.4 Git 操作错误

| 场景 | 处理 |
|------|------|
| worktree 创建失败 | 中止 attempt，提示用户检查 git 状态 |
| commit 冲突 | 暂停，提示用户手动解决 |
| branch 已存在 | 自动追加序号（`-2`、`-3`） |

### 7.5 Tester 约束违反

- 如果 Tester 尝试调用 `write_file` 等禁止 tool → 拦截，返回 tool_result error
- 连续 3 次违反 → 终止 Tester Agent Loop，输出 warning

### 7.6 ContextNote 边界

- 单条 ContextNote 最大 4000 字符
- 单次 Rework 注入的 ContextNote 总量最大 10000 字符（超出截断最早的）
- 空内容 ContextNote 直接忽略

### 7.7 WebSocket 重连

- 前端断线重连后，后端推送当前 attempt 的完整 `CodingChatEntry[]` 历史
- 如果正在 streaming，重连后从当前位置继续推送
- Stage Gate 倒计时在重连后重新计算剩余时间

---

## 8. 实施优先级

### Phase 1：基础设施

1. 数据模型扩展（CodingChatEntry、ProviderConfigSnapshot 5 角色）
2. WS 协议新增消息类型
3. ContextNote 回显 + 存储

### Phase 2：Stage Gate + Provider 切换

4. Stage Gate 后端逻辑
5. Stage Gate 前端组件
6. ProviderSelect 消息处理
7. CodingProviderConfigPanel

### Phase 3：Test Agent Loop

8. Tester Provider Agent Loop 实现
9. Tool 白名单机制
10. TestingReport 生成

### Phase 4：Rework 分析官

11. Analyst Provider 实现
12. AnalystVerdict 结构化输出解析
13. 路由决策执行

### Phase 5：前端 UX 对齐

14. ChatEntryList 复用集成
15. MessageGroupView 角色扩展
16. Timeline 左侧栏
17. 角色颜色映射

### Phase 6：CodeReview + InternalPrReview

18. CodeReviewer Provider（分析 diff）
19. InternalReviewer Provider（功能影响分析）
20. Review 结果展示

---

## 9. 与一期方案的差异对照

| 维度 | 一期 | 二期 |
|------|------|------|
| Provider 数量 | 1 个（attempt 级别锁定） | 5 个角色独立配置 |
| Provider 切换 | 不支持 | Stage Gate 间隔窗口可切换 |
| Test 阶段 | 纯命令执行 | LLM Agent Loop |
| Rework | 修改代码 | 分析官（只读路由） |
| ContextNote | 无回显、无持久化 | 回显 + 持久化 + Rework 注入 |
| 前端展示 | 简单事件列表 | 消息气泡 + tool_call 嵌套 + Timeline |
| Stage Gate | 无 | 每个 LLM 阶段前 5s 确认闸 |
| 错误恢复 | 简单重试 | 分级处理 + Provider 切换 |

---

## 10. 开放问题

1. **Tester 超时时间**：默认 5 分钟是否合适？大型项目测试可能需要更长时间
2. **ContextNote 持久化**：是否需要跨 attempt 保留？当前设计为 attempt 级别
3. **InternalReviewer 输出格式**：PR description / commit message 的具体模板待定
4. **并发 attempt**：是否允许同一 work_item 同时运行多个 attempt？当前设计为互斥
