# coding-code-review-triage Specification

## Purpose
为 Code Review 阶段需要人工介入的三类流程决策（人工分诊、验证不完整、运维阻塞）落地可操作的 blocked gate，提供「送回 Coder 返修、重试代码审查、人工继续、终止」四个处置动作且不触发 plan repair；约束单次审查结论至多落地一个门禁，并界定 Reviewer implementation defect finding 的输出契约字段边界。
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

### Requirement: 分诊门人工继续后回到原编排链

在任一 Code Review 分诊门（`code_review_output_human_triage`、`code_review_verification_incomplete`、`code_review_operational_blocker`）执行 `manual_continue` 后，系统 MUST 关闭该门、把 attempt 从 `blocked` 恢复为可执行，并唤回 runner 从 Code Review 之后的阶段继续推进。系统 MUST 复用该次已持久化的 Code Review 结论，MUST NOT 重新启动已完成的 Code Reviewer 运行。该续跑 MUST NOT 依赖提交动作的连接保持打开。

#### Scenario: manual_continue 后自动续跑

- **WHEN** 用户在 Code Review 分诊门执行 `manual_continue`，随后关闭页面与 coding socket
- **THEN** attempt 离开 `blocked` 并从 Code Review 之后的阶段继续推进，Code Reviewer 运行次数不增加

#### Scenario: 续跑中断后可再次继续

- **WHEN** `manual_continue` 已持久化但 runner 唤回失败
- **THEN** 系统落地可操作等待项并通知用户，用户点击继续后从同一阶段续跑，不重复已完成 review

### Requirement: 验证处理入口独立于分诊门四动作

三个 Code Review 分诊门的可执行动作集合 MUST 保持恰好为 `send_to_coder`、`retry_review`、`manual_continue` 与 `abort`。当门的 finding 涉及计划验证命令不可满足时，系统 MAY 在门旁提供转入独立“验证处理”面的入口；该入口 MUST NOT 作为门动作出现在门的动作集合中，转入本身 MUST NOT 唤起 plan repair，MUST NOT 关闭或改写该门及其 finding。验证处理中用户批准“计划修订”时，系统 MUST 经既有 plan amendment 链由验证处理记录发起，而不是作为该门的动作。验证处理得出用户批准的等价证据或限域例外结论后，原链继续的效果 MUST 与在该门执行 `manual_continue` 一致并留下验证处理审计。

#### Scenario: 转入验证处理不改变门动作

- **WHEN** Code Review 验证不完整门打开，用户点击“进入验证处理”
- **THEN** 门的可执行动作集合仍恰好为四动作，门与 finding 保持不变，验证处理记录被创建

#### Scenario: 验证处理批准后续跑

- **WHEN** 用户在验证处理面批准可信等价证据
- **THEN** 原门关闭并按 `manual_continue` 语义续跑，finding 保留且关联该验证处理审计，不唤起 plan repair
