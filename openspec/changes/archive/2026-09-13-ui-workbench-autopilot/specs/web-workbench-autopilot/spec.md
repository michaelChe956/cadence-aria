## Purpose

把 aria 工作区引擎的对话侧 web 前端从「多面板控制台」变为**自动执行驾驶舱**：系统自动跑全程，人只在四类卡壳点（门禁 / 分诊 / stopped / 硬错）被叫醒，全部处理动作在界面就地完成、全程不需要开命令行。本 capability 只覆盖前端呈现层与操作层（`web/src/`），引擎零改动。

## ADDED Requirements

### Requirement: 三区驾驶舱信息架构与四类卡壳条目（REQ-UI37-01）

驾驶舱 SHALL 以三区组织对话侧工作区，且三区职责严格互斥：① **待处理收件箱**（唯一需要人的地方）、② **自动执行流**（只读，展示进行中的执行单元）、③ **下钻对话流**（点任一条进详情）。② 区 MUST NOT 提供任何输入控件。收件箱 MUST 覆盖四类卡壳点——**门禁等待**、**分诊**、**stopped 接管**、**硬错**——每条目 MUST 带上下文摘要与就地动作（不跳页、不弹窗、不命令行）。**分诊 SHALL 是门禁条目的亚型，同会话同门只投影一条**（不单独设动作面，复用所属门禁条目的动作）。新条目 MUST 置顶并按严重度排序；条目存续契约 SHALL 为「有 gate 投影即存在、门闭环即收敛」——turn 三终态只更新子状态、不移除条目，唯一移除点是门闭环（或重建口径下会话状态投影已显示该门 Confirmed）；MUST NOT 做「已完成」堆积。

#### Scenario: 三区职责互斥

- **WHEN** 用户打开驾驶舱查看某会话
- **THEN** 界面呈现①收件箱、②执行流、③对话流三区，②区内不存在任何输入控件或动作按钮，人工入口只出现在①

#### Scenario: 四类卡壳点各生成条目

- **WHEN** 引擎分别进入门开（含 `needs_human` 分诊路由到 `human_confirm` 门）、`stopped_needs_human`、出向 `error` / `protocol_error` 状态
- **THEN** 收件箱分别出现门禁等待条目、stopped 接管条目、硬错条目；分诊不产生独立条目，而是作为所属门禁条目的同一性呈现（同会话同门只有一条）

#### Scenario: 条目收敛而非堆积

- **WHEN** 该门收到门闭环事件（`decision = confirm` / `terminate`），或重建后会话状态投影显示该门已 Confirmed
- **THEN** 该条目从收件箱移除，且收件箱内不出现「已完成」历史堆积

#### Scenario: turn 终态只更新子状态

- **WHEN** 收到 `human_gate_turn_completed` / `human_gate_turn_failed` / `human_gate_busy`
- **THEN** 条目分别转入「待确认」/「失败（保持门开可见）」/「处理中」子状态，条目仍然留在收件箱，门态不被 `human_gate_busy` 改变

#### Scenario: 硬错条目不提供接管

- **WHEN** 收件箱中存在硬错条目
- **THEN** 该条目 MUST NOT 出现「接管」动作，只提供重试 / 终止

### Requirement: typed 门禁与推进事件面的完整消费（REQ-UI37-02）

前端 SHALL 消费阶段 3 已锁定的 typed 门禁与推进事件面，共 **6 条新增 + 1 条既有修复**：`human_gate_turn_open`（字段 `turn_id` / `command_id` / `remaining_budget`）、`human_gate_turn_completed`（`turn_id` / `artifact_ref`）、`human_gate_turn_failed`（`turn_id` / `failure_class` / `message`）、`human_gate_busy`（`turn_id`）、`human_gate_closed`（`decision` / `stage`）、`advance_completed`（`command_id` / `attempt_id` / `workspace_entry`）、`advance_rejected`（`command_id` / `code` / `reason`）。每条事件 MUST 映射到收件箱 / 执行流 / 对话流投影与门禁三态（**门开** warning / **待确认** primary / **已过** success）。`human_gate_closed` MUST NOT 再被静默丢弃（现状为已 typed 但无处理分支）。事件分发的兜底分支 MUST 宽容：未识别的事件类型 MUST NOT 抛错中断，但 MUST 写入诊断记录。前端 SHALL NOT 要求后端新增事件或字段。

#### Scenario: 6 条新增事件逐条生效

- **WHEN** 前端依次收到 `human_gate_turn_open`、`human_gate_turn_completed`、`human_gate_turn_failed`、`human_gate_busy`、`advance_completed`、`advance_rejected`
- **THEN** 门禁条目相应建立/转待确认/转失败/转处理中，推进被正确判定为成功或被拒并回落收件箱；无一条被丢弃

#### Scenario: human_gate_closed 不再静默丢弃

- **WHEN** 前端收到 `human_gate_closed`（`decision = confirm` / `terminate`）
- **THEN** 对应门禁条目按 `decision` 收敛并移出收件箱，`stage` 用于归属阶段；该事件 MUST NOT 出现「到达到即无任何可观察效果」的情形

#### Scenario: 门禁三态迁移

- **WHEN** 同一门依次经历门开 → turn 成功 → 门闭环
- **THEN** 呈现依次为「门开」（warning、脉冲、门开自计时）→「待确认」（primary）→「已过」（条目移除），三态可由既有事件面唯一推出

#### Scenario: 未识别事件宽容且留痕

- **WHEN** 前端收到一个当前未识别的出向事件类型
- **THEN** 分发 MUST NOT 抛错、不中断其余事件处理，且该未识别类型被写入诊断记录

#### Scenario: typed turn 形态门在重连重建后仍可见

- **WHEN** 会话处于 SC typed 门（`human_gate_turn_open` 形态）时断线重连并完成重建
- **THEN** 该门的收件箱条目与门卡仍然存在（重建条件不以 legacy `human_confirm` 阶段为唯一判据），且四种门开原因全集（含 `verification_new_findings`）均可正确呈现

### Requirement: 双向命令面与门面分流（REQ-UI37-03）

前端 SHALL 支持发送 `human_gate_feedback` 与 `advance` 两条入向命令（后端已有、前端此前缺失）。门禁动作 MUST 按引擎的**两套门面分流**，MUST NOT 混用：

- **SC typed 门（`flowKind = single_candidate`）**：反馈 MUST 走 `HumanGateFeedback { command_id, feedback }`（每次提交消耗一次 manual-repair 预算）；确认 / 终止 MUST 走 `HumanConfirm` 关闭（`decision = Confirm` / `Terminate`）。
- **legacy `human_confirm` 门（flowKind 非 single_candidate）**：确认 / `RequestChange` / 终止三取值 SHALL 均走 `HumanConfirm`。

对 typed 门发起 `RequestChange` 形态的反馈 MUST NOT 存在可达入口（引擎会明确拒绝该动作）。模式切换 MUST 有明确标签，MUST NOT 用同一输入框隐式推断意图。危险动作 SHALL 具备二次确认态（点击后进入「再点一次确认」，超时自动回落）。门禁动作旁 MUST 常显来自引擎的剩余修复预算。

#### Scenario: SC typed 门提交反馈

- **WHEN** 用户在 SC typed 门卡内提交文字反馈
- **THEN** 前端发送 `HumanGateFeedback`（携带 `command_id`），此后该门的剩余修复预算按引擎事件递减并在界面上反映

#### Scenario: SC typed 门不提供 request-change 反馈

- **WHEN** 呈现 SC typed 门的门卡
- **THEN** 界面只提供反馈 / 确认 / 终止三种入口，不存在发送 `HumanConfirm{RequestChange}` 的路径

#### Scenario: legacy 门三取值齐备

- **WHEN** 呈现 legacy `human_confirm` 门
- **THEN** 界面提供确认 / `RequestChange` / 终止三种入口，三者均经 `HumanConfirm` 发出

#### Scenario: 危险动作二次确认

- **WHEN** 用户点击终止等危险动作
- **THEN** 该动作进入「再点一次确认」态，二次点击才发送命令，等待超时后自动回落到未确认态

#### Scenario: 预算余量常显

- **WHEN** 某门处于门开状态且引擎发布了剩余修复预算
- **THEN** 门禁动作旁常显该预算余量，且在每次消耗后同步更新

### Requirement: 命令幂等与去重键（REQ-UI37-04）

门禁侧去重键 SHALL 为 `turn_id`；推进侧去重键 SHALL 为 `command_id`。前端生成的门禁 / 推进命令 `command_id` MUST 使用 `crypto.randomUUID()`，**每会话每命令只生成一次**——同一用户动作在动作发起时生成，重试 / 重连重放 MUST 复用同一 `command_id`，MUST NOT 重新生成。同一 `turn_id` 或 `command_id` 的重放 SHALL 被视为同一事件幂等消费：MUST NOT 新建条目、MUST NOT 重复提醒、MUST NOT 重复扣预算。断线重连后的重建 SHALL 从持久化会话状态重放恢复去重集（MUST NOT 丢弃），确保已处理的门与已终结的推进不会在重连后被当作新事件重复提醒。

#### Scenario: 同 turn_id 重放幂等

- **WHEN** 引擎因重放向同一 `turn_id` 重发 `human_gate_turn_open`
- **THEN** 不新建收件箱条目、不重复触发提醒、不重复计入预算

#### Scenario: 重连重放复用同一 command_id

- **WHEN** 用户动作已发出后断线重连，前端需要重发该命令
- **THEN** 复用原 `command_id`（重新生成 MUST NOT 发生），引擎侧不产生第二次动作

#### Scenario: 重连后去重集恢复

- **WHEN** 断线重连完成重建
- **THEN** `turn_id` / `command_id` 去重集从持久化会话状态恢复；此前已终结的推进 MUST NOT 被历史 `advance_rejected` 重放再次回落收件箱或触发二次提醒

### Requirement: autopilot 自动推进（REQ-UI37-05）

autopilot SHALL 每会话默认开启，**唯一自动推进锚点是观测到 durable 的 `Confirmed`**（门的 close 完成事件，或会话状态投影显示该门已 Confirmed）。约束：

- **`HumanGateFeedback` 之后**：只等修复轮结果（下一次门开或 Confirmed 事件），**MUST NOT 自动发 `Advance`**。
- **compile 失败导致门重开**：MUST 在收件箱呈现、**MUST NOT 自动重试**（不自动重发 compile 命令、不自动 advance）。
- **推进被拒**（`advance_rejected`）：MUST 回落收件箱并**停止该会话的自动推进**。
- **组内单元推进**：引擎已自动，前端 MUST NOT 代发。
- **去抖**：以「观测到 Confirmed」为唯一锚点并对同一会话同一 entry 去抖——在未观测到该 entry 的 `advance_completed` / `advance_rejected` 前只发一次，同 `command_id` 复用（重连不重发）。
- **`human_gate_busy`**：MUST 立即放弃本次自动推进，不清空、不重试、不排队，等该 turn 的下一次门开 / 终态事件再重新判定。
- **停点**：默认每次人工门前必停；停点集合可配，且停点只影响「是否自动发 `Advance`」，MUST NOT 改变引擎门禁语义。MUST NOT 提供「无停点全自动」默认。

#### Scenario: 观测到 Confirmed 自动推进

- **WHEN** 前端观测到某门 durable 的 `Confirmed`
- **THEN** 前端自动发出一次 `Advance`（携带新生成的 `command_id`），人无需手工点「推进」

#### Scenario: 反馈之后不推进

- **WHEN** 用户刚提交 `HumanGateFeedback` 且尚未观测到新的 Confirmed
- **THEN** 前端 MUST NOT 发出 `Advance`，只呈现修复轮进展

#### Scenario: compile 失败不重试

- **WHEN** 因 compile 失败导致门重开（恢复动作场景）
- **THEN** 该情形回收件箱呈现，前端 MUST NOT 自动重发 compile 命令，也 MUST NOT 自动 advance

#### Scenario: 推进被拒停止自动推进

- **WHEN** 收到 `advance_rejected`
- **THEN** 该推进按 `command_id` 去重回落收件箱，且该会话的自动推进停止

#### Scenario: 同一 entry 只推进一次

- **WHEN** 自动推进已发出但尚未观测到该 entry 的 `advance_completed` / `advance_rejected`
- **THEN** 前端 MUST NOT 对同一 entry 再发第二次 `Advance`；重连后也不重发

#### Scenario: 引擎在途 turn 时放弃推进

- **WHEN** 收到 `human_gate_busy`
- **THEN** 本次自动推进被放弃（不清空、不重试、不排队），该会话等下一次门开 / 终态事件再重新判定

#### Scenario: 停点配置生效

- **WHEN** 用户在设置中将停点集合调整为「仅硬错停」或「仅 stopped 停」
- **THEN** autopilot 仅在这些停点处不自动推进，其余锚点行为不变，且引擎门禁语义不受影响

### Requirement: 四层递进卡壳提醒（REQ-UI37-06）

卡壳提醒 SHALL 为四层递进、层层叠加，且**任一层 MUST NOT 依赖用户主动查看**：

- **L1 页面内**：收件箱条目置顶 + 脉冲动画（直到被查看）+ 新卡壳到达弹 toast（toast 为本 capability 从零新建，不替代 L2 常驻警示条）。
- **L2 全局常驻**：顶部 sticky 红底白字警示条，摘要 + 「去处理」入口；**任何页面可见**（含 `/image-create`）；存续由「未处理卡壳计数 > 0」驱动，MUST NOT 定时自动隐藏、MUST NOT 可手动关闭。
- **L3 浏览器级**：浏览器标题呈现 `🔴待处理×N`（该 emoji 是全仓「图标一律 SVG、零 emoji」硬规则的唯一例外，纯文本标记，且 MUST 可经设置关闭）+ favicon 数量角标 + 系统通知（Web Notifications，需授权、可授权关闭，**卡壳 30s 未处理才触发**）；权限被拒 MUST 降级为 L1 + L2 且**不因权限被拒而丢提醒**。
- **L4 升级（双触发）**：**触发①** 剩余修复轮次进入 `≤ 1` 即转橙（warning 语义）并按次数梯度提升重复提醒频率；**触发②** 门开时长前端自计时，默认 **10min** 无人处理即升级一级、阈值可配。L4 MUST NOT 使用「超时前 5min / 预算 < 20%」口径。升级只是**提醒升级**，MUST NOT 改变任何引擎语义（不自动终止、不自动代做决策）。

L2 / L3 的计数 SHALL 为跨会话聚合量（见 REQ-UI37-07），MUST NOT 是单会话投影。多门同时打开时收件箱按严重度排序、标题/角标只显示计数、L4 重复通知按设置的节流间隔限流。

#### Scenario: L1 置顶与脉冲

- **WHEN** 新卡壳条目到达
- **THEN** 该条目置顶、持续脉冲直到被查看，并弹出 toast

#### Scenario: L1 在减弱动效下仍醒目

- **WHEN** 用户系统启用 `prefers-reduced-motion`
- **THEN** 脉冲动画降级为静态高亮边框（不使用动效但保留醒目度），MUST NOT 用 `!important` 绕过该降级

#### Scenario: L2 任何页面可见且不可关闭

- **WHEN** 未处理卡壳计数 > 0，用户在 `/image-create` 或编码侧页面
- **THEN** 顶部 sticky 红底白字警示条均可见并提供「去处理」入口；用户无法手动关闭它，计数归零后才消失

#### Scenario: L3 通知延迟与降级

- **WHEN** 某卡壳到达且 30s 内无人处理
- **THEN** 触发系统通知；若浏览器拒绝通知权限，则提醒降级为 L1 + L2 且不丢失提醒，设置页给出恢复指引

#### Scenario: L3 标题标记可关闭

- **WHEN** 用户在设置中关闭标题 emoji 标记
- **THEN** 浏览器标题不再呈现 `🔴待处理×N`，favicon 角标与其余提醒层不受影响

#### Scenario: L4 触发① 剩余轮次见底转橙

- **WHEN** 引擎发布的剩余修复预算进入 `≤ 1`
- **THEN** 该门禁条目转橙并按次数梯度加密重复提醒

#### Scenario: L4 触发② 门开自计时升级

- **WHEN** 门开事件到达后经过设置的阈值时长（默认 10min）仍无人处理
- **THEN** 提醒升级一级（转橙 + 重复提醒 + 提示音按设置项触发，默认关）；阈值可在设置中修改，计时纯前端、阈值不进入引擎

#### Scenario: 提醒过载限流

- **WHEN** 多个门同时打开
- **THEN** 收件箱按严重度排序，标题与角标只显示计数，L4 重复通知按设置页节流间隔限流

### Requirement: 跨会话聚合与降级语义（REQ-UI37-07）

L2 全局警示条与 L3 标题 / favicon 的待处理计数 SHALL 来自**跨会话聚合**，实现路径固定为**前端多 WS 订阅 watch 集合**（引擎零改动）。会话清单 MUST 经既有 REST 获取（`/api/projects` → 逐项目 `/api/projects/{project_id}/issues` 或全局 `/api/issues` → 逐 issue `GET /api/issues/{issue_id}/lifecycle?project_id=...` → 取 `workspace_sessions[]`）。**watch 集合** SHALL 由活跃会话（`status` 非终态，如 `running` / `stopped_needs_human`，按最近活动排序）构成，watch 前 **K 个，K = 8（可配）**。**全局 sticky / L3 计数源 SHALL = watch 集合的并集，MUST NOT 是全量会话**：K 内的计数精确；**K 外的会话在会话列表中仍可见，但其实时卡壳不进入全局计数**（这是明确的降级语义，MUST NOT 呈现为精确计数）。连接生命周期：已在集合内的会话 MUST 保持连接不重建（避免重连抖动），新入集合的建连、出集合的断开；设置中修改 K MUST 立即重算。MUST NOT 新增后端接口或事件。

#### Scenario: K 内计数精确

- **WHEN** watch 集合内的多个会话各自出现卡壳
- **THEN** L2 计数与 L3 标题 / 角标显示这些会话的并集计数，与收件箱实际待处理数一致

#### Scenario: 超 K 降级语义

- **WHEN** 活跃会话数量超过 K
- **THEN** 前端只 watch 最近活跃的 K 个；超出者仍出现在会话列表（REST 态），但其实时卡壳不进全局计数，界面不声称计数为全量

#### Scenario: 连接生命周期稳定

- **WHEN** 会话清单刷新后集合成员发生变化
- **THEN** 集合内既有会话的连接不被重建；新入集合者建连、出集合者断开

#### Scenario: 修改 K 立即重算

- **WHEN** 用户在设置中修改聚合窗口 K
- **THEN** watch 集合立即按新 K 重算并生效

#### Scenario: 引擎零改动

- **WHEN** 检查本 capability 的跨会话聚合实现
- **THEN** 仅使用既有 REST 与既有 workspace WS 事件面，未新增任何后端接口或事件

### Requirement: 停点接管接线与 409 呈现（REQ-UI37-08）

「接管」入口 SHALL 唯一出现在 **stopped 条目**（`stopped_needs_human` 会话），且 MUST NOT 出现在门禁卡或其他条目。接管 MUST 调用既有 `POST /api/workspace-sessions/{session_id}/takeover`（MUST NOT 新增重复入口），成功后返回的子会话 MUST 接入 ③ 对话流供人继续操作，父会话 MUST NOT 被修改（保持其状态、历史与事件不变）。该 REST 只接受可恢复的 `stopped_needs_human` 会话；其他状态（含 `Failed`）返回 **409 `workspace_session_takeover_not_allowed`** 时，前端 MUST 在条目内联呈现后端 `code` + `reason`，接管按钮 MUST 置灰并附原因文案，MUST NOT 静默失败、MUST NOT 自动重试。

#### Scenario: stopped 条目接管成功

- **WHEN** 用户对 `stopped_needs_human` 会话的收件箱条目点击「接管」
- **THEN** 前端调用既有 takeover REST，返回的子会话进入 ③ 对话流可继续操作，父会话状态与历史不变

#### Scenario: 重复接管返回同一子会话

- **WHEN** 用户对同一会话重复发起接管
- **THEN** 后端返回同一子会话，前端接入同一对话流，不产生第二个子会话

#### Scenario: 409 内联呈现且按钮置灰

- **WHEN** 接管请求返回 409 `workspace_session_takeover_not_allowed`
- **THEN** 条目内联展示后端 `code` 与 `reason`，接管按钮置灰并附原因文案，不发生静默失败与自动重试

#### Scenario: 硬错与门禁卡不提供接管

- **WHEN** 呈现硬错条目或门禁卡
- **THEN** 二者 MUST NOT 出现接管入口

### Requirement: 硬错条目与 protocol_error 的 code 分流（REQ-UI37-09）

硬错条目 SHALL 源出向 `error` / `protocol_error` 事件，携带错误码、原文摘要与最近一次动作，并提供重试 / 终止动作。`protocol_error` 到达时 MUST **先按 `code` 判定归属**：与门禁 / advance 相关的 code（门禁决策拒绝类、`ADVANCE_REPLAY_NOT_READY` / `ADVANCE_REPLAY_INCOMPLETE` 等）MUST 内联回所属条目（同 `turn_id` / `command_id` 的条目）而 MUST NOT 新建硬错条目；只有无法归属的 code 才升为硬错条目。

#### Scenario: 门禁拒绝类错误内联

- **WHEN** 收到与门禁决策相关的 `protocol_error`（可归属到某 `turn_id`）
- **THEN** 该错误内联呈现在所属门禁条目内，收件箱不新增独立硬错条目

#### Scenario: 推进重放拒绝按 command_id 归属

- **WHEN** 重连后收到 `ADVANCE_REPLAY_NOT_READY` / `ADVANCE_REPLAY_INCOMPLETE` 且可关联到某 `command_id`
- **THEN** 错误按该 `command_id` 归属处理（已终结的推进不再回落收件箱、不触发二次提醒）

#### Scenario: 无法归属才升硬错

- **WHEN** `protocol_error` 的 code 无法归属到任何在册条目
- **THEN** 收件箱新增硬错条目并呈现错误码与原文摘要

### Requirement: 设置面板（REQ-UI37-10）

驾驶舱 SHALL 提供设置面板（**对话框形式，MUST NOT 新增路由**），至少可配：提示音（默认**关**）、系统通知（默认**开**，需浏览器授权）、重复通知的升级/节流间隔、L4 门开时长阈值（默认 **10min**）、L4 重复提醒次数梯度（逐级加密）、标题 emoji 标记（默认**开**）、autopilot 停点集合（默认**每次人工门前必停**）、跨会话聚合窗口 K（默认 **8**）。设置变更 MUST 立即生效（含 K 重算 watch 集合）；设置项 MUST NOT 改变引擎语义。

#### Scenario: 设置在对话框内完成

- **WHEN** 用户打开设置
- **THEN** 以驾驶舱内对话框形式呈现，URL / 路由不发生变化

#### Scenario: 默认值符合保守口径

- **WHEN** 用户首次进入设置面板
- **THEN** 提示音为关、系统通知为开、L4 门开时长阈值为 10min、autopilot 停点为每次人工门前必停、K 为 8

#### Scenario: 配置立即生效

- **WHEN** 用户修改任一项设置
- **THEN** 该项立即对提醒 / autopilot / 聚合生效，无需刷新页面

### Requirement: 双轨特性开关与灰度切换（REQ-UI37-11）

新旧形态 SHALL 双轨并存，由**特性开关**切换：开关落点为驾驶舱页面**内部的分支早返回**（MUST NOT 新增路由、MUST NOT 新增独立开关组件），取值持久化于 `localStorage`，**默认值为旧形态（legacy）**。命中新形态取值才渲染三区驾驶舱，否则页面顶部早返回既有形态（旧代码路径原样保留，便于 parity 对比与回滚）。灰度方式 SHALL 为「默认旧形态 → parity 达标后只翻默认值」。路由层断言 MUST NOT 因该开关而改变，MUST NOT 为了让路由测试通过而把默认值翻为新形态。Phase 1 期间新驾驶舱 MUST 只覆盖对话侧，编码侧仍走旧页面。

#### Scenario: 默认仍是旧形态

- **WHEN** 用户在未修改开关的情况下打开对话侧工作区
- **THEN** 呈现既有四块形态（legacy），路由与页面挂载关系不变

#### Scenario: 命中新形态才渲染驾驶舱

- **WHEN** 开关取值为新形态（cockpit）
- **THEN** 页面渲染三区驾驶舱，且旧形态代码路径保持保留以便回滚

#### Scenario: 翻默认不动路由

- **WHEN** parity 达标后把默认值翻为新形态
- **THEN** 只改默认值，不新增/改动路由，路由层既有断言仍然成立

#### Scenario: 编码侧不受影响

- **WHEN** Phase 1 期间用户访问编码侧工作区
- **THEN** 仍走旧编码页面形态，新驾驶舱不接管编码侧

### Requirement: 分阶段交付与双层验收口径（REQ-UI37-12）

交付 SHALL 按 Phase 1a / 1b / 2 / 3 / 4 分阶段推进，**每段可独立验收**，MUST NOT 合并为一次性大验收：

- **Phase 1a**：双向 typed 协议补齐 + **单会话**三区只读 + 事实底座；**不含**全局 shell 与跨会话聚合。
- **Phase 1b**：全局 shell 骨架 + 跨会话聚合（方案 a）+ 就地动作 + autopilot + 四层提醒 + 设置 + takeover 接线。
- **Phase 2**：编码仪表盘。**Phase 3**：计划审批深度（含 DEF-3 吸收）。**Phase 4**：advance / 接管操作收口。

验收 SHALL 分两层且 MUST NOT 混用：**人工验收**只认真实链（轻语料 + `pi` provider，在界面上完成「建会话 → 看计划 → request-change / confirm / advance → 看编码进度 → 看 push」，全程不开命令行），证据形态为人工走查记录（步骤 + 截图）；**自动化替身**（`ARIA_PROVIDER_MODE=fake` + 测试端点注入 `verdict: needs_human` 驱动门开 + 前端单测覆盖三投影与门禁三态）只用于单测与回归，**MUST NOT 代替人工验收**。每 Phase SHALL 产出两份清单：自动化证据清单（三投影期望态、本 Phase 新增事件分支用例、去重键与重连重放幂等、门禁三态迁移）与人工证据清单（真实链走查步骤记录、卡壳提醒层级逐层可观察、全程无命令行）。

#### Scenario: 1a 独立验收「看得到真事实」

- **WHEN** Phase 1a 关闸
- **THEN** 双向 typed 协议补齐后，单会话三区在真实链上投影正确、只读可用；验收不含全局 shell 与跨会话聚合

#### Scenario: 1b 独立验收完整一遍

- **WHEN** Phase 1b 关闸
- **THEN** 全局 shell + 跨会话聚合 + 四类卡壳点就地动作 + autopilot + 四层提醒 + takeover 接线在真实 provider 上跑通，两段各自独立过关

#### Scenario: 自动化替身不代替人工验收

- **WHEN** 某 Phase 的自动化替身用例全部通过但真实链未走通
- **THEN** 该 Phase MUST NOT 被判定为验收通过

#### Scenario: 每 Phase 两清单齐备

- **WHEN** 某 Phase 关闸评审
- **THEN** 自动化证据清单与人工证据清单同时齐备；人工清单含真实链步骤记录与「全程无命令行」的确认

### Requirement: 引擎零改动与外部边界（REQ-UI37-13）

本 capability 的全部 Phase SHALL 只动 `web/src/`，MUST NOT 修改任何后端代码，MUST NOT 新增后端接口 / 事件 / 持久化，MUST NOT 新增前端依赖，MUST NOT 新增 Provider / CI / 持久化评估语料。编码侧 MUST NOT 新增插话入口（coding WS 无插话入口，且 single-candidate Running 阶段拒绝 `UserMessage`，为 fail-closed 约束）。`ImageCreatePage` 的**页面内容** MUST NOT 被修改（仅 L2 警示条与 toast 在其上同样可见）。运行中接管（运行中插话 / 改门禁语义）MUST NOT 在本 capability 内实现；`auto_start_coding` 显式语义 / 多仓 coding / 旧协议退役门 MUST NOT 在本 capability 内实现。长会话渲染 SHALL 虚拟化（只渲染视口内轮次），流式更新 SHALL 按帧合并。

#### Scenario: 后端零改动

- **WHEN** 检查本 capability 的改动面
- **THEN** 全部改动落在 `web/src/` 内，`src/` 无改动，且未新增后端路由、WS 事件或持久化结构

#### Scenario: 不新增前端依赖

- **WHEN** 检查依赖清单
- **THEN** 未新增前端依赖（长会话虚拟化使用既有依赖），未新增第二个组件库或第二套设计 token 体系

#### Scenario: 编码侧不新增插话入口

- **WHEN** 用户在编码侧界面
- **THEN** 不存在新增的插话 / `UserMessage` 发送入口，编码侧仅按既有能力呈现

#### Scenario: ImageCreate 页面内容不动

- **WHEN** 用户访问 `/image-create`
- **THEN** 页面内容与其组件行为不变，仅全局警示条与 toast 在其上可见

#### Scenario: 长会话性能

- **WHEN** 会话轮次很多且 `stream_chunk` 高频到达
- **THEN** 仅渲染视口内轮次，流式更新按帧合并，日志抽取与对话流渲染解耦

### Requirement: 设计系统硬规则（REQ-UI37-14）

UI MUST 采用**增量不换血**：沿用既有 `--aria-*` token、组件类与 `prefers-reduced-motion` 兜底，MUST NOT 引入第二套 token 体系或第二个组件库。新增 token SHALL 只做三组——**对话流**、**拓扑六态**（`running` / `pending` / `blocked` / `done` / `failed` / `awaiting_triage`）、**门禁三态**（门开 warning / 待确认 primary / 已过 success）——且 MUST 从现有语义色系派生、MUST NOT 新增色相（沿用靛主色、橙 CTA、绿/黄/红语义色）。字体与数字：等宽统一 `ui-monospace`，适用于 commit hash / 命令 / 耗时 / 预算计时 / 节点 id / session id；数字统一 `tabular-nums`。硬规则 MUST 全量遵守：**R1** 图标一律 SVG（Lucide）、零 emoji（唯一例外为 L3 标题标记且可关）；**R2** 触达目标 ≥ 44px（含图标按钮）；**R3** 所有可交互元素具备 `focus-visible` 焦点环；**R4** 过渡 150–300ms、不做布局抖动动画；**R5** 新增动画（脉冲 / 闪烁 / 流式光标）在 `prefers-reduced-motion` 下全降级为静态、MUST NOT 用 `!important` 绕过；**R6** 正文与图标对比度 ≥ 4.5:1（警示条红底白字按此校验）。`awaiting_triage` SHALL 是由引擎 `needs_human` 判定投影而来的 **UI 投影态**，MUST NOT 作为独立引擎状态持久化，且同会话同门只投影一条。

#### Scenario: 沿用既有 token

- **WHEN** 实施驾驶舱样式
- **THEN** 既有 `--aria-*` token 值不被修改，新增仅为三组派生 token，且未引入第二套 token 体系或组件库

#### Scenario: 六态节点复用语义色

- **WHEN** 呈现执行单元节点
- **THEN** 六态分别使用主色（running）、弱墨色（pending）、warning（blocked）、success（done）、danger（failed）、warning + 脉冲（awaiting_triage），不出现新色相

#### Scenario: 触达与焦点可达

- **WHEN** 用户以触控或键盘操作任一交互元素
- **THEN** 触达目标不小于 44px，且焦点元素呈现 `focus-visible` 焦点环

#### Scenario: 动效降级

- **WHEN** 用户系统启用 `prefers-reduced-motion`
- **THEN** 脉冲、闪烁、流式光标全部降级为静态，过渡仍在 150–300ms 区间，无 `!important` 绕过

#### Scenario: 对比度达标

- **WHEN** 校验正文、图标与警示条（红底白字）
- **THEN** 对比度不低于 4.5:1

### Requirement: Phase 2 编码仪表盘（REQ-UI37-15）

Phase 2 SHALL 在①区/②区基础上把编码侧深化为**编码仪表盘**，以**只读**为主，需要动作时统一走 Phase 4 的收口入口。四项视图：

1. **单元拓扑图**：六态节点 + 依赖连线，连线态区分「已满足 / 阻塞中」。
2. **依赖链视图**：沿依赖链展开任一元（上游提供方 / 下游消费方），支撑「为什么被挡」的定位。
3. **预算门**：进度条与剩时，**work item 60min / coding 90min 是 driver 约定口径、由前端自计时呈现，MUST NOT 声称其为引擎事件面数据**（引擎不发布这两条预算事件）；临近耗尽按 L4 升级。
4. **实时日志抽取**：从流式 chunk 事件（对话侧 `stream_chunk` / 编码侧 `coding_stream_chunk`）抽取实时日志，按节点归并、可筛选。

#### Scenario: 拓扑图与依赖连线

- **WHEN** 编码侧存在多单元及其依赖关系
- **THEN** 呈现六态节点与依赖连线，连线区分已满足与阻塞中，点击可进入依赖链视图

#### Scenario: 定位「为什么被挡」

- **WHEN** 某单元处于阻塞态
- **THEN** 可沿依赖链展开其上游提供方 / 下游消费方，明确指出阻塞来源

#### Scenario: 预算门按 driver 口径自计时

- **WHEN** 呈现 work item / coding 预算进度
- **THEN** 进度条与剩时来自前端自计时（60min / 90min 为 driver 约定口径），界面 MUST NOT 将其描述为引擎发布的事件数据

#### Scenario: 实时日志按节点归并

- **WHEN** 流式 chunk 事件持续到达
- **THEN** 实时日志按节点归并并可筛选，且与对话流渲染解耦

### Requirement: Phase 3 计划审批深度（REQ-UI37-16）

Phase 3 SHALL 深化计划审批视图，三项：

1. **轮次间 markdown revision diff**：呈现轮次间差异 + 变更摘要，回答「这一轮改了什么、为什么改」。
2. **逐条 capability / 契约核对表**：**provided vs required** 两列对照，逐条标出满足 / 缺口；缺口可一键跳转到对应 finding。
3. **DEF-3 吸收——SC 子 session 只读呈现面板**：amendment / plan-repair 子会话的只读呈现；MUST NOT 改变子会话语义、MUST NOT 提供子会话内的写操作。

#### Scenario: 轮次 diff 与摘要

- **WHEN** 计划存在多个修订轮次
- **THEN** 呈现轮次间 markdown diff 与变更摘要

#### Scenario: 契约核对表逐条对照

- **WHEN** 计划含 capability / 契约条目
- **THEN** 以 provided vs required 两列逐条标出满足 / 缺口，缺口可一键跳到对应 finding

#### Scenario: SC 子会话只读呈现且不可写

- **WHEN** 用户查看 amendment / plan-repair 子会话
- **THEN** 面板只读呈现其内容，不出现任何写操作入口，且子会话语义不被修改

### Requirement: Phase 4 操作收口（REQ-UI37-17）

Phase 4 SHALL 把分布在两页的动作入口收口为同一套操作语义与同一个审计出口，**MUST NOT 重复实现**（只做归一）：① 确认 / 反馈 / 接管 / advance 的**键盘快捷键，两页一致**；② **批量确认**仅限可幂等、无副作用的动作，危险动作 MUST NOT 参与批量；③ **历史操作审计视图**呈现谁在何时对哪个门 / 会话做了什么动作，可回查。

#### Scenario: 两页快捷键一致

- **WHEN** 用户在对话侧或编码侧使用确认 / 反馈 / 接管 / advance
- **THEN** 两页的快捷键语义一致

#### Scenario: 批量确认排除危险动作

- **WHEN** 用户选择多条条目执行批量确认
- **THEN** 仅幂等、无副作用的动作可被批量选中，危险动作（如终止、接管）不可参与批量

#### Scenario: 审计视图可回查

- **WHEN** 用户打开历史操作审计视图
- **THEN** 每条记录显示操作者、时间、目标门 / 会话与动作类型，可按目标回查

### Requirement: 自动执行流区（REQ-UI37-18）

② 自动执行流 SHALL 只读呈现「活着的东西」，每行固定四要素：**状态灯**（复用拓扑六态语义色，`running` 为主色呼吸灯，减弱动效下为静态实心）、**进度**（当前阶段 / 总阶段）、**耗时**（等宽 + `tabular-nums`，来源为引擎事件时间戳）、**拓扑小图**（迷你六态节点图）。该区 MUST NOT 提供输入控件。长时间无事件的行 SHALL 进入「静默」视觉（MUST NOT 报警——报警归四层卡壳体系）。点击任意行 / 状态灯 / 拓扑小图 SHALL 下钻 ③ 对话流。长会话 MUST 虚拟化，MUST NOT 因轮次增多而线性增加渲染成本。

#### Scenario: 每行四要素齐备

- **WHEN** 执行流中存在进行中的执行单元
- **THEN** 每行呈现状态灯、阶段进度、已耗时与拓扑小图，耗时使用等宽与 `tabular-nums`

#### Scenario: 静默不报警

- **WHEN** 某行长时间无新事件
- **THEN** 该行进入静默视觉，不产生报警式提示

#### Scenario: 点击下钻

- **WHEN** 用户点击某行或其状态灯 / 拓扑小图
- **THEN** 进入该会话 / 该门的 ③ 对话流详情
