# Spec Delta

## ADDED Requirements

### Requirement: enrollment 使用统一双载体 target 合同（REQ-C1-TARGET-01）

enrollment、plan/session 绑定、advance、compile child 及驾驶舱通知 SHALL 使用同一显式 target union：单仓 target 必须携带真实 repository 身份，逻辑代码库 target 必须同时携带 logical codebase 与 logical repository 身份。所有动作 MUST 以该 target 与当前 binding version 进行一致性核对；缺失、模糊、跨载体或不一致的 target SHALL fail-closed，不得把单仓降级为逻辑代码库或依赖路径猜测。该 delta 只提供 C1 所需的载体无关契约，不实现 C5 的完整入口/role-chain gateway 预检。

#### Scenario: 单仓 target 在 plan 链保持一致

- **WHEN** 用户对单仓 issue 明确选择真实 repository target，并执行 enrollment→plan→advance
- **THEN** enrollment、plan、advance 与通知返回同一 repository 身份，不生成 logical target 替身，不因 target 类型转换丢失绑定

#### Scenario: 逻辑代码库 target 身份完整

- **WHEN** 用户选择逻辑代码库 target 并提供 codebase 与 logical repository 身份
- **THEN** 后续绑定和恢复操作保留两级身份并按其核对，缺任一级时拒绝操作且不改绑定

#### Scenario: 跨载体或模糊目标被拒

- **WHEN** 操作携带缺失、多个候选、跨载体不匹配或与当前 enrollment 不一致的 target
- **THEN** 系统返回需重新选择/重新绑定提示，保持现有 durable 事实不变，不自动 fan-out 或猜接管
