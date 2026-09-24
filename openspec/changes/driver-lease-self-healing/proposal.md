# Proposal

## Why

F-50-1（家族第 4 次复发：F-11/F-14/F-28/F-50-1）：driver lease「被偷后永不自动夺回」。实证根因（f50-lease-diagnosis.md）：刷新自动接管正常（accept 即 provisional acquire、hello 后显式持有）；真实缺陷是——①任何第二条到同会话的连接纯 attach 即抢走租约（role 缺省 Driver 的 legacy 语义，it_core part_06b 钉死）；②observer hello 回滚交错可产出「僵尸 observer 持有者」；③holder=None 时驾驶连接写被拒（STALE_DRIVER_LEASE），唯一恢复=手动「重新接管」。触发面：门开触发其它 tab 15s 目录轮询挂 observer socket，attach→hello 处理窗与用户首次点击吻合。偷窃者身份零日志不可证（结构性可观测缺陷）。

## What Changes

1. **写时自愈（方向 A）**：arbitrate 对 holder==None 的 Driver 连接首条写消息放行并重新持有租约（幂等、单次、不放宽 observer 写拒绝）；前端收到 STALE_DRIVER_LEASE 后单次自动重发 driver hello 并重放原写命令（一次，失败则显示既有手动接管面）。
2. **租约可观测（方向 C）**：租约获取/转移/回滚写 durable 打点（持有者连接标识、时刻、原因）+ 只读诊断端点（当前持有者/最近转移序列）；偷窃定案不再依赖推测。
3. **方向 B（oracle 裁决全并入，形态二「attach 对租约零效应」）**：租约获取点收敛为 hello(driver/缺席归一) 显式获取 + A 的 holder==None 首写自愈；provisional_lease 快照/回滚机制整体删除（B②③ 被结构性吸收）。3 个语义反转用例逐名登记改写（part_06b.rs:83 / part_03.rs:579 / part_03.rs:807），对未归档 REQ-WCR-02 做交叉引用（f50-lease-oracle.md）。知情披露：仓外 raw 客户端「无 hello 且他人持有时写」将由放行变为拒绝（从未进 spec，仓内零真实客户端）。

## Capabilities

- `driver-lease-self-healing`: driver 写租约的丢失自愈（写时夺回+前端单次自动重试）与转移可观测契约

（无单独 MODIFIED；REQ-WCR-02（cockpit-fullcourse-connection-resilience，未归档父 change）的交叉引用补丁以 ADDED requirement 内引用形式落在新 capability delta——oracle 登记要求，避免未归档 capability 的 strict 校验失败）

## Non-Goals

- 不放宽 observer 写拒绝（OBSERVER_WRITE_REJECTED 语义不动）
- 不引入租约持久化到 durable session record（打点为 append-only 诊断流，不参与仲裁状态机）
- 不做多 driver 并发仲裁策略（单 driver + observer 模型不变）
- 不动 15s 目录轮询本身（它是合法的多端同步机制，只是其 attach 副作用被 A/B 治理）
