# work-item-group-autopilot Specification

## Purpose

在 story/design 仍由人工生成和确认的前提下，为显式选定自动化且仅涉及单一 logical repository 的 issue 提供不依赖页面/driver 连接的 Work Item Plan→Work Item Group coding 推进；遇到需要人的选择、计划门、编译恢复与最终确认时始终停等，由驾驶舱呈现可操作入口。未选自动化的既有流程保持原样。

## Requirements

### Requirement: 人工 design 确认后的显式授权与范围（REQ-WIGA-01）

系统 SHALL 保持 story/design 的生成和确认由人完成，并仅在 design 经人工确认后提供自动化模式选择；默认 SHALL 为不自动化。自动化 enrollment SHALL 将 target、plan、session、source、provider 和绑定版本作为同一条精确身份合同持久化；target SHALL 明确区分单仓（Single-Repository）与逻辑代码库载体，且 SHALL 是 enrollment 载体身份的唯一权威：单仓 target MUST 携带与 issue 记录所属物理仓一致的真实 repository 身份，逻辑代码库 target MUST 同时携带 logical codebase 与 logical repository 身份；单仓 enrollment MUST NOT 携带或生成任何 logical repository 替身身份。用户执行“重新绑定／换代”时 MUST 明确提供目标 plan/session、source、target 和 provider，并携带当前 expected binding/policy version。成功操作 SHALL 写入新的绑定版本并使后续动作只接受当前版本；旧绑定、旧 child 与旧代事件 SHALL 保持只读可追溯。系统 MUST NOT 按最新 session、旧 issue 扫描或后台失败发现猜测接管，也 MUST NOT 将 Manual 已认领 attempt 升级为自动。未选自动化、旧数据缺少 enrollment、或单 target 身份不明确时 SHALL 保持既有手动流程；零/多 target、跨载体或身份不一致 SHALL fail-closed 拒绝自动授权。

#### Scenario: design 确认前不出现自动授权

- **WHEN** story 或 design 尚未由人确认
- **THEN** 不创建 enrollment、不自动准备 plan，原有人工生成与确认入口继续可用

#### Scenario: 默认不自动化

- **WHEN** 用户在 design 确认后选择不自动化或旧 issue 不含 enrollment
- **THEN** plan 准备、生成、advance 和 coding 启动仍按原有手动行为执行，不因自动化补偿扫描产生动作

#### Scenario: 单 target 精确绑定及重复提交

- **WHEN** 已确认 source 只对应一个精确 target（一个单仓物理仓，或逻辑代码库选择中的一个 logical repository），用户用相同选择键重复开启自动化
- **THEN** 系统仅返回同一 enrollment、同一绑定 plan/session 与策略修订；不同 source 或 provider/options 的冲突提交明确拒绝且不改绑

#### Scenario: 单仓 target 可被授权

- **WHEN** 单仓 issue 的记录指向一个已登记的物理仓，用户在 design 确认后选择自动化并提交服务端投影的单仓 target
- **THEN** 系统创建 enrollment，其 target 为该物理仓身份、不含任何 logical repository 身份，绑定版本 1 建立且后续自动链可以启动

#### Scenario: 错仓或跨载体 target 被拒

- **WHEN** Enable 携带的单仓 repository 身份与 issue 记录所属物理仓不一致、该物理仓不存在，或对单仓 issue 提交逻辑代码库 target、对逻辑代码库 issue 提交单仓 target
- **THEN** 系统返回 422 与需重新选择目标的提示，不写 enrollment、不启动 provider

#### Scenario: 多 target 不被自动纳入

- **WHEN** 已确认 source 或冻结的 plan/attempt 涉及多个目标仓库、目标不明确或与 enrollment target 不符
- **THEN** 系统拒绝自动授权或自动启动，绝不循环启动多个 attempt；既有逐 target 手工准入仍可用

#### Scenario: 显式重绑创建新绑定版本

- **WHEN** 当前 enrollment 处于等待恢复且用户提交匹配 expected version 的 plan、session、source、target、provider
- **THEN** 系统写入新的绑定版本，返回当前 enrollment 身份与下一步操作；旧绑定保持只读，后续编排只消费新版本

#### Scenario: 绑定版本过期拒绝

- **WHEN** 用户以过期 expected binding/policy version 或不匹配的 plan/session/target 提交重绑
- **THEN** 系统返回版本冲突/需刷新错误，不覆盖当前绑定、不启动 provider、不改变旧代历史

#### Scenario: 同一命令重放幂等

- **WHEN** 客户端以相同 command_id 和相同绑定负载重复提交同一重绑/换代操作
- **THEN** 系统返回首次操作结果，不创建第二绑定版本、不重复触发后续编排；同一命令异负载被拒绝

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

对仍有效且精确绑定的单 target enrollment，服务端 SHALL 从 durable 状态重建下一动作：至多幂等准备一次 plan/session、至多启动一次对应生成、等待人处理 choice 和人工计划门、等待既有 compile/publication 成功、以独立 advance 推进到 Ready，再通过共用 StartCoding 命令启动该 attempt。服务端 SHALL 在每次 restart、recover 或 new advance 前重新读取当前绑定与租约/attempt claim。编排事件仅作为唤醒；无页面或没有任何 WS 连接时也 SHALL 继续安全动作及恢复。门关闭、approve 点击、compile 节点结束或 advance_completed 均不得被误判为 Confirmed、coding 完成或 provider 启动。租约冲突时，活跃持有者 SHALL 使系统通知用户等待；已死亡持有者仅 SHALL 展示“确认接管”操作，用户确认后才可继续；活性未知 SHALL 保持等待，不得自动抢占。所有恢复动作 SHALL 通过既有 advance/runner/初始化链继续，不能以通知投递成功冒充业务成功。

#### Scenario: 从 design 确认自动生成且停在人工门

- **WHEN** 单 target design 经人确认后选择自动化，用户关闭工作区与 driver 连接
- **THEN** 服务端准备唯一 plan/session、启动一次生成；到 choice/人工门时停等驾驶舱操作，不自动回答或代批准

#### Scenario: 确认后独立推进编码

- **WHEN** 人批准了绑定 plan 且既有 compile/publication durable 成功，enrollment 仍授权
- **THEN** 服务端请求独立 advance 到单 attempt Ready，随后经 StartCoding 发起 coding；advance 自身不启动 provider

#### Scenario: 失败或漏唤醒恢复

- **WHEN** 服务重启、事件漏投、计划创建已成功但绑定未写、或动作在持久检查点间中断
- **THEN** 后续扫描/补偿重用同一 plan/session/action 身份，已认领的启动只按权威事实恢复或进入显式人工分诊，不创建重复 plan、尝试或并行 provider run

#### Scenario: 活跃租约冲突停等

- **WHEN** restart、recover 或 new advance 重新读取到仍有活跃 owner/attempt
- **THEN** 系统保留当前 durable 状态，向驾驶舱和系统通知“请等待/已有运行”及绑定身份，不抢占、不启动第二 provider

#### Scenario: 死亡持有者经确认接管

- **WHEN** 重新读取确认旧 owner 已终态或已超过活动窗口且用户提交匹配当前版本的接管命令
- **THEN** 系统原子记录接管结果并继续原 enrollment 的安全恢复；旧 owner 的迟到写入被拒

#### Scenario: 活性未知保持等待

- **WHEN** lease/attempt claim 既不能证明活跃也不能证明死亡
- **THEN** 系统通知需要人工确认/重新绑定并保持停等，不自动 takeover、不盲目重启外部 provider

### Requirement: StartCoding 单入口与 per-attempt 单发（REQ-WIGA-04）

人工 WS 命令和 enrolled 编排器 SHALL 经同一 typed StartCoding 准入服务；enrolled 编排器在发起独立 StartCoding 或 advance 相关动作前 MUST 核对当前绑定版本、精确 target 与 attempt 身份。绑定读取失败、过期或不匹配 SHALL fail-closed。该核对不得创建 coding provider 的第二启动入口，人工门和现有 StartCoding 单入口语义不变。

#### Scenario: 手工与后台同时请求同一 attempt

- **WHEN** 人工 StartCoding 与已授权的自动 StartCoding 同时进入同一 attempt
- **THEN** 最多一个请求认领首启，另一个幂等命中该 attempt 的原状态，不出现两个 runner/provider

#### Scenario: Ready 前与身份不匹配均拒绝

- **WHEN** attempt 未被该 plan 的 durable advance 绑定置 Ready、绑定不可读、或 target 与 enrollment 不一致
- **THEN** 自动请求零启动；SC Ready 前请求复用 SC_CODING_REQUIRES_ADVANCE，禁止直接调用 runner 绕过守卫

#### Scenario: 崩溃后不盲目二次启动

- **WHEN** 启动意图已持久化但 runner 是否触达外部 provider 不明，服务重启并补偿该 attempt
- **THEN** 服务只能按单发检查点与既有恢复协议重用原启动或进入明确的人工恢复态，不重新发一条首启命令

#### Scenario: 迟到旧代回执不改变当前 attempt

- **WHEN** 旧绑定版本的 advance/start 回执在用户换代后到达
- **THEN** 系统拒绝该回执且不改变当前代绑定、attempt 或 provider 状态

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

### Requirement: Enable 前完整角色链预检（REQ-WIGA-C5-PREFLIGHT）

自动化 target 投影与 Enable SHALL 在写入 enrollment 前，按该 enrollment 在真实链上将启动的角色逐一核验：plan author、plan reviewer、coder、code reviewer 与 internal reviewer，每个角色的 provider SHALL 按真实启动路径的同一派生规则确定，而非另设独立配置。逻辑代码库载体下，任一角色的 provider 属于 provider gateway 确定性不支持的组合（无 gateway 启动能力，或该 provider 被 gateway 路由策略静态阻断）时，系统 SHALL 以 HTTP 422 与稳定错误码 `automation_role_chain_unsupported` 拒绝，错误 SHALL 列出每个不支持的角色、provider 与原因，并提示用户更换 provider 配置或更换目标；拒绝 MUST NOT 写入或修改 enrollment、MUST NOT 启动任何 provider。单仓载体 MUST NOT 施加逻辑代码库 gateway 约束：provider 可用性校验通过的合法组合 SHALL 被授权。预检只判定确定性静态不支持，MUST NOT 探活或把账号池/网关的动态不可用判为静态不支持；Enable 之后运行期的动态不可用（503）SHALL 进入既有通知停等，系统 MUST NOT 自动切换 provider 或盲目重试。GET 投影与 Enable SHALL 对同一输入给出同一判定。

#### Scenario: 逻辑代码库不支持的 coder 在授权前被拒

- **WHEN** 逻辑代码库单 target issue 的 enrollment options 使 coder 派生为 gateway 无启动能力的 provider（如 Pi），而 reviewer 合法
- **THEN** 投影与 Enable 均返回 422 `automation_role_chain_unsupported`，错误指明 coder 角色与 provider，enrollment 未写入，provider 启动计数为 0

#### Scenario: 逻辑代码库被路由阻断的 provider 被拒

- **WHEN** 逻辑代码库 issue 任一角色派生为被 gateway 路由策略静态阻断的 provider
- **THEN** 系统在授权前返回 422 并列出该角色与阻断原因，不写 enrollment

#### Scenario: 单仓合法组合不误拒

- **WHEN** 单仓 issue 的 author/coder/reviewer 派生为逻辑代码库 gateway 不支持但本机可用的 provider
- **THEN** 投影与 Enable 成功，enrollment 写入，后续 plan 与 coding 按单仓直连路径启动

#### Scenario: 运行期动态不可用进入停等

- **WHEN** Enable 已成功，之后某角色启动时账号池或网关返回 503
- **THEN** 系统保留该失败事实并通过既有通知停等，展示恢复后重试操作；不自动切换 provider、不盲重、不回滚 enrollment

### Requirement: 单仓载体的无页面自动全链（REQ-WIGA-C5-CHAIN）

对单仓 target 的有效 enrollment，服务端 SHALL 沿单仓既有路径完成与逻辑代码库同等的自动链：幂等准备唯一 plan/session、启动一次生成、在 choice 与人工计划门停等、确认后独立 advance 到唯一 attempt Ready、经共用 typed StartCoding 单发首启、coding 运行至 `WaitingForHuman ∧ FinalConfirm` 并发出“待最终确认”信息；关闭所有页面与 coding socket 不影响推进。单仓链 MUST NOT 注入逻辑代码库 gateway、MUST NOT 创建逻辑代码库 manifest/selection/snapshot。prepare、Enrolled advance 与 StartCoding SHALL 各自复核 enrollment target 与当前权威载体一致：单仓 target 的 repository 身份 MUST 等于 issue 所属物理仓、plan 的全部工作项 MUST 无 logical target 归属、attempt MUST 无逻辑代码库 target 快照；逻辑代码库 target 的 attempt 唯一 target MUST 等于 enrollment 的 logical repository。任一不符 SHALL fail-closed 零副作用，并以既有等待项呈现需重新绑定。会话 owner 投影、驾驶舱等待项、plan 已确认与待最终确认信息 SHALL 对两种载体使用同一派生与展示，并显示当前 target 身份。

#### Scenario: 单仓关闭页面后自动跑到最终确认

- **WHEN** 单仓 issue 经人确认 design 后明确 Enable 自动化，用户随即关闭全部页面与 socket，并仅在 choice/计划门出现时从驾驶舱处理
- **THEN** 系统自动生成 plan、在人工门停等、批准后自动 advance 到 Ready 并 typed 首启，coding 完成后停在 FinalConfirm 并出现一次“待最终确认”信息；全程 provider 首启恰一次

#### Scenario: 单仓链拒绝错仓与旧绑定

- **WHEN** 单仓 enrollment 的 target 与 issue 当前所属物理仓不一致，或 advance/StartCoding 回执携带旧绑定版本
- **THEN** 该动作零启动、零写入 attempt，驾驶舱出现需重新绑定的等待项，当前绑定不被覆盖

#### Scenario: 逻辑代码库 attempt target 与 enrollment 不符

- **WHEN** 逻辑代码库 enrollment 的 plan 唯一 target 或 attempt 快照指向与 enrollment target 不同的 logical repository
- **THEN** Enrolled advance 或自动 StartCoding fail-closed 拒绝，不启动 provider

#### Scenario: 手动与关闭模式不被接管

- **WHEN** 单仓 issue 选择手动或关闭 enrollment 后后台补偿扫描运行
- **THEN** 补偿扫描不准备 plan、不 advance、不启动 coding，手动链行为与本 change 前一致
