# Workspace 交互设计问题与参考项目分析

> **文档类型**：分析报告
> **日期**：2026-05-20
> **分支**：product-workbench-issue-lifecycle
> **版本**：v1.1

---

## 零、设计意图确认（产品决策）

以下为产品负责人确认的核心交互意图，作为后续分析的基准：

| # | 问题 | 确认的设计意图 |
|---|------|---------------|
| 1 | PrepareContext 阶段的输入框行为 | **可以自由对话补充上下文，然后手动点击"开始"按钮触发执行**。输入框在此阶段是对话用途，不应自动触发生成 |
| 2 | 看板卡片点击的预期行为 | **打开详情/历史**。不是直接触发生成下一阶段 |
| 3 | 交叉审核 Reviewer 的选择方式 | **用户指定，但系统推荐需要交叉审核**。即 Reviewer 不是自动分配的，用户有最终决定权，但系统应主动建议启用交叉审核 |

> 这三个决策直接影响了下文对"当前实现偏差"的判断。

---

## 一、当前交互流程概述

### 1.1 核心架构

```
首页看板 (IssueLifecycleWorkbench)
  ├── 四列看板：Issue → Story Spec → Design Spec → Work Item
  ├── 点击卡片 → 选中 + 聚焦过滤
  └── 操作按钮 → 打开/创建 Workspace

全屏 Workspace (WorkspacePage)
  ├── 左侧：Timeline 节点列表 / 消息列表
  ├── 右侧：Detail Panel（流式输出/审核结论/确认操作）
  ├── 底部：阶段进度条 + 动态操作按钮
  └── WebSocket 双向通信
```

### 1.2 Issue 生命周期流转

```
Issue 创建（绑定 Repository）
  → Story Spec 生成（Provider 执行）
    → 交叉审核（Reviewer Provider）
      → 人工确认（硬 Gate）
        → Design Spec 生成
          → 交叉审核 → 人工确认
            → Work Item 生成
              → Plan 确认 → Coding → Testing → Review → Final
```

### 1.3 Workspace 阶段状态机

```
PrepareContext → Running → CrossReview → [verdict]
                                           ├─ pass → HumanConfirm → Completed
                                           ├─ needs_human → HumanConfirm → Completed
                                           └─ revise → ReviewDecision → [用户选择]
                                                          ├─ 直接返修 → Revision → CrossReview
                                                          ├─ 补充后返修 → Revision → CrossReview
                                                          └─ 人工介入 → HumanConfirm → Completed
```

---

## 二、交互设计问题分析

### 2.1 严重问题（阻塞主链路）

> **与设计意图的偏差**：以下问题均与"零、设计意图确认"中的决策直接冲突。

| # | 问题 | 影响 | 根因 | 偏离的设计意图 |
|---|------|------|------|---------------|
| 1 | **PrepareContext 输入框语义未解耦** | 用户发送补充约束会直接触发生成，无法在准备阶段自由对话 | `startGeneration` 实际只是发送 "开始生成" 文本消息，后端将任何 `user_message` 都当作触发信号 | 违反意图 #1：应支持自由对话 + 手动触发 |
| 2 | **刷新页面后无法终止运行中的 Provider** | 用户刷新后失去对 active run 的控制 | `ActiveRun` 控制句柄绑定在单个 WebSocket 连接的内存中，无持久化 | — |
| 3 | **刷新后流式输出丢失** | 用户看不到之前的生成内容 | `stream_chunk` 未持久化到 lifecycle store | — |
| 4 | **Provider 授权确认不生效** | 真实 Provider（Claude Code）流程完全卡住 | `permission_response` 未正确传递到 provider run | — |
| 5 | **Story → Design 流转 UI 阻塞** | 确认后的 Story Spec 无法继续生成 Design Spec | 卡片点击直接打开 Workspace，无法触达"生成 Design"按钮 | 违反意图 #2：卡片点击应打开详情，"生成下一阶段"应是详情面板中的独立操作 |

### 2.2 中等问题（体验降级）

| # | 问题 | 影响 | 建议 |
|---|------|------|------|
| 6 | **WebSocket 无自动重连** | 网络抖动导致用户丢失实时更新 | 实现指数退避重连 + 重连后同步最新状态 |
| 7 | **Engine 锁竞争** | Provider 运行期间其他 WebSocket 消息被阻塞 | 将 engine 操作拆分为短锁 + 异步通道 |
| 8 | **Timeline 与 Messages 切换逻辑不一致** | 旧 session 恢复时 timeline 为空，用户看到 messages 列表 | 统一为 timeline 视图，旧消息作为 timeline 节点展示 |
| 9 | **Provider 选择面板始终可打开** | 用户困惑为什么不能切换 Provider | 非 PrepareContext 阶段隐藏或折叠 Provider 面板 |
| 10 | **Abort 后状态清理不完整** | 前端可能卡在 streaming 状态 | 后端 abort 成功后发送明确的 stage_change 消息 |

### 2.3 设计层面问题

| # | 问题 | 分析 |
|---|------|------|
| 11 | **看板 → Workspace 的导航模型不清晰** | 用户在看板选中卡片后，需要理解"打开 Workspace"和"生成下一阶段"是两个不同操作。当前 UI 将两者混在一起，导致 Story confirmed 后无法触发 Design 生成 |
| 12 | **硬 Gate 确认缺乏上下文** | 用户在 HumanConfirm 阶段需要做 confirm/request-change/terminate 决策，但当前 UI 没有提供足够的上下文（如 reviewer 的审核意见摘要、与上一版本的 diff） |
| 13 | **多版本管理不可见** | 设计支持多版本（review 不通过时追加新版本），但 UI 上没有版本切换/对比功能 |
| 14 | **竞态条件风险** | `handleLaunchWorkspace` 中 `refresh` 后立即导航，如果 refresh 未完成就跳转，可能导致状态不一致 |
| 15 | **Issue 列无 Workspace 入口** | `LifecycleCard` 中 Issue 类型卡片不显示"打开 Workspace"按钮，但 Issue 可能有关联的 workspace session |

---

## 三、参考项目分析

### 3.1 项目管理类

#### Linear（linear.app）

**核心特点**：
- Issue 状态流转：Triage → Backlog → Todo → In Progress → In Review → Done
- AI Workflows：自动将对话/反馈转化为 Issue，自动路由、标签、优先级
- Cycle 概念：类似 Sprint，Issue 归属到 Cycle 中管理
- 极简 UI：键盘快捷键驱动，状态切换流畅

**可借鉴点**：
- **状态流转的可视化**：Linear 的状态变更是即时的、可撤销的，不需要进入全屏界面
- **AI 自动化作为辅助而非主流程**：AI 做分类/路由，人做决策
- **Cycle 归集**：将散落的 Issue 归集到时间窗口中管理

**与 Aria 的差异**：
- Linear 是人驱动 + AI 辅助；Aria 是 AI 驱动 + 人审核
- Linear 的 Issue 是扁平的；Aria 有 Issue → Story → Design → Work Item 的层级派生关系

#### Plane.so（开源 JIRA/Linear 替代）

**核心特点**：
- 开源、自托管
- 支持 Issue、Cycle、Module、View 多种组织方式
- 状态自定义：用户可定义任意状态和流转规则
- 看板/列表/甘特图多视图

**可借鉴点**：
- **状态自定义能力**：允许项目定义自己的生命周期阶段
- **多视图切换**：同一数据集支持看板、列表、时间线等多种展示
- **Module 概念**：将相关 Issue 归组，类似 Aria 的 Issue → 派生实体关系

### 3.2 AI 编程 Agent 类

#### OpenHands（原 OpenDevin）

**核心特点**：
- 完整 GUI 环境：编辑器 + 终端 + 浏览器 + 实时预览
- Agent 自主规划 → 执行命令 → 编写代码 → 验证结果
- Docker 隔离执行环境
- 支持多种 LLM 后端

**可借鉴点**：
- **执行过程的完整可见性**：用户可以实时看到 Agent 在做什么（终端输出、文件变更、浏览器操作）
- **任务规划的透明性**：Agent 的思考过程和计划对用户可见
- **中断/恢复机制**：用户可以随时中断 Agent 并接管

**与 Aria 的差异**：
- OpenHands 是单任务执行；Aria 是多阶段生命周期管理
- OpenHands 的 Agent 直接操作代码；Aria 的 Provider 生成 Spec/Design/Plan

#### SWE-agent

**核心特点**：
- 基于 OpenAI Gym 的强化学习环境
- Agent 在模拟 Linux 终端中执行操作
- 支持自然语言描述思考过程
- 定义子程序封装常用操作

**可借鉴点**：
- **Agent-Computer Interface (ACI)** 设计：为 Agent 提供专门优化的交互接口，而非直接暴露原始终端
- **观察-行动循环**：每个动作后返回观察结果，Agent 据此决定下一步
- **思考过程可视化**：Agent 的推理链对用户透明

#### Cursor / Windsurf

**核心特点**：
- Cursor：指令-执行模式，用户发指令，AI 执行
- Windsurf（Cascade）：代理式协作，AI 主动分析代码库并提供前瞻性建议
- 两者都深度集成在 IDE 中

**可借鉴点**：
- **渐进式交互**：从简单的代码补全到复杂的多文件重构，交互复杂度逐步升级
- **上下文感知**：自动收集相关代码上下文，减少用户手动提供信息的负担
- **即时反馈**：代码变更实时预览，用户可以立即 accept/reject

#### OpenAI Workspace Agents（2026.04 发布）

**核心特点**：
- 由 Codex 驱动，可在团队内共享
- 从正确的系统中收集上下文
- 遵循团队流程，需要时请求批准
- 跨工具保持工作推进
- 云端运行，人不在也能继续工作

**可借鉴点**：
- **审批流程内置**：Agent 在关键节点请求人工批准，而非事后审核
- **跨工具编排**：Agent 可以调用多个外部工具完成任务
- **异步执行**：不要求用户实时在线

---

## 四、核心问题总结与改进建议

### 4.1 交互模型的根本矛盾

**当前设计的核心矛盾**：Aria 试图在一个"对话式 Workspace"中同时承载两种截然不同的交互模式：

1. **准备阶段**：用户需要自由对话、补充上下文、选择 Provider（类似 ChatGPT）
2. **执行阶段**：系统自动运行 Provider、展示流式输出、等待审核（类似 CI/CD Pipeline）

这两种模式的 UI 需求完全不同，但当前用同一个输入框和同一个消息列表来承载，导致语义混乱。

**建议**：明确区分两种模式的 UI 状态：
- PrepareContext 阶段：对话式 UI，输入框用于补充信息，有明确的"开始执行"按钮
- Running/CrossReview 阶段：Pipeline 式 UI，隐藏输入框，展示执行进度和 Timeline
- HumanConfirm 阶段：决策式 UI，突出展示审核结论和操作按钮

### 4.2 状态持久化不足

**问题**：当前大量状态绑定在 WebSocket 连接的内存中，刷新即丢失。

**参考**：
- Linear 的所有状态变更都是持久化的，刷新后完全恢复
- OpenHands 的执行历史持久化在 Docker 容器中
- Cursor 的 Agent 执行记录持久化在本地

**建议**：
- `stream_chunk` 必须持久化（至少持久化到 session 级别的 store）
- `ActiveRun` 的控制句柄需要与 WebSocket 连接解耦
- 实现 session 恢复协议：重连后发送完整的 session_state 快照

### 4.3 看板 → Workspace 的导航模型

**问题**：用户在看板上的操作意图不明确——是想"查看已有 Workspace"还是"触发生成下一阶段"？

**参考**：
- Linear：Issue 详情页内嵌操作按钮，不需要跳转到另一个全屏界面
- Plane.so：Issue 详情面板（侧滑或弹窗），保持看板上下文

**建议**：
- 引入"卡片详情面板"（侧滑抽屉），展示当前实体的状态、版本历史、关联 Workspace
- 在详情面板中提供"生成下一阶段"和"打开 Workspace"两个明确入口
- 看板上的卡片点击 → 打开详情面板（而非直接跳转 Workspace）

### 4.4 交叉审核的可见性

**问题**：交叉审核是 Aria 的核心差异化能力，但当前 UI 对审核过程的展示不够充分。

**确认的设计意图**：Reviewer 由用户指定，但系统推荐需要交叉审核。这意味着：
- 系统需要在 PrepareContext 阶段主动提示"建议启用交叉审核"
- 用户可以选择 Reviewer Provider（如指定 Codex 作为 reviewer）
- 用户也可以选择跳过交叉审核（但系统应给出风险提示）

**参考**：
- GitHub PR Review：reviewer 的评论逐行展示，author 可以逐条回复
- Linear：Issue 的 Activity 时间线展示所有状态变更和评论

**建议**：
- PrepareContext 阶段增加"Reviewer 配置"区域，默认推荐启用交叉审核
- 如果用户未指定 Reviewer，显示"建议启用交叉审核以提高质量"的提示
- Timeline 节点需要清晰标识 Agent 角色（Author vs Reviewer）
- 审核结论需要结构化展示（verdict + 具体意见 + 建议修改点）
- 返修循环需要展示版本 diff（v1 vs v2）

### 4.5 硬 Gate 决策支持

**问题**：用户在 HumanConfirm 阶段需要做出 confirm/request-change/terminate 决策，但缺乏决策依据。

**建议**：
- 在确认界面展示：Reviewer 审核摘要 + 关键变更点 + 与上一版本的 diff
- 提供"快速预览"能力：不需要展开完整 Artifact 就能看到核心内容
- 对于 request-change，提供结构化的反馈模板（而非自由文本）

---

## 五、优先级排序建议

### P0（必须修复，阻塞主链路）

1. PrepareContext 输入框语义解耦（区分"对话"和"触发执行"）
2. Provider 授权确认传递修复
3. Story → Design 流转 UI 修复

### P1（高优先级，严重影响体验）

4. WebSocket 重连 + session 恢复
5. stream_chunk 持久化
6. ActiveRun 控制句柄持久化（支持刷新后恢复控制）

### P2（中优先级，体验优化）

7. 看板卡片详情面板（侧滑抽屉）
8. Timeline 视图完善（Agent 角色标识、版本 diff）
9. 硬 Gate 决策支持 UI
10. Engine 锁竞争优化

### P3（低优先级，锦上添花）

11. 多版本对比功能
12. Provider 面板状态优化
13. 键盘快捷键支持

---

## 六、参考项目汇总

| 项目 | 类型 | 核心参考价值 | 链接 |
|------|------|-------------|------|
| **Linear** | 项目管理 | 状态流转 UX、AI Workflows、极简交互 | https://linear.app |
| **Plane.so** | 开源项目管理 | 多视图、状态自定义、Module 归组 | https://github.com/makeplane/plane |
| **OpenHands** | AI 编程 Agent | 执行可见性、任务规划透明性、中断/恢复 | https://github.com/All-Hands-AI/OpenHands |
| **SWE-agent** | AI 编程 Agent | ACI 设计、观察-行动循环、思考可视化 | https://github.com/princeton-nlp/SWE-agent |
| **Cursor** | AI IDE | 渐进式交互、即时反馈、上下文感知 | https://cursor.com |
| **Windsurf** | AI IDE | 代理式协作、前瞻性建议、多文件自动化 | https://windsurf.com |
| **OpenAI Workspace Agents** | AI Agent 平台 | 审批流程内置、跨工具编排、异步执行 | OpenAI 官方 |
| **GitHub Agentic Workflows** | CI/CD + AI | Issue 分流、PR 审查、CI 失败分析 | GitHub Actions |

---

## 七、结论

Aria 的 Workspace 交互设计在**后端架构和数据模型**层面相对完善（产品索引层、Runtime 真相源、Provider Adapter 分层清晰），但在**前端交互层**存在以下核心问题：

1. **模式混淆**：对话式交互和 Pipeline 式执行混在同一个 UI 中
2. **状态脆弱**：过度依赖 WebSocket 内存状态，缺乏持久化和恢复机制
3. **导航不清晰**：看板 → Workspace 的跳转模型让用户困惑
4. **审核可见性不足**：交叉审核作为核心能力，UI 展示不够充分

建议优先解决 P0 级别的主链路阻塞问题，然后按 P1 → P2 → P3 的顺序逐步优化。同时可以重点参考 **Linear**（状态流转 UX）和 **OpenHands**（Agent 执行可见性）的设计思路。
