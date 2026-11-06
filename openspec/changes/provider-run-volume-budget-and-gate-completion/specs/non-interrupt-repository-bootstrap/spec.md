## ADDED Requirements

### Requirement: provider run 体积预算与四层门禁（REQ-BOOT-19）

provider run 的每个**未装箱** async 对象 SHALL 受两级体积预算约束：**硬线 131,072 B**（超过即视为回归，必须被断言阻断）与**优先线 65,536 B**（逐对象收敛目标）；provider run 的**入口外壳** SHALL ≤32,768 B。

体积 SHALL 以**未装箱类型**度量（`size_of_val` 作用于 raw future），SHALL NOT 以 `Pin<Box<_>>` 的指针大小代替（恒绿即假门禁）。

入口 SHALL 只构造**小 capture 外壳**，task body SHALL 在该外壳的**首次 poll 内**被装箱（构造落堆），SHALL NOT 在 WS inbound 深栈上构造大对象。

体积门禁 SHALL 由**四层**共同构成，且各层不得互相替代：

| 层 | 证明 | 形态 |
|---|---|---|
A | 每个未装箱对象 ≤ 预算 | 静态断言 + 基线对照 |
B | 代码在**显式 2 MiB worker** 上真能跑；三守卫（engine 锁 / idle drive guard / 实体 lease）与 token、scratch、新旧 run 隔离成立 | 显式 `thread_stack_size(2 MiB)` + worker 执行 + 线程 identity + barrier |
C | 真实 axum/socket/inbound 深链 + 可信 provider start + 完整帧 + durable 进展 + 取消后再生成 | 集成测试（自有 tempdir/端口/进程组） |
D | 真实制品进程在同一合同下成立 | 子进程 + 指纹/profile/生效栈 |

C 层的 **2 MiB SHALL NOT 被任何运行时栈档位豁免**。

动态验收 SHALL NOT 以「退出码 0」为通过依据：SHALL 同时证明 ①服务未 abort 且后续 HTTP/WS 可用 ②可信 provider-start 探针或真实 adapter 日志证明实际启动（`locked`/登记 run 不算）③完整 JSON 帧被解析且满足该场景因果偏序 ④ledger/timeline/message/turn 的 durable 状态按预期进展 ⑤取消后能再生成；客户端 close/timeout、`unsupported` 拒绝、仅初始 `session_state` SHALL 判失败。

SIGABRT SHALL 被视为不可进程内捕获 ⇒ 易 abort 场景 SHALL 以子进程隔离执行（父持续排 stdout/stderr、退出状态与墙钟超时；任何 signal、异常退出、超时均失败；子模式防递归；精确单例名从 `--list` 产出后确认）。

体积数字 SHALL 标注 profile、target 与 toolchain；**debug 数字 SHALL NOT 冒充 release**。发布前 SHALL 以真实 release 制品复核同一合同。

#### Scenario: 入口外壳小且构造落在堆上

- **WHEN** WS handler 发起一次 provider run
- **THEN** 入口只构造 ≤32,768 B 的未装箱外壳；task body SHALL NOT 在 inbound 深栈上构造

#### Scenario: 超硬线的对象被断言阻断

- **WHEN** 某个未装箱 future 的体积超过 131,072 B
- **THEN** 体积门禁 SHALL 失败并报出符号、实际字节与 profile/target/toolchain

#### Scenario: C 层永不被栈档位豁免

- **WHEN** 生产运行时栈被配置为 8 MiB 或 16 MiB
- **THEN** C 层 SHALL 仍以显式 2 MiB worker 执行真实 WS 回归

#### Scenario: 退出码 0 不构成通过

- **WHEN** 动态场景仅以退出码 0 结束
- **THEN** 该场景 SHALL 判失败（未证明服务存活、实际启动、完整帧与 durable 进展）

#### Scenario: debug 数字不得冒充 release

- **WHEN** 体积证据只来自 debug profile
- **THEN** 该证据 SHALL 标注为 debug，且发布前 SHALL 以 release 制品复核
