# Spec Delta

> 基线：`openspec/specs/coding-run-operability/spec.md` 现有 REQ-CRO-01 至 REQ-CRO-06（REQ-CRO-07/08 由未归档 change `fix-local-delivery-and-chain-truth` 新增，与本 delta 无重叠）。本 delta 只新增 REQ-CRO-09 至 REQ-CRO-12，不修改既有 requirement；REQ-CRO-06 等待项通知语义保持不变，REQ-CRO-12 是其全局事实投影扩展。

## ADDED Requirements

### Requirement: coder 门大白话卡与动作集（REQ-CRO-09）

reason_code 属于 `{coding_output_human_triage, coding_operational_blocker, coder_provider_stalled}` 的开放 blocked gate SHALL 用统一分诊卡呈现「问题/做了什么/没能验证什么」三段大白话（来自 `triage_summary` 投影，缺失时问题段回落 description），按钮由 `available_actions` 驱动。新门 SHALL 下发三动作，旧门动作集 MUST 保持两动作；「验证处理」MUST 折叠在「技术详情」的二级折叠且功能与审计保留；其他门渲染 MUST 零改动。

#### Scenario: 环境阻塞门三按钮大白话卡

- **WHEN** attempt 落 `coding_operational_blocker` 门（标题「运行环境阻塞」）或 `coder_provider_stalled` 门，且携带 triage_summary
- **THEN** 卡片按固定文案与顺序显示三段大白话与三个按钮（让 AI 再试一次 / 我来说明情况，让 AI 继续 / 终止），技术详情默认折叠

#### Scenario: 旧门两按钮与旧数据回落

- **WHEN** 旧门 `coding_output_human_triage` 或不含 triage_summary 的存量 gate JSON 渲染
- **THEN** 动作恰为「让 AI 再试一次/终止」两按钮（X2 裁决，无第三按钮），问题段显示 description，反序列化不失败（`triage_summary=None`）

#### Scenario: 验证处理折叠保留

- **WHEN** 用户展开分诊卡的技术详情再展开「高级：验证处理」
- **THEN** 原「转入验证处理」按钮、命令证据区与审计行为原样可用，回调仍携带 gate 身份

#### Scenario: 其他门零改动

- **WHEN** CR 三门、`code_review_blocked`、`coder_provider_interrupted` 等其他 blocked gate 渲染
- **THEN** 渲染路径与既有快照一致，按钮集合不变

### Requirement: retry_coding_with_context 动作语义（REQ-CRO-10）

新 gate 动作 `retry_coding_with_context`（标签「我来说明情况，让 AI 继续」）SHALL 走既有 WS `gate_response`，`extra_context` 去空白后必填且 ≤2000 字符。后端 SHALL 先写 context note、成功后清 coder 会话回 Running/Coding（提交即重跑），note 写失败则动作失败、gate 保持 Open；下一轮 prompt SHALL 带出该上下文且续跑白名单包含该动作。该动作与既有 `provide_context` 并存互不替代，仅由新门（`coding_operational_blocker`、`coder_provider_stalled`）下发。

#### Scenario: 提交说明即重跑并带上下文

- **WHEN** 用户在环境阻塞门输入一两句话（非空、≤2000 字符）提交 `retry_coding_with_context`
- **THEN** 系统先写 context note 再清 coder 会话，gate 被 resolve，attempt 回到 Running/Coding，runner 续跑；下一轮 coder prompt 经既有 `append_coding_context_notes` 含「本轮补充上下文」与该文本

#### Scenario: 空与超长被拒且门保持

- **WHEN** 提交时 extra_context 去空白后为空，或长度超过 2000 字符
- **THEN** 分别返回 `coding_gate_extra_context_required` / `coding_gate_extra_context_too_long`，不写 note，gate 保持 Open，attempt 状态不变

#### Scenario: note 写盘失败不半跑

- **WHEN** context note 写盘失败
- **THEN** 动作整体失败，不清会话、不改 attempt 状态、gate 保持 Open，用户可重试；不写 quality_bypass_audit

### Requirement: choice 逐题 REST 作答（REQ-CRO-11）

coding choice 的实时帧与快照 entry SHALL 携带 `questions`；Coding 页提交 SHALL 改走与驾驶舱同一 REST 通道，恒携带逐题 `answers[question_id]`、`command_id`、`expected_run_id`（无 questions 单题回落 `default`）。取不到 `expected_run_id` 时前端 MUST NOT 发请求且卡片 SHALL 显示人话提示、不提供恢复按钮；WS `choice_response` 后端处理 SHALL 保留；失败状态 SHALL 显示人话原因且可重试。

#### Scenario: 多题逐题提交到达 provider

- **WHEN** provider 发起含两道题的 choice，用户在 Coding 页逐题作答提交
- **THEN** 请求体含两题各自 `answers[question_id]`、`command_id` 与 `expected_run_id`，provider 收到与题号匹配的非空答案，状态按 submitting→resolving→delivered 呈现

#### Scenario: 无 runner 不发请求且说人话

- **WHEN** attempt 停在 WaitingForHuman 且有开放 choice 但无活跃 runner（如服务重启后）
- **THEN** 卡片显示「AI 当前没有在运行，这个问题暂时无法回答」，无恢复按钮（X1），不发出任何提交请求

#### Scenario: 失败可见且可重试

- **WHEN** 提交返回 `expired`（410）或 `conflict`（409）/`unknown`（404）
- **THEN** 卡片分别显示失效/冲突/未知的人话原因，允许修改答案后以新 `command_id` 重试，不静默丢弃

### Requirement: coding 待办全局事实投影（REQ-CRO-12）

系统 SHALL 以 durable 事实只读投影提供 coding 待办条目 `CodingAttentionItemDto`（key/kind/project/issue/plan?/attempt?/title/summary/occurred_at/target/focus 等，kind 覆盖 choice/triage/blocked/stalled/manual_recovery/final_confirm/plan_final_confirm，target 为 coding_attempt/coding_plan/workspace_session 三型）。lifecycle SHALL 暴露 `coding_attention_items`；occurred_at MUST 取事实时间；投影 MUST NOT 建通知表/ack，事实关闭条目即消失；plan ready/partial SHALL 聚一条 `plan_final_confirm` 并抑制逐仓催点；投影核心 MUST NOT 反调 GET/lifecycle 形成递归；单仓/手动同样被投影。

#### Scenario: 各类待办有稳定 key 且处理即消失

- **WHEN** 某 attempt 分别处于开放 choice、分诊门、环境/挂起门、待人工恢复、待最终确认
- **THEN** lifecycle 返回对应 kind 的条目且 key 稳定（choice/gate/stage/final 维度），occurred_at 为事实创建时间（gate/choice 的 created_at）；对应事实关闭后条目不再出现，无已读历史

#### Scenario: 多仓聚一条 plan_final_confirm

- **WHEN** 多 target plan 的全部 WI 执行完成、剩余 attempt 均待最终确认且 readiness 就绪
- **THEN** 只出现一条 kind=plan_final_confirm 的条目指向 plan 总览，该 plan 的逐仓 final_confirm 条目被抑制；中途逐仓待确认不产生催点条目

#### Scenario: 投影不递归且不限 enrolled

- **WHEN** 投影核心计算 plan 确认状态，或单仓/手动（非 enrolled）attempt 存在待办事实
- **THEN** 核心直接读 attempt facts 并只调用块 1 纯 `build_plan_final_confirm`，不调用 GET/lifecycle；单仓/手动条目照常投影
