# driver-lease-self-healing Specification

## Purpose

driver 写租约丢失后的自愈契约：驾驶连接因租约被并发 attach 偷走或持有者悬空而收到 STALE_DRIVER_LEASE 时，系统能在不依赖用户手动「重新接管」的情况下恢复写入；租约转移全程可观测可定案。

## Requirements

### Requirement: 写时自愈（REQ-DLS-01）

当写消息到达时租约持有者为空（holder=None）且发送连接角色为 Driver，仲裁层 SHALL 放行该消息并原子地将租约重新授予该连接（含原 holder 因连接断开/交错回滚悬空的一切场景）。该自愈 SHALL 幂等且不放宽其他拒绝语义：observer 写拒绝、holder=Some(其他活跃连接) 时的 STALE 拒绝、以及非写消息的租约校验均保持不变。

#### Scenario: 刷新后首写自愈

- **WHEN** 驾驶连接持有租约后被并发 attach 偷走、偷窃连接随后悬空（holder=None），用户在该会话发送 confirm/反馈
- **THEN** 仲裁层放行该写消息并把租约重新授予当前驾驶连接，操作成功无 STALE 错误，无需手动重新接管

#### Scenario: 活跃偷窃者不被抢

- **WHEN** 租约持有者为另一条活跃连接（holder=Some(other))
- **THEN** 写消息仍按既有语义拒绝 STALE_DRIVER_LEASE（自愈只覆盖悬空态，不引入连接间争抢）

#### Scenario: observer 写拒绝不变

- **WHEN** observer 角色连接发送写消息
- **THEN** 仍按既有语义拒绝（OBSERVER_WRITE_REJECTED），自愈不适用于 observer

### Requirement: 前端单次自动重试（REQ-DLS-02）

前端写命令收到 STALE_DRIVER_LEASE 且自动重发 driver hello 成功重新持有时，SHALL 单次自动重放原写命令；重放成功即无感恢复。重试 SHALL 恰一次：仍失败则展示既有手动接管错误面（含「重新接管」入口），不循环重试。

#### Scenario: 无感恢复

- **WHEN** confirm 因租约悬空被拒且自愈层未覆盖（如持有者为僵尸 observer）
- **THEN** 前端自动重发 hello 取回租约并重放原命令一次，成功后用户无感知

#### Scenario: 重试恰一次

- **WHEN** 自动重放后仍失败
- **THEN** 不再自动重试，显示手动接管错误面

### Requirement: 租约转移可观测（REQ-DLS-03）

租约的获取、临时获取、显式持有、转移、回滚、悬空各事件 SHALL 以 append-only 诊断流 durable 打点（时刻、连接标识、角色、事件类型、原因），并提供只读诊断端点返回当前持有者与最近转移序列。打点失败 SHALL NOT 影响仲裁行为。既有 STALE_DRIVER_LEASE 错误面 SHALL 引用最近相关转移事件（可定案偷窃者身份）。

#### Scenario: 偷窃可定案

- **WHEN** 事后调查某次 STALE_DRIVER_LEASE
- **THEN** 诊断端点能给出该时刻前后的租约转移序列（谁 attach 抢走、何时悬空），无需推测

#### Scenario: 打点失败不影响仲裁

- **WHEN** 诊断流写入失败
- **THEN** 租约仲裁与写消息处理行为不变，仅缺失该次打点

### Requirement: attach 对租约零效应（REQ-DLS-04）

连接 attach SHALL NOT 对租约产生任何获取、转移或快照效应（provisional_lease 快照与回滚机制整体删除）；租约唯一获取点为 hello(driver/缺席归一) 显式获取，holder==None 时的首写自愈（REQ-DLS-01）为无 hello 连接的唯一幸存路径。observer 连接全程不触碰租约。既有 REQ-WCR-02（cockpit-fullcourse-connection-resilience change，未归档）所载「observer 无 lease/显式接管」语义与本 requirement 一致，此处显式交叉引用。仓内既有三个「attach 即抢租约」语义用例（it_core part_06b.rs:83、part_03.rs:579、part_03.rs:807 F-24 现场锚）SHALL 逐名改写为新语义（改写不删除），并补「无 role hello 保归一」覆盖用例。

#### Scenario: observer attach 不偷租约

- **WHEN** observer 连接到某会话（纯 attach，不发 hello）且该会话已有 driver 持有租约
- **THEN** 租约持有者不变，driver 写操作不受影响

#### Scenario: 无 hello 首写经自愈放行

- **WHEN** 连接未发 hello 直接发送写消息且 holder==None
- **THEN** 按 REQ-DLS-01 自愈放行并授予租约（该连接角色归一为 driver 语义）

#### Scenario: 无 hello 且他人持有时拒绝

- **WHEN** 连接未发 hello 直接发送写消息且 holder=Some(其他活跃连接)
- **THEN** 按既有语义拒绝 STALE_DRIVER_LEASE（仓外 raw 客户端行为变化已在 proposal 披露）
