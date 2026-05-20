# Workspace 产品工作台优化方案（P0 + P1）

## 文档信息

- 文档类型：技术方案
- 分支：`product-workbench-issue-lifecycle`
- 工作区：`.worktrees/product-workbench-issue-lifecycle`
- 制定日期：2026-05-20
- 版本：v1.0
- 依据：
  - `cadence/analysis-docs/2026-05-20_分析报告_ProductWorkbench交互审计与参考项目调研_v1.0.md`
  - `cadence/analysis-docs/2026-05-20_分析报告_Workspace交互设计问题与参考项目分析_v1.0.md`

---

## 一、方案总览

### 1.1 范围与不在范围

**本轮纳入（P0 + P1）**：

| 问题大类 | 报告条目 |
|---|---|
| 输入语义未解耦（user_message 同时承担补充上下文 + 触发生成） | 报告1 P0-1, 报告2 P0-1 |
| Timeline 节点详情未持久化（刷新后丢失 streaming/execution/permission/verdict） | 报告1 P0-2, 报告2 P0-2/P0-3 |
| 看板 → Workspace 导航模型混乱（Story confirmed 后无法触发 Design） | 报告2 P0-5 |
| Provider 授权 `permission_response` 链路问题 | 报告2 P0-4 |
| 刷新/断开运行被中止、策略未产品化 | 报告1 P1-断开, 报告2 P0-2 |
| 右侧 Artifact/执行 Tab 在 Timeline 存在时不可用 | 报告1 P1-Tab |
| Provider 配置可见性不够（藏在折叠面板） | 报告1 P1-Provider, 报告2 §4.4 |
| WebSocket 无自动重连 | 报告2 P1-6 |

**本轮不纳入（留下轮）**：

- Work Item 代码执行闭环（worktree attempt / coding / testing / rework / final）
- 多版本语义级 diff / 高级版本对比视图（本轮做行级 diff 摘要）
- 四列看板焦点高亮的完整改造（本轮仅做 Drawer 副产品高亮）
- 删除操作二次确认弹窗（P2）
- Project 级 Provider 默认配置前端入口
- 性能/压力测试

### 1.2 已锁定的产品决策

| # | 决策点 | 选择 | 原因 |
|---|---|---|---|
| 1 | 本轮范围 | P0 + P1 | 主链路 + 控制/可见性闭环 |
| 2 | 断开策略 | 断开即中止 + UI 拦截 + Timeline 留痕 | 当前阶段 Aria 人在环中定位；没有 attempt/worktree 隔离时后台继续是危险的 |
| 3 | 看板导航 | 侧滑详情面板 + Workspace 全屏 | 看板上下文不丢，"查看 vs 生成"心智解耦 |
| 4 | 协议拆分 | 新增 `context_note` 与 `start_generation` 两类消息 | 从协议层根治输入语义未解耦 |
| 5 | Reviewer 默认 | PrepareContext 默认勾选 + 取消时提示 | 报告2 §零确认的"用户指定 + 系统推荐" |

### 1.3 方案章节骨架

按用户路径闭环组织：

| 章节 | 闭环目标 |
|---|---|
| §2 协议层重塑 | 入站消息从"对话+猜意图"转为"意图明确的动作指令" |
| §3 Timeline 审计与会话恢复 | 让 Timeline 真正成为审计事实源；刷新后能看到完整证据 |
| §4 PrepareContext 阶段 UI | 对话补充 / 启动执行 / Provider snapshot / Reviewer 推荐就位 |
| §5 看板信息架构（侧滑详情面板） | 看板 ↔ Workspace 心智解耦；Story→Design 流转打通 |
| §6 Running / Review / Confirm 阶段 UI | Tab/Provider/Artifact 切换真实生效；硬 Gate 决策上下文落地 |
| §7 断开策略产品化 | beforeunload 拦截 + Timeline 留痕 + 恢复后明示 |
| §8 WebSocket 重连 | 网络抖动后能拿到完整 snapshot |
| §9 Permission 链路修复 | 真实 Claude Code provider 不再卡死 |
| §10 测试策略 | E2E 覆盖刷新恢复、断开中止、协议拆分、看板侧滑 |

### 1.4 主要变更面预估

- **协议**：`web/src/hooks/useWorkspaceWs.ts`、`src/web/workspace_ws_handler.rs` 入站消息类型扩展
- **持久化**：`src/product/lifecycle_store.rs` 新增 `timeline_node_details/<node_id>.json` 按节点分文件
- **后端 state**：`src/product/workspace_engine.rs::SessionState` 返回字段扩展
- **前端 store**：`web/src/state/workspace-ws-store.ts` snapshot 应用、`web/src/state/lifecycle-workbench-store.ts` Drawer 状态
- **前端组件**：新增 `LifecycleCardDrawer`（侧滑详情）、重构 `WorkspacePage` 阶段化 UI、新增 `useUnloadGuard` 拦截

---

## 二、§1 协议层重塑

### 2.1 入站消息（client → server）

| 新消息 | 仅在阶段 | 后端行为 | payload |
|---|---|---|---|
| `context_note` | PrepareContext | 追加到 session 上下文池 + 写入 Timeline `context_note` 节点；**不**启动 Provider | `{ content: string }` |
| `start_generation` | PrepareContext | 锁定 Provider 配置快照 → 写入 Timeline `start_generation` 节点 → 启动 author run | `{ provider_config: ProviderSnapshot, reviewer_enabled: bool }` |

既有动作型消息（`abort` / `permission_response` / `human_confirm` / `request_revision` / `select_revision_path`）保留语义不变。

`user_message` 不再作为运行入口，过渡期内后端收到时按 `context_note` 语义处理并打 warning log；前端不再发送。

### 2.2 阶段-消息合法性矩阵

| 阶段 | 合法入站消息 |
|---|---|
| PrepareContext | `context_note`, `start_generation`, `abort` |
| Running | `abort`, `permission_response` |
| CrossReview | `abort` |
| ReviewDecision | `select_revision_path`, `request_revision` |
| Revision | `abort` |
| HumanConfirm | `human_confirm` |
| Completed | — |

非法消息后端不沉默丢弃，回送：

```json
{ "type": "protocol_error", "code": "INVALID_MESSAGE_FOR_STAGE", "stage": "Running", "received": "context_note" }
```

前端按 `protocol_error` 显式提示。

### 2.3 出站消息（server → client）新增

| 新出站 | 用途 |
|---|---|
| `protocol_error` | 阶段校验失败、字段缺失、permission id 不匹配等 |
| `provider_locked` | 收到 `start_generation` 后回送 `{ snapshot, locked_at }` |
| `aborted_by_disconnect` | §7 断开策略产生的 Timeline 事件，重连后通过 snapshot 重放 |

### 2.4 代码触达点

- 协议定义：`src/web/workspace_ws_handler.rs::InboundMessage` enum + serde tag
- 路由分发：`workspace_ws_handler.rs::handle_message` 按阶段校验
- 前端发送：`web/src/hooks/useWorkspaceWs.ts` 新增 `sendContextNote` / `sendStartGeneration`，废弃 `sendMessage`
- 前端 store：`web/src/state/workspace-ws-store.ts` 处理 `protocol_error` / `provider_locked` / `aborted_by_disconnect`

### 2.5 迁移与兼容

- 协议变更不涉及 DB 迁移
- 旧 session 的历史 `messages` 字段作为历史展示，不需要回填
- 既有 E2E fixture 中"发送开始消息触发生成"改为发送 `start_generation`

---

## 三、§2 Timeline 审计与会话恢复

### 3.1 Timeline 节点类型枚举

| 节点类型 | 触发 | 关键字段 |
|---|---|---|
| `context_note` | client 入站 | content, author (user), ts |
| `start_generation` | client 入站 | provider_snapshot, reviewer_enabled, ts |
| `author_run` | 后端启动 author 或 revision | provider, agent_role=author, status, streaming_content, execution_events, permission_events, artifact_ref, is_revision, base_artifact_ref, ts |
| `reviewer_run` | 后端启动 reviewer | provider, agent_role=reviewer, status, streaming_content, verdict, ts |
| `human_confirm` | client 入站 | decision (confirm/request-change/terminate), context_summary_at_decision, ts |
| `aborted_by_disconnect` | §7 触发 | reason, last_active_run_id, ts |
| `protocol_error` | §1 触发 | code, received, stage, ts |

> Revision 复用 `author_run` 类型 + `is_revision: true` 旗标，避免节点类型膨胀。
>
> Artifact version 不是 Timeline 节点。每个 `author_run` 完成时必须 `artifact_ref` 指向一个 artifact version；`human_confirm` 必须绑定确认对象 artifact version。

### 3.2 节点详情数据结构

```json
{
  "node_id": "uuid",
  "session_id": "uuid",
  "type": "author_run",
  "status": "completed",
  "agent_role": "author",
  "provider": { "name": "claude-code", "model": "..." },
  "messages": [],
  "streaming_content": "",
  "execution_events": [],
  "permission_events": [
    { "request_id": "...", "request": {}, "response": {}, "ts": "..." }
  ],
  "verdict": null,
  "artifact_ref": { "artifact_id": "...", "version": 2 },
  "is_revision": false,
  "base_artifact_ref": null,
  "started_at": "...",
  "ended_at": "..."
}
```

### 3.3 持久化形式：按节点分文件

新增目录 `timeline_node_details/<node_id>.json`，节点级文件天然并发安全；snapshot 恢复时按 `timeline_nodes.json` 列出节点 id → 并行读节点详情。

`src/product/lifecycle_store.rs` 当前的单 json 文件模式扩展到节点级文件 IO。

### 3.4 写入时机

| 事件 | 写入策略 |
|---|---|
| stream chunk | 节流 200ms / 累计 4KB 触发；节点结束时立即 flush |
| execution event | 立即写入 |
| permission request/response | 立即写入（审计敏感） |
| reviewer verdict | 立即写入 |
| artifact_ref 绑定 | 立即写入 |

### 3.5 SessionState snapshot 协议扩展

`src/product/workspace_engine.rs::SessionState` 当前返回 `messages, checkpoints, artifact, providers, timeline_nodes, artifact_versions`，扩展为：

```
SessionState {
  ... 现有字段,
  timeline_node_details: Map<node_id, NodeDetail>,
  active_run_id: Option<String>
}
```

snapshot 下发时机：

- 客户端首次连接 → 全量 snapshot
- 重连成功 → 全量 snapshot（含 `aborted_by_disconnect` 节点）
- 阶段切换 → 增量 `stage_change` 事件

### 3.6 前端 store 改造

`web/src/state/workspace-ws-store.ts:211` 当前 `detailsForTimelineNodes` 创建空 detail。改为：snapshot 应用时直接把 `timeline_node_details` 灌入 `nodeDetails`。

新增 selector `selectNodeDetail(nodeId)`，`TimelineDetailPanel` 直接渲染，不再依赖运行时 stream 状态拼装。

### 3.7 测试覆盖（详见 §10）

1. 流式中刷新 → 重连 snapshot 含 streaming 累积部分
2. permission_request 未应答时刷新 → snapshot 含 permission_events pending
3. reviewer verdict 完成后刷新 → snapshot 完整含 verdict
4. 多版本 revision 后刷新 → 两个 author_run 节点都能看到完整 streaming + artifact_ref

---

## 四、§3 PrepareContext 阶段 UI

### 4.1 整体布局

```
Workspace Header（始终可见）
├─ 实体面包屑（Issue / Story Spec / Design Spec / Work Item）
├─ Provider Snapshot（author + reviewer + rounds + 旗标，未锁定/已锁定）
└─ Stage Indicator（PrepareContext / Running / ...）

主区
├─ 左：Timeline（含 context_note 节点）
└─ 右：阶段面板（按 stage 切换）
   PrepareContext 模式：
     ├─ Provider 配置区（默认展开常驻；高级配置可折叠）
     ├─ 补充上下文输入区（textarea + 已补充列表）
     └─ 主 CTA：🚀 开始生成
```

### 4.2 关键交互规则

**输入区**：

- textarea **回车换行**，不回车提交
- 按钮 [发送上下文] 提交 → `context_note` → Timeline 追加节点 → 清空输入
- "已补充上下文 N 条"折叠列表：**N ≤ 3 默认展开，> 3 折叠**

**Provider 配置区**：

- 默认展开，不再藏在折叠面板（解决报告1 P1-Provider）
- 高级配置（Superpowers / OpenSpec）独立子折叠
- "启用交叉审核" 默认勾选
- 取消勾选时下方出现提示条：`⚠️ 未启用交叉审核可能降低 artifact 质量`
- 仅配置一个 Provider 时，自动填入 author 同 Provider 并标注"建议在 Project 设置中添加其他 Provider"

**开始生成按钮**：

- 唯一启动入口。点击后：
  1. 前端发 `start_generation`（带 Provider snapshot）
  2. 收到 `provider_locked` 后切换为"已锁定"视图
  3. Stage 切换为 Running，UI 进入 §6 Running 形态
- 校验：必填 author Provider；勾选交叉审核必填 reviewer
- 失败：`protocol_error` 时按钮恢复可点 + 红色提示

### 4.3 Timeline 视图增强（PrepareContext 期间）

`context_note` 节点紧凑展示：

```
📝 上下文补充  user · 14:32
"需要支持空查询参数的兜底"
```

`start_generation` 节点展示为分隔线：

```
─── 🚀 开始生成（Provider: Claude Code → Codex, 1 轮）─── 14:35
```

### 4.4 状态机视角

| UI 元素 | PrepareContext | Running | CrossReview | HumanConfirm |
|---|---|---|---|---|
| 补充上下文输入区 | ✅ 可用 | ❌ 隐藏 | ❌ 隐藏 | ❌ 隐藏 |
| Provider 配置编辑 | ✅ 可编辑 | 🔒 锁定快照 | 🔒 锁定快照 | 🔒 锁定快照 |
| 开始生成按钮 | ✅ 显示 | ❌ 隐藏 | ❌ 隐藏 | ❌ 隐藏 |
| Header 状态徽章 | "准备中" | "运行中 · 保持本页打开" | "审核中" | "等待确认" |

### 4.5 代码触达点

- 新建 `web/src/components/workspace/PrepareContextPanel.tsx`
- 拆现有 `WorkspacePage.tsx` 输入逻辑（移除"统一输入框 → user_message"路径）
- `web/src/components/workspace/ProviderConfigPanel.tsx` 从折叠面板提升为常驻
- 新建 `web/src/hooks/useStageUI.ts`，按 stage 返回应显示的子面板

---

## 五、§4 看板信息架构：侧滑详情面板

### 5.1 卡片点击的新行为

**之前**：点击卡片 → 直接进全屏 Workspace（Story confirmed 后无路触发 Design）。

**之后**：点击卡片 → 右侧滑出 `LifecycleCardDrawer`，看板上下文不丢失。Workspace 仍作为全屏路由 `/workspace/<session_id>`。

### 5.2 Drawer 内容分区

| 分区 | 内容 |
|---|---|
| 头部 | 实体类型 + ID + 标题 + 状态徽章 |
| 关联关系 | 上游实体（链接，可点切换 Drawer）+ 关联 Workspace Session |
| 版本历史 | artifact_versions 倒序：verdict + ts + author/reviewer（diff 视图本轮仅文本行级，留下轮做高级 diff） |
| Artifact 预览 | 最新版本 markdown 渲染（折叠时只显示前 200 字） |
| 操作区 | 主 CTA（见 5.3） |

### 5.3 操作按钮矩阵

| 实体 \ 状态 | 未生成 | 生成中 | Confirmed | 已驳回 |
|---|---|---|---|---|
| Issue | 创建 Story Spec | — | 创建 Story Spec | — |
| Story Spec | 打开 Workspace | 打开 Workspace（实时） | 打开 Workspace + 🚀 生成 Design Spec | 打开 Workspace（继续返修） |
| Design Spec | 打开 Workspace | 打开 Workspace（实时） | 打开 Workspace + 🚀 生成 Work Item | 打开 Workspace（继续返修） |
| Work Item | 打开 Workspace | 打开 Workspace | 打开 Workspace（Plan 确认） | 打开 Workspace |

> Work Item 的"代码执行闭环"按钮（worktree attempt / coding / final）本轮不出现。

### 5.4 "生成下一阶段"按钮行为

点击 `🚀 生成 Design Spec`：

1. 后端创建 Design Spec 实体（关联到当前 Story Spec），同步创建 Workspace session（stage=PrepareContext）
2. Drawer 自动切到新建的 Design Spec 实体
3. 弹出二级 CTA："打开 Workspace 配置 Provider 并开始生成"
4. **不自动启动生成**（启动必须由用户在 Workspace 内显式 `start_generation`）
5. **不二次确认**（生成只是创建空 session，没启动 provider，误点零成本）

### 5.5 Drawer 与全屏 Workspace 的关系

- Drawer：总览 + 路由入口（版本、状态、操作）
- Workspace：执行视图（Timeline、流式输出、Provider 操作）
- 共享 store；Drawer 关闭后 Workspace 状态不丢

### 5.6 Drawer 的 URL / 路由

- 状态用 query param 表达：`/workbench?focus=story-12`
- 直接访问该 URL 自动打开 Drawer
- Workspace 独立路由保持：`/workspace/<session_id>`
- Drawer 宽度：**固定 480px**
- 打开时看板**不灰化**（允许并行操作）

### 5.7 焦点过滤轻量改进

Drawer 打开时，对应实体卡片在看板上加边框高亮 + 滚动到视图。完整的"四列看板焦点关系"改造留下轮。

### 5.8 代码触达点

- 新建 `web/src/components/lifecycle/LifecycleCardDrawer.tsx`
- 改 `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`：卡片 onClick 改为 `openDrawer(entityId)`
- 改 `web/src/state/lifecycle-workbench-store.ts:127`：新增 `focusedEntityId` 与 `isDrawerOpen` 状态，与 query param 双向同步
- 改 `web/src/components/lifecycle/LifecycleCard.tsx`：去掉卡片内"打开 Workspace"按钮（统一搬到 Drawer 内）
- 修复 `handleLaunchWorkspace` race：先 await refresh 再 navigate

---

## 六、§5 Running / Review / Confirm 阶段 UI

### 6.1 节点详情面板（取代页面级 Tab）

把页面级 Artifact/执行 Tab 删除，下沉到节点详情面板：

```
节点详情面板（跟随 Timeline 选中节点）
├─ Node 元信息（type + provider + status + 时间）
└─ 5 个 Tab:
   [概览] [流式输出] [执行事件] [权限] [Artifact]
```

| Tab | 内容 | 数据源 |
|---|---|---|
| 概览 | 节点元信息 + 关联 artifact_ref + 时间线 | NodeDetail 头部 |
| 流式输出 | streaming_content（运行中实时；完成后静态） | `streaming_content` |
| 执行事件 | 工具调用 / 命令 / 文件操作列表 | `execution_events` |
| 权限 | permission_events 配对展示，未应答高亮 | `permission_events` |
| Artifact | artifact_ref 对应版本预览 | `artifact_versions[ref]` |

页面级 `activeRightTab` 状态及 tab 按钮删除（功能下沉到节点级）。

### 6.2 Header 区 Provider Snapshot 强化

`WorkspacePage.tsx:439` 当前右侧小字。改造为 Header 永久主信息区：

```
Story Spec #SP-12 / v2 (running)
─────────────
Author: Claude Code (claude-opus-4-7)  🔒 locked
Reviewer: Codex (gpt-5) · 1 round       🔒 locked
Superpowers: off  OpenSpec: on

Stage: ▶ Running   Status: 🔴 运行中 · 保持本页打开
```

锁定时间戳从 `provider_locked` 出站消息取，Tooltip 显示"锁定于 14:35"。

### 6.3 各阶段 UI 要点

**Running**：左 Timeline 持续追加；右节点详情默认聚焦最新 active node + "流式输出" tab；底部仅 `[⏹ 中止]`；Header 徽章 `🔴 运行中 · 保持本页打开`（触发 §7 beforeunload）。

**CrossReview**：Timeline 追加 reviewer_run 节点；右默认聚焦 reviewer_run + "概览" tab；Header 徽章 `👀 审核中`。

**ReviewDecision**（reviewer verdict = revise）：

```
审核结论：建议返修
Reviewer: Codex   Verdict: revise
Summary: "缺少边界场景说明，第 3 节流程图与正文不符"

请选择处理路径：
○ 直接返修（author 基于审核意见自动修订）
○ 补充上下文后返修（追加 context 再让 author 修订）
○ 跳过审核结论，进入人工确认

[ 确定 ] [ ⏹ 中止 ]
```

选 ② 时弹出补充上下文输入区，仅这一轮 context 进入 revision_run 的输入。

**Revision**：Timeline 追加 `author_run` 节点（`is_revision: true`），节点旁标 "🔁 v2 修订自 v1"；Header 徽章 `🔴 修订中`。

**HumanConfirm**（决策支持）：

```
待人工确认：Story Spec v2

📊 审核摘要
   Reviewer: Codex · Verdict: pass
   关键意见：
   1. ✓ 边界场景已补齐
   2. ✓ 流程图与正文一致

📄 与上一版本对比
   [v1 → v2] 新增 12 行 · 删除 4 行
   [展开 diff]   ← 本轮纯文本行级 diff，高级 diff 留下轮

📝 Artifact 预览（v2）
   [markdown preview]

────── 决策 ──────
[ ✓ 确认 ]   [ ✎ 要求修改 ]   [ ✗ 终止 ]
```

"要求修改" → 结构化反馈表单：

```
反馈类型（可多选）：
[ ] 内容缺失  [ ] 表述不清  [ ] 与需求不符  [ ] 其他
具体描述：[textarea]
目标版本：自动填入当前 artifact 版本
[ 提交 ]
```

提交后回 ReviewDecision（选"补充上下文后返修"），结构化反馈作为 context_note 注入 Timeline。

"终止" 二次确认（影响：session 关闭）。"确认" 进入 Completed。

### 6.4 底部操作区按统一矩阵

| Stage | 主要操作 | 次要操作 |
|---|---|---|
| PrepareContext | 🚀 开始生成 | — |
| Running | — | ⏹ 中止 |
| CrossReview | — | ⏹ 中止 |
| ReviewDecision | 确定路径 | ⏹ 中止 |
| Revision | — | ⏹ 中止 |
| HumanConfirm | ✓ 确认 / ✎ 要求修改 / ✗ 终止 | — |
| Completed | — | 返回看板 |

### 6.5 代码触达点

- 重构 `web/src/pages/WorkspacePage.tsx`：拆出 `WorkspaceHeader.tsx`（Provider snapshot + Stage 徽章）、`StageActionsBar.tsx`（底部按钮）、`NodeDetailPanel.tsx`（5 tab）
- 删除页面级 `activeRightTab` 状态及 tab 按钮
- 新建 `web/src/components/workspace/stages/`：`RunningStagePanel.tsx`, `CrossReviewStagePanel.tsx`, `ReviewDecisionStagePanel.tsx`, `HumanConfirmStagePanel.tsx`（含结构化反馈表单）
- `useStageUI` hook 按 stage 渲染对应面板

---

## 七、§6 断开策略产品化

### 7.1 端到端事件流

```
用户运行中刷新 / 关 Tab
   │
   ├─ 前端：window.beforeunload 触发
   │      "运行中。刷新/关闭将中止当前 Provider 运行，是否继续？"
   │
   ├─ 用户坚持离开 → WebSocket 断开
   │
   ├─ 后端：socket close handler（src/web/workspace_ws_handler.rs:465）
   │      1. 取出 ActiveRun → abort（已存在）
   │      2. 写入 Timeline 节点：aborted_by_disconnect  ← 新增
   │      3. session.stage 切回 PrepareContext, status=open（已存在）
   │
   └─ 用户重连 → snapshot 含 aborted_by_disconnect 节点
          UI 顶部 banner：
             ⚠ 上次运行因断开被中止（14:32）
             [我知道了]  [查看 Timeline]
```

### 7.2 beforeunload 拦截

新建 `web/src/hooks/useUnloadGuard.ts`：

```typescript
useUnloadGuard({
  enabled: stage === 'Running' || stage === 'CrossReview' || stage === 'Revision',
  message: '运行中。刷新/关闭将中止当前 Provider 运行，是否继续？'
})
```

- 仅在 Running / CrossReview / Revision 启用
- 程序化导航（React Router）用 `useBlocker` 弹自定义 Modal
- 浏览器原生提示与 Modal 共用同一份 message 常量

### 7.3 后端 aborted_by_disconnect 节点写入

改造 `src/web/workspace_ws_handler.rs:465`：

```rust
on_socket_close(session_id) {
    if let Some(active_run) = take_active_run(session_id) {
        active_run.abort().await;

        timeline.append_node(TimelineNode {
            node_type: AbortedByDisconnect,
            session_id,
            payload: {
                reason: "socket_disconnect",
                last_active_run_id: active_run.id,
            },
            ts: now(),
        }).await;

        engine.transition_to_prepare_context(session_id).await;
    }
}
```

立即持久化（§3.4 立即写入策略）。

### 7.4 重连后明示

首次连接 / 重连时收到 snapshot，检查最后一个节点：

```typescript
const lastNode = snapshot.timeline_nodes.at(-1);
if (lastNode?.type === 'aborted_by_disconnect' && !hasAcknowledged(lastNode.id)) {
  showBanner({
    level: 'warn',
    text: `上次运行因断开被中止（${formatTime(lastNode.ts)}）`,
    action: { label: '查看 Timeline', onClick: () => scrollToNode(lastNode.id) },
    dismissible: true,
  });
  markAcknowledged(lastNode.id);  // localStorage 不同步到后端
}
```

### 7.5 主动中止 vs 断开中止

用户点 `⏹ 中止` → 发 `abort` → Timeline 追加 `author_run.status=aborted`。**不**生成 `aborted_by_disconnect`，便于区分主动与被动。

### 7.6 边界情况

| 场景 | 处理 |
|---|---|
| socket close 时没有 active run | 不写 Timeline 节点 |
| permission_request 未应答时断开 | 视为运行中 → 写入 aborted_by_disconnect → permission_event 保持 unanswered |
| HumanConfirm 阶段断开 | 不拦截 / 不写节点 → 重连后正常恢复 |
| 程序化导航运行中 | useBlocker 弹自定义 Modal |

### 7.7 代码触达点

- 新建 `web/src/hooks/useUnloadGuard.ts`
- 新建 `web/src/components/workspace/DisconnectBanner.tsx`
- 改 `src/web/workspace_ws_handler.rs:465`：socket close 写 `aborted_by_disconnect` 节点
- 改 `src/product/workspace_engine.rs`：暴露 `append_aborted_by_disconnect_node(session_id, active_run_id)` API
- 改 `web/src/state/workspace-ws-store.ts`：snapshot 应用时识别 banner 触发

---

## 八、§7 WebSocket 重连

### 8.1 与 §6 断开策略的边界

- 短暂网络抖动 TCP 未断 → §6 / §7 都不触发
- 网络断开 socket close → §6 触发中止 + 留痕 → §7 自动重连 → 拉 snapshot → §6 banner 弹出
- 价值：让"刷新"变自动，非运行阶段网络抖动无感

### 8.2 重连策略

新建 `web/src/hooks/useWorkspaceWsReconnect.ts`：

| 参数 | 值 |
|---|---|
| 退避序列 | 1s → 2s → 4s → 8s → 16s（上限） |
| 抖动 | ±20% jitter |
| 触发条件 | close code ≠ 1000（非用户主动） |
| 暂停 | document.hidden 时暂停，visibilitychange 恢复时立即触发一次 |
| 重试次数 | 无限（计数用于 UI 提示） |

### 8.3 UI 反馈

- 首次重连尝试不显示 banner（避免一闪）
- 失败 > 1 次显示重连进度 banner（含"手动重连"按钮）
- 重连成功 → banner 消失 + snapshot 应用

### 8.4 snapshot 全量替换

1. 客户端发 `hello` 入站消息（带 session_id + last_seen_node_id）
2. 服务端回送完整 SessionState snapshot（§3.5 已定）
3. 客户端 store **替换式**应用（不做增量 sync）

理由：节点详情量级单 session 数百，全量拉取成本低；增量 diff 引入 cursor 与一致性风险，本轮不必。

### 8.5 active_run_id 字段用途

重连后客户端用 `SessionState.active_run_id` 判断：

- 存在 → session 仍运行（§6 未触发，可能服务端重启）
- null → session 不在运行（正常状态 / §6 已中止）

客户端不主动恢复 active run；后端若仍运行，stream 自然推过来。

### 8.6 心跳与超时

| 配置 | 值 |
|---|---|
| 客户端 ping | 每 25s |
| 服务端 pong | 收到 ping 立即回 |
| 客户端超时 | 60s 无消息 → 主动 close 触发重连 |
| 服务端超时 | 90s 无客户端消息 → close 触发 §6 |

### 8.7 代码触达点

- 新建 `web/src/hooks/useWorkspaceWsReconnect.ts`
- 改 `web/src/hooks/useWorkspaceWs.ts`：发 `hello`、处理 `pong`、暴露 close code
- 改 `web/src/state/workspace-ws-store.ts`：snapshot 替换式应用
- 改 `src/web/workspace_ws_handler.rs`：处理 `hello`、ping/pong、超时 close
- 改 `src/product/workspace_engine.rs::SessionState`：补 `active_run_id` 字段

---

## 九、§8 Permission 链路修复

### 9.1 已确认的代码路径

```
前端 (WsInMessage::PermissionResponse)
   │  workspace_ws_handler.rs:341-360
   ▼
current_run.command_tx → ProviderCommand::PermissionResponse
   │  workspace_engine.rs:417-421（非 Abort 命令转发到 session.commands）
   ▼
session.commands → ProviderCommand::PermissionResponse
   │  approval_bridge.rs:152-159（listen_for_permission_commands）
   ▼
pending.remove(&id).send(decision) → 唤醒等待的 oneshot
   ▼
provider 收到 decision，继续 run
```

engine + bridge 层链路闭合。报告 2 P0-4 提到的"完全卡住"需在实施时定位真正断点。

### 9.2 实施时排查清单（按怀疑度排序）

| # | 排查项 | 检查方法 |
|---|---|---|
| 1 | bridge listen 任务是否启动 | 在 provider adapter 创建 session 时打 log，确认 `listen_for_permission_commands` 被 spawn |
| 2 | session.commands 与 bridge command_rx 是否同一 channel | 检查 provider adapter 构造时如何接线 `commands` 字段 |
| 3 | permission id 一致性 | 前端收到 PermissionRequest 时记录 id，回 PermissionResponse 时核对；后端 `next_permission_id` 与 `pending.insert` 的 id |
| 4 | ActiveRun command_tx 生命周期 | `WsInMessage::PermissionResponse` 时确认 current_run 与 PermissionRequest 发出时同一 run |
| 5 | claude_code_provider 独立 control_request 路径 | `claude_code_provider.rs:79` `parse_control_request` 是否走 bridge |
| 6 | 前端 id 字段类型 | store 是否把 string id 转 number 再回 string 导致丢失精度 |

### 9.3 防御性修复（不论根因，都应加上）

#### 9.3.1 全链路 trace log

```rust
// 1. workspace_ws_handler.rs:341
tracing::info!(permission_id = %id, approved, "ws inbound permission response");

// 2. workspace_engine.rs:417
if let ProviderCommand::PermissionResponse { id, .. } = &command {
    tracing::info!(permission_id = %id, "engine forwarding permission response");
}

// 3. approval_bridge.rs:152
tracing::info!(permission_id = %id, "bridge received permission response");

// 4. approval_bridge.rs:157
if let Some(decision_tx) = pending.lock().await.remove(&id) {
    tracing::info!(permission_id = %id, "bridge dispatched decision to pending");
} else {
    tracing::warn!(permission_id = %id, "bridge: no pending entry for id");
}
```

#### 9.3.2 unmatched id 的 protocol_error 出站

bridge 收到 id 但 pending 表里没有，主动通过 `event_tx` 发 `EngineEvent::ProtocolError { code: "PERMISSION_ID_UNMATCHED" }`，前端展示。当前是沉默丢弃。

#### 9.3.3 permission_events 持久化到 NodeDetail（§3 已定）

每个 permission_request / permission_response 配对写入 `NodeDetail.permission_events`。即使 channel 链路有问题，从持久化数据也能看到"前端发了响应但后端没消化"。

#### 9.3.4 PendingPermissions 超时清理

`approval_bridge.rs` 当前 PendingPermissions 无超时。改为：

```rust
pending.insert(id, (decision_tx, Instant::now()));

// 后台任务每 60s 扫一次，超过 15min 未响应：
//   1. 取出 decision_tx，发 PermissionDecision { approved: false, reason: "timeout" }
//   2. 通过 event_tx 发 EngineEvent::PermissionTimeout
//   3. Timeline 写 permission_events 的 timeout 状态
```

15min 不等于"拒绝"，只是清理 pending 避免内存泄漏；审批者回来后看到 timeout 节点可以重新跑。

### 9.4 前端配套

- `useWorkspaceWs.sendPermissionResponse(id, approved, reason)`：发送时 console.info 记录 id
- store 收到 PermissionRequest 时 `pendingPermissions[id]`，收到 confirm/timeout 后移除
- 节点详情"权限" tab（§6.1）展示 permission_events 列表，标注 `pending` / `approved` / `denied` / `timeout`

### 9.5 代码触达点

- `src/web/workspace_ws_handler.rs:341` 加 trace log
- `src/product/workspace_engine.rs:417` 加 trace log
- `src/cross_cutting/approval_bridge.rs:146` listen 任务加 trace + unmatched warning
- `src/cross_cutting/approval_bridge.rs` PendingPermissions 加超时清理后台任务
- `src/cross_cutting/approval_bridge.rs:152` unmatched 时发 ProtocolError 出站
- `src/product/workspace_engine.rs` 新增 `EngineEvent::ProtocolError` 与 `PermissionTimeout` 事件
- `web/src/hooks/useWorkspaceWs.ts` permission_response 发送加 console.info
- `web/src/components/workspace/NodeDetailPanel.tsx` "权限" tab 展示 events

---

## 十、§9 测试策略

### 10.1 测试分层

| 层级 | 工具 | 范围 |
|---|---|---|
| 单元测试 | rust `#[test]` + vitest | 协议序列化、节流写入、退避序列、阶段-消息合法性矩阵、focused-entity reducer |
| 集成测试 | rust `#[tokio::test]` + 模拟 WebSocket | engine + workspace_ws_handler + lifecycle_store 全链路 |
| E2E | 既有 product workbench E2E 框架 | 真实 WS + fake provider，端到端用户场景 |

E2E 为合并门槛；单元/集成测试覆盖率沿用项目默认 80%，不强制更高阈值。

### 10.2 E2E 核心场景（按闭环组织）

**A. 输入语义解耦（§1 + §3）**：
- A1. PrepareContext 发 context_note → Timeline 追加节点 → 阶段仍是 PrepareContext
- A2. 连续 3 条 context_note → 3 节点 → Provider 未启动
- A3. 点 "开始生成" → Provider 锁定 → 阶段切 Running → Timeline 出现 start_generation 节点
- A4. Running 阶段尝试发 context_note → 后端 protocol_error → 前端展示

**B. Timeline 审计 + 会话恢复（§2）**：
- B1. 流式中刷新 → snapshot 含 streaming 累积部分 + aborted_by_disconnect
- B2. permission_request 未应答时刷新 → snapshot 含 permission_events pending
- B3. reviewer verdict 完成后刷新 → snapshot 完整 verdict
- B4. 多版本 revision 后刷新 → 两个 author_run 节点完整
- B5. 单 session 100+ 节点写入/读取性能（写入 < 50ms / 读取 < 200ms）

**C. 看板侧滑详情（§4）**：
- C1. 卡片点击 → Drawer 滑出 → URL 含 `?focus=` → Workspace 路由不变
- C2. 关闭 Drawer → URL focus 清除
- C3. Story confirmed → Drawer "生成 Design Spec" 激活 → 点击后新建 Design 实体且 Drawer 跳转
- C4. Drawer 内"打开 Workspace" → 进入全屏 Workspace
- C5. URL 直接访问 `/workbench?focus=story-12` → Drawer 自动打开 + 卡片高亮
- C6. handleLaunchWorkspace race fix：refresh 未完成不导航

**D. 阶段化 UI + 节点 tab（§5）**：
- D1. 节点详情 5 tab 切换（概览/流式/执行/权限/Artifact）
- D2. Header Provider snapshot：PrepareContext 可编辑、Running 锁定 + 锁图标 + locked_at tooltip
- D3. ReviewDecision 三路径都能选 → 进入正确下一阶段
- D4. HumanConfirm 决策面板含 reviewer 摘要 + 行级 diff + artifact 预览
- D5. HumanConfirm "要求修改" → 结构化反馈（多选）→ 提交后回 ReviewDecision + Timeline 含 context_note

**E. 断开策略（§6）**：
- E1. Running 时刷新 → beforeunload 拦截 → "离开" → 后端写 aborted_by_disconnect
- E2. 重连 banner 弹出 → "我知道了" → localStorage 标记 → 再刷新不再弹
- E3. PrepareContext 刷新 → 不拦截、不写节点
- E4. HumanConfirm 刷新 → 不拦截、不写节点
- E5. 主动 ⏹ 中止 → author_run.aborted，**无** aborted_by_disconnect

**F. 自动重连（§7）**：
- F1. mock socket close (1006) → 1s 后自动重连 → snapshot 应用
- F2. 重连失败 > 1 次 → 进度 banner + 退避递增
- F3. document.hidden 暂停 → visibilitychange 触发恢复 → 立即重试
- F4. 心跳：60s 无消息触发主动 close
- F5. 服务端 90s 无客户端消息触发 close → 走 §6 路径

**G. Permission 链路（§8）**：
- G1. 正常 approve → provider 收到 decision → run 继续
- G2. 正常 deny → provider 收到 deny → run 失败/中止
- G3. unmatched id → 后端 protocol_error → 前端展示
- G4. 15min 超时 → pending 清理 + Timeline 写 timeout
- G5. 全链路 trace log 完整性（permission_id 4 个点一致）

### 10.3 既有 E2E 用例适配

`cadence/plans/2026-05-19_计划文档_E2E测试方案_product-workbench-issue-lifecycle_v1.1.md` 升级到 v1.2，作为本方案配套：

| 既有用例 | 适配 |
|---|---|
| "发送开始消息触发生成" | 改为 `start_generation` |
| "重连恢复节点和 artifact version" | 扩展断言到 `timeline_node_details`、`active_run_id` |
| "卡片点击打开 Workspace" | 改为"卡片点击打开 Drawer" + 新增"Drawer 内打开 Workspace" |
| 既有 permission 用例 | 加 trace log 断言、unmatched id 用例 |

### 10.4 单元 / 集成测试关键覆盖

**单元**：
- 阶段-消息合法性矩阵参数化（7 阶段 × N 消息）
- 节流写入：200ms 内多次 chunk 触发一次写入
- 退避序列：(1,2,4,8,16) ± 20% + document.hidden 暂停
- PendingPermissions 超时清理：fake clock 推进 15min
- focused-entity store reducer + URL 双向同步
- useStageUI hook 按 stage 返回正确子面板

**集成**：
- 协议拆分：context_note 不启动 provider；start_generation 正确锁配置 + 启动 run
- 持久化 + 恢复：完整 author_run + reviewer_run + permission cycle → 重建 engine + 读持久化 → snapshot 完整
- 断开链路：socket close 中途 → lifecycle store 含 aborted_by_disconnect
- permission unmatched：未知 id PermissionResponse → 后端发 protocol_error

### 10.5 回归基线

```
cargo test --workspace
pnpm --filter web test
pnpm --filter web test:e2e
```

发现失败分两类处理：协议/UI 变更适配 vs 非预期破坏修复。

---

## 十一、风险与缓解

| 风险 | 影响 | 缓解 |
|---|---|---|
| 协议变更未覆盖某条历史调用路径 | 旧 client 发 user_message 卡死 | 后端保留 `user_message` 兼容期处理（按 context_note 语义 + warning log），过 1-2 个版本后再废弃 |
| 节点详情按文件分存导致 I/O 数量激增 | 性能下降 | 节流 200ms 写入 + 节点结束时 flush；监控数据 §10.2 B5 性能断言 |
| Drawer 与 Workspace 状态分离引入不一致 | 用户看到陈旧版本信息 | 共享 store；Drawer 打开时订阅相同 selectors |
| permission 修复根因定位时间超估 | P0 阻塞 | 防御性修复（§9.3）先上，保证即使根因未定位也有诊断手段 |
| beforeunload 浏览器兼容性 | 部分浏览器不显示自定义消息 | 接受浏览器默认文案；自定义 Modal 仍可用（程序化导航） |
| 全量 snapshot 在节点数超大时拖慢重连 | 重连体验差 | §10.2 B5 性能断言 100 节点 < 200ms；超大场景留下轮做增量 sync |

---

## 十二、验收标准

实现完成后必须满足（综合报告 1 验收标准 + 本方案章节）：

1. PrepareContext 发送 context_note 不会触发 Provider，Timeline 记录为上下文补充节点
2. 仅点击"开始生成"才进入 Running 并锁定 Provider 配置（含 reviewer/rounds/旗标）
3. 运行中刷新页面：明示"已因断开中止"+ Timeline 含 `aborted_by_disconnect` 节点
4. 刷新后 Timeline 每个节点能看到完整 streaming_content / execution_events / permission_events / review_verdict
5. Artifact version 展示 author / reviewer / 确认者 / 确认时间
6. 看板卡片点击打开 Drawer 而非全屏 Workspace；URL 含 `?focus=` 可分享
7. Story confirmed 后 Drawer 内"生成 Design Spec"按钮激活，点击后正确创建 Design 实体
8. 右侧节点详情 5 tab 切换真实生效；页面级 Artifact/执行 tab 已删除
9. Header 永久显示 Provider snapshot；运行中显示锁图标 + locked_at
10. HumanConfirm 阶段显示 reviewer 摘要 + 行级 diff + artifact 预览；"要求修改"走结构化反馈
11. WebSocket 断开后自动重连，snapshot 全量替换；首次抖动无 banner，>1 次失败显示进度
12. 全量既有 E2E 通过；新增 §10.2 中 A-G 7 闭环用例通过

---

## 十三、下一轮（不在本轮范围）

- Work Item 代码执行闭环：worktree attempt 模型、CodingWorkspaceStage 启用、coding/testing/review/rework/final 进入 Timeline
- 多版本语义级 diff / 高级版本对比
- 四列看板焦点关系完整改造（当前 Issue 高亮 + 下游列表头显示当前 Issue）
- 删除 Project/Repository/Issue 二次确认
- Project 级 Provider 默认配置前端入口
- 性能/压力测试：高频流式写入、500+ 节点 snapshot 恢复
- snapshot 增量 sync（当节点数超过单 session 阈值时）

---

## 附录 A：协议消息总览

入站（client → server）：

```
context_note         { content }
start_generation     { provider_config, reviewer_enabled }
abort                {}
permission_response  { id, approved, reason? }
human_confirm        { decision: confirm | request-change | terminate, payload? }
request_revision     { feedback }
select_revision_path { path: revise | revise-with-context | skip-to-human, extra_context? }
hello                { session_id, last_seen_node_id? }
ping                 {}
```

出站（server → client）：

```
session_state        { ...SessionState }
timeline_node_added  { node_id, type, ... }
stream_chunk         { node_id, content }
execution_event      { node_id, event }
permission_request   { id, tool_name, description, risk_level }
review_verdict       { node_id, verdict, summary }
stage_change         { new_stage }
provider_locked      { snapshot, locked_at }
aborted_by_disconnect (snapshot 内节点形式)
permission_timeout   { permission_id, node_id }
protocol_error       { code, message, context? }
pong                 {}
```

## 附录 B：相关代码触达点汇总

后端：

- `src/web/workspace_ws_handler.rs`：协议入站类型、阶段校验、socket close 写 aborted_by_disconnect、ping/pong/hello
- `src/product/workspace_engine.rs`：SessionState 扩展（timeline_node_details, active_run_id）、ProtocolError / PermissionTimeout 事件、append_aborted_by_disconnect_node API
- `src/product/lifecycle_store.rs`：节点详情按文件持久化（`timeline_node_details/<node_id>.json`）、节流 200ms 写入
- `src/cross_cutting/approval_bridge.rs`：trace log、unmatched id 报错、PendingPermissions 超时清理

前端：

- `web/src/hooks/useWorkspaceWs.ts`：sendContextNote / sendStartGeneration、permission_response trace
- `web/src/hooks/useWorkspaceWsReconnect.ts`：退避重连 + jitter + hidden 暂停（新）
- `web/src/hooks/useUnloadGuard.ts`：beforeunload + useBlocker（新）
- `web/src/hooks/useStageUI.ts`：按 stage 切换子面板（新）
- `web/src/state/workspace-ws-store.ts`：snapshot 替换式应用、protocol_error / provider_locked / aborted_by_disconnect 处理
- `web/src/state/lifecycle-workbench-store.ts`：focusedEntityId / isDrawerOpen + URL 同步
- `web/src/components/lifecycle/LifecycleCardDrawer.tsx`（新）
- `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`：卡片 onClick 改为 openDrawer
- `web/src/components/lifecycle/LifecycleCard.tsx`：去掉卡片内"打开 Workspace"按钮
- `web/src/pages/WorkspacePage.tsx`：重构为 Header + Timeline + 阶段面板 + StageActionsBar
- `web/src/components/workspace/PrepareContextPanel.tsx`（新）
- `web/src/components/workspace/ProviderConfigPanel.tsx`：从折叠提升为常驻
- `web/src/components/workspace/NodeDetailPanel.tsx`：5 tab（新）
- `web/src/components/workspace/stages/`：RunningStagePanel / CrossReviewStagePanel / ReviewDecisionStagePanel / HumanConfirmStagePanel（新）
- `web/src/components/workspace/DisconnectBanner.tsx`（新）
- `web/src/components/workspace/WorkspaceHeader.tsx`（新）
- `web/src/components/workspace/StageActionsBar.tsx`（新）

E2E：

- `cadence/plans/2026-05-19_计划文档_E2E测试方案_product-workbench-issue-lifecycle_v1.1.md` 升级到 v1.2，覆盖 §10.2 A-G 7 闭环
