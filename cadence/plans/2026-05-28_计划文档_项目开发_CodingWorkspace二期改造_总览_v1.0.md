# CodingWorkspace 二期改造：拆分执行总览

## 文档信息

- 文档类型：计划文档
- 分支：`product-workbench-issue-lifecycle`
- 制定日期：2026-05-28
- 版本：v1.0
- 设计文档：`cadence/designs/2026-05-28_技术方案_CodingWorkspace二期改造_v1.0.md`
- 设计评审：`cadence/designs-reviews/2026-05-28_设计评审_CodingWorkspace二期改造_v1.0.md`

---

## 一、拆分原则

1. **单 session 可完成**：每个 Plan 控制在 10-15 个任务项内，确保不触发上下文压缩
2. **独立可验证**：每个 Plan 完成后有明确的验收标准
3. **依赖清晰**：Plan 之间有明确的前后依赖关系
4. **增量交付**：每个 Plan 完成后系统仍可正常运行

---

## 二、Plan 列表

| Plan | 名称 | 核心交付 | 预估任务数 | 依赖 |
|------|------|---------|-----------|------|
| P0 | 当前场景预备收口 | Work Item 上下文 + 验证命令 + prompt 可见 + PrepareContext provider_select | 6 | 无 |
| P1 | 兼容式角色模型与 WS 协议扩展 | 5 角色内部模型 + author/reviewer 兼容层 + ContextNote 回显 | 12 | P0 |
| P2 | AttemptRunner 与 Stage Gate 机制 | 后台 runner + 命令通道 + Gate 持久化 + 倒计时交互 | 14 | P1 |
| P3 | Tester 工具协议与测试报告 | ToolCall/ToolResult 协议 + 白名单 + TestingReport | 10 | P2 |
| P4 | Rework 分析官 | Analyst Provider + AnalystVerdict 解析 + 路由决策 | 9 | P1, P3 |
| P5 | 前端 UX 对齐（展示层） | ChatEntryList 复用 + MessageGroupView 扩展 + Timeline | 11 | P1, P2 |
| P6 | CodeReview + InternalPrReview | 两个 Reviewer Provider + ReviewRequest 后 InternalPrReview 展示 | 9 | P4, P5 |
| P7 | 集成验收与 E2E | 全流程串联测试 + 边界场景覆盖 | 8 | P0-P6 |

---

## 三、依赖关系图

```
P0（当前场景预备收口）
└── P1（兼容式角色模型 + 协议）
    ├── P2（AttemptRunner + Stage Gate）
    │   └── P3（Tester 工具协议）
    │       └── P4（Rework 分析官）
    └── P5（前端 UX）
        └── P6（CodeReview + InternalPrReview）
            └── P7（集成验收）
```

**可并行**：P5 可在 P1 完成后开始，但涉及 StageGate 展示的部分需要等待 P2；P3、P4、P6 仍保持串行，避免测试工具协议、Rework 路由和 Review 路由互相踩状态机。

---

## 四、各 Plan 概要

### P0：当前场景预备收口

- Work Item markdown 返回到 Coding session state
- 从 Work Item `## 验证命令` 提取 planned test commands
- Coding prompt 注入 Work Item 上下文
- Coding provider prompt 作为 execution event 可见
- PrepareContext 阶段支持 author/reviewer `provider_select`

### P1：兼容式角色模型与 WS 协议扩展

- 扩展 `CodingProviderRole` 为 5 角色，但保留现有 author/reviewer 对外协议
- 新增内部角色配置结构，并从 `ProviderConfigSnapshot` 派生默认值
- 新增 `CodingChatEntry` + `CodingEntryType`
- 改进 `CodingContextNote`（consumed_by_rework_round）
- WS 协议新增 `CodingChatEntry`、`StreamingToken`、`StreamingEnd`、`CodingStageGate`、`ProviderSelect`、`StageGateConfirm`
- ContextNote 后端处理：存储 + 回显 + optimistic echo 前端

### P2：AttemptRunner 与 Stage Gate 机制

- WebSocket handler 与执行流解耦
- 后台 `AttemptRunner` 承载 CodingWorkspace 执行
- 客户端命令通过 runner command channel 进入执行流
- Gate 状态持久化并进入 session state
- Gate 期间 ProviderSelect 处理和倒计时重置
- 前端 `StageGateEntry` 组件
- 前端 `CodingProviderConfigPanel` 组件

### P3：Tester 工具协议与测试报告

- Provider session 暴露 `ToolCall` / `ToolResult`
- Tool 白名单机制（run_command、read_file、list_files、search_code）
- 禁止 tool 拦截逻辑
- 测试命令推断（Cargo.toml / package.json / pytest）
- TestingReport 生成
- Tester 终止条件（超时、连续失败）

### P4：Rework 分析官

- Analyst Provider 实现（只读，无 tool_use）
- AnalystVerdict 结构化输出解析
- 路由决策执行（NeedsFix → Coding / NeedsHumanInput → 暂停 / NoIssue → 下一阶段）
- ContextNote 注入逻辑（consumed_by_rework_round 标记）
- Coding 重写次数限制（max_rewrite = 3）

### P5：前端 UX 对齐（展示层）

- `ChatEntryList` 复用集成到 CodingWorkspacePage
- `MessageGroupView` 角色扩展（7 种角色）
- `InlineEventRow` tool_call 嵌套展示
- Timeline 左侧栏组件
- 角色颜色映射
- `AnalystVerdictEntry` 组件
- 消息分组规则扩展

### P6：CodeReview + InternalPrReview

- CodeReviewer Provider（分析 diff，输出 findings）
- ReviewRequest 之后执行 InternalReviewer Provider（功能影响分析，输出影响范围）
- Review 后 Rework 路由（通过 → 完成 / 不通过 → Rework）
- Review 结果前端展示

### P7：集成验收与 E2E

- 全流程 happy path 测试（Coding → Test → Rework → CodeReview → ReviewRequest → InternalPrReview → 完成）
- Rework 循环测试（NeedsFix 回到 Coding）
- NeedsHumanInput 暂停恢复测试
- Stage Gate 超时自动确认测试
- Provider 切换测试
- 重写次数上限测试
- WebSocket 重连恢复测试
- 错误场景覆盖
