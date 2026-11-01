# work-item-group-autopilot Specification

## Purpose

在 story/design 仍由人工生成和确认的前提下，为显式选定自动化且仅涉及单一 logical repository 的 issue 提供不依赖页面/driver 连接的 Work Item Plan→Work Item Group coding 推进；遇到需要人的选择、计划门、编译恢复与最终确认时始终停等，由驾驶舱呈现可操作入口。未选自动化的既有流程保持原样。

## ADDED Requirements

### Requirement: 人工 design 确认后的显式授权与范围（REQ-WIGA-01）

系统 SHALL 保持 story/design 的生成和确认由人完成，并仅在 design 经人工确认后提供「自动化模式」选择；默认 SHALL 为「不自动化」，选择后者 SHALL 沿既有手工 plan/advance/coding 流程，旧 issue/已存在 plan SHALL NOT 被自动接管。选择「自动化」SHALL 建立 issue 级 durable `IssueAutomationEnrollment`，含唯一 `enrollment_id`、`enabled`、`policy_revision`、明确的 story/design 身份与版本、provider/options、绑定 plan/session 身份及允许的 target 范围；只允许恰一 logical repository，零/多 target 或身份不一致 SHALL fail-closed 拒绝自动授权，而不影响多 target 的人工流程。同一次选择的重复提交 SHALL 幂等命中，不得创建第二条绑定链；不同 source/配置冲突 SHALL 明确拒绝，不得静默改绑。旧数据无 enrollment SHALL 按 off 解释。

#### Scenario: design 确认前不出现自动授权

- **WHEN** story 或 design 尚未由人确认
- **THEN** 不创建 enrollment、不自动准备 plan，原有人工生成与确认入口继续可用

#### Scenario: 默认不自动化

- **WHEN** 用户在 design 确认后选择「不自动化」或旧 issue 不含 enrollment
- **THEN** plan 准备、生成、advance 和 coding 启动仍按原有手动行为执行，不因自动化补偿扫描产生动作

#### Scenario: 单 target 精确绑定及重复提交

- **WHEN** 已确认的 source 只对应一个 logical repository，用户用相同选择键重复开启自动化
- **THEN** 系统仅返回同一 enrollment、同一绑定 plan/session 与策略修订；不同 source 或 provider/options 的冲突提交明确拒绝且不改绑

#### Scenario: 多 target 不被自动纳入

- **WHEN** 已确认 source 或冻结的 plan/attempt 涉及多个目标仓库、目标不明确或与 enrollment target 不符
- **THEN** 系统拒绝自动授权或自动启动，绝不循环启动多个 attempt；既有逐 target 手工准入仍可用

### Requirement: durable 策略与可撤销的未消费执行许可（REQ-WIGA-02）

单一 issue 的 enrollment SHALL 是自动化授权唯一来源；`policy_revision` 的并发更新 SHALL 线性化。绑定的 plan session `run_policy` SHALL 保持 `Interactive`，MUST NOT 用 `AutoIfValid` 表示 enrollment 或跳过人工 plan 门。coding attempt SHALL 物化 `CodingStartRunPolicy = Manual | AutoStartOnce{enrollment_id, policy_revision, source_plan_revision}`，缺省为 Manual，自动许可只准该绑定 target/attempt 使用且须在消费前再核对当前授权。关闭自动化 SHALL 拦住线性化关闭后尚未认领的动作并撤销未消费的启动许可；已被认领/运行的动作不因关闭而自动 Abort。重新开启 SHALL NOT 清除已消费的单发记录或隐式重试 Failed/Aborted attempt。

#### Scenario: Interactive 保留唯一人工门

- **WHEN** enrolled plan 已经通过机械评审进入人工计划门
- **THEN** session 仍为 Interactive 并等待人 approve/feedback/abandon；仅 approve 后现有确定性 compile 成功才可能 durable Confirmed

#### Scenario: 关闭发生在启动认领前

- **WHEN** 用户关闭 enrollment 并在线性化点后才有编排动作或自动 StartCoding 请求尝试认领
- **THEN** 该动作被拒且无 provider 启动；虽已物化但未消费的 AutoStartOnce 许可不再生效

#### Scenario: 关闭发生在启动认领后

- **WHEN** 自动启动已原子认领并开始运行，用户随后关闭 enrollment 或再开启
- **THEN** 已运行 provider 不被开关隐式中止；重新开启不再次消费该 attempt 的启动许可，不自动重启 Failed/Aborted attempt

### Requirement: 无页面的确定性后端推进与人工停点（REQ-WIGA-03）

对仍有效且精确绑定的单 target enrollment，服务端 SHALL 从 durable 状态重建下一动作：至多幂等准备一次 plan/session、至多启动一次对应生成、等待人处理 choice 和人工计划门、等待既有 compile/publication 成功、以独立 `advance` 推进到 Ready、再通过共用 StartCoding 命令启动该 attempt。编排事件 SHALL 仅作为唤醒，启动扫描与有界补偿 SHALL 处理漏事件与重启；无页面或没有任何 WS 连接时也 SHALL 继续自动动作及恢复。门关闭、approve 点击、compile 节点结束、advance_completed 均 SHALL NOT 被误判为 plan Confirmed、coding 完成或 provider 启动。compile recovery、失败/中止及绑定读取故障 SHALL 停等显式处理或 fail-closed，不得盲目再发有不确定副作用的 provider 启动；编排器 MUST NOT 成为第二个 provider 驱动者。

#### Scenario: 从 design 确认自动生成且停在人工门

- **WHEN** 单 target design 经人确认后选择自动化，用户关闭工作区与 driver 连接
- **THEN** 服务端准备唯一 plan/session、启动一次生成；到 choice/人工门时停等驾驶舱操作，不自动回答或代批准

#### Scenario: 确认后独立推进编码

- **WHEN** 人批准了绑定 plan 且既有 compile/publication durable 成功，enrollment 仍授权
- **THEN** 服务端请求独立 advance 到单 attempt Ready，随后经 StartCoding 发起 coding；advance 自身不启动 provider

#### Scenario: 失败或漏唤醒恢复

- **WHEN** 服务重启、事件漏投、计划创建已成功但绑定未写、或动作在持久检查点间中断
- **THEN** 后续扫描/补偿重用同一 plan/session/action 身份，已认领的启动只按权威事实恢复或进入显式人工分诊，不创建重复 plan、尝试或并行 provider run

### Requirement: StartCoding 单入口与 per-attempt 单发（REQ-WIGA-04）

人工 WS 命令和 enrolled 编排器 SHALL 经同一 typed StartCoding 准入服务，origin 分为 `Manual` 与 `Enrolled{enrollment_id,policy_revision}`；在同一 attempt 临界区内重读 attempt、核验状态及当前授权、精确 plan/target/attempt 绑定和 durable Ready，然后认领并持久化单发意图/启动检查点，才可放行既有 coding runner。SC attempt 未经 advance Ready、绑定读取失败或绑定不符 SHALL 以 `SC_CODING_REQUIRES_ADVANCE` fail-closed 零副作用拒绝；重复 `command_id` 或同 attempt 已启动 SHALL 返回原状态/结果，不得第二次启动。自动启动绝不批量，既有显式 Restart/Recover 不得被冒充为自动首次启动；外部副作用是否发生无法判明时须呈现人工恢复，不宣称 provider exactly-once。

#### Scenario: 手工与后台同时请求同一 attempt

- **WHEN** 人工 StartCoding 与已授权的自动 StartCoding 同时进入同一 attempt
- **THEN** 最多一个请求认领首启，另一个幂等命中该 attempt 的原状态，不出现两个 runner/provider

#### Scenario: Ready 前与身份不匹配均拒绝

- **WHEN** attempt 未被该 plan 的 durable advance 绑定置 Ready、绑定不可读、或 target 与 enrollment 不一致
- **THEN** 自动请求零启动；SC Ready 前请求复用 `SC_CODING_REQUIRES_ADVANCE`，不得直接调用 runner 绕过守卫

#### Scenario: 崩溃后不盲目二次启动

- **WHEN** 启动意图已持久化但 runner 是否触达外部 provider 不明，服务重启并补偿该 attempt
- **THEN** 服务只能按单发检查点与既有恢复协议重用原启动或进入明确的人工恢复态，不重新发一条首启命令

### Requirement: 驾驶舱 choice 与人工门无 driver 操作（REQ-WIGA-05）

驾驶舱 SHALL 对 workspace plan 的 choice 提供 REST `POST /api/workspace-sessions/{session_id}/choices/{choice_id}/response`，请求携带稳定 `command_id`、`expected_run_id` 与完整 `answers[{question_id,selected_option_ids,free_text}]`；coding 阶段 choice SHALL 有与对应 attempt 绑定的可操作入口。REST 与既有 WS 回复 SHALL 共享同一 run/choice 的原子认领、解析和真实等待者交付回执，observer WS 继续只读。HTTP `200` SHALL 仅表示当前 run 等待者实际收到该答复，不承诺后续 provider 成功；回执在 HTTP 等待期限内未知 SHALL 返回 `202` 和可读取的命令状态，相同 `command_id` 同负载重试返回同一结果；未知 choice `404`、同键异负载或竞争 loser `409`、旧 run/超时 choice `410`，不得用 mpsc 入队成功假冒交付成功。pending 卡在提交/回执期间 SHALL 保留处理中状态；失效或失败显式可见。既有人工 plan 门的确认/反馈/终止与 compile recovery 继续可从驾驶舱无 driver 操作，未经人操作不得自动解门。

#### Scenario: HTTP/WS 并发只接收一个回答

- **WHEN** 两个客户端对同一 run 的同一 choice 通过 REST/WS 竞争答复
- **THEN** 仅首个成功认领的回复可交付一次；败者获得明确冲突，不改变已交付的 answers

#### Scenario: 完整多问题答案与真实回执

- **WHEN** 用户从驾驶舱提交含多个 question 的 answers，bridge 等待者实际解析并接收
- **THEN** 收到 `200` 且 answers 不丢字段；如果仅入队或回执仍未知则只可返回 `202`，不会提前删除待处理卡

#### Scenario: 旧 run 与重启后的过期选择不可冒答

- **WHEN** 用户以旧 `expected_run_id` 或进程重启前已失去等待者的 choice 回复
- **THEN** 收到 `410` 或显式失效状态，不把答复路由给新 run；用户可以看到当前可处理的人工恢复路径

#### Scenario: 未经授权的 observer 写入仍拒绝

- **WHEN** observer WS 尝试发送 choice/门/恢复命令
- **THEN** 按既有只读契约拒绝；合法驾驶舱动作须走有授权核验的 REST/应用命令入口

### Requirement: 编码 amendment 与恢复不承重于 live socket（REQ-WIGA-06）

对已授权后台运行的单 target coding，业务 amendment 应用/恢复 SHALL 在无 live coding socket 时继续推进；观察事件送达 SHALL 独立记录真实投递事实并允许重连后按 durable 事实补读，MUST NOT 因没有订阅者而假写 Delivered，也不得以假 socket 代替真实交付。coding choice、amendment 和 runner 恢复 SHALL 不依赖用户打开 Coding Workspace；人工处理门仍停等人操作，不得自动确认。

#### Scenario: 没有 coding 页面仍可应用 amendment

- **WHEN** 所有 coding WS 都已关闭，已获人工批准的 plan amendment 需要应用与恢复
- **THEN** 业务状态可继续到相应 durable 检查点，真实事件未投递则保持未送达并供重连补读，不以 `plan_amendment_coding_socket_unavailable` 拦截业务推进

#### Scenario: 观察面慢或重连不改变业务事实

- **WHEN** coding observer 迟滞/断连后重订阅，期间 amendment 已应用或 runner 已恢复
- **THEN** observer 从 durable 状态获取相同 amendment/执行结果；投递标记反映实际交付，不重复应用 amendment、不因观察端反压改写业务终态

### Requirement: 计划确认及编码执行结束的信息通知（REQ-WIGA-07）

驾驶舱 SHALL 从 durable 成功出版/compile 且 session Confirmed 的事实派生一次「plan 已确认」信息；人点击 approve、人工门关闭、compile 失败/恢复态或任意 compile 节点 Completed 均不足以发通知。coding 执行结束的通知 SHALL 在绑定 attempt durable `status=WaitingForHuman ∧ stage=FinalConfirm` 且最终 readiness 快照完整时发出，文案明确「编码执行完成，待最终确认」并引导人进入 Coding Workspace；`status=Completed` 是之后由人工 Final Confirm 造成的实际终态，不把等待态冒称「整组已交付」。两种信息 SHALL 有稳定事实 key 和 durable occurred_at，TTL 不因读取/刷新续期；不计入待处理数、批量确认或危险操作。系统 SHALL 提供有界近期完成读取，能补回前端 observer 最近 K 个会话以外的完成事实；首次加载仅展示近期历史、不得批量弹 toast/系统通知，新观察到的事实按 key 去重提醒，浏览器完全关闭期间不承诺 OS 推送。

#### Scenario: compile 失败不报成功

- **WHEN** 人批准后编译失败、进入 recovery 或 Confirmed 出版未落盘
- **THEN** 不生成 plan confirmed 信息，驾驶舱仅呈现需要处理的恢复/门卡

#### Scenario: 执行完成等待最终确认即通知

- **WHEN** 单 attempt 已到 WaitingForHuman/FinalConfirm 且 readiness 快照完整，但人尚未点击 Final Confirm
- **THEN** 驾驶舱出现一次「待最终确认」信息与 Coding Workspace 下钻，待处理计数不因信息条目增加；系统不代替人确认

#### Scenario: 真正终态与执行结束有别

- **WHEN** 人随后在 Coding Workspace 完成 Final Confirm，使 attempt.status=Completed
- **THEN** 信息可更新为已最终确认或由独立终态事实显示，原「待最终确认」不再误导；仅满足现有 group 聚合全交付判据时才可称「整组已交付」

#### Scenario: 补读乱序与刷新不刷屏

- **WHEN** 首次打开驾驶舱补读 K 窗口外近期事实，随后反复刷新、事件乱序或重连
- **THEN** 历史信息可见但首次不集中弹通知；同一稳定事实只提醒一次，TTL 从原 occurred_at 计算且不续期，待处理计数仍仅统计可操作卡壳

### Requirement: 单一归属、进度下钻与非 enrolled 零回归（REQ-WIGA-08）

会话及 observer 投影 SHALL 从 durable enrollment 的精确绑定派生 `automation:{owner, enrollment_id, policy_revision, enabled}`；仅仍绑定且授权的会话 owner=server，前端 `useCockpitAutopilot` SHALL 在归属未知时先停等、owner=server 时不再发送 advance，非 enrolled 会话继续既有 client 行为。WIG 入口 SHALL 显示既有编码仪表盘进度并可点入 Coding Workspace 实时查看；后台运行不要求页面持续流式渲染。非 enrolled 的手工业务、门禁、错误与恢复语义 SHALL 不变；源 story/design 人工流程及 Final Confirm SHALL 保持人工，自动化只改变已授权流程的执行发起者。

#### Scenario: server 归属前端退位

- **WHEN** enrolled 会话由 durable 投影确认 owner=server 或首帧尚未知归属
- **THEN** 前端不会抢发 advance，归属/修订变化时清理旧的待发意图；服务端 advance 幂等为已发命令兜底

#### Scenario: 从入口查看后台进度

- **WHEN** 后台 plan/coding 在没有打开执行页面的情况下推进，用户进入 WIG 入口
- **THEN** 可看到当前编码进度并经按钮进入 Coding Workspace 看实时执行/历史重放，无须保留此前的流式页面

#### Scenario: 两模式真实链对照

- **WHEN** 同一业务分别选择不自动化与单 target 自动化完成两条真实链
- **THEN** 手动链既有操作、门禁/错误/恢复与 Final Confirm 不变；自动链只在人工选择/计划门/choice/recovery/Final Confirm 停等，所有其他阶段无需保持页面打开
