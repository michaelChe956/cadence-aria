## Purpose

`advance` 是把 durable Confirmed 的 workitem plan 推进到 coding 就绪的显式幂等编排接口：建立（或恢复）该 plan 的 WorkItemGroup coding attempt 集与全部 durable 绑定——单一 target 场景为唯一 attempt（行为零变化），多 target 场景按 target 分流为每 `(plan, target)` 恰一个 target-attempt（分流契约以 `multi-target-group-coding` 为唯一来源）——返回 group coding workspace 入口。它只负责「就绪」（Ready 即止），不启动 coding provider：首启唯一入口是独立 typed `StartCoding` 应用命令，发起者为人工 WS 命令或 `work-item-group-autopilot` 下仍有效且精确绑定单 target attempt 的 enrolled 编排器；SC admission 在 Ready 前仍由 `SC_CODING_REQUIRES_ADVANCE` fail-closed。原条件 defer 的 opt-in 通道由本 change 显式定义，受 REQ-ADV-05 五条红线约束，不以 advance 随附形式落地。

## MODIFIED Requirements

### Requirement: advance 到 Ready 即止与 coding 启动唯一入口（REQ-ADV-05）

`advance` 的完成态 SHALL 为 `Ready`（group workspace 就绪），advance 本身及 `advance_completed` 事件 SHALL NOT 启动任何 coding provider。coding provider **首次启动**的唯一入口 SHALL 为显式 typed `StartCoding` 入站应用命令，发起者 SHALL 为人工 WS 命令，或本 change 中经 issue enrollment 与该 attempt 的有效 opt-in 授权的服务端编排器；已认领启动的恢复只续接原启动，不构成另一条首启通道。SC admission 的 coding attempt 在经 advance 置 `Ready` 之前收到 `StartCoding` SHALL fail-closed 拒绝并返回错误码 `SC_CODING_REQUIRES_ADVANCE`，attempt 与 session 状态不变；状态/绑定读取失败亦 SHALL fail-closed。此处把原条件 defer 的显式 opt-in auto 启动通道在 `work-item-group-autopilot` 中定义，不允许把自动首启实现为 advance 随附副作用，也不得绕过原 runner 的准入校验。新通道 MUST 满足以下全部五条红线：**opt-in 持久化 run_policy、默认 off、per-attempt 单发、绝不批量、不动唯一人工门**。其中 opt-in 持久化 run_policy SHALL 为 issue 级 durable enrollment 与绑定 attempt 的 `CodingStartRunPolicy=Manual|AutoStartOnce`，并非 plan session 的 `RunPolicy::AutoIfValid`；plan session SHALL 保持 Interactive。授权关闭后尚未消费的启动许可可撤销；仅恰一 logical repository 的 enrolled plan 可自动首启，多 target 的逐 target 人工 StartCoding 与 `multi-target-group-coding` REQ-MTG-03 不变。

#### Scenario: advance 完成不启动 provider

- **WHEN** advance 执行到完成态 `Ready` 并发出 `advance_completed`
- **THEN** 无任何 coding provider 被启动；StartCoding 命令仍须另行授权并进入共用准入服务

#### Scenario: Ready 前 StartCoding 被守卫拒绝

- **WHEN** SC admission 的 attempt 尚未经 advance 置 `Ready` 时收到人工或 enrolled StartCoding
- **THEN** 系统以错误码 `SC_CODING_REQUIRES_ADVANCE` fail-closed 拒绝，attempt 与 session 状态不变，无 provider 启动

#### Scenario: 编排动作不隐式启动 provider

- **WHEN** advance 完成、`advance_completed` 事件唤醒编排器或恢复重放
- **THEN** advance 本身不启动 provider；只有单 target 有效 opt-in attempt 经独立 StartCoding 命令、准入与单发认领后方能首启

#### Scenario: auto 通道红线条件 defer 登记

- **WHEN** issue 选择自动化并批准 plan，编排器准备对绑定单 target attempt 首次启动 coding
- **THEN** 仅当 opt-in 持久化 run_policy 有效、缺省 off 的许可被明确开启时允许 per-attempt 单发；绝不批量启动、不代点唯一人工计划门及人工 Final Confirm

#### Scenario: 授权关闭与多 target 禁止自动首启

- **WHEN** enrollment 在首启许可消费前关闭，或计划包含两个以上 logical repository
- **THEN** 服务端不自动启动任何 coding provider；原人工逐 target StartCoding 不受此例外扩权
