# Spec Delta

## MODIFIED Requirements

### Requirement: 手动推进与启动门（REQ-MTG-03）

**多 target** 推进 SHALL 以人工显式为默认：每个 target-attempt 的 coding provider 首启唯一入口 SHALL 保持显式 `StartCoding` 入站应用命令，SC admission 的 target-attempt 在经 advance 置 Ready 之前收到 StartCoding SHALL fail-closed 拒绝并返回 `SC_CODING_REQUIRES_ADVANCE`（per-attempt 适用，判定=advance record 与该 attempt 的绑定关系，未绑定或未 Ready 一律拒绝）。advance durable record SHALL 保持每 plan 恰一条（幂等不变）；其与 target-attempt 的绑定 SHALL 以 additive 方式扩展承载多 target-attempt 集（既有单 target 记录的 `attempt_id` 承载语义不变、零迁移）。在无精确自动化授权（无 enrollment、enrollment 关闭或不匹配）时，系统 MUST NOT 自动顺序拉起、依赖驱动拉起或以任何编排动作隐式启动跨 target 的 target-attempts——多 target 的推进顺序 SHALL 由人决定（可串行或并行对各自 Ready 的 target-attempt 发 StartCoding），不能以「循环中一次仅启动一个」规避禁止批量编排。

仅 `work-item-group-autopilot` 显式 opt-in enrollment 精确绑定授权 target（一个单仓物理仓——plan 全部工作项无 logical target 归属、attempt 无逻辑代码库 target 快照；一个逻辑代码库单成员；或 logical codebase 成员集合）且 plan 实际桶满足 ∅≠P⊆A、无未归属 WI 时，服务端方可经共享 plan-run 驱动自动推进：以独立共用 StartCoding 命令与 Enrolled advance（`AutoStartOnce` 单发许可）首启与续驱全部实际桶，首启与续驱 SHALL 沿 `CodingPlanSelection::deterministic_start_command_id` 稳定命令身份；全部自动桶（尤其非首桶）SHALL 以 Enrolled origin 启动并逐桶核对 policy/enrollment/binding；上游完成挂点 SHALL 自动启动下游就绪桶。撤销、绑定不符、越权成员、同一 plan 出现授权外 attempt 或 terminated SHALL 零 provider、禁止降级 Manual。这不构成手动顺序的变更：手动与无授权场景的推进语义保持本 requirement 第一段不变；无授权集合的跨 target-attempt 依赖就绪门自动编排仍为 defer 项，本红线不被任何「简化版编排」变体突破。

#### Scenario: per-attempt 守卫零变化

- **WHEN** 某 SC admission 的 target-attempt 尚未被 advance 绑定置 Ready 时收到 StartCoding
- **THEN** 系统以 `SC_CODING_REQUIRES_ADVANCE` fail-closed 拒绝，该 attempt 与 session 状态不变，无 provider 启动

#### Scenario: 手动逐 target 推进

- **WHEN** 多 target 分流创建完成后全部 target-attempt 就绪，且该 plan 无精确自动化授权
- **THEN** 无任何 provider 被自动启动；人对各自 target-attempt 显式发送 StartCoding（先后或并行）方进入执行

#### Scenario: 绑定关系 additive 扩展兼容存量

- **WHEN** 读取既有单 target 场景的 advance record 并判定其 attempt 的 Ready 状态
- **THEN** 判定语义与本 change 前逐字节等价（单值 attempt_id 精确匹配路径保留），多 target 集绑定仅对新场景生效

#### Scenario: 无自动跨 attempt 编排

- **WHEN** 某 target-attempt 完成、失败或进入任意状态转换，且无精确自动化授权
- **THEN** 系统不据此自动启动、排队或建议启动任何其他 target-attempt

#### Scenario: 集合授权的共享 plan-run 自动驱动

- **WHEN** 成员集合 enrollment 精确绑定该 plan（实际桶 P⊆授权集合 A），plan 已确认
- **THEN** 服务端经 Enrolled advance 置各实际桶 Ready、以 AutoStartOnce 单发与稳定命令身份自动首启全部桶；上游完成后自动启动下游就绪桶，各桶 origin 均为当前 binding 的 Enrolled

#### Scenario: 撤销或绑定不符零启动

- **WHEN** 自动驱动过程中 enrollment 被撤销、绑定被篡改、发现越权成员或 plan 已 terminated
- **THEN** 后续自动首启/续驱零 provider、不降级 Manual，停在该点等待显式处理

#### Scenario: 单 target enrollment 的唯一例外

- **WHEN** enrolled plan、权威 target 及冻结 attempt 均精确指向同一 logical repository，advance 已将该唯一 attempt 置 Ready
- **THEN** 有效 per-attempt opt-in 可通过独立 StartCoding 单发首启；advance 本身仍不启动 provider，AI choice/人工门停等保留（正常批准按 `work-item-group-autopilot` REQ-WIGA-02 自动通过）

#### Scenario: 单仓 enrollment 的唯一例外

- **WHEN** 单仓 enrollment 的 target 等于 issue 所属物理仓，plan 全部工作项无 logical target 归属，advance 已将唯一 attempt 置 Ready
- **THEN** 有效 per-attempt opt-in 可通过独立 StartCoding 单发首启；advance 本身仍不启动 provider，AI choice/人工门停等保留（正常批准按 `work-item-group-autopilot` REQ-WIGA-02 自动通过）

#### Scenario: 多 target enrollment 不能泛化为逐个自动发送

- **WHEN** 同一 plan 出现授权集合外的 target-attempt、集合 enrollment 的 plan 混入手动仓 attempt，或 enrollment 范围与冻结 attempt 不符
- **THEN** 所有对该 attempt 的自动首启均拒绝，不因单次只有一个 attempt 就突破授权边界
