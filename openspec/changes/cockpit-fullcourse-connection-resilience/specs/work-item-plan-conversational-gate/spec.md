## MODIFIED Requirements

### Requirement: 门关闭决定（REQ-CG-04）

人工门 SHALL 仅由两类决定关闭：`approve`（映射现行 `HumanConfirmDecision::Confirm`）关门并启动既有确定性 compile，compile 成功后 Plan 进入 durable Confirmed；`abandon`（映射现行 `HumanConfirmDecision::Terminate`）关门进入终态。**人工权威升级语义（显式化补句）**：approve / abandon 关门 SHALL 携带显式的人工权威升级语义——close CAS 的授权判据为 `status ∈ WaitingForHuman ∧ phase ∈ {Approval, Evaluate} ∧ 门快照在场` 三 conjunct；approve 关门在锁内将 phase 升级为 Approval 并 SHALL 同步内存相位（后续 compile 以同步后相位判定 auto_confirm，compile 成功落 Confirmed、SHALL NOT 回退 enter_human_confirm）；abandon 关门 SHALL NOT 提升 phase。该补句为既有死锁修复（4f34ba58）行为的契约化显式表达，SHALL NOT 改变任何既有行为。人工修订预算耗尽时系统 SHALL 拒绝新的 `human_gate_feedback` 并返回预算耗尽原因，门保持开启且仅接受 approve/abandon——与阶段 1 `work-item-typed-outcome-policy` 既有语义一致，本 capability 不改写该语义。门关闭 SHALL NOT 自动触发 `advance`；advance 是 Plan durable Confirmed 之后由客户端显式调用的独立动作。

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

- **WHEN** `manual_repairs_remaining` 为零且人再次发送 `human_gate_feedback`
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

### Requirement: 门与回合的 durable 恢复（REQ-CG-05）

人工门快照与人工门预算扣减、`HumanGateTurn` 预留、provider 幂等键 SHALL 作为同一 durable reservation 原子提交（在 session record 上以 CAS 写入，复用阶段 1 快照原子写入契约）；恢复时按 reservation 状态决定释放、等待或继续，保证「预算只扣一次」与「同 `command_id` 不重复启动 provider」可被严格证明。`HumanGateTurn` SHALL 为独立 durable 记录（`turn_id`、`command_id`、`feedback_text`、状态、`attempt_no`、预算预留记账、结果引用、失败分类），其 `Reserved` 状态只在与上述原子事务同提交时生效。断线重连后系统 SHALL 从 durable 状态重建门与 in-flight turn：provider 仍在运行则等待其完成；provider 已终止则以同 `turn_id` 恢复。**phase 回门节点**：approval compile 失败与修订中止（含断连中止）后，系统 SHALL 将 phase / active_node 回滚到门节点——快照门呈现与 human_confirm 消息判定重新一致，confirm / feedback SHALL 可达且 MUST NOT 被以 `WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID` 拒绝（消除「stage=human_confirm 却拒 human_confirm」的相位死锁，0429 形态）；该回滚 SHALL 以新的状态写实现，MUST NOT 改写已持久化事件前缀。门相关事件 SHALL 保持事件前缀不可变，恢复不得删除或改写已持久化事件。

#### Scenario: turn 预留原子性

- **WHEN** turn 预留在「预算已扣、turn 未写入」或「turn 已写、预算未扣」的窗口内崩溃，恢复后以同 `command_id` 重发
- **THEN** 系统按 reservation 状态释放/等待/继续，最终预算恰好扣减一次、`turn_id` 恰好一个，无双重启动或双重扣减的中间态可观察

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
