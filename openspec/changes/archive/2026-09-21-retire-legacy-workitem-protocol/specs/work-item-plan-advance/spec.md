# work-item-plan-advance Delta

## Purpose

DEF-4 契约显式化：advance 的完成态（Ready 即止）、coding provider 启动唯一入口（显式 StartCoding）与 fail-closed 守卫（SC_CODING_REQUIRES_ADVANCE）写入 spec 结项；显式 opt-in auto 通道以红线条件项登记 defer。旧协议退役使「SC 与 legacy 入口隔离」要求完成使命：legacy 入口保留句失效，准入契约由新 requirement 承接（REQ-ADV-03 → REQ-ADV-06）。

## ADDED Requirements

### Requirement: SC 准入唯一入口与共享初始化约束（REQ-ADV-06）

单候选（SC）plan SHALL 仅经 `advance` 准入 group coding，系统 SHALL NOT 存在任何其他 coding 准入入口。旧协议（legacy 逐段 plan 决策协议及其 group coding 入口）已按 `legacy-protocol-retirement` 退役删除：legacy 入口不复存在，已删除消息类型收到时 SHALL 返回 protocol error 且零副作用。两条历史入口共享的 group 初始化逻辑 SHALL 保持 attempt 唯一性、plan binding、worktree lock 与 replay 四道约束；`flow_kind` 字段仅作为存量 durable 记录的只读历史值保留（新会话一律单候选流，处置细则以 `legacy-protocol-retirement` REQ-RET-03 为唯一来源）。

#### Scenario: SC plan 绕过 advance 直接进入 coding 被拒

- **WHEN** 对 SC plan 的子 session 直接调用 coding 启动入口而未经过 `advance`
- **THEN** 系统拒绝并提示先执行 advance；session 与 attempt 状态不变

#### Scenario: 已删除 legacy 入口协议错误拒绝

- **WHEN** 客户端以已删除的 legacy 逐段 plan 决策消息尝试进入 group coding
- **THEN** 系统返回 protocol error 且零副作用，不存在 legacy 准入路径

#### Scenario: flow_kind 仅作只读历史值

- **WHEN** 读取存量 durable 记录中带 legacy 取值的 `flow_kind` 字段
- **THEN** 系统以只读历史值呈现（不路由到 legacy 引擎、不报错），新会话创建一律单候选流

### Requirement: advance 到 Ready 即止与 coding 启动唯一入口（REQ-ADV-05）

`advance` 的完成态 SHALL 为 `Ready`（group workspace 就绪），advance 本身及 `advance_completed` 事件 SHALL NOT 启动任何 coding provider。coding provider 启动的唯一入口 SHALL 为显式 `StartCoding` 入站命令（人工触发）；advance 完成、事件编排、恢复重放或任何其他服务端动作 SHALL NOT 隐式启动、批量启动或随附启动 coding provider。SC admission 的 coding attempt 在经 advance 置 `Ready` 之前收到 `StartCoding` SHALL fail-closed 拒绝并返回错误码 `SC_CODING_REQUIRES_ADVANCE`，attempt 与 session 状态不变。显式 opt-in auto 启动通道为条件 defer 项（触发条件=autopilot/驾驶舱真实 auto 需求），本 requirement 不定义其实施；若未来立项 MUST 满足全部红线：opt-in 持久化 run_policy、默认 off、per-attempt 单发、绝不批量、不动唯一人工门，且 MUST 另行 change 显式定义，MUST NOT 以隐式或随 advance 形态落地。

#### Scenario: advance 完成不启动 provider

- **WHEN** advance 执行到完成态 `Ready` 并发出 `advance_completed`
- **THEN** 无任何 coding provider 被启动（显式 `StartCoding` 之前 attempt 保持就绪不运行）

#### Scenario: Ready 前 StartCoding 被守卫拒绝

- **WHEN** SC admission 的 attempt 尚未经 advance 置 `Ready` 时收到 `StartCoding`
- **THEN** 系统以错误码 `SC_CODING_REQUIRES_ADVANCE` fail-closed 拒绝，attempt 与 session 状态不变，无 provider 启动

#### Scenario: 编排动作不隐式启动 provider

- **WHEN** advance 完成、`advance_completed` 事件编排、恢复重放或任何服务端自动动作执行
- **THEN** coding provider 仅在显式 `StartCoding` 入站命令后启动，无隐式、批量或随附启动路径

#### Scenario: auto 通道红线条件 defer 登记

- **WHEN** 未来出现 autopilot/驾驶舱真实 auto 需求并拟启动 coding provider
- **THEN** 只能以另行 change 显式定义 opt-in auto 通道且满足全部红线五条（opt-in 持久化 run_policy、默认 off、per-attempt 单发、绝不批量、不动唯一人工门）；在此之前任何 auto 启动路径不存在

## REMOVED Requirements

### Requirement: SC 与 legacy 入口隔离（REQ-ADV-03）

- **Rationale**: 旧协议已按 `legacy-protocol-retirement` 退役删除，「legacy 入口与旧协议原样保留」「退役门未满足前不删」条款失效；SC 准入与共享初始化约束由 ADDED REQ-ADV-06 承接（原「SC plan 绕过 advance 被拒」场景随迁），legacy 隔离面（原「legacy 入口行为零变化」场景）随退役消亡。
