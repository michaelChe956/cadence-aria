## ADDED Requirements

### Requirement: provider run 体积预算与四层门禁（REQ-BOOT-19）

provider run 的每个**未装箱** async 对象 SHALL 受**按归属分层的预算**（超其所属档位硬线即回归、须断言阻断；四档见场景）；入口外壳 SHALL ≤32,768 B。

体积 SHALL 以**未装箱类型**度量，SHALL NOT 以 `Pin<Box<_>>` 指针大小代替（恒绿即假门禁）。

入口 SHALL 只构造**小 capture 外壳**，task body SHALL 首次 poll 内被装箱（构造落堆），SHALL NOT 在 WS inbound 深栈上构造大对象。

门禁 SHALL 由 A（体积）/ B（显式 2 MiB worker + 三守卫与 run 隔离）/ C（真实 WS 深链）/ D（真实制品进程）四层构成，各层 SHALL NOT 互相替代；**C 层的 2 MiB SHALL NOT 被任何栈档位豁免**。


#### Scenario: 入口外壳小且构造落在堆上

- **WHEN** WS handler 发起一次 provider run
- **THEN** 入口只构造 ≤32,768 B 的未装箱外壳；task body SHALL NOT 在 inbound 深栈上构造

#### Scenario: 超硬线的对象被断言阻断

- **WHEN** 某个未装箱 future 的体积超过**其所属档位的硬线**
- **THEN** 体积门禁 SHALL 失败并报出符号、实际字节、**所属档位**与 profile/target/toolchain

#### Scenario: 四层门禁各证其责

- **WHEN** 门禁报告通过
- **THEN** A 层已证未装箱对象体积 ≤ 预算；B 层已证显式 2 MiB worker 上可运行且三守卫与新旧 run 隔离成立；C 层已证真实 axum/socket/inbound 深链 + 可信 provider start + 完整帧 + durable 进展 + 取消后再生成；D 层已证真实制品进程在同一合同下成立

#### Scenario: C 层永不被栈档位豁免

- **WHEN** 生产运行时栈被配置为 8 MiB 或更高
- **THEN** C 层 SHALL 仍以显式 2 MiB worker 执行真实 WS 回归

#### Scenario: 退出码 0 不构成通过

- **WHEN** 动态场景仅以退出码 0 结束
- **THEN** 该场景 SHALL 判失败（未证明服务存活、实际启动、完整帧与 durable 进展）

#### Scenario: debug 数字不得冒充 release

- **WHEN** 体积证据只来自 debug profile
- **THEN** 该证据 SHALL 标注为 debug，且发布前 SHALL 以 release 制品复核

#### Scenario: 四档预算与过渡档归属

- **WHEN** 门禁判定某对象所属的预算档位
- **THEN** 四档为：**P0 核心** 65,536 B · **P0 spawn/准入侧** 32,768 B · **宏体过渡档** 131,072 B · **P1 engine 侧过渡档** 262,144 B
- **AND** 宏体档与 P1 engine 档的对象其源码在写集外（宏文本未迁入 `provider_run/`、engine 侧文件未治理）⇒ SHALL 已登记归属独立 change 与收紧条件
- **AND** P1 engine 档内超过 131,072 B SHALL NOT 判回归，但 SHALL NOT 被当作已收敛

#### Scenario: 过渡档对象必须登记归属

- **WHEN** 某对象按宏体档或 P1 engine 过渡档判绿
- **THEN** 该对象 SHALL 已登记其归属的独立 change 与收紧条件；SHALL NOT 被当作已收敛
