## MODIFIED Requirements

### Requirement: 手动推进与启动门（REQ-MTG-03）

一期**多 target** 推进 SHALL 为人工显式：每个 target-attempt 的 coding provider 首启唯一入口 SHALL 保持显式 `StartCoding` 入站应用命令，SC admission 的 target-attempt 在经 advance 置 Ready 之前收到 StartCoding SHALL fail-closed 拒绝并返回 `SC_CODING_REQUIRES_ADVANCE`（per-attempt 适用，判定=advance record 与该 attempt 的绑定关系，未绑定或未 Ready 一律拒绝）。advance durable record SHALL 保持每 plan 恰一条（幂等不变）；其与 target-attempt 的绑定 SHALL 以 additive 方式扩展承载多 target-attempt 集（既有单 target 记录的 `attempt_id` 承载语义不变、零迁移）。系统 MUST NOT 自动顺序拉起、依赖驱动拉起或以任何编排动作隐式启动跨 target 的 target-attempts——多 target 的推进顺序 SHALL 由人决定（可串行或并行对各自 Ready 的 target-attempt 发 StartCoding），不能以「循环中一次仅启动一个」规避禁止批量编排。

仅 `work-item-group-autopilot` 显式 opt-in enrollment **精确绑定一个 logical repository 且只对应一个 target-attempt** 时，服务端可按 `work-item-plan-advance` REQ-ADV-05 经独立共用 StartCoding 命令首次启动该唯一 attempt；这不构成多 target 自动启动的例外。enrollment 的 target 范围为零、多于一、与冻结 attempt 不符，或同一 plan 出现多个 target-attempt 时 MUST fail-closed 禁止自动首启；既有逐 target 手动操作不受影响。多 target 自动启动及跨 target-attempt 依赖就绪门自动编排仍为二期 defer 项：触发条件=一期交付后的真实多仓使用证据，届时凭证据另行立项，本红线在本 change 内不被任何「简化版编排」变体突破。

#### Scenario: per-attempt 守卫零变化

- **WHEN** 某 SC admission 的 target-attempt 尚未被 advance 绑定置 Ready 时收到 StartCoding
- **THEN** 系统以 `SC_CODING_REQUIRES_ADVANCE` fail-closed 拒绝，该 attempt 与 session 状态不变，无 provider 启动

#### Scenario: 手动逐 target 推进

- **WHEN** 多 target 分流创建完成后全部 target-attempt 就绪
- **THEN** 无任何 provider 被自动启动；人对各自 target-attempt 显式发送 StartCoding（先后或并行）方进入执行

#### Scenario: 绑定关系 additive 扩展兼容存量

- **WHEN** 读取既有单 target 场景的 advance record 并判定其 attempt 的 Ready 状态
- **THEN** 判定语义与本 change 前逐字节等价（单值 attempt_id 精确匹配路径保留），多 target 集绑定仅对新场景生效

#### Scenario: 无自动跨 attempt 编排

- **WHEN** 某 target-attempt 完成、失败或进入任意状态转换
- **THEN** 系统不据此自动启动、排队或建议启动任何其他 target-attempt（编排缺席是一期红线）

#### Scenario: 单 target enrollment 的唯一例外

- **WHEN** enrolled plan、权威 target 及冻结 attempt 均精确指向同一 logical repository，advance 已将该唯一 attempt 置 Ready
- **THEN** 有效 per-attempt opt-in 可通过独立 StartCoding 单发首启；advance 本身仍不启动 provider，人工 plan 门保留

#### Scenario: 多 target enrollment 不能泛化为逐个自动发送

- **WHEN** enrollment/plan 中发现两个 logical repository 或两个 target-attempt，即便编排器拟逐个顺序发 StartCoding
- **THEN** 所有自动首启均拒绝，不因单次只有一个 attempt 就突破多 target 人工决定顺序的红线
