# coding-run-operability Specification

## Purpose

定义 coding run 在断连、重复启动、abort 后重启、人工继续与 reviewer 配置缺失时的可操作停等契约：已完成事实先持久化，同一时刻至多一个编码进程，所有恢复都由带对象身份的用户操作触发并回到原编排链继续。

## Requirements

### Requirement: 已完成产物先持久化后观察（REQ-CRO-01）

provider 完成一次运行后（coding 的 Coder、Code Reviewer、Internal Reviewer、返修运行，以及驱动 coding 前置的 workspace 会话修订回合），系统 SHALL 先把该运行的输出、artifact、source 与 completion checkpoint 写入既有持久面，再发送任何观察事件（WebSocket、relay 或通知）。连接 attach、detach、supersede 或观察事件投递失败 MUST NOT 取消、回滚或删除已持久化的完成结果，MUST NOT 把已完成运行改写为失败或中止。重连页面 SHALL 只从持久面补读投影。若系统无法证明 provider 是否已完成（例如完成信号与持久化之间中断），系统 SHALL 进入等待确认状态并通知用户，MUST NOT 自动重新启动该运行。

#### Scenario: finalize 与 relay 窗口断连

- **WHEN** provider 已完成返修且完成事实已持久化，驱动该 attempt 的连接在 relay 发送前断开
- **THEN** 该返修 artifact 与 completion checkpoint 保留；重连后页面补读到同一结果；后续 Code Review 只对该结果执行一次

#### Scenario: 观察通道失败不影响业务结果

- **WHEN** 没有任何 WebSocket 订阅者或观察通道投递失败
- **THEN** 业务结果与 attempt 状态照常持久化，驾驶舱 inbox 可补读，不产生失败或中止终态

#### Scenario: 完成状态不明时停等

- **WHEN** provider 输出流已结束但系统找不到对应的持久化 completion checkpoint
- **THEN** 系统落地“完成状态待确认”等待项并通知用户，展示最后活动时间与已知证据；用户确认前不启动新的 provider 运行

### Requirement: 编码进程最小互斥与接管判别（REQ-CRO-02）

同一 coding attempt 及其共享工作树 SHALL 同时至多存在一个编码进程。任何启动、重试、继续或 restart 请求 SHALL 在原子临界区内重读工作树租约与 attempt claim，并按租约三态处置：持有者活跃时拒绝第二次启动并通知“已在运行／请等待”；持有者已死（终态 attempt、锁已释放或超出活动窗口）时呈现“确认接管”操作，用户确认后方可接管并继续；活性未知时停等并要求用户确认。接管确认前系统 SHALL 展示最后活动时间与节点证据。接管后旧持有者的迟到写入 SHALL 被拒绝。同一 `command_id` 的重复请求 SHALL 返回首次结果，不产生第二个进程。系统 MUST NOT 自动抢占活跃或未知持有者，MUST NOT 为此引入 owner incarnation、epoch 或 fence 体系。

#### Scenario: 双客户端重复 kick

- **WHEN** 两个客户端对同一 attempt 几乎同时发起启动，且其中一个已成功取得编码进程
- **THEN** 另一个请求收到“已在运行”结果与通知，provider 只启动一次

#### Scenario: 同命令重放幂等

- **WHEN** 同一 `command_id` 与相同负载的启动请求被重复提交
- **THEN** 系统返回首次 durable 结果，不创建第二个编码进程或第二个 UnitRun

#### Scenario: 已死持有者经用户确认接管

- **WHEN** 工作树锁的持有者 attempt 已处于终态，用户在驾驶舱查看最后活动时间与节点证据后点击“确认接管”
- **THEN** 系统原子清出旧持有者并按当前请求继续；旧持有者随后的写入被拒绝，旧 attempt 记录只读保留

#### Scenario: 活性未知不抢占

- **WHEN** 租约持有者的活性无法由持久证据证明
- **THEN** 系统停等并通知用户确认，不自动接管、不启动 provider

### Requirement: abort 后显式 restart 可用（REQ-CRO-03）

用户明确中止 attempt 后，系统 SHALL 在同一服务进程内允许该 attempt 通过显式 restart 操作重新执行：restart SHALL 先确认该 attempt 没有活跃编码进程，再清除进程内为该 attempt 保留的中止退役状态，并经既有终态重开 admission 回到可执行状态。restart 按钮 SHALL 仅在 attempt 处于 Aborted／Failed 且无活跃编码进程时可见。restart 请求 SHALL 携带当前 attempt 身份与版本；旧 run 的迟到回执、旧版本或错对象请求 SHALL 被拒绝且 MUST NOT 复活旧 run。系统 MUST NOT 依赖重启服务来恢复 restart 能力。

#### Scenario: 同进程 abort 后 restart

- **WHEN** 用户在驾驶舱中止正在运行的 attempt，随后在同一服务进程内点击 restart
- **THEN** attempt 回到 Running 并启动一个新的编码进程，无需重启服务

#### Scenario: 旧回执不复活旧 run

- **WHEN** restart 成功后，被中止的旧 run 迟到提交完成或事件回执
- **THEN** 系统拒绝该回执，当前 attempt 状态与新 run 不受影响

#### Scenario: 仍有活跃进程时不显示 restart

- **WHEN** 中止请求已发出但旧编码进程尚未退出
- **THEN** restart 操作不可用，界面显示“正在停止”；进程退出后才可 restart

### Requirement: 人工继续后回到原编排阶段（REQ-CRO-04）

用户在 coding 门上提交允许的继续类动作（`manual_continue`、允许的质量绕过或验证处理结论）后，系统 SHALL 把门结果写入既有 gate 记录，并唤回原 runner 从该门所在阶段之后继续推进，而不是只改变门状态后停止。已完成且持久化的 review 结论 SHALL 被复用，MUST NOT 被重新执行。该继续 SHALL 不依赖提交动作的 WebSocket 连接保持打开。

#### Scenario: CodeReview 门人工继续后关闭 socket

- **WHEN** 用户在 Code Review 分诊门执行 `manual_continue` 后立即关闭页面与 coding socket
- **THEN** runner 从 Code Review 之后的阶段继续推进，已完成的 Code Reviewer 运行不被重新启动

#### Scenario: 重复提交继续动作

- **WHEN** 同一门的同一继续动作以相同 `command_id` 被重复提交
- **THEN** 系统返回首次结果，不产生第二个 runner 或重复的阶段推进

### Requirement: reviewer 配置三值完整性（REQ-CRO-05）

系统 SHALL 分别持久化 reviewer 的 effective（实际生效 provider，可为空）、provisional（用户已选但当前未生效的 provider）与 enabled（是否启用）三个值，且重建会话或 attempt 时 SHALL 按原值恢复。系统 MUST NOT 在 reviewer 为空或 disabled 时以 author/coder provider 回填 reviewer。plan 会话 reviewer disabled 时 SHALL 保持 disabled，不启动 reviewer。coding attempt 需要 Code Reviewer 或 Internal Reviewer 但其 provider 缺失时，系统 SHALL 落地“reviewer 配置缺失”等待项并通知用户，提供配置／重试操作；MUST NOT 将缺失视为自动通过或以 author 顶替。

#### Scenario: 重启后 disabled reviewer 保持 disabled

- **WHEN** plan 会话以 reviewer disabled 启动，随后服务重启并从持久面重建会话
- **THEN** 重建后的会话 reviewer 仍为 disabled、effective 为空，系统不启动任何 reviewer

#### Scenario: coding reviewer 缺失停等

- **WHEN** coding attempt 进入需要 Code Reviewer 的阶段，但其 reviewer provider 为空
- **THEN** 系统落地“reviewer 配置缺失”等待项与通知，不启动 author 作为 reviewer，不把该阶段视为通过

#### Scenario: 补齐配置后继续

- **WHEN** 用户在等待项上配置合法 reviewer 并点击重试
- **THEN** 系统以新配置从原阶段继续，历史快照不被改写

### Requirement: coding 等待项的通知与操作结果（REQ-CRO-06）

本 capability 定义的每个等待项（完成状态待确认、已在运行、确认接管、活性未知、restart、reviewer 配置缺失）SHALL 在驾驶舱 inbox 与系统通知中展示原因、attempt／unit／gate 身份、已完成步骤、可能的外部副作用、操作会做与不会做的事以及成功后的下一阶段。每个操作 SHALL 携带稳定 `command_id` 与 expected 对象版本；同键同负载返回首次结果，异负载或过期版本 fail-closed。REST 与页面动作 SHALL 调用同一应用服务。通知投递失败 MUST NOT 回滚业务事实。

#### Scenario: 过期版本操作被拒

- **WHEN** 用户在旧页面上对已被其他操作推进的 attempt 点击接管或 restart
- **THEN** 系统返回“请刷新”结果，不改变当前 attempt，不启动 provider

#### Scenario: 通知失败可补读

- **WHEN** 系统通知投递失败
- **THEN** 等待项仍可在驾驶舱 inbox 补读，业务状态不变

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

### Requirement: 转入人工恢复时的运行事实收尾（REQ-CRO-07）

coding runner 因运行期错误退出、attempt 被转入 awaiting_manual_recovery 时，系统 SHALL 在同一次转入中把该 attempt 名下仍处于运行中的 role run 记为失败，并携带与 attempt 相同的稳定原因码。收尾失败 SHALL 作为可见告警记录，MUST NOT 吞没，也 MUST NOT 阻止 attempt 转入人工恢复。收尾 MUST NOT 修改 unit 状态、启动 claim、active 指针或已落盘的完成事实。

#### Scenario: runner 死亡后 role run 不再显示运行中

- **WHEN** coding runner 在某 role run 运行期间因错误退出，attempt 被转入 awaiting_manual_recovery
- **THEN** 该 role run 的状态为失败并带有 attempt 的稳定原因码；运行历史面板不再将其显示为运行中；unit 与启动 claim 保持原值

#### Scenario: 收尾失败不阻止转入人工恢复

- **WHEN** 转入 awaiting_manual_recovery 时收尾 role run 本身失败
- **THEN** attempt 仍处于 awaiting_manual_recovery，系统留下可见告警，其余后续处理照常进行

### Requirement: 显式恢复入口与残留清理（REQ-CRO-08）

系统 SHALL 在 coding 驾驶舱为处于 awaiting_manual_recovery 的 attempt 提供可见、可操作的恢复动作，展示人工恢复原因及该动作会做什么；恢复动作 SHALL 复用既有显式恢复链路并携带当前 attempt 身份，对不处于 awaiting_manual_recovery 的 attempt 重复发出 SHALL 被拒绝且无副作用。显式恢复成功时，系统 SHALL 将该 attempt 下仍为运行中的残留 role run 记为已被取代，使其不再被展示为运行中，恢复后由重启的 runner 创建新的 role run，MUST NOT 复用被取代的 run；该清理对本 change 之前已遗留脏数据的 attempt 同样有效。恢复 MUST NOT 自动触发：服务重启、页面重连、自动续跑与依赖桥接均不得把 awaiting_manual_recovery 的 attempt 放回 Running。

#### Scenario: 驾驶舱提供恢复动作

- **WHEN** 用户打开一个 awaiting_manual_recovery 的 attempt 的 coding 页面
- **THEN** 页面显示人工恢复原因与恢复动作；点击后沿既有恢复链路回到 Running，页面不要求用户手工构造消息或重启服务

#### Scenario: 恢复清理残留运行中 role run

- **WHEN** 用户对一个 awaiting_manual_recovery 的 attempt 发出恢复动作，且该 attempt 下存在本修复前遗留的运行中 role run
- **THEN** 残留 role run 被记为已取代，attempt 回到 Running 并由新的 runner 创建新的 role run，运行历史中不存在永久运行中的旧条目

#### Scenario: 重复或错对象的恢复被拒

- **WHEN** 恢复动作发往已处于 Running、已完成或其他非 awaiting_manual_recovery 状态的 attempt
- **THEN** 系统拒绝该动作并不产生状态变化或新的 runner

#### Scenario: 无自动放行

- **WHEN** 服务重启、页面重连、自动续跑或跨 target 依赖桥接发生时存在 awaiting_manual_recovery 的 attempt
- **THEN** 该 attempt 仍保持 awaiting_manual_recovery，直到用户显式恢复
