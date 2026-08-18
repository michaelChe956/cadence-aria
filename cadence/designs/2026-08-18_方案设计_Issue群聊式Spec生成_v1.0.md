# 方案设计：Issue 群聊式 Spec 生成（Group Chat Workspace）v1.0

- 分支：`feat-b-0818-rewrite-spec-workspace`
- 日期：2026-08-18
- 状态：待评审
- 参考案例：[yetone/cumora](https://github.com/yetone/cumora)（重点参考其 `docs/COORDINATION.md` 协调机制）

## 1. 背景与目标

当前 Story Spec / Design Spec 的生成是**单作者 + 单审稿人 + 人类的固定流水线**：每个 workspace session 只有一个 `author_provider` 和一个 `reviewer_provider`，阶段固定为 `PrepareContext → Running → AuthorConfirm →（CrossReview ⇄ Revision）→ ReviewDecision → HumanConfirm → Completed`。人类只能在固定节点做决策，agent 之间无法同屏讨论。

Cumora 验证了另一种交互形态：人类与多个 AI agent 同处一个群聊室，agent 是一等参与者，主动认领工作、互相讨论。本设计将这种形态引入 Aria 的 spec 生成流程。

### 1.1 产品定位（强约束）

- **并行新增、开关切换**：老流水线完整保留，群聊模式为显式 opt-in；两种模式同步演进，直到最终抉择淘汰其一。
- **不重做**：复用现有 ProviderAdapter、LifecycleStore、产物模型、派生链等底层设施。
- **范围**：聊天室覆盖 **Issue 澄清 → Story Spec → Design Spec**；Work Item 不在第一版范围内（枚举预留扩展位）。
- **一个 Issue 一个聊天室**，三条产物线同室产出。

### 1.2 核心交互决策（已与用户逐项确认）

| 维度 | 结论 |
|---|---|
| 定稿方式 | 人类显式定稿（方案 A），逐产物线操作 |
| 消息仲裁 | 智能路由 + 自主认领融合（A+B）；agent 间可互相讨论；机制参考 Cumora |
| 职责边界 | 全链路（Issue 澄清 → Story → Design）在同一个聊天室完成 |
| 角色 | 固定角色全程常驻，coordinator 路由决定活跃时机；每角色独立绑定 provider；支持同角色多实例（如双 reviewer 对抗） |

## 2. 角色体系

角色分权参考 pi-subagents 的内置角色（worker / reviewer / oracle），并映射到现有 `AdapterRole` 与 permission modes 双层约束：

| 角色 | 权限 | 职责 | 对应 pi-subagents 角色 |
|---|---|---|---|
| `author` | 可写 | Story Spec / Issue 澄清稿的执笔人 | worker |
| `frontend-design` | 可写 | Design Spec 前端分节执笔 | worker |
| `backend-design` | 可写 | Design Spec 后端分节执笔 | worker |
| `reviewer` | 只读 | 对抗性审查，给 evidence-backed 意见 | reviewer |
| `researcher` | 只读 | 调研代码库现状、回答架构/现状问题 | oracle |

- 写权限硬编码于角色类型；provider permission modes 仍沿用现有机制，双层约束。
- 默认阵容：`author + reviewer + researcher`；用户可随时添加角色（含同角色多实例，各绑不同 provider）。

## 3. 总体架构

### 3.1 架构选型（已确认：方案一）

**独立 GroupChat 引擎 + 复用底层设施**：新建 `group_chat_engine` 与现有 `workspace_engine` 完全平行、互不感知；复用 ProviderAdapter 体系、LifecycleStore、issue_store、worktree、ProviderConfigPanel 配置形态。淘汰时任一方均为整目录删除，不留疤痕。

已排除的备选：② 扩展现有 workspace_engine（固定 stage 状态机与开放群聊范式冲突，会互相拖累）；③ 内嵌 Cumora 式服务（与单机本地工具架构不匹配，纯过度设计）。

### 3.2 模块结构

````text
src/product/group_chat_engine/
├── mod.rs                 // GroupChatEngine：会话生命周期入口
├── room.rs                // ChatRoom：时间线 + 角色阵容 + 产物线管理
├── timeline.rs            // RoomTimeline：消息/事件追加式存储
├── roles.rs               // 角色定义、写权限分权、provider 绑定
├── coordinator.rs         // triage 路由 + seen-cursor 门控 + 认领仲裁
├── agent_turn.rs          // 单角色一次发言的执行（调 ProviderAdapter）
├── finalize.rs            // 人类定稿 → 写入现有 LifecycleStore/派生链
└── prompts/               // 各角色 system prompt（含协调纪律）

web/src/pages/ChatRoomPage.tsx + web/src/components/chat-room/   // 新 UI
````

老 `workspace_engine` 零改动；`ChatRoomPage` 与 `ChatWorkspacePage` 平行新增。

## 4. 数据模型

### 4.1 核心记录

1. **`GroupChatSessionRecord`**：`id / project_id / issue_id / status(active|finalized|archived) / roles: Vec<RoleInstance> / artifact_lines: Vec<ArtifactLine> / created_at`。
2. **`RoleInstance`**：`{ role_key, provider, display_name, seen_cursor: usize }`（seen_cursor 即 Cumora 的已读游标）。
3. **`ArtifactLine`**（产物线）：`{ kind: IssueRefinement | StorySpec | DesignSpec | (WorkItem 预留), current_draft: Option<ArtifactDraft>, claim: Option<Claim>, finalized_versions: Vec<SpecVersionRecord 引用> }`。
4. **`ArtifactDraft`**：`{ version, markdown, author_role, based_on_events: usize }`（记录起草时的时间线水位，可追溯「此稿基于截至第 N 条消息的讨论」）。

### 4.2 时间线事件（追加式，统一人类消息 / agent 发言 / 系统事件）

- `UserMessage { text, mentions: Vec<role_instance_id> }`
- `AgentMessage { role_instance_id, text, artifact_ref: Option<...> }`
- `ClaimEvent { role_instance_id, artifact_line, claimed/released }`
- `HeldEvent { role_instance_id, reason }`（seen-cursor 门控 HOLD，透明展示）
- `FinalizeEvent { artifact_line, version }`

### 4.3 存储

沿用现有 json_store 模式，每个 session 一个目录：`timeline.jsonl`（事件追加）+ `session.json`（快照）。崩溃/重启后从 timeline 重放恢复，seen_cursor 落盘于事件内；进行中的 agent turn 视为未发生（与现有 interrupted recovery 同语义）。

## 5. Coordinator 仲裁与 agent 间讨论循环

机制逐一对照 Cumora `COORDINATION.md`，适配 Aria 单机进程内引擎（agent 为按需拉起的 CLI 子进程，无 SSE 常驻 daemon，多层限流可简化，但仲裁语义完整保留）。

### 5.1 消息处理流水线

````text
用户消息（可带 @mentions）
   │
   ▼
① Triage Gate（小模型/规则，纯门控）
   - 被 @ 的角色 → 必定 actionable（人类点名必须响应）
   - 未 @ → 按「角色职责 × 消息内容 + 当前产物线状态」路由 0~2 个角色
   - 原则式 prompt，不枚举场景（Cumora 反模式教训）
   │
   ▼
② Agent Turn 执行（被选中角色逐个/有限并行发言）
   快照 seen_cursor 之后的时间线 → 组 prompt → 调 ProviderAdapter
   │
   ▼
③ Freshness 门控（seen-cursor）
   turn 产出落盘前检查时间线是否有快照之后的新事件：
   - 无 → AgentMessage 入库，推进该角色 seen_cursor
   - 有 → HOLD（HeldEvent 透明入时间线），携带新事件重新生成
   语义：「回答必须基于真实已发布状态，而非猜测」
   │
   ▼
④ Verbatim-dup 门控
   与最近一条他人消息逐字相同 → HOLD，不可绕过（无合法场景）
   │
   ▼
⑤ Agent 间讨论循环
   agent 发言本身是新消息 → 回到 ①，由 triage 判定是否有其他角色
   需要响应（reviewer 对 author 新稿提意见即走此路径）
   │
   ▼
⑥ 循环熔断（确定性下限，硬编码不靠模型）
   - 硬上限：两次人类消息之间，agent 间往返 ≤ HARD_LOOP_CAP（默认 12）
   - 空转检测：响应者集合无新增参与者 → 熔断，等待人类
   - 熔断时聊天室发系统提示：「讨论已暂停，等待你的输入」
````

### 5.2 认领机制（Claim）

采用 Cumora 原则：**claim 只存在于真实的共享交付物上，聊天发言永远不 claim**。

- 执笔角色开始写某产物线草稿时原子认领 `ArtifactLine.claim`；定稿或放弃时释放。
- 作用：防止两个执笔角色同时改同一条产物线草稿；UI 展示「Story 稿当前由 author 执笔中」。
- reviewer / researcher 无写权限，天然不需要认领。

### 5.3 并发与节奏（Cumora 限流层的简化版）

- **并发上限**：同一 provider 同时最多 N 个 agent turn（默认 2，可配置）；triage 小模型调用同样受限——两层一起限（Cumora 反模式：只限一层曾导致全机静默）。
- **spawn 间隔**：同 provider 调用至少间隔 500ms（确定性间隔，不用随机抖动）。
- **速率退避**：provider 返回 rate-limit → 该 provider 所有角色静默退避 60s，不在聊天室刷错误。
- 不需要 wake debounce（Aria 无 SSE 并发唤醒，引擎自控循环）。

### 5.4 角色协作纪律（Prompt 层，对应 GLANCE_YIELD_RULES）

各角色 system prompt 共享五条 shape-level 纪律（刻意简短，不枚举场景）：

1. 人类点名某角色（即使没 @）→ 只有该角色回应，其他人不插话。
2. 只基于已发布的真实消息回应，不猜测别人将要说什么。
3. 乐观发言，引擎是安全网；被 HOLD 就重读新状态重新生成。
4. 不重复他人观点；按任务完成度（而非人头数）判断何时收手；缺席角色的活由在场者补位。
5. 永远不 claim 聊天发言，claim 只作用于产物线草稿。

## 6. 产物线流转与人类定稿

### 6.1 产物线定义

| 产物线 | 产出 | 写权限角色 | 定稿去向 |
|---|---|---|---|
| `IssueRefinement` | 细化后的 Issue 描述（Markdown） | author | 更新 `issue_store` 中该 Issue 描述（旧版本入修订历史） |
| `StorySpec` | Story Spec Markdown | author | 写入现有 `LifecycleStore`，成为该 Issue 的 Story Spec 正式版本 |
| `DesignSpec` | 前端 + 后端 Design Spec | frontend-design / backend-design 各写各的分节，author 汇总 | 写入 `LifecycleStore`，成为已定稿 Story 的 Design Spec 正式版本 |

### 6.2 无固定阶段，派生约束保留

聊天室没有阶段切换，产物线何时活跃由讨论内容决定（triage 路由）。但派生链硬约束保留（复用现有 openspec_constraints 校验）：

- `DesignSpec` 定稿前置条件：该 Issue 已有定稿的 Story Spec。未满足时定稿按钮禁用并提示。讨论本身不受限——Design 可先起草，只是不能定稿。
- `IssueRefinement` 无前置约束，任何时候可定稿。

### 6.3 草稿生命周期

1. **起草**：执笔角色认领产物线后基于聊天上下文生成草稿（挂产物线上的版本化对象）。
2. **草稿进群**：草稿以带 `artifact_ref` 的 `AgentMessage` 出现在时间线，reviewer 被 triage 唤醒审查，审稿意见引用具体分节。
3. **修订**：人类或 reviewer 的意见触发执笔角色产出 v2、v3……（由 triage 驱动，非固定 stage）。
4. **并行**：三条线可同时有草稿流转，互不阻塞，仅认领互斥。

### 6.4 人类定稿交互

- 产物线面板常驻状态卡：`未开始 / 起草中(by whom) / 待审 / 可定稿 / 已定稿 vN`。
- 点「定稿」→ 派生约束校验 → 通过后写入 `LifecycleStore`（复用 `AppendSpecVersionInput` 等现有入参），时间线追加 `FinalizeEvent`。后续派生链（Issue 看板、WorkItem workspace 发起）与老流程产物完全同源。
- 定稿后聊天室不关闭：可继续讨论并再次定稿出新版本（对应现有 Spec 版本历史能力）。

## 7. UI、开关与配置

### 7.1 开关与入口

- 全局设置新增「Spec 生成模式」：`流水线模式（默认）| 群聊模式`。
- Issue 看板聚焦态下，群聊模式中 Story/Design 列卡片替换为「群聊工作台」卡片（显示聊天室状态与已定稿产物版本）；点击进入 `ChatRoomPage`；Issue 详情可「开始群聊」直接创建 session。
- 两种模式产物同源同展示，`SpecVersionRecord` 不携带生成方式。

### 7.2 ChatRoomPage 布局

````text
┌──────────────────────────────────────────────────────┐
│ Header：Issue 标题 + 聊天室状态 + 定稿总览            │
├───────────────┬──────────────────────────────────────┤
│ 角色栏         │  聊天时间线                            │
│ · author      │  （人类消息 / agent 发言 / 系统事件     │
│   (Claude)    │   HOLD、claim、熔断提示均为可见事件）   │
│ · reviewer×2  │                                       │
│   (Codex/pi)  │                                       │
│ · researcher  │                                       │
│ · fe/be-design│                                       │
│ [＋添加角色]   │                                       │
├───────────────┴──────────────────────────────────────┤
│ 产物线面板：Issue 澄清 | Story | Design                │
│  —— 状态卡 + 草稿预览 + 定稿按钮（含约束禁用态）        │
├──────────────────────────────────────────────────────┤
│ 输入栏：@提及自动补全 + 发送                            │
└──────────────────────────────────────────────────────┘
````

- 时间线渲染复用现有 `chat-workspace` 组件（ChatEntryList/MessageGroupView 分组、text-display），新增：角色头像/名牌、HeldEvent/ClaimEvent/FinalizeEvent 的 InlineEventRow 变体、@提及高亮。
- WS 推送复用现有 workspace WS 通道，新增 group chat 事件类型。

### 7.3 角色与 Provider 配置

- 「＋添加角色」：角色池选择 → 绑定 provider（复用 ProviderConfigPanel）→ 支持同角色多实例。
- 默认阵容：author + reviewer + researcher；triage 可提示「此话题建议添加前端设计角色」。

## 8. 测试策略

- **Coordinator 单测（重点）**：triage 路由（@必达、职责路由）、seen-cursor HOLD 触发与恢复、verbatim-dup 拒绝、循环熔断（HARD_LOOP_CAP、空转检测）、claim 原子性与互斥、派生约束（Design 定稿前置 Story）。
- **Engine 集成测试**：用现有 `streaming_provider/fake.rs` 假 provider 驱动完整群聊脚本（用户消息 → 多角色发言 → reviewer 审稿 → 修订 → 定稿 → LifecycleStore 落盘断言）；崩溃恢复（timeline.jsonl 重放、seen_cursor 一致性）。
- **前端测试**：时间线事件渲染、@补全、产物线状态流转、定稿按钮禁用态（对齐现有组件测试模式）。
- 遵循项目 Rust 构建规范（标准 cargo test/clippy 命令，禁止 `-j 1`）。

## 9. 明确不做（第一版 YAGNI）

- 不做 agent 主动发起话题（stall nudge 类机制暂缓；所有循环由人类消息或草稿事件触发）。
- 不做跨聊天室共享记忆 / persona 持久化。
- 不做 Work Item 产物线（枚举预留位）。
- 不做 Cumora 式常驻服务 / 多实例同步（那是分布式实时平台的问题域：agent 常驻在线等待唤醒、多服务器实例经 Redis 同步状态；Aria 是单机单进程本地工具，agent 为按需拉起的 CLI 子进程，这些问题不存在。仅取 Cumora 的仲裁语义与群聊交互形态。将来若做「人类离线时 agent 继续讨论」才需要引入）。

## 10. 风险与待定项

| 风险 | 缓解 |
|---|---|
| token 消耗显著高于流水线模式（多角色 × 多轮讨论） | HARD_LOOP_CAP、triage 只路由 0~2 角色、小模型 triage |
| triage 小模型路由质量不佳 | 第一版提供规则兜底路径（按角色职责关键词），triage 失败退化为规则路由 |
| agent 间讨论发散不收敛 | 熔断机制 + 人类显式定稿作为唯一收敛点 |
| 双模式同步演进的维护成本 | 引擎目录级隔离；共享层（ProviderAdapter/LifecycleStore）改动需两侧测试 |
