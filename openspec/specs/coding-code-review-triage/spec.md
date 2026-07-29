# coding-code-review-triage Specification

## Purpose
TBD - created by archiving change open-code-review-triage-gate. Update Purpose after archive.
## Requirements
### Requirement: Code Review 人工分诊决策必须落地可操作门禁

当 Code Review 阶段的流程决策为 `StopForHumanTriage` 时，系统 MUST 落地一个 blocked gate，并把 coding attempt 状态从 `running` 置为 `blocked`。该 gate MUST 使用 reason code `code_review_output_human_triage`，MUST 绑定 Code Review 阶段与 Code Reviewer 角色。系统 MUST NOT 在该决策下仅推送会话状态就结束运行。

#### Scenario: Reviewer finding 未通过 plan defect 契约校验

- **WHEN** Code Reviewer 返回 `verdict=request_changes`，且其中至少一条 finding 的 `defect_class=implementation_defect` 同时携带非空 plan defect 路由字段，导致流程决策为 `StopForHumanTriage`
- **THEN** 系统落地 reason code 为 `code_review_output_human_triage` 的 blocked gate，attempt 状态为 `blocked`，且当前 coding unit 状态不被置为完成

#### Scenario: 停机后会话状态包含可操作门禁

- **WHEN** 上述 blocked gate 已落地并向客户端推送会话状态
- **THEN** 会话状态 MUST 包含该 blocked gate 及其可执行动作，使用户可在无需人工修改存储的情况下推进或终止流程

### Requirement: Code Review 验证不完整决策必须落地可操作门禁

当 Code Review 阶段的流程决策为 `RetryVerification` 时，系统 MUST 落地一个 blocked gate，并把 attempt 状态从 `running` 置为 `blocked`。该 gate MUST 使用 reason code `code_review_verification_incomplete`。系统 MUST NOT 为该决策引入任何自动化验证补跑路径。

#### Scenario: Reviewer 报告验证证据不完整

- **WHEN** Code Reviewer 返回的 finding 中存在 `defect_class=verification_incomplete` 且通过契约校验，使流程决策为 `RetryVerification`
- **THEN** 系统落地 reason code 为 `code_review_verification_incomplete` 的 blocked gate，attempt 状态为 `blocked`

### Requirement: Code Review 运维阻塞决策必须落地可操作门禁

当 Code Review 阶段的流程决策为 `OpenOperationalGate` 时，系统 MUST 落地一个 blocked gate，并把 attempt 状态从 `running` 置为 `blocked`。该 gate MUST 使用 reason code `code_review_operational_blocker`。

#### Scenario: Reviewer 报告运维阻塞

- **WHEN** Code Reviewer 返回的 finding 中存在 `defect_class=operational_blocker` 且通过契约校验，使流程决策为 `OpenOperationalGate`
- **THEN** 系统落地 reason code 为 `code_review_operational_blocker` 的 blocked gate，attempt 状态为 `blocked`

### Requirement: 分诊门禁必须提供四个人工处置动作

上述三个 Code Review 分诊门禁的可执行动作集合 MUST 为「送回 Coder 返修」、「重试代码审查」、「人工继续」与「终止」。系统 MUST NOT 在这些门禁上提供触发 plan repair 的动作。

#### Scenario: 门禁动作集合完整

- **WHEN** 任一 Code Review 分诊门禁落地
- **THEN** 其可执行动作集合 MUST 恰好包含 `send_to_coder`、`retry_review`、`manual_continue` 与 `abort`

#### Scenario: 门禁动作不触发 plan repair

- **WHEN** 用户在 Code Review 分诊门禁上执行上述任一动作
- **THEN** 系统 MUST NOT 唤起 plan repair 流程，plan repair 的既有唤起条件保持不变

### Requirement: 分诊门禁的送回 Coder 动作必须可用

在 Code Review 分诊门禁上执行「送回 Coder 返修」时，系统 MUST 走代码审查反馈返修路径，落地 rework instruction、把 stage 置为 `Coding` 并递增返修计数。该动作 MUST 支持最近一次审查结论的 verdict 为 `request_changes` 或 `blocked`，MUST NOT 因 verdict 为 `request_changes` 而拒绝执行，且 MUST NOT 走审查轮次超限反馈路径。

#### Scenario: verdict 为 request_changes 时送回 Coder

- **WHEN** 最近一次审查结论 verdict 为 `request_changes`，attempt 处于 Code Review 分诊门禁的 `blocked` 状态，用户执行 `send_to_coder` 并提供操作说明
- **THEN** 系统落地 rework instruction，attempt stage 变为 `Coding`，返修计数递增，且不返回不可执行错误

#### Scenario: 未提供操作说明时拒绝执行

- **WHEN** 用户在 Code Review 分诊门禁上执行 `send_to_coder` 但未提供操作说明
- **THEN** 系统 MUST 拒绝该动作并保持 attempt 处于 `blocked` 状态

### Requirement: 单次审查结论只允许落地一个 blocked gate

对同一次 Code Review 审查结论，系统 MUST 最多落地一个 blocked gate。既有的 `code_review_blocked` 门禁与本次新增的三个人工路由门禁 MUST 互斥。

#### Scenario: verdict 为 blocked 且无可执行 finding

- **WHEN** Code Reviewer 返回 `verdict=blocked` 且报告不含任何可执行 finding，该情形同时满足既有 `code_review_blocked` 条件与 `StopForHumanTriage` 决策
- **THEN** 系统只落地 reason code 为 `code_review_blocked` 的单个 blocked gate，不额外落地人工分诊门禁

### Requirement: Reviewer implementation defect 输出契约边界

Reviewer 的结构化输出契约 MUST 声明：`defect_class=implementation_defect` 的 finding 禁止填写 `reason_code`、`contract_refs`、`capability_refs`、`repair_target`、`confidence` 与 `plan_defect_evidence`，这些字段必须省略或为空。该契约 MUST 同时声明该类 finding 的证据出口为 `message` 与 `required_action` 的自然语言描述。系统 MUST NOT 因此放宽 plan defect finding 的既有校验判定。

#### Scenario: Reviewer 收到 implementation defect 字段边界约束

- **WHEN** 系统为 Code Reviewer 渲染 work item projection 执行上下文
- **THEN** 渲染文本 MUST 同时包含 implementation defect 的路由字段禁令与自然语言证据出口说明

#### Scenario: 校验判定保持不变

- **WHEN** 契约文案更新后，某条 finding 仍以 `defect_class=implementation_defect` 携带非空 plan defect 路由字段
- **THEN** 该 finding 仍 MUST 判定为契约校验失败，流程决策仍为 `StopForHumanTriage`

