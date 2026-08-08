# logical-codebase-aggregate-planning Specification

## Purpose

规划类产物（Issue / Story / Design / Work Item 计划）使用聚合上下文：provider 从聚合根只读启动（P1 best-effort），注入成员清单、目标仓 profile、索引摘要与政策 envelope。

## ADDED Requirements

### Requirement: 聚合规划上下文（REQ-PLN-01）
系统 SHALL 将规划类 workspace 上下文从单仓库改为逻辑代码库聚合：包含 logical_codebase 引用、成员 inventory（id/alias/path/role/profile 摘要）、focus 成员与索引 revision；prompt 注入紧凑成员清单与预算，不注入全部成员源码；超预算时确定性截断并标记未检索成员。

#### Scenario: 聚合上下文注入
- **WHEN** 为逻辑代码库下的 Issue 启动 Story/Design/Work Item 计划生成
- **THEN** provider 从聚合根（provider_context_root）只读启动，prompt 注入紧凑成员清单与预算控制，不注入全部成员源码

#### Scenario: 预算超限
- **WHEN** 成员 inventory/摘要/证据超出 token 或 byte 预算
- **THEN** 系统 SHALL 按确定性顺序截断并标记未检索成员，不撑爆 context window

### Requirement: Issue 逻辑代码库归属与参与仓库（REQ-PLN-02）
系统 SHALL 通过 `project_id` 表达 Issue 的逻辑代码库归属，通过 `IssueCodebaseSelection`（include/exclude/focus）表达参与仓库；`IssueRecord.repo_id` 仅作迁移期 focus/primary 投影；focus 可为多值、必须在 include 集合内，include/exclude 重叠时 exclude 优先。

#### Scenario: 创建逻辑代码库 Issue
- **WHEN** 创建逻辑代码库下的 Issue
- **THEN** 可指定参与/排除/重点仓库，服务端校验成员归属与优先级，DTO 返回参与仓库摘要

#### Scenario: 成员变更
- **WHEN** 规划后成员被删除/停用
- **THEN** 系统 SHALL 使相关 Issue selection 与 planning snapshot 失效或标记，阻塞指向已删除成员的新工作项

### Requirement: 规划上下文快照（REQ-PLN-03）
系统 SHALL 每次规划 run 固化 `PlanningContextSnapshot`（membership_revision、每仓 checkout revision/dirty/availability、index revision、policy digest、access fingerprint），作为 context/cwd/prompt/audit 的唯一依据。

#### Scenario: 恢复/续接规划会话
- **WHEN** 恢复或续接规划会话
- **THEN** 系统 SHALL 校验快照指纹；不一致时启动新会话并重建上下文，不沿用可能过时/越权的内容

### Requirement: Story 聚合视野（REQ-PLN-04）
系统 SHALL 使 Story 生成时列出涉及仓库（`involved_repositories`，引用 member ID）；AI 不确定涉及仓库时产生 blocker 向用户确认，不得猜测或默认塞给 primary。

#### Scenario: Story 涉及跨仓改动
- **WHEN** Story 涉及跨仓改动（如订单服务 + 库存服务 + 前端）
- **THEN** Story SHALL 明确列出涉及仓库并描述各自改动范围；AI 无法确定涉及仓库时产生 blocker

### Requirement: Design 聚合视野与改动顺序（REQ-PLN-05）
系统 SHALL 使 Design 显式携带 `logical_codebase_ref` 与 `involved_repositories`，不再回落 `issue.repo_id`；跨仓关系由 AI 从聚合索引按需检索（启发式，非语义级服务图）；Design 表达改动顺序作为 Work Item `depends_on` 依据。

#### Scenario: Design 表达跨仓改动顺序
- **WHEN** Design 涉及跨仓接口/契约变更
- **THEN** Design SHALL 表达改动顺序（如 公共契约 → provider → consumer），作为 Work Item `depends_on` 依据

### Requirement: 规划只读边界（REQ-PLN-06）
P1 规划 provider 会话 SHALL 为只读语义的 best-effort（`best_effort_configured`：Aria-owned 配置 + cwd + pre/post 检测）；仅在固定 provider/OS 越界写 fixture 通过后才可升级为 `production_verified_readonly`；未达到该级别时不得宣称「物理上无法写入」；疑似越权写入需被检测并标记。

#### Scenario: 规划运行
- **WHEN** 规划 run 完成
- **THEN** 系统 SHALL 报告 best-effort 只读状态（配置目标 + 前后检测），成员主 checkout 与聚合根无规划产生的已知写入；硬只读需 PreToolUse deny（Write/Edit when plan）或路由 Codex `:read-only`，Claude plan 模式仅为 prompt 层

### Requirement: 唯一规划 launch/resolver（REQ-PLN-07）
系统 SHALL 提供唯一 `PlanningContextResolver`，使 Story/Design/WorkItemPlan 的 context、cwd、prompt、session audit 均来自同一 `PlanningContextSnapshot`；禁止任何 `issue.repo_id`/first Story fallback。

#### Scenario: 规划全链路一致
- **WHEN** 启动、run-next、revision、resume 或 WebSocket follow-up
- **THEN** 各环节 SHALL 使用同一 PlanningContextSnapshot 解析，不出现单仓 fallback
