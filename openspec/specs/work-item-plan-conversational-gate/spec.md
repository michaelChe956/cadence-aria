# work-item-plan-conversational-gate Specification

## Purpose

单候选 workitem plan 的 interactive 人工门从一次性决策消息升级为对话式多轮协议：人在门内以 typed 反馈回合驱动 SC 专属修订，每回合 durable、可恢复、受预算约束，直到 approve/abandon 关门；预算耗尽时门保持开启、仅剩 approve/abandon（与阶段 1 一致）。

## Requirements

### Requirement: 对话式反馈回合（REQ-CG-01）

系统 SHALL 仅将服务端接受的 typed `human_gate_feedback` 入站命令（含客户端生成的 `command_id` 与自由文本反馈）计为一个修订回合；任意其他 WS 消息（普通 `UserMessage`、重连、ack、`approve`、`abandon`）SHALL NOT 触发 provider、SHALL NOT 消耗人工修订预算。服务端接受回合后 SHALL 分配 `turn_id` 并在 durable 持久化（CAS）成功后才启动 provider；同一 `command_id` 的重复提交 SHALL 返回同一 `turn_id`，同一 `turn_id` 的重复请求 SHALL NOT 重复扣减预算或重复启动 provider。

#### Scenario: 反馈回合触发一次修订

- **WHEN** 人工门处于等待态且无 in-flight turn，客户端发送 `human_gate_feedback { command_id, feedback }`
- **THEN** 服务端分配 `turn_id`、持久化 turn 记录、扣减预算、启动一次 SC revision provider，并发出 `human_gate_turn_open` 事件（含 `turn_id` 与扣减后的剩余预算）

#### Scenario: 客户端未收到响应重发同一命令

- **WHEN** 客户端在收到 `turn_open` 前以同一 `command_id` 重发 `human_gate_feedback`
- **THEN** 服务端返回首次分配的 `turn_id`，不新建回合、不重复扣预算、不重复启动 provider

### Requirement: 单飞与预算纪律（REQ-CG-02）

同一人工门同时 SHALL 至多存在一个非终态 turn；

#### Scenario: 并发反馈返回门忙

- **WHEN** 已有 in-flight turn 时收到新反馈或 approve/abandon
- **THEN** 已有 in-flight turn 时收到新反馈或 approve/abandon SHALL 均返回 `gate_busy`，不隐式排队、不关门；

#### Scenario: 终止决定延后处理

- **WHEN** 人工门收到终止决定而当前 turn 尚未进入终态
- **THEN** 终止决定仅在 turn 进入终态后处理。

#### Scenario: 投影当前可用预算

- **WHEN** 系统计算 `manual_repairs_remaining`
- **THEN** `manual_repairs_remaining` SHALL 表达当前可用预算：同 logical gate 总额沿现默认最多3，等于总额减已计费额（delivered_charge＋nonstructural_failure_charge）减活动 hold。

#### Scenario: 反馈即占预算预留

- **WHEN** 接受真实反馈
- **THEN** 接受真实反馈时以 durable hold 立即占用一轮可用预算，不标为已交付；

#### Scenario: 交付时同回合结算

- **WHEN** 结构合法且对应原业务请求的新 source/IR/report 与 current refs 成功提交
- **THEN** 结构合法、对应原业务请求的新 source/IR/report 与 current refs 成功提交后同 turn settle 一次为 delivered_charge。

#### Scenario: 结构耗尽释放预留

- **WHEN** 收到 typed structural_exhausted_before_delivery
- **THEN** 仅 typed structural_exhausted_before_delivery SHALL 释放本轮 hold（released_structural），不实扣用户轮次；

#### Scenario: 已交付后不退款

- **WHEN** 已 deliver 后复评未闭合
- **THEN** 已 deliver 后复评未闭合不退款。

#### Scenario: 非结构终态结转扣次

- **WHEN** provider 错误、超时、取消或出现其他非结构终态
- **THEN** provider 错误、超时、取消及其他非结构终态 SHALL 维持原预约扣次不退，将 held 结转 nonstructural_failure_charge 并记精确 reason，不能假标已交付或遗留永久 hold。

#### Scenario: 旧记录不追溯重算

- **WHEN** 存在旧记录且其预算已扣
- **THEN** 旧记录已扣预算不追溯重算退款；

#### Scenario: 预留降低可用预算

- **WHEN** 新记录发生 REQ-CG-01 所述扣减
- **THEN** REQ-CG-01 所述扣减在新记录中指预留 hold 降低可用预算，不改变仅真实用户命令建立 turn 的规则。

#### Scenario: 仅新门重置预算

- **WHEN** 建立新 logical gate（新候选/新会话）
- **THEN** **预算重置边界**：普通 SC 修订门的快照预算仅在**新 logical gate**（新候选/新会话）建立时取默认值；

#### Scenario: 同门预算结转续用

- **WHEN** 同一 logical gate 内的修订轮次重建快照
- **THEN** 同一 logical gate 内的修订轮次 SHALL 从 durable 快照 carry-forward（修订成功重建快照时接续剩余值，不得重置回默认）——原「普通 SC 修订门内的人工修订 turn 成功完成并经 Evaluate policy route 重建门快照时，快照预算 SHALL 重置为默认值」修订为本句语义。

#### Scenario: 修订门预算接续

- **WHEN** amendment 人工门按 REQ-GCE-03 场景二重开
- **THEN** **amendment 人工门**的接续语义不变（REQ-GCE-03 场景二重开的原门，预算接续原 `human_gate_snapshot`）。

#### Scenario: 判别复用重开谓词

- **WHEN** 判别 amendment 门的接续或守卫
- **THEN** amendment 门的接续/守卫判别 SHALL 绑定 durable amendment 事实，且 MUST 复用重开授权（`probe_amendment_gate_context`）所用的同一完整谓词。

#### Scenario: 门消息面仅收三类

- **WHEN** SC interactive 或 amendment 门收到消息
- **THEN** **SC 门消息面边界**：SC interactive 与 amendment 门 SHALL 仅接受 `human_gate_feedback`、approve（`Confirm` 语义）与 abandon（显式 typed 入站命令）三类消息；

#### Scenario: 关门用门专属决策

- **WHEN** 承载关门决策
- **THEN** 关门决策 SHALL 以门专属 typed 决策承载，MUST NOT 依赖 legacy `HumanConfirmDecision`；

#### Scenario: 错误类型零副作用

- **WHEN** 收到错误消息类型
- **THEN** 收到错误消息类型时系统 SHALL 返回 stage-specific protocol error 且零副作用。

#### Scenario: 合法恢复操作放行

- **WHEN** 门状态为可恢复 Final Compile 且阶段/flow/durable recovery 事实匹配
- **THEN** 在门状态明确为可恢复 Final Compile 且阶段/flow/durable recovery 事实匹配时，现有 `WorkItemPlanCompileRecoveryAction` MAY 作为独立的 compile recovery 操作经 Cockpit WS 放行；

#### Scenario: 恢复操作不越边界

- **WHEN** 经 Cockpit WS 放行 `WorkItemPlanCompileRecoveryAction`
- **THEN** 该操作不是人工门反馈或关门决定，不改变三类人工门消息边界。

#### Scenario: 非法状态拒绝恢复

- **WHEN** 处于其他阶段、错误 flow、缺少 recovery 事实或已终态
- **THEN** 其他阶段、错误 flow、缺少 recovery 事实或已终态时 SHALL 返回 stage-specific protocol error 且零副作用。

#### Scenario: 计入用户动作计数

- **WHEN** 接受真实反馈
- **THEN** **门内修订轮次的 accepted 计数**：每次接受的真实反馈 SHALL 计入gate-local accepted_feedback_turns，与hold可用预算同session CAS；

#### Scenario: 计数标注用户动作

- **WHEN** 投影 accepted_feedback_turns
- **THEN** 它表示用户动作而非计费轮次。

#### Scenario: 分账投影四类轮次

- **WHEN** 系统投影已提交、已交付、非结构失败扣次与可用轮次
- **THEN** 系统 SHALL 分别投影已提交X轮、已交付Y轮、原合同非结构失败扣次F、可用Z轮。

#### Scenario: 不污染策略计数

- **WHEN** 发生用户 turn 及其 L1 结构 attempt
- **THEN** 用户turn及其L1结构attempt SHALL NOT 增加run_history repairs_used/manual_repairs_used，L1只记独立结构事件；

#### Scenario: 技术返修沿原计数

- **WHEN** 发生原 policy 技术返修
- **THEN** 只有原policy技术返修沿其既有计数。

#### Scenario: 结构尝试不计入计数

- **WHEN** 发生结构 attempt
- **THEN** 结构attempt不得增加accepted，也不重置同logical gate预算。

#### Scenario: 门内并发反馈被拒绝

- **WHEN** 存在 in-flight turn 时客户端发送新的 `human_gate_feedback`
- **THEN** 服务端返回 `gate_busy`（含当前 `turn_id`），预算与 provider 启动计数不变；门状态不变

#### Scenario: 回合进行中收到终止决定被拒绝

- **WHEN** turn 处于非终态时人发送 approve 或 abandon
- **THEN** 系统返回 `gate_busy`（含当前 `turn_id`），门状态不变、不关门；终止决定仅在 turn 进入终态后处理

#### Scenario: provider 瞬断恢复

- **WHEN** provider 在 turn 执行中 ws 断线或流式中断
- **THEN** 系统以同一 `turn_id` 递增 `attempt_no` 重试，同 turn 用户 hold/已结算记录不重复写；L0恢复与L1结构追加共享最多2物理attempt。超过上限后 turn 置 `Failed(provider_err)`，候选保持不变，未结算 hold 按非结构失败扣次结转而非退款

#### Scenario: 已删除 legacy 决策消息零副作用拒绝

- **WHEN** SC 门开启期间客户端发送已删除的 legacy `human_confirm` 类决策消息
- **THEN** 系统返回 stage-specific protocol error 且零副作用（门状态、预算、turn 均不变），关门决策不依赖 `HumanConfirmDecision`

#### Scenario: 合法 recovery action 与人工门命令并存

- **WHEN** SC 会话处于可恢复 Final Compile 状态，客户端从 Cockpit 发送既有 `WorkItemPlanCompileRecoveryAction`
- **THEN** WS 接受该独立 recovery 操作并按既有 recovery 语义处理；`Confirm`、`HumanGateFeedback`、`AbandonHumanGate` 的语义和放行边界不变

#### Scenario: 非 recovery 状态拒绝 recovery action

- **WHEN** 客户端在 generate、普通 HumanConfirm、AuthorConfirm 或已终态发送 `WorkItemPlanCompileRecoveryAction`
- **THEN** 系统返回 stage-specific protocol error，门、compile、预算与 provider 状态均不变

#### Scenario: recovery action 不替代关门命令

- **WHEN** 用户需要批准、反馈或终止 SC 人工门
- **THEN** Cockpit 继续分别发送 `Confirm`、`HumanGateFeedback` 或 `AbandonHumanGate`，不得把 recovery action 映射为其中任一命令

#### Scenario: 同 logical gate 修订不重置预算

- **WHEN** 同一 logical gate 内人工反馈修订成功且 Evaluate 重建门快照
- **THEN** 重建快照接续 durable 剩余 `manual_repairs_remaining`（如 3→2 后仍为 2），accepted_feedback_turns 递增；不回填默认值

#### Scenario: accepted 计数不污染 policy 计数

- **WHEN** 门内修订轮次发生
- **THEN** run_history 的 repairs_used/manual_repairs_used 不因真实用户turn或其L1同授权结构attempt增加，L1计入独立结构事件；只有既有policy技术返修仍沿其repairs_used合同，门卡从gate-local用户动作/receipt分账呈现轮次

#### Scenario: 结构未交付与非结构失败分账

- **WHEN** 一轮结构拒收耗尽从未deliver，另一轮因provider/timeout/取消终态
- **THEN** 前者released_structural恢复可用预算且已交付计数不增，后者nonstructural_failure_charge不退且不假标交付；accepted计数均保用户动作事实，物理attempt不增加该数

### Requirement: SC manual revision 专属路径（REQ-CG-03）

人工反馈触发的单候选修订 SHALL 走 SC 专属路径：以反馈文本、当前候选 markdown 全文与必要 grammar 边界构造独立预算的 revision prompt；

#### Scenario: 完整输出过结构门

- **WHEN** provider 产出修订结果
- **THEN** provider SHALL 输出完整修订版 markdown（非 diff/patch），输出经已有交付规范化后进入 SC compiler、canonical/split/preflight、target/成员归属、Inputs×change_order有效图及scope/intent/trusted commands全量同源结构门；

#### Scenario: 坏来源不提升候选

- **WHEN** 修订产物为坏 source
- **THEN** 坏source不得提升为可审核current候选。

#### Scenario: 不走历史约束链

- **WHEN** 走 SC 专属修订路径
- **THEN** 该路径 SHALL NOT 经过 legacy 中文标题 artifact 约束链；

#### Scenario: 不复用作者预算

- **WHEN** 构造 revision prompt
- **THEN** SHALL NOT 复用或扩张 SC author prompt 的 19,000 字节预算；

#### Scenario: 注入语言与字面量规则

- **WHEN** 构造 revision prompt 注入规则
- **THEN** 修订 prompt SHALL 注入 `.claude/rules/language.md` 全文与结构字面量优先规则句以维持中文 plan 与 grammar 字面量纪律，SHALL NOT 注入 code-usage/code-reading 摘要。

#### Scenario: 反馈受长度限制

- **WHEN** 反馈文本进入修订门
- **THEN** 反馈文本 SHALL 受确定性 bounded-field 长度限制；

#### Scenario: 超限零副作用拒绝

- **WHEN** 反馈文本超过 bounded-field 上限
- **THEN** 超限 SHALL 在创建 turn 前拒绝，零预算消耗、零 provider 启动。

#### Scenario: 内联预算内整体发送

- **WHEN** 完整组装输入不超过 inline 预算
- **THEN** 创建 turn 与扣减回合预算之前，系统 SHALL 以 UTF-8 字节计算完整组装输入（当前候选全文＋固定合同＋feedback＋上下文）并与所选 provider 的真实输入预算比较：不超过 inline 预算时整体内联发送；

#### Scenario: 超预算完整分块传输

- **WHEN** 超过 inline 预算但在 provider 硬限内
- **THEN** 超过 inline 预算但在 provider 硬限内时 SHALL 通过既有 artifact 读取能力或完整有序分块传输完整候选原文，并记录组装 digest；

#### Scenario: 候选不得截断替代

- **WHEN** 传输完整候选原文
- **THEN** 候选 SHALL NOT 被截断、摘要替代或以“模型可自行读取本地路径”为前提，SHALL NOT 新增第二个候选权威。

#### Scenario: 不可读时提前停止

- **WHEN** artifact 不可读、分块缺失或超过 provider 硬限
- **THEN** artifact 不可读、分块缺失或超过 provider 硬限时，系统 SHALL 在 turn CAS 之前拒绝并通知用户，门状态与预算不变；

#### Scenario: 点击后才开新回合

- **WHEN** 用户点击“分段返修／重试”
- **THEN** 用户点击“分段返修／重试”后才开启新回合。

#### Scenario: 尝试次数带上限

- **WHEN** 统计每个 `turn_id` 的物理 `attempt_no`
- **THEN** 每个 `turn_id` 的物理 `attempt_no` SHALL 含原调用最多2；

#### Scenario: 同回合进入重试

- **WHEN** attempt1 结构拒收且原 turn 未终态、活动 grant、事实齐全、无 unknown 调用及 anchor waiting
- **THEN** attempt1结构拒收在原 turn 未终态、活动 grant、事实齐全、无 unknown 调用及 anchor waiting 时 SHALL 携机器指令以同turn ResumeSameTurn 进入attempt2。

#### Scenario: 恢复不新增第三次

- **WHEN** L0 恢复已消费 attempt2
- **THEN** L0恢复若已消费attempt2，L1不得新增第三次；

#### Scenario: 不嵌套作者循环

- **WHEN** 处理结构 finding
- **THEN** 不得额外嵌套原+2 author循环或按finding重置。

#### Scenario: 预算与尝试次数分立

- **WHEN** 计算用户交付预算与结构 attempt 上限
- **THEN** 用户交付预算最多3与结构attempt上限2 SHALL 分立，不互抵、不借贷；

#### Scenario: 启动次数独立记账

- **WHEN** 发生真实 provider start
- **THEN** 每次真实 provider start SHALL 写入 provider-start ledger，reservation与actual launch分账，预算每turn按REQ-CG-02互斥receipt恰一次，SHALL NOT 以WebSocket事件或旧started布尔推断启动次数。

#### Scenario: 修订教学写死保留规则

- **WHEN** 构造修订教学
- **THEN** 修订教学 SHALL 写死「只改反馈点名的内容，其余逐字保留」及反面清单（禁止删字段、清空 Outputs、省略 Handoff Schema 三字段）。

#### Scenario: 优先同授权闭环

- **WHEN** 修订后出现结构 finding
- **THEN** 修订后结构finding SHALL 优先同授权L1机器指令闭环；

#### Scenario: 新内容仍经作者

- **WHEN** 修订产生新内容
- **THEN** 新内容即使唯一解也经author，指令不得伪造typed human feedback或删除AC/真实Inputs。

#### Scenario: 上下文事实完整保留

- **WHEN** 构造 prompt 及 digest
- **THEN** 原业务反馈、同源grammar/完整合法模板、成员/仓序事实和resolved predicates SHALL 完整保留于prompt及digest。

#### Scenario: 合法提交才结算

- **WHEN** 修订产物为合法候选并提交
- **THEN** 只有合法候选提交才交付结算并grant进入Reviewing，原turn仍Running；

#### Scenario: 复评分流优先

- **WHEN** 复评同一 refs/scope
- **THEN** 复评同refs/scope分流先于旧policy/reservation，禁止TriggerAggregateRepair旁路新开author；

#### Scenario: 收口回门不自动确认

- **WHEN** 回合收口
- **THEN** 收口后回原门不自动confirm。

#### Scenario: 重复发现回同门

- **WHEN** 出现非 L1 重复 findings
- **THEN** 非L1重复findings仍按原阶段1合同回同门，不重置预算。

#### Scenario: 修订成功回呈

- **WHEN** 回合输出通过全量结构套件并提交原业务请求对应的immutable refs
- **THEN** user settle恰一次且grant进入Reviewing，turn仍Running/gate_busy；原reviewer或synthetic pass收口后才以human_gate_turn_completed回呈同代候选并回等待门，人可继续业务反馈或approve，不自动确认

#### Scenario: 修订不过校验

- **WHEN** 输出被确定性结构套件拒绝且同turn仍有合法attempt2机会
- **THEN** turn不提前Failed，机器指令携原业务反馈/完整模板沿同grant同turn驱动attempt2，不新反馈、不扣另一轮；到顶从未deliver时Failed(validation_reject)+系统故障且release hold恰一次，最近合法候选保持；已deliver则保留settled receipt不退，不能要求用户修行号或拓扑

#### Scenario: 反馈超长零副作用拒绝

- **WHEN** 反馈文本超过 bounded-field 上限
- **THEN** 系统拒绝创建 turn，预算与 provider 启动计数不变；人收到超限原因后可改短重发（新 `command_id`）

#### Scenario: 大候选完整返修

- **WHEN** 当前候选为 28,695 字节，与固定合同和 feedback 组装后超过 inline 预算但在 provider 硬限内
- **THEN** 系统以 artifact 或完整有序分块传输完整候选原文，provider 基于完整候选返修，组装 digest 被记录，候选未被截断

#### Scenario: 分块缺失或超硬限在 CAS 前停等

- **WHEN** 组装时 artifact 不可读、任一分块缺失，或完整输入超过 provider 硬限
- **THEN** 系统在创建 turn 与扣减预算前拒绝并通知用户，门保持等待态、预算与 provider 启动计数不变；用户点击“分段返修／重试”后才开启新回合

#### Scenario: 同指纹重现回同一门

- **WHEN** 修订后的新候选经复评产出的 findings 与既有指纹重复
- **THEN** 服务端证实L1时先按同grant剩余attempt闭环，不按重复finding新开池；无合法机会停工程故障；其余按阶段1防环契约回同一人工门，不新门、不重置预算

#### Scenario: 不触碰 legacy 约束链

- **WHEN** SC manual revision 产出英文 grammar 标题的 markdown
- **THEN** 产物经同源全量SC结构套件判定（compiler/canonical/split/preflight/target/有效图及原scope护栏），不经过legacy中文标题artifact约束

### Requirement: 门关闭决定（REQ-CG-04）

人工门 SHALL 仅由两类决定关闭：`approve`（以 `Confirm` 语义承载，关门并启动既有确定性 compile，compile 成功后 Plan 进入 durable Confirmed）与 `abandon`（以显式 typed 入站命令承载，关门进入终态）——两类关门决定均以门专属 typed 决策承载，MUST NOT 映射或依赖 legacy `HumanConfirmDecision` 枚举。

#### Scenario: 人工权威升级语义

- **WHEN** approve / abandon 关门
- **THEN** **人工权威升级语义（显式化补句）**：approve / abandon 关门 SHALL 携带显式的人工权威升级语义——close CAS 的授权判据为 `status ∈ WaitingForHuman ∧ phase ∈ {Approval, Evaluate} ∧ 门快照在场` 三 conjunct；

#### Scenario: 批准同步内存相位

- **WHEN** approve 关门在锁内升级 phase
- **THEN** approve 关门在锁内将 phase 升级为 Approval 并 SHALL 同步内存相位（后续 compile 以同步后相位判定 auto_confirm，compile 成功落 Confirmed、SHALL NOT 回退 enter_human_confirm）；

#### Scenario: 放弃不提升相位

- **WHEN** abandon 关门
- **THEN** abandon 关门 SHALL NOT 提升 phase。

#### Scenario: 补句不改既有行为

- **WHEN** 该补句作为契约化显式表达
- **THEN** 该补句为既有死锁修复（4f34ba58）行为的契约化显式表达，SHALL NOT 改变任何既有行为。

#### Scenario: 预算耗尽拒绝反馈

- **WHEN** 用户已计费预算耗尽
- **THEN** 用户已计费预算（delivered_charge＋nonstructural_failure_charge）耗尽时系统 SHALL 拒绝新 human_gate_feedback，返回用户预算耗尽原因，原门保持开启且仅剩approve/abandon；

#### Scenario: 结构耗尽不冒充预算

- **WHEN** 结构 attempt 耗尽
- **THEN** 结构attempt耗尽 SHALL NOT 冒充用户预算耗尽。

#### Scenario: 坏产物禁止批准

- **WHEN** 产物结构合法性未过
- **THEN** 任何结构合法性未过的产物 SHALL 禁止approve，保留最近合法候选/原安全恢复，不能以预算耗尽假通过；

#### Scenario: 修复后仍需确认

- **WHEN** L1 修复成功
- **THEN** L1修复成功仍须原人工confirm，不自动关门/推进。

#### Scenario: 关门不自动推进

- **WHEN** 人工门关闭
- **THEN** 门关闭 SHALL NOT 自动触发 `advance`：未 enrolled plan 的 advance 仍由客户端在 durable Confirmed 后显式调用；

#### Scenario: 已登记计划可推进

- **WHEN** plan 已成功 compile/publication 进入 durable Confirmed 且 issue 仍保持有效单 target enrollment
- **THEN** 仅当 plan 已成功 compile/publication 进入 durable Confirmed 且 issue 仍对该精确 plan/session/target 保持有效单 target enrollment，服务端编排器方可**独立**请求 advance。

#### Scenario: 未成功不等同确认

- **WHEN** 门关闭、approve 点击或 compile/recovery 未成功
- **THEN** 门关闭、approve 点击或 compile/recovery 未成功 SHALL NOT 等同 Confirmed，不得成为服务端推进的依据；

#### Scenario: 编排器不代点门

- **WHEN** 人工门需要关闭或推进
- **THEN** 人工门仍只有原宿主，编排器不代点。

#### Scenario: approve 不绕过 compile

- **WHEN** 人在门内发出 `approve`
- **THEN** 系统关门并启动确定性 compile；仅当 compile 成功时 Plan 才进入 durable Confirmed；compile 失败按既有 fail-closed 路径处理

#### Scenario: abandon 经 typed 命令关门

- **WHEN** 人在门内发出显式 typed abandon 命令
- **THEN** 系统关门进入终态，全程不经过 legacy `HumanConfirmDecision` 通道，事件前缀不可变语义保持

#### Scenario: 人工权威升级在 Evaluate 相位同样成立

- **WHEN** 会话 `status = WaitingForHuman`、phase 停留在 Evaluate（如修订置回后）且门快照在场，人发出 `approve` 或 `abandon`
- **THEN** close CAS 按三 conjunct 判据授权关门（人工权威升级），不被「Evaluate 漂移 = 真冲突」拒绝；approve 路径锁内升级 phase=Approval 并同步内存相位，abandon 路径不提升 phase

#### Scenario: compile 以同步后相位判定

- **WHEN** approve 关门完成 phase 升级与内存同步后确定性 compile 执行
- **THEN** compile 以同步后的相位判定 auto_confirm：成功落 durable Confirmed，不回退 enter_human_confirm

#### Scenario: 预算耗尽拒绝新反馈

- **WHEN** 无活动hold且用户已计费预算耗满、manual_repairs_remaining为零且人再次发送 `human_gate_feedback`
- **THEN** 系统拒绝该反馈并返回预算耗尽原因，不创建 turn、不扣减、不启动 provider；门保持开启，仅接受 approve/abandon

#### Scenario: 伪造重开三元组不命中 amendment 接续

- **WHEN** 无任何 Open/Applying `PlanAmendmentContext` 的普通 SC 会话被通用状态写入伪造成 Completed+WaitingForHuman 三元组后，Evaluate 重建或迟到 review verdict 到达
- **THEN** 重建走普通门重置公式（默认预算 − run_history 计数），不继承遗留快照；终态评审守卫照常丢弃该 verdict，不重路由

#### Scenario: amendment 判别命中而快照缺席 fail-closed

- **WHEN** durable 重开签名与 Open/Applying `PlanAmendmentContext` 均在场，但原 `human_gate_snapshot` 缺席（损坏/外部删改）时触发 Approval 重建
- **THEN** 系统以 AbortFatal{PersistenceFailure} 报错终止（含 `human_gate_amendment_snapshot_missing` 诊断），不落任何重建快照，不回退普通重置公式

#### Scenario: 判别 durable 读失败 fail-closed 不进重置公式

- **WHEN** 真 amendment 门重开在场的会话，其判别所需 durable 事实（session record 或 `PlanAmendmentContext` 文件）损坏/不可读（权限、目录读失败、瞬态 I/O）时触发 Evaluate 重建或迟到 review verdict 到达
- **THEN** 系统以 AbortFatal{PersistenceFailure} 显式 fail-closed 终止（含 `persistence_failure` 诊断），不把「无法读取」当作「无 amendment 事实」走普通重置公式凭空恢复预算，也不静默丢弃 verdict 保持门开启；诊断写回 durable session record

#### Scenario: 应用窗口不判 amendment 门

- **WHEN** `PlanAmendmentContext` 已先行 Open→Applying 而 group attempt 已离开 AwaitingPlanAmendment（应用窗口内）时，迟到 review verdict 或 Evaluate 重建到达该 Completed+WaitingForHuman 会话
- **THEN** 终态评审守卫照常丢弃该 verdict（不重路由）；Evaluate 重建走普通门重置公式，不接续快照预算——判别与 probe 放行重开的完整谓词同源

#### Scenario: enrolled plan 仅在确认落盘后独立推进

- **WHEN** 人关门批准了 enrollment 精确绑定的 plan，但 compile/recovery 尚未成功落下 durable Confirmed
- **THEN** 不自动 advance；只有随后成功出版且当前仍 enrolled 时编排器才可独立请求 advance，不另建人工门

#### Scenario: 普通 plan 关门仍不自动推进

- **WHEN** 未 enrolled plan durable Confirmed 或曾 enrollment 但已关闭
- **THEN** 原有客户端显式 advance 语义不变，服务器不因门关闭或确认而自动 advance

#### Scenario: 结构耗尽不得假批准

- **WHEN** 结构attempt耗尽但用户计费预算尚有余量或当前候选结构未合法
- **THEN** 呈工程故障并保最近合法refs，不以“预算耗尽仅剩approve”放行坏候选，不自动confirm/compile/advance；仅合法原门动作仍可达

### Requirement: 门与回合的 durable 恢复（REQ-CG-05）

人工门快照与用户hold、accepted反馈计数、HumanGateTurn预留、provider幂等键 SHALL 以session锁/整记录CAS作为同一耐久reservation权威提交（在 session record 上以 CAS 写入，复用阶段 1 快照原子写入契约）；

#### Scenario: 恢复按预留状态处理

- **WHEN** 恢复时处理 reservation 状态
- **THEN** 恢复时按 reservation 状态决定释放、等待或继续，保证每turn held→delivered_charge/nonstructural_failure_charge/released_structural三类终态receipt幂等互斥、恰一次claim且真实start有独立ledger；

#### Scenario: 不承诺跨文件原子

- **WHEN** 承诺 durability 范围
- **THEN** 不承诺多JSON文件原子性或未知外部调用恰一次。

#### Scenario: 同会话原子绑定

- **WHEN** 写入预算 receipt、attempt reason 与各类引用
- **THEN** 预算receipt、attempt reason与source/IR/report/context/机器instruction/prompt digest及grant phase SHALL 同session CAS绑定；

#### Scenario: 按持久证据恢复

- **WHEN** 恢复跨 turn 文件
- **THEN** 跨turn文件按durable证据恢复，CAS失败不得伪结算。

#### Scenario: 回合独立持久记录

- **WHEN** 定义 `HumanGateTurn`
- **THEN** `HumanGateTurn` SHALL 为独立 durable 记录（`turn_id`、`command_id`、`feedback_text`、状态、`attempt_no`、预算预留记账、结果引用、失败分类），其 `Reserved` 状态只在与上述原子事务同提交时生效。

#### Scenario: 重连等待运行完成

- **WHEN** 断线重连后重建门与 in-flight turn 且 provider 仍在运行
- **THEN** 断线重连后系统 SHALL 从 durable 状态重建门与 in-flight turn：provider 仍在运行则等待其完成；

#### Scenario: 安全才同回合恢复

- **WHEN** provider 已终止
- **THEN** provider已终止且可安全继续才同turn恢复；

#### Scenario: 只评估或恢复评审

- **WHEN** 已有 output 或处于 Reviewing
- **THEN** 有output只assess/commit，Reviewing仅恢复绑定review，不见新artifact即Completed，不将原授权hash与已绑定中间candidate差异误认外来漂移；

#### Scenario: 恢复不重置预算

- **WHEN** 处理 Attempt2Claimed 续同 key 与 unknown 调用
- **THEN** Attempt2Claimed续同key不递增，unknown调用等待，anchor BlockedByLcRecovery优先且显式fresh核recovery×turn×grant×attempt×prompt join、不重置预算。

#### Scenario: 旧记录不补授权退款

- **WHEN** 面对旧 turn 或旧 record
- **THEN** 旧turn/旧record不追溯补授权或退款。

#### Scenario: 相位回门节点

- **WHEN** approval compile 失败或修订中止（含断连中止）
- **THEN** **phase 回门节点**：approval compile 失败与修订中止（含断连中止）后，系统 SHALL 将 phase / active_node 回滚到门节点——快照门呈现与 human_confirm 消息判定重新一致，confirm / feedback SHALL 可达且 MUST NOT 被以 `WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID` 拒绝（消除「stage=human_confirm 却拒 human_confirm」的相位死锁，0429 形态）；

#### Scenario: 回滚用新状态写

- **WHEN** 执行 phase / active_node 回滚
- **THEN** 该回滚 SHALL 以新的状态写实现，MUST NOT 改写已持久化事件前缀。

#### Scenario: 事件前缀不可变

- **WHEN** 恢复门相关事件
- **THEN** 门相关事件 SHALL 保持事件前缀不可变，恢复不得删除或改写已持久化事件。

#### Scenario: turn 预留原子性

- **WHEN** turn 预留在「预算已扣、turn 未写入」或「turn 已写、预算未扣」的窗口内崩溃，恢复后以同 `command_id` 重发
- **THEN** 系统按durable reservation/receipt证据等待或继续，同command仅一个turn/hold；最终同turn仅一次交付扣次、非结构失败扣次或结构未交付release，三者互斥，无双扣/双refund/双claim，不假称未知launch为零start

#### Scenario: 回合中断线重连

- **WHEN** 客户端在 turn 处于 provider 运行中断线后重连
- **THEN** 门状态、in-flight turn、剩余预算与既有事件原样恢复呈现；provider 已完成的结果不丢失，已死 provider 以同 `turn_id` 恢复

#### Scenario: approval compile 失败后 confirm 可达

- **WHEN** approve 关门后确定性 compile 失败、门保持开启（或按恢复动作回到门）
- **THEN** phase / active_node 回滚到门节点，人在快照门上的 confirm / 恢复动作被引擎按门语义受理，不撞 STAGE_INVALID 相位拒绝

#### Scenario: 修订中止后重连 confirm 真实关闭（0429 形态）

- **WHEN** 修订轮 provider 运行中 driver 断连中止，用户重连后快照门（resumable）呈现并发出 confirm
- **THEN** phase 已回门节点，confirm 被受理且门真实关闭（或获得明确可诊断的拒绝而非 STAGE_INVALID 死拒）；本场景承接 3.7 挂账「C 正向门关闭」的验收口径

#### Scenario: 回滚不改写事件前缀

- **WHEN** phase / active_node 回滚到门节点被执行
- **THEN** 已持久化的 timeline 事件前缀原样保留，回滚以新的状态写/迁移事件表达，无历史事件删除或改写

#### Scenario: Reviewing崩溃与重复结算

- **WHEN** 合法候选已交付settled但复评/终态写入前崩溃后重连
- **THEN** 恢复同grant同refs/scope评审，turn保持in-flight，不重复settle、不release、不另开author；事件前缀原样保留

#### Scenario: 非结构失败保持原扣次

- **WHEN** 未deliver的turn由provider错误、超时或取消终态收口
- **THEN** held恰一次转nonstructural_failure_charge、reason准确、可用预算不退；界面不假标已交付，不残留hold

### Requirement: auto stopped 接管入口（REQ-CG-06）

系统 SHALL 扩展现有 `takeover_stopped_needs_human` HTTP 端点（`src/web/handlers/workspace_session.rs`）的接管后恢复语义，SHALL NOT 新增重复入口（阶段 1 REQ-TOP-06 将本操作入口延迟至阶段 3，本 requirement 兑现之）。接管前置：仅无 fatal/persistence diagnostic 的 `stopped_needs_human` auto session 可被接管（阶段 1 既有前提）。接管时 child session SHALL 继承 parent 的人工门快照（不可变引用或复制后的校验快照）、候选/诊断引用与 `manual_repairs_remaining`（不重置），以 interactive + 对话式人工门启动；parent 保持 `stopped_needs_human`，其状态、历史与事件 SHALL NOT 被篡改。重复接管 SHALL 返回同一 child，不新建。

#### Scenario: 接管 auto stopped session

- **WHEN** 用户对 `stopped_needs_human` 的 auto session 经现有接管端点发起接管操作
- **THEN** 系统创建新 interactive session，继承原门快照与剩余预算进入对话式人工门，可发送 `human_gate_feedback`；原 session 事件前缀不变，接管会话的人工预算等于快照剩余值；重复接管返回同一 child

### Requirement: 修订门重开快照保留（REQ-CG-07）

SC 计划批准的 durable Confirmed 终态 SHALL 保留 human gate snapshot 与最近一次 Markdown 候选文本作为修订门重开与预算接续的唯一来源；系统 SHALL NOT 在批准落库时清除该快照。

#### Scenario: 批准后快照在场供修订门重开

- **WHEN** 单候选计划经 approve→compile 落入 Confirmed+Completed 终态
- **THEN** durable session 记录仍保留 human gate snapshot，后续 amendment 反馈可经 CAS 重开同一门并从同一快照接续预算

#### Scenario: 修订基线回落到最近 Markdown 制品

- **WHEN** 修订回合构建 prompt 时当前 artifact 版本无 markdown（如已被投影版本覆盖）
- **THEN** 系统 SHALL 回落取最近一个 Markdown artifact version（批准时的计划文本）作为修订基线；版本列表完全无 Markdown 时按候选缺失拒绝

### Requirement: 候选快照驱动的 plan 门恢复（REQ-C1-GATE-01）

当 plan 的候选门因 relay 失败、无 WS consumer、慢 observer 或其他观察面故障进入可恢复等待时，系统 SHALL 先将完整 candidate snapshot、source revision、budget、gate 身份与诊断事实持久化，再展示恢复操作。驾驶舱和系统通知 MUST 提供“恢复运行”或“从权威候选重建”按钮；没有完整候选快照时 SHALL 禁止裸 approve。恢复/重建成功后 SHALL 回到现有 typed feedback/approve/abandon 协议，不创建第二候选权威或自动代替用户关门。

#### Scenario: relay 失败后恢复原门

- **WHEN** relay 失败且 candidate snapshot、source revision、budget 和 gate 事实已完整持久化
- **THEN** 驾驶舱显示失败原因与“恢复运行”操作；用户点击后系统恢复原反馈/批准面并由既有编排继续，issue 不被 abandon

#### Scenario: 缺失快照禁止裸批准

- **WHEN** plan 门处于恢复等待但缺少完整 candidate snapshot 或其 source/budget 事实不可读
- **THEN** 系统不显示或拒绝 approve，显示“从权威候选重建/恢复运行”操作和缺失原因；不扣预算、不启动 provider、不伪造门已批准

#### Scenario: 无 WS consumer 仍可从驾驶舱恢复

- **WHEN** observer/WS consumer 不存在或投递失败但 durable 候选和门转换已落盘
- **THEN** 系统通过 inbox/系统通知呈现可恢复卡片，用户经 REST/驾驶舱操作提交反馈或批准后继续原门；观察面失败不被当作候选业务失败

### Requirement: plan 门操作结果通知（REQ-C1-GATE-02）

每个恢复等待项 SHALL 向驾驶舱 inbox/系统通知提供失败原因、已完成步骤、目标身份、plan/session/gate 身份、可能外部副作用、按钮动作及成功后的下一阶段。恢复操作 MUST 携带稳定 `command_id`、gate/绑定身份和 expected version；同键同负载重放 SHALL 返回同一结果，过期对象 SHALL fail-closed。

#### Scenario: 重复恢复命令幂等

- **WHEN** 客户端在首次响应未知时以同一 `command_id` 重试恢复操作
- **THEN** 系统返回首次操作结果，不重复重建候选、不重复扣预算、不启动第二 provider

#### Scenario: 旧 gate 操作被拒

- **WHEN** 用户以过期 gate/version 或不同 plan/session 身份提交恢复、feedback 或 approve
- **THEN** 系统返回需刷新/重新绑定提示，门、候选、预算和历史均不变

### Requirement: 平台自动处置结果必须回显给 owner（REQ-CG-08）

候选存在自动修复或等价规范化时，人工门 MUST 呈现「平台已自动处理 N 项」并可展开每项的依据与前后内容。owner MUST NOT 被要求回答平台已机械解决的问题，门卡文案 MUST NOT 声称平台修改了 design 或业务范围。

#### Scenario: 处置结果可见

- **WHEN** 候选编译前应用了 2 项自动处置后通过
- **THEN** 门卡 MUST 显示处置计数与明细入口，且 MUST NOT 把这两项列入待处理项

#### Scenario: 不越权表述

- **WHEN** 平台对 plan 内字段做了等价规范化
- **THEN** 门卡文案 MUST NOT 声称修改了 design 或业务范围

### Requirement: 需要 owner 判断的事项一次答完（REQ-CG-09）

编译失败需要 owner 判断时，verdict MUST 同时填充 owner_question 与至少两个 owner_options，问句与选项 MUST 由 typed 事实生成并标注各选项后果。同一轮门 MUST 把全部待决事项合并呈现，owner 的回答 MUST 落入已处置事实清单，后续轮次 MUST NOT 就同一未变事实再次提问。

#### Scenario: 环根因在 design 时合并一次问清

- **WHEN** 依赖环无法在 plan 内消解且根因是 design 仓序
- **THEN** 门 MUST 一次呈现一个问题与至少两个选项并逐项说明影响

#### Scenario: verdict 不得留空问句

- **WHEN** 门判定需要 owner 决定
- **THEN** verdict MUST 含 owner_question 与至少两个 owner_options，缺失时 MUST NOT 渲染空白裁决块

#### Scenario: 已回答事实不再重复提问

- **WHEN** owner 已回答某事实且后续轮次该事实未变化
- **THEN** 门 MUST NOT 再次以 owner_question 呈现该事实

### Requirement: 因取消或终止导致的人工修订失败必须记为中止并明说选择未生效（REQ-CG-10）

人工门修订 turn 的 provider run 因取消或终止而失败时，SHALL 记为中止而非 `failed` / `provider_err`，并 MUST 在对话流中明确告诉 owner「修订被中断，你的选择未生效」。本 requirement 复用既有中止语义（`TimelineNodeStatus::Aborted`、`HumanGateTurnFailureClass::Cancelled`、`RunAbortOutcome` 三态、REQ-WIGA-16/17），MUST NOT 引入第二套「中止」状态。

#### Scenario: 终止期间的修订不再记为 provider 故障

- **WHEN** owner 在修订 turn 的 provider run 进行中点击终止面，run 被取消，驱动因取消返回中止哨兵 `provider run aborted`
- **THEN** 该 turn SHALL 为 `Failed` 且 `failure_class=Cancelled`，MUST NOT 为 `provider_err`
- **AND** 修订 author 载体节点 `status` SHALL 为 `Aborted`，`summary` SHALL 逐字为 `运行已中止：修订被中断，你的选择未生效`，MUST NOT 为 `Failed` 或 `人工修订失败，本轮反馈未采纳`
- **AND** SHALL 发出 `human_gate_turn_failed`，`failure_class` 为 `cancelled`，`message` 包含 `你的选择未生效`

#### Scenario: 真实 provider 失败仍为 provider_err

- **WHEN** 修订驱动因 provider 真实失败、idle watchdog 或选择等待超时返回错误（消息不是中止哨兵）
- **THEN** turn SHALL 为 `Failed` 且 `failure_class=ProviderErr`，帧 `failure_class` 为 `provider_err`
- **AND** 节点 SHALL 为 `Failed`，`summary` 为 `人工修订失败，本轮反馈未采纳`，行为与本 change 前逐字相同

#### Scenario: 已中止的节点不被改写回失败

- **WHEN** 修订 author 载体节点已被 `finish_aborted_run` 置为 `Aborted`，随后 turn 因 `Cancelled` 终态化
- **THEN** 节点 SHALL 保持 `Aborted`（summary 补全为含明说句的文案），MUST NOT 回到 `Failed`

#### Scenario: 取消兜底路径文案一致

- **WHEN** drive 在取消收口窗口内未完成，由 `fail_cancelled_human_gate_revision_turn` 兜底终态化 turn
- **THEN** 回显帧 `failure_class` SHALL 为 `cancelled`，`message` SHALL 包含 `你的选择未生效`
- **AND** 修订 author 载体节点 SHALL 为 `Aborted`

#### Scenario: 对话流出现明说句且刷新后仍在

- **WHEN** 时间线存在 `status=aborted` 且 `summary="运行已中止：修订被中断，你的选择未生效"` 的 author 节点
- **THEN** 重建的对话流 SHALL 含一条内容逐字为 `修订被中断，你的选择未生效` 的系统条目，位于该节点已有条目之后
- **AND** 该条目来自 durable 节点 `summary`，`session_state` 全量重放后 SHALL 仍在

#### Scenario: 普通中止不误加修订句

- **WHEN** 时间线存在 `status=aborted`、`summary="运行已中止"` 的节点（非人工修订）
- **THEN** 对话流 MUST NOT 为其追加 `修订被中断，你的选择未生效` 条目
