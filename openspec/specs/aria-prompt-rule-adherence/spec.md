## Purpose

确保 Aria 的 Provider prompt 直接遵守 Cadence-skills 的权威 OpenSpec 与 Superpowers 路由规则，同时保持既有的候选产物、审批 gate、恢复和结构化输出契约。

## Requirements

### Requirement: 新建与恢复的 agent 任务直接遵守 Cadence 路由规则
Aria SHALL 使每个新建或恢复的 Provider agent 任务直接引用 `agent-routing-kernel.md` 与 `openspec-superpowers-workflow.md`，并只声明该任务当前阶段对应的必调 Skill、前置 gate 和 OpenSpec/Plan 条件。Aria MUST NOT 用内部规则副本、状态机或伪造的 Skill 执行记录替代这些原始规则。

#### Scenario: Story Spec author 开始新的方案任务
- **WHEN** Aria 为 Story Spec author 创建新的 Provider 请求
- **THEN** prompt 必须直接指向两份 Cadence 原始规则，并声明新功能、行为变化或方案讨论所要求的 `using-superpowers → brainstorming` 路由

#### Scenario: Coding 请求恢复已确认 Plan 的实施
- **WHEN** Aria 恢复 Coder 或 bounded rework 的 Provider 任务
- **THEN** prompt 必须要求按原规则重新路由，并声明实施只能在已确认 OpenSpec 与 Plan 范围内进行以及写代码前的 TDD 要求

### Requirement: 节点必须保持已有 OpenSpec、Superpowers 和输出契约
Aria SHALL 在插入原始规则引用时保留每个节点原有的角色边界、`[openspec_contract]`、`[superpowers_contract]`、traceability、候选 artifact 权限、daemon canonical writeback、结构化输出和 Provider resume 契约。新增路由文本 MUST NOT 改变既有 JSON、sentinel 或 artifact fence 的解析要求。

#### Scenario: WorkItemPlan 生成规则被补强
- **WHEN** Aria 为 WorkItemPlan outline、draft、review 或 revision 组装 prompt
- **THEN** prompt 必须同时保留既有 confirmed Story/Design、`writing-plans`、最少拆分和验证约束，并引用当前阶段的 Cadence 原始规则

#### Scenario: Code Reviewer 收到规则化 prompt
- **WHEN** Aria 为 CodeReviewer 或 GroupFinalReview 组装新的审查请求
- **THEN** prompt 必须保留只读审查、EvaluationContextPack、findings schema 和既有审查材料协议，同时只声明审查阶段所需的规则

### Requirement: 纯格式修复不得重复触发工作流路由
Aria MUST 将同一 Provider 会话内仅用于 JSON、nonce sentinel、artifact fence 或结构化输出解析修复的 follow-up 与新任务或恢复任务区分。纯格式修复 prompt MUST NOT 要求再次调用 Skill、再次输出首段路由回执或重新进入 OpenSpec 生命周期。

#### Scenario: Reviewer 结构化 JSON 需要修复
- **WHEN** reviewer 输出因 nonce、JSON 或 sentinel 格式无法解析而 Aria 发送修复 follow-up
- **THEN** follow-up 只能要求修复既有输出格式并保留原 schema，且不得包含新的路由回执或阶段重置要求

#### Scenario: Artifact fence 需要重试
- **WHEN** author 候选 artifact 仅因 fence 或结构化包装不合规而需要重试
- **THEN** retry prompt 必须维持原候选内容和输出契约，且不得把 retry 作为新的 brainstorming、planning 或 coding 任务

### Requirement: 全部角色按实际阶段最小化接收规则提示
Aria SHALL 覆盖 Story Spec、Design Spec、Work Item/WorkItemPlan、Coding、Tester、Code Review、组级 PR Review、返修、集成验证、最终审查和独立 Runtime Unit Provider 节点。每个 prompt MUST NOT 同时罗列不属于当前任务的完整生命周期，也 MUST NOT 依赖 `cadence-workflow`、Hook、插件或阅读状态机。

#### Scenario: 设计确认后进入 Work Item 计划
- **WHEN** Aria 在用户确认设计并完成对应 OpenSpec 契约后启动 Work Item 计划生成
- **THEN** prompt 必须声明 `writing-plans` 阶段和 Plan 前置条件，而不得重新要求 Story/Design author 执行完整 brainstorming

#### Scenario: 最终审查与归档
- **WHEN** Aria 启动集成验证、最终审查、归档或分支收尾相关节点
- **THEN** prompt 必须只声明验证、审查、sync/archive 或分支收尾的当前规则阶段，并不得出现 `cadence-workflow` 依赖

### Requirement: 规则接入具有回归保护
Aria MUST 为各类 prompt 生命周期提供回归测试，至少覆盖新建或恢复任务的规则注入、既有契约保留和纯格式修复隔离。测试 MUST 验证 Story、Design、Work Item、Coding、Code Review 与组级 PR Review 的代表入口。

#### Scenario: 回归测试验证既有契约未丢失
- **WHEN** 修改任一受覆盖 prompt builder
- **THEN** 对应测试必须确认原有 OpenSpec/Superpowers 合同、角色权限与结构化输出 schema 仍然存在，并确认新增规则提示符合该节点阶段
