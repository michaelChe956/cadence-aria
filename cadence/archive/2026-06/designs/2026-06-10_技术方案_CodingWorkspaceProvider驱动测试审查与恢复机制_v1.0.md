# CodingWorkspace Provider 驱动测试审查与恢复机制技术方案

## 文档信息

- 文档类型：技术方案
- 版本：v1.0
- 日期：2026-06-10
- 工作分支：`bugfix_test_branch`
- 适用范围：Coding Workspace 的 Testing、Analyst、Code Reviewer、Internal Reviewer 节点
- 关联记录：`cadence/analysis-docs/2026-06-10_状态记录_CodingWorkspace测试与代码审查节点问题_v1.0.md`

## 背景

2026-06-09 真实端到端测试暴露了两个核心问题：

1. Testing 节点实际只执行了少量命令，未覆盖 Work Item 中期望的贯通测试、冒烟测试、安全检查等验证范围。
2. Code Reviewer 输出不符合后端 schema 时，attempt 进入 `blocked`，但前端没有可恢复 gate，用户只能看到流程卡住。

这两个问题不能通过简单写死更多测试命令解决。Aria 是通用项目工作台，不应内置某种语言、框架或测试工具的判断逻辑。Testing、Analyst、Code Reviewer、Internal Reviewer 的专业判断应由对应 Provider 基于 Story Spec、Design Spec、Work Item、代码 diff、项目规则和 OpenSpec/Superpowers 工作法完成。

Aria 的职责是提供稳定的上下文、流程契约、工具边界、证据记录、状态恢复与 UI 展示。

## 目标

- Testing 节点由 Tester Provider 先制定 Test Plan，再按 Test Plan 执行，不再允许“随便跑几条命令后声明通过”。
- Analyst、Code Reviewer、Internal Reviewer 都必须显式使用 OpenSpec 与 Superpowers 工作法。
- Reviewer 输出异常或 Provider 中断时，Aria 保留原始输出并生成可恢复 blocked gate。
- Aria 不写死测试内容、审查内容、语言生态或具体安全工具，只校验流程契约和证据完整性。
- 前端展示 Test Plan、执行步骤、证据、缺失项、blocked 原因和恢复动作。

## 非目标

- 不在 Aria 核心中硬编码 Rust、Node、Python、Go、Java 等生态的测试命令。
- 不由 Aria 判断某个项目必须执行哪些测试类型。
- 不把“安全测试”“渗透测试”的具体工具固化到核心逻辑。
- 不替代 Provider 的专业判断，不在 Aria 里实现通用测试策略引擎。
- 不在本方案中扩展真实浏览器自动化、安全扫描、HTTP API 工具的完整实现；这些可以作为后续工具能力补充。

## 核心原则

### Provider 决策，Aria 契约

Testing、Analyst、Code Reviewer、Internal Reviewer 的判断由 Provider 完成。Aria 只负责：

- 提供完整且可追踪的上下文。
- 要求 Provider 输出结构化计划或结构化报告。
- 按计划执行或协助执行工具调用。
- 持久化证据和原始输出。
- 校验 required step 是否执行完整。
- 在 blocked 时提供可恢复 gate。

### 上下文完整，但不过量

Story Spec、Design Spec、Work Item 都应进入 Tester 和 Reviewer 的上下文，但不能无脑拼接全部 Markdown。Aria 应构建按角色裁剪的 `EvaluationContextPack`：

- 文档短时可提供完整原文。
- 文档长时提供结构化摘要、关键章节、artifact id、version id 和必要原文片段。
- 缺失或冲突必须显式标记，不允许 Provider 静默猜测。

### OpenSpec 与 Superpowers 强制启用

Tester、Analyst、Code Reviewer、Internal Reviewer 均必须在 prompt 和执行契约中显式使用：

- OpenSpec：用于对齐 Story/Design/Work Item 的需求、设计约束、任务追踪关系和变更语义。
- Superpowers：用于约束 Provider 的工程方法，例如系统化调试、TDD、代码审查、接收反馈、验证前置等工作法。

这不是可选提示，而是 Coding Workspace Provider Runtime Contract 的一部分。

## 总体流程

```text
Story Spec + Design Spec + Work Item
        + Repo Facts + Diff + Project Rules
        + OpenSpec Context + Superpowers Contract
        + Available Tools + Safety Policy
                    |
                    v
          EvaluationContextPack
                    |
          +---------+----------+
          |                    |
          v                    v
      Tester              Reviewer / Analyst
   plan_tests              review / analyze
          |                    |
          v                    v
      TestPlan          ReviewReport / AnalystDecision
          |
          v
   execute_test_plan
          |
          v
      TestingReport
          |
          v
  pass / fail / blocked gate
```

## EvaluationContextPack

### 目的

`EvaluationContextPack` 是 Testing、Analyst、Code Reviewer、Internal Reviewer 的统一上下文载体。它负责把产品产物、代码现场、项目规则、OpenSpec 约束和 Superpowers 工作法打包给 Provider。

### 建议结构

```text
EvaluationContextPack
- issue
  - id
  - title
  - description
  - change_id
- story_spec
  - artifact_id
  - version_id
  - title
  - requirements
  - acceptance_criteria
  - non_functional_requirements
  - raw_markdown_or_sections
- design_spec
  - artifact_id
  - version_id
  - title
  - decisions
  - components
  - api_contracts
  - data_model
  - risks
  - security_constraints
  - raw_markdown_or_sections
- work_item
  - artifact_id
  - version_id
  - title
  - tasks
  - dependencies
  - verification_section
  - risks
  - raw_markdown_or_sections
- repo_context
  - repository_id
  - repository_path
  - branch_name
  - base_branch
  - changed_files
  - diff
  - file_tree_summary
  - project_rules
- openspec_context
  - enabled
  - active_change_id
  - relevant_requirements
  - traceability_notes
- superpowers_context
  - enabled
  - required_methods_by_role
- execution_context
  - attempt_id
  - stage
  - provider_role
  - provider_name
  - allowed_tools
  - safety_policy
- context_warnings
  - missing_story_spec
  - missing_design_spec
  - version_conflict
  - requirement_conflict
```

### 缺失处理

- 缺 Work Item：Coding Attempt 应 blocked，因为 Coding Attempt 以 Work Item 为执行锚点。
- 缺 Story Spec 或 Design Spec：可以继续，但必须在 `context_warnings` 中标记，Provider 的 TestPlan 或 ReviewReport 必须声明上下文不完整。
- Story/Design/Work Item 存在明显冲突：应进入 human clarification 或 blocked gate，不允许 Provider 自行静默选择。

## OpenSpec 与 Superpowers 契约

### 统一 Provider Runtime Contract

每个相关 Provider prompt 必须包含统一契约：

```text
[openspec_contract]
- 你必须使用 Story Spec、Design Spec、Work Item 的追踪关系判断当前任务。
- 不得忽略已确认的 requirement、design decision、task dependency、risk。
- 如发现 Story/Design/Work Item 冲突，必须报告 blocked 或请求人工澄清。
- 不得把 OpenSpec 当作运行时真值；它是需求、设计和追踪约束来源。

[superpowers_contract]
- 你必须遵循系统化调试：先证据、后结论。
- 你必须遵循测试驱动或验证前置：先定义应验证内容，再执行验证。
- 你必须在完成判断前给出验证证据。
- 你不得用未经执行的推断替代测试或审查证据。
```

### 角色差异

Tester：

- 使用 OpenSpec 判断 Story AC、Design 约束和 Work Item 验证范围。
- 使用 Superpowers 的系统化调试、TDD、验证前置原则。
- 先输出 TestPlan，再执行。

Analyst：

- 使用 OpenSpec 判断失败、review findings 或 blocked 输出与需求/设计/任务的关系。
- 使用 Superpowers 的系统化调试和接收反馈原则。
- 生成返修策略时必须引用证据，不得凭空改范围。

Code Reviewer：

- 使用 OpenSpec 判断实现是否满足 Story、Design 和 Work Item。
- 使用 Superpowers 的代码审查原则，优先发现正确性、边界、安全、测试缺口和设计偏离。
- 必须结合 TestingReport 判断测试是否可信。

Internal Reviewer：

- 使用 OpenSpec 判断 push/PR 前状态是否满足变更要求。
- 使用 Superpowers 的代码审查和验证前置原则。
- 必须审查最终 diff、TestingReport、ReviewRequest 和提交范围。

## Tester Node 设计

### 两段式流程

Testing 节点拆为两段：

1. `plan_tests`
2. `execute_test_plan`

Provider 不能跳过 `plan_tests` 直接宣称测试通过。

### TestPlan Schema

```text
TestPlan
- id
- attempt_id
- summary
- context_warnings
- assumptions
- steps
  - id
  - title
  - intent
  - required
  - tool
  - risk_level
  - command_or_tool_input
  - evidence_expectation
  - related_requirements
  - related_design_constraints
  - related_work_item_tasks
- created_at
- raw_provider_output_ref
```

### TestPlan 生成规则

Tester Provider 基于 `EvaluationContextPack` 自行决定测试范围。计划可以包含：

- 单元测试。
- 集成测试。
- API 贯通测试。
- 前端冒烟测试。
- 安全测试。
- 基础渗透测试。
- 静态检查。
- 类型检查。
- 构建验证。
- 项目特定验证。

Aria 不内置这些内容，只要求 Provider 说明每个 step 的意图、必要性、工具和证据预期。

### Aria 校验规则

Aria 对 TestPlan 只做契约校验：

- JSON 结构合法。
- step id 唯一。
- required / tool / evidence expectation 等必填字段存在。
- tool 在允许列表中。
- 命令或工具输入不违反安全策略。
- 高风险 step 需要 permission gate。
- context warning 不得被静默忽略。

## 执行 TestPlan

### 工具调用必须绑定 step

Provider 执行任何计划内工具调用时，必须带 `step_id`：

```text
tool_call
- step_id
- tool
- input
```

未绑定 step 的工具调用可以记录为 `unplanned_commands`，但不能计入 required step 通过。

### Plan Revision

执行中如果 Provider 发现计划不足，可以输出 `plan_revision`：

- 说明新增、删除或修改 step 的原因。
- 说明与 Story/Design/Work Item 的关系。
- Aria 持久化 revision。
- 如果 revision 增加高风险操作，需要重新触发 permission gate。

第一阶段可以先不实现完整 revision UI，但数据模型应预留。

## TestingReport v2

### 结构

```text
TestingReport
- id
- attempt_id
- plan_id
- plan_summary
- steps
  - step_id
  - title
  - required
  - status
  - evidence_refs
  - provider_analysis
  - started_at
  - completed_at
- unplanned_commands
- missing_required_steps
- skipped_required_steps
- context_warnings
- overall_status
- provider_claim
- raw_provider_output_ref
- backend_verified
- started_at
- completed_at
```

### 状态语义

- `passed`：所有 required steps 已执行且通过。
- `failed`：至少一个 required step 已执行但失败。
- `blocked`：TestPlan 缺失、结构非法、required step 未执行、工具缺失、权限缺失、上下文冲突或 Provider 中断。
- `passed_with_warnings`：所有 required steps 通过，但 optional step 失败、跳过或存在上下文警告。

### 关键约束

Provider 最终声称 passed 不足以让 TestingReport passed。Aria 必须基于 TestPlan 和 step execution 判断整体状态。

## Analyst Node 设计

Analyst 负责处理 Testing failed/blocked、Code Review request_changes/blocked、Internal Review request_changes/blocked 等返修分析。

### 输入

- `EvaluationContextPack`
- 最近一次 TestingReport、CodeReviewReport 或 InternalPrReview
- 失败或 blocked 的原始证据
- 当前 diff
- 历史返修轮次

### 输出

```text
AnalystDecision
- verdict: needs_fix | needs_human_input | proceed | blocked
- summary
- rework_instructions
- evidence_refs
- related_requirements
- related_design_constraints
- related_work_item_tasks
- raw_provider_output_ref
```

### 约束

- Analyst 必须使用 OpenSpec 判断返修是否改变范围。
- Analyst 必须使用 Superpowers 系统化调试原则，先定位根因，再给返修指令。
- 如果证据不足，应输出 `needs_human_input` 或 `blocked`，不得伪造结论。

## Code Reviewer 设计

### 输入

- `EvaluationContextPack`
- 当前 diff
- Testing TestPlan
- TestingReport
- 相关 evidence refs
- 历史 AnalystDecision

### 输出

```text
CodeReviewReport
- id
- attempt_id
- round
- verdict: approve | request_changes | blocked
- summary
- findings
  - severity
  - message
  - required_action
  - evidence
  - file_path
  - line
  - source_stage
  - related_requirements
  - related_design_constraints
  - related_work_item_tasks
- tested_evidence_refs
- diff_refs
- raw_provider_output_ref
- created_at
```

### 语义

- `approve`：实现、测试证据和需求设计约束均可接受。
- `request_changes`：发现明确需要返修的问题。
- `blocked`：审查无法形成可靠结论，例如 schema 严重不完整、上下文冲突、Provider 中断、证据不足。

### 解析容错

Aria 应兼容常见字段别名：

- `file` -> `file_path`
- `description` / `summary` / `failure_scenario` -> `message`
- `recommendation` / `fix` -> `required_action`
- 缺 `source_stage` 默认 `code_review`
- 缺非关键字段时保留 finding，不丢弃整个 report

完全无法解析时：

- 保存 raw output。
- 生成 `verdict=blocked`。
- 创建 Review blocked gate。

## Internal Reviewer 设计

Internal Reviewer 负责 push/PR 前最终审查。

### 输入

- `EvaluationContextPack`
- 最终 diff
- TestingReport
- CodeReviewReport
- ReviewRequest
- commit sha
- push status

### 输出

```text
InternalPrReview
- verdict: approve | request_changes | blocked
- summary
- findings
- impact_scope
- pr_description
- commit_message_suggestion
- tested_evidence_refs
- diff_refs
- raw_provider_output_ref
```

### 约束

- 必须使用 OpenSpec 检查最终变更是否符合需求、设计和 Work Item。
- 必须使用 Superpowers 审查方法检查正确性、安全性、测试证据、提交范围。
- 若 TestingReport 不可信或缺 required steps，应 request_changes 或 blocked，不应 approve。

## Blocked Gate 设计

### 适用场景

- Tester 未输出 TestPlan。
- TestPlan schema 非法。
- required step 未执行。
- 工具权限不足。
- Provider 中断或超时。
- Reviewer 输出无法解析。
- Story/Design/Work Item 存在冲突。
- Analyst 需要人工输入。

### Gate 类型

复用或扩展 `CodingGateKind::Blocked`。

```text
CodingBlockedGate
- gate_id
- attempt_id
- stage
- reason_code
- title
- description
- evidence_refs
- raw_provider_output_ref
- available_actions
- status
- created_at
- updated_at
```

### Testing blocked actions

- `retry_test_plan`：重新生成 TestPlan。
- `rerun_missing_steps`：重新执行缺失 required steps。
- `provide_context`：提交补充上下文。
- `manual_continue`：人工标记继续，记录风险。
- `abort`：中止 attempt。

### Review blocked actions

- `retry_review`：重试审查。
- `send_raw_output_to_analyst`：把原始 reviewer 输出交给 Analyst 返修分析。
- `provide_context`：提交补充上下文。
- `manual_continue`：人工标记继续，记录风险。
- `abort`：中止 attempt。

### 持久化要求

Blocked gate 必须落盘。刷新页面、重连 WebSocket、重启服务后，`pending_gates` 仍应包含未处理 gate。

## 前端展示

Coding Workspace 页面需要展示：

- Test Plan：步骤、required/optional、工具、状态。
- Step Evidence：stdout/stderr、tool result、provider analysis。
- Testing Summary：missing required steps、warnings、overall status。
- Review Findings：findings、对应需求/设计/任务、证据。
- Blocked Gate：原因、原始输出摘要、恢复动作。

用户不能只看到“测试通过/失败”。必须能回答：

- Provider 计划验证什么？
- 实际执行了什么？
- 哪些 required step 没执行？
- 证据在哪里？
- 为什么 blocked？
- 下一步可以怎么恢复？

## 数据流

```text
Start Testing
    |
    v
build EvaluationContextPack
    |
    v
Tester Provider: plan_tests
    |
    v
persist TestPlan
    |
    v
execute plan steps
    |
    v
persist evidence
    |
    v
build TestingReport v2
    |
    +--> passed / passed_with_warnings -> Code Review
    |
    +--> failed -> Analyst
    |
    +--> blocked -> Blocked Gate
```

```text
Start Code Review
    |
    v
build EvaluationContextPack + TestingReport + diff
    |
    v
Code Reviewer Provider
    |
    v
parse and persist CodeReviewReport
    |
    +--> approve -> Review Request / next stage
    |
    +--> request_changes -> Analyst
    |
    +--> blocked -> Blocked Gate
```

## 实施分期

### 第一阶段：流程闭环

- 新增 `EvaluationContextPack` 构建。
- Tester Node 强制先输出 TestPlan。
- 工具调用绑定 `step_id`。
- 新增 TestPlan 持久化。
- 扩展 TestingReport 为 plan-based report。
- Code Reviewer 保存 raw output 并容错解析。
- Testing/Review blocked gate 落盘并进入 `pending_gates`。
- 前端展示 TestPlan、step 状态和 blocked actions。
- Provider prompt 增加 OpenSpec/Superpowers 契约。

### 第二阶段：体验和工具增强

- 支持 TestPlan revision。
- 增加 HTTP/API smoke 工具。
- 增加 browser smoke 工具。
- 增加安全测试工具封装。
- 增强 context conflict detection。
- 优化证据链 UI。
- 增加人工 override 审计记录。

## 测试策略

### 后端测试

- Tester 未输出 TestPlan 时，Testing blocked。
- TestPlan schema 非法时，Testing blocked。
- required step 未执行完时，Testing blocked。
- required step failed 时，Testing failed。
- required steps 全过、optional failed 时，Testing `passed_with_warnings`。
- 未绑定 step 的命令不计入 required step。
- Code Reviewer 缺少非关键字段时仍保留 findings。
- Code Reviewer 完全非 JSON 时生成 blocked report 和 blocked gate。
- Analyst prompt 包含 EvaluationContextPack、OpenSpec/Superpowers 契约和失败证据。
- Internal Reviewer prompt 包含最终 diff、TestingReport、CodeReviewReport、OpenSpec/Superpowers 契约。
- 重连后 `pending_gates` 包含持久化 blocked gate。

### 前端测试

- 显示 TestPlan step 列表。
- 显示 required/optional 和 step status。
- 显示 missing required steps。
- 显示 blocked gate 恢复动作。
- 点击 `retry_review`、`manual_continue`、`abort` 等动作时发送正确 WS 消息。
- 刷新恢复后 blocked gate 仍可见。

### 真实 E2E 验证

- 创建真实 Work Item。
- Tester Provider 先输出 TestPlan。
- TestPlan 中包含 Provider 基于 Work Item、Story、Design 自行制定的验证步骤。
- Aria 按 step 记录执行证据。
- 故意让 provider 漏跑 required step，确认 Testing 不会 passed。
- 故意让 reviewer 输出非标准 JSON，确认 blocked gate 可恢复。
- 确认 Tester、Analyst、Code Reviewer、Internal Reviewer prompt 均包含 OpenSpec 与 Superpowers 契约。

## 风险与缓解

- 风险：上下文过大导致 Provider 抓不住重点。
  - 缓解：使用结构化摘要 + 关键章节 + artifact 引用，不盲目塞全文。
- 风险：Provider 产出的 TestPlan 质量不稳定。
  - 缓解：schema 校验、required step 完整性校验、plan revision、blocked gate。
- 风险：用户误用 manual continue 跳过风险。
  - 缓解：记录人工 override、显示风险提示、保留审计证据。
- 风险：OpenSpec/Superpowers 只出现在 prompt 文案中，Provider 实际不遵守。
  - 缓解：报告 schema 要求引用 related requirements/design/tasks 和 evidence refs；Reviewer 检查 TestingReport 是否体现 OpenSpec/Superpowers 契约。
- 风险：第一阶段改动面较大。
  - 缓解：先实现最小 plan/report/gate 闭环，工具增强和 UI 深化放到第二阶段。

## 建议默认决策

- `manual_continue` 不直接从 Testing blocked 进入 Code Review。默认先进入 Analyst，由 Analyst 生成风险说明和继续建议；用户二次确认后才继续。
- TestPlan 的 raw markdown 上下文截断阈值第一阶段采用统一预算，后续再按 Provider 类型配置。
- OpenSpec/Superpowers 契约第一阶段写入 prompt 并持久化到 raw provider prompt 证据；第二阶段再抽象为独立 prompt contract version。
- `passed_with_warnings` 第一阶段作为 TestingReport 的新语义状态进入前后端类型；若兼容成本过高，可短期映射为 `passed` 加 warnings 字段，但 UI 必须显式展示警告。

## 结论

本方案将 Coding Workspace 的 Testing/Review 从命令驱动升级为 Provider-driven plan/report/gate 流程。Aria 不写死测试或审查内容，而是强制 Provider 基于 Story Spec、Design Spec、Work Item、OpenSpec 和 Superpowers 制定计划、执行验证、产出证据，并在失败或 blocked 时提供可恢复路径。
