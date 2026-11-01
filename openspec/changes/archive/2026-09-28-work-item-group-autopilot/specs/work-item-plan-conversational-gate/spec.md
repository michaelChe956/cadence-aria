## MODIFIED Requirements

### Requirement: 门关闭决定（REQ-CG-04）

人工门 SHALL 仅由两类决定关闭：`approve`（以 `Confirm` 语义承载，关门并启动既有确定性 compile，compile 成功后 Plan 进入 durable Confirmed）与 `abandon`（以显式 typed 入站命令承载，关门进入终态）——两类关门决定均以门专属 typed 决策承载，MUST NOT 映射或依赖 legacy `HumanConfirmDecision` 枚举。人工修订预算耗尽时系统 SHALL 拒绝新的 `human_gate_feedback` 并返回预算耗尽原因，门保持开启且仅接受 approve/abandon——与阶段 1 `work-item-typed-outcome-policy` 既有语义一致，本 capability 不改写该语义。门关闭 SHALL NOT 自动触发 `advance`：未 enrolled plan 的 advance 仍由客户端在 durable Confirmed 后显式调用；仅当 plan 已成功 compile/publication 进入 durable Confirmed 且 issue 仍对该精确 plan/session/target 保持有效单 target enrollment，服务端编排器方可**独立**请求 advance。门关闭、approve 点击或 compile/recovery 未成功 SHALL NOT 等同 Confirmed，不得成为服务端推进的依据；人工门仍只有原宿主，编排器不代点。

#### Scenario: approve 不绕过 compile

- **WHEN** 人在门内发出 approve
- **THEN** 系统关门并启动确定性 compile；仅当 compile 成功时 Plan 才进入 durable Confirmed；compile 失败按既有 fail-closed 路径处理

#### Scenario: abandon 经 typed 命令关门

- **WHEN** 人在门内发出显式 typed abandon 命令
- **THEN** 系统关门进入终态，全程不经过 legacy `HumanConfirmDecision` 通道，事件前缀不可变语义保持

#### Scenario: 伪造重开三元组不命中 amendment 接续

- **WHEN** 无任何 Open/Applying `PlanAmendmentContext` 的普通 SC 会话被通用状态写入伪造成 Completed+WaitingForHuman 三元组后，Evaluate 重建或迟到 review verdict 到达
- **THEN** 重建走普通门重置公式（默认预算 − run_history 计数），不继承遗留快照；终态评审守卫照常丢弃该 verdict，不重路由

#### Scenario: amendment 判别命中而快照缺席 fail-closed

- **WHEN** durable 重开签名与 Open/Applying `PlanAmendmentContext` 均在场，但原 `human_gate_snapshot` 缺席（损坏/外部删改）时触发 Approval 重建
- **THEN** 系统以 AbortFatal{PersistenceFailure} 报错终止（含 `human_gate_amendment_snapshot_missing` 诊断），不落任何重建快照，不回退普通门重置公式

#### Scenario: 判别 durable 读失败 fail-closed 不进重置公式

- **WHEN** 真 amendment 门重开在场的会话，其判别所需 durable 事实（session record 或 `PlanAmendmentContext` 文件）损坏/不可读（权限、目录读失败、瞬态 I/O）时触发 Evaluate 重建或迟到 review verdict 到达
- **THEN** 系统以 AbortFatal{PersistenceFailure} 显式 fail-closed 终止（含 `persistence_failure` 诊断），不把「无法读取」当作「无 amendment 事实」走普通门重置公式凭空恢复预算，也不静默丢弃 verdict 保持门开启；诊断写回 durable session record

#### Scenario: 应用窗口不判 amendment 门

- **WHEN** `PlanAmendmentContext` 已先行 Open→Applying 而 group attempt 已离开 AwaitingPlanAmendment（应用窗口内）时，迟到 review verdict 或 Evaluate 重建到达该 Completed+WaitingForHuman 会话
- **THEN** 终态评审守卫照常丢弃该 verdict（不重路由）；Evaluate 重建走普通门重置公式，不接续快照预算——判别与 probe 放行重开的完整谓词同源

#### Scenario: 预算耗尽拒绝新反馈

- **WHEN** `manual_repairs_remaining` 为零且人再次发送 `human_gate_feedback`
- **THEN** 系统拒绝该反馈并返回预算耗尽原因，不创建 turn、不扣减、不启动 provider；门保持开启，仅接受 approve/abandon

#### Scenario: enrolled plan 仅在确认落盘后独立推进

- **WHEN** 人关门批准了 enrollment 精确绑定的 plan，但 compile/recovery 尚未成功落下 durable Confirmed
- **THEN** 不自动 advance；只有随后成功出版且当前仍 enrolled 时编排器才可独立请求 advance，不另建人工门

#### Scenario: 普通 plan 关门仍不自动推进

- **WHEN** 未 enrolled plan durable Confirmed 或曾 enrollment 但已关闭
- **THEN** 原有客户端显式 advance 语义不变，服务器不因门关闭或确认而自动 advance
